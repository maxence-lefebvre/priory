//! A person's own session answers and dismisses an ask through `priory mcp --outside`, only where a
//! lead could (`explain::delegable`), and Priory records the session as the decider
//! (docs/daemon/tools.md#the-outside-server).

use super::*;

/// The hc terminal and Claude session of the person's own session.
const SESSION: [(&str, &str); 2] = [
    ("HC_TERMINAL_ID", "t9"),
    ("CLAUDE_CODE_SESSION_ID", "8f3c2a10"),
];

/// A setup whose task 1 sleeps on its agent's ask at step `implement` of `workflow`, and then
/// passes once the ask is settled.
fn asleep_on_an_agent_ask(name: &str, workflow: &str) -> Setup {
    let mut setup = Setup::with_workflow(name, workflow);
    setup.calls(&[ask(), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup
}

/// Task 1's only ask, as `priory asks --all --json` lists it.
fn only_ask(setup: &Setup) -> serde_json::Value {
    let asks = asks_json(setup);
    assert_eq!(asks["asks"].as_array().map(Vec::len), Some(1), "{asks}");
    asks["asks"][0].clone()
}

/// The lines of Since you looked on the desk.
fn since_you_looked(setup: &Setup) -> Vec<String> {
    let desk: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["desk", "--json"]).stdout).unwrap();
    desk["since_you_looked"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["what"].as_str().map(str::to_owned))
        .collect()
}

/// Asserts the ask is still open, decided by nobody, with no session record in the trace.
fn still_open(setup: &Setup) {
    let ask = only_ask(setup);
    assert_eq!(
        [&ask["state"], &ask["answered_by"]],
        [&json!("open"), &json!(null)],
        "{ask}"
    );
    for kind in [
        "factory.ask.session_answered",
        "factory.ask.session_dismissed",
    ] {
        assert!(traced(setup, kind).is_empty(), "{kind}");
    }
}

#[test]
fn an_outside_session_answers_an_agents_reversible_ask_and_is_recorded_as_the_decider() {
    let setup = asleep_on_an_agent_ask("outside-answers", WORKFLOW);

    let (text, failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": 1, "revision": 1, "answer": "sqlite"}),
    );

    assert_eq!(
        (text.as_str(), failed),
        (
            "ask 1 answered; Priory records that session t9 8f3c2a10 decided it, not a person",
            false
        )
    );
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["answer"], &ask["answered_by"]],
        [&json!("sqlite"), &json!("session:t9 8f3c2a10")],
        "{ask}"
    );
    let typed = setup.typed();
    assert!(
        typed[1].starts_with("Answer to ask 1 (revision 1): sqlite\n"),
        "{typed:?}"
    );
    let records = traced(&setup, "factory.ask.session_answered");
    assert_eq!(records.len(), 1, "{records:?}");
    let data = &records[0]["data"];
    assert_eq!(
        [
            &data["task"],
            &data["step"],
            &data["ask"],
            &data["revision"],
            &data["option"],
            &data["session"]
        ],
        [
            &json!("1"),
            &json!("implement"),
            &json!("1"),
            &json!(1),
            &json!("sqlite"),
            &json!("t9 8f3c2a10")
        ],
        "{data}"
    );
    let lines = since_you_looked(&setup);
    assert!(
        lines.contains(&"ask 1 answered \"sqlite\" by session t9 8f3c2a10".to_owned()),
        "{lines:?}"
    );
}

#[test]
fn an_outside_session_dismisses_an_agents_reversible_ask_and_is_recorded_as_the_decider() {
    let setup = asleep_on_an_agent_ask("outside-dismisses", WORKFLOW);

    let (text, failed) = setup.outside_call(&SESSION, "dismiss", json!({"ask": 1, "revision": 1}));

    assert_eq!(
        (text.as_str(), failed),
        (
            "ask 1 dismissed; Priory records that session t9 8f3c2a10 decided it, not a person",
            false
        )
    );
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let ask = only_ask(&setup);
    assert_eq!(
        [&ask["state"], &ask["withdrawn_why"], &ask["answered_by"]],
        [
            &json!("withdrawn"),
            &json!("dismissed by session t9 8f3c2a10"),
            &json!("session:t9 8f3c2a10")
        ],
        "{ask}"
    );
    let typed = setup.typed();
    assert!(
        typed[1].starts_with(
            "Session t9 8f3c2a10 dismissed ask 1 without an answer. Continue your task."
        ),
        "{typed:?}"
    );
    let records = traced(&setup, "factory.ask.session_dismissed");
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(
        [&records[0]["data"]["ask"], &records[0]["data"]["option"]],
        [&json!("1"), &json!(null)]
    );
    let lines = since_you_looked(&setup);
    assert!(
        lines.contains(&"ask 1 dismissed by session t9 8f3c2a10".to_owned()),
        "{lines:?}"
    );
}

