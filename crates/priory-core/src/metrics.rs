//! Priory's own metrics, sent as OTLP/HTTP JSON to `<otlp>/v1/metrics` once per interval, under a
//! per-minute cap (docs/daemon/metrics.md).

use crate::otlp;
use crate::room::{CAUSES, RESTORES, Share};
use priory_factory::factory::{Backend, Factory, METRIC_GROUPS, Price};
use priory_intake::intake::{EVENTS, FAILURES, MISSED, OUTPUT};
use priory_intake::log::{self, Log};
use priory_runtime::{room, runtime, ticks, worktrees};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EVERY: Duration = Duration::from_secs(60);
const CAP: u64 = 1000;
const MINUTE: Duration = Duration::from_secs(60);

/// The upper bounds of every histogram's buckets, in seconds; the last bucket holds the rest.
const BOUNDS: [f64; 13] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0,
];

const PROBE: &str = "priory.log.probe.duration";
const CONNECTION_WAIT: &str = "priory.log.connection.wait";
const START: &str = "priory.run.start.duration";
/// How far back each sample looks again for a start or recipe that ended, so one whose row was
/// written late is still counted once.
const OVERLAP: f64 = 5.0;

/// How many intervals the work group sends its last attention per merge before it reads it again.
const ATTENTION_EVERY: u32 = 10;
/// The kinds of ask, each with its oldest's age.
const ASK_KINDS: [&str; 3] = ["step", "agent", "outside"];
/// The kinds of token a step record counts, in the order `report::dollars` prices them.
const TOKEN_TYPES: [&str; 4] = ["input", "output", "cache_read", "cache_write"];

/// Whether the exporter wants the log's backlog, which only the daemon's own thread may ask for.
static WANT_BACKLOG: AtomicBool = AtomicBool::new(false);
/// The backlog the daemon read since the last request, if it reached its log.
static BACKLOG: Mutex<Option<Vec<(String, u64)>>> = Mutex::new(None);

/// Whether the exporter sends the room group, which only the daemon's own thread can measure.
static ROOM_ON: AtomicBool = AtomicBool::new(false);
/// The room's points since the last request, and why the room could not be measured.
static ROOM: Mutex<Option<(Vec<Point>, Option<String>)>> = Mutex::new(None);

/// The groups this process sends: `PRIORY_METRICS` when set, else the factory's `metrics`.
pub fn groups(factory: &Factory) -> Result<Vec<String>, String> {
    let Ok(text) = std::env::var("PRIORY_METRICS") else {
        return Ok(factory
            .telemetry()
            .map(|telemetry| telemetry.metrics.clone())
            .unwrap_or_default());
    };
    groups_of(&text)
}

/// The groups `PRIORY_METRICS` names, as `process,log` or `none`.
fn groups_of(text: &str) -> Result<Vec<String>, String> {
    if text.trim() == "none" {
        return Ok(Vec::new());
    }
    let mut groups = Vec::new();
    for group in text
        .split(',')
        .map(str::trim)
        .filter(|group| !group.is_empty())
    {
        if !METRIC_GROUPS.contains(&group) {
            return Err(format!(
                "PRIORY_METRICS names `{group}`, which is not a metrics group: the groups are {}, \
                 or none",
                METRIC_GROUPS.join(", ")
            ));
        }
        if !groups.iter().any(|known| known == group) {
            groups.push(group.to_owned());
        }
    }
    Ok(groups)
}

/// The groups `service` sends of `groups`: the daemon alone owns the log, the runs, the work and
/// the room, and each process sends its own intake events, the polls' from the intake, the webhooks' from
/// the server.
fn owned(service: &str, groups: Vec<String>) -> Vec<String> {
    groups
        .into_iter()
        .filter(|group| group == "process" || group == "intake" || service == "daemon")
        .collect()
}

