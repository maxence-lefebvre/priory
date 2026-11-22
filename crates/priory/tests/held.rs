mod tmux_dir;

use herdr_bus::{Bus, Message, Output as BusOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

const WORKFLOW: &str = r#"
start = "approve-merge"

[[steps]]
id = "approve-merge"
decides = "human"
effect = "none"
options = ["merge", "drop"]
next = { merge = "done", drop = "stop" }
"#;

const HELD: &str = r#"description = "The change held for a week after its merge."
on = "merge"
kind = "held"
after = "merged + 7d"
fails = ["reverted", "fixed"]
"#;

const OUTPUT: &str = "factory.intake";
const NOW: &str = "2026-11-21T12:00:00Z";

struct Setup {
    dir: PathBuf,
    tmux: TmuxDir,
}

/// The instant `days` days before `NOW`.
fn ago(days: i64) -> String {
    let now: jiff::Timestamp = NOW.parse().unwrap();
    (now - jiff::SignedDuration::from_hours(24 * days)).to_string()
}

impl Setup {
    /// A factory with the scorer `held.toml` and `tasks` tasks, numbered from 1. Task `n` owns the
    /// branch `priory/n-fix`.
    fn new(name: &str, tasks: u64) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("held")
            .join(name);
        priory_runtime::runtime::remove_sealed(&dir);
        for (path, content) in [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/workflows/implement.toml", WORKFLOW),
            ("factory/scorers/held.toml", HELD),
        ] {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
        };
        setup.bus().declare(&BusOutput::new(OUTPUT)).unwrap();
        let requests: Vec<Message> = (1..=tasks)
            .map(|task| {
                let key = format!("cli:task-{task}");
                Message::new(
                    "factory.task.requested",
                    json!({ "workflow": "implement", "key": key, "band": null, "lane": null,
                            "trigger": "cli", "source": "source" }),
                )
                .subject(key)
            })
            .collect();
        setup.handle(&requests);
        for task in 1..=tasks {
            setup.sql(
                "INSERT INTO owns (task, resource) VALUES (?1, 'github:owner/repo@priory/' || ?1 || '-fix')",
                &[&(task as i64)],
            );
        }
        setup
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("PRIORY_NOW", NOW)
            .args(args);
        command
    }

    fn priory(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Publishes `messages`, then lets the daemon route them once.
    fn handle(&self, messages: &[Message]) {
        for message in messages {
            self.bus().publish(OUTPUT, message).unwrap();
        }
        succeeded(&self.priory(&["daemon", &self.factory(), "--once"]));
    }

    fn sql(&self, sql: &str, params: &[&dyn rusqlite::ToSql]) {
        rusqlite::Connection::open(self.dir.join("priory.db"))
            .unwrap()
            .execute(sql, params)
            .unwrap();
    }

    /// The `held_scores` of `priory task ID --json`.
    fn held(&self, task: u64) -> Value {
        let output = self.priory(&["task", &task.to_string(), "--json"]);
        succeeded(&output);
        let task: Value = serde_json::from_slice(&output.stdout).unwrap();
        task["held_scores"].clone()
    }

    /// Each `held_scores` entry of `task` as `(scorer, verdict, reason, happened)`.
    fn graded(&self, task: u64) -> Vec<(String, String, String, String)> {
        self.held(task)
            .as_array()
            .unwrap()
            .iter()
            .map(|score| {
                let field = |name: &str| score[name].as_str().unwrap().to_owned();
                (
                    field("scorer"),
                    field("verdict"),
                    field("reason"),
                    field("happened"),
                )
            })
            .collect()
    }

    /// The `factory.step.labeled` records of `task` that a held scorer wrote, as the trace holds
    /// them.
    fn labeled(&self, task: u64) -> Vec<Value> {
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        let mut statement = db
            .prepare(
                "SELECT event FROM trace WHERE type = 'factory.step.labeled'
                 AND json_extract(event, '$.data.task') = ?1
                 AND json_extract(event, '$.data.label') = 'scored' ORDER BY seq",
            )
            .unwrap();
        statement
            .query_map([task.to_string()], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|event| serde_json::from_str(&event.unwrap()).unwrap())
            .collect()
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An event of pull request `number`, whose head is task `task`'s branch, `days` days ago.
fn on_pull(kind: &str, number: u64, task: u64, days: i64, data: Value) -> Message {
    let mut fields = json!({
        "repo": "owner/repo", "pull_request": number, "head_branch": format!("priory/{task}-fix"),
        "head_sha": format!("h{number}"), "base_branch": "main",
        "open": kind != "github.pull_request.closed",
        "url": format!("https://github.com/owner/repo/pull/{number}"),
        "baseline": false, "triggers": [], "actor": "max", "title": format!("Fix {task}"),
        "body": "", "github_id": number,
    });
    for (field, value) in data.as_object().unwrap() {
        fields[field] = value.clone();
    }
    Message::new(kind, fields)
        .subject(format!("owner/repo#{number}"))
        .time(ago(days).parse().unwrap())
}

fn merged(number: u64, task: u64, days: i64) -> Message {
    on_pull(
        "github.pull_request.closed",
        number,
        task,
        days,
        json!({ "merged": true, "merge_commit": format!("m{number}") }),
    )
}

/// A pull request merged `days` days ago that reverts pull request `number`.
fn revert(number: u64, days: i64) -> Message {
    on_pull(
        "github.pull_request.closed",
        100 + number,
        99,
        days,
        json!({ "merged": true, "merge_commit": format!("m{}", 100 + number),
                "body": format!("Reverts owner/repo#{number}") }),
    )
}

#[test]
fn a_held_scorer_grades_each_merged_task_once_its_week_has_passed() {
    let setup = Setup::new("week", 5);
    setup.handle(&[
        // Merged 10 days ago and reverted on day 3: fails.
        merged(1, 1, 10),
        revert(1, 7),
        // Merged 10 days ago, nothing after: passes.
        merged(2, 2, 10),
        // Merged 20 days ago and reverted on day 10, after its week: passes.
        merged(3, 3, 20),
        revert(3, 10),
        // Merged 2 days ago: its week has not passed.
        merged(4, 4, 2),
    ]);
    // Task 5 never merged.

    assert_eq!(
        setup.graded(1),
        [(
            "held".to_owned(),
            "fail".to_owned(),
            "reverted https://github.com/owner/repo/pull/101".to_owned(),
            ago(3),
        )]
    );
    let pass = |days| {
        vec![(
            "held".to_owned(),
            "pass".to_owned(),
            "no reverted or fixed by merged + 7d".to_owned(),
            ago(days),
        )]
    };
    assert_eq!(setup.graded(2), pass(3));
    assert_eq!(setup.graded(3), pass(13));
    assert_eq!(setup.held(4), json!([]));
    assert_eq!(setup.held(5), json!([]));

    let record = setup.labeled(1).pop().unwrap();
    assert_eq!(record["type"], "factory.step.labeled");
    let data = &record["data"];
    assert_eq!(data["label"], "scored");
    assert_eq!(data["scorer"], "held");
    assert_eq!(data["kind"], "held");
    assert_eq!(data["verdict"], "fail");
    assert_eq!(data["workflow"], "implement");

    // Grading again adds nothing.
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    assert_eq!(setup.graded(1).len(), 1);
    assert_eq!(setup.graded(2).len(), 1);
}

#[test]
fn a_later_task_that_fixes_the_merged_one_within_its_week_fails_it() {
    let setup = Setup::new("fixed", 3);
    // Task 3 fixes task 1 on day 2 of its week and task 2 on day 9.
    for (other, days) in [(1, 8), (2, 1)] {
        setup.sql(
            "INSERT INTO relations (task, kind, other, by, created) VALUES (3, 'fixes', ?1, 'priory', ?2)",
            &[&other, &ago(days)],
        );
    }

    setup.handle(&[merged(1, 1, 10), merged(2, 2, 10)]);

    assert_eq!(
        setup.graded(1),
        [(
            "held".to_owned(),
            "fail".to_owned(),
            "fixed by task 3".to_owned(),
            ago(3),
        )]
    );
    assert_eq!(setup.graded(2)[0].1, "pass");
}

#[test]
fn a_task_with_a_pull_request_still_open_in_its_stack_is_not_graded() {
    let setup = Setup::new("stack", 1);
    setup.sql(
        "INSERT INTO pull_requests (task, resource, branch, base, state)
         VALUES (1, 'github:owner/repo#2', 'priory/1-two', 'priory/1-fix', 'open')",
        &[],
    );

    setup.handle(&[merged(1, 1, 10)]);

    assert_eq!(setup.held(1), json!([]));
}

#[test]
fn priory_score_refuses_a_held_scorer_by_name() {
    let setup = Setup::new("score", 1);

    let output = setup.priory(&["score", &setup.factory(), "1", "--scorer", "held"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory score: held is a held scorer: the daemon grades a task with it at merged + 7d\n"
    );
}

#[test]
fn a_running_daemon_grades_a_task_whose_moment_came() {
    let setup = Setup::new("daemon", 1);
    setup.bus().publish(OUTPUT, &merged(1, 1, 10)).unwrap();

    let mut daemon = setup
        .command(&["daemon", &setup.factory()])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let graded = loop {
        let graded = setup.labeled(1);
        if !graded.is_empty() || Instant::now() > deadline {
            break graded;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    daemon.kill().unwrap();
    let output = daemon.wait_with_output().unwrap();

    assert_eq!(
        graded.len(),
        1,
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(graded[0]["data"]["verdict"], "pass");
}
