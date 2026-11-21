mod tmux_dir;

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;
use tmux_dir::TmuxDir;

const WORKFLOW: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\n+++\n";

const SERVE: &str =
    "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n";

const PRICES: &str = "[m1]\ninput = 1.0\noutput = 2.0\ncache_read = 0.1\ncache_write = 1.25\n";

const TOKEN: &str = "s3cret-token";

/// Tuesday noon in Paris, where the week started on Monday 00:00, Sunday 23:00 UTC.
const NOW: &str = "2026-11-17T11:00:00Z";

/// A factory with prices, a state file and Priory home under the test's own directory, on a clock
/// fixed at `NOW` in Paris. Dropping it kills the server it started.
struct Setup {
    dir: PathBuf,
    server: Option<Child>,
    tmux: TmuxDir,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("costs")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", SERVE),
            ("factory/prices.toml", PRICES),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
            server: None,
        };
        succeeded(&setup.priory(&["tasks"]));
        setup
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("TEST_TOKEN", TOKEN)
            .env("TZ", "Europe/Paris")
            .env("PRIORY_NOW", NOW);
        command
    }

    fn priory(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    /// `priory costs FACTORY <args>`, which must succeed, as its standard output.
    fn costs(&self, args: &[&str]) -> String {
        let factory = self.factory();
        let output = self.priory(&[&["costs", &factory][..], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    fn costs_json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.costs(&[args, &["--json"]].concat())).unwrap()
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("priory.db")).unwrap()
    }

    /// Appends a model step's record at `time`, with `tokens` of `model`, or none.
    fn step(&self, time: &str, model: &str, tokens: Option<[i64; 4]>) {
        let mut data = json!({
            "workflow": "implement", "task": "1", "step": "implement", "decider": "model",
            "outcome": "passed", "model": model, "machine": "box",
        });
        if let Some([input, output, cache_read, cache_write]) = tokens {
            data["tokens"] = json!({
                "input": input, "output": output, "cache_read": cache_read,
                "cache_write": cache_write,
            });
        }
        self.db()
            .execute(
                "INSERT INTO trace (id, type, time, event) VALUES
                 (lower(hex(randomblob(16))), 'factory.step.ended', ?1, json_object('data', json(?2)))",
                [time, &data.to_string()],
            )
            .unwrap();
    }

    /// Starts `priory serve` and returns the address it prints once it listens.
    fn serve(&mut self) -> String {
        let mut child = self
            .command()
            .args(["serve", &self.factory()])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        self.server = Some(child);
        loop {
            let mut line = String::new();
            assert!(stderr.read_line(&mut line).unwrap() > 0, "serve exited");
            if let Some(address) = line.trim().strip_prefix("priory: serving on http://") {
                return address.to_owned();
            }
        }
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn costs_compare_this_week_from_monday_in_the_local_timezone_with_last_week() {
    let setup = Setup::new("weeks");
    // Monday 00:30 in Paris, so this week; then Sunday 23:30 in Paris, so last week.
    setup.step(
        "2026-11-15T23:30:00.000Z",
        "m1",
        Some([1_000_000, 500_000, 0, 0]),
    );
    setup.step("2026-11-15T22:30:00.000Z", "m1", Some([3_000_000, 0, 0, 0]));
    setup.step("2026-11-14T10:00:00.000Z", "m1", Some([0, 1_000_000, 0, 0]));
    // The week before last, and a record after now, count in neither.
    setup.step("2026-11-08T22:30:00.000Z", "m1", Some([9_000_000, 0, 0, 0]));
    setup.step("2026-11-17T12:00:00.000Z", "m1", Some([9_000_000, 0, 0, 0]));

    let costs = setup.costs_json(&[]);

    assert_eq!(
        [&costs["current"], &costs["previous"]],
        [
            &json!({ "since": "2026-11-15T23:00:00.000Z", "until": "2026-11-17T11:00:00.000Z",
                     "cost_usd": 2.0, "tokens": { "input": 1_000_000, "output": 500_000,
                     "cache_read": 0, "cache_write": 0 }, "unpriced": [], "unknown": 0,
                     "merges": 0 }),
            &json!({ "since": "2026-11-08T23:00:00.000Z", "until": "2026-11-15T23:00:00.000Z",
                     "cost_usd": 5.0, "tokens": { "input": 3_000_000, "output": 1_000_000,
                     "cache_read": 0, "cache_write": 0 }, "unpriced": [], "unknown": 0,
                     "merges": 0 }),
        ]
    );
    let days: Vec<_> = costs["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|day| {
            (
                day["day"].as_str().unwrap(),
                day["cost_usd"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        days,
        [
            ("2026-11-14", 2.0),
            ("2026-11-15", 3.0),
            ("2026-11-16", 2.0)
        ]
    );
    let text = setup.costs(&[]);
    assert!(
        text.contains(
            "current\t2026-11-15T23:00:00.000Z\t2026-11-17T11:00:00.000Z\t$2.000000\t-\t0\n"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "previous\t2026-11-08T23:00:00.000Z\t2026-11-15T23:00:00.000Z\t$5.000000\t-\t0\n"
        ),
        "{text}"
    );
    assert!(text.contains("2026-11-15\t$3.000000\t-\t0\n"), "{text}");
}

#[test]
fn costs_by_model_price_each_model_and_list_unpriced_tokens_apart() {
    let setup = Setup::new("by-model");
    setup.step(
        "2026-11-16T09:00:00.000Z",
        "m1",
        Some([1_000_000, 0, 1_000_000, 0]),
    );
    setup.step("2026-11-16T10:00:00.000Z", "m2", Some([1_000, 500, 0, 0]));
    setup.step("2026-11-16T11:00:00.000Z", "m1", None);

    let costs = setup.costs_json(&["--by", "model"]);

    let tokens = |input, output, cache_read| json!({ "input": input, "output": output, "cache_read": cache_read, "cache_write": 0 });
    assert_eq!(
        costs["split"],
        json!([
            { "model": "m1", "cost_usd": 1.1, "tokens": tokens(1_000_000, 0, 1_000_000),
              "unpriced": [], "unknown": 1 },
            { "model": "m2", "cost_usd": null, "tokens": tokens(1_000, 500, 0),
              "unpriced": [{ "model": "m2", "tokens": tokens(1_000, 500, 0) }], "unknown": 0 },
        ])
    );
    assert_eq!(costs["current"]["cost_usd"], 1.1);
    assert_eq!(
        costs["current"]["unpriced"],
        json!([{ "model": "m2", "tokens": tokens(1_000, 500, 0) }])
    );
    let text = setup.costs(&["--by", "model"]);
    assert!(
        text.contains("model\tcost\tnotes\nm1\t$1.100000\ttokens unknown for 1 records\nm2\t-\tprice missing for m2: 1500 tokens\n"),
        "{text}"
    );
}

#[test]
fn merges_per_day_count_only_the_merged_pull_requests_a_task_owned() {
    let setup = Setup::new("merges");
    let db = setup.db();
    // The owning tasks matter only through `owns`, so they are left out. Task 3 owns its branch,
    // as a worker's task does, and #1's second merged close is the same merge.
    db.execute_batch(
        "PRAGMA foreign_keys = OFF;
         INSERT INTO owns (task, resource) VALUES (1, 'github:o/r#1'), (1, 'github:o/r#2'),
           (2, 'github:o/r#4'), (3, 'github:o/r@priory/x');
         INSERT INTO inbox (id, type, subject, time, data, links, outcome) VALUES
           ('m1', 'github.pull_request.closed', 'o/r#1', '2026-11-16T09:00:00Z',
            '{\"merged\":true}', '[]', 'stored'),
           ('m2', 'github.pull_request.closed', 'o/r#2', '2026-11-16T09:00:00Z',
            '{\"merged\":false}', '[]', 'stored'),
           ('m3', 'github.pull_request.closed', 'o/r#3', '2026-11-16T09:00:00Z',
            '{\"merged\":true}', '[]', 'stored'),
           ('m4', 'github.pull_request.closed', 'o/r#4', '2026-11-15T22:30:00Z',
            '{\"merged\":true}', '[]', 'stored'),
           ('m5', 'github.pull_request.closed', 'o/r#5', '2026-11-16T10:00:00Z',
            '{\"merged\":true,\"repo\":\"o/r\",\"head_branch\":\"priory/x\"}', '[]', 'stored'),
           ('m6', 'github.pull_request.closed', 'o/r#1', '2026-11-17T08:00:00Z',
            '{\"merged\":true}', '[]', 'stored');",
    )
    .unwrap();

    let costs = setup.costs_json(&[]);

    assert_eq!(
        [&costs["current"]["merges"], &costs["previous"]["merges"]],
        [2, 1]
    );
    let days: Vec<_> = costs["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|day| {
            (
                day["day"].as_str().unwrap(),
                day["merges"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(days, [("2026-11-15", 1), ("2026-11-16", 2)]);
    assert_eq!(costs["days"][0]["cost_usd"], 0.0);
}

/// `GET path` with the token, as the status code and body.
fn get(address: &str, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, body.to_owned())
}

#[test]
fn the_costs_endpoint_answers_what_costs_json_prints() {
    let mut setup = Setup::new("endpoint");
    setup.step("2026-11-16T09:00:00.000Z", "m1", Some([1_000_000, 0, 0, 0]));
    setup.step("2026-11-10T09:00:00.000Z", "m2", Some([1_000, 0, 0, 0]));
    let address = setup.serve();

    for (path, args) in [
        ("/api/costs", vec![]),
        (
            "/api/costs?since=3%20days&by=machine",
            vec!["--since", "3 days", "--by", "machine"],
        ),
    ] {
        let (status, body) = get(&address, path);
        assert_eq!(status, 200, "{path}: {body}");
        let served: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(served, setup.costs_json(&args), "{path}");
    }
    for path in [
        "/api/costs?by=colour",
        "/api/costs?since=soon",
        "/api/costs?who=me",
        "/api/costs?since=%zz",
        "/api/costs?since=2026-11-18T00:00:00Z",
        "/api/costs?since=2026",
    ] {
        assert_eq!(get(&address, path).0, 400, "{path}");
    }
}
