//! `priory hold` on a running task (docs/daemon/runs.md#holds).

use super::preempt::{SAME_AGENT, SHIP, URGENT_TYPE, on};
use super::*;

const HELD: &str = "held: room for task 2, until a person releases it";
const MARK: &str = "holds after this turn: room for task 2, until a person releases it";

/// Starts task 1 and keeps it inside its first turn.
fn mid_turn(setup: &mut Setup, request: Message) {
    setup.hold("hold-stop");
    setup.publish(&request);
    setup.go();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
}

fn hold(setup: &Setup, flags: &[&str]) -> String {
    let args = [&["1", "--reason", "room for task 2"], flags].concat();
    person(setup, "hold", &args)
}

fn refused(setup: &Setup, flags: &[&str]) -> String {
    let args = [&["hold", "1", "--reason", "room for task 2"], flags].concat();
    let output = setup.priory(&args);
    assert!(!output.status.success(), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

fn until_state(setup: &Setup, state: &str) {
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == state)
    });
}

/// Task 1's `held` rows on the desk: what each says.
fn on_desk(setup: &Setup) -> Vec<String> {
    let desk = setup.rows("desk");
    let held = desk.iter().filter(|row| row[1] == "held");
    held.map(|row| row[5].clone()).collect()
}

/// Releases task 1 and asserts that its next run resumes the first one's session in a worktree
/// that still has `draft.txt`, with a prompt starting with `first`.
fn resumes(setup: &Setup, first: &str) {
    assert_eq!(person(setup, "release", &["1"]), "task 1 is ready\n");
    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_eq!(
        [&runs[0][7], &runs[1][4]],
        ["held", &runs[0][4]],
        "{runs:?}"
    );
    setup.until("runs", |_| has(&launched(setup), ["--resume", &runs[0][4]]));
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(typed[1].starts_with(first), "{typed:?}");
    let draft = setup.dir.join("home/worktrees/owner/repo/1/draft.txt");
    assert_eq!(fs::read_to_string(draft).unwrap(), "uncommitted\n");
}

fn draft(setup: &Setup) {
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::write(worktree.join("draft.txt"), "uncommitted\n").unwrap();
}

#[test]
fn a_running_task_is_held_at_its_turn_end_and_resumes_its_session() {
    let mut setup = Setup::new("hold-after-turn");
    mid_turn(&mut setup, requested(42));
    draft(&setup);

    assert_eq!(hold(&setup, &[]), format!("task 1 {MARK}\n"));

    assert_eq!(held_as(&setup, "1"), ["running", MARK]);
    assert_eq!(on_desk(&setup), ["holds after this turn: room for task 2"]);
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(held_as(&setup, "1")[0], "running");
    // The mark is stored, so it outlives the daemon.
    setup.crash();
    setup.go();
    setup.release("hold-stop");
    until_state(&setup, "held");
    assert_eq!(held_as(&setup, "1")[1], HELD);
    assert_eq!(on_desk(&setup), [HELD]);
    setup.gone("priory-1");
    resumes(
        &setup,
        "A person held this task at the end of your turn: room for task 2, until a person releases it. The hold is released.\nYour session and worktree are as you left them. Continue.\n",
    );
}

#[test]
fn hold_now_interrupts_the_turn_and_the_task_resumes_its_session() {
    let mut setup = Setup::new("hold-now");
    mid_turn(&mut setup, requested(42));
    draft(&setup);

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));

    setup.gone("priory-1");
    assert_eq!(held_as(&setup, "1"), ["held", HELD]);
    setup.release("hold-stop");
    resumes(
        &setup,
        "A person interrupted your last turn to hold this task: room for task 2, until a person releases it. The hold is released. Your last call may not have finished.\nYour session and worktree are as you left them. Check the worktree, then continue.\n",
    );
}

#[test]
fn hold_now_on_a_marked_task_replaces_the_mark() {
    let mut setup = Setup::new("hold-mark-now");
    mid_turn(&mut setup, requested(42));
    hold(&setup, &["--until", "2000-01-01T00:00:00Z"]);

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));

    assert_eq!(on_desk(&setup), [HELD]);
}

#[test]
fn hold_now_holds_a_task_that_waits_in_the_queue() {
    let setup = queued_without_room("hold-now-queued");

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));
}

