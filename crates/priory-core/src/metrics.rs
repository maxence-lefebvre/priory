//! Priory's own metrics, sent as OTLP/HTTP JSON to `<otlp>/v1/metrics` once per interval, under a
//! per-minute cap (docs/daemon/metrics.md).

use crate::otlp;
use priory_factory::factory::{Factory, METRIC_GROUPS};
use priory_runtime::{room, runtime, ticks};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const EVERY: Duration = Duration::from_secs(60);
const CAP: u64 = 1000;
const MINUTE: Duration = Duration::from_secs(60);

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

/// Starts sending `groups` on a thread of its own, as `service`: `daemon`, `intake` or `serve`.
/// Sends nothing without `[telemetry]` or a group.
pub fn start(factory: &Factory, service: &str, groups: Vec<String>) -> Result<(), String> {
    let Some(endpoint) = factory.otlp() else {
        return Ok(());
    };
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
        cap: telemetry.metrics_cap.unwrap_or(CAP),
        started: nanos(SystemTime::now()),
        born: Instant::now(),
        sent: VecDeque::new(),
        dropped: BTreeMap::new(),
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

/// What one process sends and remembers between requests.
struct Exporter {
    endpoint: String,
    url: String,
    headers: Option<PathBuf>,
    body: PathBuf,
    resource: Vec<Value>,
    groups: Vec<String>,
    cap: u64,
    /// When the process started, in Unix nanoseconds: every counter's start time.
    started: u128,
    born: Instant,
    /// The points each accepted request carried, in the last minute.
    sent: VecDeque<(Instant, u64)>,
    /// `priory.metrics.dropped`, by group, none for a send, and kind.
    dropped: BTreeMap<(Option<&'static str>, &'static str), u64>,
    /// The error texts already logged.
    logged: HashSet<String>,
    prefix: String,
}

/// The kind of a point, in the order the cap drops them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Gauge,
    Counter,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Gauge => "gauge",
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
    attributes: Vec<(&'static str, &'static str)>,
    value: Number,
}

#[derive(Debug, Clone, Copy)]
enum Number {
    Int(u64),
    Double(f64),
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
            let counts: Vec<String> = drops
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
        for (kind, count) in drops {
            // One group today; a later group's drops count under their own.
            *self
                .dropped
                .entry((Some("process"), kind.name()))
                .or_default() += count;
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
                if self.logged.insert(line.clone()) {
                    eprintln!("{}: {line}", self.prefix);
                }
            }
        }
    }

    /// The points of this interval, for each group this process sends.
    fn sample(&mut self) -> Vec<Point> {
        let mut points = Vec::new();
        if self.groups.iter().any(|group| group == "process") {
            let point = |kind, name, unit, attributes, value| Point {
                group: "process",
                kind,
                name,
                unit,
                attributes,
                value,
            };
            let ticks = ticks::take();
            for (stat, value) in tick_stats(ticks) {
                let attributes = vec![("stat", stat)];
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
                        let attributes = vec![("cpu.mode", mode)];
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
                    let line = format!("metrics: cannot read the process's own use: {error}");
                    if self.logged.insert(line.clone()) {
                        eprintln!("{}: {line}", self.prefix);
                    }
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
        points
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
            match point.value {
                Number::Int(value) => data["asInt"] = json!(value.to_string()),
                Number::Double(value) => data["asDouble"] = json!(value),
            }
            if point.kind == Kind::Counter {
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
/// from the group with most of them, then counters. Returns what stays and the count cut per kind.
fn capped(
    mut points: Vec<Point>,
    budget: u64,
    dropped: &BTreeMap<(Option<&'static str>, &'static str), u64>,
) -> (Vec<Point>, BTreeMap<Kind, u64>) {
    let mut cut: BTreeMap<Kind, u64> = BTreeMap::new();
    loop {
        let series = dropped.len() as u64
            + cut
                .keys()
                .filter(|kind| !dropped.contains_key(&(Some("process"), kind.name())))
                .count() as u64;
        if points.len() as u64 + series <= budget {
            return (points, cut);
        }
        let Some(kind) = points.iter().map(|point| point.kind).min() else {
            return (points, cut);
        };
        let mut groups: BTreeMap<&str, usize> = BTreeMap::new();
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
        *cut.entry(kind).or_default() += 1;
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
        assert_eq!(cut, BTreeMap::from([(Kind::Gauge, 2)]));

        let (kept, cut) = capped(points.clone(), 2, &BTreeMap::new());
        assert_eq!(kept.len(), 0);
        assert_eq!(cut, BTreeMap::from([(Kind::Gauge, 2), (Kind::Counter, 2)]));

        let (kept, cut) = capped(points, 4, &BTreeMap::new());
        assert_eq!((kept.len(), cut.len()), (4, 0));
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
             or none"
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
