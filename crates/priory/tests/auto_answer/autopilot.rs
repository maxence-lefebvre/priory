//! Autopilot: while a person has it on, the daemon answers the eligible asks, leaves the others and
//! reports both (docs/daemon/waits.md).

use super::*;

impl Setup {
    /// `priory autopilot <args>`, with the factory's folder, as `on` takes it.
    fn autopilot(&self, args: &[&str]) -> Exit {
        let factory = self.dir.join("factory");
        let mut all = vec!["autopilot"];
        all.extend(args);
        if args[0] == "on" {
            all.insert(2, factory.to_str().unwrap());
        }
        self.priory(&all)
    }

    fn autopilot_on(&self, args: &[&str]) {
        let mut all = vec!["on"];
        all.extend(args);
        let output = self.autopilot(&all);
        assert!(output.status.success(), "{output:?}");
    }

    fn autopilot_report(&self) -> serde_json::Value {
        let output = self
            .command(env!("CARGO_BIN_EXE_priory"))
            .args(["autopilot", "report", "--json"])
            .env("PRIORY_FACTORY", self.dir.join("factory"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn desk_json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.priory(&["desk", "--json"]).stdout).unwrap()
    }
}

/// A setup whose task sleeps on an ask stated at `confidence`, and passes once it is answered.
fn asleep(name: &str, workflow: &str, confidence: &str) -> Setup {
    let mut setup = Setup::with_workflow(name, workflow);
    setup.calls(&[confident_ask(confidence), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    waits_for_a_person(&setup);
    setup
}

/// Waits until autopilot says why it leaves ask `ask`, and returns the reason.
fn left(setup: &Setup, ask: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let asks = asks_json(setup);
        let row = asks["asks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"].to_string().trim_matches('"') == ask);
        if let Some(why) = row.and_then(|row| row["autopilot_left"].as_str()) {
            assert_eq!(row.unwrap()["answered_by"], json!(null), "{asks}");
            return why.to_owned();
        }
        assert!(Instant::now() < deadline, "autopilot left no ask: {asks}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn factory(setup: &Setup) -> String {
    setup.dir.join("factory").to_str().unwrap().to_owned()
}

/// Runs the command autopilot's report prints to reverse an answer, with `text` as its placeholder.
fn reverses(setup: &Setup, command: &str, text: &str) {
    for part in command.split(" && ") {
        let words: Vec<&str> = part
            .split(' ')
            .map(|word| match word {
                "TEXT" | "OPTION" => text,
                word => word,
            })
            .collect();
        assert_eq!(words[0], "priory", "{command}");
        let ran = setup.priory(&words[1..]);
        assert!(ran.status.success(), "{command}: {ran:?}");
    }
}

fn reverse(setup: &Setup) -> String {
    let report = setup.autopilot_report();
    report["answered"][0]["reverse"]
        .as_str()
        .unwrap_or_else(|| panic!("{report}"))
        .to_owned()
}

fn done(setup: &Setup) {
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
}

#[test]
fn on_answers_an_ask_already_open_and_records_it_as_autopilots() {
    let setup = asleep("pilot-open", WORKFLOW, "high");

    setup.autopilot_on(&["--for", "1h"]);

    done(&setup);
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["answer"], &ask["answered_by"]],
        [&json!("sqlite"), &json!("autopilot")],
        "{ask}"
    );
    let records = traced(&setup, "factory.ask.autopilot_answered");
    assert_eq!(records.len(), 1, "{records:?}");
    let data = &records[0]["data"];
    let why =
        "its confidence, high, meets high, and a person decides again before any irreversible step";
    assert_eq!(
        [
            &data["ask"],
            &data["option"],
            &data["why"],
            &data["window"],
            &data["rationale"]
        ],
        [
            &json!("1"),
            &json!("sqlite"),
            &json!(why),
            &json!(1),
            &json!(RATIONALE)
        ],
        "{data}"
    );
    let desk = setup.desk_json();
    assert_eq!(
        [&desk["autopilot"]["level"], &desk["autopilot"]["answered"]],
        [&json!("high"), &json!(1)],
        "{desk}"
    );
    assert!(
        desk["since_you_looked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["what"] == format!("ask 1 answered \"sqlite\" by autopilot: {why}")),
        "{desk}"
    );
    let report = setup.autopilot_report();
    assert_eq!(
        [
            &report["answered"][0]["question"],
            &report["answered"][0]["option"],
            &report["answered"][0]["why"],
            &report["answered"][0]["reverse"]
        ],
        [
            &json!("Which database?"),
            &json!("sqlite"),
            &json!(why),
            &json!(null)
        ],
        "{report}"
    );
    assert_eq!(
        report["answered"][0]["no_reverse"],
        "task 1 is done and owns no pull request: ask for the change in a new task, with `priory add TEXT`"
    );
    assert_eq!(report["left"], json!([]));
    let printed = setup.autopilot(&["report"]);
    assert!(
        String::from_utf8_lossy(&printed.stdout)
            .contains("  reverse: none. task 1 is done and owns no pull request"),
        "{printed:?}"
    );
}

