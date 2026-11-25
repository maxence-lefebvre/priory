//! Urgent wakes: a `blocker` head stops a `later` run at its turn end (docs/daemon/room.md).

use super::*;

/// A factory whose `blocker` band may stop runs of lower bands.
const URGENT: &str = "name = \"demo\"\n\n[queue]\npreempt = [\"blocker\"]\n";

/// `URGENT` with a `[serve]` that lets a person hold a run's keyboard.
pub(super) const URGENT_TYPE: &str = "name = \"demo\"\n\n[queue]\npreempt = [\"blocker\"]\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\nterminal = \"type\"\n";

/// Task 1's reason while its run waits for its turn end to stop for task 2.
const MARKED: &str = "stops at its next turn end for task 2 (blocker)";

/// The prompt a run stopped for task 2 resumes with.
const STOPPED: &str = "Priory stopped your last run at the end of your turn to make room for task 2, in band blocker.\nYour session and worktree are as you left them. Continue.\n";

/// Task 2's reason while it waits for task 1's 4 GB.
const WAITS: &str = "waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";

/// Two model steps of the implementer, so a result carries its process into the next one.
pub(super) const SAME_AGENT: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "check", failed = "stop" }

[[steps]]
id = "check"
decides = "model"
agent = "implementer"
prompt = "Check the change."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

/// One model step that pushes, which the factory lets run without a person.
pub(super) const SHIP: &str = r#"
start = "ship"

[[steps]]
id = "ship"
decides = "model"
agent = "implementer"
prompt = "Ship the change."
effect = "irreversible"
autonomous = "a test of a step that pushes"
next = { passed = "done", failed = "stop" }
"#;

impl Setup {
    /// A factory whose `blocker` band preempts, with room for one 4 GB run.
    fn urgent(name: &str) -> Setup {
        let setup = Setup::new(name);
        fs::write(setup.dir.join("factory/factory.toml"), URGENT).unwrap();
        setup.memory(12);
        setup
    }

    /// Starts the daemon with the token a `[serve]` table names.
    pub(super) fn go(&mut self) {
        self.start_with(
            Path::new(env!("CARGO_BIN_EXE_priory")),
            &[("TEST_TOKEN", "token")],
        );
    }

    /// Writes `workflows/<name>.toml`.
    pub(super) fn workflow(&self, name: &str, workflow: &str) {
        fs::write(
            self.dir.join(format!("factory/workflows/{name}.toml")),
            workflow,
        )
        .unwrap();
    }

    /// Runs `sql` on the state file and returns the first column of its first row as text.
    pub(super) fn sql(&self, sql: &str) -> Option<String> {
        use rusqlite::types::Value;
        let value = rusqlite::Connection::open(self.dir.join("priory.db"))
            .unwrap()
            .query_row(sql, [], |row| row.get::<_, Value>(0))
            .unwrap();
        match value {
            Value::Null => None,
            Value::Integer(number) => Some(number.to_string()),
            Value::Text(text) => Some(text),
            other => panic!("{other:?}"),
        }
    }

    /// Holds task `task`'s keyboard, as `POST /api/runs/<run>/keyboard` does, until the year 9999.
    pub(super) fn hold_keyboard(&self, task: &str) {
        rusqlite::Connection::open(self.dir.join("priory.db"))
            .unwrap()
            .execute(
                "INSERT INTO keyboard_grants (run, hash, label, started, ends)
                 SELECT id, hex(randomblob(16)), 'max', 'now', '9999-12-31T00:00:00Z' FROM runs
                 WHERE task = ?1 AND ended IS NULL",
                [task],
            )
            .unwrap();
    }

    /// Attaches a tmux client in control mode to task `task`'s session, as a person watching.
    fn watch(&self, task: &str) -> Child {
        self.attach_client(task, &[])
    }

    fn attach_client(&self, task: &str, flags: &[&str]) -> Child {
        let client = self
            .command("tmux")
            .args(["-C", "attach-session"])
            .args(flags)
            .args(["-t", &format!("=priory-{task}")])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while self
            .tmux(&["list-clients", "-t", &format!("=priory-{task}")])
            .stdout
            .is_empty()
        {
            assert!(Instant::now() < deadline, "the client never attached");
            std::thread::sleep(Duration::from_millis(50));
        }
        client
    }

