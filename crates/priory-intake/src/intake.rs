use herdr_bus::{Bus, Message, MessageQuery, Output, Timestamp, Uuid};
use priory_factory::factory::{Factory, Trigger};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::process::Command;
use std::time::Duration;

pub const OUTPUT: &str = "factory.intake";
const LABELED: &str = "github.issues.labeled";
const OPENED: &str = "github.issues.opened";
const PR_OPENED: &str = "github.pull_request.opened";
const PR_LABELED: &str = "github.pull_request.labeled";
const PR_CLOSED: &str = "github.pull_request.closed";
const REVIEWED: &str = "github.pull_request_review.submitted";
const CHECKED: &str = "github.check_run.completed";
/// The streams of a repo: the message type each is named after.
const STREAMS: [&str; 4] = [LABELED, OPENED, REVIEWED, CHECKED];
pub(crate) const TASK_REQUESTED: &str = "factory.task.requested";
const BASELINE: &str = "factory.intake.baseline";
const PAGE: usize = 100;
const EVERY: Duration = Duration::from_secs(60);

/// A repo and the source message type polled for it.
type Stream = (String, &'static str);

/// Where polling stopped, per stream: the highest issue event id for labeled events, the highest
/// issue number for opened issues. A stream without one is still in its baseline.
type Cursors = BTreeMap<Stream, u64>;

/// The `factory.intake` declaration, shared with `priory request`, which publishes on it too.
pub(crate) fn output() -> Output {
    Output::new(OUTPUT)
        .describe("Priory intake: source events and the tasks they request")
        .types([
            LABELED,
            OPENED,
            PR_OPENED,
            PR_LABELED,
            PR_CLOSED,
            REVIEWED,
            CHECKED,
            crate::request::REQUESTED,
            TASK_REQUESTED,
            BASELINE,
        ])
}

pub fn run(factory: &Factory, once: bool) -> Result<(), String> {
    let triggers: Vec<(&str, &Trigger)> = factory
        .triggers
        .iter()
        .filter(|(_, trigger)| trigger.value.source == "github")
        .map(|(name, trigger)| (name.as_str(), &trigger.value))
        .collect();
    let repos: BTreeSet<&str> = triggers
        .iter()
        .filter_map(|(_, trigger)| trigger.repo.as_deref())
        .collect();
    let bus = Bus::open_default().map_err(|error| error.to_string())?;
    bus.declare(&output()).map_err(|error| error.to_string())?;
    let (newest, mut cursors) = newest(&bus, &repos).map_err(|error| error.to_string())?;
    for (stream, source) in newest {
        // A crash may have stopped between this source and its task requests.
        if source.data["baseline"] != true {
            dispatch(&bus, &triggers, &source)?;
        }
        if let Some(cursor) = cursors.get_mut(&stream) {
            *cursor = (*cursor).max(position(&source.data, stream.1));
        }
    }
    // shortcut: grows by one id per source event for the life of the process; prune ids of closed
    // pull requests if a long-running intake ever needs it.
    let mut published = HashSet::new();
    loop {
        match fetch(&repos, &cursors) {
            // A publish error ends the intake: its restart recovers the source it left behind.
            Ok(polled) => publish_all(&bus, &triggers, polled, &mut cursors, &mut published)?,
            Err(error) if once => return Err(error),
            Err(error) => eprintln!("{error}"),
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(EVERY);
    }
}

/// The newest source event per stream, and the baseline position of each stream that finished
/// its baseline, read back newest first.
fn newest(
    bus: &Bus,
    repos: &BTreeSet<&str>,
) -> herdr_bus::Result<(BTreeMap<Stream, Message>, Cursors)> {
    let wanted = repos.len() * STREAMS.len();
    let mut newest = BTreeMap::new();
    let mut baselines = Cursors::new();
    let mut before = None;
    // shortcut: reads back to each stream's baseline marker, once per start; a stored cursor would
    // make this constant once the output is large.
    while baselines.len() < wanted {
        let page = bus.messages(&MessageQuery {
            output: Some(OUTPUT.into()),
            before,
            limit: 500,
            ..MessageQuery::default()
        })?;
        let Some(oldest) = page.last() else {
            break;
        };
        before = Some(oldest.seq);
        for stored in &page {
            let data = &stored.message.data;
            let Some(repo) = data["repo"].as_str().filter(|repo| repos.contains(repo)) else {
                continue;
            };
            match stored.message.kind.as_str() {
                BASELINE => {
                    let Some(kind) = kind(data["stream"].as_str().unwrap_or_default()) else {
                        continue;
                    };
                    let position = data["position"].as_u64().unwrap_or_default();
                    baselines
                        .entry((repo.to_string(), kind))
                        .or_insert(position);
                }
                other => {
                    if let Some(kind) = kind(other) {
                        newest
                            .entry((repo.to_string(), kind))
                            .or_insert_with(|| stored.message.clone());
                    }
                }
            }
        }
    }
    Ok((newest, baselines))
}

/// The stream a message type belongs to: issue events, the issues list, reviews or checks.
fn kind(name: &str) -> Option<&'static str> {
    match name {
        PR_LABELED | PR_CLOSED => Some(LABELED),
        PR_OPENED => Some(OPENED),
        _ => STREAMS.into_iter().find(|kind| *kind == name),
    }
}

/// Where a source event stands in its stream: its issue or pull request number in the issues
/// list, its GitHub id otherwise.
fn position(data: &serde_json::Value, kind: &str) -> u64 {
    let number = data.get("issue").unwrap_or(&data["pull_request"]);
    let field = if kind == OPENED {
        number
    } else {
        &data["github_id"]
    };
    field.as_u64().unwrap_or_default()
}

/// What one poll of a stream found: its new events, oldest first, and the highest position seen.
struct Polled {
    stream: Stream,
    events: Vec<Observed>,
    top: Option<u64>,
}

/// Fetches every repo first, so that a failing `gh` publishes nothing.
fn fetch(repos: &BTreeSet<&str>, cursors: &Cursors) -> Result<Vec<Polled>, String> {
    let mut polled = Vec::new();
    for repo in repos {
        polled.extend(fetch_repo(repo, cursors)?);
    }
    Ok(polled)
}

fn publish_all(
    bus: &Bus,
    triggers: &[(&str, &Trigger)],
    polled: Vec<Polled>,
    cursors: &mut Cursors,
    published: &mut HashSet<Uuid>,
) -> Result<(), String> {
    for Polled {
        stream,
        events,
        top,
    } in polled
    {
        for event in events {
            let source = event.message(triggers)?;
            // Reviews and checks are read again on every poll; herdr-bus would drop them too.
            if !published.insert(source.id) {
                continue;
            }
            publish(bus, &source)?;
            dispatch(bus, triggers, &source)?;
        }
        let cursor = cursors.get(&stream).copied();
        let top = top.max(cursor).unwrap_or_default();
        if cursor.is_none() {
            publish(bus, &baseline(&stream, top))?;
        }
        cursors.insert(stream, top);
    }
    Ok(())
}

/// Marks the end of a stream's baseline, even when its first page held no event.
fn baseline((repo, kind): &Stream, position: u64) -> Message {
    let data = json!({ "repo": repo, "stream": kind, "position": position });
    Message::new(BASELINE, data)
        .id(id(&format!("{BASELINE}:{repo}:{kind}")))
        .subject(format!("{repo} {kind}"))
}

/// Publishes a task request for each trigger the source names, among those still defined.
fn dispatch(bus: &Bus, triggers: &[(&str, &Trigger)], source: &Message) -> Result<(), String> {
    let named = source.data["triggers"].as_array().into_iter().flatten();
    for name in named.filter_map(|name| name.as_str()) {
        if let Some((name, trigger)) = triggers.iter().find(|(other, _)| *other == name) {
            publish(bus, &task_requested(name, trigger, source))?;
        }
    }
    Ok(())
}

fn publish(bus: &Bus, message: &Message) -> Result<(), String> {
    bus.publish(OUTPUT, message)
        .map(drop)
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))
}