#[test]
fn a_new_ask_is_answered_before_its_lead_and_off_stops_the_next_one() {
    let mut setup = hands_off("pilot-new", WORKFLOW);
    setup.autopilot_on(&[]);
    setup.calls(&[confident_ask("high"), confident_ask("high")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |_| {
        asks_json(&setup)["asks"][0]["answered_by"] == "autopilot"
    });
    assert_eq!(setup.rows("tasks").len(), 1, "no lead task");

    assert!(setup.autopilot(&["off", &factory(&setup)]).status.success());
    setup.release("hold-stop");

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(asks[0][0], "2");
    assert_eq!(asks_json(&setup)["asks"][1]["routed_to"], "lead");
    let status = setup.autopilot(&["status"]);
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "autopilot is off\n"
    );
}

#[test]
fn the_end_passes_while_the_daemon_is_down_and_the_next_start_reports_it() {
    let mut setup = asleep("pilot-end", WORKFLOW, "high");
    setup.crash();
    setup.autopilot_on(&["--for", "5s"]);
    std::thread::sleep(Duration::from_millis(5100));

    setup.start();

    let inbox = setup.until("inbox", |rows| rows.iter().any(|row| row[2] == "autopilot"));
    let line = inbox.iter().find(|row| row[2] == "autopilot").unwrap();
    assert_eq!(
        line[5],
        "autopilot ended: it answered 0 and left 0. See `priory autopilot report 1`"
    );
    waits_for_a_person(&setup);
    let desk = setup.desk_json();
    assert_eq!(desk["autopilot"], json!(null));
    assert_eq!(setup.autopilot_report()["why"], "time");
}

#[test]
fn a_restart_inside_the_window_keeps_it_on_and_a_second_start_reports_no_end() {
    let mut setup = Setup::new("pilot-restart");
    setup.autopilot_on(&["--for", "1h"]);
    setup.start();
    setup.crash();
    setup.calls(&[confident_ask("high"), result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    done(&setup);
    assert_eq!(only_ask(&setup)["answered_by"], "autopilot");
    assert!(setup.rows("inbox").iter().all(|row| row[2] != "autopilot"));
}

#[test]
fn an_ask_under_the_level_is_left_marked_and_medium_answers_it() {
    let setup = asleep("pilot-level", WORKFLOW, "medium");

    setup.autopilot_on(&["--for", "1h"]);

    assert_eq!(left(&setup, "1"), "its confidence is medium, under high");
    let desk = setup.desk_json();
    let item = &desk["items"][0];
    assert_eq!(
        item["autopilot_left"],
        "its confidence is medium, under high"
    );
    assert!(
        item["what"]
            .as_str()
            .unwrap()
            .starts_with("autopilot left this (its confidence is medium, under high): "),
        "{item}"
    );
    let report = setup.autopilot_report();
    assert_eq!(
        [&report["left"][0]["state"], &report["left"][0]["reverse"]],
        [&json!("open"), &json!(null)],
        "{report}"
    );

    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);

    done(&setup);
    assert_eq!(only_ask(&setup)["answered_by"], "autopilot");
}

#[test]
fn a_lane_it_does_not_cover_is_left() {
    let setup = asleep("pilot-lane", WORKFLOW, "high");

    setup.autopilot_on(&["--for", "1h", "--lane", "core/other"]);

    assert_eq!(left(&setup, "1"), "its lane is not covered");

    setup.autopilot_on(&["--for", "1h", "--lane", "core/other", "--lane", "none/misc"]);

    done(&setup);
}

#[test]
fn an_ask_without_confidence_or_toward_an_irreversible_step_is_left() {
    let mut plain = Setup::new("pilot-no-confidence");
    plain.calls(&[ask()]);
    plain.publish(&requested(42));
    plain.start();
    waits_for_a_person(&plain);
    let irreversible = asleep("pilot-irreversible", AUTONOMOUS, "high");

    plain.autopilot_on(&["--for", "1h", "--level", "medium"]);
    irreversible.autopilot_on(&["--for", "1h", "--level", "medium"]);

    assert_eq!(left(&plain, "1"), "its agent stated no confidence");
    assert_eq!(
        left(&irreversible, "1"),
        "answering it can let irreversible step open-pr run before a person decides again"
    );
}

#[test]
fn a_command_ask_and_an_approval_are_left() {
    let mut command = Setup::with_workflow("pilot-command", STEPS);
    fs::write(
        command.dir.join("factory/factory.toml"),
        "name = \"demo\"\n\n[asks.targets]\nops = \"ops\"\n",
    )
    .unwrap();
    fs::create_dir_all(command.dir.join("factory/ops")).unwrap();
    let mut ask = command_ask(json!({ "argv": ["./go"], "target": "ops" }));
    ask["arguments"]["confidence"] = json!("high");
    ask["arguments"]["rationale"] = json!("the fix is tested");
    command.calls(&[ask]);
    command.autopilot_on(&["--for", "1h"]);
    command.publish(&requested(42));
    let mut approval = Setup::with_workflow("pilot-approval", OPEN_PR);
    approval.calls(&[passed_with_evidence()]);
    approval.autopilot_on(&["--for", "1h"]);
    approval.publish(&requested(42));

    command.start();
    approval.start();

    assert_eq!(left(&command, "1"), "it runs a command");
    assert_eq!(
        left(&approval, "1"),
        "step approve-pr approves irreversible step open-pr"
    );
}

#[test]
fn a_batchs_go_is_left() {
    let mut setup = batched("pilot-go");
    let batch = json!({ "title": "t", "entries": three_entries() });
    setup.calls(&[planned(batch)]);
    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);
    batch_add(&setup, &["ENG-40", "ENG-41", "ENG-43"]);

    setup.start();

    assert_eq!(
        left(&setup, "1"),
        "step go approves irreversible step start"
    );
}

