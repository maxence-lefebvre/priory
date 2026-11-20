//! Report lines that scripts and automations post to the inbox for a person to read.

use rusqlite::{Connection, params};

/// The inbox type of a posted report.
pub const REPORT: &str = "priory.report";

/// The first segment of every message type the intake and the daemon publish.
const DAEMON_SOURCES: [&str; 7] = [
    "cli", "factory", "github", "linear", "priory", "schedule", "slack",
];

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
    let line: String = line.chars().take(LINE_MAX).collect();
    db.execute(
        "INSERT INTO inbox (id, type, subject, time, data, links, outcome)
         VALUES (lower(hex(randomblob(16))), ?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
           json_object('line', ?3), '[]', 'report')",
        params![REPORT, source, line],
    )
    .map_err(|error| error.to_string())?;
    Ok(db.last_insert_rowid())
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
