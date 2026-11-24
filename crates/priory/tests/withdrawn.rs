//! Why and when an ask or wait was withdrawn, kept in `withdrawn_reason` and `withdrawn_at`, and
//! backfilled from the sentinel values `withdrawn` held before version 58 and the withdrawal log.

#[allow(dead_code)]
mod harness;

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Output;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("withdrawn")
        .join(name);
    priory_runtime::runtime::remove_sealed(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn priory(dir: &Path, args: &[&str]) -> Output {
    let output = harness::priory()
        .args(args)
        .env("PRIORY_DB", dir.join("priory.db"))
        .env("PRIORY_HOME", dir.join("home"))
        .env("HERDR_BUS_DB", dir.join("bus.db"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    output
}

/// Sets the state file back to version 57: `withdrawn` holds the reason as a sentinel.
fn as_version_57(db: &rusqlite::Connection) {
    let added: bool = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('asks') WHERE name = 'withdrawn_reason'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if added {
        db.execute_batch(
            "DROP TRIGGER ask_withdrawn;
             ALTER TABLE asks DROP COLUMN withdrawn_reason;
             ALTER TABLE waits DROP COLUMN withdrawn_reason;
             ALTER TABLE ask_withdrawals DROP COLUMN reason;
             CREATE TRIGGER ask_withdrawn AFTER UPDATE OF withdrawn ON asks
               WHEN NEW.withdrawn IS NOT NULL AND NEW.withdrawn IS NOT OLD.withdrawn BEGIN
               INSERT INTO ask_withdrawals (ask, revision, withdrawn)
                 VALUES (NEW.id, NEW.revision, NEW.withdrawn);
             END;",
        )
        .unwrap();
    }
    db.execute_batch(
        "DROP VIEW task_text; CREATE VIEW task_text AS SELECT tasks.id AS task,
           coalesce(json_extract(inbox.data, '$.title'), '') AS title,
           coalesce(json_extract(inbox.data, '$.text'), json_extract(inbox.data, '$.body'),
             json_extract(inbox.data, '$.description'), json_extract(inbox.data, '$.prompt'), '') AS request,
           coalesce(tasks.tldr, '') AS tldr,
           coalesce((SELECT group_concat(summary || ' ' || coalesce(evidence, ''), ' ') FROM results
             WHERE results.task = tasks.id), '') AS results
           FROM tasks LEFT JOIN inbox ON inbox.id = tasks.source; ALTER TABLE tasks DROP COLUMN pre_title; ALTER TABLE tasks DROP COLUMN pre_text;
         ALTER TABLE tasks DROP COLUMN pre_context; ALTER TABLE tasks DROP COLUMN pre_script; DROP TABLE pending_requests;
         ALTER TABLE memory_candidates DROP COLUMN step; ALTER TABLE gates DROP COLUMN lessons; ALTER TABLE gates DROP COLUMN ask; ALTER TABLE gates DROP COLUMN judges;
         ALTER TABLE runs DROP COLUMN kit_hash; ALTER TABLE deliveries DROP COLUMN cleared; ALTER TABLE snoozes DROP COLUMN ended;
         DROP TRIGGER ask_withdrawn_at; DROP TRIGGER wait_withdrawn_at;
         ALTER TABLE asks DROP COLUMN withdrawn_at; ALTER TABLE waits DROP COLUMN withdrawn_at;
         DROP TRIGGER ask_returns_from_its_lead; DROP TRIGGER push_an_intake_failure;
         DROP TRIGGER push_an_uncertain_prompt; DROP TRIGGER push_an_uncertain_command;
         ALTER TABLE waits DROP COLUMN green; ALTER TABLE tasks DROP COLUMN files;
         ALTER TABLE tasks DROP COLUMN files_run; ALTER TABLE tasks DROP COLUMN merge_risk;
         ALTER TABLE tasks DROP COLUMN plan_goal; ALTER TABLE tasks DROP COLUMN plan_testing; ALTER TABLE tasks DROP COLUMN plan_brief; ALTER TABLE tasks DROP COLUMN verification;
         ALTER TABLE results DROP COLUMN merge_risk; ALTER TABLE subscriptions DROP COLUMN chosen;
         ALTER TABLE approvals DROP COLUMN generation;
         ALTER TABLE tasks DROP COLUMN base_task; ALTER TABLE tasks DROP COLUMN batch; ALTER TABLE tasks DROP COLUMN batch_entry; DROP TABLE idle_compactions; ALTER TABLE shared_sessions DROP COLUMN definition;
         DROP INDEX todos_open_connector; ALTER TABLE todos DROP COLUMN connector; ALTER TABLE waits DROP COLUMN after; ALTER TABLE tasks DROP COLUMN parent_pull_request; ALTER TABLE tasks DROP COLUMN parent_branch; ALTER TABLE tasks DROP COLUMN parent_head; DROP TABLE memories_repo; ALTER TABLE runs DROP COLUMN memories_commit; DROP TABLE kit; ALTER TABLE runs DROP COLUMN debate; ALTER TABLE runs DROP COLUMN kit_commit; ALTER TABLE runs DROP COLUMN preempted_for; ALTER TABLE tasks DROP COLUMN intent; ALTER TABLE tasks DROP COLUMN noop; ALTER TABLE runs DROP COLUMN marked; ALTER TABLE tasks DROP COLUMN preempted; ALTER TABLE tasks DROP COLUMN protected; ALTER TABLE tasks DROP COLUMN base_sha; ALTER TABLE tasks DROP COLUMN added_by; ALTER TABLE claims DROP COLUMN generation; ALTER TABLE claims DROP COLUMN published_generation; ALTER TABLE asks DROP COLUMN confidence; ALTER TABLE asks DROP COLUMN rationale; ALTER TABLE tasks DROP COLUMN auto_answer; ALTER TABLE runs DROP COLUMN session_started; ALTER TABLE runs DROP COLUMN harness; ALTER TABLE runs DROP COLUMN resumes; ALTER TABLE runs DROP COLUMN sandbox; ALTER TABLE runs DROP COLUMN direnv; ALTER TABLE tasks DROP COLUMN issue_order; ALTER TABLE tasks DROP COLUMN slot; DROP TABLE lapses; DROP TRIGGER push_a_landed_wrong; DROP TRIGGER push_a_stall; ALTER TABLE push_subscriptions DROP COLUMN kinds; DROP TABLE messages; DROP TABLE pairs; DROP TABLE news; PRAGMA user_version = 57;",
    )
    .unwrap();
}

#[test]
fn an_upgrade_names_the_reason_each_sentinel_meant() {
    let dir = scratch("upgrade");
    priory(&dir, &["tasks"]);
    let db = rusqlite::Connection::open(dir.join("priory.db")).unwrap();
    as_version_57(&db);
    db.execute_batch(
        "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source)
           VALUES (1, 'issue-1', 'implement', 'work', 'asleep', 'lane', 'cli', 'cli-1');
         INSERT INTO runs (id, task, step, tmux, session) VALUES (1, 1, 'work', 't', 's');
         INSERT INTO asks (id, task, run, step, kind, question, options, created) VALUES
           (1, 1, 1, 'work', 'agent', 'A?', '[]', '2026-11-16T09:00:00Z'),
           (2, 1, 1, 'work', 'agent', 'B?', '[]', '2026-11-16T09:00:00Z'),
           (3, 1, 1, 'work', 'agent', 'C?', '[]', '2026-11-16T09:00:00Z'),
           (4, 1, 1, 'work', 'agent', 'D?', '[]', '2026-11-16T09:00:00Z');
         UPDATE asks SET withdrawn = CASE id WHEN 1 THEN -1 WHEN 2 THEN -2 WHEN 3 THEN 0 ELSE 2 END;
         INSERT INTO waits (task, run, kind, created, withdrawn) VALUES
           (1, 1, 'checks', '2026-11-16T09:00:00Z', -2),
           (1, 1, 'checks', '2026-11-16T09:00:00Z', 0),
           (1, 1, 'checks', '2026-11-16T09:00:00Z', 2);",
    )
    .unwrap();
    drop(db);

    let task: Value =
        serde_json::from_slice(&priory(&dir, &["task", "1", "--json"]).stdout).unwrap();

    let reasons = |list: &str| -> Vec<Value> {
        task[list]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["withdrawn_reason"].clone())
            .collect()
    };
    assert_eq!(
        reasons("asks"),
        ["dismissed", "sibling-fired", "moved-on", "revived"]
    );
    // No wait of its run fired and its run's ask was dismissed, so 64 names the dismiss.
    assert_eq!(reasons("waits"), ["dismissed", "moved-on", "revived"]);
    let db = rusqlite::Connection::open(dir.join("priory.db")).unwrap();
    let logged: Vec<String> = db
        .prepare("SELECT reason FROM ask_withdrawals ORDER BY ask")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        logged,
        ["dismissed", "sibling-fired", "moved-on", "revived"]
    );
    let logged_at: Vec<Value> = db
        .prepare("SELECT created FROM ask_withdrawals ORDER BY ask")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|at| Value::from(at.unwrap()))
        .collect();
    let at = |list: &str| -> Vec<Value> {
        task[list]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["withdrawn_at"].clone())
            .collect()
    };
    // The log holds when each ask was withdrawn; nothing held it for a wait.
    assert_eq!(at("asks"), logged_at);
    assert_eq!(at("waits"), [Value::Null, Value::Null, Value::Null]);
}

