use super::gh::{Login, gh};
use super::{CHECK_RUNS, CHECKED, COMMENTED, Merge, Observed, REVIEWED, Red, RedCheck};
use herdr_bus::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// GitHub creates check runs seconds after a push, and a push updates the pull request.
const GRACE_SECONDS: i64 = 5 * 60;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Pull {
    pub(super) number: u64,
    pub(super) head_ref_name: String,
    pub(super) head_ref_oid: String,
    pub(super) base_ref_name: String,
    #[serde(default)]
    head_repository: Option<Repository>,
    pub(super) merged: bool,
    #[serde(default)]
    pub(super) merge_commit: Option<Oid>,
    #[serde(default)]
    pub(super) body: String,
    state: String,
    #[serde(default)]
    author: Option<Login>,
    updated_at: String,
    reviews: Nodes<Review>,
    #[serde(default)]
    comments: Nodes<Comment>,
    commits: Nodes<Commit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name_with_owner: String,
}

#[derive(Deserialize)]
pub(super) struct Oid {
    pub(super) oid: String,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

impl<T> Default for Nodes<T> {
    fn default() -> Self {
        Nodes { nodes: Vec::new() }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Comment {
    database_id: u64,
    body: String,
    url: String,
    created_at: String,
    author: Option<Author>,
    #[serde(default)]
    author_association: Option<String>,
}

#[derive(Deserialize)]
struct Author {
    login: String,
    #[serde(rename = "__typename", default)]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Review {
    database_id: u64,
    state: String,
    #[serde(default)]
    body: String,
    submitted_at: Option<String>,
    url: String,
    #[serde(default)]
    commit: Option<Oid>,
    author: Option<Author>,
    #[serde(default)]
    author_association: Option<String>,
}

#[derive(Deserialize)]
struct Commit {
    commit: Rollup,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Rollup {
    status_check_rollup: Option<Contexts>,
}

#[derive(Deserialize)]
struct Contexts {
    contexts: Page<Context>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    total_count: usize,
    nodes: Vec<T>,
}

/// A check run, or a commit status with every field absent.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Context {
    database_id: Option<u64>,
    name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    completed_at: Option<String>,
    url: Option<String>,
    check_suite: Option<Suite>,
}

#[derive(Deserialize)]
struct Suite {
    app: Option<App>,
}

#[derive(Deserialize)]
struct App {
    slug: String,
}

/// The submitted reviews and completed check runs of `pulls`, oldest first, and a snapshot of
/// each head's check runs once it has some, or has none past the grace period.
pub(super) fn reviews_and_checks<'a>(
    repo: &str,
    pulls: impl Iterator<Item = &'a Pull>,
    cursor: impl Fn(&'static str) -> Option<u64>,
) -> (Vec<Observed>, Vec<Observed>) {
    let mut reviews = Vec::new();
    let mut checks = Vec::new();
    for pull in pulls {
        let on_pull = |kind, name: String, github_id: u64, time: &str, url: &str| {
            let mut data = json!({
                "repo": repo,
                "pull_request": pull.number,
                "url": url,
                "head_branch": pull.head_ref_name,
                "head_sha": pull.head_ref_oid,
                "base_branch": pull.base_ref_name,
                "time": time,
                "github_id": github_id,
            });
            pull.head_repo(&mut data);
            Observed {
                kind,
                name,
                repo: repo.to_string(),
                number: pull.number,
                label: None,
                baseline: cursor(kind).is_none(),
                data,
            }
        };
        for review in &pull.reviews.nodes {
            let Some(time) = review.submitted_at.as_deref() else {
                continue;
            };
            let name = format!("github:{repo}:pulls/reviews:{}", review.database_id);
            let mut observed = on_pull(REVIEWED, name, review.database_id, time, &review.url);
            observed.data["state"] = json!(review.state.to_lowercase());
            observed.data["body"] = json!(review.body);
            observed.data["commit"] = json!(review.commit.as_ref().map(|commit| &commit.oid));
            observed.data["actor"] = json!(review.author.as_ref().map(|author| &author.login));
            if review
                .author
                .as_ref()
                .is_some_and(|author| author.kind == "Bot")
            {
                observed.data["bot"] = json!(true);
            }
            associate(&mut observed.data, &json!(review.author_association));
            observed.data["open"] = json!(pull.state == "OPEN");
            pull.author(&mut observed.data);
            reviews.push(observed);
        }
        let people = pull.comments.nodes.iter().filter(|comment| {
            comment
                .author
                .as_ref()
                .is_none_or(|author| author.kind != "Bot")
        });
        for comment in people {
            let mut data = json!({
                "url": comment.url,
                "actor": comment.author.as_ref().map(|author| &author.login),
                "body": comment.body,
                "time": comment.created_at,
                "github_id": comment.database_id,
            });
            associate(&mut data, &json!(comment.author_association));
            let mut observed = self::comment(COMMENTED, repo, pull.number, &pull.branches(), data);
            observed.baseline = cursor(REVIEWED).is_none();
            reviews.push(observed);
        }
        let pages: Vec<_> = pull
            .commits
            .nodes
            .iter()
            .filter_map(|commit| commit.commit.status_check_rollup.as_ref())
            .map(|rollup| &rollup.contexts)
            .collect();
        // Contexts past the first page may be check runs still running, so they count as pending.
        let pending: usize = pages
            .iter()
            .map(|page| {
                let running = page.nodes.iter().filter(|check| {
                    check.database_id.is_some() && check.status.as_deref() != Some("COMPLETED")
                });
                running.count() + page.total_count.saturating_sub(page.nodes.len())
            })
            .sum();
        let runs: usize = pages
            .iter()
            .map(|page| {
                let runs = page
                    .nodes
                    .iter()
                    .filter(|check| check.database_id.is_some());
                runs.count() + page.total_count.saturating_sub(page.nodes.len())
            })
            .sum();
        let now = Timestamp::now();
        let quiet = pull
            .updated_at
            .parse::<Timestamp>()
            .is_ok_and(|updated| updated.as_second() + GRACE_SECONDS <= now.as_second());
        if runs > 0 || quiet {
            let some = if runs > 0 { "some" } else { "none" };
            let name = format!(
                "github:{repo}:pulls/{}:check-runs:{}:{some}",
                pull.number, pull.head_ref_oid
            );
            let url = format!("https://github.com/{repo}/pull/{}", pull.number);
            let mut observed = on_pull(CHECK_RUNS, name, 0, &now.to_string(), &url);
            observed.baseline = cursor(CHECKED).is_none();
            let data = observed.data.as_object_mut().expect("an object");
            data.remove("github_id");
            data.insert("runs".into(), json!(runs));
            data.insert("updated_at".into(), json!(pull.updated_at));
            checks.push(observed);
        }
        let contexts = pages.iter().flat_map(|page| &page.nodes);
        let failing = contexts
            .clone()
            .filter(|check| {
                check.database_id.is_some() && check.status.as_deref() == Some("COMPLETED")
            })
            .filter(|check| {
                let conclusion = check
                    .conclusion
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase();
                !super::PASSING.contains(&conclusion.as_str())
            })
            .count();
        for check in contexts.filter(|check| check.status.as_deref() == Some("COMPLETED")) {
            let (Some(id), Some(time)) = (check.database_id, check.completed_at.as_deref()) else {
                continue;
            };
            // One check run can sit on the head of two pull requests.
            let name = format!("github:{repo}:check-runs:{id}:pulls/{}", pull.number);
            let url = check.url.as_deref().unwrap_or_default();
            let mut observed = on_pull(CHECKED, name, id, time, url);
            observed.data["name"] = json!(check.name);
            observed.data["conclusion"] = json!(
                check
                    .conclusion
                    .as_ref()
                    .map(|conclusion| conclusion.to_lowercase())
            );
            let app = check
                .check_suite
                .as_ref()
                .and_then(|suite| suite.app.as_ref());
            observed.data["app"] = json!(app.map(|app| &app.slug));
            observed.data["pending"] = json!(pending);
            observed.data["failing"] = json!(failing);
            checks.push(observed);
        }
    }
    checks.sort_by_key(|check| check.data["github_id"].as_u64());
    (reviews, checks)
}

#[derive(Deserialize)]
struct CommitRuns {
    total_count: usize,
    check_runs: Vec<Run>,
}

#[derive(Deserialize)]
struct Run {
    id: u64,
    name: String,
    #[serde(default)]
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    completed_at: Option<String>,
    html_url: String,
    app: Option<App>,
}

/// The completed check runs on the merge commits of `merges` in `repo`, as events of the pull
/// requests that merged there. A commit is read once per poll until every run completed, when the
/// daemon's record of these events stops `merges` listing it, or until it has no run past the grace
/// period, which `unchecked` remembers. A cancelled run waits for its name's run on a later commit
/// of the base branch, as `later_run` reads it.
pub(super) fn merge_checks(
    repo: &str,
    merges: &[Merge],
    unchecked: &mut HashSet<String>,
    cursor: impl Fn(&'static str) -> Option<u64>,
) -> Result<Vec<Observed>, String> {
    let now = super::now()?.as_second();
    let mut checks = Vec::new();
    let unread: Vec<&Merge> = merges
        .iter()
        .filter(|merge| merge.repo == repo && !unchecked.contains(&merge.commit))
        .collect();
    for merge in unread {
        let path = format!(
            "repos/{repo}/commits/{}/check-runs?per_page=100",
            merge.commit
        );
        let found: CommitRuns = gh(&path, &[])?;
        let merged = merge
            .time
            .parse::<Timestamp>()
            .map_or(now, |time| time.as_second());
        if found.total_count == 0 && merged + GRACE_SECONDS <= now {
            unchecked.insert(merge.commit.clone());
        }
        let conclusion = |run: &Run| run.conclusion.as_deref().unwrap_or_default().to_lowercase();
        let mut done = Vec::new();
        for run in found
            .check_runs
            .iter()
            .filter(|run| run.status == "completed")
        {
            let later = match conclusion(run).as_str() {
                // A base branch deleted after the merge answers 404: count the run as it was.
                "cancelled" => {
                    // The grace runs from the cancel: a newer merge's run starts about then.
                    let fresh = run
                        .completed_at
                        .as_deref()
                        .and_then(|time| time.parse::<Timestamp>().ok())
                        .is_some_and(|time| time.as_second() + GRACE_SECONDS > now);
                    later_run(repo, merge, &run.name, fresh).unwrap_or(Later::Not)
                }
                _ => Later::Not,
            };
            if !matches!(later, Later::Running) {
                done.push((run, later));
            }
        }
        let pending = found.total_count - done.len();
        let failing = done
            .iter()
            .filter(|(run, later)| {
                !super::PASSING.contains(&conclusion(run).as_str())
                    && !matches!(later, Later::Succeeded(_))
            })
            .count();
        for (run, later) in done {
            let Some(time) = run.completed_at.as_deref() else {
                continue;
            };
            let mut data = json!({
                "repo": repo,
                "pull_request": merge.pull_request,
                "url": run.html_url,
                "head_branch": merge.head_branch,
                "head_sha": merge.commit,
                "base_branch": merge.base_branch,
                "time": time,
                "github_id": run.id,
                "name": run.name,
                "conclusion": conclusion(run),
                "app": run.app.as_ref().map(|app| &app.slug),
                "pending": pending,
                "failing": failing,
            });
            if let Later::Succeeded(sha) = later {
                data["superseded_by"] = json!(sha);
            }
            checks.push(Observed {
                kind: CHECKED,
                name: format!(
                    "github:{repo}:check-runs:{}:pulls/{}",
                    run.id, merge.pull_request
                ),
                repo: repo.to_string(),
                number: merge.pull_request,
                label: None,
                baseline: cursor(CHECKED).is_none(),
                data,
            });
        }
    }
    Ok(checks)
}

/// What the run named like a cancelled one did on a later commit of the base branch.
enum Later {
    /// It succeeded on this descendant commit, so the cancelled run is not red.
    Succeeded(String),
    /// It has not concluded yet, so the cancelled run waits for it.
    Running,
    /// There is none on a descendant commit, or it failed: the cancelled run counts as red.
    Not,
}

#[derive(Deserialize)]
struct Compare {
    status: String,
}

/// The run named `name` on the tip of `merge`'s base branch, when the tip descends from the merge
/// commit. A cancelled tip run waits too: a newer push cancelled it, and its own run follows. While
/// `fresh`, a tip with no run of that name waits for one to start.
fn later_run(repo: &str, merge: &Merge, name: &str, fresh: bool) -> Result<Later, String> {
    let path = format!(
        "repos/{repo}/commits/heads/{}/check-runs",
        encoded(&merge.base_branch)
    );
    let check_name = format!("check_name={name}");
    let found: CommitRuns = gh(&path, &["-X", "GET", "-f", &check_name])?;
    // shortcut: a fresh empty tip waits without a compare, so a diverged tip delays red by the grace.
    let Some(tip) = found.check_runs.first() else {
        return Ok(if fresh { Later::Running } else { Later::Not });
    };
    if tip.head_sha.is_empty() || tip.head_sha == merge.commit {
        return Ok(Later::Not);
    }
    let path = format!("repos/{repo}/compare/{}...{}", merge.commit, tip.head_sha);
    let compare: Compare = gh(&path, &["-X", "GET", "-f", "per_page=1"])?;
    if compare.status != "ahead" {
        return Ok(Later::Not);
    }
    let conclusion = tip.conclusion.as_deref().unwrap_or_default().to_lowercase();
    Ok(match (tip.status.as_str(), conclusion.as_str()) {
        ("completed", "success") => Later::Succeeded(tip.head_sha.clone()),
        ("completed", "cancelled") => Later::Running,
        ("completed", _) => Later::Not,
        _ => Later::Running,
    })
}

/// Whether every check that failed on `red`'s merge commit passed later on a commit of the base
/// branch that contains the merge. A read that fails, as for a deleted base branch, says no.
pub(super) fn recovered(red: &Red) -> bool {
    !red.checks.is_empty()
        && red.checks.iter().all(|check| {
            let tip = passed_tip(red, check).unwrap_or_default();
            tip.iter()
                .chain(&check.passed)
                .any(|sha| contains(red, sha).unwrap_or_default())
        })
}

/// The tip of `red`'s base branch, when every run of `check` there succeeded, later than the red one.
fn passed_tip(red: &Red, check: &RedCheck) -> Result<Option<String>, String> {
    let path = format!(
        "repos/{}/commits/heads/{}/check-runs",
        red.repo,
        encoded(&red.base_branch)
    );
    let check_name = format!("check_name={}", check.name);
    let found: CommitRuns = gh(
        &path,
        &["-X", "GET", "-f", &check_name, "-f", "per_page=100"],
    )?;
    let runs: Vec<&Run> = found
        .check_runs
        .iter()
        .filter(|run| run.app.as_ref().map(|app| &app.slug) == check.app.as_ref())
        .collect();
    let passed = found.total_count == found.check_runs.len()
        && runs.iter().all(|run| {
            run.id > check.id
                && run.status == "completed"
                && run.conclusion.as_deref() == Some("success")
        });
    Ok(runs
        .first()
        .filter(|_| passed)
        .map(|run| run.head_sha.clone()))
}

/// Whether the commit `sha` is `red`'s merge commit or descends from it.
fn contains(red: &Red, sha: &str) -> Result<bool, String> {
    if sha == red.commit {
        return Ok(true);
    }
    let path = format!("repos/{}/compare/{}...{sha}", red.repo, red.commit);
    let compare: Compare = gh(&path, &["-X", "GET", "-f", "per_page=1"])?;
    Ok(compare.status == "ahead")
}

/// `text` percent-encoded as one URL path segment, so a branch such as `release/x` stays one.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

const PULL_FIELDS: &str = "fragment pull on PullRequest { number headRefName headRefOid baseRefName headRepository { nameWithOwner } merged mergeCommit { oid } body state author { login } \
    reviews(last: 50) { nodes { databaseId state body submittedAt url commit { oid } author { login __typename } authorAssociation } } \
    comments(last: 50) { nodes { databaseId body url createdAt author { login __typename } authorAssociation } } \
    updatedAt commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { totalCount nodes { \
    __typename ... on CheckRun { databaseId name status conclusion completedAt url checkSuite { app { slug } } } \
    } } } } } } }";

/// How many pull requests one GraphQL request reads. Each brings up to 50 reviews, 50 comments and
/// 100 check runs, so 100 per request made about 20,000 nodes, which GitHub on a busy repository
/// answered with a 504.
const PULLS_PER_REQUEST: usize = 25;

fn open_pulls_query() -> String {
    format!(
        "query($owner: String!, $name: String!, $endCursor: String) {{ repository(owner: $owner, name: $name) {{ \
         pullRequests(states: OPEN, first: {PULLS_PER_REQUEST}, after: $endCursor) {{ pageInfo {{ hasNextPage endCursor }} \
         nodes {{ ...pull }} }} }} }} {PULL_FIELDS}"
    )
}

/// The open pull requests of `repo`, `PULLS_PER_REQUEST` per page.
pub(super) fn open_pulls(repo: &str) -> Result<Vec<Pull>, String> {
    let (owner, name) = repo.split_once('/').unwrap_or_default();
    let pages: Vec<serde_json::Value> = gh(
        "graphql",
        &[
            "--paginate",
            "--slurp",
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("name={name}"),
            "-f",
            &format!("query={}", open_pulls_query()),
        ],
    )?;
    let nodes = pages
        .iter()
        .flat_map(|page| page["data"]["repository"]["pullRequests"]["nodes"].as_array())
        .flatten();
    nodes
        .map(|pull| {
            serde_json::from_value(pull.clone()).map_err(|error| format!("gh api graphql: {error}"))
        })
        .collect()
}

/// One query per `PULLS_PER_REQUEST` of `numbers`, each naming its pull requests by alias.
fn by_number_queries(numbers: &BTreeSet<u64>) -> Vec<String> {
    let numbers: Vec<u64> = numbers.iter().copied().collect();
    numbers
        .chunks(PULLS_PER_REQUEST)
        .map(|chunk| {
            let fields: String = chunk
                .iter()
                .map(|number| format!("pr{number}: pullRequest(number: {number}) {{ ...pull }} "))
                .collect();
            format!(
                "query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{ {fields}}} }} {PULL_FIELDS}"
            )
        })
        .collect()
}

/// The pull requests `numbers` name, `PULLS_PER_REQUEST` to a GraphQL query; none when `numbers`
/// is empty.
pub(super) fn pull_requests(
    repo: &str,
    numbers: &BTreeSet<u64>,
) -> Result<BTreeMap<u64, Pull>, String> {
    let (owner, name) = repo.split_once('/').unwrap_or_default();
    let mut pulls = BTreeMap::new();
    for query in by_number_queries(numbers) {
        let answer: serde_json::Value = gh(
            "graphql",
            &[
                "-f",
                &format!("owner={owner}"),
                "-f",
                &format!("name={name}"),
                "-f",
                &format!("query={query}"),
            ],
        )?;
        let found = answer["data"]["repository"]
            .as_object()
            .into_iter()
            .flatten();
        for (_, pull) in found.filter(|(_, pull)| !pull.is_null()) {
            let pull: Pull = serde_json::from_value(pull.clone())
                .map_err(|error| format!("gh api graphql: {error}"))?;
            pulls.insert(pull.number, pull);
        }
    }
    Ok(pulls)
}

impl Pull {
    /// The head and base fields an event on this pull request carries.
    pub(super) fn branches(&self) -> Value {
        let mut branches = json!({
            "head_branch": self.head_ref_name,
            "head_sha": self.head_ref_oid,
            "base_branch": self.base_ref_name,
            "open": self.state == "OPEN",
        });
        self.head_repo(&mut branches);
        self.author(&mut branches);
        branches
    }

    /// Sets `author` on `data` to the login that opened the pull request, when GitHub names one.
    fn author(&self, data: &mut Value) {
        if let Some(author) = &self.author {
            data["author"] = json!(author.login);
        }
    }

    /// Sets `head_repo` on `data` when GitHub still names the head's repository.
    pub(super) fn head_repo(&self, data: &mut Value) {
        if let Some(head) = &self.head_repository {
            data["head_repo"] = json!(head.name_with_owner);
        }
    }
}

/// A comment on pull request `number` as an event of `kind`: `branches` holds its head and base
/// fields and whether it is open, and `data` the comment's own, its `github_id` among them.
pub(super) fn comment(
    kind: &'static str,
    repo: &str,
    number: u64,
    branches: &Value,
    mut data: Value,
) -> Observed {
    let path = if kind == COMMENTED { "issues" } else { "pulls" };
    let name = format!("github:{repo}:{path}/comments:{}", data["github_id"]);
    data["repo"] = json!(repo);
    data["pull_request"] = json!(number);
    for field in ["head_branch", "head_sha", "base_branch", "open"] {
        data[field] = branches[field].clone();
    }
    for field in ["head_repo", "author"] {
        if let Some(value) = branches.get(field) {
            data[field] = value.clone();
        }
    }
    Observed {
        kind,
        name,
        repo: repo.to_string(),
        number,
        label: None,
        baseline: false,
        data,
    }
}

/// Sets `association` on `data` to GitHub's `author_association`, when it gives one.
pub(super) fn associate(data: &mut Value, association: &Value) {
    if association.is_string() {
        data["association"] = association.clone();
    }
}

/// The fields of a review thread comment, as `pulls/comments` and a webhook give it; none for a
/// bot's. Its thread is its first comment's id.
pub(super) fn thread_comment(comment: &Value) -> Option<Value> {
    if comment["user"]["type"] == "Bot" {
        return None;
    }
    Some(json!({
        "url": comment["html_url"],
        "actor": comment["user"]["login"],
        "body": comment["body"],
        "path": comment["path"],
        "line": comment["line"],
        "thread": comment["in_reply_to_id"].as_u64().or(comment["id"].as_u64()),
        "time": comment["created_at"],
        "github_id": comment["id"],
    }))
    .map(|mut data| {
        associate(&mut data, &comment["author_association"]);
        data
    })
}

/// The newest 100 review thread comments of `repo`, on any pull request: one REST call, where the
/// GraphQL query would pay for every thread of every open pull request.
pub(super) fn thread_comments(repo: &str) -> Result<Vec<Value>, String> {
    gh(
        &format!("repos/{repo}/pulls/comments?sort=created&direction=desc&per_page=100"),
        &[],
    )
}

/// The newest 100 conversation comments of `repo`, on its issues and pull requests alike: one REST
/// call, as for review thread comments.
pub(super) fn issue_comments(repo: &str) -> Result<Vec<Value>, String> {
    gh(
        &format!("repos/{repo}/issues/comments?sort=created&direction=desc&per_page=100"),
        &[],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_is_encoded_as_one_path_segment() {
        assert_eq!(encoded("main"), "main");
        assert_eq!(encoded("release/1.x"), "release%2F1.x");
        assert_eq!(encoded("fix #2 é"), "fix%20%232%20%C3%A9");
    }

    /// The most nodes a GraphQL selection can return: each `first:` or `last:` connection
    /// multiplies the nodes under it, as GitHub counts them against its limits.
    fn nodes(selection: &str) -> u64 {
        let (mut total, mut stack, mut pending) = (0, vec![1u64], None);
        let mut rest = selection;
        while let Some(at) = rest.find(['{', '}', ':']) {
            let (before, after) = rest.split_at(at);
            match after.as_bytes()[0] {
                b'{' => {
                    let top = *stack.last().unwrap();
                    let size = pending.take().map_or(top, |size: u64| top * size);
                    if size != top {
                        total += size;
                    }
                    stack.push(size);
                }
                b'}' => {
                    stack.pop();
                }
                _ => {
                    if before.ends_with("first") || before.ends_with("last") {
                        let digits: String = after[1..]
                            .trim_start()
                            .chars()
                            .take_while(char::is_ascii_digit)
                            .collect();
                        pending = digits.parse().ok();
                    }
                }
            }
            rest = &after[1..];
        }
        total
    }

    #[test]
    fn the_node_count_reads_nested_connections() {
        assert_eq!(
            nodes("a(first: 10) { b(last: 5) { c } d(first: 2) { e } }"),
            80
        );
    }

    #[test]
    fn one_request_of_the_pull_request_query_returns_at_most_6000_nodes() {
        let per_pull = nodes(PULL_FIELDS);
        let open = nodes(&open_pulls_query()) + PULLS_PER_REQUEST as u64 * per_pull;
        let numbers: BTreeSet<u64> = (1..=200).collect();
        let named = by_number_queries(&numbers);
        assert!(open <= 6000, "{open} nodes per page of open pull requests");
        for query in &named {
            let aliases = query.matches(": pullRequest(number:").count() as u64;
            let named = aliases * (1 + per_pull);
            assert!(
                named <= 6000,
                "{named} nodes in one query of named pull requests"
            );
        }
        assert_eq!(
            named
                .iter()
                .map(|query| query.matches(": pullRequest(number:").count())
                .sum::<usize>(),
            200
        );
    }
}
