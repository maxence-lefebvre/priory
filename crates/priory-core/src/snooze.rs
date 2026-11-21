use crate::trace::{self, Trace};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

/// A desk item: an ask, a blocked or failed task, or an uncertain prompt, by the id the desk shows.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub kind: String,
    pub id: i64,
}

/// `priory snooze`: hides `snooze` from the desk until `until`, an RFC 3339 UTC time to the
/// second, or brings it back when `until` is absent.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Snooze {
    pub snooze: Item,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// The `until` of a snooze `span` from now, such as `1h`, or at the time `at`, as RFC 3339 UTC
/// rounded up to the second, so the item never comes back before the time asked for. With
/// neither, `None`: the snooze clears.
pub fn until(span: Option<&str>, at: Option<&str>) -> Result<Option<String>, String> {
    let at = match (span, at) {
        (Some(span), _) => {
            let span = span
                .parse::<jiff::Span>()
                .map_err(|error| format!("{span}: {error}"))?;
            jiff::Zoned::now()
                .checked_add(span)
                .map_err(|error| error.to_string())?
                .timestamp()
        }
        (None, Some(at)) => at
            .parse::<jiff::Timestamp>()
            .map_err(|error| format!("{at}: {error}"))?,
        (None, None) => return Ok(None),
    };
    let second = jiff::TimestampRound::new()
        .smallest(jiff::Unit::Second)
        .mode(jiff::RoundMode::Ceil);
    let at = at.round(second).map_err(|error| error.to_string())?;
    Ok(Some(at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()))
}

/// The snooze fields of an item, from a left joined row of `snoozes`: `snoozed_until` while it is
/// snoozed, `snoozed_at` once a person snoozed it, and `back_early` as JSON text once a change
/// ended the snooze before its time.
pub const FIELDS: &str = "CASE WHEN snoozes.back IS NULL
    AND snoozes.until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now') THEN snoozes.until END AS snoozed_until,
  snoozes.snoozed AS snoozed_at,
  CASE WHEN snoozes.back IS NOT NULL
    THEN json_object('at', snoozes.back, 'reason', snoozes.back_reason) END AS back_early";

/// The task of each kind of item while the desk lists it.
fn listed(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "ask" => {
            "SELECT asks.task FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1
             AND asks.answer IS NULL AND asks.withdrawn IS NULL
             AND tasks.state NOT IN ('done', 'failed', 'dropped')"
        }
        "blocked" => "SELECT id FROM tasks WHERE id = ?1 AND state = 'blocked'",
        "failed" => "SELECT id FROM tasks WHERE id = ?1 AND state = 'failed'",
        "prompt" => {
            "SELECT outbox.task FROM outbox JOIN tasks ON tasks.id = outbox.task
             WHERE outbox.id = ?1 AND outbox.state = 'uncertain'
             AND tasks.state NOT IN ('done', 'failed', 'dropped')"
        }
        _ => return None,
    })
}

/// Snoozes or clears an item, and records it in the trace. Snoozing changes no task, run or wait.
pub(crate) fn snooze(
    db: &mut Connection,
    snooze: &Snooze,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let Item { kind, id } = &snooze.snooze;
    let Some(listed) = listed(kind) else {
        return Ok(Err(format!(
            "an item is an ask, blocked, failed or prompt, not {kind}"
        )));
    };
    // A task's blocked and failed items share one snooze, so it survives the move between them.
    let stored = match kind.as_str() {
        "blocked" | "failed" => "task",
        kind => kind,
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // A replace or clear would drop an end the tick has not traced yet.
    end(&tx, trace)?;
    let Some(until) = &snooze.until else {
        let task = tx
            .query_row(
                "DELETE FROM snoozes WHERE kind = ?1 AND item = ?2 RETURNING task",
                params![stored, id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(task) = task else {
            return Ok(Err(format!("{kind} {id} is not snoozed")));
        };
        trace::snoozed(&tx, trace, task, kind, *id, None)?;
        tx.commit()?;
        return Ok(Ok(format!("{kind} {id} is back on the desk")));
    };
    let (valid, future): (bool, bool) = tx.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', ?1) IS ?1,
         ?1 > strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        [until],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !valid {
        return Ok(Err(format!(
            "{until} is not a UTC time such as 2026-11-20T18:00:00Z"
        )));
    }
    if !future {
        return Ok(Err(format!("{until} is not in the future")));
    }
    let Some(task) = tx
        .query_row(listed, [id], |row| row.get::<_, i64>(0))
        .optional()?
    else {
        return Ok(Err(format!("{kind} {id} is not on the desk")));
    };
    tx.execute(
        "INSERT OR REPLACE INTO snoozes (kind, item, task, snoozed, until)
         VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?4)",
        params![stored, id, task, until],
    )?;
    trace::snoozed(&tx, trace, task, kind, *id, Some(until))?;
    tx.commit()?;
    Ok(Ok(format!("{kind} {id} is snoozed until {until}")))
}

/// Marks each snooze that ended, at its time or early on a change, and traces its end once. The
/// mark bumps the state's revision, so a client waiting on it reads the item back.
pub(crate) fn tick(db: &mut Connection, trace: &Trace) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    end(&tx, trace)?;
    tx.commit()
}

fn end(tx: &rusqlite::Transaction, trace: &Trace) -> rusqlite::Result<()> {
    let ended: Vec<(String, i64, i64, String)> = tx
        .prepare(
            "UPDATE snoozes SET ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE ended IS NULL
               AND (back IS NOT NULL OR until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             RETURNING kind, item, task, coalesce(back_reason, 'time')",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (kind, item, task, reason) in &ended {
        trace::snooze_ended(tx, trace, (*task, kind, *item), reason)?;
    }
    Ok(())
}
