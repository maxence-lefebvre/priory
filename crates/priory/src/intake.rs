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
const PAGE: usize = 100;
const EVERY: Duration = Duration::from_secs(60);

/// Where polling stopped, per repo and message type: the highest issue event id for labeled
/// events, the highest issue number for opened issues.
type Cursors = BTreeMap<(String, &'static str), u64>;

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
            .types([LABELED, OPENED, TASK_REQUESTED]),
    )
    .map_err(|error| error.to_string())?;
    let newest = newest(&bus, &repos).map_err(|error| error.to_string())?;
    let mut cursors = Cursors::new();
    for ((repo, kind), source) in newest {
        // A crash may have stopped between this source and its task requests.
        dispatch(&bus, &triggers, &source)?;
        cursors.insert((repo, kind), position(&source.data, kind));
    }
    loop {
        match fetch(&repos, &cursors) {
            // A publish error ends the intake: its restart recovers the source it left behind.
            Ok(observed) => publish_all(&bus, &triggers, observed, &mut cursors)?,
            Err(error) if once => return Err(error),
            Err(error) => eprintln!("{error}"),
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(EVERY);
    }
}

/// The newest source event published per repo and type, read back newest first.
fn newest(
    bus: &Bus,
    repos: &BTreeSet<&str>,
) -> herdr_bus::Result<BTreeMap<(String, &'static str), Message>> {
    let wanted = repos.len() * 2;
    let mut newest = BTreeMap::new();
    let mut before = None;
    // shortcut: a repo with no event of one type makes this read the whole output, once per start.
    while newest.len() < wanted {
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
            let kind = match stored.message.kind.as_str() {
                LABELED => LABELED,
                OPENED => OPENED,
                _ => continue,
            };
            let Some(repo) = stored.message.data["repo"].as_str() else {
                continue;
            };
            if repos.contains(repo) {
                newest
                    .entry((repo.to_string(), kind))
                    .or_insert_with(|| stored.message.clone());
            }
        }
    }
    Ok(newest)
}

/// Where a source event stands in its stream: its issue event id, or its issue number.
fn position(data: &serde_json::Value, kind: &str) -> u64 {
    let field = if kind == OPENED { "issue" } else { "github_id" };
    data[field].as_u64().unwrap_or_default()
}

/// Fetches every repo first, so that a failing `gh` publishes nothing.
fn fetch(repos: &BTreeSet<&str>, cursors: &Cursors) -> Result<Vec<Observed>, String> {
    let mut observed = Vec::new();
    for repo in repos {
        observed.extend(labeled(
            repo,
            cursors.get(&(repo.to_string(), LABELED)).copied(),
        )?);
        observed.extend(opened(
            repo,
            cursors.get(&(repo.to_string(), OPENED)).copied(),
        )?);
    }
    Ok(observed)
}

fn publish_all(
    bus: &Bus,
    triggers: &[(&str, &Trigger)],
    observed: Vec<Observed>,
    cursors: &mut Cursors,
) -> Result<(), String> {
    for event in observed {
        let source = event.message()?;
        publish(bus, &source)?;
        dispatch(bus, triggers, &source)?;
        cursors.insert((event.repo, event.kind), position(&source.data, event.kind));
    }
    Ok(())
}

/// Publishes one task request per trigger that matches `source`.
fn dispatch(bus: &Bus, triggers: &[(&str, &Trigger)], source: &Message) -> Result<(), String> {
    let data = &source.data;
    for (name, trigger) in triggers {
        let label = trigger
            .filter
            .as_ref()
            .and_then(|filter| filter.get("label"));
        let matches = trigger.repo.as_deref() == data["repo"].as_str()
            && format!("github.{}", trigger.on) == source.kind
            && label.is_none_or(|label| data["label"].as_str() == Some(label));
        if matches {
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
}

impl Observed {
    fn message(&self) -> Result<Message, String> {
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

fn labeled(repo: &str, cursor: Option<u64>) -> Result<Vec<Observed>, String> {
    let events: Vec<IssueEvent> = pages(
        &format!("repos/{repo}/issues/events?"),
        cursor,
        |event: &IssueEvent| event.id,
    )?;
    Ok(events
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
        })
        .collect())
}

fn opened(repo: &str, cursor: Option<u64>) -> Result<Vec<Observed>, String> {
    let issues: Vec<Issue> = pages(
        &format!("repos/{repo}/issues?state=all&sort=created&direction=desc&"),
        cursor,
        |issue: &Issue| issue.number,
    )?;
    Ok(issues
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
        })
        .collect())
}

/// Newest-first pages until one reaches `cursor`, returned oldest first from `cursor` on.
/// Without a cursor, only the first page: older history is not published.
fn pages<T: DeserializeOwned>(
    path: &str,
    cursor: Option<u64>,
    order: impl Fn(&T) -> u64,
) -> Result<Vec<T>, String> {
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
    items.retain(|item| cursor.is_none_or(|cursor| order(item) > cursor));
    items.sort_by_key(&order);
    Ok(items)
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
