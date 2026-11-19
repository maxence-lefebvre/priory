use crate::room::Short;
use priory_factory::factory::SWEEP;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use std::time::Duration;

/// The trigger a sweeper task names. No trigger file has a `/` in its name, so none collides.
pub const TRIGGER: &str = "priory/sweep";
/// The key of every sweeper task, so at most one is active.
const KEY: &str = "priory:sweep";

/// Opens, updates or ends the queue's stall: its head waits for room. A stall keeps its start while
/// the same resource is short, whichever task heads the queue.
pub(crate) fn record(db: &Connection, stall: Option<(i64, &Short)>) -> rusqlite::Result<()> {
    let ended = "UPDATE stalls SET ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                 WHERE ended IS NULL AND resource IS NOT ?1";
    let Some((task, short)) = stall else {
        db.execute(ended, [None::<&str>])?;
        return Ok(());
    };
    db.execute(ended, [short.resource])?;
    let updated = db.execute(
        "UPDATE stalls SET task = ?1, missing = ?2 WHERE ended IS NULL",
        params![task, short.missing],
    )?;
    if updated == 0 {
        db.execute(
            "INSERT INTO stalls (resource, missing, task) VALUES (?1, ?2, ?3)",
            params![short.resource, short.missing, task],
        )?;
    }
    Ok(())
}

/// The desk's first line while the queue is stalled.
pub fn banner(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT 'stalled since ' || since || ': short of ' || resource || ' by ' || missing
           || '; task ' || task || ' waits for room'
         FROM stalls WHERE ended IS NULL",
        [],
        |row| row.get(0),
    )
    .optional()
}

/// The open stall whose scripts have not started, marking them started.
pub(crate) fn unswept(db: &Connection) -> rusqlite::Result<Option<(i64, String)>> {
    db.query_row(
        "UPDATE stalls SET scripts = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE ended IS NULL AND scripts IS NULL RETURNING id, resource",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// Records a script of `stall` as started.
pub(crate) fn started(db: &Connection, stall: i64, script: &str) -> rusqlite::Result<i64> {
    db.execute(
        "INSERT INTO sweeps (stall, script) VALUES (?1, ?2)",
        params![stall, script],
    )?;
    Ok(db.last_insert_rowid())
}

/// Records the process group of a running script, so a restarted daemon can kill it.
pub(crate) fn spawned(db: &Connection, sweep: i64, pid: u32) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE sweeps SET pid = ?2 WHERE id = ?1",
        params![sweep, pid],
    )?;
    Ok(())
}

/// Records how a script ended. Once every script of its stall ended, the stall is swept.
pub(crate) fn ended(
    db: &Connection,
    sweep: i64,
    status: &str,
    output: &str,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE sweeps SET ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), status = ?2, output = ?3
         WHERE id = ?1",
        params![sweep, status, output],
    )?;
    swept(db)
}

/// Marks the open stall swept once its scripts started and none still runs.
pub(crate) fn swept(db: &Connection) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE stalls SET swept = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE ended IS NULL AND scripts IS NOT NULL AND swept IS NULL
           AND NOT EXISTS (SELECT 1 FROM sweeps WHERE stall = stalls.id AND ended IS NULL)",
        [],
    )?;
    Ok(())
}

/// Ends the scripts a stopped daemon left running, since no one bounds them any more, and returns
/// their process groups to kill.
pub(crate) fn orphaned(db: &Connection) -> rusqlite::Result<Vec<u32>> {
    let mut statement = db.prepare(
        "UPDATE sweeps SET ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
           status = 'killed when the daemon restarted', output = ''
         WHERE ended IS NULL RETURNING pid",
    )?;
    let pids = statement
        .query_map([], |row| row.get::<_, Option<u32>>(0))?
        .filter_map(Result::transpose)
        .collect::<rusqlite::Result<_>>()?;
    swept(db)?;
    Ok(pids)
}

/// Starts a sweeper task at `start` of the `sweep` workflow once the open stall has outlasted its
/// scripts by `after`, unless the stall has one. While a sweeper of an earlier stall is active, the
/// stall waits for it to end, then gets its own. Its source event carries the stall and each script's output, so its first prompt names them.
pub(crate) fn call_sweeper(
    db: &mut Connection,
    after: Duration,
    band: Option<&str>,
    start: &str,
    home: &Path,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let due: Option<i64> = tx
        .query_row(
            "SELECT id FROM stalls WHERE ended IS NULL AND sweeper IS NULL
               AND swept <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)",
            [format!("-{} seconds", after.as_secs())],
            |row| row.get(0),
        )
        .optional()?;
    let Some(stall) = due else {
        return Ok(());
    };
    let active = tx
        .query_row(
            "SELECT 1 FROM tasks WHERE key = ?1 AND state NOT IN ('done', 'failed', 'dropped')",
            [KEY],
            |_| Ok(()),
        )
        .optional()?;
    if active.is_some() {
        return Ok(());
    }
    let source = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO tasks
           (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
         VALUES (?1, ?2, ?3, 'queued', ?4, 'priory/sweep', 0, ?5, ?6,
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![KEY, SWEEP, start, band, TRIGGER, source],
    )?;
    let task = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
        params![task, KEY],
    )?;
    tx.execute(
        "INSERT INTO inbox (id, type, time, data, links, task, outcome)
         SELECT ?2, 'priory.queue.stalled', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
           json_object('resource', resource, 'missing', missing, 'since', since,
             'task', task, 'home', ?4, 'scripts', json((
               SELECT json_group_array(json_object('script', script, 'status', status,
                 'output', output))
               FROM (SELECT * FROM sweeps WHERE stall = ?1 ORDER BY id)))),
           '[]', ?3, 'created'
         FROM stalls WHERE id = ?1",
        params![stall, source, task, home.to_string_lossy()],
    )?;
    tx.execute(
        "UPDATE stalls SET sweeper = ?2 WHERE id = ?1",
        params![stall, task],
    )?;
    tx.commit()
}
