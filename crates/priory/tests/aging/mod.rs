//! `[queue] age`: a task that waits moves up one band in start order per `age` (docs/daemon/runs.md).

use super::*;

/// Task 2's reason while it waits for task 1's 4 GB.
const WAITS: &str = "waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";

const PAUSED: &str = "waits: priory is paused";

impl Setup {
    /// A paused daemon with `queue` as its `[queue]` table and room for one 4 GB run.
    fn aging(name: &str, queue: &str) -> Setup {
        let setup = Setup::new(name);
        fs::write(
            setup.dir.join("factory/factory.toml"),
            format!("name = \"demo\"\n\n[queue]\n{queue}"),
        )
        .unwrap();
        setup.memory(12);
        assert!(setup.priory(&["pause"]).status.success());
        setup
    }

    /// Makes task `id` have waited `minutes` in the queue.
    fn waited(&self, id: &str, minutes: u32) {
        rusqlite::Connection::open(self.dir.join("priory.db"))
            .unwrap()
            .execute(
                "UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2) WHERE id = ?1",
                rusqlite::params![id, format!("-{minutes} minutes")],
            )
            .unwrap();
    }

    /// `priory queue --json`'s waiting tasks, as `(id, band, aged)`.
    fn waiting(&self) -> Vec<(i64, String, Option<String>)> {
        let queue = self.priory(&["queue", "--json"]);
        assert!(queue.status.success(), "{queue:?}");
        let queue: serde_json::Value = serde_json::from_slice(&queue.stdout).unwrap();
        queue["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| {
                (
                    task["id"].as_i64().unwrap(),
                    task["band"].as_str().unwrap().to_owned(),
                    task["aged"].as_str().map(str::to_owned),
                )
            })
            .collect()
    }

    /// Waits until every task in `ids` is queued with the pause as its reason.
    fn until_paused(&self, ids: &[&str]) {
        self.until("tasks", |rows| {
            ids.iter().all(|id| {
                rows.iter()
                    .any(|row| row[0] == *id && row[4] == "queued" && row[7] == PAUSED)
            })
        });
    }
}

fn in_band(issue: u64, band: &str) -> Message {
    requested_in(issue, Some(band))
}

#[test]
fn a_task_that_waited_an_age_starts_ahead_of_a_younger_task_of_the_band_above() {
    let mut setup = Setup::aging("aging", "age = \"1h\"\n");
    setup.publish(&in_band(42, "later"));
    setup.publish(&in_band(43, "today"));
    setup.start();
    setup.until_paused(&["1", "2"]);

    setup.waited("1", 90);

    assert_eq!(
        setup.waiting(),
        [
            (1, "later".into(), Some("today".into())),
            (2, "today".into(), None)
        ]
    );
    let text = setup.priory(&["queue"]);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.starts_with(
            "position\tband_position\tid\tkey\tworkflow\tstep\tstate\tband\taged\tlane\treason\n1\t1\t1\t"
        ),
        "{text}"
    );
    assert!(text.contains("\tlater\ttoday\t"), "{text}");
    assert!(setup.priory(&["unpause"]).status.success());
    let runs = setup.until("runs", |rows| !rows.is_empty());
    assert_eq!(runs[0][1], "1", "{runs:?}");
    setup.until("tasks", |rows| {
        rows.iter().any(|row| row[0] == "2" && row[7] == WAITS)
    });
    assert_eq!(setup.rows("runs").len(), 1);

    // Without an age, the same wait leaves task 1 behind the band above.
    let mut off = Setup::aging("aging-off", "");
    off.publish(&in_band(42, "later"));
    off.publish(&in_band(43, "today"));
    off.start();
    off.until_paused(&["1", "2"]);
    off.waited("1", 600);
    assert_eq!(
        off.waiting(),
        [(2, "today".into(), None), (1, "later".into(), None)]
    );
    assert!(off.priory(&["unpause"]).status.success());
    let runs = off.until("runs", |rows| !rows.is_empty());
    assert_eq!(runs[0][1], "2", "{runs:?}");
}

#[test]
fn an_aged_task_starts_after_the_tasks_a_person_ordered_in_the_band_it_reaches() {
    let mut setup = Setup::aging("aging-order", "age = \"1h\"\n");
    setup.publish(&in_band(42, "today"));
    setup.publish(&in_band(43, "later"));
    setup.publish(&in_band(44, "today"));
    setup.start();
    setup.until_paused(&["1", "2", "3"]);
    assert!(setup.priory(&["move", "3", "--top"]).status.success());

    setup.waited("2", 90);

    // Task 3 is in a person's order of today. Task 2 joins today's unordered tasks, oldest first.
    assert_eq!(
        setup.waiting(),
        [
            (3, "today".into(), None),
            (1, "today".into(), None),
            (2, "later".into(), Some("today".into()))
        ]
    );
}

#[test]
fn age_stops_at_the_highest_band_not_in_preempt() {
    let mut setup = Setup::aging("aging-top", "preempt = [\"blocker\"]\nage = \"1h\"\n");
    setup.publish(&in_band(42, "later"));
    setup.start();
    setup.until_paused(&["1"]);

    setup.waited("1", 600);

    assert_eq!(setup.waiting(), [(1, "later".into(), Some("now".into()))]);
}

#[test]
fn a_task_aged_into_a_preempt_band_stops_no_run() {
    let mut setup = Setup::aging("aging-preempt", "preempt = [\"now\"]\nage = \"1h\"\n");
    assert!(setup.priory(&["unpause"]).status.success());
    setup.hold("hold-stop");
    setup.publish(&in_band(42, "later"));
    setup.start();
    setup.written("fake/submitted");
    assert!(setup.priory(&["pause"]).status.success());
    setup.publish(&in_band(43, "later"));
    setup.until_paused(&["2"]);

    setup.waited("2", 150);

    assert_eq!(setup.waiting(), [(2, "later".into(), Some("now".into()))]);
    assert!(setup.priory(&["unpause"]).status.success());
    let tasks = setup.until("tasks", |rows| {
        rows.iter()
            .any(|row| row[0] == "2" && row[7].starts_with("waits for room"))
    });
    let head = tasks.iter().find(|row| row[0] == "2").unwrap();
    assert_eq!(head[7], WAITS, "{tasks:?}");
    let running = tasks.iter().find(|row| row[0] == "1").unwrap();
    assert_eq!([&running[4], &running[7]], ["running", "-"], "{tasks:?}");
}
