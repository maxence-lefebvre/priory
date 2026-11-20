//! The state the listing commands and the HTTP server show, one JSON payload per listing.

use crate::{post, snooze, stall, state, sweep};
use jiff::Timestamp;
use priory_factory::factory::{Factory, Trigger};
use priory_intake::intake::schedule;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Params, params};
use serde_json::{Map, Value, json};
use std::path::Path;

/// Each queued or ready task's place in the start order, overall and in its band, from 1.
fn places() -> String {
    format!(
        "WITH queue AS (SELECT id, row_number() OVER (ORDER BY {order}) AS place,
           row_number() OVER (PARTITION BY band ORDER BY {order}) AS band_place
         FROM tasks WHERE state IN ('queued', 'ready'))",
        order = state::START_ORDER
    )
}

/// Asks, blocked tasks and uncertain prompts oldest first, then the queue in start order, each with
/// its snooze.
fn desk_items() -> String {
    format!(
        "{} SELECT desk.since, desk.kind, desk.id, desk.task,
           (SELECT key FROM tasks WHERE tasks.id = desk.task) AS key,
           (SELECT step FROM tasks WHERE tasks.id = desk.task) AS step, desk.lane, desk.what,
           (SELECT generation FROM tasks WHERE tasks.id = desk.task) AS generation, {}
         FROM ({DESK}) AS desk
         LEFT JOIN snoozes ON snoozes.item = desk.id AND typeof(desk.id) = 'integer' AND snoozes.kind =
           CASE WHEN desk.kind IN ('blocked', 'failed') THEN 'task' ELSE desk.kind END
         ORDER BY desk.part, CASE WHEN desk.part = 0 THEN desk.since END, desk.kind, desk.id",
        places(),
        snooze::FIELDS
    )
}

const DESK: &str = "SELECT 0 AS part, asks.created AS since, 'ask' AS kind,
                 coalesce(asks.name, asks.id) AS id, asks.task AS task, tasks.lane AS lane,
                 asks.question || ' ' || asks.options AS what
               FROM asks LEFT JOIN tasks ON tasks.id = asks.task
               WHERE asks.answer IS NULL AND asks.withdrawn IS NULL
                 AND coalesce(tasks.state, '') NOT IN ('done', 'failed', 'dropped')
               UNION ALL
               SELECT 0, deliveries.ended, 'delivery', asks.name, NULL, NULL,
                 'the answer did not reach ' || asks.session || ' after ' || deliveries.tries
                   || ' tries: ' || coalesce(deliveries.error, '')
               FROM deliveries JOIN asks ON asks.id = deliveries.ask
               WHERE deliveries.state = 'failed' AND deliveries.revision = asks.revision
                 AND deliveries.id = (SELECT max(id) FROM deliveries AS later
                   WHERE later.ask = deliveries.ask)
               UNION ALL
               SELECT 0, changed, 'blocked', id, id, lane, reason FROM tasks WHERE state = 'blocked'
               UNION ALL
               SELECT 0, changed, 'failed', id, id, lane,
                 coalesce(reason, 'failed') || '; revive: priory revive ' || id
               FROM tasks WHERE state = 'failed'
               UNION ALL
               SELECT 0, outbox.attempted, 'prompt', outbox.id, outbox.task, tasks.lane, outbox.text
               FROM outbox JOIN tasks ON tasks.id = outbox.task
               WHERE outbox.state = 'uncertain' AND tasks.state NOT IN ('done', 'failed', 'dropped')
               UNION ALL
               SELECT 0, holds.created, 'held', holds.id, holds.task, tasks.lane, tasks.reason
               FROM holds JOIN tasks ON tasks.id = holds.task
               WHERE holds.released IS NULL AND tasks.state = 'held'
               UNION ALL
               SELECT 0, sweeps.started, 'sweep', sweeps.id, stalls.task, tasks.lane,
                 sweeps.script || ': ' || coalesce(sweeps.status
                   || coalesce(nullif(': ' || sweeps.output, ': '), ''), 'running')
               FROM sweeps JOIN stalls ON stalls.id = sweeps.stall JOIN tasks ON tasks.id = stalls.task
               WHERE stalls.ended IS NULL
               UNION ALL
               SELECT 0, grants.created, 'memory', grants.id, grants.task, tasks.lane,
                 'waits for ' || printf('%.1f', grants.bytes / 1073741824.0) || ' GB: '
                 || grants.command
               FROM grants LEFT JOIN tasks ON tasks.id = grants.task WHERE grants.state = 'waiting'
               UNION ALL
               SELECT 0, retired, 'retired', id, id, lane,
                 'its worktree was removed to free disk; it resurrects from ' || branch
                   || ' when its wait fires'
               FROM tasks WHERE state = 'asleep' AND retired IS NOT NULL
               UNION ALL
               SELECT 0, time, 'report', seq, NULL, NULL,
                 subject || ': ' || json_extract(data, '$.line')
               FROM inbox WHERE type = 'priory.report' AND read IS NULL
               UNION ALL
               SELECT 1, tasks.changed, 'queue', queue.place, tasks.id, tasks.lane,
                 coalesce(tasks.band, '-') || coalesce(': ' || tasks.reason, '')
               FROM tasks JOIN queue USING (id)";

