//! Debate steps through the daemon, with the fake harness scripted to call `position`.

use super::*;

const ARCHITECT: &str = "+++\ndescription = \"Proposes designs.\"\nharness = \"claude-code\"\nmodel = \"claude-fake-5\"\n+++\nYou weigh designs.\n";

const SKEPTIC: &str = "+++\ndescription = \"Doubts designs.\"\nharness = \"claude-code\"\nmodel = \"claude-fake-5\"\n+++\nYou doubt designs.\n";

const CODEX_SKEPTIC: &str = "+++\ndescription = \"Doubts designs.\"\nharness = \"codex\"\nmodel = \"gpt-fake-5\"\n+++\nYou doubt designs.\n";

/// Dollars per million tokens: Claude's input at 1, Codex's input at 10.
const PRICES: &str = "[claude-fake-5]\ninput = 1.0\noutput = 2.0\ncache_read = 0.5\ncache_write = 1.0\n\n[gpt-fake-5]\ninput = 10.0\noutput = 20.0\ncache_read = 5.0\ncache_write = 0.0\n";

const QUESTION: &str = "Should the cache live in the daemon or in its own process?";

/// A workflow whose start is a debate of `rounds` rounds and `cap`, whose verdicts end the task.
fn debate_workflow(rounds: u32, cap: &str, timeout: &str) -> String {
    format!(
        r#"
start = "debate"

[[steps]]
id = "debate"
decides = "model"
effect = "read"
debate = {{ agents = ["architect", "skeptic"], rounds = {rounds}, cap = "{cap}" }}
prompt = "{QUESTION}"
timeout = "{timeout}"
next = {{ daemon = "done", process = "stop", undecided = "stop" }}
"#
    )
}

/// A setup whose factory has the debaters `architect` and `skeptic`, priced, with `workflow`.
fn debating(name: &str, workflow: &str) -> Setup {
    let setup = Setup::with_workflow(name, workflow);
    write_debaters(&setup, SKEPTIC);
    setup
}

fn write_debaters(setup: &Setup, skeptic: &str) {
    for (agent, body) in [("architect", ARCHITECT), ("skeptic", skeptic)] {
        let file = setup.dir.join(format!("factory/agents/{agent}/agent.md"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, body).unwrap();
    }
    fs::write(setup.dir.join("factory/prices.toml"), PRICES).unwrap();
}

fn position(verdict: &str, argument: &str) -> serde_json::Value {
    json!({ "name": "position", "arguments": { "verdict": verdict, "argument": argument } })
}

fn conceding(verdict: &str, argument: &str, concedes: &str) -> serde_json::Value {
    json!({ "name": "position", "arguments": {
        "verdict": verdict, "argument": argument, "concedes": concedes } })
}

/// Waits until task 1 is in `state`, and returns its row.
pub(super) fn until_state(setup: &Setup, state: &str) -> Vec<String> {
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == state)
    })[0]
        .clone()
}