#[test]
fn a_step_an_edit_removed_is_left() {
    let mut setup = asleep("pilot-removed", WORKFLOW, "high");
    setup.crash();
    let file = setup.dir.join("factory/workflows/implement.toml");
    let renamed = fs::read_to_string(&file)
        .unwrap()
        .replace("\"implement\"", "\"build\"");
    fs::write(&file, renamed).unwrap();
    setup.start();

    setup.autopilot_on(&["--for", "1h"]);

    assert_eq!(
        left(&setup, "1"),
        "its step implement is no longer in the workflow"
    );
}

#[test]
fn a_snoozed_ask_is_left_until_its_snooze_ends() {
    let setup = asleep("pilot-snoozed", WORKFLOW, "high");
    let snoozed = setup.priory(&["snooze", "ask", "1", "--for", "1h"]);
    assert!(snoozed.status.success(), "{snoozed:?}");

    setup.autopilot_on(&["--for", "2h"]);

    assert_eq!(left(&setup, "1"), "it is snoozed");
    assert!(
        setup
            .priory(&["snooze", "ask", "1", "--clear"])
            .status
            .success()
    );
    done(&setup);
}

#[test]
fn an_ask_already_with_a_lead_is_left_to_it() {
    let mut setup = hands_off("pilot-lead", WORKFLOW);
    setup.memory(0);
    setup.calls(&[confident_ask("high"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.memory(60);
    setup.hold("hold-submit");
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "running")
    });
    setup.release("hold-submit");
    setup.hold("hold-stop");
    setup.until("asks", |rows| rows.len() == 1);

    setup.autopilot_on(&["--for", "1h"]);

    assert_eq!(left(&setup, "1"), "it is with lead");
}

#[test]
fn a_spend_caps_hold_and_an_outside_ask_are_left() {
    let mut setup = Setup::new("pilot-priory");
    setup.caps("[caps]\ntask = 50.0\n");
    setup.calls(&[serde_json::Value::Null]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "held")
    });
    let fields = json!({ "question": "Ship?", "options": ["yes"], "recommendation": "yes" });
    let asked = setup.priory(&["ask", "ship.1", &fields.to_string()]);
    assert!(asked.status.success(), "{asked:?}");

    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);

    assert_eq!(left(&setup, "1"), "Priory asked it, not an agent");
    assert_eq!(left(&setup, "ship.1"), "it was asked outside Priory");
}

#[test]
fn a_person_who_answers_first_wins_and_the_report_says_so() {
    let setup = asleep("pilot-person-first", WORKFLOW, "medium");
    setup.autopilot_on(&["--for", "1h"]);
    left(&setup, "1");

    let answered = setup.priory(&["answer", "1", "postgres", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);
    done(&setup);
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["answer"], &ask["answered_by"]],
        [&json!("postgres"), &json!("cli")]
    );
    let first = setup.autopilot(&["report", "1", "--json"]);
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first["left"][0]["answered_by"], "cli", "{first}");
    assert!(traced(&setup, "factory.ask.autopilot_answered").is_empty());
}

#[test]
fn a_persons_answer_replaces_autopilots_while_its_prompt_is_unsent() {
    let mut setup = Setup::new("pilot-replace");
    setup.calls(&[confident_ask("high"), result("passed")]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |_| {
        only_ask_or_null(&setup)["answered_by"] == "autopilot"
    });
    assert_eq!(reverse(&setup), "priory answer --revision 1 1 OPTION");

    reverses(&setup, &reverse(&setup), "postgres");

    let replaced = until_traced(&setup, "factory.ask.autopilot_replaced");
    let data = &replaced["data"];
    assert_eq!(
        [
            &data["task"],
            &data["ask"],
            &data["revision"],
            &data["option"],
            &data["replaced"],
            &data["by"]
        ],
        [
            &json!("1"),
            &json!("1"),
            &json!(1),
            &json!("postgres"),
            &json!("sqlite"),
            &json!("cli")
        ],
        "{data}"
    );
    setup.release("hold-stop");
    done(&setup);
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["answer"], &ask["answered_by"]],
        [&json!("postgres"), &json!("cli")],
        "{ask}"
    );
    let typed = setup.typed();
    assert!(
        typed[1].starts_with("Answer to ask 1 (revision 1): postgres\n"),
        "{typed:?}"
    );
    assert!(
        typed.iter().all(|text| !text.contains("sqlite")),
        "{typed:?}"
    );
}