fn asks_query() -> String {
    format!(
        "SELECT coalesce(asks.name, asks.id) AS id, asks.task,
     coalesce(tasks.key, asks.key) AS key, asks.step, tasks.lane, asks.run,
     CASE WHEN asks.withdrawn IS NOT NULL THEN 'withdrawn'
       WHEN asks.answer IS NOT NULL THEN 'answered' ELSE 'open' END AS state,
     CASE asks.withdrawn WHEN -1 THEN 'dismissed' WHEN 0 THEN 'a later run called another tool'
       ELSE 'revived as generation ' || asks.withdrawn END AS withdrawn_why,
     asks.revision, asks.question, asks.options, asks.recommendation, asks.tradeoff, asks.answer,
     asks.answered AS answered_at, asks.created AS since, asks.title, asks.text,
     asks.what_waits, asks.next_action, asks.command, asks.coordinator, asks.run_id, asks.session,
     (SELECT json_object('state', state, 'revision', revision, 'tries', tries, 'error', error,
        'since', coalesce(ended, created))
      FROM deliveries WHERE deliveries.ask = asks.id ORDER BY id DESC LIMIT 1) AS delivery, {}
   FROM asks LEFT JOIN tasks ON tasks.id = asks.task
   LEFT JOIN snoozes ON snoozes.kind = 'ask' AND snoozes.item = asks.id WHERE ?1 IS NULL OR asks.task = ?1 ORDER BY asks.id",
        snooze::FIELDS
    )
}

/// What `read` returns, read in one snapshot with the revision it read. An object gains
/// `revision`; an array, or an object with a `revision` of its own, such as a report, becomes
/// `{"revision": N, "<name>": ...}`.
pub fn revised(
    db: &Connection,
    name: &str,
    read: impl FnOnce(&Connection) -> rusqlite::Result<Option<Value>>,
) -> rusqlite::Result<Option<Value>> {
    let _snapshot = db.unchecked_transaction()?;
    let revision = state::revision(db)?;
    Ok(read(db)?.map(|value| match value {
        Value::Object(mut object) if !object.contains_key("revision") => {
            object.insert("revision".to_owned(), revision.into());
            Value::Object(object)
        }
        value => json!({ "revision": revision, name: value }),
    }))
}

/// Every inbox message and report after sequence number `since`, oldest first.
pub fn inbox(db: &Connection, since: i64) -> rusqlite::Result<Value> {
    rows(
        db,
        "SELECT seq, type, subject, task, outcome, time,
           CASE type WHEN ?2 THEN json_extract(data, '$.line') END AS line, read
         FROM inbox WHERE seq > ?1 ORDER BY seq",
        params![since, post::REPORT],
    )
}

/// Every task in start order, or the tasks of one lane, of one grade, or both.
pub fn tasks(db: &Connection, lane: Option<&str>, grade: Option<&str>) -> rusqlite::Result<Value> {
    let query = format!(
        "{} SELECT id, key, workflow, step, state, band, lane, reason, queue.place AS position,
           generation, retired, tldr, changed AS since, grade, grade_reason
         FROM tasks LEFT JOIN queue USING (id)
         WHERE (?1 IS NULL OR lane = ?1) AND (?2 IS NULL OR grade = ?2) ORDER BY {}",
        places(),
        state::START_ORDER
    );
    rows(db, &query, [lane, grade])
}

