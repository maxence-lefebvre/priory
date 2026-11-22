//! Priory's own metrics, sent as OTLP/HTTP JSON to `<otlp>/v1/metrics` once per interval, under a
//! per-minute cap (docs/daemon/metrics.md).

use crate::otlp;
use priory_factory::factory::{Backend, Factory, METRIC_GROUPS};
use priory_intake::intake::OUTPUT;
use priory_intake::log::{self, Log};
use priory_runtime::{room, runtime, ticks};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet, VecDeque};
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

/// Whether the exporter wants the log's backlog, which only the daemon's own thread may ask for.
static WANT_BACKLOG: AtomicBool = AtomicBool::new(false);
/// The backlog the daemon read since the last request, if it reached its log.
static BACKLOG: Mutex<Option<Vec<(String, u64)>>> = Mutex::new(None);

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

/// The groups `service` sends of `groups`: the daemon alone owns the log.
fn owned(service: &str, groups: Vec<String>) -> Vec<String> {
    groups
        .into_iter()
        .filter(|group| group == "process" || service == "daemon")
        .collect()
}

/// Times the first read of a daemon's tick, the log's probe, as `outcome` says it went.
pub(crate) fn probed(elapsed: Duration, outcome: Result<(), &(dyn std::error::Error + 'static)>) {
    let outcome = match outcome {
        Ok(()) => "ok",
        Err(error) if log::timed_out(error) => "timeout",
        Err(_) => "error",
    };
    ticks::timed_as(PROBE, Some(outcome), elapsed);
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
        backend,
        cap: telemetry.metrics_cap.unwrap_or(CAP),
        started: nanos(SystemTime::now()),
        born: Instant::now(),
        sent: VecDeque::new(),
        dropped: BTreeMap::new(),
        histograms: BTreeMap::new(),
        logged: HashSet::new(),
        prefix: format!("priory {service}"),
    };
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
    /// Each histogram series since the process started, by metric and attributes.
    histograms: BTreeMap<(&'static str, Attributes), Histogram>,
    /// The error texts already logged.
    logged: HashSet<String>,
    prefix: String,
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
            self.log(timed, &mut points);
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
    fn log(&mut self, timed: Vec<ticks::Timed>, points: &mut Vec<Point>) {
        let point = |kind, name, unit, attributes, value| Point {
            group: "log",
            kind,
            name,
            unit,
            attributes,
            value,
        };
        for timed in timed {
            let attributes = match (timed.metric, timed.outcome) {
                (PROBE, Some(outcome)) => vec![
                    ("priory.log.backend", self.backend.to_owned()),
                    ("outcome", outcome.to_owned()),
                ],
                (CONNECTION_WAIT, None) => Vec::new(),
                _ => continue,
            };
            self.histograms
                .entry((timed.metric, attributes))
                .or_default()
                .observe(timed.seconds);
        }
        for ((name, attributes), histogram) in &self.histograms {
            let value = Number::Histogram(histogram.clone());
            points.push(point(Kind::Histogram, name, "s", attributes.clone(), value));
        }
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
    fn only_the_daemon_sends_the_log_group() {
        let groups = || vec!["process".to_owned(), "log".to_owned()];
        assert_eq!(owned("daemon", groups()), ["process", "log"]);
        assert_eq!(owned("intake", groups()), ["process"]);
        assert_eq!(owned("serve", groups()), ["process"]);
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
             log, or none"
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
