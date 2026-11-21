use crate::state::Signal;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Anthropic's longer prompt cache lifetime, in minutes: a turn after it reads the context at full price.
const CACHE: i64 = 60;
/// How long a shared session sits idle before the daemon compacts it, leaving the launch, the
/// resume and the `/compact` five minutes inside the cache's lifetime.
const AFTER: i64 = 55;
/// How long an idle compaction may take before the daemon kills it.
const WITHIN: &str = "10 minutes";
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

/// A shared session idle long enough to compact.
pub(crate) struct Due {
    pub session: String,
    /// Where its last run worked, which the compaction resumes it in.
    pub worktree: String,
    pub transcript: String,
}

/// The shared session of `agent` when it has been idle between `AFTER` and `CACHE` minutes, with
/// no live run, and no idle compaction since its last hook.
pub(crate) fn due(db: &Connection, agent: &str) -> rusqlite::Result<Option<Due>> {
    db.query_row(
        &format!(
            "WITH last AS (SELECT session, max(signaled) AS signaled FROM runs
               WHERE session = (SELECT session FROM shared_sessions WHERE agent = ?1))
             SELECT last.session, tasks.worktree, runs.transcript FROM last
             JOIN runs ON runs.id = (SELECT max(id) FROM runs WHERE session = last.session)
             JOIN tasks ON tasks.id = runs.task
             WHERE last.signaled BETWEEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{CACHE} minutes')
               AND strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{AFTER} minutes')
             AND NOT EXISTS (SELECT 1 FROM runs WHERE session = last.session AND ended IS NULL)
             AND NOT EXISTS (SELECT 1 FROM idle_compactions
               WHERE session = last.session AND started >= last.signaled)
             AND tasks.worktree IS NOT NULL AND runs.transcript IS NOT NULL"
        ),
        [agent],
        |row| {
            Ok(Due {
                session: row.get(0)?,
                worktree: row.get(1)?,
                transcript: row.get(2)?,
            })
        },
    )
    .optional()
}

pub(crate) fn begin(
    db: &Connection,
    agent: &str,
    session: &str,
    tmux: &str,
) -> rusqlite::Result<i64> {
    db.execute(
        "INSERT INTO idle_compactions (agent, session, tmux) VALUES (?1, ?2, ?3)",
        params![agent, session, tmux],
    )?;
    Ok(db.last_insert_rowid())
}

pub(crate) fn end(db: &Connection, id: i64, ended_as: &str) -> rusqlite::Result<()> {
    db.execute(
        &format!("UPDATE idle_compactions SET ended = {NOW}, ended_as = ?2 WHERE id = ?1"),
        params![id, ended_as],
    )
    .map(drop)
}

/// A live idle compaction and what the daemon does with it next.
pub(crate) struct Live {
    pub id: i64,
    pub session: String,
    pub tmux: String,
    /// Its `SessionStart` arrived and no `/compact` was typed yet.
    pub ready: bool,
    /// No `/compact` was typed before the cache may have expired.
    pub late: bool,
    pub overdue: bool,
}

/// The tmux sessions of every idle compaction, which a restarted daemon kills: one it ended may
/// have crashed before it killed the session.
pub(crate) fn tmux_sessions(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare("SELECT DISTINCT tmux FROM idle_compactions")?;
    statement.query_map([], |row| row.get(0))?.collect()
}

pub(crate) fn live(db: &Connection) -> rusqlite::Result<Vec<Live>> {
    let mut statement = db.prepare(&format!(
        "SELECT id, session, tmux, ready AND typed IS NULL,
           typed IS NULL AND (SELECT max(signaled) FROM runs WHERE runs.session = idle_compactions.session)
             <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{CACHE} minutes'),
           started <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{WITHIN}')
         FROM idle_compactions WHERE ended IS NULL"
    ))?;
    statement
        .query_map([], |row| {
            Ok(Live {
                id: row.get(0)?,
                session: row.get(1)?,
                tmux: row.get(2)?,
                ready: row.get(3)?,
                late: row.get(4)?,
                overdue: row.get(5)?,
            })
        })?
        .collect()
}

pub(crate) fn typed(db: &Connection, id: i64) -> rusqlite::Result<()> {
    db.execute(
        &format!("UPDATE idle_compactions SET typed = {NOW} WHERE id = ?1"),
        [id],
    )
    .map(drop)
}

/// Whether the shared session of `agent` is being compacted, so no run may resume it yet.
pub(crate) fn compacting(db: &Connection, agent: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM idle_compactions WHERE agent = ?1 AND ended IS NULL)",
        [agent],
        |row| row.get(0),
    )
}

/// Applies a hook of a session that no run holds, as an idle compaction's. Returns the tmux session
/// to kill once its compaction ended.
pub(crate) fn signal(tx: &Transaction, signal: &Signal) -> rusqlite::Result<Option<String>> {
    let Some((id, tmux)) = tx
        .query_row(
            "SELECT id, tmux FROM idle_compactions WHERE session = ?1 AND ended IS NULL",
            [&signal.session],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    if signal.event != "SessionStart" {
        return Ok(None);
    }
    if signal.source.as_deref() != Some("compact") {
        tx.execute("UPDATE idle_compactions SET ready = 1 WHERE id = ?1", [id])?;
        return Ok(None);
    }
    end(tx, id, "compacted")?;
    Ok(Some(tmux))
}