/// Times the first read of a daemon's tick, the log's probe, as `outcome` says it went.
pub(crate) fn probed(elapsed: Duration, outcome: Result<(), &(dyn std::error::Error + 'static)>) {
    let outcome = match outcome {
        Ok(()) => "ok",
        Err(error) if log::timed_out(error) => "timeout",
        Err(_) => "error",
    };
    ticks::timed_as(PROBE, &[("outcome", outcome)], elapsed);
}

/// Reads the log's backlog when the exporter wants it, on the daemon's thread once it reached its
/// log. A failed read leaves it wanted.
pub(crate) fn sample_backlog(log: &dyn Log) {
    if !WANT_BACKLOG.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(backlog) = log.backlog(OUTPUT)
        && let Ok(mut slot) = BACKLOG.lock()
    {
        *slot = Some(backlog);
        WANT_BACKLOG.store(false, Ordering::SeqCst);
    }
}

/// Forgets a backlog read before the daemon lost its log: it is no longer current.
pub(crate) fn unreached() {
    if let Ok(mut slot) = BACKLOG.lock() {
        *slot = None;
    }
}

/// Whether the exporter wants the room's points: each one a tick `measured` anyway, else one per
/// request.
pub(crate) fn wants_room(measured: bool) -> bool {
    ROOM_ON.load(Ordering::Relaxed)
        && (measured || ROOM.lock().is_ok_and(|points| points.is_none()))
}

/// Keeps the room group's points for the next request. `promised` is the memory live runs and held
/// grants reserve. Without `waiting`, the pass did not see the whole queue.
pub(crate) fn sample_room(
    db: &Connection,
    share: &Share,
    room: &Result<room::Room, String>,
    promised: u64,
    waiting: Option<BTreeMap<&'static str, u64>>,
) -> rusqlite::Result<()> {
    let mut points = Vec::new();
    let mut push = |name, unit, attributes: Attributes, value| {
        points.push(Point {
            group: "room",
            kind: Kind::Gauge,
            name,
            unit,
            attributes,
            value,
        });
    };
    let mut held = 0;
    for state in ["waiting", "held"] {
        let (count, bytes): (i64, i64) = db.query_row(
            "SELECT count(*), coalesce(sum(bytes), 0) FROM grants WHERE state = ?1",
            [state],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if state == "held" {
            held = bytes as u64;
        }
        let attributes = vec![("priory.grant.state", state.to_owned())];
        let count = Number::Int(count as u64);
        push("priory.room.grants", "{grant}", attributes.clone(), count);
        let bytes = Number::Int(bytes as u64);
        push("priory.room.grants.memory", "By", attributes, bytes);
    }
    if let Ok(room) = room {
        let limit = room.cpus as f64 * share.cpu;
        let ceiling = crate::room::ceiling(room, share).round() as u64;
        let floor = crate::room::disk_floor(room).round() as u64;
        for (name, unit, value) in [
            ("priory.room.load", "1", Number::Double(room.load)),
            ("priory.room.load.limit", "1", Number::Double(limit)),
            (
                "priory.room.memory.available",
                "By",
                Number::Int(room.memory_free),
            ),
            (
                "priory.room.memory.total",
                "By",
                Number::Int(room.memory_total),
            ),
            ("priory.room.memory.limit", "By", Number::Int(ceiling)),
            ("priory.room.disk.free", "By", Number::Int(room.disk_free)),
            ("priory.room.disk.floor", "By", Number::Int(floor)),
        ] {
            push(name, unit, Vec::new(), value);
        }
    }
    let runs = Number::Int(promised.saturating_sub(held));
    push("priory.room.memory.promised", "By", Vec::new(), runs);
    if let Some(mut waiting) = waiting {
        let restoring: i64 = db.query_row(
            "SELECT count(*) FROM tasks
             WHERE reason = ?1 AND state NOT IN ('done', 'failed', 'dropped')",
            [RESTORES],
            |row| row.get(0),
        )?;
        *waiting.entry("disk").or_default() += restoring as u64;
        for cause in CAUSES {
            let attributes = vec![("priory.room.cause", cause.to_owned())];
            let count = Number::Int(waiting.get(cause).copied().unwrap_or_default());
            push("priory.room.tasks.waiting", "{task}", attributes, count);
        }
    }
    if let Ok(mut slot) = ROOM.lock() {
        *slot = Some((points, room.as_ref().err().cloned()));
    }
    Ok(())
}

/// Starts sending `groups` on a thread of its own, as `service`: `daemon`, `intake` or `serve`.
/// Sends nothing without `[telemetry]` or a group this process owns.
pub fn start(factory: &Factory, service: &str, groups: Vec<String>) -> Result<(), String> {
    let Some(endpoint) = factory.otlp() else {
        return Ok(());
    };
    let groups = owned(service, groups);
    if groups.is_empty() {
        return Ok(());
    }
    let telemetry = factory.telemetry().expect("an endpoint is in [telemetry]");
    let home = runtime::home();
    std::fs::create_dir_all(&home).map_err(|error| format!("{}: {error}", home.display()))?;
    let headers = otlp::header_file(factory, &home, &format!("otlp-headers-{service}"))?;
    let host = room::hostname();
    let runner = crate::runners::runner(factory, host.as_deref())
        .ok()
        .flatten();
    let resource = [
        ("service.name", json!("priory")),
        ("priory.service", json!(service)),
        ("priory.factory", json!(factory.name)),
        ("priory.runner", json!(runner)),
        ("host.name", json!(host)),
        (
            "service.instance.id",
            json!(uuid::Uuid::new_v4().to_string()),
        ),
    ];
    let endpoint = endpoint.trim_end_matches('/').to_owned();
    let agents: BTreeMap<String, &'static str> = factory
        .agents
        .iter()
        .map(|(name, agent)| (name.clone(), agent.value.harness.name()))
        .collect();
    let started = SystemTime::now();
    let backend = match factory.log().backend {
        Backend::HerdrBus => "herdr-bus",
        Backend::Postgres => "postgres",
    };
    let mut exporter = Exporter {
        url: format!("{endpoint}/v1/metrics"),
        endpoint,
        headers,
        body: home.join(format!("metrics-{service}.json")),
        resource: resource
            .iter()
            .filter_map(|(key, value)| otlp::attribute(key, value))
            .collect(),
        groups,
        polls: service == "intake",
        backend,
        cap: telemetry.metrics_cap.unwrap_or(CAP),
        started: nanos(started),
        since: started
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |since| since.as_secs_f64()),
        seen: HashSet::new(),
        agents,
        home: home.clone(),
        born: Instant::now(),
        sent: VecDeque::new(),
        dropped: BTreeMap::new(),
        histograms: BTreeMap::new(),
        logged: HashSet::new(),
        prefix: format!("priory {service}"),
        book: crate::spend::Book::new(factory),
        prices: factory.prices.clone(),
        traced: None,
        tokens: BTreeMap::new(),
        cost: BTreeMap::new(),
        series: BTreeSet::new(),
        attention: None,
        attention_in: 0,
    };
    ROOM_ON.store(exporter.sends("room"), Ordering::Relaxed);
    if exporter.sends("work") {
        exporter.traced = crate::state::read_only()
            .and_then(|db| last_trace(&db).map_err(|error| error.to_string()))
            .ok();
    }
    let every = telemetry.metrics_every.unwrap_or(EVERY);
    ticks::keep();
    std::thread::Builder::new()
        .name("priory-metrics".to_owned())
        .spawn(move || {
            loop {
                let at = Instant::now();
                exporter.send();
                std::thread::sleep(every.saturating_sub(at.elapsed()));
            }
        })
        .map_err(|error| format!("start the metrics thread: {error}"))?;
    Ok(())
}

/// A point's attributes, each a key and its value.
type Attributes = Vec<(&'static str, String)>;

/// What one process sends and remembers between requests.
struct Exporter {
    endpoint: String,
    url: String,
    headers: Option<PathBuf>,
    body: PathBuf,
    resource: Vec<Value>,
    groups: Vec<String>,
    /// Whether this process is the intake, whose polls `intake_sources` records.
    polls: bool,
    /// The log's backend, `herdr-bus` or `postgres`.
    backend: &'static str,
    cap: u64,
    /// When the process started, in Unix nanoseconds: every counter's start time.
    started: u128,
    born: Instant,
    /// The points each accepted request carried, in the last minute.
    sent: VecDeque<(Instant, u64)>,
    /// `priory.metrics.dropped`, by group, none for a send, and kind.
    dropped: BTreeMap<(Option<&'static str>, &'static str), u64>,
    /// Each histogram series since the process started, by group, metric and attributes.
    histograms: BTreeMap<(&'static str, &'static str, Attributes), Histogram>,
    /// Unix seconds from which the next sample looks for starts and recipes that ended.
    since: f64,
    /// The runs and recipes the last sample counted, which the next one looks at again.
    seen: HashSet<(&'static str, i64)>,
    /// Each agent's harness, for a run whose row does not name one.
    agents: BTreeMap<String, &'static str>,
    /// Priory home, whose `worktrees` folder holds the tasks' worktrees.
    home: PathBuf,
    /// The error texts already logged.
    logged: HashSet<String>,
    prefix: String,
    /// The spend caps and what prices them, as `priory costs` reads them.
    book: crate::spend::Book,
    prices: BTreeMap<String, Price>,
    /// The last trace record whose tokens the work group counted; none until it read the trace.
    traced: Option<i64>,
    /// `priory.tokens` and `priory.cost` since the process started, by their attributes.
    tokens: BTreeMap<Attributes, u64>,
    cost: BTreeMap<Attributes, f64>,
    /// The gauge series of open asks and tasks sent before, sent at 0 once they have none.
    series: BTreeSet<(&'static str, Attributes)>,
    /// The last person-minutes per merge, and the intervals left before it is read again.
    attention: Option<f64>,
    attention_in: u32,
}

/// The kind of a point, in the order the cap drops them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Gauge,
    Histogram,
    Counter,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Gauge => "gauge",
            Kind::Histogram => "histogram",
            Kind::Counter => "counter",
        }
    }
}

#[derive(Debug, Clone)]
struct Point {
    group: &'static str,
    kind: Kind,
    name: &'static str,
    unit: &'static str,
    attributes: Attributes,
    value: Number,
}

#[derive(Debug, Clone)]
enum Number {
    Int(u64),
    Double(f64),
    Histogram(Histogram),
}

/// A cumulative histogram over `BOUNDS`.
#[derive(Debug, Clone, Default, PartialEq)]
struct Histogram {
    count: u64,
    sum: f64,
    buckets: Vec<u64>,
}

impl Histogram {
    fn observe(&mut self, seconds: f64) {
        if self.buckets.is_empty() {
            self.buckets = vec![0; BOUNDS.len() + 1];
        }
        let bucket = BOUNDS
            .iter()
            .position(|bound| seconds <= *bound)
            .unwrap_or(BOUNDS.len());
        self.buckets[bucket] += 1;
        self.count += 1;
        self.sum += seconds;
    }
}

impl Exporter {
    fn send(&mut self) {
        let points = self.sample();
        let now = Instant::now();
        while self
            .sent
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= MINUTE)
        {
            self.sent.pop_front();
        }
        let used: u64 = self.sent.iter().map(|(_, points)| points).sum();
        let (points, drops) = capped(points, self.cap.saturating_sub(used), &self.dropped);
        if !drops.is_empty() {
            let mut kinds: BTreeMap<Kind, u64> = BTreeMap::new();
            for ((_, kind), count) in &drops {
                *kinds.entry(*kind).or_default() += count;
            }
            let counts: Vec<String> = kinds
                .iter()
                .map(|(kind, count)| format!("{count} {} points", kind.name()))
                .collect();
            eprintln!(
                "{}: metrics past metrics_cap = {}: dropped {}",
                self.prefix,
                self.cap,
                counts.join(" and ")
            );
        }
        for ((group, kind), count) in drops {
            *self.dropped.entry((Some(group), kind.name())).or_default() += count;
        }
        let count = points.len() as u64 + self.dropped.len() as u64;
        let request = self.request(&points);
        let outcome = std::fs::write(&self.body, request.to_string())
            .map_err(|error| format!("{}: {error}", self.body.display()))
            .and_then(|()| {
                runtime::post_json(&self.url, self.headers.as_deref(), &self.body)
                    .and_then(std::process::Child::wait_with_output)
                    .map_err(|error| format!("curl: {error}"))
            })
            .and_then(|output| {
                let code = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                match (output.status.success(), code.starts_with('2')) {
                    (true, true) => Ok(()),
                    (true, false) => Err(format!("HTTP {code}")),
                    (false, _) => Err(format!("{}: {stderr}", output.status)),
                }
            });
        match outcome {
            Ok(()) => self.sent.push_back((now, count)),
            Err(error) => {
                *self.dropped.entry((None, "send")).or_default() += 1;
                let line = format!("sending metrics to {} failed ({error})", self.endpoint);
                self.once(line);
            }
        }
    }

    /// Logs `line` unless this process already did.
    fn once(&mut self, line: String) {
        if self.logged.insert(line.clone()) {
            eprintln!("{}: {line}", self.prefix);
        }
    }

    fn sends(&self, group: &str) -> bool {
        self.groups.iter().any(|sent| sent == group)
    }

    /// The points of this interval, for each group this process sends.
    fn sample(&mut self) -> Vec<Point> {
        let mut points = Vec::new();
        let ticks = ticks::take();
        let timed = ticks::take_timed();
        if self.sends("process") {
            self.process(ticks, &mut points);
        }
        if self.sends("log") {
            self.log(&timed, &mut points);
        }
        if self.sends("runs") {
            self.runs(&timed, &mut points);
        }
        if self.sends("intake") {
            self.intake(&mut points);
        }
        if self.sends("work") {
            self.work(&mut points);
        }
        if self.sends("room") {
            self.room(&mut points);
        }
        points
    }

    fn process(&mut self, ticks: Vec<f64>, points: &mut Vec<Point>) {
        let point = |kind, name, unit, attributes, value| Point {
            group: "process",
            kind,
            name,
            unit,
            attributes,
            value,
        };
        for (stat, value) in tick_stats(ticks) {
            let attributes = vec![("stat", stat.to_owned())];
            let value = Number::Double(value);
            points.push(point(
                Kind::Gauge,
                "priory.process.tick.duration",
                "s",
                attributes,
                value,
            ));
        }
        match room::own() {
            Ok(own) => {
                let memory = Number::Int(own.memory);
                points.push(point(
                    Kind::Gauge,
                    "process.memory.usage",
                    "By",
                    Vec::new(),
                    memory,
                ));
                for (mode, seconds) in [("user", own.user), ("system", own.system)] {
                    let attributes = vec![("cpu.mode", mode.to_owned())];
                    let seconds = Number::Double(seconds);
                    points.push(point(
                        Kind::Counter,
                        "process.cpu.time",
                        "s",
                        attributes,
                        seconds,
                    ));
                }
            }
            Err(error) => {
                self.once(format!(
                    "metrics: cannot read the process's own use: {error}"
                ));
            }
        }
        let uptime = Number::Double(self.born.elapsed().as_secs_f64());
        points.push(point(
            Kind::Gauge,
            "process.uptime",
            "s",
            Vec::new(),
            uptime,
        ));
    }

    /// The log group: the probes and connection waits timed since the last request, the backlog
    /// the daemon read, and what the state file says of the link, the queue and the leases.
    fn log(&mut self, timed: &[ticks::Timed], points: &mut Vec<Point>) {
        let point = |kind, name, unit, attributes, value| Point {
            group: "log",
            kind,
            name,
            unit,
            attributes,
            value,
        };
        for timed in timed {
            let mut attributes = match timed.metric {
                PROBE => vec![("priory.log.backend", self.backend.to_owned())],
                CONNECTION_WAIT => Vec::new(),
                _ => continue,
            };
            attributes.extend(owned_attributes(&timed.attributes));
            self.observe("log", timed.metric, attributes, timed.seconds);
        }
        self.histograms_of("log", points);
        let backlog = BACKLOG.lock().ok().and_then(|mut slot| slot.take());
        WANT_BACKLOG.store(true, Ordering::SeqCst);
        for (consumer, count) in backlog.into_iter().flatten() {
            let attributes = vec![
                ("priory.log.output", OUTPUT.to_owned()),
                ("priory.log.consumer", consumer),
            ];
            let value = Number::Int(count);
            points.push(point(
                Kind::Gauge,
                "priory.log.backlog",
                "{message}",
                attributes,
                value,
            ));
        }
        let state = crate::state::read_only()
            .and_then(|db| link_state(&db).map_err(|error| error.to_string()));
        match state {
            Ok(state) => {
                let age = Number::Int(state.unreachable);
                points.push(point(
                    Kind::Gauge,
                    "priory.log.unreachable.age",
                    "s",
                    Vec::new(),
                    age,
                ));
                let queued = Number::Int(state.queued);
                points.push(point(
                    Kind::Gauge,
                    "priory.log.queued",
                    "{message}",
                    Vec::new(),
                    queued,
                ));
                for (runner, age) in state.leases {
                    let attributes = vec![("priory.lease.runner", runner)];
                    let age = Number::Int(age);
                    points.push(point(
                        Kind::Gauge,
                        "priory.runner.lease.age",
                        "s",
                        attributes,
                        age,
                    ));
                }
            }
            Err(error) => self.once(format!("metrics: cannot read the log's state: {error}")),
        }
    }

    /// The runs group: each run's start and each phase of a worktree's start, as histograms, the
    /// starts still waiting for `SessionStart`, and the worktrees under Priory home.
    fn runs(&mut self, timed: &[ticks::Timed], points: &mut Vec<Point>) {
        for timed in timed.iter().filter(|timed| timed.metric == runtime::PHASE) {
            let attributes = owned_attributes(&timed.attributes);
            self.observe("runs", runtime::PHASE, attributes, timed.seconds);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |now| now.as_secs_f64());
        let state = crate::state::read_only()
            .and_then(|db| run_state(&db, self.since).map_err(|error| error.to_string()));
        match state {
            Ok(state) => {
                let mut seen = HashSet::new();
                for ended in state.ended {
                    seen.insert(ended.key);
                    if self.seen.contains(&ended.key) {
                        continue;
                    }
                    let attributes = match ended.key.0 {
                        "run" => {
                            let Some(harness) = ended
                                .harness
                                .as_deref()
                                .and_then(|harness| known_harness(harness, &self.agents))
                            else {
                                continue;
                            };
                            vec![
                                ("priory.harness", harness.to_owned()),
                                ("priory.session", ended.session.to_owned()),
                                ("outcome", ended.outcome.to_owned()),
                            ]
                        }
                        _ => vec![
                            ("priory.phase", ended.session.to_owned()),
                            ("outcome", ended.outcome.to_owned()),
                        ],
                    };
                    let metric = if ended.key.0 == "run" {
                        START
                    } else {
                        runtime::PHASE
                    };
                    self.observe("runs", metric, attributes, ended.seconds.max(0.0));
                }
                self.seen = seen;
                self.since = self.since.max(now - OVERLAP);
                let mut starting: BTreeMap<&'static str, (u64, u64)> = self
                    .agents
                    .values()
                    .map(|harness| (*harness, (0, 0)))
                    .collect();
                for (harness, age) in state.starting {
                    let Some(harness) = harness
                        .as_deref()
                        .and_then(|harness| known_harness(harness, &self.agents))
                    else {
                        continue;
                    };
                    let (count, oldest) = starting.entry(harness).or_default();
                    *count += 1;
                    *oldest = (*oldest).max(age);
                }
                let point = |kind, name, unit, attributes, value| Point {
                    group: "runs",
                    kind,
                    name,
                    unit,
                    attributes,
                    value,
                };
                for (harness, (count, oldest)) in starting {
                    let attributes = vec![("priory.harness", harness.to_owned())];
                    points.push(point(
                        Kind::Gauge,
                        "priory.run.starts.active",
                        "{run}",
                        attributes.clone(),
                        Number::Int(count),
                    ));
                    points.push(point(
                        Kind::Gauge,
                        "priory.run.start.oldest.age",
                        "s",
                        attributes,
                        Number::Int(oldest),
                    ));
                }
            }
            Err(error) => self.once(format!("metrics: cannot read the runs' state: {error}")),
        }
        self.histograms_of("runs", points);
        let count = worktrees::found(&self.home).len() as u64;
        points.push(Point {
            group: "runs",
            kind: Kind::Gauge,
            name: "priory.worktrees.active",
            unit: "{worktree}",
            attributes: Vec::new(),
            value: Number::Int(count),
        });
    }

    /// The intake group: the events this process published, the polls that missed a webhook's or
    /// failed, by source, and, from the intake, each polled source's age since its last good poll.
    fn intake(&mut self, points: &mut Vec<Point>) {
        for ((metric, attributes), count) in ticks::counts() {
            let (name, unit) = match metric {
                EVENTS => (EVENTS, "{event}"),
                MISSED => (MISSED, "{event}"),
                FAILURES => (FAILURES, "{poll}"),
                _ => continue,
            };
            points.push(Point {
                group: "intake",
                kind: Kind::Counter,
                name,
                unit,
                attributes: owned_attributes(&attributes),
                value: Number::Int(count),
            });
        }
        if !self.polls || std::env::var_os("PRIORY_DB").is_none() {
            return;
        }
        let ages = crate::state::read_only()
            .and_then(|db| poll_ages(&db).map_err(|error| error.to_string()));
        match ages {
            Ok(ages) => {
                for (source, age) in ages {
                    points.push(Point {
                        group: "intake",
                        kind: Kind::Gauge,
                        name: "priory.intake.poll.age",
                        unit: "s",
                        attributes: vec![("priory.source", source)],
                        value: Number::Int(age),
                    });
                }
            }
            Err(error) => self.once(format!("metrics: cannot read the intake's polls: {error}")),
        }
    }

    /// The room group: what the daemon's thread last measured. An interval in which it ended no tick
    /// sends no point.
    fn room(&mut self, points: &mut Vec<Point>) {
        let Some((room, error)) = ROOM.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        points.extend(room);
        if let Some(error) = error {
            self.once(format!("metrics: cannot measure the room: {error}"));
        }
    }

    /// The work group: the open asks and the oldest's age, the tasks by state, the tokens and cost
    /// of the trace records appended since the process started, each cap's headroom, and the
    /// person-minutes per merge, read once every `ATTENTION_EVERY` intervals.
    fn work(&mut self, points: &mut Vec<Point>) {
        let read = crate::state::read_only().and_then(|db| {
            let now = priory_intake::intake::now()?.to_string();
            let error = |error: rusqlite::Error| error.to_string();
            let traced = match self.traced {
                Some(traced) => traced,
                None => last_trace(&db).map_err(error)?,
            };
            let state = work_state(&db, traced).map_err(error)?;
            let caps = crate::spend::report(&db, &self.book, &now).map_err(error)?;
            let attention = if self.attention_in == 0 {
                let window = crate::report::window(&db, None, &now)?;
                let attention = crate::report::attention::attention(&db, &window, "workflow")?;
                Some(attention.json()["person_minutes_per_merge"].as_f64())
            } else {
                None
            };
            Ok((traced, state, caps, attention))
        });
        let (traced, state, caps, attention) = match read {
            Ok(read) => read,
            Err(error) => {
                self.once(format!("metrics: cannot read the factory's work: {error}"));
                return;
            }
        };
        let point = |kind, name, unit, attributes, value| Point {
            group: "work",
            kind,
            name,
            unit,
            attributes,
            value,
        };
        let mut gauges: Vec<(&'static str, Attributes, u64)> = Vec::new();
        for (kind, lane, count) in state.asks {
            let mut attributes = vec![("priory.ask.kind", kind)];
            attributes.extend(lane.map(|lane| ("priory.lane", lane)));
            gauges.push(("priory.asks.open", attributes, count));
        }
        for (state, workflow, count) in state.tasks {
            let attributes = vec![("priory.state", state), ("priory.workflow", workflow)];
            gauges.push(("priory.tasks", attributes, count));
        }
        for (name, attributes) in &self.series {
            if !gauges
                .iter()
                .any(|(sent, now, _)| sent == name && now == attributes)
            {
                gauges.push((name, attributes.clone(), 0));
            }
        }
        gauges.sort();
        for (name, attributes, count) in gauges {
            self.series.insert((name, attributes.clone()));
            let unit = if name == "priory.tasks" {
                "{task}"
            } else {
                "{ask}"
            };
            points.push(point(
                Kind::Gauge,
                name,
                unit,
                attributes,
                Number::Int(count),
            ));
        }
        for kind in ASK_KINDS {
            let age = state.oldest.get(kind).copied().unwrap_or_default();
            points.push(point(
                Kind::Gauge,
                "priory.asks.oldest.age",
                "s",
                vec![("priory.ask.kind", kind.to_owned())],
                Number::Int(age),
            ));
        }
        if self.attention_in == 0 {
            self.attention = attention.flatten();
            self.attention_in = ATTENTION_EVERY;
        }
        self.attention_in -= 1;
        if let Some(per_merge) = self.attention {
            points.push(point(
                Kind::Gauge,
                "priory.attention.per_merge",
                "min",
                Vec::new(),
                Number::Double(per_merge),
            ));
        }
        for cap in caps {
            let scope = cap["scope"].as_str().unwrap_or_default();
            let (scope, name) = match scope.split_once(' ') {
                Some((scope, name)) => (scope, Some(name)),
                None => (scope, None),
            };
            let mut attributes = vec![("priory.cap.scope", scope.to_owned())];
            attributes.extend(name.map(|name| ("priory.cap.name", name.to_owned())));
            attributes.push((
                "priory.cap.window",
                cap["period"].as_str().unwrap_or_default().to_owned(),
            ));
            let headroom = cap["cap_usd"].as_f64().unwrap_or_default()
                - cap["spent_usd"].as_f64().unwrap_or_default();
            points.push(point(
                Kind::Gauge,
                "priory.spend.headroom",
                "USD",
                attributes,
                Number::Double(headroom),
            ));
        }
        self.traced = Some(state.traced.unwrap_or(traced));
        for record in state.records {
            let mut attributes = Vec::new();
            attributes.extend(
                record
                    .workflow
                    .map(|workflow| ("priory.workflow", workflow)),
            );
            attributes.extend(record.agent.map(|agent| ("gen_ai.agent.name", agent)));
            attributes.extend(
                record
                    .model
                    .clone()
                    .map(|model| ("gen_ai.request.model", model)),
            );
            for (kind, tokens) in TOKEN_TYPES.into_iter().zip(record.tokens) {
                let mut attributes = attributes.clone();
                attributes.push(("gen_ai.token.type", kind.to_owned()));
                *self.tokens.entry(attributes).or_default() +=
                    u64::try_from(tokens).unwrap_or_default();
            }
            if let Some(price) = record.model.and_then(|model| self.prices.get(&model)) {
                *self.cost.entry(attributes).or_default() +=
                    crate::report::dollars(&record.tokens, price);
            }
        }
        for (attributes, tokens) in &self.tokens {
            points.push(point(
                Kind::Counter,
                "priory.tokens",
                "{token}",
                attributes.clone(),
                Number::Int(*tokens),
            ));
        }
        for (attributes, dollars) in &self.cost {
            points.push(point(
                Kind::Counter,
                "priory.cost",
                "USD",
                attributes.clone(),
                Number::Double(*dollars),
            ));
        }
    }

    /// Counts one duration of histogram `metric` of `group` under `attributes`.
    fn observe(
        &mut self,
        group: &'static str,
        metric: &'static str,
        attributes: Attributes,
        seconds: f64,
    ) {
        self.histograms
            .entry((group, metric, attributes))
            .or_default()
            .observe(seconds);
    }

    /// Every histogram series of `group` since the process started.
    fn histograms_of(&self, group: &'static str, points: &mut Vec<Point>) {
        for ((of, name, attributes), histogram) in &self.histograms {
            if *of == group {
                points.push(Point {
                    group,
                    kind: Kind::Histogram,
                    name,
                    unit: "s",
                    attributes: attributes.clone(),
                    value: Number::Histogram(histogram.clone()),
                });
            }
        }
    }

    /// One `ExportMetricsServiceRequest` holding `points` and `priory.metrics.dropped`.
    fn request(&self, points: &[Point]) -> Value {
        let now = nanos(SystemTime::now()).to_string();
        let started = self.started.to_string();
        let mut metrics: Vec<(&str, &str, Kind, Vec<Value>)> = Vec::new();
        for point in points {
            let attributes: Vec<Value> = point
                .attributes
                .iter()
                .filter_map(|(key, value)| otlp::attribute(key, &json!(value)))
                .collect();
            let mut data = json!({ "timeUnixNano": now, "attributes": attributes });
            match &point.value {
                Number::Int(value) => data["asInt"] = json!(value.to_string()),
                Number::Double(value) => data["asDouble"] = json!(value),
                Number::Histogram(histogram) => {
                    data["count"] = json!(histogram.count.to_string());
                    data["sum"] = json!(histogram.sum);
                    data["bucketCounts"] = histogram
                        .buckets
                        .iter()
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .into();
                    data["explicitBounds"] = json!(BOUNDS);
                }
            }
            if point.kind != Kind::Gauge {
                data["startTimeUnixNano"] = json!(started);
            }
            match metrics.iter_mut().find(|metric| metric.0 == point.name) {
                Some(metric) => metric.3.push(data),
                None => metrics.push((point.name, point.unit, point.kind, vec![data])),
            }
        }
        let dropped: Vec<Value> = self
            .dropped
            .iter()
            .map(|((group, kind), count)| {
                let attributes: Vec<Value> = [("priory.group", *group), ("kind", Some(*kind))]
                    .into_iter()
                    .filter_map(|(key, value)| otlp::attribute(key, &json!(value)))
                    .collect();
                json!({
                    "startTimeUnixNano": started,
                    "timeUnixNano": now,
                    "asInt": count.to_string(),
                    "attributes": attributes,
                })
            })
            .collect();
        if !dropped.is_empty() {
            metrics.push(("priory.metrics.dropped", "{point}", Kind::Counter, dropped));
        }
        let metrics: Vec<Value> = metrics
            .into_iter()
            .map(|(name, unit, kind, points)| match kind {
                Kind::Gauge => json!({ "name": name, "unit": unit, "gauge": { "dataPoints": points } }),
                Kind::Histogram => json!({
                    "name": name,
                    "unit": unit,
                    "histogram": { "aggregationTemporality": 2, "dataPoints": points },
                }),
                Kind::Counter => json!({
                    "name": name,
                    "unit": unit,
                    "sum": { "aggregationTemporality": 2, "isMonotonic": true, "dataPoints": points },
                }),
            })
            .collect();
        json!({ "resourceMetrics": [{
            "resource": { "attributes": self.resource },
            "scopeMetrics": [{ "scope": { "name": "priory" }, "metrics": metrics }],
        }] })
    }
}

/// What the state file says of the daemon's link to its log.
struct LinkState {
    /// Seconds since the log stopped answering; 0 while it answers.
    unreachable: u64,
    /// The messages that wait to publish.
    queued: u64,
    /// Each runner's lease age in log time: the newest alive time any lease holds, less its own.
    leases: Vec<(String, u64)>,
}

fn link_state(db: &Connection) -> rusqlite::Result<LinkState> {
    let unreachable: Option<i64> = db
        .query_row(
            "SELECT CAST(strftime('%s', 'now') AS INTEGER) - CAST(strftime('%s', since) AS INTEGER)
             FROM log_link",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let queued: i64 = db.query_row("SELECT count(*) FROM sends", [], |row| row.get(0))?;
    let leases = db
        .prepare(
            "SELECT runner, (SELECT max(alive) FROM leases) - alive FROM leases ORDER BY runner",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let whole = |value: i64| u64::try_from(value).unwrap_or_default();
    Ok(LinkState {
        unreachable: unreachable.map_or(0, whole),
        queued: whole(queued),
        leases: leases
            .into_iter()
            .map(|(runner, age)| (runner, whole(age)))
            .collect(),
    })
}

/// The newest trace record's sequence number, 0 in an empty trace.
fn last_trace(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("SELECT coalesce(max(seq), 0) FROM trace", [], |row| {
        row.get(0)
    })
}

/// A priced trace record: a model step, a debate turn, a handler or a summarizer call.
struct Record {
    workflow: Option<String>,
    agent: Option<String>,
    model: Option<String>,
    tokens: [i64; 4],
}

/// What the state file says of the factory's work.
struct WorkState {
    /// The open asks no one snoozed, by kind and lane.
    asks: Vec<(String, Option<String>, u64)>,
    /// The seconds since the oldest of those was asked, in its current revision, by kind.
    oldest: BTreeMap<String, u64>,
    /// The tasks not ended, by state and workflow.
    tasks: Vec<(String, String, u64)>,
    /// The priced records with tokens appended after the one asked, and the newest one read.
    records: Vec<Record>,
    traced: Option<i64>,
}

fn work_state(db: &Connection, after: i64) -> rusqlite::Result<WorkState> {
    let mut asks = Vec::new();
    let mut oldest: BTreeMap<String, u64> = BTreeMap::new();
    let rows = db
        .prepare(
            "SELECT asks.kind, tasks.lane, count(*), max(CAST((julianday('now')
               - julianday(coalesce(ask_revisions.created, asks.created))) * 86400 AS INTEGER))
             FROM asks LEFT JOIN tasks ON tasks.id = asks.task
             LEFT JOIN ask_revisions ON ask_revisions.ask = asks.id
               AND ask_revisions.revision = asks.revision
             LEFT JOIN snoozes ON snoozes.kind = 'ask' AND snoozes.item = asks.id
             WHERE asks.answer IS NULL AND asks.withdrawn IS NULL
               AND coalesce(tasks.state, '') NOT IN ('done', 'failed', 'dropped')
               AND NOT coalesce(snoozes.back IS NULL
                 AND snoozes.until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 0)
             GROUP BY 1, 2 ORDER BY 1, 2",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let whole = |value: i64| u64::try_from(value).unwrap_or_default();
    for (kind, lane, count, age) in rows {
        let age = whole(age.unwrap_or_default());
        let top = oldest.entry(kind.clone()).or_default();
        *top = (*top).max(age);
        asks.push((kind, lane, whole(count)));
    }
    let tasks = db
        .prepare(
            "SELECT state, workflow, count(*) FROM tasks
             WHERE state NOT IN ('done', 'failed', 'dropped') GROUP BY 1, 2 ORDER BY 1, 2",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, whole(row.get::<_, i64>(2)?)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut traced = None;
    let mut records = Vec::new();
    let mut statement = db.prepare(
        "SELECT trace.seq, json_extract(trace.event, '$.data.workflow'),
           CASE trace.type WHEN 'factory.handler.called' THEN 'handler'
             WHEN 'factory.summarizer.called' THEN 'summarizer' ELSE runs.agent END,
           json_extract(trace.event, '$.data.model'),
           json_extract(trace.event, '$.data.tokens') IS NOT NULL,
           coalesce(json_extract(trace.event, '$.data.tokens.input'), 0),
           coalesce(json_extract(trace.event, '$.data.tokens.output'), 0),
           coalesce(json_extract(trace.event, '$.data.tokens.cache_read'), 0),
           coalesce(json_extract(trace.event, '$.data.tokens.cache_write'), 0),
           trace.type IN ('factory.handler.called', 'factory.summarizer.called',
               'factory.debate.turn_ended')
             OR (trace.type = 'factory.step.ended'
             AND json_extract(trace.event, '$.data.decider') = 'model'
             AND json_extract(trace.event, '$.data.debate') IS NULL)
         FROM trace
         LEFT JOIN runs ON runs.id = CAST(json_extract(trace.event, '$.data.run') AS INTEGER)
         WHERE trace.seq > ?1 ORDER BY trace.seq",
    )?;
    let mut rows = statement.query([after])?;
    while let Some(row) = rows.next()? {
        traced = Some(row.get::<_, i64>(0)?);
        let (has_tokens, priced): (bool, bool) = (row.get(4)?, row.get(9)?);
        if !has_tokens || !priced {
            continue;
        }
        records.push(Record {
            workflow: row.get(1)?,
            agent: row.get(2)?,
            model: row.get(3)?,
            tokens: [row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?],
        });
    }
    Ok(WorkState {
        asks,
        oldest,
        tasks,
        records,
        traced,
    })
}

/// Each polled source and the seconds since its last poll with no error: since its last poll, or
/// since it started failing while it fails.
fn poll_ages(db: &Connection) -> rusqlite::Result<Vec<(String, u64)>> {
    db.prepare(
        "SELECT source, CAST((julianday('now') - julianday(coalesce(failing_since, polled))) * 86400
           AS INTEGER) FROM intake_sources ORDER BY source",
    )?
    .query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            u64::try_from(row.get::<_, Option<i64>>(1)?.unwrap_or_default()).unwrap_or_default(),
        ))
    })?
    .collect()
}

