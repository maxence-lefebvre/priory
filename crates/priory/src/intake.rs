use crate::factory::{Factory, Trigger};
use herdr_bus::{Bus, Message, MessageQuery, Output, Timestamp, Uuid};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Duration;

pub const OUTPUT: &str = "factory.intake";
const LABELED: &str = "github.issues.labeled";
const OPENED: &str = "github.issues.opened";
const TASK_REQUESTED: &str = "factory.task.requested";
const BASELINE: &str = "factory.intake.baseline";
const PAGE: usize = 100;
const EVERY: Duration = Duration::from_secs(60);

/// A repo and the source message type polled for it.
type Stream = (String, &'static str);

/// Where polling stopped, per stream: the highest issue event id for labeled events, the highest
/// issue number for opened issues. A stream without one is still in its baseline.
type Cursors = BTreeMap<Stream, u64>;

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
    bus.declare(
        &Output::new(OUTPUT)
            .describe("Priory intake: source events and the tasks they request")
            .types([LABELED, OPENED, TASK_REQUESTED, BASELINE]),
    )
    .map_err(|error| error.to_string())?;
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
    loop {
        match fetch(&repos, &cursors) {
            // A publish error ends the intake: its restart recovers the source it left behind.
            Ok(polled) => publish_all(&bus, &triggers, polled, &mut cursors)?,
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
    let wanted = repos.len() * 2;
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

fn kind(name: &str) -> Option<&'static str> {
    [LABELED, OPENED].into_iter().find(|kind| *kind == name)
}

/// Where a source event stands in its stream: its issue event id, or its issue number.
fn position(data: &serde_json::Value, kind: &str) -> u64 {
    let field = if kind == OPENED { "issue" } else { "github_id" };
    data[field].as_u64().unwrap_or_default()
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
        for kind in [LABELED, OPENED] {
            let stream = (repo.to_string(), kind);
            let cursor = cursors.get(&stream).copied();
            let (events, top) = if kind == LABELED {
                labeled(repo, cursor)?
            } else {
                opened(repo, cursor)?
            };
            polled.push(Polled {
                stream,
                events,
                top,
            });
        }
    }
    Ok(polled)
}

fn publish_all(
    bus: &Bus,
    triggers: &[(&str, &Trigger)],
    polled: Vec<Polled>,
    cursors: &mut Cursors,
) -> Result<(), String> {
    for Polled {
        stream,
        events,
        top,
    } in polled
    {
        for event in events {
            let source = event.message(triggers)?;
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

/// One GitHub event the intake publishes, oldest first within its type.
struct Observed {
    kind: &'static str,
    github_id: u64,
    repo: String,
    issue: Issue,
    label: Option<String>,
    actor: Option<String>,
    time: String,
    /// From the first poll of its repo and type, which requests tasks only for `backfill` triggers.
    baseline: bool,
}

impl Observed {
    fn message(&self, triggers: &[(&str, &Trigger)]) -> Result<Message, String> {
        let time: Timestamp = self
            .time
            .parse()
            .map_err(|error| format!("GitHub time `{}`: {error}", self.time))?;
        let mut data = json!({
            "repo": self.repo,
            "issue": self.issue.number,
            "title": self.issue.title,
            "url": self.issue.html_url,
            "actor": self.actor,
            "time": self.time,
            "github_id": self.github_id,
            "baseline": self.baseline,
            "triggers": self.matching(triggers),
        });
        if let Some(label) = &self.label {
            data["label"] = json!(label);
        }
        let name = match self.kind {
            LABELED => format!("github:{}:issues/events:{}", self.repo, self.github_id),
            _ => format!("github:{}:issues:{}:opened", self.repo, self.github_id),
        };
        Ok(Message::new(self.kind, data)
            .id(id(&name))
            .subject(format!("{}#{}", self.repo, self.issue.number))
            .time(time)
            .link("issue", &self.issue.html_url))
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
    let key = format!(
        "github:{}#{}",
        source.data["repo"].as_str().unwrap_or_default(),
        source.data["issue"]
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
fn id(name: &str) -> Uuid {
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
struct IssueEvent {
    id: u64,
    event: String,
    created_at: String,
    actor: Option<Login>,
    label: Option<Label>,
    issue: Issue,
}

fn labeled(repo: &str, cursor: Option<u64>) -> Result<(Vec<Observed>, Option<u64>), String> {
    let (events, top): (Vec<IssueEvent>, _) = pages(
        &format!("repos/{repo}/issues/events?"),
        cursor,
        |event: &IssueEvent| event.id,
    )?;
    let observed = events
        .into_iter()
        .filter(|event| event.event == "labeled" && event.issue.pull_request.is_none())
        .map(|event| Observed {
            kind: LABELED,
            github_id: event.id,
            repo: repo.to_string(),
            label: event.label.map(|label| label.name),
            actor: event.actor.map(|actor| actor.login),
            time: event.created_at,
            issue: event.issue,
            baseline: cursor.is_none(),
        })
        .collect();
    Ok((observed, top))
}

fn opened(repo: &str, cursor: Option<u64>) -> Result<(Vec<Observed>, Option<u64>), String> {
    let (issues, top): (Vec<Issue>, _) = pages(
        &format!("repos/{repo}/issues?state=all&sort=created&direction=desc&"),
        cursor,
        |issue: &Issue| issue.number,
    )?;
    let observed = issues
        .into_iter()
        .filter(|issue| issue.pull_request.is_none())
        .map(|issue| Observed {
            kind: OPENED,
            github_id: issue.id,
            repo: repo.to_string(),
            label: None,
            actor: issue.user.as_ref().map(|user| user.login.clone()),
            time: issue.created_at.clone(),
            issue,
            baseline: cursor.is_none(),
        })
        .collect();
    Ok((observed, top))
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
        let batch: Vec<T> = gh(&format!("{path}per_page={PAGE}&page={page}"))?;
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

fn gh<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let output = Command::new("gh")
        .args(["api", path])
        .output()
        .map_err(|error| format!("gh api {path}: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh api {path} failed: {}", stderr.trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("gh api {path}: {error}"))
}
