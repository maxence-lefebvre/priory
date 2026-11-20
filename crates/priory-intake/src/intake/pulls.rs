use super::gh::{Login, gh};
use super::{CHECK_RUNS, CHECKED, COMMENTED, Observed, REVIEWED};
use herdr_bus::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// GitHub creates check runs seconds after a push, and a push updates the pull request.
const GRACE_SECONDS: i64 = 5 * 60;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Pull {
    pub(super) number: u64,
    pub(super) head_ref_name: String,
    pub(super) head_ref_oid: String,
    pub(super) base_ref_name: String,
    pub(super) merged: bool,
    updated_at: String,
    reviews: Nodes<Review>,
    #[serde(default)]
    comments: Nodes<Comment>,
    commits: Nodes<Commit>,
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
}

#[derive(Deserialize)]
struct Author {
    login: String,
    #[serde(rename = "__typename")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Review {
    database_id: u64,
    state: String,
    submitted_at: Option<String>,
    url: String,
    author: Option<Login>,
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
            let data = json!({
                "repo": repo,
                "pull_request": pull.number,
                "url": url,
                "head_branch": pull.head_ref_name,
                "head_sha": pull.head_ref_oid,
                "base_branch": pull.base_ref_name,
                "time": time,
                "github_id": github_id,
            });
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
            observed.data["actor"] = json!(review.author.as_ref().map(|author| &author.login));
            reviews.push(observed);
        }
        let people = pull.comments.nodes.iter().filter(|comment| {
            comment
                .author
                .as_ref()
                .is_none_or(|author| author.kind != "Bot")
        });
        for comment in people {
            let data = json!({
                "url": comment.url,
                "actor": comment.author.as_ref().map(|author| &author.login),
                "body": comment.body,
                "time": comment.created_at,
                "github_id": comment.database_id,
            });
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

const PULL_FIELDS: &str = "fragment pull on PullRequest { number headRefName headRefOid baseRefName merged \
    reviews(last: 50) { nodes { databaseId state submittedAt url author { login } } } \
    comments(last: 50) { nodes { databaseId body url createdAt author { login __typename } } } \
    updatedAt commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { totalCount nodes { \
    __typename ... on CheckRun { databaseId name status conclusion completedAt url checkSuite { app { slug } } } \
    } } } } } } }";

/// The open pull requests of `repo`, 100 per page.
pub(super) fn open_pulls(repo: &str) -> Result<Vec<Pull>, String> {
    let (owner, name) = repo.split_once('/').unwrap_or_default();
    let query = format!(
        "query($owner: String!, $name: String!, $endCursor: String) {{ repository(owner: $owner, name: $name) {{ \
         pullRequests(states: OPEN, first: 100, after: $endCursor) {{ pageInfo {{ hasNextPage endCursor }} \
         nodes {{ ...pull }} }} }} }} {PULL_FIELDS}"
    );
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
            &format!("query={query}"),
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

/// The pull requests `numbers` name, in one GraphQL query; none when `numbers` is empty.
pub(super) fn pull_requests(
    repo: &str,
    numbers: &BTreeSet<u64>,
) -> Result<BTreeMap<u64, Pull>, String> {
    if numbers.is_empty() {
        return Ok(BTreeMap::new());
    }
    let (owner, name) = repo.split_once('/').unwrap_or_default();
    let fields: String = numbers
        .iter()
        .map(|number| format!("pr{number}: pullRequest(number: {number}) {{ ...pull }} "))
        .collect();
    let query = format!(
        "query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{ {fields}}} }} {PULL_FIELDS}"
    );
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
    found
        .filter(|(_, pull)| !pull.is_null())
        .map(|(_, pull)| {
            let pull: Pull = serde_json::from_value(pull.clone())
                .map_err(|error| format!("gh api graphql: {error}"))?;
            Ok((pull.number, pull))
        })
        .collect()
}

impl Pull {
    /// The head and base fields an event on this pull request carries.
    pub(super) fn branches(&self) -> Value {
        json!({
            "head_branch": self.head_ref_name,
            "head_sha": self.head_ref_oid,
            "base_branch": self.base_ref_name,
        })
    }
}

/// A comment on pull request `number` as an event of `kind`: `branches` holds its head and base
/// fields, and `data` the comment's own, its `github_id` among them.
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
    for field in ["head_branch", "head_sha", "base_branch"] {
        data[field] = branches[field].clone();
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
}

/// The newest 100 review thread comments of `repo`, on any pull request: one REST call, where the
/// GraphQL query would pay for every thread of every open pull request.
pub(super) fn thread_comments(repo: &str) -> Result<Vec<Value>, String> {
    gh(
        &format!("repos/{repo}/pulls/comments?sort=created&direction=desc&per_page=100"),
        &[],
    )
}