#[test]
fn an_outside_session_answers_a_human_steps_ask_and_an_outside_ask_by_its_name() {
    let mut setup = Setup::with_workflow("outside-step", STEPS);
    setup.calls(&[result("unsure"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);

    let (step, step_failed) = setup.outside_call(
        &[],
        "answer",
        json!({"ask": 1, "revision": 1, "answer": "approved"}),
    );
    let (outside, outside_failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": "cache.ship", "revision": 1, "answer": "ship"}),
    );

    assert!(!step_failed, "{step}");
    assert!(!outside_failed, "{outside}");
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let asks = asks_json(&setup);
    let by: Vec<_> = asks["asks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ask| (ask["id"].clone(), ask["answered_by"].clone()))
        .collect();
    assert_eq!(
        by,
        [
            (json!(1), json!("session:outside")),
            (json!("cache.ship"), json!("session:t9 8f3c2a10"))
        ],
        "{asks}"
    );
    let records = traced(&setup, "factory.ask.session_answered");
    let outside = &records[1]["data"];
    assert_eq!(
        [&outside["ask"], &outside["option"], &outside["task"]],
        [&json!("cache.ship"), &json!("ship"), &json!(null)],
        "{outside}"
    );
}

#[test]
fn an_ask_that_approves_an_irreversible_step_is_refused_and_stays_the_persons() {
    let mut setup = Setup::with_workflow("outside-approves", OPEN_PR);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);

    let (answered, answer_failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": 1, "revision": 1, "answer": "open"}),
    );
    let (dismissed, dismiss_failed) =
        setup.outside_call(&SESSION, "dismiss", json!({"ask": 1, "revision": 1}));

    let refused = "ask 1 waits for a person: step approve-pr approves irreversible step open-pr. A person answers it in the desk.";
    assert_eq!((answered.as_str(), answer_failed), (refused, true));
    assert_eq!((dismissed.as_str(), dismiss_failed), (refused, true));
    still_open(&setup);
}

#[test]
fn an_ask_before_an_irreversible_step_is_refused_and_stays_the_persons() {
    let setup = asleep_on_an_agent_ask("outside-irreversible", AUTONOMOUS);

    let (text, failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": 1, "revision": 1, "answer": "sqlite"}),
    );

    assert_eq!(
        (text.as_str(), failed),
        (
            "ask 1 waits for a person: answering it can let irreversible step open-pr run before a person decides again. A person answers it in the desk.",
            true
        )
    );
    still_open(&setup);
}

#[test]
fn an_ask_that_runs_a_command_is_refused_and_stays_the_persons() {
    let setup = asleep_on_a_command_ask("outside-command", go_in_ops());

    let (text, failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": 1, "revision": 1, "answer": "wait"}),
    );

    assert_eq!(
        (text.as_str(), failed),
        (
            "ask 1 waits for a person: it runs a command. A person answers it in the desk.",
            true
        )
    );
    still_open(&setup);
    assert!(!setup.dir.join("factory/ops/runs.log").exists());
}

#[test]
fn a_stale_revision_is_refused_as_the_cli_refuses_it() {
    let setup = asleep_on_an_agent_ask("outside-stale", WORKFLOW);

    let (answered, answer_failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": 1, "revision": 2, "answer": "sqlite"}),
    );
    let (dismissed, dismiss_failed) =
        setup.outside_call(&SESSION, "dismiss", json!({"ask": 1, "revision": 0}));
    let (unrevised, unrevised_failed) =
        setup.outside_call(&SESSION, "answer", json!({"ask": 1, "answer": "sqlite"}));

    assert_eq!(
        (answered.as_str(), answer_failed),
        ("ask 1 is at revision 1, not 2", true)
    );
    assert_eq!(
        (dismissed.as_str(), dismiss_failed),
        ("ask 1 is at revision 1, not 0", true)
    );
    assert_eq!(
        (unrevised.as_str(), unrevised_failed),
        (
            "revision must be the ask's revision, as asks lists it",
            true
        )
    );
    still_open(&setup);
}

#[test]
fn a_run_still_cannot_start_the_outside_server_to_answer() {
    let setup = asleep_on_an_agent_ask("outside-in-a-run", WORKFLOW);

    let (exit, replies) = setup.outside(
        &[("PRIORY_SESSION", "6f1c0b9e")],
        &[outside_call(
            "answer",
            json!({"ask": 1, "revision": 1, "answer": "sqlite"}),
        )],
    );

    assert_eq!(exit.status.code(), Some(2), "{exit:?}");
    assert!(replies.is_empty(), "{replies:?}");
    still_open(&setup);
}

#[test]
fn a_session_cannot_answer_the_ask_it_put_to_the_person() {
    let mut setup = Setup::new("outside-own-ask");
    setup.start();
    listening(&setup);
    let (stored, _) = setup.outside_call(&SESSION, "ask", mcp_ask("cache.ship", &["ship", "wait"]));
    assert!(stored.starts_with("stored as ask cache.ship"), "{stored}");

    let (text, failed) = setup.outside_call(
        &SESSION,
        "answer",
        json!({"ask": "cache.ship", "revision": 1, "answer": "ship"}),
    );

    assert_eq!(
        (text.as_str(), failed),
        (
            "ask cache.ship waits for a person: this session asked it. A person answers it in the desk.",
            true
        )
    );
    still_open(&setup);
}