/// Every ask with its state, oldest first.
pub fn asks(db: &Connection) -> rusqlite::Result<Value> {
    asks_of(db, None)
}

fn asks_of(db: &Connection, task: Option<i64>) -> rusqlite::Result<Value> {
    let mut asks = rows(db, &asks_query(), [task])?;
    for ask in asks.as_array_mut().into_iter().flatten() {
        if let Some(options) = ask["options"].as_str() {
            ask["options"] = serde_json::from_str(options).unwrap_or(Value::Null);
        }
        if let Some(delivery) = ask["delivery"].as_str() {
            ask["delivery"] = serde_json::from_str(delivery).unwrap_or(Value::Null);
        }
        parse_back_early(ask);
    }
    Ok(asks)
}

/// The waiting tasks in start order, then the held ones with their hold, the live runs, and the
/// tasks that ended in the last day, and up next: the next three runs, with `schedules`' firings.
pub fn queue(
    db: &Connection,
    schedules: &[(String, Trigger)],
    now: Timestamp,
) -> rusqlite::Result<Value> {
    let waiting = format!(
        "{} SELECT position, band_position, id, key, workflow, step, state, band, lane, reason, since,
           hold, hold_until, hold_target, hold_resource FROM (
           SELECT 0 AS part, queue.place AS position, queue.band_place AS band_position, id, key,
             workflow, step, state, band, lane, reason, changed AS since, NULL AS hold,
             NULL AS hold_until, NULL AS hold_target, NULL AS hold_resource, queue.place AS sort
           FROM tasks JOIN queue USING (id)
           UNION ALL
           SELECT 1, NULL, NULL, tasks.id, key, workflow, step, state, band, lane, tasks.reason,
             changed, holds.kind, holds.until, holds.target, holds.resource, holds.id
           FROM tasks JOIN holds ON holds.task = tasks.id AND holds.released IS NULL
           WHERE tasks.state = 'held'
         ) ORDER BY part, sort",
        places()
    );
    let waiting = rows(db, &waiting, [])?;
    Ok(json!({
        "up_next": up_next(db, &waiting, schedules, now)?,
        "waiting": waiting,
        "running": rows(
            db,
            "SELECT runs.id AS run, runs.task, tasks.key, runs.step, runs.agent, runs.started
             FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL ORDER BY runs.id",
            [],
        )?,
        "ended": rows(
            db,
            "SELECT id, key, workflow, step, state, reason, changed AS ended FROM tasks
             WHERE state IN ('done', 'failed', 'dropped')
               AND changed >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 day')
             ORDER BY changed DESC, id DESC",
            [],
        )?,
    }))
}

/// The factory's schedule triggers by name.
pub fn schedule_triggers(factory: &Factory) -> Vec<(String, Trigger)> {
    factory
        .triggers
        .iter()
        .filter(|(_, trigger)| trigger.value.source == "schedule")
        .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
        .collect()
}

/// The next three runs, soonest first: queued tasks now in start order, tasks held until a time,
/// and schedule firings that start or wake a task.
fn up_next(
    db: &Connection,
    waiting: &Value,
    schedules: &[(String, Trigger)],
    now: Timestamp,
) -> rusqlite::Result<Value> {
    let entry = |at: String, kind, row: &Value, trigger: &Value, why: String| json!({ "at": at, "kind": kind, "task": row["id"], "key": row["key"], "trigger": trigger, "why": why });
    let mut next: Vec<(Timestamp, Value)> = Vec::new();
    for row in waiting.as_array().into_iter().flatten() {
        if let Some(position) = row["position"].as_i64() {
            let why = match (position, row["band_position"].as_i64()) {
                (1, _) => "first in the queue".to_owned(),
                (_, Some(1)) => "a band above it".to_owned(),
                _ => format!("queue position {position}"),
            };
            next.push((
                now,
                entry(now.to_string(), "queued", row, &Value::Null, why),
            ));
            continue;
        }
        let until = row["hold_until"]
            .as_str()
            .and_then(|until| until.parse().ok());
        if let (Some("time"), Some(until)) = (row["hold"].as_str(), until) {
            let at = Timestamp::max(until, now);
            let why = format!("held until {until}");
            next.push((at, entry(at.to_string(), "held", row, &Value::Null, why)));
        }
    }
    for firing in firings(db, schedules, now)? {
        // A held task waits for its release, and a live task that is not asleep coalesces the
        // firing, so neither starts a run then.
        if firing.trigger.hold.is_some()
            || firing
                .task
                .as_ref()
                .is_some_and(|(_, state)| state != "asleep")
        {
            continue;
        }
        // Each firing of a keyless schedule is its own task; a fixed key's later ones coalesce.
        let count = if firing.key.is_none() { 3 } else { 1 };
        let row = json!({ "id": firing.task.map(|(id, _)| id), "key": firing.key });
        let slots = std::iter::successors(firing.next, |at| schedule::next(firing.trigger, *at));
        for at in slots.take(count) {
            let why = format!("schedule {} fires", firing.name);
            next.push((
                at,
                entry(at.to_string(), "scheduled", &row, &json!(firing.name), why),
            ));
        }
    }
    next.sort_by_key(|(at, _)| *at);
    Ok(next.into_iter().take(3).map(|(_, entry)| entry).collect())
}