    /// Waits until the reason of task `id` satisfies `done`, and returns it.
    fn until_reason(&self, id: &str, done: impl Fn(&str) -> bool) -> String {
        let rows = self.until("tasks", |rows| {
            task(rows, id).is_some_and(|row| done(&row[7]))
        });
        task(&rows, id).unwrap()[7].clone()
    }

    /// The prompts queued or typed for task `id`, oldest first.
    pub(super) fn prompts(&self, id: &str) -> Vec<String> {
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        let mut statement = db
            .prepare("SELECT text FROM outbox WHERE task = ?1 ORDER BY id")
            .unwrap();
        statement
            .query_map([id], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

fn later(issue: u64) -> Message {
    requested_in(issue, Some("later"))
}

fn blocker(issue: u64) -> Message {
    requested_in(issue, Some("blocker"))
}

/// `message`, routed to workflow `workflow`.
pub(super) fn on(workflow: &str, mut message: Message) -> Message {
    message.data["workflow"] = json!(workflow);
    message
}

/// The row of task `id`.
fn task<'a>(rows: &'a [Vec<String>], id: &str) -> Option<&'a Vec<String>> {
    rows.iter().find(|row| row[0] == id)
}

/// The row of run `id`.
fn run<'a>(rows: &'a [Vec<String>], id: &str) -> Option<&'a Vec<String>> {
    task(rows, id)
}

/// Starts `first` in band later, held at the end of its first turn, then queues task 2 in band
/// blocker, and returns once task 2 marked task 1's run.
fn marked_with(setup: &mut Setup, first: Message) {
    setup.hold("hold-stop");
    setup.publish(&first);
    setup.go();
    // Its turn started, so the run is not idle between turns.
    setup.written("fake/submitted");
    setup.publish(&blocker(43));
    setup.until_reason("1", |reason| reason == MARKED);
}

fn marked(setup: &mut Setup) {
    marked_with(setup, later(42));
}

/// Whether a nudge for task `id` was queued or typed.
fn nudged(setup: &Setup, id: &str) -> bool {
    setup
        .prompts(id)
        .iter()
        .any(|prompt| prompt.starts_with(NUDGE.lines().next().unwrap()))
}

#[test]
fn a_blocker_task_stops_a_later_run_at_its_turn_end_and_starts_in_its_room() {
    let mut setup = Setup::urgent("preempt");
    marked(&mut setup);

    assert_eq!(
        setup.until_reason("2", |reason| reason.starts_with(WAITS)),
        format!("{WAITS}; task 1 (later) stops at its next turn end")
    );
    let queued = setup.priory(&["queue", "--json"]);
    let queued: serde_json::Value = serde_json::from_slice(&queued.stdout).unwrap();
    let head = &queued["waiting"][0];
    assert_eq!((&head["id"], &head["stops"]), (&json!(2), &json!([1])));
    assert_eq!(head["protected"], json!([]));
    setup.release("hold-stop");

    let runs = setup.until("runs", |rows| {
        run(rows, "1").is_some_and(|row| row[7] == "preempted")
            && run(rows, "2").is_some_and(|row| row[1] == "2")
    });
    assert_eq!(runs.len(), 2, "{runs:?}");
    let tasks = setup.until("tasks", |rows| {
        task(rows, "1").is_some_and(|row| row[4] == "ready" && row[7].starts_with("stopped"))
    });
    assert_eq!(
        task(&tasks, "1").unwrap()[7],
        "stopped for task 2 (blocker); resumes when there is room"
    );
    assert_eq!(task(&tasks, "2").unwrap()[4], "running");
    let prompts = setup.prompts("1");
    assert!(prompts.last().unwrap().starts_with(STOPPED), "{prompts:?}");
    assert!(!nudged(&setup, "1"), "{prompts:?}");
    let record = setup
        .sql("SELECT event FROM trace WHERE event LIKE '%factory.run.preempted%'")
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&record).unwrap();
    let data = &record["data"];
    assert_eq!(
        [
            &data["run"],
            &data["task"],
            &data["band"],
            &data["head_task"],
            &data["head_band"]
        ],
        [
            &json!("1"),
            &json!("1"),
            &json!("later"),
            &json!("2"),
            &json!("blocker")
        ]
    );
    assert!(data["waited_ms"].as_i64().unwrap() >= 0, "{data}");
}

