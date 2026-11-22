//! The review limit: new starts wait while a lane or a batch has too many pull requests waiting for
//! a person (docs/daemon/runs.md#review-limit).

use super::*;

/// A factory whose lanes and batches each start no new task while one pull request waits.
const LIMIT_ONE: &str = "name = \"demo\"\n\n[queue]\nreview_limit = 1\n";

/// Why a new task of lane none/misc waits while task 1's pull request waits for a person.
const LANE_WAITS: &str = "waits: review limit: lane none/misc has 1 pull request waiting for a person, at the limit of 1";

impl Setup {
    /// A factory with `LIMIT_ONE` whose task 1 is done with pull request 7 open, and whose fake
    /// `claude` then makes the calls `next`.
    fn reviewing(name: &str, next: &[serde_json::Value]) -> Setup {
        let mut setup = Setup::new(name);
        fs::write(setup.dir.join("factory/factory.toml"), LIMIT_ONE).unwrap();
        let calls: Vec<_> = std::iter::once(result("passed"))
            .chain(next.iter().cloned())
            .collect();
        setup.calls(&calls);
        setup.publish(&requested(42));
        setup.start();
        setup.until("tasks", |rows| {
            rows.first().is_some_and(|row| row[4] == "done")
        });
        setup.write_state(
            "INSERT INTO pull_requests (task, resource, branch, base)
             VALUES (1, 'github:owner/repo#7', 'priory/1', 'main')",
        );
        setup
    }

    /// Runs `statement` on the state file.
    fn write_state(&self, statement: &str) {
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        db.busy_timeout(Duration::from_secs(10)).unwrap();
        db.execute_batch(statement).unwrap();
    }
}

#[test]
fn a_lane_at_its_review_limit_holds_new_starts_until_a_pull_request_merges() {
    let setup = Setup::reviewing("review-lane", &[result("passed")]);

    setup.publish(&requested(43));

    let tasks = setup.until("tasks", |rows| {
        rows.get(1).is_some_and(|row| row[7] == LANE_WAITS)
    });
    assert_eq!(tasks[1][4], "queued", "{tasks:?}");
    assert_eq!(setup.rows("runs").len(), 1);
    let desk = setup.priory(&["desk"]);
    let desk = String::from_utf8(desk.stdout).unwrap();
    assert_eq!(
        desk.lines().next(),
        Some(
            "review limit reached: lane none/misc has 1 pull request waiting for a person, at the limit of 1; its new runs wait"
        ),
        "{desk}"
    );
    let json = setup.priory(&["desk", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(
        json["reviews"],
        json!([
            "review limit reached: lane none/misc has 1 pull request waiting for a person, at the limit of 1; its new runs wait"
        ])
    );

    setup.publish(&closed_7(true, 400));

    setup.until("runs", |rows| rows.len() == 2);
    let desk = setup.priory(&["desk"]);
    let desk = String::from_utf8(desk.stdout).unwrap();
    assert!(!desk.contains("review limit"), "{desk}");
}

#[test]
fn a_woken_task_runs_again_while_its_lane_is_at_its_review_limit() {
    let mut setup = Setup::new("review-woken");
    fs::write(setup.dir.join("factory/factory.toml"), LIMIT_ONE).unwrap();
    setup.calls(&[result("passed"), ask(), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    setup.publish(&requested(43));
    setup.until("tasks", |rows| {
        rows.get(1).is_some_and(|row| row[4] == "asleep")
    });
    setup.write_state(
        "INSERT INTO pull_requests (task, resource, branch, base)
         VALUES (1, 'github:owner/repo#7', 'priory/1', 'main')",
    );
    setup.publish(&requested(44));
    setup.until("tasks", |rows| {
        rows.get(2).is_some_and(|row| row[7] == LANE_WAITS)
    });

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    let tasks = setup.until("tasks", |rows| {
        rows.get(1).is_some_and(|row| row[4] == "done")
    });
    assert_eq!(tasks[2][4], "queued", "{tasks:?}");
    assert_eq!(tasks[2][7], LANE_WAITS);
    assert_eq!(setup.rows("runs").len(), 3, "task 3 never started");
}

#[test]
fn a_batch_at_its_review_limit_holds_its_new_entries_and_no_other_task() {
    let setup = Setup::reviewing("review-batch", &[result("passed")]);
    // Task 1 is an entry of batch 9 in another lane, so only the batch counts its pull request.
    setup.write_state("UPDATE tasks SET lane = 'core/old', batch = 9 WHERE id = 1");
    assert!(setup.priory(&["pause"]).status.success());
    setup.publish(&requested(43));
    setup.until("tasks", |rows| {
        rows.get(1)
            .is_some_and(|row| row[7] == "waits: priory is paused")
    });
    setup.write_state("UPDATE tasks SET batch = 9 WHERE id = 2");

    assert!(setup.priory(&["unpause"]).status.success());

    let waits =
        "waits: review limit: batch 9 has 1 pull request waiting for a person, at the limit of 1";
    let tasks = setup.until("tasks", |rows| rows[1][7] == waits);
    assert_eq!(tasks[1][4], "queued", "{tasks:?}");
    setup.publish(&requested(44));
    setup.until("runs", |rows| rows.len() == 2 && rows[1][1] == "3");
}

#[test]
fn a_task_of_the_first_band_starts_while_its_lane_is_at_its_review_limit() {
    let setup = Setup::reviewing("review-blocker", &[result("passed")]);

    setup.publish(&requested_in(43, Some("blocker")));

    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_eq!(runs[1][1], "2", "{runs:?}");
    let tasks = setup.rows("tasks");
    assert!(!tasks[1][7].contains("review limit"), "{tasks:?}");
}