#[test]
fn an_upgrade_names_the_answer_dismiss_or_stack_ask_that_withdrew_an_old_wait() {
    let dir = scratch("corrected");
    priory(&dir, &["tasks"]);
    let db = rusqlite::Connection::open(dir.join("priory.db")).unwrap();
    as_version_57(&db);
    db.execute_batch(
        "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source)
           VALUES (1, 'issue-1', 'implement', 'work', 'asleep', 'lane', 'cli', 'cli-1');
         INSERT INTO runs (id, task, step, tmux, session, ended) VALUES
           (1, 1, 'work', 't', 's1', '2026-11-16T09:30:00Z'),
           (2, 1, 'work', 't', 's2', '2026-11-16T09:30:00Z'),
           (3, 1, 'work', 't', 's3', '2026-11-16T09:30:00Z'),
           (4, 1, 'work', 't', 's4', NULL);
         INSERT INTO asks (id, task, run, step, kind, question, options, created, answer, withdrawn) VALUES
           (1, 1, 1, 'work', 'agent', 'A?', '[]', '2026-11-16T09:00:00Z', 'yes', NULL),
           (2, 1, 2, 'work', 'agent', 'B?', '[]', '2026-11-16T09:00:00Z', NULL, -1);
         INSERT INTO waits (task, run, kind, created, fired, daemon, withdrawn) VALUES
           (1, 1, 'checks', '2026-11-16T09:00:00Z', NULL, 0, -2),
           (1, 2, 'checks', '2026-11-16T09:00:00Z', NULL, 0, -2),
           (1, 3, 'checks', '2026-11-16T09:00:00Z', NULL, 0, -2),
           (1, 3, 'time', '2026-11-16T09:00:00Z', '2026-11-16T10:00:00Z', 0, NULL),
           (1, 4, 'checks', '2026-11-16T09:00:00Z', NULL, 1, -2);",
    )
    .unwrap();
    drop(db);

    let task: Value =
        serde_json::from_slice(&priory(&dir, &["task", "1", "--json"]).stdout).unwrap();

    let reasons: Vec<Value> = task["waits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["withdrawn_reason"].clone())
        .collect();
    assert_eq!(
        reasons,
        [
            Value::from("answered-elsewhere"),
            Value::from("dismissed"),
            Value::from("sibling-fired"),
            Value::Null,
            Value::from("asked"),
        ]
    );
}
