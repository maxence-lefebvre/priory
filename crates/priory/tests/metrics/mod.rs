//! Priory's own metrics over OTLP (docs/daemon/metrics.md). A fake collector on 127.0.0.1 records
//! each request the real `curl` sends to `/v1/metrics`; the fake `curl` hands those calls on.

use super::*;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// An OTLP/HTTP endpoint on 127.0.0.1 that records each request as `{path, body}` and answers
/// with the next queued status, 200 once none is queued.
pub(super) struct Collector {
    pub(super) url: String,
    received: Arc<Mutex<Vec<Value>>>,
    answers: Arc<Mutex<VecDeque<u16>>>,
}

impl Collector {
    pub(super) fn start() -> Collector {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let received = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(VecDeque::new()));
        let (into, from) = (Arc::clone(&received), Arc::clone(&answers));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                receive(stream, &into, &from);
            }
        });
        Collector {
            url,
            received,
            answers,
        }
    }

    /// Answers the next request with `status`.
    fn answer(&self, status: u16) {
        self.answers.lock().unwrap().push_back(status);
    }

    fn requests(&self) -> Vec<Value> {
        self.received.lock().unwrap().clone()
    }

    /// Waits until `done` holds for the requests received, and returns them.
    pub(super) fn until(&self, setup: &Setup, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let requests = self.requests();
            if done(&requests) {
                return requests;
            }
            assert!(
                Instant::now() < deadline,
                "{} requests: {requests:?}\ndaemon: {}",
                requests.len(),
                fs::read_to_string(setup.dir.join("daemon.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits for `count` requests and returns them.
    fn until_count(&self, setup: &Setup, count: usize) -> Vec<Value> {
        self.until(setup, |requests| requests.len() >= count)
    }
}

fn receive(stream: TcpStream, into: &Mutex<Vec<Value>>, answers: &Mutex<VecDeque<u16>>) {
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
    let (mut length, mut expect) = (0, false);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap(),
            "expect" => expect = true,
            _ => {}
        }
    }
    if expect {
        writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").unwrap();
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    into.lock()
        .unwrap()
        .push(json!({ "path": path, "body": body }));
    let status = answers.lock().unwrap().pop_front().unwrap_or(200);
    let reply = format!("HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = writer.write_all(reply.as_bytes());
}

impl Setup {
    /// A factory that exports to `collector`, with the extra `[telemetry]` lines `keys`.
    fn metrics(&self, collector: &Collector, keys: &str) {
        let factory = format!(
            "name = \"demo\"\n\n[telemetry]\notlp = \"{}\"\n{keys}",
            collector.url
        );
        fs::write(self.dir.join("factory/factory.toml"), factory).unwrap();
    }

    /// The process's own resident memory in kB and CPU time in clock ticks, in `PRIORY_PROC/self`.
    fn own(&self, memory: u64, user: u64, system: u64) {
        let dir = self.dir.join("proc/self");
        fs::create_dir_all(&dir).unwrap();
        let files = [
            (
                "status",
                format!("Name:\tpriory\nVmRSS:\t   {memory} kB\nThreads:\t3\n"),
            ),
            (
                "stat",
                format!(
                    "4242 (priory (x) d) S 1 4242 4242 0 -1 4194560 100 0 0 0 {user} {system} 0 0 20 0 3 0 100\n"
                ),
            ),
        ];
        for (name, text) in files {
            let partial = dir.join(format!("{name}.new"));
            fs::write(&partial, text).unwrap();
            fs::rename(partial, dir.join(name)).unwrap();
        }
    }
}

/// Each data point of metric `name` in an OTLP metrics request, as its attributes and its value: a
/// histogram's value is its count.
pub(super) fn points(request: &Value, name: &str) -> Vec<(Value, f64)> {
    let metrics = request["body"]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
        .as_array()
        .unwrap();
    metrics
        .iter()
        .filter(|metric| metric["name"] == name)
        .flat_map(|metric| {
            let data = ["gauge", "sum", "histogram"]
                .into_iter()
                .find_map(|kind| metric.get(kind))
                .unwrap();
            data["dataPoints"].as_array().unwrap().clone()
        })
        .map(|point| {
            let value = point
                .get("asDouble")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| {
                    let int = point.get("asInt").unwrap_or(&point["count"]);
                    int.as_str().unwrap().parse::<f64>().unwrap()
                });
            (attributes(&point["attributes"]), value)
        })
        .collect()
}

/// The names of the metrics in a request.
pub(super) fn names(request: &Value) -> Vec<String> {
    request["body"]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|metric| metric["name"].as_str().unwrap().to_owned())
        .collect()
}

/// An OTLP attribute list as one JSON object, each value as a string or a number.
fn attributes(list: &Value) -> Value {
    let object: serde_json::Map<String, Value> = list
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let key = item["key"].as_str().unwrap();
            (key.to_owned(), attribute(list, key))
        })
        .collect();
    Value::Object(object)
}

pub(super) fn resource(request: &Value) -> Value {
    attributes(&request["body"]["resourceMetrics"][0]["resource"]["attributes"])
}

fn service(request: &Value) -> Value {
    resource(request)["priory.service"].clone()
}