/// Each schedule trigger by name with its next firing after `now`, its fixed key, and the live task
/// that key names, which a firing wakes or coalesces into.
pub fn schedules(
    db: &Connection,
    schedules: &[(String, Trigger)],
    now: Timestamp,
) -> rusqlite::Result<Value> {
    let listed = firings(db, schedules, now)?.into_iter().map(|firing| {
        let (task, task_state) = firing.task.unzip();
        json!({
            "trigger": firing.name,
            "workflow": firing.trigger.workflow,
            "next": firing.next.map(|next| next.to_string()),
            "key": firing.key,
            "task": task,
            "task_state": task_state,
        })
    });
    Ok(listed.collect())
}

struct Firing<'a> {
    name: &'a str,
    trigger: &'a Trigger,
    next: Option<Timestamp>,
    key: Option<String>,
    /// The live task the fixed key names, and its state.
    task: Option<(i64, String)>,
}

fn firings<'a>(
    db: &Connection,
    schedules: &'a [(String, Trigger)],
    now: Timestamp,
) -> rusqlite::Result<Vec<Firing<'a>>> {
    let mut live = db.prepare(
        "SELECT id, state FROM tasks WHERE key = ?1 AND state NOT IN ('done', 'failed', 'dropped')
         ORDER BY id DESC LIMIT 1",
    )?;
    let mut firings = Vec::new();
    for (name, trigger) in schedules {
        let key = trigger.key.as_ref().map(|key| format!("schedule:{key}"));
        let task = match &key {
            Some(key) => live
                .query_row([key], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?,
            None => None,
        };
        firings.push(Firing {
            name,
            trigger,
            next: schedule::next(trigger, now),
            key,
            task,
        });
    }
    Ok(firings)
}

/// `back_early` as the object it is stored as.
fn parse_back_early(item: &mut Value) {
    if let Some(back) = item["back_early"].as_str() {
        item["back_early"] = serde_json::from_str(back).unwrap_or(Value::Null);
    }
}

/// The person a desk is for when no one is named.
pub const ME: &str = "me";

/// The trace records and inbox entries after `person`'s last visit, newest first, at most 50.
fn since_you_looked(db: &Connection, person: &str) -> rusqlite::Result<Value> {
    rows(
        db,
        "WITH events AS (
           SELECT time, CAST(json_extract(event, '$.data.task') AS INTEGER) AS task,
             type || coalesce(' ' || json_extract(event, '$.data.step'), '')
               || coalesce(': ' || json_extract(event, '$.data.outcome'), '') AS what,
             0 AS source, seq
           FROM trace
           UNION ALL
           SELECT time, task, CASE type WHEN ?2 THEN subject || ': ' || json_extract(data, '$.line')
               ELSE type || coalesce(' ' || subject, '') || coalesce(': ' || outcome, '') END,
             1, seq
           FROM inbox)
         SELECT time, task, (SELECT key FROM tasks WHERE tasks.id = events.task) AS key, what
         FROM events
         WHERE julianday(time) >= coalesce((SELECT julianday(seen) FROM visits WHERE person = ?1), 0)
         ORDER BY julianday(time) DESC, source DESC, seq DESC LIMIT 50",
        params![person, post::REPORT],
    )
}

