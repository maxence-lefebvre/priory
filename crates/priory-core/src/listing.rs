//! The state the listing commands and the HTTP server show, one JSON payload per listing.

use crate::{post, snooze, stall, state, sweep};
use jiff::Timestamp;
use priory_factory::explain;
use priory_factory::factory::{Factory, Trigger};
use priory_intake::intake::schedule;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Params, params};
use serde_json::{Map, Value, json};
use std::path::Path;

/// Each queued or ready task's place in the start order, overall and in its band, from 1.
/// The conditions of a task's open hold as a JSON array, null when it has none.
const CONDITIONS: &str = "(SELECT json_group_array(json_object('kind', kind, 'until', until,
     'task', target, 'resource', resource, 'met', met, 'met_as', met_as))
   FROM (SELECT * FROM holds WHERE holds.task = tasks.id AND holds.released IS NULL ORDER BY id)
   HAVING count(*) > 0) AS conditions";

/// A task's title: its TL;DR when set, else the first line of its request, cut to 120 characters.
const TITLE: &str = "(SELECT substr(coalesce(nullif(task_text.tldr, ''),
     substr(task_text.request, 1, instr(task_text.request || char(10), char(10)) - 1)), 1, 120)
   FROM task_text WHERE task_text.task = tasks.id) AS title";

fn places() -> String {
    format!(
        "WITH queue AS (SELECT id, row_number() OVER (ORDER BY {order}) AS place,
           row_number() OVER (PARTITION BY band ORDER BY {order}) AS band_place
         FROM tasks WHERE state IN ('queued', 'ready'))",
        order = state::START_ORDER
    )
}

/// Whether an open ask's answer lets an irreversible step run, over `asks` joined to its `tasks`.
const IRREVERSIBLE: &str = "CASE asks.kind
       WHEN 'step' THEN EXISTS (SELECT 1 FROM irreversible_asks AS gate
         WHERE gate.workflow = tasks.workflow AND gate.step = asks.step)
         OR coalesce(json_array_length(asks.command, '$.runs') > 0, 0)
       WHEN 'agent' THEN coalesce(json_array_length(asks.command, '$.runs') > 0, 0)
       ELSE 0 END";

/// A desk item's group in the ranking (see `RANKS`), null for the queue.
fn urgency() -> String {
    format!(
        "CASE WHEN desk.part = 1 OR desk.kind = 'batch' THEN NULL
           WHEN desk.kind IN ('blocked', 'failed', 'prompt', 'command')
             OR desk.kind = 'report' AND desk.what LIKE 'intake: %' THEN 'alarm'
           WHEN desk.kind = 'ask' THEN (SELECT CASE WHEN {IRREVERSIBLE} THEN 'irreversible'
               WHEN tasks.key GLOB '*:review:*' THEN 'review'
               WHEN asks.kind = 'step' THEN 'plan' ELSE 'ask' END
             FROM asks LEFT JOIN tasks ON tasks.id = asks.task
             WHERE coalesce(asks.name, asks.id) = desk.id
               AND asks.answer IS NULL AND asks.withdrawn IS NULL)
           WHEN desk.kind = 'todo' THEN 'todo'
           ELSE 'rest' END AS urgency"
    )
}

/// An item's title as a notification carries it: an ask's title, or its task and step when it
/// has none, since a question can hold a result's summary; a task's key and kind; a report's
/// source, never its line, which can quote an error.
const DESK_TITLE: &str = "CASE WHEN desk.kind = 'ask' THEN (SELECT coalesce(asks.title,
             'task ' || tasks.key || ' asks at ' || asks.step, 'ask ' || asks.name)
           FROM asks LEFT JOIN tasks ON tasks.id = asks.task
           WHERE coalesce(asks.name, asks.id) = desk.id
             AND asks.answer IS NULL AND asks.withdrawn IS NULL)
         WHEN desk.task IS NOT NULL THEN 'task '
           || (SELECT key FROM tasks WHERE tasks.id = desk.task) || ' ' || desk.kind
         WHEN desk.kind = 'report' AND desk.what LIKE 'intake: %' THEN 'the intake fails to poll '
           || substr(desk.what, 9, instr(substr(desk.what, 9), ':') - 1)
         WHEN desk.kind = 'report' THEN 'a report from '
           || substr(desk.what, 1, instr(desk.what, ':') - 1)
         ELSE desk.kind || ' ' || desk.id END AS title";

/// Asks, blocked tasks and uncertain prompts oldest first, then the queue in start order, each with
/// its snooze, its group in the ranking and its title.
fn desk_items() -> String {
    format!(
        "{} SELECT desk.since, desk.kind, desk.id, desk.task,
           (SELECT key FROM tasks WHERE tasks.id = desk.task) AS key,
           (SELECT step FROM tasks WHERE tasks.id = desk.task) AS step, desk.lane, desk.what,
           (SELECT generation FROM tasks WHERE tasks.id = desk.task) AS generation, {},
           {}, {DESK_TITLE}
         FROM ({DESK}) AS desk
         LEFT JOIN snoozes ON snoozes.item = desk.id AND typeof(desk.id) = 'integer' AND snoozes.kind =
           CASE WHEN desk.kind IN ('blocked', 'failed') THEN 'task' ELSE desk.kind END
         ORDER BY desk.part, CASE WHEN desk.part = 0 THEN desk.since END, desk.kind, desk.id",
        places(),
        snooze::FIELDS,
        urgency()
    )
}

