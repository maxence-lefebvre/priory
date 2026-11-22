//! `priory label TASK wrong`: a person marks a task `marked-wrong` with the evidence they read, or
//! clears that mark. Only a person runs it, so no agent moves a number that describes agents.

use crate::trace::{self, Trace};
use jiff::Timestamp;
use priory_factory::factory::Factory;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

/// The label a person's mark writes.
pub const MARKED_WRONG: &str = "marked-wrong";

/// The evidence of each mark of `task` that no clear withdrew, oldest first.
pub(crate) fn marks(db: &Connection, task: &str) -> rusqlite::Result<Vec<String>> {
    db.prepare(
        "SELECT json_extract(event, '$.data.evidence') FROM trace
         WHERE type = 'factory.step.labeled' AND json_extract(event, '$.data.task') = ?1
         AND json_extract(event, '$.data.label') = ?2
         AND seq > coalesce((SELECT max(seq) FROM trace WHERE type = 'factory.step.unlabeled'
           AND json_extract(event, '$.data.task') = ?1
           AND json_extract(event, '$.data.label') = ?2), 0)
         ORDER BY seq",
    )?
    .query_map([task, MARKED_WRONG], |row| row.get(0))?
    .collect()
}

/// When a mark says its event happened: an RFC 3339 instant, or a date read as its midnight UTC.
fn happened(text: &str, now: Timestamp) -> Result<Timestamp, String> {
    let at = text
        .parse::<Timestamp>()
        .or_else(|_| {
            text.parse::<jiff::civil::Date>()
                .and_then(|date| date.to_zoned(jiff::tz::TimeZone::UTC))
                .map(|zoned| zoned.timestamp())
        })
        .map_err(|_| {
            format!(
                "--happened {text:?} is neither an RFC 3339 instant nor a date such as 2026-10-02"
            )
        })?;
    if at > now {
        return Err(format!("--happened {text} is after now"));
    }
    Ok(at)
}

fn trace(factory: &Factory) -> Trace<'_> {
    Trace {
        factory,
        revision: priory_runtime::runtime::revision(&factory.dir),
        machine: priory_runtime::room::hostname(),
        runner: None,
        usage: priory_runtime::transcript::usage,
    }
}

fn exists(db: &Connection, task: i64) -> Result<(), String> {
    db.query_row("SELECT 1 FROM tasks WHERE id = ?1", [task], |_| Ok(()))
        .optional()
        .map_err(|error| error.to_string())?
        .map(|_| ())
        .ok_or_else(|| format!("no task {task}"))
}

/// Marks `task` wrong with `evidence`, an http or https URL, as of `happened`, else `now`.
pub fn mark(
    db: &mut Connection,
    factory: &Factory,
    task: i64,
    evidence: &str,
    happened_at: Option<&str>,
    now: Timestamp,
) -> Result<String, String> {
    let error = |error: rusqlite::Error| error.to_string();
    if !["https://", "http://"]
        .iter()
        .any(|scheme| evidence.starts_with(scheme) && evidence.len() > scheme.len())
    {
        return Err(format!(
            "--evidence {evidence:?} is not a URL: name the issue, pull request or commit that shows the task was wrong"
        ));
    }
    let at = match happened_at {
        Some(text) => happened(text, now)?,
        None => now,
    }
    .to_string();
    exists(db, task)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    trace::labeled(
        &tx,
        &trace(factory),
        task,
        MARKED_WRONG,
        evidence,
        Some(&at),
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(format!("task {task} marked wrong: {evidence}"))
}

/// Withdraws every mark of `task` that no clear withdrew yet.
pub fn clear(
    db: &mut Connection,
    factory: &Factory,
    task: i64,
    now: Timestamp,
) -> Result<String, String> {
    let error = |error: rusqlite::Error| error.to_string();
    exists(db, task)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    if marks(&tx, &task.to_string()).map_err(error)?.is_empty() {
        return Err(format!("task {task} is not marked wrong"));
    }
    trace::unlabeled(&tx, &trace(factory), task, MARKED_WRONG, &now.to_string()).map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(format!("task {task} is no longer marked wrong"))
}

/// `POST /api/tasks/{id}/label` on the daemon's socket: `{"label": {"task", "evidence", "happened",
/// "clear"}}`, a mark with evidence or a clear.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub label: Request,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub task: i64,
    pub evidence: Option<String>,
    pub happened: Option<String>,
    pub clear: bool,
}

/// Marks or clears as `priory label TASK wrong` does, at `now`.
pub fn apply(
    db: &mut Connection,
    factory: &Factory,
    request: &Request,
    now: Timestamp,
) -> Result<String, String> {
    match &request.evidence {
        Some(evidence) => mark(
            db,
            factory,
            request.task,
            evidence,
            request.happened.as_deref(),
            now,
        ),
        None => clear(db, factory, request.task, now),
    }
}
