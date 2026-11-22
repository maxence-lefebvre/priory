//! `priory patterns`: the day's stopped, repaired and marked-wrong tasks grouped into failure
//! patterns, shown on the desk, closed and dismissed.

#[allow(dead_code)]
mod harness;

mod tmux_dir;

use herdr_bus::{Bus, Message, Output as BusOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use tmux_dir::TmuxDir;

const WORKFLOW: &str = r#"
start = "plan"

[[steps]]
id = "plan"
decides = "human"
effect = "none"
options = ["go"]
next = { go = "implement" }

[[steps]]
id = "implement"
decides = "human"
effect = "none"
options = ["done"]
next = { done = "done" }
"#;

const OUTPUT: &str = "factory.intake";
const TESTS_FAILED: &str = "cargo test failed: 3 tests failed in 41s";
const TESTS_FAILED_AGAIN: &str = "cargo test failed: 12 tests failed in 9s";

struct Setup {
    dir: PathBuf,
    tmux: TmuxDir,
}

impl Setup {
    /// A factory with `tasks` tasks of workflow `implement`, numbered from 1.
    fn new(name: &str, tasks: u64) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("patterns")
            .join(name);
        priory_runtime::runtime::remove_sealed(&dir);
        for (path, content) in [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/workflows/implement.toml", WORKFLOW),
        ] {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
        };
        let bus = Bus::open(&setup.dir.join("bus.db")).unwrap();
        bus.declare(&BusOutput::new(OUTPUT)).unwrap();
        for task in 1..=tasks {
            let key = format!("cli:task-{task}");
            let message = Message::new(
                "factory.task.requested",
                json!({ "workflow": "implement", "key": key, "band": null, "lane": null,
                        "trigger": "cli", "source": "source" }),
            )
            .subject(key);
            bus.publish(OUTPUT, &message).unwrap();
        }
        succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
        setup
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn priory(&self, args: &[&str]) -> Output {
        harness::priory()
            .env_remove("TMUX")
            .env_remove("PRIORY_NOW")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .args(args)
            .output()
            .unwrap()
    }

    fn sql(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) {
        rusqlite::Connection::open(self.dir.join("priory.db"))
            .unwrap()
            .execute(sql, params)
            .unwrap();
    }

    /// Stops `task` as `state` at `step` with `reason`, `minutes` minutes ago.
    fn stop(&self, task: i64, state: &str, step: &str, reason: &str, minutes: i64) {
        self.sql(
            "UPDATE tasks SET state = ?2, step = ?3, reason = ?4 WHERE id = ?1",
            &[&task, &state, &step, &reason],
        );
        self.sql(
            "UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2) WHERE id = ?1",
            &[&task, &format!("-{minutes} minutes")],
        );
    }

    /// `priory patterns find` on the factory, with `args` after it.
    fn find(&self, args: &[&str]) -> String {
        let factory = self.factory();
        let output = self.priory(&[&["patterns", "find", &factory][..], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    /// `priory patterns --json`, with `--all` when `all`.
    fn listed(&self, all: bool) -> Vec<Value> {
        let args: &[&str] = if all {
            &["patterns", "--all", "--json"]
        } else {
            &["patterns", "--json"]
        };
        let output = self.priory(args);
        succeeded(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// The pattern the listing names for `step`, with every pattern listed.
    fn pattern(&self, step: &str) -> Value {
        self.listed(true)
            .into_iter()
            .find(|pattern| pattern["step"] == step)
            .unwrap_or_else(|| panic!("no pattern at {step}"))
    }

    /// The desk's `pattern` items.
    fn desk(&self) -> Vec<Value> {
        let output = self.priory(&["desk", "--json"]);
        succeeded(&output);
        let desk: Value = serde_json::from_slice(&output.stdout).unwrap();
        desk["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["kind"] == "pattern")
            .cloned()
            .collect()
    }

    /// `priory patterns VERB FACTORY ID`.
    fn close(&self, verb: &str, id: &str) -> Output {
        self.priory(&["patterns", verb, &self.factory(), id])
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
fn find_groups_the_days_stopped_tasks_by_step_and_the_shape_of_their_reason() {
    let setup = Setup::new("grouped", 4);
    setup.stop(1, "failed", "implement", TESTS_FAILED, 2);
    setup.stop(2, "failed", "implement", TESTS_FAILED_AGAIN, 1);
    setup.stop(
        3,
        "blocked",
        "implement",
        "a permission prompt asked for Bash(rm)",
        1,
    );
    setup.stop(4, "failed", "plan", TESTS_FAILED, 1);

    let found = setup.find(&[]);

    assert!(found.starts_with("4 new matches from "), "{found}");
    let listed = setup.listed(false);
    assert_eq!(listed.len(), 1, "{listed:?}");
    let pattern = &listed[0];
    assert_eq!(
        [
            &pattern["workflow"],
            &pattern["step"],
            &pattern["state"],
            &pattern["tasks"],
            &pattern["text"],
            &pattern["signature"]
        ],
        [
            &json!("implement"),
            &json!("implement"),
            &json!("open"),
            &json!([1, 2]),
            &json!(TESTS_FAILED_AGAIN),
            &json!("cargo test failed: # tests failed in #s")
        ]
    );
    assert_eq!(setup.listed(true).len(), 3);
    let id = pattern["id"].as_str().unwrap();
    let desk = setup.desk();
    assert_eq!(desk.len(), 1, "{desk:?}");
    assert_eq!(desk[0]["id"], id);
    assert_eq!(desk[0]["task"], Value::Null);
    assert_eq!(
        desk[0]["what"],
        format!(
            "implement/implement: 2 tasks stopped for one reason: {TESTS_FAILED_AGAIN}; see it with `priory patterns show {id}`"
        )
    );
    let output = setup.priory(&["desk", "--json"]);
    let ranked: Value = serde_json::from_slice(&output.stdout).unwrap();
    let ranked = ranked["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "pattern")
        .unwrap();
    assert_eq!(ranked["title"], "tasks stopped for one reason");
    assert_eq!(ranked["why"], "it waits on a person");
}

#[test]
fn find_keeps_no_secret_of_a_reason_in_the_pattern_or_the_trace() {
    let setup = Setup::new("secret", 1);
    let token = "ghp_abcdefghijklmnopqrstuvwxyz";
    setup.stop(
        1,
        "failed",
        "implement",
        &format!("git push failed for {token}"),
        1,
    );

    setup.find(&[]);

    let pattern = setup.pattern("implement");
    assert!(!pattern.to_string().contains(token), "{pattern}");
    let trace: Vec<String> = rusqlite::Connection::open(setup.dir.join("priory.db"))
        .unwrap()
        .prepare("SELECT event FROM trace")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        trace.iter().all(|event| !event.contains(token)),
        "{trace:?}"
    );
}

#[test]
fn find_records_each_stop_once_and_a_new_task_grows_its_pattern() {
    let setup = Setup::new("grows", 3);
    setup.stop(1, "failed", "implement", TESTS_FAILED, 2);
    setup.stop(2, "failed", "implement", TESTS_FAILED_AGAIN, 1);
    setup.find(&[]);

    let again = setup.find(&[]);
    setup.stop(3, "failed", "implement", TESTS_FAILED, 0);
    let grown = setup.find(&[]);

    assert!(again.starts_with("0 new matches from "), "{again}");
    assert_eq!(again.lines().count(), 1, "{again}");
    let id = setup.pattern("implement")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        grown.lines().nth(1),
        Some(format!("{id}\topen\timplement/implement\t+1, 3 tasks\t{TESTS_FAILED}").as_str()),
        "{grown}"
    );
    assert_eq!(setup.pattern("implement")["tasks"], json!([1, 2, 3]));
    assert_eq!(setup.listed(true).len(), 1);
}

#[test]
fn find_reads_only_its_window() {
    let setup = Setup::new("window", 1);
    setup.stop(1, "failed", "implement", TESTS_FAILED, 3 * 24 * 60);

    let day = setup.find(&[]);
    let week = setup.find(&["--since", "7 days"]);

    assert!(day.starts_with("0 new matches from "), "{day}");
    assert!(week.starts_with("1 new match from "), "{week}");
}

#[test]
fn repairs_and_marks_join_patterns_and_a_cleared_mark_does_not() {
    let setup = Setup::new("repairs-and-marks", 5);
    for (task, free) in [(1, "10.0"), (2, "8.5")] {
        let id = format!("repair-{task}");
        let event = json!({ "id": id, "type": "factory.task.repaired",
                            "time": "2026-01-01T00:00:00Z",
                            "data": { "workflow": "implement", "task": task.to_string(),
                                      "key": format!("cli:task-{task}"), "generation": 1,
                                      "step": "implement",
                                      "cause": format!("disk full ({free} GB of disk free, keeps 20.0 GB free)"),
                                      "action": "sweep", "outcome": "ready" } });
        setup.sql(
            "INSERT INTO trace (id, type, time, event)
             VALUES (?1, 'factory.task.repaired', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?2)",
            &[&id, &event.to_string()],
        );
    }
    let factory = setup.factory();
    for task in ["3", "4", "5"] {
        succeeded(&setup.priory(&[
            "label",
            &factory,
            task,
            "wrong",
            "--evidence",
            "https://github.com/owner/repo/pull/50",
        ]));
    }
    succeeded(&setup.priory(&["label", &factory, "5", "wrong", "--clear"]));

    setup.find(&[]);

    let repaired = setup.pattern("implement");
    assert_eq!(repaired["tasks"], json!([1, 2]));
    assert_eq!(
        repaired["signature"],
        "disk full (# gb of disk free, keeps # gb free)"
    );
    let marked = setup.pattern("plan");
    assert_eq!(marked["tasks"], json!([3, 4]));
    assert_eq!(marked["text"], "marked wrong by a person");
    let shown = setup.priory(&["patterns", "show", repaired["id"].as_str().unwrap()]);
    succeeded(&shown);
    let shown = String::from_utf8(shown.stdout).unwrap();
    assert!(
        shown.starts_with(&format!(
            "pattern {}: open, implement/implement, 2 tasks, last ",
            repaired["id"].as_str().unwrap()
        )),
        "{shown}"
    );
    assert!(
        shown.contains("\ntask\tkey\tkind\thappened\ttext\n1\tcli:task-1\trepaired\t"),
        "{shown}"
    );
}

#[test]
fn a_closed_pattern_leaves_the_desk_until_a_task_stops_after_the_close() {
    let setup = Setup::new("closed", 4);
    setup.stop(1, "failed", "implement", TESTS_FAILED, 2);
    setup.stop(2, "failed", "implement", TESTS_FAILED_AGAIN, 1);
    setup.find(&[]);
    let id = setup.pattern("implement")["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let closed = setup.close("close", &id);
    let twice = setup.close("close", &id);
    // Found after the close, but it stopped before it.
    setup.stop(4, "failed", "implement", TESTS_FAILED, 30);
    setup.find(&[]);
    let still = setup.pattern("implement");
    let desk_while_closed = setup.desk();
    setup.stop(3, "failed", "implement", TESTS_FAILED, 0);
    setup.find(&[]);

    succeeded(&closed);
    assert_eq!(
        String::from_utf8_lossy(&closed.stdout),
        format!("pattern {id} closed: the next task that matches it opens it again\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&twice.stderr),
        format!("priory patterns close: pattern {id} is already closed\n")
    );
    assert_eq!(still["state"], "closed");
    assert_eq!(still["tasks"], json!([1, 2, 4]));
    assert_eq!(desk_while_closed, Vec::<Value>::new());
    assert_eq!(setup.pattern("implement")["state"], "open");
    let desk = setup.desk();
    assert_eq!(desk.len(), 1, "{desk:?}");
    assert!(
        desk[0]["what"]
            .as_str()
            .unwrap()
            .starts_with("implement/implement: 4 tasks stopped for one reason: "),
        "{desk:?}"
    );
}

#[test]
fn a_dismissed_pattern_never_comes_back() {
    let setup = Setup::new("dismissed", 3);
    setup.stop(1, "failed", "implement", TESTS_FAILED, 2);
    setup.stop(2, "failed", "implement", TESTS_FAILED_AGAIN, 1);
    setup.find(&[]);
    let id = setup.pattern("implement")["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let dismissed = setup.close("dismiss", &id);
    setup.stop(3, "failed", "implement", TESTS_FAILED, 0);
    setup.find(&[]);
    let again = setup.close("close", &id);
    let unknown = setup.priory(&["patterns", "show", "ffffffff"]);

    succeeded(&dismissed);
    assert_eq!(
        String::from_utf8_lossy(&dismissed.stdout),
        format!("pattern {id} dismissed: no task brings it back\n")
    );
    let pattern = setup.pattern("implement");
    assert_eq!(pattern["state"], "dismissed");
    assert_eq!(pattern["tasks"], json!([1, 2, 3]));
    assert_eq!(setup.desk(), Vec::<Value>::new());
    assert_eq!(setup.listed(false), Vec::<Value>::new());
    assert_eq!(
        String::from_utf8_lossy(&again.stderr),
        format!("priory patterns close: pattern {id} is already dismissed\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&unknown.stderr),
        "priory patterns show: no pattern ffffffff: list them with `priory patterns --all`\n"
    );
}
