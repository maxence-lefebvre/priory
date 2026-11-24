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
        let output = self.autopilot(&["report", "--json"]);
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
            &json!("priory revive 1 --step implement")
        ],
        "{report}"
    );
    assert_eq!(report["left"], json!([]));
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

    assert!(setup.autopilot(&["off"]).status.success());
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
    setup.autopilot_on(&["--for", "1s"]);
    std::thread::sleep(Duration::from_millis(2100));

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
    assert_eq!(
        setup.autopilot_report()["answered"][0]["reverse"],
        "priory answer --revision 1 1 OPTION"
    );

    let answered = setup.priory(&["answer", "1", "postgres", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
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

/// A `plan` setup whose `approve_plans` names its workflow, or not, ending its plan `grade`.
fn plan(name: &str, grade: &str, approve: bool) -> Setup {
    let mut setup = intervened(name, GRADED, "plan", "");
    if approve {
        fs::write(
            setup.dir.join("factory/factory.toml"),
            "name = \"demo\"\n\n[autopilot]\napprove_plans = [\"implement\"]\n",
        )
        .unwrap();
    }
    setup.calls(&[
        json!({ "name": "result", "arguments": { "outcome": grade, "summary": "Plan" } }),
    ]);
    setup.autopilot_on(&["--for", "1h"]);
    setup.publish(&requested(42));
    setup.start();
    setup
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
    let report = green.autopilot_report();
    assert_eq!(
        [
            &report["answered"][0]["option"],
            &report["answered"][0]["reverse"]
        ],
        [
            &json!("approved"),
            &json!("priory cancel 1, then priory revive 1 --step plan")
        ],
        "{report}"
    );
}

#[test]
fn a_green_plan_autopilot_may_approve_passes_the_critic_first() {
    let mut setup = intervened("pilot-critic", CRITICIZED, "plan", "");
    critic(&setup, &["fail\nThe plan names no test."]);
    let settings = setup.dir.join("factory/factory.toml");
    let text = fs::read_to_string(&settings).unwrap();
    fs::write(
        &settings,
        format!("{text}\n[autopilot]\napprove_plans = [\"implement\"]\n"),
    )
    .unwrap();
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
fn a_plan_the_critic_never_read_is_left() {
    let mut setup = intervened("pilot-critic-late", CRITICIZED, "plan", "");
    critic(&setup, &[]);
    let settings = setup.dir.join("factory/factory.toml");
    let text = fs::read_to_string(&settings).unwrap();
    fs::write(
        &settings,
        format!("{text}\n[autopilot]\napprove_plans = [\"implement\"]\n"),
    )
    .unwrap();
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
