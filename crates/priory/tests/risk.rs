mod tmux_dir;

use herdr_bus::{Bus, Message, Output as BusOutput};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tmux_dir::TmuxDir;

const WORKFLOW: &str = r#"
start = "plan"

[[steps]]
id = "plan"
decides = "human"
effect = "none"
options = ["red", "yellow", "green"]
next = { red = "plan", yellow = "approve-pr", green = "approve-pr" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["approved", "rejected"]
next = { approved = "open-pr", rejected = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "approve-merge", failed = "stop" }

[[steps]]
id = "approve-merge"
decides = "human"
effect = "none"
approves = ["merge-pr"]
options = ["merge", "drop"]
next = { merge = "merge-pr", drop = "stop" }

[[steps]]
id = "merge-pr"
decides = "code"
action = "merge-pr"
effect = "irreversible"
next = { merged = "done", failed = "stop" }
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
    /// A factory with `tasks` tasks of workflow `implement`, numbered from 1.
    fn new(name: &str, tasks: u64) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("risk")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
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

    fn priory(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("PRIORY_NOW", NOW)
            .args(args)
            .output()
            .unwrap()
    }

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

    /// A `factory.step.ended` record of `task` at `step` with `outcome`, `days` days ago.
    fn ended(&self, task: u64, step: &str, outcome: &str, days: i64) {
        let effect = match step {
            "open-pr" | "merge-pr" => "irreversible",
            _ => "none",
        };
        let decider = if effect == "irreversible" {
            "code"
        } else {
            "human"
        };
        let time = ago(days);
        let id = format!("{task}-{step}-{days}-{outcome}");
        let event = json!({
            "id": id, "type": "factory.step.ended", "time": time,
            "data": { "factory": "demo", "factory_revision": "r1", "workflow": "implement",
                      "task": task.to_string(), "generation": 1, "step": step,
                      "decider": decider, "effect": effect, "outcome": outcome },
        });
        self.sql(
            "INSERT INTO trace (id, type, time, event) VALUES (?1, 'factory.step.ended', ?2, ?3)",
            &[&id, &time, &event.to_string()],
        );
    }

    /// A task that ended `plan` green and reached `open-pr` `days` days ago.
    fn reached(&self, task: u64, days: i64) {
        self.ended(task, "plan", "green", days + 1);
        self.ended(task, "approve-pr", "approved", days);
        self.ended(task, "open-pr", "opened", days);
    }

    /// A label record of `task`, as a person's answer writes it, `days` days ago.
    fn labeled(&self, task: u64, label: &str, days: Option<i64>) {
        let mut data = json!({ "task": task.to_string(), "workflow": "implement",
                               "label": label, "evidence": "approve-merge: drop" });
        if let Some(days) = days {
            data["happened"] = json!(ago(days));
        }
        let id = format!("label-{task}-{label}-{days:?}");
        let event = json!({ "id": id, "type": "factory.step.labeled", "time": NOW, "data": data });
        self.sql(
            "INSERT INTO trace (id, type, time, event) VALUES (?1, 'factory.step.labeled', ?2, ?3)",
            &[&id, &NOW, &event.to_string()],
        );
    }

    /// Open ask `id` of `task` at human step `step`.
    fn ask(&self, id: i64, task: u64, step: &str) {
        self.sql(
            "INSERT INTO asks (id, task, step, kind, question, options, created)
             VALUES (?1, ?2, ?3, 'step', 'Go?', '[]', ?4)",
            &[&id, &(task as i64), &step, &NOW],
        );
    }

    /// Ask `id` as `priory asks --json` lists it.
    fn listed_ask(&self, id: i64) -> Value {
        let output = self.priory(&["asks", "--json"]);
        succeeded(&output);
        let asks: Value = serde_json::from_slice(&output.stdout).unwrap();
        asks["asks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ask| ask["id"] == id)
            .unwrap()
            .clone()
    }

    fn report(&self, args: &[&str]) -> String {
        let output = self.priory(&[&["report", "risk", &self.factory()][..], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.report(&["--json"])).unwrap()
    }

    /// The target `step` of `--json`.
    fn target(&self, step: &str) -> Value {
        self.json()["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|target| target["step"] == step)
            .unwrap()
            .clone()
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

fn opened(number: u64, task: u64, days: i64) -> Message {
    on_pull("github.pull_request.opened", number, task, days, json!({}))
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

/// The row of `target` for `step` and `outcome`, `None` for its own row.
fn row(target: &Value, step: &str, outcome: Option<&str>) -> Value {
    target["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["step"] == step && row["outcome"].as_str() == outcome)
        .cloned()
        .unwrap_or(Value::Null)
}

/// `trials`, `judged wrong` and `landed wrong` of a row.
fn counts(row: &Value) -> (i64, i64, i64) {
    (
        row["trials"].as_i64().unwrap(),
        row["judged_wrong"]["k"].as_i64().unwrap(),
        row["landed_wrong"]["k"].as_i64().unwrap(),
    )
}

#[test]
fn a_revert_within_the_horizon_is_landed_wrong_and_a_reach_under_30_days_is_pending() {
    let setup = Setup::new("reverted", 4);
    setup.reached(1, 31);
    setup.reached(2, 29);
    setup.reached(3, 170);
    setup.handle(&[
        opened(1, 1, 31),
        merged(1, 1, 30),
        revert(1, 21),
        opened(2, 2, 29),
        merged(2, 2, 28),
        revert(2, 19),
        opened(3, 3, 170),
        merged(3, 3, 169),
        revert(3, 160),
    ]);

    let target = setup.target("open-pr");
    assert_eq!(counts(&row(&target, "open-pr", None)), (2, 2, 2));
    assert_eq!(counts(&row(&target, "plan", Some("green"))), (2, 2, 2));
    assert_eq!(target["reached"], 3);
    assert_eq!(target["pending"], 1);
    assert_eq!(target["adverse"][0]["label"], "reverted");
    assert_eq!(target["adverse"][0]["outcome"], "landed wrong");
}

#[test]
fn a_revert_after_the_horizon_is_late_and_one_on_day_29_stored_later_counts() {
    let setup = Setup::new("late", 2);
    setup.reached(1, 70);
    setup.reached(2, 70);
    setup.handle(&[
        opened(1, 1, 70),
        merged(1, 1, 69),
        revert(1, 39),
        opened(2, 2, 70),
        merged(2, 2, 69),
    ]);
    // Day 29 of task 2, which the daemon only stores now, on day 70.
    setup.handle(&[revert(2, 41)]);

    let target = setup.target("open-pr");
    assert_eq!(counts(&row(&target, "open-pr", None)), (2, 1, 1));
    assert_eq!(target["late"], 1);
    assert_eq!(target["adverse"][0]["task"], "2");
}

#[test]
fn a_task_with_no_pull_request_by_the_horizon_is_unjudged_and_a_label_with_no_date_undated() {
    let setup = Setup::new("unjudged", 3);
    setup.reached(1, 40);
    setup.reached(2, 40);
    setup.reached(3, 40);
    setup.handle(&[opened(2, 2, 9), opened(3, 3, 40)]);
    setup.labeled(3, "closed-unmerged", None);

    let target = setup.target("open-pr");
    assert_eq!(
        target["unjudged"], 2,
        "task 2's pull request opened on day 31"
    );
    assert_eq!(target["undated"], 1);
    assert_eq!(counts(&row(&target, "open-pr", None)), (0, 0, 0));
}

#[test]
fn a_change_requested_before_a_second_irreversible_step_and_after_it_judges_both() {
    let setup = Setup::new("two-targets", 1);
    setup.sql("UPDATE tasks SET grade = 'green' WHERE id = 1", &[]);
    setup.reached(1, 60);
    setup.ended(1, "approve-merge", "merge", 50);
    setup.ended(1, "merge-pr", "merged", 50);
    let review = |days: i64, id: u64| {
        on_pull(
            "github.pull_request_review.submitted",
            1,
            1,
            days,
            json!({ "state": "changes_requested", "github_id": id, "commit": "h1" }),
        )
    };
    setup.handle(&[opened(1, 1, 60), review(55, 501), review(45, 502)]);

    for step in ["open-pr", "merge-pr"] {
        let target = setup.target(step);
        assert_eq!(counts(&row(&target, step, None)), (1, 1, 0), "{step}");
    }
    let grades: Value =
        serde_json::from_slice(&setup.priory(&["grades", "--json"]).stdout).unwrap();
    assert_eq!(
        grades["grades"][2]["labels"]["changes-requested"]["count"],
        1
    );
}

#[test]
fn a_task_that_visited_plan_twice_counts_its_last_outcome_and_a_revived_task_is_one_trial() {
    let setup = Setup::new("visits", 1);
    setup.ended(1, "plan", "red", 45);
    setup.reached(1, 40);
    setup.ended(1, "plan", "red", 35);
    setup.ended(1, "open-pr", "opened", 34);
    setup.handle(&[opened(1, 1, 40)]);

    let target = setup.target("open-pr");
    assert_eq!(counts(&row(&target, "open-pr", None)), (1, 0, 0));
    assert_eq!(counts(&row(&target, "plan", Some("green"))), (1, 0, 0));
    assert_eq!(row(&target, "plan", Some("red")), Value::Null);
}

#[test]
fn each_irreversible_step_has_its_own_horizon() {
    let setup = Setup::new("horizons", 1);
    setup.reached(1, 40);
    setup.ended(1, "approve-merge", "merge", 20);
    setup.ended(1, "merge-pr", "merged", 20);
    setup.handle(&[opened(1, 1, 40)]);

    assert_eq!(setup.target("open-pr")["pending"], 0);
    assert_eq!(counts(&row(&setup.target("open-pr"), "open-pr", None)).0, 1);
    assert_eq!(setup.target("merge-pr")["pending"], 1);
}

#[test]
fn a_stack_with_one_pull_request_reverted_and_one_closed_is_one_landed_wrong_trial() {
    let setup = Setup::new("stack", 1);
    setup.reached(1, 40);
    for number in [1, 2] {
        setup.sql(
            "INSERT INTO pull_requests (task, resource, branch, base, created)
             VALUES (1, 'github:owner/repo#' || ?1, 'stack-' || ?1, 'main', ?2)",
            &[&number, &ago(40)],
        );
    }
    setup.handle(&[
        merged(1, 1, 38),
        on_pull(
            "github.pull_request.closed",
            2,
            1,
            37,
            json!({ "merged": false }),
        ),
        revert(1, 30),
    ]);

    let target = setup.target("open-pr");
    assert_eq!(counts(&row(&target, "open-pr", None)), (1, 1, 1));
    assert_eq!(target["open"], 0);
}

#[test]
fn a_rejection_at_the_gate_is_caught_and_one_after_the_step_judges_it_wrong() {
    let setup = Setup::new("caught", 2);
    setup.ended(1, "plan", "green", 41);
    setup.ended(1, "approve-pr", "rejected", 40);
    setup.labeled(1, "approval-rejected", Some(40));
    setup.reached(2, 40);
    setup.ended(2, "approve-merge", "drop", 35);
    setup.labeled(2, "approval-rejected", Some(35));
    setup.handle(&[opened(2, 2, 40)]);

    let target = setup.target("open-pr");
    assert_eq!(target["caught"], 1);
    assert_eq!(counts(&row(&target, "open-pr", None)), (1, 1, 0));
    assert_eq!(setup.target("merge-pr")["caught"], 1);
}

#[test]
fn a_pull_request_open_on_day_30_is_a_trial_counted_open_and_not_adverse() {
    let setup = Setup::new("open", 1);
    setup.reached(1, 40);
    setup.handle(&[opened(1, 1, 40), merged(1, 1, 5)]);

    let target = setup.target("open-pr");
    assert_eq!(counts(&row(&target, "open-pr", None)), (1, 0, 0));
    assert_eq!(target["open"], 1);
}

#[test]
fn a_row_shows_percentages_from_20_trials_and_since_leaves_out_older_reaches() {
    let setup = Setup::new("floor", 21);
    for task in 1..=19 {
        setup.reached(task, 40);
        setup.handle(&[opened(task, task, 40)]);
    }
    let text = setup.report(&[]);
    let own = text
        .lines()
        .find(|line| line.starts_with("open-pr "))
        .unwrap();
    assert_eq!(
        own.split_whitespace().collect::<Vec<_>>().join(" "),
        "open-pr 0 of 19, needs 20 0 of 19, needs 20"
    );
    let own = row(&setup.target("open-pr"), "open-pr", None);
    assert_eq!(own["shown"], false);
    assert!(own["judged_wrong"]["p95"].as_f64().unwrap() > 0.1);

    setup.reached(20, 100);
    setup.handle(&[opened(20, 20, 100)]);
    let text = setup.report(&[]);
    let text = &text[text.find("implement / open-pr").unwrap()..];
    assert_eq!(
        text.lines().take(5).collect::<Vec<_>>(),
        [
            "implement / open-pr: reached 20, caught 0 at its gate, pending 0, unjudged 0, undated 0, open 20, late 0",
            "horizon 30 days, window 180 days, 1 factory revisions, 0 models",
            "",
            "row                  judged wrong  5%    median  95%    landed wrong  5%    median  95%",
            "open-pr              0 of 20       0.2%  3.2%    13.3%  0 of 20       0.2%  3.2%    13.3%",
        ]
    );
    assert_eq!(
        text.lines().nth(5),
        Some(
            "approve-pr approved  0 of 20       0.2%  3.2%    13.3%  0 of 20       0.2%  3.2%    13.3%"
        )
    );

    let since = setup.report(&["--since", "60 days", "--json"]);
    let since: Value = serde_json::from_str(&since).unwrap();
    assert_eq!(since["targets"][1]["rows"][0]["trials"], 19);
}

#[test]
fn an_irreversible_asks_estimate_is_the_own_row_of_the_step_its_answer_lets_run() {
    let setup = Setup::new("ask-row", 21);
    for task in 1..=20 {
        setup.reached(task, 40);
        setup.handle(&[opened(task, task, 40)]);
    }
    setup.ask(1, 21, "approve-pr");
    setup.ask(2, 21, "approve-merge");
    setup.ask(3, 21, "plan");

    let before_open_pr = setup.listed_ask(1);
    assert_eq!(before_open_pr["risk"], json!(["irreversible"]));
    let estimate = &before_open_pr["risk_estimate"];
    assert_eq!(
        (
            &estimate["workflow"],
            &estimate["step"],
            &estimate["trials"]
        ),
        (&json!("implement"), &json!("open-pr"), &json!(20))
    );
    assert_eq!(
        estimate["line"],
        "History: 0 of 20 tasks through this step were judged wrong within 30 days, 0 after the merge. 95 % posterior upper bounds: 13.3 % and 13.3 %."
    );
    let before_merge = setup.listed_ask(2);
    assert_eq!(before_merge["risk_estimate"]["step"], "merge-pr");
    assert_eq!(before_merge["risk_estimate"]["trials"], 0);
    let plan = setup.listed_ask(3);
    assert_eq!(plan["risk"], json!([]));
    assert!(plan.get("risk_estimate").is_none(), "{plan}");

    let desk = setup.priory(&["desk", "--json"]);
    succeeded(&desk);
    let desk: Value = serde_json::from_slice(&desk.stdout).unwrap();
    let item = desk["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "ask" && item["id"] == 1)
        .unwrap();
    assert_eq!(item["risk_estimate"]["step"], "open-pr");

    let asks = setup.priory(&["asks"]);
    succeeded(&asks);
    assert!(
        String::from_utf8_lossy(&asks.stdout)
            .contains("History: 0 of 20 tasks through this step were judged wrong"),
        "{}",
        String::from_utf8_lossy(&asks.stdout)
    );
}

#[test]
fn an_asks_estimate_under_20_trials_gives_the_counts_and_too_few_to_estimate() {
    let setup = Setup::new("ask-few", 13);
    for task in 1..=12 {
        setup.reached(task, 40);
        setup.handle(&[opened(task, task, 40)]);
    }
    setup.handle(&[
        merged(1, 1, 39),
        revert(1, 30),
        merged(2, 2, 39),
        revert(2, 30),
    ]);
    setup.ask(1, 13, "approve-pr");

    let estimate = &setup.listed_ask(1)["risk_estimate"];
    assert_eq!(estimate["shown"], false);
    assert_eq!(
        estimate["line"],
        "History: 2 of 12 tasks through this step were judged wrong within 30 days, 2 after the merge. Too few to estimate."
    );
}

#[test]
fn check_explain_shows_each_steps_history_when_it_finds_the_state_file_and_dashes_otherwise() {
    let setup = Setup::new("explain", 21);
    for task in 1..=20 {
        setup.reached(task, 40);
        setup.handle(&[opened(task, task, 40)]);
    }
    setup.ended(21, "plan", "green", 41);
    setup.ended(21, "approve-pr", "rejected", 40);

    let explained = setup.priory(&["check", &setup.factory(), "--explain"]);
    succeeded(&explained);
    let text = String::from_utf8(explained.stdout).unwrap();
    assert_eq!(
        text.lines().take(3).collect::<Vec<_>>(),
        [
            "workflow\tstep\tgate\tcaught\tjudged wrong\tlanded wrong",
            "implement\topen-pr\tapprove-pr\t1\t0 of 20, 95% bound 13.3%\t0 of 20, 95% bound 13.3%",
            "implement\tmerge-pr\tapprove-merge\t0\t0 of 0, needs 20\t0 of 0, needs 20",
        ]
    );
    let flows = setup.priory(&["flows", &setup.factory(), "--json"]);
    succeeded(&flows);
    let flows: Value = serde_json::from_slice(&flows.stdout).unwrap();
    let open_pr = &flows["irreversible"][0]["risk_estimate"];
    assert_eq!(
        (&open_pr["caught"], &open_pr["trials"]),
        (&json!(1), &json!(20))
    );

    let missing = setup.dir.join("missing.db");
    let without = Command::new(env!("CARGO_BIN_EXE_priory"))
        .env("PRIORY_DB", &missing)
        .args(["check", &setup.factory(), "--explain"])
        .output()
        .unwrap();
    succeeded(&without);
    assert_eq!(
        String::from_utf8(without.stdout).unwrap().lines().nth(1),
        Some("implement\topen-pr\tapprove-pr\t-\t-\t-")
    );
    let lint = Command::new(env!("CARGO_BIN_EXE_priory"))
        .env("PRIORY_DB", &missing)
        .args(["check", &setup.factory()])
        .output()
        .unwrap();
    succeeded(&lint);
    assert!(!missing.exists());
}

#[test]
fn a_step_record_missing_a_field_a_trial_needs_is_skipped() {
    let setup = Setup::new("partial", 1);
    setup.reached(1, 40);
    setup.handle(&[opened(1, 1, 40)]);
    let event = json!({ "id": "partial", "type": "factory.step.ended", "time": ago(5),
                        "data": { "step": "implement", "model": "m" } });
    setup.sql(
        "INSERT INTO trace (id, type, time, event) VALUES ('partial', 'factory.step.ended', ?1, ?2)",
        &[&ago(5), &event.to_string()],
    );

    assert_eq!(
        counts(&row(&setup.target("open-pr"), "open-pr", None)),
        (1, 0, 0)
    );
}
