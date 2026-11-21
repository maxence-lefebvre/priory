use herdr_bus::{Bus, Message, MessageQuery, Output, Timestamp, Uuid};
use priory_factory::factory::{Factory, Trigger};
mod gh;
mod issues;
mod linear;
mod pulls;
mod reviews;
pub mod schedule;
mod slack;
mod webhook;

pub use linear::state as linear_state;
pub use webhook::Github;

use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::Duration;

pub const OUTPUT: &str = "factory.intake";
const LABELED: &str = "github.issues.labeled";
const OPENED: &str = "github.issues.opened";
pub const ISSUE_CLOSED: &str = "github.issues.closed";
pub const ISSUE_COMMENTED: &str = "github.issues.comment";
const PR_OPENED: &str = "github.pull_request.opened";
const PR_LABELED: &str = "github.pull_request.labeled";
pub const PR_CLOSED: &str = "github.pull_request.closed";
pub const REVIEWED: &str = "github.pull_request_review.submitted";
pub const COMMENTED: &str = "github.pull_request.comment";
pub const THREAD_COMMENTED: &str = "github.pull_request_review_comment";
pub const CHECKED: &str = "github.check_run.completed";
pub const CHECK_RUNS: &str = "github.pull_request.check_runs";
/// The check run conclusions that count as green, lowercase as a check event carries them.
pub const PASSING: [&str; 3] = ["success", "neutral", "skipped"];
/// The streams of a repo: the message type each is named after.
const STREAMS: [&str; 4] = [LABELED, OPENED, REVIEWED, CHECKED];
pub(crate) const TASK_REQUESTED: &str = "factory.task.requested";
const BASELINE: &str = "factory.intake.baseline";
const EVERY: Duration = Duration::from_secs(60);
/// How far back a writer looks for the other writer's report of the same change.
const LOOKBACK_SECONDS: i64 = 7 * 24 * 60 * 60;
const LOOKBACK_MESSAGES: usize = 5000;

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
            ISSUE_CLOSED,
            ISSUE_COMMENTED,
            PR_OPENED,
            PR_LABELED,
            PR_CLOSED,
            REVIEWED,
            COMMENTED,
            THREAD_COMMENTED,
            CHECKED,
            CHECK_RUNS,
            reviews::REQUESTED,
            reviews::REMOVED,
            crate::request::REQUESTED,
            schedule::FIRED,
            linear::ENTERED,
            linear::LEFT,
            linear::VIEW_CHANGED,
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

/// What one poll found of each source it polled, `github`, `linear` or `slack`: none when the poll
/// succeeded, else its errors joined by `; `.
pub type Outcomes<'a> = [(&'a str, Option<String>)];