#[test]
fn a_daemon_sends_its_process_metrics_under_a_resource_naming_the_factory_host_and_service() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-daemon");
    setup.metrics(&collector, "metrics = [\"process\"]\n");
    setup.own(20480, 250, 50);

    setup.start();

    let requests = collector.until_count(&setup, 1);
    let request = &requests[0];
    assert_eq!(request["path"], "/v1/metrics");
    let resource = resource(request);
    assert_eq!(resource["service.name"], "priory");
    assert_eq!(resource["priory.service"], "daemon");
    assert_eq!(resource["priory.factory"], "demo");
    assert_eq!(resource["host.name"], "runner");
    assert_eq!(resource["priory.runner"], Value::Null);
    let instance = resource["service.instance.id"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(instance).is_ok(), "{instance}");
    assert_eq!(
        points(request, "process.memory.usage"),
        vec![(json!({}), 20480.0 * 1024.0)]
    );
    assert_eq!(
        points(request, "process.cpu.time"),
        vec![
            (json!({ "cpu.mode": "user" }), 2.5),
            (json!({ "cpu.mode": "system" }), 0.5)
        ]
    );
    let uptime = points(request, "process.uptime");
    assert!(uptime.len() == 1 && uptime[0].1 >= 0.0, "{uptime:?}");
}

#[test]
fn without_metrics_nothing_goes_to_v1_metrics_and_spans_still_go_to_v1_traces() {
    let collector = Collector::start();
    let mut setup = Setup::with_workflow("metrics-off", STEPS);
    setup.metrics(&collector, "");
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });

    setup.until_spans(5);

    assert!(
        collector.requests().is_empty(),
        "{:?}",
        collector.requests()
    );
}

#[test]
fn priory_metrics_none_sends_nothing_though_the_factory_names_groups() {
    let collector = Collector::start();
    let mut setup = Setup::with_workflow("metrics-none", STEPS);
    setup.metrics(&collector, "metrics = [\"process\"]\n");
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    setup.start_with(exe, &[("PRIORY_METRICS", "none")]);
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });

    setup.until_spans(5);

    assert!(
        collector.requests().is_empty(),
        "{:?}",
        collector.requests()
    );
}

#[test]
fn priory_metrics_naming_an_unknown_group_stops_the_daemon_with_its_name() {
    let collector = Collector::start();
    let setup = Setup::new("metrics-unknown");
    setup.metrics(&collector, "");

    let mut daemon = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["daemon", setup.dir.join("factory").to_str().unwrap()])
        .env("PRIORY_METRICS", "process,nope")
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            daemon.kill().unwrap();
            panic!("the daemon did not stop");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stderr = String::new();
    daemon
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(!status.success());
    assert_eq!(
        stderr,
        "priory daemon: PRIORY_METRICS names `nope`, which is not a metrics group: the groups are \
         process, log, runs, intake, work, room, or none\n"
    );
}

#[test]
fn past_the_cap_gauges_go_first_and_the_next_request_counts_them_as_dropped() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-cap");
    setup.metrics(
        &collector,
        "metrics = [\"process\"]\nmetrics_every = \"10s\"\nmetrics_cap = 3\n",
    );
    setup.own(20480, 250, 50);

    setup.start();

    let requests = collector.until_count(&setup, 2);
    let first = &requests[0];
    assert!(
        names(first).contains(&"process.cpu.time".to_owned()),
        "{first}"
    );
    for gauge in ["process.memory.usage", "process.uptime"] {
        assert!(!names(first).contains(&gauge.to_owned()), "{first}");
    }
    let gauges = json!({ "priory.group": "process", "kind": "gauge" });
    let dropped = points(&requests[1], "priory.metrics.dropped");
    let gauges = dropped.iter().find(|(attributes, _)| *attributes == gauges);
    assert!(
        gauges.is_some_and(|(_, count)| *count >= 4.0),
        "{dropped:?}"
    );
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert!(
        log.contains("priory daemon: metrics past metrics_cap = 3: dropped 2 gauge points"),
        "{log}"
    );
}

#[test]
fn a_refused_request_is_not_retried_and_the_next_one_carries_the_totals_and_counts_the_send() {
    let collector = Collector::start();
    collector.answer(503);
    let mut setup = Setup::new("metrics-refused");
    setup.metrics(
        &collector,
        "metrics = [\"process\"]\nmetrics_every = \"10s\"\n",
    );
    setup.own(20480, 250, 50);
    setup.start();
    collector.until_count(&setup, 1);

    setup.own(20480, 400, 80);
    let requests = collector.until_count(&setup, 2);

    let cpu = |request: &Value| points(request, "process.cpu.time");
    assert_eq!(cpu(&requests[0])[0].1, 2.5);
    assert_eq!(
        cpu(&requests[1]),
        vec![
            (json!({ "cpu.mode": "user" }), 4.0),
            (json!({ "cpu.mode": "system" }), 0.8)
        ]
    );
    let start = |request: &Value| {
        let metrics = request["body"]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"].clone();
        let cpu = metrics
            .as_array()
            .unwrap()
            .iter()
            .find(|metric| metric["name"] == "process.cpu.time")
            .cloned()
            .unwrap();
        cpu["sum"]["dataPoints"][0]["startTimeUnixNano"].clone()
    };
    assert_eq!(start(&requests[0]), start(&requests[1]));
    assert_eq!(
        points(&requests[1], "priory.metrics.dropped"),
        vec![(json!({ "kind": "send" }), 1.0)]
    );
    let ticks: Vec<Value> = points(&requests[1], "priory.process.tick.duration")
        .into_iter()
        .map(|(attributes, _)| attributes["stat"].clone())
        .collect();
    assert_eq!(ticks, ["avg", "p95", "max"]);
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    let failed = format!("sending metrics to {} failed (HTTP 503)", collector.url);
    assert_eq!(log.matches(&failed).count(), 1, "{log}");
}