#[test]
fn a_stopped_task_resumes_its_session_in_its_worktree_with_the_stop_prompt_first() {
    let mut setup = Setup::urgent("preempt-resume");
    setup.calls(&[serde_json::Value::Null, result("passed")]);
    marked(&mut setup);
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::write(worktree.join("draft.txt"), "uncommitted\n").unwrap();

    setup.release("hold-stop");

    let runs = setup.until("runs", |rows| rows.len() == 3);
    assert_eq!([&runs[2][1], &runs[2][4]], ["1", &runs[0][4]], "{runs:?}");
    // The run is stored before its harness starts and writes its arguments.
    setup.until("runs", |_| {
        has(&launched(&setup), ["--resume", &runs[0][4]])
    });
    assert_eq!(
        fs::read_to_string(worktree.join("draft.txt")).unwrap(),
        "uncommitted\n"
    );
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(typed[2].starts_with(STOPPED), "{typed:?}");
}

#[test]
fn a_marked_result_into_a_step_of_the_same_agent_ends_the_run_where_it_would_carry_on() {
    let mut setup = Setup::urgent("preempt-carry");
    setup.workflow("two", SAME_AGENT);
    setup.calls(&[result("passed")]);
    marked_with(&mut setup, on("two", later(42)));

    setup.release("hold-stop");

    setup.until("tasks", |rows| {
        task(rows, "1").is_some_and(|row| row[4] == "ready" && row[3] == "check")
            && task(rows, "2").is_some_and(|row| row[4] == "running")
    });
    let runs = setup.rows("runs");
    let own: Vec<_> = runs.iter().filter(|row| row[1] == "1").collect();
    assert_eq!(
        own.len(),
        1,
        "no run of task 1 is live after its stop: {runs:?}"
    );
    assert_eq!(own[0][7], "preempted");
    let prompts = setup.prompts("1");
    let last = prompts.last().unwrap();
    assert!(last.starts_with(STOPPED), "{prompts:?}");
    assert!(last.contains("Check the change."), "{prompts:?}");

    // Without a mark, the same result carries the process on.
    let mut unmarked = Setup::urgent("preempt-carry-unmarked");
    unmarked.workflow("two", SAME_AGENT);
    unmarked.calls(&[result("passed")]);
    unmarked.publish(&on("two", later(42)));
    unmarked.go();
    let runs = unmarked.until("runs", |rows| rows.len() == 2);
    assert_eq!([&runs[0][7], &runs[1][1]], ["moved", "1"], "{runs:?}");
    assert_eq!(runs[0][3], runs[1][3], "one tmux session carries on");
}

#[test]
fn a_marked_run_that_waits_sleeps_as_usual() {
    let mut setup = Setup::urgent("preempt-wait");
    setup.calls(&[wait(json!({ "type": "time", "until": "1 hour" }))]);
    marked(&mut setup);

    setup.release("hold-stop");

    let tasks = setup.until("tasks", |rows| {
        task(rows, "1").is_some_and(|row| row[4] == "asleep")
            && task(rows, "2").is_some_and(|row| row[4] == "running")
    });
    assert_eq!(task(&tasks, "1").unwrap()[7], "-");
    assert_eq!(run(&setup.rows("runs"), "1").unwrap()[7], "asleep");
    assert!(
        setup
            .prompts("1")
            .iter()
            .all(|prompt| !prompt.starts_with("Priory stopped"))
    );
}