/// The pause's line, a stalled queue's banner, the asleep and snoozed counts, what waits on a person, the queue, then
/// every worktree under `home` that stays and why. The snoozed items show only with `all`. Then
/// `person`'s last visit and what happened since.
pub fn desk(db: &Connection, home: &Path, all: bool, person: &str) -> rusqlite::Result<Value> {
    let mut items = rows(db, &desk_items(), [])?;
    let list = items.as_array_mut().expect("rows are an array");
    list.iter_mut().for_each(parse_back_early);
    let snoozed = list
        .iter()
        .filter(|item| !item["snoozed_until"].is_null())
        .count();
    list.retain(|item| all || item["snoozed_until"].is_null());
    for (path, task, reason) in sweep::stays(db, home)? {
        list.push(json!({
            "since": null, "kind": "worktree", "id": path, "task": task, "key": null,
            "step": null, "lane": null, "what": reason, "generation": null,
            "snoozed_until": null, "snoozed_at": null, "back_early": null,
        }));
    }
    let asleep: i64 = db.query_row(
        "SELECT count(*) FROM tasks WHERE state = 'asleep'",
        [],
        |row| row.get(0),
    )?;
    Ok(json!({
        "paused": state::pause_banner(db)?,
        "banner": stall::banner(db)?,
        "counts": { "asleep": asleep, "snoozed": snoozed },
        "items": items,
        "last_visit": db
            .query_row("SELECT seen FROM visits WHERE person = ?1", [person], |row| row.get::<_, String>(0))
            .optional()?,
        "since_you_looked": since_you_looked(db, person)?,
    }))
}

/// One task with its runs, code steps, asks, waits and results; none when it does not exist.
pub fn task(db: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    let found = rows(
        db,
        &format!(
            "{} SELECT id, key, workflow, step, state, band, lane, reason, queue.place AS position,
               generation, retired, tldr, changed AS since, repo, branch, grade, grade_reason
             FROM tasks LEFT JOIN queue USING (id) WHERE id = ?1",
            places()
        ),
        [id],
    )?;
    let Some(mut task) = found.as_array().and_then(|rows| rows.first()).cloned() else {
        return Ok(None);
    };
    let lists = [
        (
            "runs",
            "SELECT id, step, agent, tmux, coalesce(thread, session) AS session, started, ended,
               ended_as AS end FROM runs WHERE task = ?1 ORDER BY id",
        ),
        (
            "steps",
            "SELECT id, step, action, status, pull_request, commit_sha, error, created
             FROM actions WHERE task = ?1 ORDER BY id",
        ),
        (
            "waits",
            "SELECT id, run, kind, resource, until, target, created, fired, withdrawn
             FROM waits WHERE task = ?1 ORDER BY id",
        ),
        (
            "results",
            "SELECT run, step, outcome, summary, evidence, commit_sha, reason, created
             FROM results WHERE task = ?1 ORDER BY run",
        ),
    ];
    for (name, query) in lists {
        task[name] = rows(db, query, [id])?;
    }
    task["asks"] = asks_of(db, Some(id))?;
    Ok(Some(task))
}

/// The rows of `query` as JSON objects keyed by column name.
fn rows(db: &Connection, query: &str, params: impl Params) -> rusqlite::Result<Value> {
    let mut statement = db.prepare(query)?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let rows = statement
        .query_map(params, |row| {
            let mut object = Map::new();
            for (i, name) in names.iter().enumerate() {
                let value = match row.get_ref(i)? {
                    ValueRef::Null | ValueRef::Blob(_) => Value::Null,
                    ValueRef::Integer(n) => n.into(),
                    ValueRef::Real(x) => x.into(),
                    ValueRef::Text(text) => String::from_utf8_lossy(text).into(),
                };
                object.insert(name.clone(), value);
            }
            Ok(Value::Object(object))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::Array(rows))
}

/// Prints `columns` of each row tab-separated after a header: `-` for null and `\n` for a newline.
pub fn print(columns: &[&str], rows: &Value) {
    state::emit(&columns.join("\t"));
    for row in rows.as_array().into_iter().flatten() {
        let cells: Vec<String> = columns
            .iter()
            .map(|column| match &row[*column] {
                Value::Null => "-".to_owned(),
                Value::String(text) => text.replace('\n', "\\n"),
                value => value.to_string(),
            })
            .collect();
        state::emit(&cells.join("\t"));
    }
}