const DESK: &str = "SELECT 0 AS part, asks.created AS since, 'ask' AS kind,
                 coalesce(asks.name, asks.id) AS id, asks.task AS task, tasks.lane AS lane,
                 coalesce('routed to ' || asks.routed || ': ', '') || asks.question || ' '
                   || asks.options AS what
               FROM asks LEFT JOIN tasks ON tasks.id = asks.task
               WHERE asks.answer IS NULL AND asks.withdrawn IS NULL
                 AND coalesce(tasks.state, '') NOT IN ('done', 'failed', 'dropped')
               UNION ALL
               SELECT 0, deliveries.ended, 'delivery', asks.name, NULL, NULL,
                 'the answer did not reach ' || asks.session || ' after ' || deliveries.tries
                   || ' tries: ' || coalesce(deliveries.error, '')
               FROM deliveries JOIN asks ON asks.id = deliveries.ask
               WHERE deliveries.state = 'failed' AND deliveries.cleared IS NULL
                 AND deliveries.revision = asks.revision
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
               SELECT 0, actions.ended, 'command', actions.id, actions.task, tasks.lane,
                 'Priory stopped while answer ' || json_quote(asks.answer) || ' to ask ' || asks.id
                   || ' ran ' || json_extract(actions.command, '$.argv') || ' in '
                   || json_extract(actions.command, '$.dir') || ': check the target'
               FROM actions JOIN asks ON asks.id = actions.ask JOIN tasks ON tasks.id = actions.task
               WHERE actions.action = 'answer' AND actions.status = 'uncertain'
                 AND tasks.state NOT IN ('done', 'failed', 'dropped')
               UNION ALL
               SELECT 0, holds.created, 'held', holds.id, holds.task, tasks.lane, tasks.reason
               FROM holds JOIN tasks ON tasks.id = holds.task
               WHERE holds.released IS NULL AND tasks.state = 'held' AND holds.id = (
                 SELECT min(id) FROM holds AS hold WHERE hold.task = holds.task
                 AND hold.released IS NULL)
                 AND (tasks.batch_entry IS NULL OR EXISTS (SELECT 1 FROM holds AS hold
                   WHERE hold.task = holds.task AND hold.released IS NULL
                   AND (hold.kind != 'merged' OR EXISTS (SELECT 1 FROM asks WHERE asks.id = hold.ask
                     AND asks.answer IS NULL AND asks.withdrawn IS NULL))))
               UNION ALL
               SELECT 0, batches.went, 'batch', batches.id, batches.task, tasks.lane, batches.title
               FROM batches JOIN tasks ON tasks.id = batches.task WHERE batches.went IS NOT NULL
                 AND (tasks.state NOT IN ('done', 'failed', 'dropped') OR EXISTS (SELECT 1
                   FROM tasks AS entry LEFT JOIN pull_requests ON pull_requests.task = entry.id
                   WHERE entry.batch = batches.id AND entry.id != batches.task
                   AND (entry.state NOT IN ('done', 'failed', 'dropped')
                     OR pull_requests.state = 'open')))
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
               SELECT 0, created, 'todo', id, NULL, NULL,
                 text || coalesce(', due ' || due, '') FROM todos WHERE done IS NULL
               UNION ALL
               SELECT 1, tasks.changed, 'queue', queue.place, tasks.id, tasks.lane,
                 coalesce(tasks.band, '-') || coalesce(': ' || tasks.reason, '')
               FROM tasks JOIN queue USING (id)";

fn asks_query() -> String {
    format!(
        "SELECT coalesce(asks.name, asks.id) AS id, asks.task,
     coalesce(tasks.key, asks.key) AS key, asks.step, tasks.lane, tasks.band, tasks.repo, asks.run,
     CASE WHEN asks.withdrawn IS NOT NULL THEN 'withdrawn'
       WHEN asks.answer IS NOT NULL THEN 'answered' ELSE 'open' END AS state,
     asks.withdrawn_reason, asks.withdrawn_at,
     CASE asks.withdrawn_reason WHEN 'sibling-fired' THEN 'a wait of its run fired first'
       WHEN 'moved-on' THEN 'a later run called another tool'
       WHEN 'revived' THEN 'its task was revived'
       WHEN 'predecessor-revived' THEN 'the task it waits on was revived' ELSE asks.withdrawn_reason END AS withdrawn_why,
     asks.revision, asks.question, asks.options, asks.recommendation, asks.tradeoff, asks.answer,
     asks.answered AS answered_at, asks.answered_by, asks.routed AS routed_to, asks.created AS since, asks.title, asks.text,
     asks.what_waits, asks.next_action, asks.command, asks.coordinator, asks.run_id, asks.session,
     (SELECT json_object('state', state, 'revision', revision, 'tries', tries, 'error', error,
        'since', coalesce(ended, created), 'cleared', cleared)
      FROM deliveries WHERE deliveries.ask = asks.id ORDER BY id DESC LIMIT 1) AS delivery,
     (SELECT json_object('status', status, 'revision', revision, 'error', error, 'stdout', stdout,
        'stderr', stderr)
      FROM actions WHERE actions.ask = asks.id ORDER BY id DESC LIMIT 1) AS ran,
     (SELECT json_group_array(json_object('revision', revision, 'question', question,
        'options', json(options), 'recommendation', recommendation, 'text', text, 'at', created,
        'answer', answer, 'answered_at', answered))
      FROM (SELECT * FROM ask_revisions WHERE ask = asks.id ORDER BY revision)) AS revisions,
     (SELECT json_group_array(json_object('revision', revision, 'answer', answer, 'by', by,
        'why', why, 'at', created))
      FROM (SELECT * FROM ask_refusals WHERE ask = asks.id ORDER BY id)) AS refused,
     {IRREVERSIBLE} AS irreversible, tasks.merge_risk, tasks.grade,
     asks.kind != 'step' AND NOT EXISTS (SELECT 1 FROM holds
       WHERE holds.ask = asks.id AND holds.released IS NULL) AS free_text, {}
   FROM asks LEFT JOIN tasks ON tasks.id = asks.task
   LEFT JOIN snoozes ON snoozes.kind = 'ask' AND snoozes.item = asks.id
   WHERE (?1 IS NULL OR asks.task = ?1) AND (?2 IS NULL OR asks.id = ?2)
     AND (?3 IS NULL OR (asks.answer IS NULL AND asks.withdrawn IS NULL)
       OR coalesce(asks.answered, asks.withdrawn_at, asks.created) >= ?3)
   ORDER BY asks.id",
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

/// The events after sequence number `since` that reached no task and changed nothing. `owned`
/// groups them by the resource each names first, newest first, when a task owned or subscribed to
/// it: with that task and the command that routes the next event. `other` counts the rest per
/// source, since no task ever owned them.
/// `outsider` lists the comments and reviews from outside the repository, newest first.
pub fn unrouted(db: &Connection, since: i64) -> rusqlite::Result<Value> {
    let mut owned: Vec<(String, Value, Vec<Value>)> = Vec::new();
    let mut other: Vec<(String, usize)> = Vec::new();
    for (resource, event) in unrouted_events(db, since, None)? {
        if let Some((_, _, events)) = owned.iter_mut().find(|(named, ..)| *named == resource) {
            events.push(event);
            continue;
        }
        if let Some(task) = owner(db, &resource)? {
            owned.push((resource, task, vec![event]));
            continue;
        }
        let source = resource.split(':').next().unwrap_or_default().to_owned();
        match other.iter_mut().find(|(named, _)| *named == source) {
            Some((_, count)) => *count += 1,
            None => other.push((source, 1)),
        }
    }
    other.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let owned: Vec<Value> = owned
        .into_iter()
        .map(|(resource, task, events)| {
            let request_text = request_text(&resource, &events);
            let follow_up = follow_up(&task, &resource, request_text.as_deref());
            json!({
                "resource": resource, "task": task, "events": events,
                "follow_up": follow_up, "request_text": request_text,
            })
        })
        .collect();
    let other: Vec<Value> = other
        .into_iter()
        .map(|(source, events)| json!({ "source": source, "events": events }))
        .collect();
    Ok(json!({
        "unrouted": { "owned": owned, "other": other },
        "outsider": outsider_events(db, since, None)?,
    }))
}

/// Each comment or review after `since`, and within `window` (from, to) when given, from someone
/// outside the repository, newest first, with who wrote it.
fn outsider_events(
    db: &Connection,
    since: i64,
    window: Option<(&str, &str)>,
) -> rusqlite::Result<Value> {
    let (from, to) = window.unzip();
    rows(
        db,
        "SELECT seq, type, subject, time, json_extract(data, '$.actor') AS actor FROM inbox
         WHERE outcome = 'outsider' AND seq > ?1
           AND (?2 IS NULL OR julianday(time) BETWEEN julianday(?2) AND julianday(?3))
         ORDER BY seq DESC",
        params![since, from, to],
    )
}

/// The newest task that owned or subscribed to `resource`, in any state.
fn owner(db: &Connection, resource: &str) -> rusqlite::Result<Option<Value>> {
    db.query_row(
        "SELECT id, key, state, workflow FROM tasks
         WHERE id = (SELECT max(task) FROM owned WHERE resource = ?1)",
        [resource],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?, "key": row.get::<_, String>(1)?,
                "state": row.get::<_, String>(2)?, "workflow": row.get::<_, String>(3)?,
            }))
        },
    )
    .optional()
}