#[test]
fn a_marked_run_whose_ask_was_answered_before_its_stop_ends_and_resumes_with_the_stop_then_the_answer()
 {
    let mut setup = Setup::urgent("preempt-answer");
    setup.workflow("steps", STEPS);
    setup.calls(&[ask(), result("passed")]);
    marked_with(&mut setup, on("steps", later(42)));
    setup.until("asks", |rows| rows.len() == 1);
    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");

    setup.release("hold-stop");

    let runs = setup.until("runs", |rows| rows.len() == 3);
    assert_eq!([&runs[0][7], &runs[2][1]], ["preempted", "1"], "{runs:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(typed[2].starts_with(STOPPED), "{typed:?}");
    assert!(typed[2][STOPPED.len()..].contains("sqlite"), "{typed:?}");
}

#[test]
fn a_marked_run_with_an_unconfirmed_prompt_goes_on_and_stops_at_its_next_turn_end() {
    let mut setup = Setup::urgent("preempt-unconfirmed");
    fs::write(setup.dir.join("fake/trailer"), " Wait for approval.").unwrap();
    marked(&mut setup);
    assert_eq!(setup.rows("outbox")[0][3], "attempted");

    setup.release("hold-stop");
    // The Stop leaves the attempted prompt's run live, with its mark.
    setup.until("runs", |_| {
        setup.sql("SELECT idle FROM runs WHERE id = 1") == Some("1".to_owned())
    });
    assert_eq!(
        setup.sql("SELECT preempted_for FROM runs WHERE id = 1"),
        Some("2".to_owned())
    );
    assert_eq!(task(&setup.rows("tasks"), "1").unwrap()[7], MARKED);
    let uncertain_before = setup
        .sql("SELECT group_concat(id) FROM outbox WHERE state IN ('attempted', 'uncertain')")
        .unwrap_or_default();

    setup.crash();
    setup.go();

    setup.until("outbox", |rows| rows[0][3] == "uncertain");
    assert_eq!(
        setup.sql("SELECT group_concat(id) FROM outbox WHERE state = 'uncertain'"),
        Some(uncertain_before)
    );
    assert_eq!(run(&setup.rows("runs"), "1").unwrap()[6], "-");
    // The person submits the same words again: the prompt is confirmed and its turn ends.
    fs::remove_file(setup.dir.join("fake/trailer")).unwrap();
    let text = setup.sql("SELECT text FROM outbox WHERE id = 1").unwrap();
    let mut load = setup
        .command("tmux")
        .args(["load-buffer", "-"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut load.stdin.take().unwrap(), text.as_bytes()).unwrap();
    assert!(load.wait().unwrap().success());
    assert!(
        setup
            .tmux(&["paste-buffer", "-p", "-t", "=priory-1:"])
            .status
            .success()
    );
    assert!(
        setup
            .tmux(&["send-keys", "-t", "=priory-1:", "Enter"])
            .status
            .success()
    );

    setup.until("runs", |rows| {
        run(rows, "1").is_some_and(|row| row[7] == "preempted")
    });
    setup.until("tasks", |rows| {
        task(rows, "2").is_some_and(|row| row[4] == "running")
    });
}

#[test]
fn a_marked_run_past_compact_at_ends_without_a_compaction_and_its_next_run_compacts() {
    let mut setup = Setup::urgent("preempt-compact");
    setup.compact_at(5000, &[6000, 6000, 6000, 6000, 6000]);
    // The blocker's agent sets no `compactAt`, so it ends in one result.
    setup.workflow(
        "review",
        &WORKFLOW.replace("\"implementer\"", "\"reviewer\""),
    );
    setup.calls(&[serde_json::Value::Null, result("passed")]);
    setup.hold("hold-stop");
    setup.publish(&later(42));
    setup.go();
    setup.written("fake/submitted");
    setup.publish(&on("review", blocker(43)));
    setup.until_reason("1", |reason| reason == MARKED);

    setup.release("hold-stop");

    setup.until("runs", |rows| {
        run(rows, "1").is_some_and(|row| row[7] == "preempted")
    });
    assert!(
        setup
            .prompts("1")
            .iter()
            .all(|prompt| !prompt.starts_with("/compact")),
        "{:?}",
        setup.prompts("1")
    );
    let runs = setup.until("runs", |rows| rows.len() == 3);
    assert_eq!(runs[2][1], "1");
    setup.until("outbox", |_| setup.compactions() == 1);
    let typed = setup.typed();
    let stop = typed.iter().position(|p| p.starts_with(STOPPED)).unwrap();
    let compact = typed.iter().position(|p| p == "/compact\n").unwrap();
    assert!(stop < compact, "{typed:?}");
    assert_eq!(
        setup.sql("SELECT run FROM outbox WHERE compact = 1"),
        Some("3".to_owned())
    );
}

#[test]
fn a_client_attached_after_the_mark_withdraws_it_and_the_run_gets_its_nudge() {
    let mut setup = Setup::urgent("preempt-watched");
    marked(&mut setup);

    let mut client = setup.watch("1");

    assert_eq!(
        setup.until_reason("2", |reason| reason.contains("stops no run")),
        format!("{WAITS}; stops no run: task 1 is watched in tmux")
    );
    assert_eq!(task(&setup.rows("tasks"), "1").unwrap()[7], "-");
    setup.release("hold-stop");
    setup.until("outbox", |_| nudged(&setup, "1"));
    assert_eq!(setup.rows("runs").len(), 1);
    assert_eq!(setup.rows("runs")[0][7], "-");
    client.kill().unwrap();
    client.wait().unwrap();
}

#[test]
fn a_tmux_that_cannot_list_clients_protects_the_run() {
    let mut setup = Setup::urgent("preempt-clients-fail");
    fs::write(setup.dir.join("fake/tmux-fail-clients"), "").unwrap();
    setup.hold("hold-stop");
    setup.publish(&later(42));
    setup.go();
    setup.written("fake/submitted");

    setup.publish(&blocker(43));

    assert_eq!(
        setup.until_reason("2", |reason| reason.starts_with(WAITS)),
        format!("{WAITS}; stops no run: task 1 is watched in tmux")
    );
    setup.release("hold-stop");
    setup.until("outbox", |_| nudged(&setup, "1"));
    assert_eq!(setup.rows("runs").len(), 1);
}

#[test]
fn without_preempt_or_with_the_head_outside_it_no_run_is_marked() {
    for (name, factory, band) in [
        ("preempt-off", "name = \"demo\"\n", "blocker"),
        ("preempt-other-band", URGENT, "now"),
    ] {
        let mut setup = Setup::new(name);
        fs::write(setup.dir.join("factory/factory.toml"), factory).unwrap();
        setup.memory(12);
        setup.hold("hold-stop");
        setup.publish(&later(42));
        setup.go();
        setup.written("fake/submitted");

        setup.publish(&requested_in(43, Some(band)));

        assert_eq!(
            setup.until_reason("2", |reason| reason.starts_with(WAITS)),
            WAITS
        );
        assert_eq!(task(&setup.rows("tasks"), "1").unwrap()[7], "-");
        setup.release("hold-stop");
        setup.until("outbox", |_| nudged(&setup, "1"));
        assert_eq!(setup.rows("runs").len(), 1);
        assert_eq!(
            setup.sql("SELECT count(*) FROM runs WHERE preempted_for IS NOT NULL"),
            Some("0".to_owned())
        );
    }
}

#[test]
fn protected_runs_are_never_marked_and_the_heads_reason_names_each_one() {
    let mut setup = Setup::new("preempt-protected");
    fs::write(setup.dir.join("factory/factory.toml"), URGENT_TYPE).unwrap();
    setup.workflow("ship", SHIP);
    let lead = LEAD.replace("You implement", "You lead");
    fs::create_dir_all(setup.dir.join("factory/agents/lead")).unwrap();
    fs::write(setup.dir.join("factory/agents/lead/agent.md"), lead).unwrap();
    setup.workflow("lead", &WORKFLOW.replace("\"implementer\"", "\"lead\""));
    // Room for five 4 GB runs, and not a sixth.
    setup.memory(28);
    setup.hold("hold-stop");
    setup.publish(&on("ship", later(42)));
    setup.publish(&later(43));
    setup.publish(&later(44));
    setup.publish(&on("lead", later(45)));
    setup.publish(&blocker(46));
    setup.go();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 5);
    setup.hold_keyboard("2");
    let mut client = setup.watch("3");

    setup.publish(&blocker(47));

    let waits = "waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";
    assert_eq!(
        setup.until_reason("6", |reason| reason.contains("stops no run")),
        format!(
            "{waits}; stops no run: task 4 runs in a shared session, task 3 is watched in tmux, \
             task 2's keyboard is held, task 1 is at irreversible step ship, task 5 is in band blocker"
        )
    );
    let queued = setup.priory(&["queue", "--json"]);
    let queued: serde_json::Value = serde_json::from_slice(&queued.stdout).unwrap();
    let head = &queued["waiting"][0];
    assert_eq!((&head["id"], &head["stops"]), (&json!(6), &json!([])));
    assert_eq!(
        head["protected"][3],
        json!({ "task": 1, "reason": "task 1 is at irreversible step ship" })
    );
    assert_eq!(
        setup.sql("SELECT count(*) FROM runs WHERE preempted_for IS NOT NULL"),
        Some("0".to_owned())
    );
    client.kill().unwrap();
    client.wait().unwrap();
}

#[test]
fn a_keyboard_taken_after_the_mark_withdraws_it_and_the_run_gets_its_nudge() {
    let mut setup = Setup::urgent("preempt-keyboard");
    fs::write(setup.dir.join("factory/factory.toml"), URGENT_TYPE).unwrap();
    marked(&mut setup);

    setup.hold_keyboard("1");

    assert_eq!(
        setup.until_reason("2", |reason| reason.contains("stops no run")),
        format!("{WAITS}; stops no run: task 1's keyboard is held")
    );
    setup.release("hold-stop");
    setup.until("outbox", |_| nudged(&setup, "1"));
    assert_eq!(setup.rows("runs").len(), 1);
    assert_eq!(setup.rows("runs")[0][7], "-");
}

#[test]
fn the_head_marks_the_last_runs_in_start_order_until_it_fits_and_none_when_all_would_not() {
    let mut setup = Setup::urgent("preempt-pick");
    let review = WORKFLOW.replace("\"implementer\"", "\"reviewer\"");
    setup.workflow("review", &review);
    // 15 GB free keeps 6.4: room for two 4 GB runs.
    setup.memory(15);
    setup.hold("hold-stop");
    setup.publish(&later(42));
    setup.publish(&later(43));
    setup.go();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);

    setup.publish(&on("review", blocker(44)));

    let marked = "stops at its next turn end for task 3 (blocker)";
    let reason = setup.until_reason("2", |reason| reason == marked);
    assert_eq!(reason, marked);
    assert_eq!(task(&setup.rows("tasks"), "1").unwrap()[7], "-");
    assert!(
        setup
            .until_reason("3", |reason| reason
                .ends_with("task 2 (later) stops at its next turn end"))
            .starts_with("waits for room: 7.0 GB")
    );

    for (memory, both) in [("8GB", true), ("10GB", false)] {
        setup.crash();
        let reviewer = REVIEWER.replacen("+++\n", &format!("+++\nmemory = \"{memory}\"\n"), 1);
        fs::write(setup.dir.join("factory/agents/reviewer/agent.md"), reviewer).unwrap();
        setup.go();
        if both {
            setup.until_reason("1", |reason| reason == marked);
            assert_eq!(task(&setup.rows("tasks"), "2").unwrap()[7], marked);
            assert!(
                setup
                    .until_reason("3", |reason| reason.contains("stop at"))
                    .ends_with("task 2 (later), task 1 (later) stop at their next turn end")
            );
        } else {
            assert!(
                setup
                    .until_reason("3", |reason| reason.contains("stops no run"))
                    .ends_with("; stops no run: the runs it may stop free too little")
            );
            let tasks = setup.rows("tasks");
            assert_eq!(
                [
                    &task(&tasks, "1").unwrap()[7],
                    &task(&tasks, "2").unwrap()[7]
                ],
                ["-", "-"]
            );
        }
    }
}