/// A timed duration's attributes as a point's.
fn owned_attributes(attributes: &[(&'static str, &'static str)]) -> Attributes {
    attributes
        .iter()
        .map(|(key, value)| (*key, (*value).to_owned()))
        .collect()
}

/// `harness` as a harness name, or the harness of the agent it names: a run from before the row
/// kept its harness names its agent instead.
fn known_harness(harness: &str, agents: &BTreeMap<String, &'static str>) -> Option<&'static str> {
    match harness {
        "claude-code" => Some("claude-code"),
        "codex" => Some("codex"),
        agent => agents.get(agent).copied(),
    }
}

/// A run's start, or a recipe, that ended.
struct Ended {
    /// `run` or `recipe`, and its id.
    key: (&'static str, i64),
    /// A run's harness, or its agent when the row names no harness.
    harness: Option<String>,
    /// A run's `first` or `resume`, or a recipe's moment.
    session: &'static str,
    outcome: &'static str,
    seconds: f64,
}

/// What the state file says of the runs' starts and the recipes.
struct RunState {
    /// The starts and recipes that ended at or after the time asked.
    ended: Vec<Ended>,
    /// Each live run with no `SessionStart` yet: its harness, or its agent, and its age in seconds.
    starting: Vec<(Option<String>, u64)>,
}

/// A text time's Unix seconds, in SQL.
macro_rules! unix {
    ($column:literal) => {
        concat!("(julianday(", $column, ") - 2440587.5) * 86400")
    };
}

fn run_state(db: &Connection, since: f64) -> rusqlite::Result<RunState> {
    let mut ended = Vec::new();
    let starts = db
        .prepare(concat!(
            "SELECT id, coalesce(harness, agent), coalesce(resumes, 0), session_started IS NOT NULL,
               (julianday(coalesce(session_started, ended)) - julianday(started)) * 86400
             FROM runs WHERE (session_started IS NOT NULL AND ",
            unix!("session_started"),
            " >= ?1) OR (session_started IS NULL AND ready = 0 AND ended IS NOT NULL AND ",
            unix!("ended"),
            " >= ?1)"
        ))?
        .query_map([since], |row| {
            let resumes: bool = row.get(2)?;
            let ok: bool = row.get(3)?;
            Ok(Ended {
                key: ("run", row.get(0)?),
                harness: row.get(1)?,
                session: if resumes { "resume" } else { "first" },
                outcome: if ok { "ok" } else { "error" },
                seconds: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ended.extend(starts);
    let recipes = db
        .prepare(concat!(
            "SELECT id, moment, coalesce(status, ''), (julianday(ended) - julianday(started)) * 86400
             FROM recipes WHERE started IS NOT NULL AND ended IS NOT NULL AND ",
            unix!("ended"),
            " >= ?1"
        ))?
        .query_map([since], |row| {
            let moment: String = row.get(1)?;
            let status: String = row.get(2)?;
            Ok(Ended {
                key: ("recipe", row.get(0)?),
                harness: None,
                session: match moment.as_str() {
                    "setup" => "setup",
                    "resume" => "resume",
                    _ => "teardown",
                },
                outcome: if status == "exit 0" {
                    "ok"
                } else if status.starts_with("timed out") {
                    "timeout"
                } else {
                    "error"
                },
                seconds: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ended.extend(recipes);
    let starting = db
        .prepare(
            "SELECT coalesce(harness, agent), CAST((julianday('now') - julianday(started)) * 86400 AS INTEGER)
             FROM runs WHERE ended IS NULL AND ready = 0",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(RunState { ended, starting })
}

/// The ticks' average, 95th percentile by nearest rank, and maximum; none without a tick.
fn tick_stats(mut ticks: Vec<f64>) -> Vec<(&'static str, f64)> {
    if ticks.is_empty() {
        return Vec::new();
    }
    ticks.sort_by(f64::total_cmp);
    let count = ticks.len();
    let average = ticks.iter().sum::<f64>() / count as f64;
    let rank = (count * 95).div_ceil(100).max(1);
    vec![
        ("avg", average),
        ("p95", ticks[rank - 1]),
        ("max", ticks[count - 1]),
    ]
}

/// `points` cut to fit `budget` with `priory.metrics.dropped`, which is never cut: gauges first,
/// then histograms, then counters, each from the group with most of that kind. Returns what stays
/// and the count cut per group and kind.
fn capped(
    mut points: Vec<Point>,
    budget: u64,
    dropped: &BTreeMap<(Option<&'static str>, &'static str), u64>,
) -> (Vec<Point>, BTreeMap<(&'static str, Kind), u64>) {
    let mut cut: BTreeMap<(&'static str, Kind), u64> = BTreeMap::new();
    loop {
        let series = dropped.len() as u64
            + cut
                .keys()
                .filter(|(group, kind)| !dropped.contains_key(&(Some(*group), kind.name())))
                .count() as u64;
        if points.len() as u64 + series <= budget {
            return (points, cut);
        }
        let Some(kind) = points.iter().map(|point| point.kind).min() else {
            return (points, cut);
        };
        let mut groups: BTreeMap<&'static str, usize> = BTreeMap::new();
        for point in points.iter().filter(|point| point.kind == kind) {
            *groups.entry(point.group).or_default() += 1;
        }
        let group = groups
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(group, _)| group)
            .unwrap_or_default();
        let last = points
            .iter()
            .rposition(|point| point.kind == kind && point.group == group)
            .expect("the group has a point of this kind");
        points.remove(last);
        *cut.entry((group, kind)).or_default() += 1;
    }
}

fn nanos(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn point(kind: Kind, name: &'static str) -> Point {
        Point {
            group: "process",
            kind,
            name,
            unit: "s",
            attributes: Vec::new(),
            value: Number::Int(1),
        }
    }

    #[test]
    fn the_cap_cuts_gauges_before_counters_and_leaves_room_for_the_drop_counter() {
        let points = vec![
            point(Kind::Gauge, "a"),
            point(Kind::Counter, "b"),
            point(Kind::Gauge, "c"),
            point(Kind::Counter, "d"),
        ];

        let (kept, cut) = capped(points.clone(), 3, &BTreeMap::new());
        let names: Vec<_> = kept.iter().map(|point| point.name).collect();
        assert_eq!(names, ["b", "d"]);
        assert_eq!(cut, BTreeMap::from([(("process", Kind::Gauge), 2)]));

        let (kept, cut) = capped(points.clone(), 2, &BTreeMap::new());
        assert_eq!(kept.len(), 0);
        assert_eq!(
            cut,
            BTreeMap::from([
                (("process", Kind::Gauge), 2),
                (("process", Kind::Counter), 2)
            ])
        );

        let (kept, cut) = capped(points, 4, &BTreeMap::new());
        assert_eq!((kept.len(), cut.len()), (4, 0));
    }

    #[test]
    fn the_cap_cuts_every_gauge_before_a_histogram_and_a_histogram_before_a_counter() {
        let log = |kind, name| Point {
            group: "log",
            ..point(kind, name)
        };
        let points = vec![
            log(Kind::Histogram, "probe"),
            point(Kind::Gauge, "uptime"),
            point(Kind::Counter, "cpu"),
            log(Kind::Gauge, "backlog"),
            log(Kind::Gauge, "queued"),
        ];

        let (kept, cut) = capped(points, 4, &BTreeMap::new());

        let names: Vec<_> = kept.iter().map(|point| point.name).collect();
        assert_eq!(names, ["probe", "cpu"]);
        assert_eq!(
            cut,
            BTreeMap::from([(("log", Kind::Gauge), 2), (("process", Kind::Gauge), 1)])
        );

        let points = vec![
            point(Kind::Counter, "cpu"),
            log(Kind::Histogram, "probe"),
            point(Kind::Counter, "cpu"),
        ];
        let dropped = BTreeMap::from([
            ((Some("log"), "histogram"), 1),
            ((Some("process"), "counter"), 1),
        ]);
        let (kept, cut) = capped(points, 4, &dropped);
        let names: Vec<_> = kept.iter().map(|point| point.name).collect();
        assert_eq!(names, ["cpu", "cpu"]);
        assert_eq!(cut, BTreeMap::from([(("log", Kind::Histogram), 1)]));
    }

    #[test]
    fn a_histogram_counts_each_duration_in_the_first_bucket_whose_bound_holds_it() {
        let mut histogram = Histogram::default();
        for seconds in [0.001, 0.005, 0.2, 2.0, 61.0] {
            histogram.observe(seconds);
        }
        let mut buckets = vec![0; BOUNDS.len() + 1];
        for bucket in [0, 0, 5, 8, BOUNDS.len()] {
            buckets[bucket] += 1;
        }
        assert_eq!(histogram.buckets, buckets);
        assert_eq!(histogram.count, 5);
        assert!((histogram.sum - 63.206).abs() < 1e-9);
    }

    #[test]
    fn the_run_state_reads_each_start_and_recipe_that_ended_since_a_time_by_outcome() {
        let db = crate::state::memory();
        db.execute_batch(
            "INSERT INTO tasks (key, workflow, step, state, trigger, source)
               VALUES ('cli:a', 'build', 'implement', 'running', 'cli', 'a');
             INSERT INTO runs (task, step, agent, tmux, session, started, ended, ready, harness,
               resumes, session_started)
             VALUES (1, 'implement', 'implementer', 'p', 'a', '2026-11-23T10:00:00Z', NULL, 1,
                 'claude-code', 1, '2026-11-23T10:00:02.500Z'),
               (1, 'implement', 'implementer', 'p', 'b', '2026-11-23T10:00:00Z',
                 '2026-11-23T10:01:00Z', 0, NULL, NULL, NULL),
               (1, 'implement', 'implementer', 'p', 'c', '2026-11-23T09:00:00Z',
                 '2026-11-23T09:01:00Z', 0, 'codex', 0, NULL);
             INSERT INTO recipes (task, moment, worktree, argv, timeout, started, ended, status)
             VALUES (1, 'setup', 'w', '[]', 60, '2026-11-23T10:00:00Z', '2026-11-23T10:00:30Z',
                 'timed out after 30s');",
        )
        .unwrap();
        let since = 1_795_428_000.0; // 2026-11-23T10:00:00Z

        let state = run_state(&db, since).unwrap();

        let ended: Vec<_> = state
            .ended
            .iter()
            .map(|ended| {
                (
                    ended.key,
                    ended.harness.clone(),
                    ended.session,
                    ended.outcome,
                    ended.seconds.round(),
                )
            })
            .collect();
        assert_eq!(
            ended,
            [
                (
                    ("run", 1),
                    Some("claude-code".to_owned()),
                    "resume",
                    "ok",
                    3.0
                ),
                (
                    ("run", 2),
                    Some("implementer".to_owned()),
                    "first",
                    "error",
                    60.0
                ),
                (("recipe", 1), None, "setup", "timeout", 30.0),
            ]
        );
        assert!(state.starting.is_empty());
        let agents = BTreeMap::from([("implementer".to_owned(), "codex")]);
        assert_eq!(known_harness("implementer", &agents), Some("codex"));
        assert_eq!(known_harness("nobody", &agents), None);
    }

    #[test]
    fn only_the_daemon_sends_the_log_runs_work_and_room_groups_and_every_process_its_intake_group()
    {
        let groups = || {
            ["process", "log", "runs", "intake", "work", "room"]
                .map(str::to_owned)
                .to_vec()
        };
        assert_eq!(
            owned("daemon", groups()),
            ["process", "log", "runs", "intake", "work", "room"]
        );
        assert_eq!(owned("intake", groups()), ["process", "intake"]);
        assert_eq!(owned("serve", groups()), ["process", "intake"]);
    }

    #[test]
    fn a_source_s_poll_age_runs_from_its_last_poll_or_from_when_it_started_failing() {
        let db = crate::state::memory();
        db.execute_batch(
            "INSERT INTO intake_sources (source, polled, error, failing_since)
             VALUES ('github', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-30 seconds'), NULL, NULL),
               ('linear', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-5 seconds'), 'timed out after 300s',
                 strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-3600 seconds'));",
        )
        .unwrap();

        let ages = poll_ages(&db).unwrap();

        assert_eq!(ages.len(), 2);
        assert_eq!(ages[0].0, "github");
        assert!((29..=31).contains(&ages[0].1), "{ages:?}");
        assert_eq!(ages[1].0, "linear");
        assert!((3599..=3601).contains(&ages[1].1), "{ages:?}");
    }

    #[test]
    fn the_tick_statistics_are_the_average_the_nearest_rank_p95_and_the_maximum() {
        let ticks: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(
            tick_stats(ticks),
            vec![("avg", 10.5), ("p95", 19.0), ("max", 20.0)]
        );
        assert_eq!(
            tick_stats(vec![3.0]),
            vec![("avg", 3.0), ("p95", 3.0), ("max", 3.0)]
        );
        assert!(tick_stats(Vec::new()).is_empty());
    }

    #[test]
    fn priory_metrics_names_groups_or_none_and_refuses_an_unknown_one() {
        assert_eq!(groups_of("none").unwrap(), Vec::<String>::new());
        assert_eq!(groups_of(" process , process,").unwrap(), ["process"]);
        assert_eq!(
            groups_of("process,nope").unwrap_err(),
            "PRIORY_METRICS names `nope`, which is not a metrics group: the groups are process, \
             log, runs, intake, work, room, or none"
        );
    }

    proptest! {
        #[test]
        fn priory_metrics_of_any_text_returns_or_refuses_without_panicking(text in "\\PC{0,100}") {
            let _ = groups_of(&text);
        }

        #[test]
        fn a_capped_request_fits_its_budget_unless_only_the_drop_counter_is_left(
            gauges in 0usize..20,
            counters in 0usize..20,
            budget in 0u64..50,
        ) {
            let points: Vec<Point> = std::iter::repeat_n(point(Kind::Gauge, "g"), gauges)
                .chain(std::iter::repeat_n(point(Kind::Counter, "c"), counters))
                .collect();
            let (kept, cut) = capped(points, budget, &BTreeMap::new());
            prop_assert!(kept.is_empty() || kept.len() as u64 + cut.len() as u64 <= budget);
            let counters_kept = kept.iter().filter(|point| point.kind == Kind::Counter).count();
            prop_assert!(counters_kept == counters || kept.iter().all(|point| point.kind == Kind::Counter));
        }
    }
}