fn only_ask_or_null(setup: &Setup) -> serde_json::Value {
    asks_json(setup)["asks"][0].clone()
}

#[test]
fn a_persons_answer_after_the_agent_read_autopilots_is_refused_with_what_to_do() {
    let mut setup = Setup::new("pilot-late");
    setup.calls(&[confident_ask("high"), serde_json::Value::Null]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));
    setup.start();
    let deadline = Instant::now() + Duration::from_secs(60);
    while setup.typed().len() < 2 {
        assert!(Instant::now() < deadline, "the answer was never typed");
        std::thread::sleep(Duration::from_millis(50));
    }

    let refused = setup.priory(&["answer", "1", "postgres", "--revision", "1"]);

    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("already answered: sqlite by autopilot, and its task went on: tell it with `priory say 1 TEXT`"),
        "{refused:?}"
    );
    assert_eq!(only_ask(&setup)["answer"], "sqlite");
}

#[test]
fn a_dismissed_ask_is_never_answered_and_the_report_says_it_was_withdrawn() {
    let setup = asleep("pilot-withdrawn", WORKFLOW, "medium");
    setup.autopilot_on(&["--for", "1h"]);
    left(&setup, "1");
    assert!(setup.priory(&["dismiss", "1"]).status.success());

    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);

    done(&setup);
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["state"], &ask["answer"]],
        [&json!("withdrawn"), &json!(null)]
    );
    let first = setup.autopilot(&["report", "1", "--json"]);
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first["left"][0]["state"], "withdrawn", "{first}");
}

/// A `plan` setup with a critic that passes each plan it reads, whose `approve_plans` names its
/// workflow, or not, ending its plan `grade`.
fn plan(name: &str, grade: &str, approve: bool) -> Setup {
    let mut setup = intervened(name, GRADED, "plan", "");
    critic(&setup, &["pass\nThe plan is sound."]);
    if approve {
        approve_plans(&setup);
    }
    setup.calls(&[
        json!({ "name": "result", "arguments": { "outcome": grade, "summary": "Plan" } }),
    ]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));
    setup.start();
    setup
}

fn approve_plans(setup: &Setup) {
    let settings = setup.dir.join("factory/factory.toml");
    let text = fs::read_to_string(&settings).unwrap();
    fs::write(
        &settings,
        format!("{text}\n[autopilot]\napprove_plans = [\"implement\"]\n"),
    )
    .unwrap();
}

#[test]
fn a_green_plan_of_a_named_workflow_is_approved_and_the_rest_are_left() {
    let green = plan("pilot-plan-green", "green", true);
    let yellow = plan("pilot-plan-yellow", "yellow", true);
    let unnamed = plan("pilot-plan-unnamed", "green", false);

    green.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[3] == "regrade")
    });
    assert_eq!(left(&yellow, "1"), "its plan is graded yellow, not green");
    assert_eq!(
        left(&unnamed, "1"),
        "it approves a plan, and approve_plans does not name implement"
    );
    assert_eq!(
        green.autopilot_report()["answered"][0]["option"],
        "approved"
    );
    green.calls(&[]);
    assert_eq!(
        reverse(&green),
        "priory cancel 1 && priory revive 1 --step plan"
    );

    reverses(&green, &reverse(&green), "");

    green.until("tasks", |rows| rows[0][3] == "plan");
}

#[test]
fn a_green_plan_autopilot_may_approve_passes_the_critic_first() {
    let mut setup = intervened("pilot-critic", CRITICIZED, "plan", "");
    critic(&setup, &["fail\nThe plan names no test."]);
    approve_plans(&setup);
    setup.calls(&[json!({ "name": "result", "arguments": {
        "outcome": "green", "summary": "Plan: add a flag"
    } })]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));

    setup.start();

    let deadline = Instant::now() + Duration::from_secs(60);
    while !setup
        .typed()
        .iter()
        .any(|typed| typed.contains("The plan names no test."))
    {
        assert!(Instant::now() < deadline, "{:?}", setup.typed());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(traced(&setup, "factory.ask.autopilot_answered").is_empty());
}