/// The command that routes `resource`'s next event, given its owner: revive a failed or dropped
/// one, add a task of the same workflow whose text keys it by `resource` after a done one, else
/// subscribe the active one. None after a done task when no URL can key a new task by it.
fn follow_up(task: &Value, resource: &str, request_text: Option<&str>) -> Option<String> {
    let id = &task["id"];
    match task["state"].as_str().unwrap_or_default() {
        "failed" | "dropped" => Some(format!("priory revive {id}")),
        "done" => request_text.map(|text| {
            let workflow = task["workflow"].as_str().unwrap_or_default();
            format!(
                "priory add --workflow {} {}",
                quoted(workflow),
                quoted(text)
            )
        }),
        _ => Some(format!("priory subscribe {id} {}", quoted(resource))),
    }
}

/// The text of a new task whose key is `resource`: a GitHub or Linear URL, which `priory add`
/// turns into that key, and the newest event's number, so each follow-up is a new request.
fn request_text(resource: &str, events: &[Value]) -> Option<String> {
    let url = match resource.split_once(':')? {
        ("github", pull) => {
            let (repo, number) = pull.rsplit_once('#')?;
            format!("https://github.com/{repo}/issues/{number}")
        }
        ("linear", _) => events
            .iter()
            .find_map(|event| {
                event["url"]
                    .as_str()
                    .filter(|url| url.starts_with("https://linear.app/"))
            })?
            .to_owned(),
        _ => return None,
    };
    Some(format!(
        "Follow up on {url} after event {}",
        events.first()?["seq"]
    ))
}

/// `text` as one shell word: as it is when it holds only safe characters, else single-quoted.
fn quoted(text: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "-_./:#@,+=%".contains(c);
    if !text.is_empty() && !text.starts_with('#') && text.chars().all(safe) {
        return text.to_owned();
    }
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Each unrouted event after `since`, and within `window` (from, to) when given, newest first,
/// with the first resource it names. An event that names no resource is left out: no task could
/// own it.
fn unrouted_events(
    db: &Connection,
    since: i64,
    window: Option<(&str, &str)>,
) -> rusqlite::Result<Vec<(String, Value)>> {
    let mut statement = db.prepare(
        "SELECT seq, type, subject, time, data FROM inbox
         WHERE task IS NULL AND outcome = 'ignored' AND seq > ?1
           AND (?2 IS NULL OR julianday(time) BETWEEN julianday(?2) AND julianday(?3))
           AND NOT EXISTS (SELECT 1 FROM tasks WHERE tasks.source = inbox.id)
         ORDER BY seq DESC",
    )?;
    let (from, to) = window.unzip();
    let mut rows = statement.query(params![since, from, to])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let (kind, subject): (String, Option<String>) = (row.get(1)?, row.get(2)?);
        let data: String = row.get(4)?;
        let mut message = herdr_bus::Message::new(
            kind.as_str(),
            serde_json::from_str(&data).unwrap_or(Value::Null),
        );
        message.subject = subject.clone();
        let Some(resource) = crate::route::resources(&message).into_iter().next() else {
            continue;
        };
        let said = ["body", "text", "title"]
            .iter()
            .find_map(|field| message.data[field].as_str())
            .and_then(|text| text.lines().find(|line| !line.trim().is_empty()));
        let event = json!({
            "seq": row.get::<_, i64>(0)?, "type": kind, "subject": subject,
            "time": row.get::<_, String>(3)?, "line": said, "url": message.data["url"].as_str(),
        });
        events.push((resource, event));
    }
    Ok(events)
}

/// Every task in start order, or the tasks of one lane, of one grade, or both.
pub fn tasks(db: &Connection, lane: Option<&str>, grade: Option<&str>) -> rusqlite::Result<Value> {
    tasks_since(db, lane, grade, None)
}

/// The tasks still going and those that ended since `since`, or every task without it, in start
/// order, as `tasks`, with the number of tasks left out as `older`.
pub fn recent_tasks(db: &Connection, since: Option<&str>) -> rusqlite::Result<Value> {
    let older: i64 = db.query_row(
        "SELECT count(*) FROM tasks WHERE state IN ('done', 'failed', 'dropped') AND changed < ?1",
        [since],
        |row| row.get(0),
    )?;
    Ok(json!({ "tasks": tasks_since(db, None, None, since)?, "older": older }))
}

fn tasks_since(
    db: &Connection,
    lane: Option<&str>,
    grade: Option<&str>,
    since: Option<&str>,
) -> rusqlite::Result<Value> {
    let query = format!(
        "{} SELECT id, key, workflow, step, state, band, lane, reason, queue.place AS position,
           generation, retired, tldr, changed AS since, grade, grade_reason, progress, {CONDITIONS}
         FROM tasks LEFT JOIN queue USING (id)
         WHERE (?1 IS NULL OR lane = ?1) AND (?2 IS NULL OR grade = ?2)
           AND (?3 IS NULL OR state NOT IN ('done', 'failed', 'dropped') OR changed >= ?3)
         ORDER BY {}",
        places(),
        state::START_ORDER
    );
    let mut tasks = rows(db, &query, [lane, grade, since])?;
    for task in tasks.as_array_mut().expect("rows are an array") {
        parse_json(task);
        let id = task["id"].as_i64().expect("a task has an id");
        task["stack"] = crate::stack::json(db, id)?;
        task["relations"] = crate::relate::json(db, id)?;
    }
    Ok(tasks)
}

