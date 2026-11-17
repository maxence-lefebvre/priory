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
    let mut cursors = cursors(&bus, &repos).map_err(|error| error.to_string())?;
    loop {
        let polled = poll(&bus, &triggers, &repos, &mut cursors);
        if once {
            return polled;
        }
        if let Err(error) = polled {
            eprintln!("{error}");
        }
        std::thread::sleep(EVERY);
    }
}

/// Reads the cursors back from what the intake already published, newest first.
fn cursors(bus: &Bus, repos: &BTreeSet<&str>) -> herdr_bus::Result<Cursors> {
    let wanted = repos.len() * 2;
    let mut cursors = Cursors::new();
    let mut before = None;
    while cursors.len() < wanted {
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
            let data = &stored.message.data;
            let position = if kind == OPENED { "issue" } else { "github_id" };
            let (Some(repo), Some(id)) = (data["repo"].as_str(), data[position].as_u64()) else {
                continue;
            };
            if repos.contains(repo) {
                let cursor = cursors.entry((repo.to_string(), kind)).or_default();
                *cursor = (*cursor).max(id);
            }
        }
    }
    Ok(cursors)
}

/// Fetches every repo first, so that a failing `gh` publishes nothing.
fn poll(
    bus: &Bus,
    triggers: &[(&str, &Trigger)],
    repos: &BTreeSet<&str>,
    cursors: &mut Cursors,
) -> Result<(), String> {
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
    for event in observed {
        let source = event.message()?;
        let publish = |message: &Message| {
            bus.publish(OUTPUT, message)
                .map_err(|error| format!("publish on {OUTPUT}: {error}"))
        };
        publish(&source)?;
        for (name, trigger) in triggers.iter().filter(|(_, t)| event.matches(t)) {
            publish(&task_requested(name, trigger, &event, &source))?;
        }
        cursors.insert((event.repo.clone(), event.kind), event.position());
    }
    Ok(())
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

    fn position(&self) -> u64 {
        if self.kind == OPENED {
            return self.issue.number;
        }
        self.github_id
    }

    fn matches(&self, trigger: &Trigger) -> bool {
        let on = format!("github.{}", trigger.on);
        let label = trigger
            .filter
            .as_ref()
            .and_then(|filter| filter.get("label"));
        trigger.repo.as_deref() == Some(self.repo.as_str())
            && on == self.kind
            && label.is_none_or(|label| self.label.as_ref() == Some(label))
    }
}

fn task_requested(name: &str, trigger: &Trigger, event: &Observed, source: &Message) -> Message {
    let key = format!("github:{}#{}", event.repo, event.issue.number);
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
    items.retain(|item| cursor.is_none_or(|cursor| order(item) >= cursor));
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
