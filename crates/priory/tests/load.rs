//! A load test: one daemon state with hundreds of tasks, thousands of inbox events, hundreds of
//! asks and thousands of trace records. It prints how long each read and routing step takes and
//! fails when one passes its budget. Run it with:
//!
//! ```sh
//! cargo test -p priory --test load --release -- --ignored --nocapture
//! LOAD_SCALE=10 cargo test -p priory --test load --release -- --ignored --nocapture
//! ```
//!
//! `LOAD_SCALE` multiplies every count. The budgets do not scale: they hold at ten times the load.

use herdr_bus::{Bus, Message, Output};
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output as Exit, Stdio};
use std::time::{Duration, Instant};

const NEW_ISSUES: u64 = 100;

struct Counts {
    tasks: i64,
    inbox_events: i64,
    asks: i64,
    trace_records: i64,
}

fn counts() -> Counts {
    let scale = std::env::var("LOAD_SCALE")
        .map(|scale| scale.parse().expect("LOAD_SCALE is a whole number"))
        .unwrap_or(1);
    Counts {
        tasks: 500 * scale,
        inbox_events: 5_000 * scale,
        asks: 300 * scale,
        trace_records: 2_000 * scale,
    }
}

const FACTORY: &str =
    "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n";
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
const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
band = "today"
"#;
const PRICES: &str = "[m1]\ninput = 1.0\noutput = 2.0\ncache_read = 0.1\ncache_write = 1.25\n";
const OUTPUT: &str = "factory.intake";
const TOKEN: &str = "s3cret-token";
const NOW: &str = "2026-11-17T11:00:00Z";

/// The longest each measurement may take, in milliseconds, on a release build.
const BUDGETS: [(&str, u128); 9] = [
    ("daemon tick", 1_000),
    ("route 100 issues", 3_000),
    ("desk --json", 500),
    ("report attention", 500),
    ("report costs", 500),
    ("GET /api/asks", 500),
    ("GET /api/tasks", 500),
    ("GET /api/events wake", 250),
    ("revision bump, one write", 50),
];

struct Load {
    dir: PathBuf,
    server: Option<Child>,
}