#[test]
fn the_intake_and_serve_each_send_their_own_metrics() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-services");
    setup.metrics(
        &collector,
        "metrics = [\"process\"]\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n",
    );
    let factory = setup.dir.join("factory");
    let factory = factory.to_str().unwrap();

    for verb in ["intake", "serve"] {
        let log = fs::File::create(setup.dir.join("daemon.log")).unwrap();
        setup.daemon = Some(
            setup
                .command(env!("CARGO_BIN_EXE_priory"))
                .args([verb, factory])
                .env("TEST_TOKEN", "t0ken")
                .stderr(log)
                .spawn()
                .unwrap(),
        );

        let requests = collector.until(&setup, |requests| {
            requests.iter().any(|request| service(request) == verb)
        });

        let request = requests.iter().find(|request| service(request) == verb);
        assert_eq!(resource(request.unwrap())["priory.factory"], "demo");
        setup.crash();
    }
    assert!(
        collector
            .requests()
            .iter()
            .all(|request| service(request) != "daemon")
    );
}

/// The one histogram `name` in a request.
pub(super) fn histogram(request: &Value, name: &str) -> Value {
    let metrics = &request["body"]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"];
    let metric = metrics
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["name"] == name);
    metric.unwrap_or_else(|| panic!("no {name} in {request}"))["histogram"].clone()
}

#[test]
fn a_daemon_sends_its_log_s_probe_and_backlog_and_each_runner_s_lease_age_in_log_time() {
    let collector = Collector::start();
    let mut a = runner("metrics-log", "a", None);
    let mut b = runner("metrics-log", "b", Some(&a));
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    a.start_with(exe, &[("PRIORY_NOW", T0)]);
    logged(&a, "priory.runner.alive", json!({ "runner": "a" }));
    a.crash();

    b.metrics(&collector, "metrics = [\"log\"]\nmetrics_every = \"10s\"\n");
    b.start_with(exe, &[("PRIORY_NOW", AN_HOUR_LATER)]);

    let lease = |request: &Value| -> Vec<(Value, f64)> {
        let mut ages = points(request, "priory.runner.lease.age");
        ages.sort_by_key(|one| one.0.to_string());
        ages
    };
    let backlog = |request: &Value| points(request, "priory.log.backlog");
    let requests = collector.until(&b, |requests| {
        requests
            .iter()
            .any(|request| lease(request).len() == 2 && !backlog(request).is_empty())
    });
    let request = requests
        .iter()
        .find(|request| lease(request).len() == 2 && !backlog(request).is_empty())
        .unwrap();
    assert_eq!(resource(request)["priory.runner"], "b");
    assert_eq!(
        lease(request),
        vec![
            (json!({ "priory.lease.runner": "a" }), 3600.0),
            (json!({ "priory.lease.runner": "b" }), 0.0)
        ]
    );
    let behind = json!({ "priory.log.output": OUTPUT, "priory.log.consumer": "priory.a" });
    let behind = backlog(request)
        .into_iter()
        .find(|(attributes, _)| *attributes == behind);
    assert!(behind.is_some_and(|(_, count)| count >= 1.0), "{request}");
    let probes = points(request, "priory.log.probe.duration");
    let ok = json!({ "priory.log.backend": "herdr-bus", "outcome": "ok" });
    assert!(
        probes
            .iter()
            .any(|(attributes, count)| *attributes == ok && *count >= 1.0),
        "{probes:?}"
    );
    let probe = histogram(request, "priory.log.probe.duration");
    assert_eq!(probe["aggregationTemporality"], 2);
    let point = &probe["dataPoints"][0];
    let bounds = point["explicitBounds"].as_array().unwrap();
    assert_eq!(
        point["bucketCounts"].as_array().unwrap().len(),
        bounds.len() + 1
    );
    assert_eq!(
        points(request, "priory.log.unreachable.age"),
        vec![(json!({}), 0.0)]
    );
    assert_eq!(points(request, "priory.log.queued").len(), 1, "{request}");
    assert!(!names(request).contains(&"priory.log.connection.wait".to_owned()));
    assert!(!names(request).contains(&"process.uptime".to_owned()));
}

#[test]
fn the_intake_and_serve_send_no_log_group_which_only_the_daemon_owns() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-log-intake");
    setup.metrics(&collector, "metrics = [\"process\", \"log\"]\n");
    let factory = setup.dir.join("factory");
    let log = fs::File::create(setup.dir.join("daemon.log")).unwrap();
    setup.daemon = Some(
        setup
            .command(env!("CARGO_BIN_EXE_priory"))
            .args(["intake", factory.to_str().unwrap()])
            .stderr(log)
            .spawn()
            .unwrap(),
    );

    let requests = collector.until(&setup, |requests| !requests.is_empty());

    let names = names(&requests[0]);
    assert!(names.contains(&"process.uptime".to_owned()), "{names:?}");
    assert!(
        names.iter().all(|name| !name.starts_with("priory.log.")),
        "{names:?}"
    );
}