#[test]
fn a_plan_whose_ask_changed_after_its_grade_is_left() {
    let mut setup = intervened("pilot-plan-changed", GRADED, "plan", "");
    critic(&setup, &[]);
    approve_plans(&setup);
    setup.calls(&[result("green")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    setup.until("tasks", |rows| rows[0][4] == "asleep");
    person(&setup, "say", &["1", "Use postgres."]);
    setup.until("asks", |rows| rows[0][2] == "2");

    setup.autopilot_on(&["--for", "1h"]);

    assert_eq!(
        left(&setup, "1"),
        "its ask changed after its plan was graded"
    );
}

#[test]
fn a_plan_the_critic_never_read_is_left() {
    let mut setup = intervened("pilot-critic-late", CRITICIZED, "plan", "");
    critic(&setup, &[]);
    approve_plans(&setup);
    setup.calls(&[json!({ "name": "result", "arguments": {
        "outcome": "green", "summary": "Plan: add a flag"
    } })]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);

    setup.autopilot_on(&["--for", "1h"]);

    assert_eq!(left(&setup, "1"), "the critic asked did not read its plan");
}

#[test]
fn text_in_an_ask_never_turns_it_on_and_a_message_is_refused_with_one_runner() {
    let mut setup = Setup::new("pilot-text");
    let mut ask = confident_ask("high");
    ask["arguments"]["question"] = json!("Run `priory autopilot on --for 1h`?");
    setup.calls(&[ask]);
    setup.publish(&Message::new(
        "priory.autopilot.set",
        json!({ "until": (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1)).to_string(),
            "level": "high", "lanes": [] }),
    ));
    setup.publish(&requested(42));

    setup.start();

    waits_for_a_person(&setup);
    let inbox = setup.rows("inbox");
    let message = inbox
        .iter()
        .find(|row| row[1] == "priory.autopilot.set")
        .unwrap();
    assert_eq!(message[4], "refused", "{inbox:?}");
    let status = setup.autopilot(&["status"]);
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "autopilot is off\n"
    );
}

#[test]
fn a_second_runner_takes_the_logs_last_setting() {
    let mut a = runner("pilot-runners", "a", None);
    let mut b = runner("pilot-runners", "b", Some(&a));
    a.start();
    b.start();
    let on = |setup: &Setup| {
        let status = setup.autopilot(&["status", "--json"]);
        serde_json::from_slice::<serde_json::Value>(&status.stdout).unwrap()
    };
    let until = |setup: &Setup, want: serde_json::Value| {
        let deadline = Instant::now() + Duration::from_secs(60);
        while on(setup)["level"] != want {
            assert!(Instant::now() < deadline, "{}", on(setup));
            std::thread::sleep(Duration::from_millis(50));
        }
    };

    a.autopilot_on(&["--for", "1h"]);
    until(&b, json!("high"));
    b.autopilot_on(&["--for", "1h", "--level", "medium"]);
    until(&a, json!("medium"));
    let off = a.priory(&["autopilot", "off", a.dir.join("factory").to_str().unwrap()]);
    assert!(off.status.success(), "{off:?}");

    until(&b, json!(null));
    assert_eq!(on(&a), json!(null));
}

