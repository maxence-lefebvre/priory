//! Approve by confidence: the daemon answers an agent's reversible ask with its recommendation
//! when the stated confidence meets the task's `auto_answer` level (docs/daemon/waits.md).

use super::*;

mod autopilot;

const RATIONALE: &str = "one file, no server to run";

/// A setup whose trigger sets `auto_answer = "<level>"`, with `extra` lines after it.
fn auto_answering(name: &str, workflow: &str, level: &str, extra: &str) -> Setup {
    let setup = Setup::with_workflow(name, workflow);
    fs::write(
        setup.dir.join("factory/triggers/agent-label.toml"),
        format!("{TRIGGER}auto_answer = \"{level}\"\n{extra}"),
    )
    .unwrap();
    setup
}

/// `ask()` with a recommendation among its options, stated at `confidence`.
fn confident_ask(confidence: &str) -> serde_json::Value {
    let mut ask = ask();
    ask["arguments"]["recommendation"] = json!("sqlite");
    ask["arguments"]["confidence"] = json!(confidence);
    ask["arguments"]["rationale"] = json!(RATIONALE);
    ask
}

/// Task 1's only ask, as `priory asks --all --json` lists it.
fn only_ask(setup: &Setup) -> serde_json::Value {
    let asks = asks_json(setup);
    assert_eq!(asks["asks"].as_array().map(Vec::len), Some(1), "{asks}");
    asks["asks"][0].clone()
}

/// Waits until task 1 sleeps on an open ask, and returns it.
fn waits_for_a_person(setup: &Setup) -> serde_json::Value {
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    let ask = only_ask(setup);
    assert_eq!(
        [&ask["state"], &ask["answered_by"]],
        [&json!("open"), &json!(null)],
        "{ask}"
    );
    assert!(traced(setup, "factory.ask.auto_answered").is_empty());
    ask
}

#[test]
fn an_ask_at_the_tasks_level_is_answered_by_auto_and_the_task_wakes_with_its_recommendation() {
    let mut setup = auto_answering("auto-high", WORKFLOW, "high", "");
    setup.calls(&[confident_ask("high"), result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let ask = only_ask(&setup);
    assert_eq!(
        [
            &ask["answer"],
            &ask["answered_by"],
            &ask["confidence"],
            &ask["rationale"]
        ],
        [
            &json!("sqlite"),
            &json!("auto"),
            &json!("high"),
            &json!(RATIONALE)
        ],
        "{ask}"
    );
    let typed = setup.typed();
    assert!(
        typed[1].starts_with("Answer to ask 1 (revision 1): sqlite\n"),
        "{typed:?}"
    );
    assert_eq!(setup.rows("runs").len(), 1, "the run never slept");
    let (text, error) = reply(&setup.replies()[0]);
    assert_eq!(
        (text.as_str(), error),
        (
            "ask 1 stored: end your turn now; the answer arrives as your next prompt",
            false
        )
    );
    let records = traced(&setup, "factory.ask.auto_answered");
    assert_eq!(records.len(), 1, "{records:?}");
    let data = &records[0]["data"];
    assert_eq!(
        [
            &data["task"],
            &data["step"],
            &data["ask"],
            &data["revision"],
            &data["option"],
            &data["confidence"],
            &data["rationale"],
            &data["level"]
        ],
        [
            &json!("1"),
            &json!("implement"),
            &json!("1"),
            &json!(1),
            &json!("sqlite"),
            &json!("high"),
            &json!(RATIONALE),
            &json!("high")
        ],
        "{data}"
    );
    let desk: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["desk", "--json"]).stdout).unwrap();
    let lines: Vec<&str> = desk["since_you_looked"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["what"].as_str())
        .collect();
    assert!(
        lines.contains(&"ask 1 answered \"sqlite\" by auto (high): one file, no server to run"),
        "{lines:?}"
    );

    let overturned = setup.priory(&["answer", "1", "postgres", "--revision", "1"]);

    assert!(!overturned.status.success(), "{overturned:?}");
    assert_eq!(only_ask(&setup)["answer"], "sqlite");
}