#[test]
fn a_hold_whose_condition_is_met_by_the_turn_end_lets_the_task_go_on() {
    let mut setup = Setup::new("hold-after-met");
    mid_turn(&mut setup, requested(42));
    hold(&setup, &["--until", "2000-01-01T00:00:00Z"]);

    setup.release("hold-stop");

    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_eq!(
        [&runs[0][7], &runs[1][4]],
        ["held", &runs[0][4]],
        "{runs:?}"
    );
}

#[test]
fn a_result_into_a_step_of_the_same_agent_is_held_before_that_step() {
    let mut setup = Setup::new("hold-carry");
    setup.workflow("two", SAME_AGENT);
    setup.calls(&[result("passed")]);
    mid_turn(&mut setup, on("two", requested(42)));
    hold(&setup, &[]);

    setup.release("hold-stop");

    until_state(&setup, "held");
    assert_eq!(setup.rows("tasks")[0][3], "check");
    assert_eq!(setup.rows("runs").len(), 1);
    let prompts = setup.prompts("1");
    assert!(
        prompts.last().unwrap().contains("Check the change."),
        "{prompts:?}"
    );
}

#[test]
fn a_marked_task_that_asks_sleeps_and_is_held_once_it_is_answered() {
    let mut setup = Setup::new("hold-ask");
    setup.calls(&[ask()]);
    mid_turn(&mut setup, requested(42));
    hold(&setup, &[]);

    setup.release("hold-stop");

    until_state(&setup, "asleep");
    assert_eq!(on_desk(&setup), ["holds once it is ready: room for task 2"]);
    person(&setup, "answer", &["1", "sqlite", "--revision", "1"]);
    until_state(&setup, "held");
    assert_eq!(setup.rows("runs").len(), 1);
    assert_eq!(person(&setup, "release", &["1"]), "task 1 is ready\n");
    setup.until("runs", |rows| rows.len() == 2);
}

#[test]
fn release_withdraws_the_mark_of_a_running_task() {
    let mut setup = Setup::new("hold-withdraw");
    mid_turn(&mut setup, requested(42));
    hold(&setup, &[]);

    assert_eq!(
        person(&setup, "release", &["1"]),
        "task 1 is no longer to be held\n"
    );

    assert_eq!(held_as(&setup, "1"), ["running", "-"]);
    assert!(on_desk(&setup).is_empty());
    setup.release("hold-stop");
    setup.until("outbox", |rows| rows.len() == 2);
    assert_eq!(setup.rows("runs")[0][7], "-");
}

#[test]
fn a_cancelled_marked_task_leaves_no_hold() {
    let mut setup = Setup::new("hold-cancel");
    mid_turn(&mut setup, requested(42));
    hold(&setup, &[]);

    person(&setup, "cancel", &["1"]);

    assert_eq!(held_as(&setup, "1")[0], "dropped");
    assert!(on_desk(&setup).is_empty());
}

#[test]
fn a_marked_task_that_ends_leaves_no_hold() {
    let mut setup = Setup::new("hold-ended");
    setup.calls(&[result("passed")]);
    mid_turn(&mut setup, requested(42));
    hold(&setup, &[]);

    setup.release("hold-stop");

    until_state(&setup, "done");
    setup.until("tasks", |_| {
        setup.sql("SELECT released_as FROM holds") == Some("its task ended".to_owned())
    });
}

#[test]
fn an_irreversible_step_refuses_hold_now_and_is_not_stopped_at_its_turn_end() {
    let mut setup = Setup::new("hold-irreversible");
    setup.workflow("ship", SHIP);
    mid_turn(&mut setup, on("ship", requested(42)));

    assert_eq!(
        refused(&setup, &["--now"]),
        "priory hold: task 1 is at irreversible step ship: leave out --now to hold it after its turn\n"
    );
    assert_eq!(held_as(&setup, "1"), ["running", "-"]);
    hold(&setup, &[]);
    setup.release("hold-stop");
    // Its turn ended with no tool, so it goes on with a nudge.
    setup.until("outbox", |rows| rows.len() == 2);
    assert_eq!(setup.rows("runs")[0][7], "-");
}

