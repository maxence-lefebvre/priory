//! The review limit: a lane's or a batch's new tasks wait to start while `[queue] review_limit` of
//! its pull requests wait for a person.

use rusqlite::{Connection, ToSql, params};
use std::num::NonZeroU32;

/// The start of the reason of a task the review limit holds.
const WAITS: &str = "waits: review limit: ";

/// The open pull requests of tasks whose agent works on none of them now, among the tasks `scope`
/// selects with `?1`.
fn waiting(db: &Connection, scope: &str, value: &dyn ToSql) -> rusqlite::Result<u32> {
    db.query_row(
        &format!(
            "SELECT count(DISTINCT pull_requests.resource) FROM pull_requests
             JOIN tasks ON tasks.id = pull_requests.task
             WHERE pull_requests.state = 'open'
             AND tasks.state NOT IN ('queued', 'ready', 'running', 'failed', 'dropped')
             AND {scope}"
        ),
        [value],
        |row| row.get(0),
    )
}

/// Why the review limit holds `task`, a task whose first run has not started, if it does: its
/// batch's pull requests count first, then its lane's.
pub(crate) fn waits(
    db: &Connection,
    limit: Option<NonZeroU32>,
    task: i64,
) -> rusqlite::Result<Option<String>> {
    let Some(limit) = limit else {
        return Ok(None);
    };
    let (lane, batch): (Option<String>, Option<i64>) = db.query_row(
        "SELECT lane, batch FROM tasks WHERE id = ?1",
        params![task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if let Some(batch) = batch {
        let count = waiting(db, "tasks.batch = ?1", &batch)?;
        if count >= limit.get() {
            return Ok(Some(reason(&format!("batch {batch} has"), count, limit)));
        }
    }
    let count = waiting(db, "tasks.lane IS ?1", &lane)?;
    if count < limit.get() {
        return Ok(None);
    }
    let scope = match &lane {
        Some(lane) => format!("lane {lane} has"),
        None => "the tasks with no lane have".to_owned(),
    };
    Ok(Some(reason(&scope, count, limit)))
}

fn reason(scope: &str, count: u32, limit: NonZeroU32) -> String {
    let requests = if count == 1 {
        "pull request"
    } else {
        "pull requests"
    };
    format!("{WAITS}{scope} {count} {requests} waiting for a person, at the limit of {limit}")
}

/// The desk's lines for each lane and batch whose review limit holds a queued task now.
pub fn desk_lines(db: &Connection) -> rusqlite::Result<Vec<String>> {
    db.prepare(
        "SELECT DISTINCT substr(reason, length(?1) + 1) FROM tasks
         WHERE state IN ('queued', 'ready') AND substr(reason, 1, length(?1)) = ?1
         ORDER BY 1",
    )?
    .query_map([WAITS], |row| {
        Ok(format!(
            "review limit reached: {}; its new runs wait",
            row.get::<_, String>(0)?
        ))
    })?
    .collect()
}