#[test]
fn a_view_only_machine_turns_it_on_for_every_runner() {
    let (a, laptop, _, _) = asked_on_a_viewed("pilot-view");

    laptop.autopilot_on(&["--for", "1h"]);

    let deadline = Instant::now() + Duration::from_secs(60);
    while a.desk_json()["autopilot"].is_null() {
        assert!(Instant::now() < deadline, "a never read the setting");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!laptop.desk_json()["autopilot"].is_null());
    let off = laptop.priory(&[
        "autopilot",
        "off",
        laptop.dir.join("factory").to_str().unwrap(),
    ]);
    assert!(off.status.success(), "{off:?}");
    while !a.desk_json()["autopilot"].is_null() {
        assert!(Instant::now() < deadline, "a never read the off");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A step with an irreversible effect whose agent can ask, after a person's approval.
const ANNOUNCE: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "approve", failed = "stop" }

[[steps]]
id = "approve"
decides = "human"
effect = "none"
approves = ["announce"]
options = ["go", "stop"]
next = { go = "announce", stop = "stop" }

[[steps]]
id = "announce"
decides = "model"
agent = "implementer"
prompt = "Announce the change."
effect = "irreversible"
next = { passed = "done", failed = "stop" }
"#;

const INSIDE: &str =
    "it was asked inside irreversible step announce, which runs on a person's approval";

/// `ANNOUNCE` with its announcement granted by the factory, and no approval.
fn autonomous_announce() -> String {
    let (start, approve) = ANNOUNCE.split_once("[[steps]]\nid = \"approve\"").unwrap();
    let (_, announce) = approve.split_once("[[steps]]").unwrap();
    format!("{start}[[steps]]{announce}")
        .replace("passed = \"approve\"", "passed = \"announce\"")
        .replace(
            "effect = \"irreversible\"",
            "effect = \"irreversible\"\nautonomous = \"it only drafts the announcement\"",
        )
}

/// Starts `setup`, whose workflow is `ANNOUNCE`, and approves its announcement, inside which its
/// agent asks with a high confidence.
fn asks_inside(setup: &mut Setup) {
    setup.calls(&[
        passed_with_evidence(),
        confident_ask("high"),
        result("passed"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    setup.commit(1, "widget.txt");
    let approved = setup.priory(&["answer", "1", "go", "--revision", "1"]);
    assert!(approved.status.success(), "{approved:?}");
    setup.until("asks", |rows| rows.len() == 1 && rows[0][0] == "2");
    setup.until("tasks", |rows| rows[0][4] == "asleep");
}

/// As `asks_inside`, for `autonomous_announce`, where no person approves the step first.
fn asks_inside_autonomous(setup: &mut Setup) {
    setup.calls(&[
        passed_with_evidence(),
        confident_ask("high"),
        result("passed"),
    ]);
    setup.publish(&requested(42));
    setup.start();
}

fn open_for_a_person(setup: &Setup) {
    let ask = &asks_json(setup)["asks"][1];
    assert_eq!(
        [&ask["state"], &ask["answered_by"], &ask["routed_to"]],
        [&json!("open"), &json!(null), &json!(null)],
        "{ask}"
    );
}

#[test]
fn autopilot_leaves_an_ask_inside_an_approved_irreversible_step_and_answers_an_autonomous_ones() {
    let mut approved = Setup::with_workflow("pilot-inside", ANNOUNCE);
    let mut autonomous = Setup::with_workflow("pilot-inside-auto", &autonomous_announce());
    approved.autopilot_on(&["--for", "1h"]);
    autonomous.autopilot_on(&["--for", "1h"]);

    asks_inside(&mut approved);
    asks_inside_autonomous(&mut autonomous);

    assert_eq!(left(&approved, "2"), INSIDE);
    open_for_a_person(&approved);
    done(&autonomous);
    let record = until_traced(&autonomous, "factory.ask.autopilot_answered");
    assert_eq!(
        record["data"]["why"],
        "its confidence, high, meets high, and the factory lets irreversible step announce run autonomously, and a person decides again before any later irreversible step"
    );
}

#[test]
fn auto_answer_leaves_an_ask_inside_an_approved_irreversible_step_and_answers_an_autonomous_ones() {
    let mut approved = auto_answering("auto-inside", ANNOUNCE, "medium", "");
    let mut autonomous = auto_answering("auto-inside-auto", &autonomous_announce(), "medium", "");

    asks_inside(&mut approved);
    asks_inside_autonomous(&mut autonomous);

    open_for_a_person(&approved);
    done(&autonomous);
    assert_eq!(only_ask(&autonomous)["answered_by"], "auto");
}

#[test]
fn a_lead_gets_no_ask_inside_an_approved_irreversible_step_and_gets_an_autonomous_ones() {
    let mut approved = hands_off("lead-inside", ANNOUNCE);
    let mut autonomous = hands_off("lead-inside-auto", &autonomous_announce());

    asks_inside(&mut approved);
    asks_inside_autonomous(&mut autonomous);

    open_for_a_person(&approved);
    assert_eq!(approved.rows("tasks").len(), 1, "no lead task");
    autonomous.until("tasks", |rows| rows.len() == 2);
    assert_eq!(only_ask(&autonomous)["routed_to"], "lead");
}

#[test]
fn an_outside_session_answers_no_ask_inside_an_approved_irreversible_step_but_an_autonomous_ones() {
    let mut approved = Setup::with_workflow("outside-inside", ANNOUNCE);
    let mut autonomous = Setup::with_workflow("outside-inside-auto", &autonomous_announce());
    let session = [
        ("HC_TERMINAL_ID", "t9"),
        ("CLAUDE_CODE_SESSION_ID", "8f3c2a10"),
    ];
    asks_inside(&mut approved);
    asks_inside_autonomous(&mut autonomous);
    waits_for_a_person(&autonomous);

    let (refused, failed) = approved.outside_call(
        &session,
        "answer",
        json!({"ask": 2, "revision": 1, "answer": "sqlite"}),
    );
    let (_, refused_autonomous) = autonomous.outside_call(
        &session,
        "answer",
        json!({"ask": 1, "revision": 1, "answer": "sqlite"}),
    );

    assert!(failed && refused.contains(INSIDE), "{refused}");
    open_for_a_person(&approved);
    assert!(!refused_autonomous);
    done(&autonomous);
}

#[test]
fn a_done_task_with_an_open_pull_request_is_told_to_reopen() {
    let mut setup = Setup::new("pilot-reverse-done");
    setup.calls(&[
        confident_ask("high"),
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.publish(&reviewed());
    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] != "-");
    setup.gone("priory-1");
    setup.publish(&check(1, 0));
    done(&setup);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(reverse(&setup), "priory say 1 TEXT");

    reverses(&setup, &reverse(&setup), "Use postgres instead.");

    setup.until("runs", |rows| rows.len() == 3);
}

#[test]
fn a_plan_whose_step_an_edit_removed_has_no_reversal() {
    let setup = plan("pilot-plan-removed", "green", true);
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[3] == "regrade")
    });
    let file = setup.dir.join("factory/workflows/implement.toml");
    let renamed = fs::read_to_string(&file)
        .unwrap()
        .replace("\"plan\"", "\"design\"");
    fs::write(&file, renamed).unwrap();

    let report = setup.autopilot_report();

    assert_eq!(
        [
            &report["answered"][0]["reverse"],
            &report["answered"][0]["no_reverse"]
        ],
        [
            &json!(null),
            &json!("task 1 cannot revive at step plan, which is no longer in workflow implement")
        ],
        "{report}"
    );
}

#[test]
fn a_dropped_tasks_answer_is_reversed_by_a_revive_that_works() {
    let mut setup = Setup::new("pilot-reverse-dropped");
    setup.calls(&[confident_ask("high"), result("failed")]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first()
            .is_some_and(|row| row[4] == "dropped" || row[4] == "failed")
    });
    assert_eq!(reverse(&setup), "priory revive 1 --step implement");
    setup.calls(&[]);

    reverses(&setup, &reverse(&setup), "");
}