#[test]
fn a_task_stopped_once_is_not_marked_again_until_its_next_result() {
    let mut setup = Setup::urgent("preempt-once");
    setup.workflow("two", SAME_AGENT);
    marked_with(&mut setup, on("two", later(42)));
    setup.release("hold-stop");
    setup.until("runs", |rows| {
        run(rows, "1").is_some_and(|row| row[7] == "preempted")
    });
    // Every later turn waits before its tool call until the test lets it go on.
    setup.hold("hold-call");
    setup.until("tasks", |rows| {
        task(rows, "2").is_some_and(|row| row[4] == "running")
    });
    assert!(setup.priory(&["cancel", "2"]).status.success());
    setup.until("runs", |rows| rows.len() == 3);

    setup.publish(&blocker(44));

    assert_eq!(
        setup.until_reason("3", |reason| reason.contains("stops no run")),
        format!("{WAITS}; stops no run: task 1 was already stopped once")
    );
    // The resumed turn ends with no tool, the nudge's turn with a result into the next step.
    setup.calls(&[serde_json::Value::Null, result("passed")]);
    setup.hold("hold-stop");
    setup.release("hold-call");
    setup.calls_left(1);
    setup.hold("hold-call");
    setup.release("hold-stop");
    setup.until("outbox", |rows| {
        rows.iter().any(|row| {
            row[1] == "1" && row[3] == "confirmed" && row[4].starts_with("Your last turn")
        })
    });
    setup.hold("hold-stop");
    setup.release("hold-call");
    setup.calls_left(0);
    setup.until_reason("1", |reason| {
        reason == "stops at its next turn end for task 3 (blocker)"
    });
    setup.release("hold-stop");
    let runs = setup.until("runs", |rows| {
        run(rows, "3").is_some_and(|row| row[7] == "preempted")
    });
    assert_eq!(
        runs.iter().filter(|row| row[1] == "1").count(),
        2,
        "{runs:?}"
    );
    setup.until("tasks", |rows| {
        task(rows, "1").is_some_and(|row| row[3] == "check")
    });
}