#[test]
fn a_medium_ask_goes_to_the_desk_under_high_with_its_confidence_and_rationale() {
    let mut setup = auto_answering("auto-medium-under-high", WORKFLOW, "high", "");
    setup.calls(&[confident_ask("medium")]);
    setup.publish(&requested(42));

    setup.start();

    let ask = waits_for_a_person(&setup);
    assert_eq!(
        [&ask["confidence"], &ask["rationale"]],
        [&json!("medium"), &json!(RATIONALE)]
    );
    let desk: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["desk", "--json"]).stdout).unwrap();
    let item = &desk["items"][0];
    assert_eq!(
        [&item["kind"], &item["confidence"], &item["rationale"]],
        [&json!("ask"), &json!("medium"), &json!(RATIONALE)],
        "{item}"
    );
}

#[test]
fn a_medium_ask_is_answered_under_medium() {
    let mut setup = auto_answering("auto-medium", WORKFLOW, "medium", "");
    setup.calls(&[confident_ask("medium"), result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    assert_eq!(only_ask(&setup)["answered_by"], "auto");
    assert_eq!(task_json(&setup, "1")["auto_answer"], "medium");
}

#[test]
fn an_ask_without_a_confidence_goes_to_the_desk_on_a_task_with_a_level() {
    let mut setup = auto_answering("auto-no-confidence", WORKFLOW, "medium", "");
    setup.calls(&[ask()]);
    setup.publish(&requested(42));

    setup.start();

    let ask = waits_for_a_person(&setup);
    assert_eq!(ask["confidence"], json!(null));
}

#[test]
fn an_ask_whose_answer_lets_an_irreversible_step_run_goes_to_the_desk_at_any_confidence() {
    let mut setup = auto_answering("auto-irreversible", AUTONOMOUS, "medium", "");
    setup.calls(&[confident_ask("high")]);
    setup.publish(&requested(42));

    setup.start();

    waits_for_a_person(&setup);
}

#[test]
fn a_human_step_that_approves_goes_to_the_desk() {
    let mut setup = auto_answering("auto-approves", OPEN_PR, "medium", "");
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));

    setup.start();

    let ask = waits_for_a_person(&setup);
    assert_eq!(ask["step"], "approve-pr");
}

#[test]
fn a_command_ask_goes_to_the_desk_at_any_confidence() {
    let mut setup = auto_answering("auto-command", STEPS, "medium", "");
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\n\n[asks.targets]\nops = \"ops\"\n",
    )
    .unwrap();
    fs::create_dir_all(setup.dir.join("factory/ops")).unwrap();
    let mut ask = command_ask(json!({ "argv": ["./go"], "target": "ops" }));
    ask["arguments"]["confidence"] = json!("high");
    ask["arguments"]["rationale"] = json!("the fix is tested");
    setup.calls(&[ask]);
    setup.publish(&requested(42));

    setup.start();

    let ask = waits_for_a_person(&setup);
    assert_eq!(ask["confidence"], "high");
}

#[test]
fn a_plan_ask_goes_to_the_desk() {
    let mut setup = auto_answering("auto-plan", GRADED, "medium", "intervention = \"plan\"\n");
    setup.calls(&[result("green")]);
    setup.publish(&requested(42));

    setup.start();

    let ask = waits_for_a_person(&setup);
    assert_eq!(ask["step"], "plan");
}