/// The agent of each debate turn, in order, from the trace.
fn speakers(setup: &Setup) -> Vec<String> {
    traced(setup, "factory.debate.turn_ended")
        .iter()
        .map(|turn| {
            turn["data"]["agent"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// Task 1's stored results, from `priory task 1 --json`.
fn results(setup: &Setup) -> Vec<serde_json::Value> {
    task_json(setup, "1")["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn two_openings_with_the_same_verdict_end_the_step_with_it_after_two_runs() {
    let mut setup = debating("debate-agree", &debate_workflow(2, "$5.00", "15m"));
    setup.calls(&[
        position(
            "process",
            "A crash in the cache must not take the daemon down.",
        ),
        position("process", "One hop on a socket is cheap."),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "dropped");
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!(speakers(&setup), ["architect", "skeptic"]);
    let results = results(&setup);
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["outcome"], "process");
    let summary = results[0]["summary"].as_str().unwrap();
    assert!(
        summary.starts_with("Both agents chose process at the opening."),
        "{summary}"
    );
    assert!(
        summary.contains("skeptic: One hop on a socket is cheap."),
        "{summary}"
    );
    let ended = traced(&setup, "factory.step.ended");
    let debate = &ended[0]["data"]["debate"];
    assert_eq!(debate["rounds"], 0, "{debate}");
    assert_eq!(debate["openings_agreed"], true);
    assert_eq!(debate["changed"], json!([]));
    assert_eq!(ended[0]["data"]["decider"], "model");
}

#[test]
fn a_second_agent_that_changes_its_verdict_in_round_one_ends_the_step_with_the_shared_one() {
    let mut setup = debating("debate-converge", &debate_workflow(2, "$5.00", "15m"));
    setup.calls(&[
        position("process", "Isolate the crash."),
        position("daemon", "No new service file."),
        position(
            "process",
            "A service file is one line; a crash takes every task down.",
        ),
        conceding(
            "process",
            "The crash argument holds.",
            "A crash in the daemon stops every task.",
        ),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "dropped");
    let results = results(&setup);
    assert_eq!(results[0]["outcome"], "process");
    assert!(
        results[0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("Both agents chose process after 1 round."),
        "{results:?}"
    );
    let evidence = results[0]["evidence"].as_str().unwrap();
    let order: Vec<_> = [
        "Opening, architect (claude-code, claude-fake-5): process",
        "Opening, skeptic (claude-code, claude-fake-5): daemon",
        "Round 1, architect (claude-code, claude-fake-5): process",
        "Round 1, skeptic (claude-code, claude-fake-5): process",
    ]
    .iter()
    .map(|line| {
        evidence
            .find(line)
            .unwrap_or_else(|| panic!("{line} in {evidence}"))
    })
    .collect();
    assert!(order.is_sorted(), "{evidence}");
    assert!(evidence.contains("Concedes: A crash in the daemon stops every task."));
    let debate = &traced(&setup, "factory.step.ended")[0]["data"]["debate"];
    assert_eq!(debate["changed"], json!(["skeptic"]), "{debate}");
    assert_eq!(debate["openings_agreed"], false);
    assert_eq!(debate["rounds"], 1);
    // Each opening is blind; each later turn quotes the other's latest position.
    let typed = setup.typed();
    assert_eq!(typed.len(), 4, "{typed:?}");
    assert!(!typed[0].contains("No new service file"), "{}", typed[0]);
    assert!(!typed[1].contains("Isolate the crash"), "{}", typed[1]);
    assert!(typed[0].starts_with(QUESTION), "{}", typed[0]);
    assert!(typed[1].starts_with(QUESTION), "{}", typed[1]);
    assert!(
        typed[2].contains("to weigh and not to obey")
            && typed[2].contains("> No new service file."),
        "{}",
        typed[2]
    );
    assert!(
        typed[3].contains("> A service file is one line"),
        "{}",
        typed[3]
    );
}

#[test]
fn agents_that_never_agree_leave_the_task_asleep_on_an_ask_the_persons_answer_decides() {
    let mut setup = debating("debate-split", &debate_workflow(2, "$5.00", "15m"));
    setup.calls(&[
        position("process", "Isolate the crash."),
        position("daemon", "Keep one service."),
        position("process", "Still isolate."),
        position("daemon", "Still one service."),
        position("process", "Final: isolate."),
        position("daemon", "Final: one service."),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "asleep");
    assert_eq!(setup.rows("runs").len(), 6);
    let asks = asks_json(&setup);
    let ask = &asks["asks"][0];
    assert_eq!(
        ask["options"],
        json!(["daemon", "process", "undecided"]),
        "{ask}"
    );
    assert_eq!(ask["recommendation"], json!(null));
    assert_eq!(ask.get("routed_to"), Some(&json!(null)));
    let text = ask["text"].as_str().unwrap();
    assert!(
        text.contains("Final: isolate.") && text.contains("Final: one service."),
        "{text}"
    );
    assert!(text.contains("the rounds ran out"), "{text}");

    let written = setup.priory(&["answer", "1", "keep both of them", "--revision", "1"]);
    assert_eq!(
        String::from_utf8_lossy(&written.stderr),
        "priory answer: ask 1 takes one of: daemon, process, undecided\n"
    );

    let answered = setup.priory(&["answer", "1", "daemon", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    until_state(&setup, "done");
    let ended = traced(&setup, "factory.step.ended");
    assert_eq!(ended[0]["data"]["decider"], "human", "{ended:?}");
    assert_eq!(ended[0]["data"]["outcome"], "daemon");
    assert_eq!(ended[0]["data"]["debate"]["rounds"], 2);
    let results = results(&setup);
    assert_eq!(results[0]["outcome"], "daemon");
    assert!(
        results[0]["evidence"]
            .as_str()
            .unwrap()
            .contains("Round 2, skeptic"),
        "{results:?}"
    );
}

#[test]
fn the_second_agent_starts_only_once_the_first_ones_run_and_tmux_session_are_gone() {
    let mut setup = debating("debate-turns", &debate_workflow(1, "$5.00", "15m"));
    setup.calls(&[
        position("process", "Isolate."),
        position("process", "Agreed."),
    ]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/submitted");
    assert_eq!(
        setup.rows("runs").len(),
        1,
        "no second run while the first speaks"
    );
    setup.hold("hold-submit");

    setup.release("hold-call");

    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_ne!(
        runs[0][7], "-",
        "the first run ended before the second: {runs:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let panes = setup.tmux(&["list-panes", "-a", "-F", "#{@priory}"]);
        let tags: Vec<String> = String::from_utf8_lossy(&panes.stdout)
            .lines()
            .map(str::to_owned)
            .collect();
        if tags == [runs[1][4].clone()] {
            break;
        }
        assert!(
            !tags.contains(&runs[0][4]),
            "the first agent's pane lives on: {tags:?}"
        );
        assert!(Instant::now() < deadline, "{tags:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.release("hold-submit");
    until_state(&setup, "dropped");
}

#[test]
fn result_ask_and_wait_are_refused_in_a_debate_turn_and_a_bad_position_with_its_reason() {
    let mut setup = debating("debate-refused", &debate_workflow(1, "$5.00", "15m"));
    let long = "x".repeat(4001);
    setup.calls(&[
        json!([
            result("process"),
            ask(),
            { "name": "wait", "arguments": { "on": [{ "type": "time", "until": "1 hour" }] } },
            position("maybe", "Unsure."),
            position("process", &long),
            position("process", "Isolate."),
        ]),
        position("process", "Agreed."),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "dropped");
    let replies: Vec<_> = setup.replies().iter().map(reply).collect();
    for refused in &replies[..3] {
        assert_eq!(
            refused,
            &("a debate turn ends with position".to_owned(), true)
        );
    }
    assert_eq!(
        replies[3],
        (
            "verdict \"maybe\" is not one of step debate's outcomes: daemon, process, undecided"
                .to_owned(),
            true
        )
    );
    assert_eq!(
        replies[4],
        (
            "argument is 4001 characters; a position's argument is at most 4000".to_owned(),
            true
        )
    );
    assert!(!replies[5].1, "{replies:?}");
}

#[test]
fn position_outside_a_debate_turn_is_an_unknown_tool() {
    let mut setup = Setup::new("debate-unknown");
    setup.calls(&[json!([position("passed", "Done."), result("passed")])]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "done");
    assert_eq!(
        reply(&setup.replies()[0]),
        ("unknown tool position".to_owned(), true)
    );
}

#[test]
fn a_turn_with_no_position_gets_one_nudge_then_the_debate_goes_to_the_person() {
    let mut setup = debating("debate-silent", &debate_workflow(1, "$5.00", "15m"));
    setup.calls(&[serde_json::Value::Null, serde_json::Value::Null]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "asleep");
    let typed = setup.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
    assert!(
        typed[1].starts_with(
            "Your last turn ended in text only, so Priory cannot tell your position in the debate.\nEnd this turn by calling the position tool"
        ),
        "{}",
        typed[1]
    );
    let text = asks_json(&setup)["asks"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        text.contains("architect's turn ended with no position, even after a nudge"),
        "{text}"
    );
    assert_eq!(setup.rows("runs").len(), 1);
}

#[test]
fn a_turn_past_its_timeout_sends_the_debate_to_the_person() {
    let mut setup = debating("debate-timeout", &debate_workflow(1, "$5.00", "1s"));
    setup.calls(&[position("process", "Isolate.")]);
    setup.hold("hold-call");
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "asleep");
    let text = asks_json(&setup)["asks"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(text.contains("architect's turn run 1 timed out"), "{text}");
    let runs = setup.rows("runs");
    assert_eq!(runs[0][7], "timeout", "{runs:?}");
    setup.gone("priory-1");
}

#[test]
fn each_turn_is_priced_at_its_own_agents_model_and_a_codex_turns_tokens_come_from_its_session_file()
{
    let mut setup = debating("debate-codex", &debate_workflow(1, "$5.00", "15m"));
    write_debaters(&setup, CODEX_SKEPTIC);
    setup.calls(&[
        position("process", "Isolate."),
        position("process", "Agreed."),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "dropped");
    let turns = traced(&setup, "factory.debate.turn_ended");
    assert_eq!(turns.len(), 2, "{turns:?}");
    let claude = &turns[0]["data"];
    assert_eq!(
        [&claude["agent"], &claude["harness"], &claude["model"]],
        ["architect", "claude-code", "claude-fake-5"]
    );
    assert_eq!(
        claude["tokens"],
        json!({ "input": 101, "output": 11, "cache_read": 1001, "cache_write": 51 })
    );
    let codex = &turns[1]["data"];
    assert_eq!(
        [&codex["agent"], &codex["harness"], &codex["model"]],
        ["skeptic", "codex", "gpt-fake-5"]
    );
    // Turn 2's last usage: 302 input, 100 of them cached, and 22 output.
    assert_eq!(
        codex["tokens"],
        json!({ "input": 202, "output": 22, "cache_read": 100, "cache_write": 0 })
    );
    let ended = &traced(&setup, "factory.step.ended")[0]["data"];
    assert_eq!(ended.get("tokens"), None, "{ended}");
    assert_eq!(ended.get("model"), None, "{ended}");
    let claude_usd = (101.0 * 1.0 + 11.0 * 2.0 + 1001.0 * 0.5 + 51.0 * 1.0) / 1e6;
    let codex_usd = (202.0 * 10.0 + 22.0 * 20.0 + 100.0 * 5.0) / 1e6;
    let cost = ended["debate"]["cost_usd"].as_f64().unwrap();
    assert!(
        (cost - (claude_usd + codex_usd)).abs() < 1e-9,
        "{cost} != {}",
        claude_usd + codex_usd
    );
}

#[test]
fn a_cap_the_first_round_passes_starts_no_second_round_and_the_ask_names_the_cost() {
    let mut setup = debating("debate-cap", &debate_workflow(2, "$0.02", "15m"));
    fs::write(
        setup.dir.join("factory/prices.toml"),
        "[claude-fake-5]\ninput = 10.0\noutput = 20.0\ncache_read = 5.0\ncache_write = 10.0\n",
    )
    .unwrap();
    setup.calls(&[
        position("process", "Isolate."),
        position("daemon", "One service."),
        position("process", "Still isolate."),
        position("daemon", "Still one service."),
        position("process", "Never asked."),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "asleep");
    // Each turn costs about $0.0067, so the cap stops the debate after round 1's first turn.
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 3, "no second round: {runs:?}");
    let text = asks_json(&setup)["asks"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        text.contains("the cap was reached: the debate cost $0.02 of its $0.02 cap"),
        "{text}"
    );
}

#[test]
fn a_debate_that_did_not_converge_on_a_hands_off_task_asks_the_person_not_the_lead() {
    let mut setup = hands_off("debate-hands-off", &debate_workflow(1, "$5.00", "15m"));
    write_debaters(&setup, SKEPTIC);
    setup.calls(&[
        position("process", "Isolate."),
        position("daemon", "One service."),
        position("process", "Still isolate."),
        position("daemon", "Still one service."),
    ]);
    setup.publish(&requested(42));

    setup.start();

    until_state(&setup, "asleep");
    let asks = asks_json(&setup);
    assert_eq!(
        asks["asks"][0].get("routed_to"),
        Some(&json!(null)),
        "{asks}"
    );
    assert_eq!(setup.rows("tasks").len(), 1, "no lead task");
}

#[test]
fn a_restart_between_two_turns_resumes_at_the_next_speaker_and_asks_no_position_twice() {
    let mut setup = debating("debate-restart", &debate_workflow(1, "$5.00", "15m"));
    setup.calls(&[
        position("process", "Isolate."),
        position("daemon", "One service."),
        position("process", "Still isolate."),
        position("process", "Agreed."),
    ]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    let deadline = Instant::now() + Duration::from_secs(60);
    while setup.replies().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the first position never arrived"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.crash();

    setup.release("hold-stop");
    setup.start();

    until_state(&setup, "dropped");
    assert_eq!(
        speakers(&setup),
        ["architect", "skeptic", "architect", "skeptic"]
    );
    assert_eq!(setup.typed().len(), 4, "{:?}", setup.typed());
    let evidence = results(&setup)[0]["evidence"].as_str().unwrap().to_owned();
    assert_eq!(
        evidence.matches("Opening, architect").count(),
        1,
        "{evidence}"
    );
}

const IRREVERSIBLE_AFTER: &str = r#"
start = "debate"

[[steps]]
id = "debate"
decides = "model"
effect = "read"
debate = { agents = ["architect", "skeptic"], rounds = 1, cap = "$5.00" }
prompt = "Open the pull request now?"
timeout = "15m"
next = { open = "open-pr", wait = "stop" }

[[steps]]
id = "approve"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

#[test]
fn a_debates_verdict_toward_an_irreversible_step_still_needs_its_approving_human_step() {
    let setup = debating("debate-irreversible", IRREVERSIBLE_AFTER);
    let checked = setup.priory(&["check", setup.dir.join("factory").to_str().unwrap()]);
    assert!(!checked.status.success());
    assert!(
        String::from_utf8_lossy(&checked.stderr).contains(
            "irreversible, and a path from start reaches it with no human step that approves it"
        ),
        "{checked:?}"
    );

    let workflow =
        IRREVERSIBLE_AFTER.replace("open = \"open-pr\", wait", "open = \"approve\", wait");
    let mut setup = debating("debate-approve", &workflow);
    setup.calls(&[position("open", "Ready."), position("open", "Ready too.")]);
    setup.publish(&requested(42));

    setup.start();

    let task = until_state(&setup, "asleep");
    assert_eq!(task[3], "approve");
    let asks = asks_json(&setup);
    assert_eq!(asks["asks"][0]["step"], "approve", "{asks}");
}

/// A debate whose `daemon` verdict leads to an `implement` step the implementer runs.
const DEBATE_THEN_IMPLEMENT: &str = r#"
start = "debate"

[[steps]]
id = "debate"
decides = "model"
effect = "read"
debate = { agents = ["architect", "skeptic"], rounds = 1, cap = "$5.00" }
prompt = "Should the cache live in the daemon or in its own process?"
timeout = "15m"
next = { daemon = "implement", process = "stop" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
effect = "reversible"
prompt = "Implement the verdict."
next = { passed = "done" }
"#;

#[test]
fn an_event_and_a_say_during_a_debate_turn_reach_the_step_after_the_debate_and_no_debater() {
    let mut setup = debating("debate-told", DEBATE_THEN_IMPLEMENT);
    setup.calls(&[
        position("daemon", "No new service file."),
        position("daemon", "Agreed."),
        json!({ "name": "result", "arguments": { "outcome": "passed", "summary": "Done.",
                                                 "feedback": { "u": "trivial" } } }),
    ]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/submitted");

    setup.publish(&issue_commented(901));
    setup.until("inbox", |rows| rows.len() == 2);
    let said = setup.priory(&["say", "1", "Keep the old API working."]);
    assert!(said.status.success(), "{said:?}");
    assert!(
        String::from_utf8_lossy(&said.stdout).contains("after the debate"),
        "{said:?}"
    );
    setup.release("hold-call");

    until_state(&setup, "done");
    let typed = setup.typed();
    assert_eq!(typed.len(), 3, "{typed:?}");
    for turn in &typed[..2] {
        assert!(!turn.contains("Also the empty case."), "{turn}");
        assert!(!turn.contains("Keep the old API working."), "{turn}");
    }
    assert!(
        typed[2].starts_with("Implement the verdict."),
        "{}",
        typed[2]
    );
    assert!(typed[2].contains("Also the empty case."), "{}", typed[2]);
    assert!(
        typed[2].contains("Keep the old API working."),
        "{}",
        typed[2]
    );
}

#[test]
fn a_debate_reads_no_note_and_no_written_answer() {
    let workflow = debate_workflow(2, "$5.00", "15m")
        .replace("daemon = \"done\"", "daemon = \"check\"")
        + r#"
[[steps]]
id = "check"
decides = "human"
effect = "none"
options = ["go", "halt"]
next = { go = "debate", halt = "stop" }
"#;
    let mut setup = debating("debate-written", &workflow);
    setup.calls(&[position("daemon", "a"), position("daemon", "b")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    until_state(&setup, "asleep");

    assert_eq!(
        refused_answer(&setup, "weigh latency"),
        "priory answer: ask 1 takes one of: go, halt; no earlier model step reads a written answer\n"
    );
    assert_eq!(
        refused_answer(&setup, "go: weigh latency"),
        "priory answer: ask 1 answered go leads to debate, which reads no note: answer go alone\n"
    );
    assert_eq!(asks_json(&setup)["asks"][0]["state"], "open");
    assert_eq!(with(&setup.rows("outbox"), 3, "queued"), 0);
}

#[test]
fn a_hold_during_a_debate_turn_holds_the_task_before_the_next_turn() {
    let mut setup = debating("debate-hold", &debate_workflow(2, "$5.00", "15m"));
    setup.calls(&[
        position("process", "Isolate."),
        position("process", "Agreed."),
    ]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/submitted");
    let refused = setup.priory(&["hold", "1", "--reason", "room", "--now"]);
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "priory hold: task 1 is in a debate turn: leave out --now to hold it after its turn\n"
    );
    person(&setup, "hold", &["1", "--reason", "room"]);

    setup.release("hold-stop");

    until_state(&setup, "held");
    assert_eq!(speakers(&setup), ["architect"]);
    person(&setup, "release", &["1"]);
    until_state(&setup, "dropped");
    assert_eq!(speakers(&setup), ["architect", "skeptic"]);
    assert_eq!(results(&setup)[0]["outcome"], "process");
}