#[test]
fn a_head_that_waits_for_load_or_for_a_shared_agent_marks_nothing() {
    let mut setup = Setup::urgent("preempt-load");
    setup.memory(20);
    setup.hold("hold-stop");
    setup.publish(&later(42));
    setup.go();
    setup.written("fake/submitted");
    setup.load(100_000.0);

    setup.publish(&blocker(43));

    let reason = setup.until_reason("2", |reason| reason.starts_with("waits for room: load"));
    assert!(!reason.contains(';'), "{reason}");
    assert_eq!(task(&setup.rows("tasks"), "1").unwrap()[7], "-");

    let mut shared = Setup::urgent("preempt-shared-agent");
    fs::write(shared.dir.join("factory/agents/implementer/agent.md"), LEAD).unwrap();
    shared.hold("hold-stop");
    shared.publish(&later(42));
    shared.go();
    shared.written("fake/submitted");

    shared.publish(&blocker(43));

    assert_eq!(
        shared.until_reason("2", |reason| reason.starts_with("waits for agent")),
        "waits for agent implementer: its shared session serves task 1"
    );
    assert_eq!(task(&shared.rows("tasks"), "1").unwrap()[7], "-");
}

#[test]
fn a_head_cancelled_before_the_turn_end_withdraws_the_mark_and_the_run_gets_its_nudge() {
    let mut setup = Setup::urgent("preempt-cancel");
    marked(&mut setup);

    assert!(setup.priory(&["cancel", "2"]).status.success());

    setup.until_reason("1", |reason| reason == "-");
    setup.release("hold-stop");
    setup.until("outbox", |_| nudged(&setup, "1"));
    assert_eq!(setup.rows("runs").len(), 1);
    assert_eq!(setup.rows("runs")[0][7], "-");
}