#[test]
fn a_working_tasks_answer_is_reversed_by_a_say_that_works() {
    let mut setup = Setup::new("pilot-reverse-say");
    setup.calls(&[confident_ask("high"), serde_json::Value::Null]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(reverse(&setup), "priory say 1 TEXT");

    reverses(&setup, &reverse(&setup), "Use postgres instead.");
}

#[test]
fn off_ends_it_here_and_fails_when_the_factory_names_no_runner_to_tell() {
    let setup = Setup::new("pilot-off-broken");
    setup.autopilot_on(&["--for", "1h"]);
    fs::write(setup.dir.join("factory/factory.toml"), "name = [").unwrap();

    let off = setup.autopilot(&["off", &factory(&setup)]);

    assert!(!off.status.success(), "{off:?}");
    assert!(
        String::from_utf8_lossy(&off.stderr)
            .contains("it is off here, and no other runner was told: the factory does not load"),
        "{off:?}"
    );
    let status = setup.autopilot(&["status"]);
    assert_eq!(
        String::from_utf8_lossy(&status.stdout),
        "autopilot is off\n"
    );
}

/// How many autopilot settings `setup` read from the log.
fn settings_read(setup: &Setup) -> usize {
    let inbox = setup.rows("inbox");
    inbox
        .iter()
        .filter(|row| row[1] == "priory.autopilot.set")
        .count()
}

fn status_level(setup: &Setup) -> serde_json::Value {
    let status = setup.autopilot(&["status", "--json"]);
    serde_json::from_slice::<serde_json::Value>(&status.stdout).unwrap()["level"].clone()
}

fn off(setup: &Setup) {
    let off = setup.autopilot(&["off", &factory(setup)]);
    assert!(off.status.success(), "{off:?}");
}

/// Waits until `setup` read `count` settings from the log.
fn until_read(setup: &Setup, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while settings_read(setup) < count {
        assert!(Instant::now() < deadline, "{:?}", setup.rows("inbox"));
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_off_then_an_on_made_while_the_daemon_is_down_stay_on_once_it_reads_them_back() {
    let mut a = runner("pilot-off-on", "a", None);
    let mut b = runner("pilot-off-on", "b", Some(&a));
    a.start();
    b.start();
    a.autopilot_on(&["--for", "1h"]);
    until_read(&a, 1);
    until_read(&b, 1);
    a.crash();
    off(&a);
    a.autopilot_on(&["--for", "1h", "--level", "medium"]);

    a.start();

    until_read(&a, 3);
    until_read(&b, 3);
    assert_eq!(
        [status_level(&a), status_level(&b)],
        [json!("medium"), json!("medium")]
    );
    let report = a.autopilot_report();
    assert_eq!(
        [&report["window"], &report["ended"]],
        [&json!(2), &json!(null)],
        "{report}"
    );
}

#[test]
fn an_off_is_not_undone_by_an_older_on_it_had_not_read() {
    let mut a = runner("pilot-off-unread", "a", None);
    let mut b = runner("pilot-off-unread", "b", Some(&a));
    a.start();
    b.start();
    a.crash();
    b.autopilot_on(&["--for", "1h"]);
    logged(&b, "priory.autopilot.set", json!({ "level": "high" }));
    off(&a);

    a.start();

    until_read(&a, 2);
    until_read(&b, 2);
    assert_eq!(
        [status_level(&a), status_level(&b)],
        [json!(null), json!(null)]
    );
    let report = a.autopilot(&["report"]);
    assert!(
        String::from_utf8_lossy(&report.stderr).contains("autopilot was never on"),
        "{report:?}"
    );
}

#[test]
fn an_off_that_told_no_runner_is_not_undone_by_an_older_on() {
    let mut a = runner("pilot-off-untold", "a", None);
    let mut b = runner("pilot-off-untold", "b", Some(&a));
    a.start();
    b.start();
    a.crash();
    b.autopilot_on(&["--for", "1h"]);
    logged(&b, "priory.autopilot.set", json!({ "level": "high" }));
    let settings = a.dir.join("factory/factory.toml");
    let text = fs::read_to_string(&settings).unwrap();
    fs::write(&settings, "name = [").unwrap();
    let off = a.autopilot(&["off", &factory(&a)]);
    assert!(!off.status.success(), "{off:?}");
    fs::write(&settings, text).unwrap();

    a.start();

    until_read(&a, 1);
    assert_eq!(status_level(&a), json!(null));
    assert_eq!(status_level(&b), json!("high"));
}

#[test]
fn a_lane_that_is_no_lane_is_refused() {
    let setup = Setup::new("pilot-bad-lane");
    let output = setup.autopilot(&["on", "--for", "1h", "--lane", "core"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("--lane `core` is not a lane such as `team/theme`"),
        "{output:?}"
    );
}

#[test]
fn the_ask_of_a_task_a_person_holds_is_left() {
    let mut setup = Setup::new("pilot-held");
    setup.hold("hold-stop");
    setup.calls(&[confident_ask("high")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    person(&setup, "hold", &["1", "--reason", "room for task 2"]);

    setup.autopilot_on(&["--for", "1h"]);

    assert_eq!(left(&setup, "1"), "its task is held");
}

#[test]
fn a_clock_set_back_ends_it() {
    let setup = asleep("pilot-clock", WORKFLOW, "medium");
    setup.autopilot_on(&["--for", "1h"]);
    left(&setup, "1");

    rusqlite::Connection::open(setup.dir.join("priory.db"))
        .unwrap()
        .execute(
            "UPDATE autopilot_state SET clock = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+55 seconds')",
            [],
        )
        .unwrap();

    setup.until("inbox", |rows| rows.iter().any(|row| row[2] == "autopilot"));
    assert_eq!(setup.autopilot_report()["why"], "clock");
    assert_eq!(status_level(&setup), json!(null));
}

#[test]
fn a_gates_ask_is_left() {
    let (setup, _) = held_by_an_uncalibrated_judge("pilot-gate", "name = \"demo\"\n");

    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);

    assert_eq!(left(&setup, "gate-1-1"), "Priory asked it, not an agent");
}

#[test]
fn a_debates_ask_is_left() {
    let setup = super::super::debate::split("pilot-debate");

    setup.autopilot_on(&["--for", "1h", "--level", "medium"]);

    assert_eq!(left(&setup, "1"), "it picks a debate's answer");
}

#[test]
fn an_approval_asked_again_at_a_new_commit_is_left() {
    let setup = asked_to_open("pilot-again", APPROVE_THEN_POLISH);
    setup.calls(&[passed_with_evidence()]);
    setup.hold("hold-stop");
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.written("fake/tools.log");
    while setup.replies().len() < 2 {
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.commit(1, "polish.txt");
    setup.autopilot_on(&["--for", "1h"]);

    setup.release("hold-stop");

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(asks[0][2], "2");
    assert_eq!(
        left(&setup, "1"),
        "step approve-pr approves irreversible step open-pr"
    );
    assert!(setup.rows("actions").is_empty());
}

#[test]
fn it_answers_the_open_revision_and_a_persons_answer_to_an_older_one_is_refused() {
    let mut setup = hands_off("pilot-stale", WORKFLOW);
    setup.calls(&[
        confident_ask("high"),
        json!({ "name": "result", "arguments": { "outcome": "skipped", "summary": "Not mine." } }),
        result("passed"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "done");

    setup.autopilot_on(&["--for", "1h"]);

    done(&setup);
    let record = until_traced(&setup, "factory.ask.autopilot_answered");
    assert_eq!(record["data"]["revision"], 2, "{record}");
    let stale = setup.priory(&["answer", "1", "postgres", "--revision", "1"]);
    assert!(!stale.status.success(), "{stale:?}");
    assert_eq!(only_ask(&setup)["answer"], "sqlite");
    assert!(traced(&setup, "factory.ask.autopilot_replaced").is_empty());
}

#[test]
fn an_answer_sent_from_another_runner_replaces_autopilots_while_its_prompt_is_unsent() {
    let mut a = runner("pilot-remote", "a", None);
    let mut b = runner("pilot-remote", "b", Some(&a));
    a.calls(&[confident_ask("high"), result("passed")]);
    a.autopilot_on(&["--for", "1h"]);
    a.hold("hold-stop");
    a.publish(&requested(42));
    a.start();
    a.until("tasks", |_| {
        only_ask_or_null(&a)["answered_by"] == "autopilot"
    });
    b.start();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !asks_json(&b)["asks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|ask| ask["id"] == "a:1" && ask["state"] == "answered")
    {
        assert!(Instant::now() < deadline, "{}", asks_json(&b));
        std::thread::sleep(Duration::from_millis(50));
    }

    let sent = b.priory(&["answer", "--revision", "1", &factory(&b), "a:1", "postgres"]);

    assert!(sent.status.success(), "{sent:?}");
    until_traced(&a, "factory.ask.autopilot_replaced");
    a.release("hold-stop");
    done(&a);
    assert_eq!(only_ask(&a)["answer"], "postgres");
}