/// One GitHub event the intake publishes, oldest first within its stream.
struct Observed {
    kind: &'static str,
    /// The name of its UUIDv5.
    name: String,
    repo: String,
    number: u64,
    label: Option<String>,
    /// From the first poll of its stream, which requests tasks only for `backfill` triggers.
    baseline: bool,
    /// Every field but `baseline` and `triggers`.
    data: serde_json::Value,
}

impl Observed {
    fn message(&self, triggers: &[(&str, &Trigger)]) -> Result<Message, String> {
        let raw = self.data["time"].as_str().unwrap_or_default();
        let time: Timestamp = raw
            .parse()
            .map_err(|error| format!("GitHub time `{raw}`: {error}"))?;
        let mut data = self.data.clone();
        data["baseline"] = json!(self.baseline);
        data["triggers"] = json!(self.matching(triggers));
        let url = self.data["url"].as_str().unwrap_or_default().to_string();
        let link = if self.kind.starts_with("github.issues.") {
            "issue"
        } else {
            "pull_request"
        };
        Ok(Message::new(self.kind, data)
            .id(id(&self.name))
            .subject(format!("{}#{}", self.repo, self.number))
            .time(time)
            .link(link, &url))
    }

    fn matching<'a>(&self, triggers: &[(&'a str, &Trigger)]) -> Vec<&'a str> {
        let matches = |trigger: &Trigger| {
            let label = trigger
                .filter
                .as_ref()
                .and_then(|filter| filter.get("label"));
            trigger.repo.as_deref() == Some(self.repo.as_str())
                && format!("github.{}", trigger.on) == self.kind
                && label.is_none_or(|label| self.label.as_ref() == Some(label))
                && (!self.baseline || trigger.backfill == Some(true))
        };
        triggers
            .iter()
            .filter(|(_, trigger)| matches(trigger))
            .map(|(name, _)| *name)
            .collect()
    }
}