/// The value of the point of `name` with `attributes` in `request`, if any.
fn point(request: &Value, name: &str, attributes: &Value) -> Option<f64> {
    points(request, name)
        .into_iter()
        .find(|(found, _)| found == attributes)
        .map(|(_, value)| value)
}

/// A start's attributes on `priory.run.start.duration`.
fn start(session: &str, outcome: &str) -> Value {
    json!({ "priory.harness": "claude-code", "priory.session": session, "outcome": outcome })
}

/// A phase's attributes on `priory.worktree.phase.duration`.
fn phase(phase: &str) -> Value {
    json!({ "priory.phase": phase, "outcome": "ok" })
}

#[test]
fn a_daemon_sends_each_run_s_start_by_session_and_its_worktree_s_phases() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-runs");
    setup.metrics(
        &collector,
        "metrics = [\"runs\"]\nmetrics_every = \"10s\"\n\n[repos.\"owner/repo\"]\nsetup = { shell = \"true\" }\n",
    );
    setup.calls(&[ask(), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    answer_the_ask(&setup);
    setup.until("tasks", |rows| rows[0][4] == "done");

    let started = |request: &Value| {
        point(request, "priory.run.start.duration", &start("first", "ok")).is_some_and(|n| n >= 1.0)
            && point(request, "priory.run.start.duration", &start("resume", "ok"))
                .is_some_and(|n| n >= 1.0)
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(started));
    let request = requests.iter().find(|request| started(request)).unwrap();
    let histogram = histogram(request, "priory.run.start.duration");
    assert_eq!(histogram["aggregationTemporality"], 2);
    for name in ["clone", "fetch", "checkout", "setup"] {
        let count = point(request, "priory.worktree.phase.duration", &phase(name));
        assert!(count.is_some_and(|count| count >= 1.0), "{name}: {request}");
    }
    let harness = json!({ "priory.harness": "claude-code" });
    assert_eq!(
        point(request, "priory.run.starts.active", &harness),
        Some(0.0)
    );
    assert_eq!(
        point(request, "priory.run.start.oldest.age", &harness),
        Some(0.0)
    );
    assert_eq!(
        points(request, "priory.worktrees.active").len(),
        1,
        "{request}"
    );
    assert!(!names(request).contains(&"process.uptime".to_owned()));
}

#[test]
fn a_run_with_no_session_start_shows_as_an_active_start_whose_age_climbs() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-runs-silent");
    setup.metrics(
        &collector,
        "metrics = [\"runs\"]\nmetrics_every = \"10s\"\n",
    );
    setup.hold("silent");
    setup.publish(&requested(42));
    setup.start();

    let harness = json!({ "priory.harness": "claude-code" });
    let hung = |request: &Value| {
        point(request, "priory.run.starts.active", &harness) == Some(1.0)
            && point(request, "priory.run.start.oldest.age", &harness).is_some_and(|age| age >= 1.0)
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(hung));

    let request = requests.iter().find(|request| hung(request)).unwrap();
    assert!(
        points(request, "priory.run.start.duration").is_empty(),
        "a start still waiting has no duration yet: {request}"
    );
    assert_eq!(setup.rows("runs")[0][7], "-");
}

#[test]
fn a_run_on_claudes_trust_dialog_counts_as_a_start_that_ended_in_error() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-runs-trust");
    setup.metrics(
        &collector,
        "metrics = [\"runs\"]\nmetrics_every = \"10s\"\n",
    );
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::write(
        setup.dir.join("fake/silent"),
        TRUST_DIALOG.replace("WORKTREE", &worktree.display().to_string()),
    )
    .unwrap();
    setup.publish(&requested(42));
    setup.start();

    let failed = |request: &Value| {
        point(
            request,
            "priory.run.start.duration",
            &start("first", "error"),
        )
        .is_some_and(|n| n >= 1.0)
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(failed));

    let request = requests.iter().find(|request| failed(request)).unwrap();
    assert_eq!(
        point(request, "priory.run.start.duration", &start("first", "ok")),
        None
    );
    let harness = json!({ "priory.harness": "claude-code" });
    assert_eq!(
        point(request, "priory.run.starts.active", &harness),
        Some(0.0)
    );
}

/// `[serve]` and a GitHub webhook, whose secret is `TEST_SECRET`.
const HOOKS: &str = "\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n\n[webhooks.github]\nsecret_env = \"TEST_SECRET\"\n";

const SECRET: &str = "s3cret";

/// An event of `owner/repo`'s issue events: issue `number` labeled `label`, as `gh api` lists it.
fn issue_event(id: u64, number: u64, label: &str) -> Value {
    json!({
        "id": id,
        "event": "labeled",
        "created_at": "2026-11-17T10:00:00Z",
        "actor": { "login": "max" },
        "label": { "name": label, "color": "ededed" },
        "issue": {
            "id": 9000 + number,
            "number": number,
            "title": "Fix the thing",
            "html_url": format!("https://github.com/owner/repo/issues/{number}"),
            "user": { "login": "max" },
            "created_at": "2026-11-17T09:00:00Z"
        }
    })
}