#[test]
fn a_daemon_restart_between_the_mark_and_the_turn_end_keeps_the_mark() {
    let mut setup = Setup::urgent("preempt-restart");
    marked(&mut setup);

    setup.crash();
    assert_eq!(
        setup.sql("SELECT preempted_for FROM runs WHERE id = 1"),
        Some("2".to_owned())
    );
    setup.go();
    setup.release("hold-stop");

    setup.until("runs", |rows| {
        run(rows, "1").is_some_and(|row| row[7] == "preempted")
    });
}

#[test]
fn a_marked_run_over_its_steps_budget_fails_rather_than_stopping() {
    let mut setup = Setup::urgent("preempt-budget");
    setup.workflow("budgeted", &budgeted("0.01"));
    fs::write(setup.dir.join("factory/prices.toml"), DEAR).unwrap();
    // The daemon reads the turn's tokens before its end, so the budget may end the run before
    // the urgent wake marks it: either way it fails.
    setup.hold("hold-stop");
    setup.publish(&on("budgeted", later(42)));
    setup.go();
    setup.written("fake/submitted");
    setup.publish(&blocker(43));
    setup.release("hold-stop");

    setup.until("tasks", |rows| {
        task(rows, "1").is_some_and(|row| row[4] == "failed")
    });
    assert_eq!(run(&setup.rows("runs"), "1").unwrap()[7], "budget");
    assert!(
        setup
            .prompts("1")
            .iter()
            .all(|prompt| !prompt.starts_with("Priory stopped"))
    );
}