#[test]
fn a_shared_session_refuses_hold_now() {
    let mut setup = shared("hold-shared");
    mid_turn(&mut setup, requested(42));

    assert_eq!(
        refused(&setup, &["--now"]),
        "priory hold: task 1 runs in a shared session: leave out --now to hold it after its turn\n"
    );
}

#[test]
fn a_held_keyboard_keeps_the_run_until_it_is_let_go() {
    let mut setup = Setup::new("hold-keyboard");
    fs::write(setup.dir.join("factory/factory.toml"), URGENT_TYPE).unwrap();
    mid_turn(&mut setup, requested(42));
    setup.hold_keyboard("1");
    assert!(refused(&setup, &["--now"]).contains("keyboard is held"));
    hold(&setup, &[]);

    setup.release("hold-stop");
    setup.until("runs", |_| {
        setup.sql("SELECT idle FROM runs WHERE id = 1") == Some("1".to_owned())
    });
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(held_as(&setup, "1")[0], "running");

    // The run is idle, so the tick holds it.
    rusqlite::Connection::open(setup.dir.join("priory.db"))
        .unwrap()
        .execute("UPDATE keyboard_grants SET ended = 'now'", [])
        .unwrap();
    until_state(&setup, "held");
    assert_eq!(setup.rows("runs")[0][7], "held");
}

#[test]
fn an_asleep_task_cannot_be_held() {
    let mut setup = Setup::new("hold-asleep");
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.go();
    until_state(&setup, "asleep");

    assert_eq!(
        refused(&setup, &[]),
        "priory hold: task 1 is asleep: only a queued, ready or running task can be held\n"
    );
}

#[test]
fn hold_now_keeps_a_result_the_turn_already_sent() {
    let mut setup = Setup::new("hold-now-result");
    setup.workflow("two", SAME_AGENT);
    setup.calls(&[result("passed")]);
    mid_turn(&mut setup, on("two", requested(42)));
    setup.written("fake/tools.log");
    setup.until("runs", |_| {
        setup.sql("SELECT count(*) FROM results") == Some("1".to_owned())
    });

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));

    assert_eq!(setup.rows("tasks")[0][3], "check");
    let prompts = setup.prompts("1");
    assert!(
        prompts.last().unwrap().contains("Check the change."),
        "{prompts:?}"
    );
}

#[test]
fn hold_now_with_an_unconfirmed_prompt_leaves_it_on_the_desk_and_the_task_resumes() {
    let mut setup = Setup::new("hold-now-unconfirmed");
    fs::write(setup.dir.join("fake/trailer"), " Wait for approval.").unwrap();
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.go();
    setup.written("fake/submitted");
    draft(&setup);

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));

    setup.gone("priory-1");
    fs::remove_file(setup.dir.join("fake/trailer")).unwrap();
    setup.release("hold-stop");
    assert_eq!(person(&setup, "release", &["1"]), "task 1 is ready\n");
    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_eq!(runs[0][4], runs[1][4], "{runs:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert!(setup.typed()[1].starts_with("A person interrupted your last turn"));
}

#[test]
fn hold_now_before_the_session_starts_keeps_the_step_prompt() {
    let mut setup = Setup::new("hold-now-unready");
    fs::write(setup.dir.join("fake/silent"), "Starting").unwrap();
    setup.publish(&requested(42));
    setup.go();
    setup.until("runs", |rows| rows.len() == 1);

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));

    setup.gone("priory-1");
    fs::remove_file(setup.dir.join("fake/silent")).unwrap();
    person(&setup, "release", &["1"]);
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let typed = setup.typed();
    assert!(typed[0].contains("Implement the issue."), "{typed:?}");
}

#[test]
fn hold_now_before_a_budgeted_steps_first_tokens_holds_it() {
    let mut setup = Setup::with_workflow("hold-now-budget", &budgeted("5.00"));
    fs::write(setup.dir.join("factory/prices.toml"), DEAR).unwrap();
    fs::write(setup.dir.join("fake/silent"), "Starting").unwrap();
    setup.publish(&requested(42));
    setup.go();
    setup.until("runs", |rows| rows.len() == 1);

    assert_eq!(hold(&setup, &["--now"]), format!("task 1 {HELD}\n"));
}
