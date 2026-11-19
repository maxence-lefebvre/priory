use herdr_bus::{Bus, Message, MessageQuery, Output, Timestamp, Uuid};
use priory_factory::factory::{Factory, Trigger};
mod gh;
mod issues;
mod linear;
mod pulls;
mod schedule;
mod slack;

use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;

pub const OUTPUT: &str = "factory.intake";
const LABELED: &str = "github.issues.labeled";
const OPENED: &str = "github.issues.opened";
const PR_OPENED: &str = "github.pull_request.opened";
const PR_LABELED: &str = "github.pull_request.labeled";
pub const PR_CLOSED: &str = "github.pull_request.closed";
const REVIEWED: &str = "github.pull_request_review.submitted";
pub const CHECKED: &str = "github.check_run.completed";
pub const CHECK_RUNS: &str = "github.pull_request.check_runs";
/// The check run conclusions that count as green, lowercase as a check event carries them.
pub const PASSING: [&str; 3] = ["success", "neutral", "skipped"];
/// The streams of a repo: the message type each is named after.
const STREAMS: [&str; 4] = [LABELED, OPENED, REVIEWED, CHECKED];
pub(crate) const TASK_REQUESTED: &str = "factory.task.requested";
const BASELINE: &str = "factory.intake.baseline";
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
            CHECK_RUNS,
            crate::request::REQUESTED,
            schedule::FIRED,
            linear::ENTERED,
            linear::LEFT,
            linear::CREATED,
            linear::STATE_CHANGED,
            linear::BLOCKERS,
            linear::COMMENTED,
            slack::POSTED,
            slack::REPLIED,
            TASK_REQUESTED,
            BASELINE,
        ])
}

/// A resource an open wait names, `linear:<issue>`, `slack:<channel>:<thread ts>` or
/// `github:<owner/name>#<number>`, and the time of its agent's last prompt: the intake publishes
/// what happens to it from then on.
pub struct Watch {
    pub resource: String,
    pub since: Timestamp,
}

/// Polls the sources until stopped, or once. `watched` reads the open waits on each poll, so that
/// the intake polls the Slack threads, Linear issues and pull request repositories they name.
pub fn run(
    factory: &Factory,
    once: bool,
    watched: fn() -> Result<Vec<Watch>, String>,
) -> Result<(), String> {
    priory_runtime::runtime::bound_commands(factory.command_timeout());
    let from = |source: &str| -> Vec<(&str, &Trigger)> {
        factory
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.value.source == source)
            .map(|(name, trigger)| (name.as_str(), &trigger.value))
            .collect()
    };
    let (triggers, schedules) = (from("github"), from("schedule"));
    // Any trigger's task can open a pull request whose checks and reviews its waits need.
    let repos: BTreeSet<&str> = factory
        .triggers
        .values()
        .filter_map(|trigger| trigger.value.repo.as_deref())
        .collect();
    let linear_triggers = [from("linear"), from("linear.view")].concat();
    let slack_triggers = from("slack");
    let bus = Bus::open_default().map_err(|error| error.to_string())?;
    bus.declare(&output()).map_err(|error| error.to_string())?;
    let mut linear = (!linear_triggers.is_empty())
        .then(|| linear::Linear::start(&bus, linear_triggers.clone()))
        .transpose()?;
    let mut slack = (!slack_triggers.is_empty())
        .then(|| slack::Slack::start(&bus, slack_triggers.clone()))
        .transpose()?;
    // The loop reports a failing read of the waits; here it only costs those repos their cursors.
    let waiting = watched().unwrap_or_default();
    let resumed: BTreeSet<&str> = repos
        .iter()
        .copied()
        .chain(waiting.iter().filter_map(pull_request_repo))
        .collect();
    let (newest, mut cursors) = newest(&bus, &resumed).map_err(|error| error.to_string())?;
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
        let watching = match watched() {
            Ok(watching) => watching,
            Err(error) if once => return Err(error),
            Err(error) => {
                eprintln!("{error}");
                Vec::new()
            }
        };
        let names = |source: &str| {
            watching
                .iter()
                .any(|watch| watch.resource.starts_with(source))
        };
        // A wait starts a source the triggers left off; a missing key fails that poll only.
        if linear.is_none() && names("linear:") {
            match linear::Linear::start(&bus, Vec::new()) {
                Ok(started) => linear = Some(started),
                Err(error) if once => return Err(error),
                Err(error) => eprintln!("{error}"),
            }
        }
        if slack.is_none() && names("slack:") {
            match slack::Slack::start(&bus, Vec::new()) {
                Ok(started) => slack = Some(started),
                Err(error) if once => return Err(error),
                Err(error) => eprintln!("{error}"),
            }
        }
        schedule::fire(&bus, &schedules, now()?)?;
        let waited = watching.iter().filter_map(pull_request_repo);
        let polled: BTreeSet<&str> = repos.iter().copied().chain(waited).collect();
        let (found, failed) = fetch(&polled, &cursors);
        // A publish error ends the intake: its restart recovers the source it left behind.
        publish_all(&bus, &triggers, found, &mut cursors, &mut published)?;
        for error in failed {
            if once {
                return Err(error);
            }
            eprintln!("{error}");
        }
        if let Some(linear) = &mut linear {
            match linear.fetch() {
                Ok(polled) => linear.publish_all(&bus, polled)?,
                Err(error) if once => return Err(error),
                Err(error) => eprintln!("{error}"),
            }
        }
        if let Some(slack) = &mut slack {
            match slack.poll(&bus, &watching) {
                Ok(()) => {}
                Err(error) if once => return Err(error),
                Err(error) => eprintln!("{error}"),
            }
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(EVERY);
    }
}