#[test]
fn a_spend_cap_ends_no_run_a_person_watches_in_tmux() {
    let mut setup = Setup::new("cap-watched");
    setup.caps("[caps]\ntask = 50.0\n");
    setup.calls(&[serde_json::Value::Null, result("passed")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
    let mut client = setup.watch("1");

    setup.release("hold-stop");

    let tasks = setup.until("tasks", |rows| {
        rows.first()
            .is_some_and(|row| row[4] == "done" || row[4] == "held")
    });
    client.kill().unwrap();
    client.wait().unwrap();
    assert_eq!(tasks[0][4], "done", "{tasks:?}");
    let runs = setup.rows("runs");
    assert!(runs.iter().all(|run| run[7] != "capped"), "{runs:?}");
}

/// One budgeted model step of the reviewer, whom no guard covers.
fn probe() -> String {
    budgeted("0.01").replace("agent = \"implementer\"", "agent = \"reviewer\"")
}

impl Setup {
    /// Task 1, on `implement` with a budget of $0.01 in a factory `before` changed, mid-turn past
    /// the budget under `guard`, and task 2, an unguarded probe whose run the check inside a turn
    /// ends: the pass that ended it read task 1's run too. Task 1's turn then ends with its result.
    fn guarded_over_budget(
        name: &str,
        implement: &str,
        before: impl FnOnce(&Setup),
        guard: impl FnOnce(&Setup),
    ) -> Setup {
        let mut setup = Setup::with_workflow(name, implement);
        before(&setup);
        setup.workflow("probe", &probe());
        fs::write(setup.dir.join("factory/prices.toml"), DEAR).unwrap();
        setup.calls(&[result("passed")]);
        setup.hold("hold-submit");
        setup.hold("hold-call");
        setup.publish(&requested(42));
        setup.go();
        setup.written("fake/pending");
        guard(&setup);
        setup.release("hold-submit");
        setup.until("runs", |_| {
            setup.sql(
                "SELECT count(*) FROM runs WHERE task = 1 AND ready AND idle = 0
                 AND transcript IS NOT NULL",
            ) == Some("1".to_owned())
        });
        setup.publish(&on("probe", requested(43)));
        setup.until("tasks", |rows| {
            task(rows, "2").is_some_and(|row| row[4] == "failed")
        });
        assert_eq!(task(&setup.rows("tasks"), "1").unwrap()[4], "running");
        setup.release("hold-call");
        setup.until("tasks", |rows| {
            task(rows, "1").is_some_and(|row| row[4] == "done" || row[4] == "failed")
        });
        setup
    }

    /// Asserts task 1 ended `done` at its turn end, its overrun on its record.
    fn overran(&self) {
        assert_eq!(task(&self.rows("tasks"), "1").unwrap()[4], "done");
        assert_ne!(run(&self.rows("runs"), "1").unwrap()[7], "budget");
        let overrun = self
            .trace()
            .into_iter()
            .find(|record| {
                record["type"] == "factory.step.ended" && record["data"]["outcome"] == "passed"
            })
            .map(|record| record["data"]["overrun"]["budget_usd"].clone());
        assert_eq!(overrun, Some(json!(0.01)));
    }
}

#[test]
fn the_budget_inside_a_turn_ends_no_run_at_an_irreversible_step() {
    let ship = SHIP.replace(
        "effect = \"irreversible\"",
        "effect = \"irreversible\"\nbudget = \"0.01\"",
    );
    let setup = Setup::guarded_over_budget("budget-irreversible", &ship, |_| {}, |_| {});
    setup.overran();
}

#[test]
fn the_budget_inside_a_turn_ends_no_run_whose_keyboard_a_person_holds() {
    let typed = |setup: &Setup| {
        fs::write(setup.dir.join("factory/factory.toml"), URGENT_TYPE).unwrap();
    };
    let setup = Setup::guarded_over_budget("budget-keyboard", &budgeted("0.01"), typed, |setup| {
        setup.hold_keyboard("1");
    });
    setup.overran();
}

#[test]
fn the_budget_inside_a_turn_ends_no_run_in_a_shared_session() {
    let lead = |setup: &Setup| {
        fs::write(setup.dir.join("factory/agents/implementer/agent.md"), LEAD).unwrap();
    };
    let setup = Setup::guarded_over_budget("budget-shared", &budgeted("0.01"), lead, |_| {});
    setup.overran();
}

#[test]
fn the_budget_inside_a_turn_ends_no_run_a_person_watches_in_tmux() {
    let mut client = None;
    let setup = Setup::guarded_over_budget(
        "budget-watched",
        &budgeted("0.01"),
        |_| {},
        |setup| {
            client = Some(setup.watch("1"));
        },
    );
    let mut client = client.unwrap();
    client.kill().unwrap();
    client.wait().unwrap();
    setup.overran();
}

#[test]
fn the_budget_inside_a_turn_ends_a_run_with_only_a_read_only_client() {
    let mut setup = Setup::with_workflow("budget-read-only-client", &budgeted("0.01"));
    fs::write(setup.dir.join("factory/prices.toml"), DEAR).unwrap();
    setup.calls(&[result("passed")]);
    // The client attaches before the turn writes the tokens that pass the budget.
    setup.hold("hold-submit");
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/pending");
    let mut client = setup.attach_client("1", &["-f", "read-only,ignore-size"]);

    setup.release("hold-submit");

    assert_eq!(
        setup.until_task("failed"),
        "step implement spent $0.011200, over its budget of $0.01"
    );
    assert_eq!(setup.rows("runs")[0][7], "budget");
    client.wait().unwrap();
}