impl Setup {
    /// Makes `gh` in `bin` print `events` for `owner/repo`'s issue events, and `[]` for any other
    /// call; or fail every call with `fail`.
    fn issue_events(&self, events: &Value, fail: Option<&str>) {
        let answer = self.dir.join("fake/events.json");
        fs::write(&answer, events.to_string()).unwrap();
        let script = match fail {
            Some(error) => format!("#!/bin/sh\necho '{error}' >&2\nexit 1\n"),
            None => format!(
                "#!/bin/sh\ncase \"$*\" in\n  *repos/owner/repo/issues/events*) cat '{}' ;;\n  *) echo '[]' ;;\nesac\n",
                answer.display()
            ),
        };
        let gh = self.dir.join("bin/gh");
        fs::create_dir_all(gh.parent().unwrap()).unwrap();
        fs::write(&gh, script).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// `priory <verb> <factory> <args>`, with `bin` and then the real `curl` first on `PATH`.
    fn process(&self, verb: &str, args: &[&str]) -> std::process::Command {
        let path = std::env::join_paths(
            std::iter::once(self.dir.join("bin"))
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut command = self.command(env!("CARGO_BIN_EXE_priory"));
        command
            .arg(verb)
            .arg(self.dir.join("factory"))
            .args(args)
            .env("PATH", path)
            .env("TEST_TOKEN", "t0ken")
            .env("TEST_SECRET", SECRET);
        command
    }

    /// Starts `priory intake` as this setup's process, its stderr in `daemon.log`.
    fn start_intake(&mut self) {
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        self.daemon = Some(self.process("intake", &[]).stderr(log).spawn().unwrap());
    }
}

/// The attributes of `priory.intake.events` for one event of `source` `via` a poll or a webhook.
fn event(source: &str, via: &str, outcome: &str) -> Value {
    json!({ "priory.source": source, "priory.via": via, "outcome": outcome })
}

#[test]
fn an_intake_counts_the_events_its_poll_publishes_and_those_a_webhook_should_have_sent_first() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-intake-poll");
    setup.metrics(
        &collector,
        &format!("metrics = [\"intake\"]\nmetrics_every = \"10s\"\n{HOOKS}"),
    );
    setup.issue_events(&json!([issue_event(100, 41, "bug")]), None);
    let baseline = setup.process("intake", &["--once"]).output().unwrap();
    assert!(
        baseline.status.success(),
        "{}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    setup.issue_events(
        &json!([issue_event(101, 42, "agent"), issue_event(100, 41, "bug")]),
        None,
    );

    setup.start_intake();

    let github = json!({ "priory.source": "github" });
    let polled = |request: &Value| {
        point(
            request,
            "priory.intake.events",
            &event("github", "poll", "published"),
        )
        .is_some()
            && point(request, "priory.intake.poll.age", &github).is_some()
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(polled));
    let request = requests.iter().find(|request| polled(request)).unwrap();
    assert_eq!(service(request), "intake");
    assert_eq!(
        point(
            request,
            "priory.intake.events",
            &event("github", "poll", "published")
        ),
        Some(1.0)
    );
    assert_eq!(
        point(request, "priory.intake.poll.missed", &github),
        Some(1.0)
    );
    let age = point(request, "priory.intake.poll.age", &github);
    assert!(
        age.is_some_and(|age| (0.0..60.0).contains(&age)),
        "{request}"
    );
    assert!(points(request, "priory.intake.poll.failures").is_empty());
    let counter = request["body"]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["name"] == "priory.intake.events")
        .cloned()
        .unwrap();
    assert_eq!(counter["sum"]["isMonotonic"], true);
    assert!(!names(request).contains(&"process.uptime".to_owned()));
}

#[test]
fn an_intake_counts_each_failed_poll_and_its_source_s_age_since_it_started_failing() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-intake-failing");
    setup.metrics(
        &collector,
        "metrics = [\"intake\"]\nmetrics_every = \"10s\"\n",
    );
    setup.issue_events(&json!([]), Some("HTTP 404: Not Found"));

    setup.start_intake();

    let failure = json!({ "priory.source": "github", "reason": "error" });
    let github = json!({ "priory.source": "github" });
    let failed = |request: &Value| {
        point(request, "priory.intake.poll.failures", &failure).is_some_and(|n| n >= 1.0)
            && point(request, "priory.intake.poll.age", &github).is_some()
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(failed));
    let request = requests.iter().find(|request| failed(request)).unwrap();
    assert!(points(request, "priory.intake.poll.missed").is_empty());
    assert!(
        points(request, "priory.intake.events").is_empty(),
        "{request}"
    );
}

/// `sha256=` and the hex HMAC-SHA256 of `body` under `SECRET`, as GitHub signs a delivery.
fn signed(body: &str) -> String {
    use hmac::{KeyInit, Mac};
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256={hex}")
}