fn task_requested(name: &str, trigger: &Trigger, source: &Message) -> Message {
    let data = &source.data;
    let number = data.get("issue").unwrap_or(&data["pull_request"]);
    let key = format!(
        "github:{}#{number}",
        data["repo"].as_str().unwrap_or_default()
    );
    let data = json!({
        "workflow": trigger.workflow,
        "key": key,
        "band": trigger.band,
        "lane": trigger.lane,
        "trigger": name,
        "source": source.id.to_string(),
    });
    Message::new(TASK_REQUESTED, data)
        .id(id(&format!("{TASK_REQUESTED}:{name}:{}", source.id)))
        .subject(key)
        .time(source.time)
}

/// UUIDv5 in Priory's namespace, so a retried publish keeps its id.
pub(crate) fn id(name: &str) -> Uuid {
    let namespace = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        b"https://github.com/maxence-lefebvre/priory/intake",
    );
    Uuid::new_v5(&namespace, name.as_bytes())
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct Issue {
    id: u64,
    number: u64,
    title: String,
    html_url: String,
    user: Option<Login>,
    created_at: String,
    pull_request: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pull {
    number: u64,
    head_ref_name: String,
    head_ref_oid: String,
    base_ref_name: String,
    merged: bool,
    reviews: Nodes<Review>,
    commits: Nodes<Commit>,
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
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

#[derive(Deserialize)]
struct IssueEvent {
    id: u64,
    event: String,
    created_at: String,
    actor: Option<Login>,
    label: Option<Label>,
    issue: Issue,
}

/// One poll of `repo`'s streams. Pull request events need a GraphQL query for their branches.
fn fetch_repo(repo: &str, cursors: &Cursors) -> Result<Vec<Polled>, String> {
    let cursor = |kind| cursors.get(&(repo.to_string(), kind)).copied();
    let (events, events_top): (Vec<IssueEvent>, _) = pages(
        &format!("repos/{repo}/issues/events?"),
        cursor(LABELED),
        |event: &IssueEvent| event.id,
    )?;
    let (issues, issues_top): (Vec<Issue>, _) = pages(
        &format!("repos/{repo}/issues?state=all&sort=created&direction=desc&"),
        cursor(OPENED),
        |issue: &Issue| issue.number,
    )?;
    let numbers: BTreeSet<u64> = events
        .iter()
        .map(|event| &event.issue)
        .chain(&issues)
        .filter(|issue| issue.pull_request.is_some())
        .map(|issue| issue.number)
        .collect();
    let pulls = pull_requests(repo, &numbers)?;
    let mut last_close = BTreeMap::new();
    for event in events.iter().filter(|event| event.event == "closed") {
        last_close.insert(event.issue.number, event.id);
    }
    let source = |kind, baseline, issue: &Issue, id: u64, actor: Option<&Login>, time: &str| {
        let mut data = json!({
            "repo": repo,
            "title": issue.title,
            "url": issue.html_url,
            "actor": actor.map(|actor| &actor.login),
            "time": time,
            "github_id": id,
        });
        let number = issue.number;
        if issue.pull_request.is_none() {
            data["issue"] = json!(number);
        } else {
            let pull = pulls.get(&number);
            data["pull_request"] = json!(number);
            data["head_branch"] = json!(pull.map(|pull| &pull.head_ref_name));
            data["head_sha"] = json!(pull.map(|pull| &pull.head_ref_oid));
            data["base_branch"] = json!(pull.map(|pull| &pull.base_ref_name));
        }
        Observed {
            kind,
            name: String::new(),
            repo: repo.to_string(),
            number,
            label: None,
            baseline,
            data,
        }
    };
    let labeled = events
        .iter()
        .filter_map(|event| {
            let pull_request = event.issue.pull_request.is_some();
            let kind = match event.event.as_str() {
                "labeled" if pull_request => PR_LABELED,
                "labeled" => LABELED,
                "closed" if pull_request => PR_CLOSED,
                _ => return None,
            };
            let mut observed = source(
                kind,
                cursor(LABELED).is_none(),
                &event.issue,
                event.id,
                event.actor.as_ref(),
                &event.created_at,
            );
            observed.name = format!("github:{repo}:issues/events:{}", event.id);
            if kind == PR_CLOSED {
                // The pull request's state is today's: only its last close can be the merge.
                let last = last_close.get(&event.issue.number) == Some(&event.id);
                let pull = pulls.get(&event.issue.number);
                observed.data["merged"] = json!(pull.map(|pull| pull.merged && last));
            } else {
                observed.label = event.label.as_ref().map(|label| label.name.clone());
                observed.data["label"] = json!(observed.label);
            }
            Some(observed)
        })
        .collect();
    let opened = issues
        .iter()
        .map(|issue| {
            let kind = if issue.pull_request.is_none() {
                OPENED
            } else {
                PR_OPENED
            };
            let mut observed = source(
                kind,
                cursor(OPENED).is_none(),
                issue,
                issue.id,
                issue.user.as_ref(),
                &issue.created_at,
            );
            observed.name = format!("github:{repo}:issues:{}:opened", issue.id);
            observed
        })
        .collect();
    let open = open_pulls(repo)?;
    let (reviews, checks) = reviews_and_checks(repo, open.iter().chain(pulls.values()), cursor);
    let top = |events: &[Observed]| {
        events
            .iter()
            .filter_map(|event| event.data["github_id"].as_u64())
            .max()
    };
    // Reviews and checks first: a crash after a close would end their polling.
    Ok(vec![
        Polled {
            stream: (repo.to_string(), REVIEWED),
            top: top(&reviews),
            events: reviews,
        },
        Polled {
            stream: (repo.to_string(), CHECKED),
            top: top(&checks),
            events: checks,
        },
        Polled {
            stream: (repo.to_string(), LABELED),
            events: labeled,
            top: events_top,
        },
        Polled {
            stream: (repo.to_string(), OPENED),
            events: opened,
            top: issues_top,
        },
    ])
}

/// The submitted reviews and completed check runs of `pulls`, oldest first.
fn reviews_and_checks<'a>(
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
        let contexts = pages.iter().flat_map(|page| &page.nodes);
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
            checks.push(observed);
        }
    }
    reviews.sort_by_key(|review| review.data["github_id"].as_u64());
    checks.sort_by_key(|check| check.data["github_id"].as_u64());
    (reviews, checks)
}