/// `progress`, `conditions` and `waits` as the JSON they are stored or read as.
/// The active tasks that hold pull request `number` of `repo`, each with how it holds it: `key`,
/// `stack`, `wait`, `subscription`, `review`, or else `owns`, the ownership routing reads. Those
/// that own its branch, by key or stack, come first, then by id. `repo` matches in any case, as
/// GitHub's owner and repository names do.
pub fn holding(db: &Connection, repo: &str, number: u64) -> rusqlite::Result<Value> {
    let mut tasks = rows(
        db,
        "WITH named (task, how, rank) AS (
           SELECT id, 'key', 0 FROM tasks WHERE key = ?1 COLLATE NOCASE
           UNION ALL SELECT task, 'stack', 1 FROM pull_requests WHERE resource = ?1 COLLATE NOCASE
           UNION ALL SELECT DISTINCT task, 'wait', 2 FROM waits WHERE resource = ?1 COLLATE NOCASE
           UNION ALL SELECT task, 'subscription', 3 FROM subscriptions
             WHERE resource = ?1 COLLATE NOCASE
           UNION ALL SELECT id, 'review', 4 FROM tasks WHERE lower(key) GLOB lower(?2)
         ),
         held (task, how, rank) AS (
           SELECT * FROM named
           UNION ALL SELECT task, 'owns', 5 FROM owns
             WHERE resource = ?1 COLLATE NOCASE AND task NOT IN (SELECT task FROM named)
         )
         SELECT id, key, workflow, step, state, json_group_array(how) AS holds
         FROM (SELECT * FROM held ORDER BY rank) AS held JOIN tasks ON tasks.id = held.task
         WHERE state NOT IN ('done', 'failed', 'dropped')
         GROUP BY id ORDER BY min(rank) > 1, id",
        params![
            format!("github:{repo}#{number}"),
            format!("github:{repo}:pulls/{number}:review:*")
        ],
    )?;
    for task in tasks.as_array_mut().expect("rows are an array") {
        let holds = task["holds"].as_str().unwrap_or("[]");
        task["holds"] = serde_json::from_str(holds).unwrap_or(Value::Null);
    }
    Ok(tasks)
}

fn parse_json(task: &mut Value) {
    for field in ["progress", "conditions", "waits"] {
        if let Some(text) = task[field].as_str() {
            task[field] = serde_json::from_str(text).unwrap_or(Value::Null);
        }
    }
}

/// An ask's risk values in the order the Risk facet shows them; `none` picks an ask with no risk.
pub const RISKS: [&str; 8] = [
    "irreversible",
    "high",
    "medium",
    "low",
    "red",
    "yellow",
    "green",
    "none",
];

/// Refuses `risk` with `RISKS` when it is not one of them.
pub fn check_risk(risk: &str) -> Result<(), String> {
    if RISKS.contains(&risk) {
        return Ok(());
    }
    Err(format!("risk `{risk}` is not one of {}", RISKS.join(", ")))
}

/// Every ask with its state, its task's band and its risk, oldest first, or the asks of tasks in
/// one of `bands` with one of `risks`, beside the factory's bands highest first.
pub fn asks(db: &Connection, bands: &[&str], risks: &[&str]) -> rusqlite::Result<Value> {
    let asks = asks_in(db, bands, risks, None)?;
    Ok(json!({ "asks": asks, "bands": state::band_names(db)? }))
}