impl Load {
    fn new() -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("load");
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", FACTORY),
            ("factory/prices.toml", PRICES),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
            ("factory/triggers/agent-label.toml", TRIGGER),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let load = Load { dir, server: None };
        load.bus().declare(&Output::new(OUTPUT)).unwrap();
        succeeded(&load.priory(&["tasks"]));
        load
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("priory.db")).unwrap()
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .env_remove("TMUX")
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("TEST_TOKEN", TOKEN)
            .env("TZ", "UTC")
            .env("PRIORY_NOW", NOW);
        command
    }

    fn priory(&self, args: &[&str]) -> Exit {
        self.command().args(args).output().unwrap()
    }

    /// How long `priory <args>` takes, which must succeed.
    fn time(&self, args: &[&str]) -> Duration {
        let start = Instant::now();
        succeeded(&self.priory(args));
        start.elapsed()
    }

    fn seed(&self) {
        let Counts {
            tasks,
            inbox_events,
            asks,
            trace_records,
        } = counts();
        let mut db = self.db();
        let tx = db.transaction().unwrap();
        for id in 1..=tasks {
            let state = if id <= asks { "asleep" } else { "done" };
            tx.execute(
                "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source)
                 VALUES (?1, 'issue-' || ?1, 'implement', 'implement', ?2, 'lane-' || (?1 % 5),
                         'cli', 'cli-' || ?1)",
                rusqlite::params![id, state],
            )
            .unwrap();
        }
        for id in 1..=asks {
            let answered = id % 3 == 0;
            tx.execute(
                "INSERT INTO asks (id, task, step, kind, question, options, created)
                 VALUES (?1, ?1, 'implement', 'agent', 'Which?', '[]',
                         strftime('%Y-%m-%dT%H:%M:%SZ', '2026-11-16T08:00:00Z', ?2))",
                rusqlite::params![id, format!("+{id} minutes")],
            )
            .unwrap();
            if answered {
                tx.execute(
                    "UPDATE asks SET answer = 'yes', answered = '2026-11-16T20:00:00Z' WHERE id = ?1",
                    [id],
                )
                .unwrap();
            }
        }
        for seq in 1..=inbox_events {
            tx.execute(
                "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome)
                 VALUES (lower(hex(randomblob(16))), 'github.issues.labeled', 'owner/repo#' || ?1,
                         '2026-11-16T09:00:00Z', json_object('issue', ?1), '[]', ?2, 'ignored')",
                rusqlite::params![seq, 1 + seq % tasks],
            )
            .unwrap();
        }
        for number in 1..=trace_records {
            let data = json!({
                "workflow": "implement", "task": (1 + number % tasks).to_string(),
                "step": "implement", "decider": "model", "outcome": "passed", "model": "m1",
                "machine": "box",
                "tokens": { "input": 1000, "output": 500, "cache_read": 0, "cache_write": 0 },
            });
            tx.execute(
                "INSERT INTO trace (id, type, time, event) VALUES
                 (lower(hex(randomblob(16))), 'factory.step.ended',
                  strftime('%Y-%m-%dT%H:%M:%S.000Z', '2026-11-16T00:00:00Z', ?1),
                  json_object('data', json(?2)))",
                rusqlite::params![format!("+{} seconds", number * 30), data.to_string()],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    fn publish_new_issues(&self) {
        let bus = self.bus();
        for issue in 10_000..10_000 + NEW_ISSUES {
            let source = Message::new(
                "github.issues.labeled",
                json!({ "repo": "owner/repo", "issue": issue, "label": "agent",
                        "github_id": 100 + issue, "baseline": false,
                        "triggers": ["agent-label"] }),
            )
            .subject(format!("owner/repo#{issue}"));
            let key = format!("github:owner/repo#{issue}");
            let requested = Message::new(
                "factory.task.requested",
                json!({ "workflow": "implement", "key": key, "band": "today", "lane": null,
                        "trigger": "agent-label", "source": source.id.to_string() }),
            )
            .subject(key);
            bus.publish(OUTPUT, &source).unwrap();
            bus.publish(OUTPUT, &requested).unwrap();
        }
    }

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

impl Drop for Load {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

fn succeeded(output: &Exit) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn get(address: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
    response
        .split_once("\r\n\r\n")
        .map_or("", |(_, body)| body)
        .to_owned()
}

fn timed_get(address: &str, path: &str) -> Duration {
    let start = Instant::now();
    get(address, path);
    start.elapsed()
}

/// The slowest of `runs` runs, which is what a budget guards.
fn slowest(runs: usize, mut run: impl FnMut() -> Duration) -> Duration {
    (0..runs).map(|_| run()).max().unwrap()
}

#[test]
#[ignore = "a load test: run it with --ignored, on a release build"]
fn the_daemon_reads_and_routes_within_budget_on_a_loaded_state() {
    let mut load = Load::new();
    load.seed();
    let factory = load.factory();
    let mut measured: Vec<(&str, Duration)> = Vec::new();

    measured.push((
        "daemon tick",
        slowest(3, || load.time(&["daemon", &factory, "--once"])),
    ));
    load.publish_new_issues();
    measured.push((
        "route 100 issues",
        load.time(&["daemon", &factory, "--once"]),
    ));
    measured.push(("desk --json", slowest(3, || load.time(&["desk", "--json"]))));
    measured.push((
        "report attention",
        slowest(3, || {
            load.time(&["report", "attention", "--since", "2026-11-10T00:00:00Z"])
        }),
    ));
    measured.push((
        "report costs",
        slowest(3, || load.time(&["costs", &factory])),
    ));

    let address = load.serve();
    measured.push((
        "GET /api/asks",
        slowest(5, || timed_get(&address, "/api/asks")),
    ));
    measured.push((
        "GET /api/tasks",
        slowest(5, || timed_get(&address, "/api/tasks")),
    ));

    let revision = revision(&load);
    let waiting = {
        let address = address.clone();
        std::thread::spawn(move || {
            get(
                &address,
                &format!("/api/events?since={revision}&timeout=10"),
            );
            Instant::now()
        })
    };
    std::thread::sleep(Duration::from_millis(500));
    let db = load.db();
    let written = Instant::now();
    db.execute("UPDATE tasks SET lane = 'lane-9' WHERE id = 1", [])
        .unwrap();
    let committed = Instant::now();
    let write = committed - written;
    measured.push((
        "GET /api/events wake",
        waiting.join().unwrap().saturating_duration_since(committed),
    ));
    measured.push(("revision bump, one write", write));

    println!("\n{:<28}{:>10}{:>10}", "measure", "ms", "budget");
    let mut over = Vec::new();
    for (name, took) in &measured {
        let budget = BUDGETS.iter().find(|(budget, _)| budget == name).unwrap().1;
        println!("{name:<28}{:>10}{budget:>10}", took.as_millis());
        if took.as_millis() > budget {
            over.push(*name);
        }
    }
    assert!(over.is_empty(), "over budget: {over:?}");
}

fn revision(load: &Load) -> i64 {
    load.db()
        .query_row("SELECT value FROM revision", [], |row| row.get(0))
        .unwrap()
}