const PULL_FIELDS: &str = "fragment pull on PullRequest { number headRefName headRefOid baseRefName merged \
    reviews(last: 50) { nodes { databaseId state submittedAt url author { login } } } \
    commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { totalCount nodes { \
    __typename ... on CheckRun { databaseId name status conclusion completedAt url checkSuite { app { slug } } } \
    } } } } } } }";

/// The open pull requests of `repo`, 100 per page.
fn open_pulls(repo: &str) -> Result<Vec<Pull>, String> {
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
fn pull_requests(repo: &str, numbers: &BTreeSet<u64>) -> Result<BTreeMap<u64, Pull>, String> {
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

/// Newest-first pages until one reaches `cursor`: the items after `cursor`, oldest first, and the
/// highest position seen. Without a cursor, only the first page: older history is not published.
fn pages<T: DeserializeOwned>(
    path: &str,
    cursor: Option<u64>,
    order: impl Fn(&T) -> u64,
) -> Result<(Vec<T>, Option<u64>), String> {
    let mut items: Vec<T> = Vec::new();
    for page in 1.. {
        let batch: Vec<T> = gh(&format!("{path}per_page={PAGE}&page={page}"), &[])?;
        let last = batch.len() < PAGE
            || cursor.is_none_or(|cursor| batch.iter().any(|item| order(item) <= cursor));
        items.extend(batch);
        if last {
            break;
        }
    }
    let top = items.iter().map(&order).max();
    items.retain(|item| cursor.is_none_or(|cursor| order(item) > cursor));
    items.sort_by_key(&order);
    Ok((items, top))
}

fn gh<T: DeserializeOwned>(path: &str, fields: &[&str]) -> Result<T, String> {
    let output = Command::new("gh")
        .args(["api", path])
        .args(fields)
        .output()
        .map_err(|error| format!("gh api {path}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api {path} failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("gh api {path}: {error}"))
}