/// As `asks`, with only the open asks and those closed since `since` when it is given, and the
/// number of answered and withdrawn asks left out as `older`. A withdrawn ask counts as closed
/// when it was withdrawn, or when it was asked if it was withdrawn before schema 71 kept the time.
pub fn recent_asks(
    db: &Connection,
    bands: &[&str],
    risks: &[&str],
    since: Option<&str>,
) -> rusqlite::Result<Value> {
    let (answered, withdrawn) = if risks.is_empty() {
        db.query_row(
            "SELECT count(*) FILTER (WHERE asks.withdrawn IS NULL),
               count(*) FILTER (WHERE asks.withdrawn IS NOT NULL)
             FROM asks LEFT JOIN tasks ON tasks.id = asks.task
             WHERE (asks.answer IS NOT NULL OR asks.withdrawn IS NOT NULL)
               AND coalesce(asks.answered, asks.withdrawn_at, asks.created) < ?1
               AND (?2 = '[]' OR tasks.band IN (SELECT value FROM json_each(?2)))",
            params![since, json!(bands).to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    } else if let Some(since) = since {
        // An ask's risk is computed per row, so the older asks of a risk are counted from the rows.
        let all = asks_in(db, bands, risks, None)?;
        let older = |state: &str| {
            all.as_array()
                .into_iter()
                .flatten()
                .filter(|ask| {
                    let closed = ask["answered_at"].as_str();
                    let closed = closed
                        .or(ask["withdrawn_at"].as_str())
                        .or(ask["since"].as_str());
                    ask["state"] == state && closed.is_some_and(|closed| closed < since)
                })
                .count() as i64
        };
        (older("answered"), older("withdrawn"))
    } else {
        (0, 0)
    };
    Ok(json!({
        "asks": asks_in(db, bands, risks, since)?,
        "bands": state::band_names(db)?,
        "older": { "answered": answered, "withdrawn": withdrawn },
    }))
}

fn asks_in(
    db: &Connection,
    bands: &[&str],
    risks: &[&str],
    since: Option<&str>,
) -> rusqlite::Result<Value> {
    let mut asks = asks_of(db, None, None, since)?;
    if let (Some(asks), false) = (asks.as_array_mut(), bands.is_empty()) {
        asks.retain(|ask| {
            ask["band"]
                .as_str()
                .is_some_and(|band| bands.contains(&band))
        });
    }
    if let (Some(asks), false) = (asks.as_array_mut(), risks.is_empty()) {
        asks.retain(|ask| {
            let risk = ask["risk"].as_array().expect("every ask carries its risk");
            risk.iter()
                .filter_map(Value::as_str)
                .chain(risk.is_empty().then_some("none"))
                .any(|value| risks.contains(&value))
        });
    }
    Ok(asks)
}

/// Ask `id` as `asks` lists it, if there is one.
pub fn ask(db: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    let asks = asks_of(db, None, Some(id), None)?;
    Ok(asks.as_array().and_then(|asks| asks.first()).cloned())
}

fn asks_of(
    db: &Connection,
    task: Option<i64>,
    id: Option<i64>,
    since: Option<&str>,
) -> rusqlite::Result<Value> {
    let mut asks = rows(db, &asks_query(), params![task, id, since])?;
    for ask in asks.as_array_mut().into_iter().flatten() {
        if let Some(options) = ask["options"].as_str() {
            ask["options"] = serde_json::from_str(options).unwrap_or(Value::Null);
        }
        for field in ["delivery", "ran", "revisions", "refused", "merge_risk"] {
            if let Some(text) = ask[field].as_str() {
                ask[field] = serde_json::from_str(text).unwrap_or(Value::Null);
            }
        }
        // An agent's command is an object; an outside ask's stays the string it sent.
        let agents = ask["task"].is_i64();
        if let Some(Ok(command)) = ask["command"]
            .as_str()
            .filter(|_| agents)
            .map(serde_json::from_str)
        {
            ask["command"] = command;
        }
        parse_back_early(ask);
        let object = ask.as_object_mut().expect("rows are objects");
        let irreversible = object
            .remove("irreversible")
            .and_then(|value| value.as_i64())
            == Some(1);
        let risk: Vec<Value> = irreversible
            .then(|| json!("irreversible"))
            .into_iter()
            .chain(object["merge_risk"].get("verdict").cloned())
            .chain(object.remove("grade").filter(Value::is_string))
            .collect();
        object.insert("risk".to_owned(), risk.into());
        let free_text = object["free_text"].as_i64() == Some(1);
        object.insert("free_text".to_owned(), free_text.into());
    }
    Ok(asks)
}

/// The factory's bands highest first, its workflow names, the waiting tasks in start order, then the held ones with
/// their hold, the live runs, and the tasks that ended in the last day, each with its title, and up next: the next three runs, with `schedules`' firings.
pub fn queue(
    db: &Connection,
    schedules: &[(String, Trigger)],
    workflows: &[String],
    now: Timestamp,
) -> rusqlite::Result<Value> {
    let waiting = format!(
        "{} SELECT position, band_position, id, key, title, workflow, step, state, band, lane, reason,
           since, conditions FROM (
           SELECT 0 AS part, queue.place AS position, queue.band_place AS band_position, id, key,
             {TITLE}, workflow, step, state, band, lane, reason, changed AS since, NULL AS conditions,
             queue.place AS sort
           FROM tasks JOIN queue USING (id)
           UNION ALL
           SELECT 1, NULL, NULL, tasks.id, key, {TITLE}, workflow, step, state, band, lane, tasks.reason,
             changed, {CONDITIONS},
             (SELECT min(id) FROM holds WHERE holds.task = tasks.id AND holds.released IS NULL)
           FROM tasks WHERE tasks.state = 'held'
         ) ORDER BY part, sort",
        places()
    );
    let mut waiting = rows(db, &waiting, [])?;
    waiting
        .as_array_mut()
        .expect("rows are an array")
        .iter_mut()
        .for_each(parse_json);
    Ok(json!({
        "bands": state::band_names(db)?,
        "workflows": workflows,
        "up_next": up_next(db, &waiting, schedules, now)?,
        "waiting": waiting,
        "asleep": asleep(db)?,
        "running": rows(
            db,
            &format!(
                "SELECT runs.id AS run, runs.task, tasks.key, {TITLE}, runs.step, runs.agent,
                   runs.started
                 FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL
                 ORDER BY runs.id"
            ),
            [],
        )?,
        "ended": rows(
            db,
            &format!(
                "SELECT id, key, {TITLE}, workflow, step, state, reason, changed AS ended FROM tasks
                 WHERE state IN ('done', 'failed', 'dropped')
                   AND changed >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 day')
                 ORDER BY changed DESC, id DESC"
            ),
            [],
        )?,
    }))
}

/// The asleep tasks, longest asleep first, each with its open waits: what each waits on, as
/// `priory waits` shows it, and since when.
fn asleep(db: &Connection) -> rusqlite::Result<Value> {
    let mut asleep = rows(
        db,
        &format!(
            "SELECT id, key, {TITLE}, workflow, step, band, lane, changed AS since, retired,
               (SELECT json_group_array(json_object('kind', kind, 'on', coalesce(resource, until,
                  'task ' || target, printf('%.1f GB free', bytes / 1073741824.0)),
                  'since', created))
                FROM (SELECT * FROM waits WHERE waits.task = tasks.id AND fired IS NULL
                  AND withdrawn IS NULL ORDER BY id)) AS waits
             FROM tasks WHERE state = 'asleep' ORDER BY changed, id"
        ),
        [],
    )?;
    asleep
        .as_array_mut()
        .expect("rows are an array")
        .iter_mut()
        .for_each(parse_json);
    Ok(asleep)
}

/// The factory's resolved views, as `serve` and `priory views --json` show them.
pub fn views(factory: &Factory) -> Value {
    json!({ "factory": factory.name, "views": factory.views() })
}

/// The factory's links, as `serve` and `priory links --json` show them.
pub fn links(factory: &Factory) -> Value {
    json!({ "links": factory.links() })
}

/// What `priory check --explain --json` prints and `GET /api/flows` serves: the irreversible steps
/// with their gates, the paths to one with no human decision, the tools reached, and, for the
/// drawing, each workflow's steps and edges.
pub fn flows(factory: &Factory) -> Value {
    let mut irreversible = Vec::new();
    let mut paths = Vec::new();
    let mut proofs = Vec::new();
    let mut workflows = Vec::new();
    for (name, workflow) in &factory.workflows {
        let workflow = &workflow.value;
        let gates = explain::gates(workflow);
        for gate in &gates {
            irreversible.push(json!({
                "workflow": name,
                "step": gate.step,
                "approved_by": gate.approved_by,
                "autonomous": gate.autonomous,
            }));
        }
        for path in explain::unattended_paths(workflow) {
            paths.push(json!({ "workflow": name, "path": path }));
        }
        for (step, outcome, proof) in explain::proofs(workflow) {
            proofs.push(json!({
                "workflow": name, "step": step, "outcome": outcome, "proof": proof.name(),
            }));
        }
        let steps: Vec<_> = workflow
            .steps
            .iter()
            .map(|step| {
                let edges: Vec<_> = step
                    .next
                    .iter()
                    .map(|(outcome, to)| {
                        json!({ "outcome": outcome, "to": to, "by": step.by(outcome) })
                    })
                    .collect();
                let mut value = json!({
                    "id": step.id,
                    "decides": step.decides.name(),
                    "effect": step.effect.name(),
                    "kind": explain::kind(step),
                    "edges": edges,
                });
                let optional = [
                    ("agent", step.agent.as_ref().map(|agent| json!(agent))),
                    (
                        "options",
                        (!step.options.is_empty()).then(|| json!(step.options)),
                    ),
                    (
                        "approves",
                        (!step.approves.is_empty()).then(|| json!(step.approves)),
                    ),
                    (
                        "autonomous",
                        step.autonomous.as_ref().map(|reason| json!(reason)),
                    ),
                ];
                for (key, field) in optional {
                    if let Some(field) = field {
                        value[key] = field;
                    }
                }
                if let Some(gate) = gates.iter().find(|gate| gate.step == step.id) {
                    value["approved_by"] = json!(gate.approved_by);
                }
                value
            })
            .collect();
        workflows.push(json!({ "name": name, "start": workflow.start, "steps": steps }));
    }
    let tools: Vec<_> = explain::tools(factory)
        .iter()
        .map(|reach| {
            json!({
                "agent": reach.agent,
                "guard": reach.guard,
                "workflow": reach.workflow,
                "why": reach.why,
            })
        })
        .collect();
    json!({
        "irreversible": irreversible,
        "paths": paths,
        "tools": tools,
        "proofs": proofs,
        "workflows": workflows,
    })
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
    let entry = |at: String, kind, row: &Value, trigger: &Value, why: String| json!({ "at": at, "kind": kind, "task": row["id"], "key": row["key"], "title": row["title"], "trigger": trigger, "why": why });
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
        // A held task comes up once its time passes, when time is all it still waits for.
        let unmet: Vec<&Value> = row["conditions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|condition| condition["met"].is_null())
            .collect();
        let times: Option<Vec<Timestamp>> = unmet
            .iter()
            .map(|condition| condition["until"].as_str()?.parse().ok())
            .collect();
        if let Some(until) = times.and_then(|times| times.into_iter().max()) {
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
        let row = json!({
            "id": firing.task.map(|(id, _)| id), "key": firing.key, "title": firing.title,
        });
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
    /// That task's title.
    title: Option<String>,
}

fn firings<'a>(
    db: &Connection,
    schedules: &'a [(String, Trigger)],
    now: Timestamp,
) -> rusqlite::Result<Vec<Firing<'a>>> {
    let mut live = db.prepare(&format!(
        "SELECT id, state, {TITLE} FROM tasks
         WHERE key = ?1 AND state NOT IN ('done', 'failed', 'dropped') ORDER BY id DESC LIMIT 1"
    ))?;
    let mut firings = Vec::new();
    for (name, trigger) in schedules {
        let key = trigger.key.as_ref().map(|key| format!("schedule:{key}"));
        let (task, title) = match &key {
            Some(key) => live
                .query_row([key], |row| {
                    Ok((Some((row.get(0)?, row.get(1)?)), row.get(2)?))
                })
                .optional()?
                .unwrap_or_default(),
            None => (None, None),
        };
        firings.push(Firing {
            name,
            trigger,
            next: schedule::next(trigger, now),
            key,
            task,
            title,
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

/// The trace records and inbox entries after `person`'s last visit, newest first, at most 50,
/// without the events no task took.
fn since_you_looked(db: &Connection, person: &str) -> rusqlite::Result<Value> {
    rows(
        db,
        "WITH visited AS (SELECT coalesce((SELECT julianday(seen) FROM visits WHERE person = ?1), 0) AS at),
         events AS (
           SELECT * FROM (SELECT time, CAST(json_extract(event, '$.data.task') AS INTEGER) AS task,
               type || coalesce(' ' || json_extract(event, '$.data.step'), '')
                 || coalesce(': ' || json_extract(event, '$.data.outcome'), '') AS what,
               0 AS source, seq
             FROM trace WHERE julianday(time) >= (SELECT at FROM visited)
             ORDER BY julianday(time) DESC, seq DESC LIMIT 50)
           UNION ALL
           SELECT * FROM (SELECT time, task,
               CASE type WHEN ?2 THEN subject || ': ' || json_extract(data, '$.line')
                 ELSE type || coalesce(' ' || subject, '') || coalesce(': ' || outcome, '') END,
               1, seq
             FROM inbox WHERE julianday(time) >= (SELECT at FROM visited)
             -- An event no task took would crowd out the rest in a busy repository.
             AND outcome IS NOT 'ignored'
             ORDER BY julianday(time) DESC, seq DESC LIMIT 50))
         SELECT time, task, (SELECT key FROM tasks WHERE tasks.id = events.task) AS key, what
         FROM events
         ORDER BY julianday(time) DESC, source DESC, seq DESC LIMIT 50",
        params![person, post::REPORT],
    )
}

/// Why an alarm ranks first.
pub(crate) const ALARM: &str = "an alarm: work stopped and waits on a person";

/// The ranking's groups, first to last, each with why its items rank where they do. Irreversible
/// and review asks share a group.
const RANKS: [(&str, usize, &str); 7] = [
    ("alarm", 0, ALARM),
    (
        "irreversible",
        1,
        "its answer lets an irreversible step run",
    ),
    ("review", 1, "it reviews a pull request"),
    ("plan", 2, "it approves a plan"),
    ("ask", 3, "it waits on a person"),
    ("todo", 4, "a to-do of yours"),
    ("rest", 5, "it waits on a person"),
];

/// What waits on a person, best first: by group in `RANKS`, then oldest first. The queue and
/// snoozed items never rank. Each item gains `why` and keeps its `title`.
fn rank(items: &[Value]) -> Vec<Value> {
    let mut ranked: Vec<(usize, Value)> = items
        .iter()
        .filter(|item| item["snoozed_until"].is_null())
        .filter_map(|item| {
            let (_, group, why) = RANKS
                .iter()
                .find(|(urgency, ..)| item["urgency"] == *urgency)?;
            let mut item = item.clone();
            let object = item.as_object_mut().expect("rows are objects");
            object.remove("urgency");
            let acts = object.remove("acts").filter(|acts| !acts.is_null());
            object.insert("why".to_owned(), acts.unwrap_or(json!(why)));
            Some((*group, item))
        })
        .collect();
    ranked.sort_by(|(a, x), (b, y)| (a, x["since"].as_str()).cmp(&(b, y["since"].as_str())));
    ranked.into_iter().map(|(_, item)| item).collect()
}

/// The desk's `next`: what ranks first, if anything waits on a person.
pub(crate) fn next(db: &Connection) -> rusqlite::Result<Option<Value>> {
    let mut items = rows(db, &desk_items(), [])?;
    let items = items.as_array_mut().expect("rows are an array");
    routed_to(db, items)?;
    acts(db, items)?;
    Ok(rank(items).into_iter().next())
}

/// The title and task of the desk's `kind` item `id` while it ranks as an alarm and no one snoozed
/// it, for a push.
pub(crate) fn alarm(
    db: &Connection,
    kind: &str,
    id: i64,
) -> rusqlite::Result<Option<(String, Option<i64>)>> {
    let items = rows(db, &desk_items(), [])?;
    Ok(items
        .as_array()
        .expect("rows are an array")
        .iter()
        .find(|item| {
            item["kind"] == kind
                && item["id"] == id
                && item["urgency"] == "alarm"
                && item["snoozed_until"].is_null()
        })
        .map(|item| {
            (
                item["title"].as_str().unwrap_or_default().to_owned(),
                item["task"].as_i64(),
            )
        }))
}

/// Gives each `ask` item `acts`: what its answer does, when it runs a command or lets an
/// irreversible step run, or null. The command's `runs` is non-empty for an ask that runs one (see
/// `IRREVERSIBLE`).
fn acts(db: &Connection, items: &mut [Value]) -> rusqlite::Result<()> {
    let mut asked = db.prepare(
        "SELECT asks.command, (SELECT nullif(gate.lets, '') FROM irreversible_asks AS gate
           WHERE gate.workflow = tasks.workflow AND gate.step = asks.step AND asks.kind = 'step')
         FROM asks LEFT JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
    )?;
    for item in items.iter_mut().filter(|item| item["kind"] == "ask") {
        let (command, lets) = match item["id"].as_i64() {
            Some(ask) => asked.query_row([ask], |row| Ok((row.get(0)?, row.get(1)?)))?,
            None => (None::<String>, None::<String>),
        };
        item["acts"] = command
            .and_then(|command| serde_json::from_str::<Value>(&command).ok())
            .filter(|command| {
                command["runs"]
                    .as_array()
                    .is_some_and(|runs| !runs.is_empty())
            })
            .and_then(|command| {
                let program = command["argv"][0].as_str()?;
                let target = command["target"].as_str().unwrap_or("worktree");
                Some(json!(format!("it runs `{program}` in {target}")))
            })
            .or_else(|| lets.map(|step| json!(format!("it lets step `{step}` run"))))
            .unwrap_or(Value::Null);
    }
    Ok(())
}

/// Gives each `ask` item the lead it is routed to, or null.
fn routed_to(db: &Connection, items: &mut [Value]) -> rusqlite::Result<()> {
    let mut routed = db.prepare("SELECT routed FROM asks WHERE id = ?1")?;
    for item in items.iter_mut().filter(|item| item["kind"] == "ask") {
        let to = match item["id"].as_i64() {
            Some(ask) => routed.query_row([ask], |row| row.get::<_, Option<String>>(0))?,
            None => None,
        };
        item["routed_to"] = json!(to);
    }
    Ok(())
}

/// The pause's line, a stalled queue's banner, the intake's banner, the asleep and snoozed counts, what waits on a person, the queue, then
/// every worktree under `home` that stays and why. The snoozed items show only with `all`. Then
/// `person`'s last visit and what happened since.
pub fn desk(
    db: &Connection,
    home: &Path,
    all: bool,
    person: &str,
    now: Timestamp,
) -> rusqlite::Result<Value> {
    let mut items = rows(db, &desk_items(), [])?;
    let list = items.as_array_mut().expect("rows are an array");
    list.iter_mut().for_each(parse_back_early);
    for item in list.iter_mut().filter(|item| item["kind"] == "batch") {
        let id = item["id"].as_i64().expect("a batch's id is a number");
        item["what"] = json!(crate::batch::desk_line(db, id)?);
    }
    list.retain(|item| item["kind"] != "batch" || !item["what"].is_null());
    let snoozed = list
        .iter()
        .filter(|item| !item["snoozed_until"].is_null())
        .count();
    routed_to(db, list)?;
    acts(db, list)?;
    let ranked = rank(list);
    for item in list.iter_mut() {
        let item = item.as_object_mut().expect("rows are objects");
        item.remove("urgency");
        item.remove("acts");
        item.remove("title");
    }
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
    let day_ago = (now - jiff::SignedDuration::from_hours(24)).to_string();
    let day = (day_ago.as_str(), now.to_string());
    let mut unrouted = 0;
    for (resource, _) in unrouted_events(db, 0, Some((day.0, &day.1)))? {
        unrouted += usize::from(owner(db, &resource)?.is_some());
    }
    let outsider = outsider_events(db, 0, Some((day.0, &day.1)))?
        .as_array()
        .map_or(0, Vec::len);
    Ok(json!({
        "paused": state::pause_banner(db)?,
        "banner": stall::banner(db)?,
        "intake": post::intake_banner(db)?,
        "counts": {
            "asleep": asleep, "snoozed": snoozed, "unrouted": unrouted, "outsider": outsider,
        },
        "items": items,
        "next": ranked.first(),
        "ranked": ranked,
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
               generation, retired, tldr, changed AS since, repo, branch, grade, grade_reason,
               merge_risk, progress, plan_goal, plan_testing, plan_brief, verification, intervention, lead, {CONDITIONS}
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
               ended_as AS end, (SELECT outcome FROM results WHERE results.run = runs.id) AS outcome,
               (SELECT json_extract(event, '$.data.tokens') FROM trace
                WHERE type = 'factory.step.ended'
                  AND json_extract(event, '$.data.run') = CAST(runs.id AS TEXT)
                ORDER BY seq DESC LIMIT 1) AS tokens
             FROM runs WHERE task = ?1 ORDER BY id",
        ),
        (
            "steps",
            "SELECT id, step, action, status, pull_request, commit_sha, error, stdout, stderr,
               created
             FROM actions WHERE task = ?1 ORDER BY id",
        ),
        (
            "waits",
            "SELECT id, run, kind, resource, until, target, created, fired, withdrawn,
               withdrawn_reason, withdrawn_at
             FROM waits WHERE task = ?1 ORDER BY id",
        ),
        (
            "results",
            "SELECT run, step, outcome, summary, evidence, commit_sha, reason, merge_risk, created
             FROM results WHERE task = ?1 ORDER BY run",
        ),
        (
            "subscriptions",
            "SELECT resource, created FROM subscriptions
             WHERE task = ?1 AND (SELECT state FROM tasks WHERE id = ?1) NOT IN ('failed', 'dropped')
             ORDER BY created, resource",
        ),
        (
            "inbox",
            "WITH own AS (SELECT seq FROM inbox
               WHERE task = ?1 OR id = (SELECT source FROM tasks WHERE id = ?1))
             SELECT inbox.seq, inbox.type, inbox.subject, inbox.time, inbox.task,
               CASE WHEN inbox.seq IN own THEN inbox.outcome
                 ELSE coalesce(heard.outcome, inbox.outcome) END AS outcome,
               CASE WHEN inbox.seq IN own THEN outbox.state ELSE prompt.state END AS prompt,
               (SELECT min(id) FROM waits WHERE task = ?1 AND waits.inbox = inbox.seq) AS wait
             FROM inbox LEFT JOIN outbox ON outbox.id = inbox.outbox
             LEFT JOIN subscription_deliveries AS heard ON heard.seq = inbox.seq AND heard.task = ?1
             LEFT JOIN outbox AS prompt ON prompt.id = heard.outbox
             WHERE inbox.seq IN own OR heard.task IS NOT NULL
               OR inbox.seq IN (SELECT inbox FROM waits WHERE task = ?1 AND inbox IS NOT NULL)
             ORDER BY inbox.seq DESC",
        ),
    ];
    for (name, query) in lists {
        task[name] = rows(db, query, [id])?;
    }
    task["triage"] = rows(
        db,
        "SELECT item, class, run, created FROM feedback_triage WHERE task = ?1 ORDER BY rowid",
        [id],
    )?;
    task["asks"] = asks_of(db, Some(id), None, None)?;
    task["stack"] = crate::stack::json(db, id)?;
    task["relations"] = crate::relate::json(db, id)?;
    for run in task["runs"].as_array_mut().expect("rows are an array") {
        if let Some(tokens) = run["tokens"].as_str() {
            run["tokens"] = serde_json::from_str(tokens).unwrap_or(Value::Null);
        }
    }
    let parse = |merge_risk: &mut Value| {
        if let Some(text) = merge_risk.as_str() {
            *merge_risk = serde_json::from_str(text).unwrap_or(Value::Null);
        }
    };
    parse(&mut task["merge_risk"]);
    for result in task["results"].as_array_mut().expect("rows are an array") {
        parse(&mut result["merge_risk"]);
    }
    let first = |rows: Value| rows.as_array().and_then(|rows| rows.first()).cloned();
    let request = first(rows(
        db,
        "SELECT nullif(title, '') AS title, nullif(request, '') AS request
         FROM task_text WHERE task = ?1",
        [id],
    )?)
    .unwrap_or_else(|| json!({ "title": null, "request": null }));
    let latest = |step: &str| {
        task["results"]
            .as_array()
            .and_then(|results| results.iter().rev().find(|result| result["step"] == step))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let (brief, plan) = (latest(BRIEF_STEP), latest(PLAN_STEP));
    task["brief"] = json!({
        "title": request["title"],
        "request": request["request"],
        "result": brief,
    });
    task["plan"] = plan;
    task["pull_request"] = pull_request(db, id)?.unwrap_or(Value::Null);
    parse_json(&mut task);
    Ok(Some(task))
}

/// The step whose result is a task's brief, and the one whose result is its plan.
const BRIEF_STEP: &str = "groom";
const PLAN_STEP: &str = "plan";

/// The latest pull request task `id` owns, as its stored GitHub events tell it: its title, state,
/// latest review and the latest conclusion of each check run on its latest head. The head comes
/// from the pull request's own events, since a check run on an older commit can complete later.
fn pull_request(db: &Connection, id: i64) -> rusqlite::Result<Option<Value>> {
    let found = rows(
        db,
        "WITH pr AS (SELECT substr(owns.resource, 8) AS subject FROM owns
           WHERE owns.task = ?1 AND owns.resource LIKE 'github:%#%' AND EXISTS (
             SELECT 1 FROM inbox WHERE inbox.subject = substr(owns.resource, 8)
               AND (inbox.type LIKE 'github.pull_request%' OR inbox.type LIKE 'github.check_run%'))
           ORDER BY owns.rowid DESC LIMIT 1),
         events AS (SELECT inbox.seq, inbox.type, inbox.time, inbox.data FROM inbox JOIN pr
           ON inbox.subject = pr.subject
           WHERE inbox.type LIKE 'github.pull_request%' OR inbox.type LIKE 'github.check_run%'),
         head AS (SELECT data FROM events
           ORDER BY type LIKE 'github.pull_request%' DESC, seq DESC LIMIT 1)
         SELECT json_extract(head.data, '$.repo') AS repo,
           json_extract(head.data, '$.pull_request') AS number,
           (SELECT json_extract(data, '$.title') FROM events
            WHERE json_extract(data, '$.title') IS NOT NULL ORDER BY seq DESC LIMIT 1) AS title,
           'https://github.com/' || json_extract(head.data, '$.repo') || '/pull/'
             || json_extract(head.data, '$.pull_request') AS url,
           json_extract(head.data, '$.head_branch') AS branch,
           json_extract(head.data, '$.base_branch') AS base,
           json_extract(head.data, '$.head_sha') AS head_sha,
           coalesce((SELECT CASE WHEN json_extract(data, '$.merged') THEN 'merged' ELSE 'closed' END
             FROM events WHERE type = 'github.pull_request.closed' ORDER BY seq DESC LIMIT 1),
             'open') AS state,
           (SELECT json_extract(data, '$.state') FROM events
            WHERE type = 'github.pull_request_review.submitted' ORDER BY seq DESC LIMIT 1) AS review,
           (SELECT json_group_array(json_object('name', name, 'conclusion', conclusion, 'at', time))
            FROM (SELECT json_extract(data, '$.name') AS name,
                json_extract(data, '$.conclusion') AS conclusion, time,
                row_number() OVER (PARTITION BY json_extract(data, '$.name') ORDER BY seq DESC) AS nth
              FROM events WHERE type = 'github.check_run.completed'
                AND json_extract(data, '$.head_sha') = json_extract(head.data, '$.head_sha'))
            WHERE nth = 1 ORDER BY name) AS checks
         FROM head",
        [id],
    )?;
    let Some(mut pull) = found.as_array().and_then(|rows| rows.first()).cloned() else {
        return Ok(None);
    };
    if let Some(checks) = pull["checks"].as_str() {
        pull["checks"] = serde_json::from_str(checks).unwrap_or(Value::Null);
    }
    Ok(Some(pull))
}

/// The rows of `query` as JSON objects keyed by column name.
pub(crate) fn rows(db: &Connection, query: &str, params: impl Params) -> rusqlite::Result<Value> {
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