#[test]
fn a_holds_ask_goes_to_the_desk() {
    let mut setup = Setup::new("auto-hold");
    setup.start();
    let factory = setup.dir.join("factory");
    let output = setup.priory(&[
        "request",
        factory.to_str().unwrap(),
        "--workflow",
        "implement",
        "--key",
        "login",
        "--hold",
        "after the fix merges",
        "--merge",
        "owner/repo#7",
        "--auto-answer",
        "high",
        "--repo",
        "owner/repo",
        "Fix the login page",
    ]);
    assert!(output.status.success(), "{output:?}");
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "held")
    });
    assert_eq!(task_json(&setup, "1")["auto_answer"], "high");

    setup.publish(&on_pull(
        "github.pull_request.closed",
        json!({ "merged": false, "github_id": 400 }),
    ));

    setup.until("asks", |rows| rows.len() == 1);
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["state"], &ask["answered_by"]],
        [&json!("open"), &json!(null)],
        "{ask}"
    );
}

#[test]
fn an_ask_with_a_bad_confidence_is_refused_with_its_reason() {
    let mut setup = auto_answering("auto-refused", WORKFLOW, "high", "");
    let mut no_rationale = confident_ask("high");
    no_rationale["arguments"]
        .as_object_mut()
        .unwrap()
        .remove("rationale");
    let mut two_lines = confident_ask("high");
    two_lines["arguments"]["rationale"] = json!("one file\nno server");
    let mut off_option = confident_ask("high");
    off_option["arguments"]["recommendation"] = json!("sqlite: one file");
    let unknown = confident_ask("certain");
    let mut long = confident_ask("high");
    long["arguments"]["rationale"] = json!("x".repeat(201));
    setup.calls(&[json!([
        no_rationale,
        two_lines,
        off_option,
        unknown,
        long,
        result("passed")
    ])]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let refused: Vec<(String, bool)> = setup.replies().iter().take(5).map(reply).collect();
    let error = |text: &str| (text.to_owned(), true);
    assert_eq!(
        refused,
        [
            error("a confidence needs a rationale: one line on why"),
            error("rationale must be one line"),
            error(
                "a confidence needs a recommendation that is one of the options: sqlite, postgres"
            ),
            error("confidence must be high, medium or low"),
            error("rationale must be at most 200 characters"),
        ]
    );
    assert!(asks_json(&setup)["asks"].as_array().unwrap().is_empty());
}

#[test]
fn a_low_ask_of_a_hands_off_task_skips_the_lead_and_reaches_the_person() {
    let mut setup = hands_off("auto-low-lead", WORKFLOW);
    setup.calls(&[confident_ask("low")]);
    setup.publish(&requested(42));

    setup.start();

    let ask = waits_for_a_person(&setup);
    assert_eq!(ask["routed_to"], json!(null));
    assert_eq!(setup.rows("tasks").len(), 1, "no lead task");
}

#[test]
fn an_eligible_ask_of_a_hands_off_task_is_answered_by_auto_before_its_lead() {
    let mut setup = hands_off("auto-before-lead", WORKFLOW);
    let trigger = setup.dir.join("factory/triggers/agent-label.toml");
    let with_level = fs::read_to_string(&trigger).unwrap() + "auto_answer = \"high\"\n";
    fs::write(&trigger, with_level).unwrap();
    setup.calls(&[confident_ask("high"), result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["answered_by"], &ask["routed_to"]],
        [&json!("auto"), &json!(null)]
    );
    assert_eq!(setup.rows("tasks").len(), 1, "no lead task");
}

#[test]
fn an_approval_asked_again_at_a_new_commit_goes_to_the_desk() {
    let mut setup = auto_answering("auto-new-commit", APPROVE_THEN_POLISH, "medium", "");
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    setup.gone("priory-1");
    setup.calls(&[passed_with_evidence()]);
    setup.hold("hold-stop");
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.written("fake/tools.log");
    while setup.replies().len() < 2 {
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.commit(1, "polish.txt");

    setup.release("hold-stop");

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(asks[0][2], "2");
    assert_eq!(only_ask(&setup)["answered_by"], json!(null));
    assert!(traced(&setup, "factory.ask.auto_answered").is_empty());
}