/// The repository of a pull request wait, `owner/name` from `github:owner/name#42`.
fn pull_request_repo(watch: &Watch) -> Option<&str> {
    let pull = watch.resource.strip_prefix("github:")?;
    pull.split(['#', '@']).next()
}

/// The time, or `PRIORY_NOW` when set, so a test can stand at any time.
fn now() -> Result<Timestamp, String> {
    match std::env::var("PRIORY_NOW") {
        Ok(now) => now
            .parse()
            .map_err(|error| format!("PRIORY_NOW `{now}`: {error}")),
        Err(_) => Ok(Timestamp::now()),
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

/// Fetches every repo, and the error of each that fails: a failing `gh` publishes nothing for its
/// repo, and the others still publish.
fn fetch(repos: &BTreeSet<&str>, cursors: &Cursors) -> (Vec<Polled>, Vec<String>) {
    let mut polled = Vec::new();
    let mut failed = Vec::new();
    for repo in repos {
        match issues::fetch_repo(repo, cursors) {
            Ok(found) => polled.extend(found),
            Err(error) => failed.push(error),
        }
    }
    (polled, failed)
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
            let data = &source.data;
            let number = data.get("issue").unwrap_or(&data["pull_request"]);
            let key = match (number.as_str(), data["channel"].as_str()) {
                (_, Some(channel)) => format!(
                    "slack:{channel}:{}",
                    data["ts"].as_str().unwrap_or_default()
                ),
                (Some(identifier), _) => format!("linear:{identifier}"),
                (None, None) => format!(
                    "github:{}#{number}",
                    data["repo"].as_str().unwrap_or_default()
                ),
            };
            publish(bus, &task_requested(name, trigger, source, key))?;
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

fn task_requested(name: &str, trigger: &Trigger, source: &Message, key: String) -> Message {
    let mut data = json!({
        "workflow": trigger.workflow,
        "key": key,
        "band": trigger.band,
        "lane": trigger.lane,
        "trigger": name,
        "source": source.id.to_string(),
    });
    for field in ["team", "project"] {
        if let Some(value) = source.data.get(field) {
            data[field] = value.clone();
        }
    }
    if let Some(hold) = &trigger.hold {
        data["hold"] = json!(hold);
    }
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