/// POSTs an `issues` delivery to `<address>/hooks/github` and returns the status.
fn deliver(address: &str, delivery: &str, signature: &str, body: &str) -> u16 {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let request = format!(
        "POST /hooks/github HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\n\
         X-GitHub-Event: issues\r\nX-GitHub-Delivery: {delivery}\r\nX-Hub-Signature-256: {signature}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response[9..12].parse().unwrap()
}

#[test]
fn the_server_counts_a_delivery_published_then_redelivered_and_one_whose_signature_fails() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-intake-webhook");
    setup.metrics(
        &collector,
        &format!("metrics = [\"intake\"]\nmetrics_every = \"10s\"\n{HOOKS}"),
    );
    let mut server = setup
        .process("serve", &[])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(server.stderr.take().unwrap());
    setup.daemon = Some(server);
    let address = loop {
        let mut line = String::new();
        assert!(stderr.read_line(&mut line).unwrap() > 0, "serve exited");
        if let Some(address) = line.trim().strip_prefix("priory: serving on http://") {
            break address.to_owned();
        }
    };
    let mut log = fs::File::create(setup.dir.join("daemon.log")).unwrap();
    std::thread::spawn(move || std::io::copy(&mut stderr, &mut log));
    let now = herdr_bus::Timestamp::now().to_string();
    let body = json!({
        "action": "labeled",
        "issue": {
            "id": 9042,
            "number": 42,
            "title": "Fix the thing",
            "html_url": "https://github.com/owner/repo/issues/42",
            "user": { "login": "max" },
            "created_at": now,
            "updated_at": now
        },
        "label": { "id": 1, "name": "agent", "color": "ededed" },
        "repository": { "id": 1, "full_name": "owner/repo" },
        "sender": { "login": "max" }
    })
    .to_string();

    assert_eq!(deliver(&address, "d-1", &signed(&body), &body), 202);
    assert_eq!(deliver(&address, "d-2", &signed(&body), &body), 202);
    assert_eq!(deliver(&address, "d-3", "sha256=00", &body), 401);

    let counted = |request: &Value| {
        ["published", "duplicate", "refused"].iter().all(|outcome| {
            point(
                request,
                "priory.intake.events",
                &event("github", "webhook", outcome),
            ) == Some(1.0)
        })
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(counted));
    let request = requests.iter().find(|request| counted(request)).unwrap();
    assert_eq!(service(request), "serve");
    assert!(points(request, "priory.intake.poll.age").is_empty());
    assert!(points(request, "priory.intake.poll.missed").is_empty());
}

/// Dollars per million tokens of the fake `claude`'s model.
const PRICES: &str =
    "[claude-fake-5]\ninput = 3.0\noutput = 15.0\ncache_read = 0.3\ncache_write = 3.75\n";

impl Setup {
    /// `priory <args> --json`, which must succeed, as JSON.
    fn json(&self, args: &[&str]) -> Value {
        let output = self.priory(&[args, &["--json"]].concat());
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn a_daemon_sends_the_open_asks_the_oldest_s_age_the_tasks_by_state_and_each_cap_s_headroom() {
    let collector = Collector::start();
    let mut setup = Setup::with_workflow("metrics-work", STEPS);
    setup.metrics(
        &collector,
        "metrics = [\"work\"]\nmetrics_every = \"10s\"\n\n[caps]\nday = 1.0\n",
    );
    fs::write(setup.dir.join("factory/prices.toml"), PRICES).unwrap();
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    let agent = json!({ "priory.ask.kind": "agent", "priory.lane": "none/misc" });
    let asleep = json!({ "priory.state": "asleep", "priory.workflow": "implement" });
    let asking = |request: &Value| {
        point(request, "priory.asks.open", &agent) == Some(1.0)
            && point(request, "priory.tasks", &asleep) == Some(1.0)
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(asking));

    let request = requests.iter().find(|request| asking(request)).unwrap();
    for kind in ["step", "agent", "outside"] {
        let age = point(
            request,
            "priory.asks.oldest.age",
            &json!({ "priory.ask.kind": kind }),
        );
        assert!(age.is_some_and(|age| age >= 0.0), "{kind}: {request}");
    }
    assert_eq!(points(request, "priory.asks.open").len(), 1, "{request}");
    assert_eq!(points(request, "priory.tasks").len(), 1, "{request}");
    let factory = setup.dir.join("factory");
    let costs = setup.json(&["costs", factory.to_str().unwrap()]);
    let cap = &costs["caps"][0];
    let headroom = cap["cap_usd"].as_f64().unwrap() - cap["spent_usd"].as_f64().unwrap();
    let day = json!({ "priory.cap.scope": "factory", "priory.cap.window": "day" });
    let sent = point(request, "priory.spend.headroom", &day);
    assert!(
        sent.is_some_and(|sent| (sent - headroom).abs() < 1e-9 && sent <= 1.0),
        "{sent:?} != {headroom}: {request}"
    );
    assert!(!names(request).contains(&"process.uptime".to_owned()));
}

#[test]
fn a_daemon_counts_the_tokens_and_cost_of_the_steps_that_ended_since_it_started() {
    let collector = Collector::start();
    let mut setup = Setup::with_workflow("metrics-work-tokens", STEPS);
    fs::write(setup.dir.join("factory/prices.toml"), PRICES).unwrap();
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(41));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    setup.crash();
    setup.metrics(
        &collector,
        "metrics = [\"work\"]\nmetrics_every = \"10s\"\n",
    );
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.len() == 2 && rows.iter().all(|row| row[4] == "done")
    });
    let step = |kind: &str| {
        json!({ "priory.workflow": "implement", "gen_ai.agent.name": "implementer",
                "gen_ai.request.model": "claude-fake-5", "gen_ai.token.type": kind })
    };
    let counted = |request: &Value| point(request, "priory.tokens", &step("input")).is_some();
    let requests = collector.until(&setup, |requests| {
        requests.iter().filter(|request| counted(request)).count() >= 2
    });
    let sent: Vec<&Value> = requests.iter().filter(|request| counted(request)).collect();
    // Only the second task's two steps, the fake's turns 3 and 4, count: the first task's ended
    // before the daemon started.
    for request in &sent[sent.len() - 2..] {
        for (kind, tokens) in [
            ("input", 207.0),
            ("output", 27.0),
            ("cache_read", 2007.0),
            ("cache_write", 107.0),
        ] {
            assert_eq!(
                point(request, "priory.tokens", &step(kind)),
                Some(tokens),
                "{kind}: {request}"
            );
        }
        let model = json!({ "priory.workflow": "implement", "gen_ai.agent.name": "implementer",
                            "gen_ai.request.model": "claude-fake-5" });
        let cost = point(request, "priory.cost", &model);
        assert!(
            cost.is_some_and(|cost| (cost - 0.002_029_35).abs() < 1e-9),
            "{cost:?}: {request}"
        );
    }
}

