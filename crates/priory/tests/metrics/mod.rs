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
struct Collector {
    url: String,
    received: Arc<Mutex<Vec<Value>>>,
    answers: Arc<Mutex<VecDeque<u16>>>,
}

impl Collector {
    fn start() -> Collector {
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
    fn until(&self, setup: &Setup, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
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

/// Each data point of metric `name` in an OTLP metrics request, as its attributes and its value.
fn points(request: &Value, name: &str) -> Vec<(Value, f64)> {
    let metrics = request["body"]["resourceMetrics"][0]["scopeMetrics"][0]["metrics"]
        .as_array()
        .unwrap();
    metrics
        .iter()
        .filter(|metric| metric["name"] == name)
        .flat_map(|metric| {
            let data = ["gauge", "sum"]
                .into_iter()
                .find_map(|kind| metric.get(kind))
                .unwrap();
            data["dataPoints"].as_array().unwrap().clone()
        })
        .map(|point| {
            let value = point
                .get("asDouble")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| point["asInt"].as_str().unwrap().parse::<f64>().unwrap());
            (attributes(&point["attributes"]), value)
        })
        .collect()
}

/// The names of the metrics in a request.
fn names(request: &Value) -> Vec<String> {
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

fn resource(request: &Value) -> Value {
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

    let output = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["daemon", setup.dir.join("factory").to_str().unwrap()])
        .env("PRIORY_METRICS", "process,nope")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory daemon: PRIORY_METRICS names `nope`, which is not a metrics group: the groups are \
         process, or none\n"
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