/// Polls the sources until stopped, or once. `watched` reads the open waits on each poll, so that
/// the intake polls the Slack threads, Linear issues and pull request repositories they name.
/// `health` records each poll's outcomes at the poll's time.
pub fn run(
    factory: &Factory,
    once: bool,
    watched: fn() -> Result<Vec<Watch>, String>,
    health: fn(&Outcomes, Timestamp) -> Result<(), String>,
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
    let review_triggers: Vec<(&str, &Trigger)> = triggers
        .iter()
        .copied()
        .filter(|(_, trigger)| trigger.reviewer.is_some())
        .collect();
    let bus = Bus::open_default().map_err(|error| error.to_string())?;
    bus.declare(&output()).map_err(|error| error.to_string())?;
    // A source the triggers need that cannot start, such as one without its key, ends the intake;
    // recording it first lets the desk say why while systemd restarts it.
    let started = |source: &str, error: String| {
        if let Err(unrecorded) = now().and_then(|at| health(&[(source, Some(error.clone()))], at)) {
            eprintln!("{unrecorded}");
        }
        error
    };
    let mut linear = (!linear_triggers.is_empty())
        .then(|| linear::Linear::start(&bus, linear_triggers.clone()))
        .transpose()
        .map_err(|error| started("linear", error))?;
    let mut reviews = (!review_triggers.is_empty())
        .then(|| reviews::Reviews::start(&bus, review_triggers))
        .transpose()
        .map_err(|error| started("github", error))?;
    let mut slack = (!slack_triggers.is_empty())
        .then(|| slack::Slack::start(&bus, slack_triggers.clone()))
        .transpose()
        .map_err(|error| started("slack", error))?;
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
        // Each source's errors this poll; a wait starts a source the triggers left off, and a
        // missing key fails that poll only.
        let mut failed: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        if linear.is_none() && names("linear:") {
            match linear::Linear::start(&bus, Vec::new()) {
                Ok(started) => linear = Some(started),
                Err(error) => failed.entry("linear").or_default().push(error),
            }
        }
        if slack.is_none() && names("slack:") {
            match slack::Slack::start(&bus, Vec::new()) {
                Ok(started) => slack = Some(started),
                Err(error) => failed.entry("slack").or_default().push(error),
            }
        }
        schedule::fire(&bus, &schedules, now()?)?;
        let waited = watching.iter().filter_map(pull_request_repo);
        let polled: BTreeSet<&str> = repos.iter().copied().chain(waited).collect();
        let (found, errors) = fetch(&polled, &cursors);
        // A publish error ends the intake: its restart recovers the source it left behind.
        publish_all(&bus, &triggers, found, &mut cursors, &mut published)?;
        failed.entry("github").or_default().extend(errors);
        if let Some(reviews) = &mut reviews {
            let errors = reviews.poll(&bus, now()?)?;
            failed.entry("github").or_default().extend(errors);
        }
        if let Some(linear) = &mut linear {
            match linear.fetch() {
                Ok(polled) => linear.publish_all(&bus, polled)?,
                Err(error) => failed.entry("linear").or_default().push(error),
            }
        }
        if let Some(Err(error)) = slack.as_mut().map(|slack| slack.poll(&bus, &watching)) {
            failed.entry("slack").or_default().push(error);
        }
        let sources = [
            ("github", true),
            ("linear", linear.is_some()),
            ("slack", slack.is_some()),
        ];
        let outcomes: Vec<(&str, Option<String>)> = sources
            .into_iter()
            .filter(|(source, on)| *on || failed.contains_key(source))
            .map(|(source, _)| {
                let errors = failed.get(source).filter(|errors| !errors.is_empty());
                (source, errors.map(|errors| errors.join("; ")))
            })
            .collect();
        let recorded = health(&outcomes, now()?);
        for error in recorded
            .err()
            .into_iter()
            .chain(failed.into_values().flatten())
        {
            if once {
                return Err(error);
            }
            eprintln!("{error}");
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
pub fn now() -> Result<Timestamp, String> {
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
            // Polling owns its streams' positions; a delivery can arrive out of order.
            if data.get("delivery").is_some() {
                continue;
            }
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
        PR_LABELED | PR_CLOSED | ISSUE_CLOSED => Some(LABELED),
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
    // Taken at the first event a webhook may also report, and held so none publishes meanwhile.
    let mut delivered: Option<(std::fs::File, Vec<Message>)> = None;
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
            if CHANGES.contains(&source.kind.as_str()) {
                let (_, delivered) = match &mut delivered {
                    Some(delivered) => delivered,
                    None => delivered.insert((claim()?, recent(bus)?)),
                };
                // A crash may have stopped the webhook between its event and its task requests.
                if let Some(other) = delivered.iter().find(|other| same_change(other, &source)) {
                    dispatch(bus, triggers, other)?;
                    continue;
                }
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

/// The types whose webhook id cannot be the poll's, since the payload lacks GitHub's issue event
/// id or issue id: each writer compares the change itself with the other's reports.
const CHANGES: [&str; 4] = [LABELED, PR_LABELED, PR_CLOSED, PR_OPENED];

/// An exclusive lock between the intake and webhook deliveries, held from a change match to the
/// publish it allows, and released when dropped.
fn claim() -> Result<std::fs::File, String> {
    let path = Bus::default_path()
        .map_err(|error| error.to_string())?
        .with_extension("changes.lock");
    let lock = std::fs::File::create(&path)
        .and_then(|file| file.lock().map(|()| file))
        .map_err(|error| format!("lock {}: {error}", path.display()))?;
    Ok(lock)
}

/// Whether the output already holds the other writer's report of `message`'s change.
fn reported(bus: &Bus, message: &Message) -> Result<bool, String> {
    if !CHANGES.contains(&message.kind.as_str()) {
        return Ok(false);
    }
    Ok(recent(bus)?.iter().any(|other| same_change(other, message)))
}

/// The source events of `CHANGES` types on the output, newest first, back 7 days and at most
/// `LOOKBACK_MESSAGES` messages.
fn recent(bus: &Bus) -> Result<Vec<Message>, String> {
    let since = Timestamp::now().as_second() - LOOKBACK_SECONDS;
    let mut found = Vec::new();
    let mut before = None;
    let mut read = 0;
    while read < LOOKBACK_MESSAGES {
        let page = bus
            .messages(&MessageQuery {
                output: Some(OUTPUT.into()),
                before,
                limit: 500,
                ..MessageQuery::default()
            })
            .map_err(|error| format!("read {OUTPUT}: {error}"))?;
        let Some(oldest) = page.last() else {
            break;
        };
        before = Some(oldest.seq);
        read += page.len();
        let old = oldest.published.as_second() < since;
        found.extend(
            page.into_iter()
                .filter(|stored| stored.published.as_second() >= since)
                .map(|stored| stored.message)
                .filter(|message| CHANGES.contains(&message.kind.as_str())),
        );
        if old {
            break;
        }
    }
    Ok(found)
}

/// Whether one writer's `a` and the other's `b` report the same change: same type, issue or pull
/// request and label, and for a label within 60 seconds, for a close at the same second. A pull
/// request opens once.
fn same_change(a: &Message, b: &Message) -> bool {
    let webhook = |message: &Message| message.data.get("delivery").is_some();
    let apart = (a.time.as_second() - b.time.as_second()).abs();
    webhook(a) != webhook(b)
        && a.kind == b.kind
        && a.subject == b.subject
        && a.data["label"] == b.data["label"]
        && match a.kind.as_str() {
            PR_CLOSED => apart == 0,
            PR_OPENED => true,
            _ => apart <= 60,
        }
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
            let repo = data["repo"].as_str().unwrap_or_default();
            let key = match (number.as_str(), data["channel"].as_str()) {
                _ if source.kind == reviews::REQUESTED => {
                    review_key(repo, number, data["reviewer"].as_str().unwrap_or_default())
                }
                (_, Some(channel)) => format!(
                    "slack:{channel}:{}",
                    data["ts"].as_str().unwrap_or_default()
                ),
                (Some(identifier), _) => format!("linear:{identifier}"),
                (None, None) => format!("github:{repo}#{number}"),
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
    // A review request names no repository in its trigger: its task works in the pull request's.
    if source.kind == reviews::REQUESTED {
        data["repo"] = source.data["repo"].clone();
    }
    if let Some(hold) = &trigger.hold {
        data["hold"] = json!(hold);
    }
    Message::new(TASK_REQUESTED, data)
        .id(id(&format!("{TASK_REQUESTED}:{name}:{}", source.id)))
        .subject(key)
        .time(source.time)
}

/// The key of the task that reviews pull request `number` of `repo` for `reviewer`.
pub fn review_key(repo: &str, number: impl std::fmt::Display, reviewer: &str) -> String {
    format!("github:{repo}:pulls/{number}:review:{reviewer}")
}

/// UUIDv5 in Priory's namespace, so a retried publish keeps its id.
pub(crate) fn id(name: &str) -> Uuid {
    let namespace = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        b"https://github.com/maxence-lefebvre/priory/intake",
    );
    Uuid::new_v5(&namespace, name.as_bytes())
}