#[test]
fn a_daemon_sends_the_person_minutes_per_merge_that_report_attention_prints() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-work-attention");
    setup.metrics(
        &collector,
        "metrics = [\"work\"]\nmetrics_every = \"10s\"\n",
    );
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    // One answer, 5 person-minutes, and one merged pull request the task owned, this week.
    db.execute_batch(
        "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source)
           VALUES (7, 'cli:seeded', 'implement', 'implement', 'done', 'none/misc', 'cli', 'seeded');
         INSERT INTO asks (id, task, step, kind, question, options, created)
           VALUES (7, 7, 'implement', 'agent', 'Which?', '[]', '2026-11-16T09:00:00Z');
         UPDATE ask_revisions SET created = '2026-11-16T09:00:00Z' WHERE ask = 7;
         UPDATE asks SET answer = 'yes', answered = '2026-11-16T10:00:00Z' WHERE id = 7;
         INSERT INTO owns (task, resource) VALUES (7, 'github:o/r#1');
         INSERT INTO inbox (id, type, subject, time, data, links, outcome) VALUES
           ('m1', 'github.pull_request.closed', 'o/r#1', '2026-11-16T11:00:00Z',
            '{\"merged\":true}', '[]', 'stored');",
    )
    .unwrap();
    drop(db);
    let now = [("PRIORY_NOW", "2026-11-17T11:00:00Z"), ("TZ", "UTC")];

    setup.start_with(Path::new(env!("CARGO_BIN_EXE_priory")), &now);

    let per_merge = |request: &Value| points(request, "priory.attention.per_merge");
    let requests = collector.until(&setup, |requests| {
        requests
            .iter()
            .any(|request| !per_merge(request).is_empty())
    });
    let request = requests
        .iter()
        .find(|request| !per_merge(request).is_empty())
        .unwrap();
    let report = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .envs(now)
        .args(["report", "attention", "--json"])
        .output()
        .unwrap();
    assert!(report.status.success(), "{report:?}");
    let report: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report["person_minutes_per_merge"], 5.0);
    assert_eq!(per_merge(request), vec![(json!({}), 5.0)]);
}

const GB: f64 = (1u64 << 30) as f64;
const ROOM: &str = "metrics = [\"room\"]\nmetrics_every = \"10s\"\n";

/// The tasks waiting for room by cause, in a request with the room group.
fn waiting(request: &Value) -> Vec<(String, f64)> {
    points(request, "priory.room.tasks.waiting")
        .into_iter()
        .map(|(attributes, count)| {
            let cause = attributes["priory.room.cause"].as_str().unwrap().to_owned();
            (cause, count)
        })
        .collect()
}

/// `waiting` with `count` tasks under `cause` and none under any other.
fn only(cause: &str, count: f64) -> Vec<(String, f64)> {
    ["load", "memory", "disk", "unmeasured"]
        .map(|one| (one.to_owned(), if one == cause { count } else { 0.0 }))
        .to_vec()
}

/// Waits for a request whose tasks wait for room as `expected`, and returns it.
fn until_waiting(setup: &Setup, collector: &Collector, expected: &[(String, f64)]) -> Value {
    let found = |request: &Value| waiting(request) == expected;
    let requests = collector.until(setup, |requests| requests.iter().any(found));
    requests.into_iter().find(found).unwrap()
}

fn cpus() -> f64 {
    std::thread::available_parallelism().unwrap().get() as f64
}

