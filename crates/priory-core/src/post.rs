//! Report lines that scripts and automations post to the inbox for a person to read.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

/// The inbox type of a posted report.
pub const REPORT: &str = "priory.report";

/// The first segment of every message type the intake and the daemon publish.
const DAEMON_SOURCES: [&str; 8] = [
    "cli", "factory", "github", "intake", "linear", "priory", "schedule", "slack",
];

/// The source of the reports the intake posts when a poll fails.
const INTAKE: &str = "intake";

const LINE_MAX: usize = 500;

/// Stores one report line from `source` and returns its inbox sequence number. A line past 500
/// characters is cut there.
pub fn post(db: &Connection, source: &str, line: &str) -> Result<i64, String> {
    let valid = !source.is_empty()
        && source.len() <= 32
        && source
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err("the source must match [a-z0-9-]{1,32}".into());
    }
    if DAEMON_SOURCES.contains(&source) {
        return Err(format!("only priory itself posts as {source}"));
    }
    if line.contains(['\n', '\r']) {
        return Err("the line must be one line".into());
    }
    report(db, source, line)?;
    Ok(db.last_insert_rowid())
}

/// Stores one report line from `source`, which may be one of Priory's own sources.
pub(crate) fn report(db: &Connection, source: &str, line: &str) -> Result<(), String> {
    let line: String = line.chars().take(LINE_MAX).collect();
    db.execute(
        "INSERT INTO inbox (id, type, subject, time, data, links, outcome)
         VALUES (lower(hex(randomblob(16))), ?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
           json_object('line', ?3), '[]', 'report')",
        params![REPORT, source, line],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Records a poll of intake `source` at `at`. A new error posts one report; the next good poll, or
/// another error, marks it read.
pub fn polled(
    db: &mut Connection,
    source: &str,
    error: Option<&str>,
    at: &str,
) -> Result<(), String> {
    // Immediate, so a busy daemon makes it wait: a read that then writes cannot wait in WAL mode.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let (before, report): (Option<String>, Option<i64>) = tx
        .query_row(
            "SELECT error, report FROM intake_sources WHERE source = ?1",
            [source],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let changed = before.as_deref() != error;
    if changed {
        tx.execute(
            "UPDATE inbox SET read = coalesce(read, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             WHERE seq = ?1",
            [report],
        )
        .map_err(|error| error.to_string())?;
    }
    let report = match error {
        Some(error) if changed => {
            self::report(&tx, INTAKE, &format!("{source}: {error}"))?;
            Some(tx.last_insert_rowid())
        }
        _ => report.filter(|_| error.is_some()),
    };
    tx.execute(
        "INSERT INTO intake_sources (source, polled, error, failing_since, report)
         VALUES (?1, ?2, ?3, CASE WHEN ?3 IS NOT NULL THEN ?2 END, ?4)
         ON CONFLICT (source) DO UPDATE SET polled = ?2, error = ?3, report = ?4,
           failing_since = CASE WHEN ?3 IS NULL THEN NULL ELSE coalesce(failing_since, ?2) END",
        params![source, at, error, report],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

/// The intake's banner: when no source polled in the last 5 minutes, or else which fail and why.
pub fn intake_banner(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT CASE WHEN max(polled) < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-5 minutes')
           THEN 'the intake has not polled since ' || max(polled)
             || ': waits on GitHub, Linear and Slack cannot fire'
           ELSE 'the intake fails to poll ' || (SELECT group_concat(source || ' since '
             || failing_since || ': ' || error, '; ') FROM intake_sources WHERE error IS NOT NULL)
           END
         FROM intake_sources",
        [],
        |row| row.get(0),
    )
}

/// `POST /api/inbox/{seq}/read` on the daemon's socket: `{"read": SEQ}`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadLine {
    pub read: i64,
}

/// Marks report `seq` read, so it leaves the desk. Marking it again keeps the first time.
pub fn read(db: &Connection, seq: i64) -> Result<(), String> {
    let changed = db
        .execute(
            "UPDATE inbox SET read = coalesce(read, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             WHERE seq = ?1 AND type = ?2",
            params![seq, REPORT],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err(format!("{seq} is not a report"));
    }
    Ok(())
}