#[test]
fn an_idle_daemon_sends_its_room_against_the_ceilings_and_a_zero_for_each_state_and_cause() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-room-idle");
    setup.metrics(&collector, ROOM);
    fs::create_dir_all(setup.dir.join("factory/runners")).unwrap();
    let runner = "platform = { os = \"linux\" }\nagents = { cpu = \"50%\", memory = \"75%\" }\n";
    fs::write(setup.dir.join("factory/runners/runner.toml"), runner).unwrap();
    setup.memory(40);
    setup.load(1.5);
    setup.disk(300);

    setup.start();

    let request = until_waiting(&setup, &collector, &only("none", 0.0));
    let value = |name: &str| point(&request, name, &json!({}));
    assert_eq!(value("priory.room.load"), Some(1.5));
    assert_eq!(value("priory.room.load.limit"), Some(cpus() * 0.5));
    assert_eq!(value("priory.room.memory.available"), Some(40.0 * GB));
    assert_eq!(value("priory.room.memory.total"), Some(64.0 * GB));
    assert_eq!(value("priory.room.memory.limit"), Some(48.0 * GB));
    assert_eq!(value("priory.room.memory.promised"), Some(0.0));
    assert_eq!(value("priory.room.disk.free"), Some(300.0 * GB));
    assert_eq!(value("priory.room.disk.floor"), Some(20.0 * GB));
    let none = [
        (json!({ "priory.grant.state": "waiting" }), 0.0),
        (json!({ "priory.grant.state": "held" }), 0.0),
    ];
    assert_eq!(points(&request, "priory.room.grants"), none);
    assert_eq!(points(&request, "priory.room.grants.memory"), none);
}

#[test]
fn a_daemon_counts_the_tasks_waiting_for_room_by_what_the_head_lacks() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-room-causes");
    with_review(&setup, "6GB");
    setup.metrics(&collector, ROOM);
    setup.memory(11);
    setup.publish(&requested_in(42, Some("now")));
    setup.publish(&review_requested(43));

    setup.start();

    until_waiting(&setup, &collector, &only("memory", 2.0));
    setup.disk(10);
    setup.memory(60);
    until_waiting(&setup, &collector, &only("disk", 2.0));
    setup.load(cpus());
    setup.disk(500);
    until_waiting(&setup, &collector, &only("load", 2.0));
    setup.load(0.0);
    setup.until("tasks", |rows| rows.iter().all(|row| row[4] == "running"));
    let sent = collector.requests().len();
    collector.until_count(&setup, sent + 2);
    let request = collector.requests().pop().unwrap();
    assert_eq!(waiting(&request), only("none", 0.0));
}

#[test]
fn a_room_that_cannot_be_measured_counts_its_waiting_task_as_unmeasured_and_logs_why() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-room-unmeasured");
    setup.metrics(&collector, ROOM);
    fs::remove_file(setup.dir.join("proc/meminfo")).unwrap();
    setup.publish(&requested(42));

    setup.start();

    let request = until_waiting(&setup, &collector, &only("unmeasured", 1.0));
    assert!(!names(&request).contains(&"priory.room.load".to_owned()));
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert!(
        log.contains("priory daemon: metrics: cannot measure the room: "),
        "{log}"
    );
}

#[test]
fn a_daemon_sends_the_grants_by_state_and_the_runs_promise_without_the_held_memory() {
    let collector = Collector::start();
    let mut setup = Setup::new("metrics-room-grants");
    setup.metrics(&collector, ROOM);
    setup.hold("hold-stop");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1);
    setup.hold("hold-a");
    setup.hold("hold-b");
    let a = acquire(&setup, "30", &held("a"));
    setup.until("memory", |rows| rows.len() == 1 && rows[0][2] == "held");
    let b = acquire(&setup, "40", &held("b"));
    setup.until("memory", |rows| rows.len() == 2);

    let of = |state: &str| json!({ "priory.grant.state": state });
    let both = |request: &Value| {
        ["waiting", "held"]
            .iter()
            .all(|state| point(request, "priory.room.grants", &of(state)) == Some(1.0))
    };
    let requests = collector.until(&setup, |requests| requests.iter().any(both));
    for state in ["hold-a", "hold-b", "hold-stop"] {
        setup.release(state);
    }
    assert!(a.wait_with_output().unwrap().status.success());
    assert!(b.wait_with_output().unwrap().status.success());

    let request = requests.iter().find(|request| both(request)).unwrap();
    let memory = |state| point(request, "priory.room.grants.memory", &of(state));
    assert_eq!(memory("held"), Some(30.0 * GB));
    assert_eq!(memory("waiting"), Some(40.0 * GB));
    assert_eq!(
        point(request, "priory.room.memory.promised", &json!({})),
        Some(4.0 * GB)
    );
}

#[test]
fn a_task_waiting_for_room_costs_no_second_measurement_a_tick_with_the_room_group_on() {
    for groups in ["\"log\"", "\"log\", \"room\""] {
        let collector = Collector::start();
        let mut setup = Setup::new("metrics-room-once");
        setup.metrics(
            &collector,
            &format!("metrics = [{groups}]\nmetrics_every = \"10s\"\n"),
        );
        setup.memory(5);
        setup.publish(&requested(42));
        setup.start();

        // The log's probe times the first read of each tick, so its count is the ticks.
        let ticks = |request: &Value| {
            points(request, "priory.log.probe.duration")
                .iter()
                .map(|(_, count)| count)
                .sum::<f64>()
        };
        let requests = collector.until(&setup, |requests| {
            requests.last().is_some_and(|last| ticks(last) >= 15.0)
        });

        let measured = fake_log(&setup, "df.log").len() as f64;
        let ticks = ticks(requests.last().unwrap());
        assert!(
            measured >= 10.0 && measured <= ticks + 5.0,
            "{groups}: {measured} measurements in {ticks} ticks"
        );
    }
}
