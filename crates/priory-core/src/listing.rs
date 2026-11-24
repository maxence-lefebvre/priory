//! The state the listing commands and the HTTP server show, one JSON payload per listing.

use crate::{post, snooze, stall, state, sweep};
use jiff::Timestamp;
use priory_factory::explain;
use priory_factory::factory::{Factory, Trigger};
use priory_intake::intake::{CHECKED, schedule};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, Params, params};
use serde_json::{Map, Value, json};
use std::io::Read;
use std::path::Path;

/// An ask's id in every listing, on the desk, in `/api/asks` and on its web view page: its name
/// when it has one, as an outside ask or a daemon's ask does, else its row.
macro_rules! ask_id {
    () => {
        "coalesce(asks.name, asks.id)"
    };
}

/// [`ask_id!`] as a constant, for `format!`.
pub(crate) const ASK_ID: &str = ask_id!();

/// The web view's page of the ask whose [`ASK_ID`] is `id`, which a notification opens.
pub(crate) fn ask_page(id: &str) -> String {
    format!("/asks/{id}")
}

/// The web view's page of task `task`, which a notification or a report's link opens.
pub(crate) fn task_page(task: i64) -> String {
    format!("/tasks/{task}")
}

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

/// The workflow and irreversible step a step ask's answer lets run, as a JSON array, or null.
const LETS: &str = "(SELECT json_array(gate.workflow, gate.lets) FROM irreversible_asks AS gate
       WHERE gate.workflow = tasks.workflow AND gate.step = asks.step AND asks.kind = 'step'
         AND gate.lets != '')";

/// Gives each ask with `lets` its `risk_estimate`, null when the trace cannot be read.
fn risk_estimates(db: &Connection, asks: &mut [Value]) {
    let lets = |ask: &mut Value| {
        let lets = ask.as_object_mut()?.remove("lets")?;
        serde_json::from_str::<(String, String)>(lets.as_str()?).ok()
    };
    let wanted: Vec<_> = asks.iter_mut().map(lets).collect();
    if wanted.iter().all(Option::is_none) {
        return;
    }
    let risk = priory_intake::intake::now()
        .and_then(|now| crate::risk::recent(db, &crate::risk::Gates::default(), now))
        .ok();
    for (ask, lets) in asks.iter_mut().zip(wanted) {
        if let Some((workflow, step)) = lets {
            ask["risk_estimate"] = risk
                .as_ref()
                .map_or(Value::Null, |risk| risk.estimate(&workflow, &step));
        }
    }
}

/// A desk item's group in the ranking (see `RANKS`), null for the queue.
fn urgency() -> String {
    format!(
        "CASE WHEN desk.part = 1 OR desk.kind = 'batch' THEN NULL
           WHEN desk.kind IN ('blocked', 'failed', 'prompt', 'command')
             OR desk.kind = 'report' AND (desk.what LIKE 'intake: %'
               OR desk.what LIKE 'priory: landed wrong: %') THEN 'alarm'
           WHEN desk.kind = 'ask' THEN (SELECT CASE WHEN {IRREVERSIBLE} THEN 'irreversible'
               WHEN tasks.key GLOB '*:review:*' THEN 'review'
               WHEN asks.kind = 'step' THEN 'plan' ELSE 'ask' END
             FROM asks LEFT JOIN tasks ON tasks.id = asks.task
             WHERE {ASK_ID} = desk.id
               AND asks.answer IS NULL AND asks.withdrawn IS NULL)
           WHEN desk.kind = 'todo' THEN 'todo'
           ELSE 'rest' END AS urgency"
    )
}

/// An item's title as a notification carries it: an ask's title, or its task and step when it
/// has none, since a question can hold a result's summary; a task's key and kind; a report's
/// source, never its line, which can quote an error.
const DESK_TITLE: &str = concat!(
    "CASE WHEN desk.kind = 'ask' THEN (SELECT coalesce(asks.title,
             'task ' || tasks.key || ' asks at ' || asks.step, 'ask ' || asks.name)
           FROM asks LEFT JOIN tasks ON tasks.id = asks.task
           WHERE ",
    ask_id!(),
    " = desk.id
             AND asks.answer IS NULL AND asks.withdrawn IS NULL)
         WHEN desk.kind = 'memories' THEN 'memory candidates wait for a person'
         WHEN desk.kind = 'pattern' THEN 'tasks stopped for one reason'
         WHEN desk.task IS NOT NULL THEN 'task '
           || (SELECT key FROM tasks WHERE tasks.id = desk.task) || ' ' || desk.kind
         WHEN desk.kind = 'report' AND desk.what LIKE 'intake: %' THEN 'the intake fails to poll '
           || substr(desk.what, 9, instr(substr(desk.what, 9), ':') - 1)
         WHEN desk.kind = 'report' AND desk.what LIKE 'priory: landed wrong: %'
           THEN 'a merge landed wrong'
         WHEN desk.kind = 'report' THEN 'a report from '
           || substr(desk.what, 1, instr(desk.what, ':') - 1)
         ELSE desk.kind || ' ' || desk.id END AS title"
);

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
         LEFT JOIN snoozes ON snoozes.item = CASE WHEN typeof(desk.id) = 'integer' THEN desk.id
             WHEN desk.kind = 'ask' THEN (SELECT id FROM asks WHERE asks.name = desk.id) END
           AND snoozes.kind =
           CASE WHEN desk.kind IN ('blocked', 'failed') THEN 'task' ELSE desk.kind END
         ORDER BY desk.part, CASE WHEN desk.part = 0 THEN desk.since END, desk.kind, desk.id",
        places(),
        snooze::FIELDS,
        urgency()
    )
}

/// No open `parked` hold holds the task: adopted tasks stay out of the lists until released.
const UNPARKED: &str = "NOT EXISTS (SELECT 1 FROM holds AS parked WHERE parked.task = tasks.id
  AND parked.kind = 'parked' AND parked.released IS NULL)";

const DESK: &str = concat!(
    "SELECT 0 AS part, asks.created AS since, 'ask' AS kind,
                 ",
    ask_id!(),
    " AS id, asks.task AS task, tasks.lane AS lane,
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
                 AND hold.released IS NULL) AND holds.kind != 'parked'
                 AND (tasks.batch_entry IS NULL OR EXISTS (SELECT 1 FROM holds AS hold
                   WHERE hold.task = holds.task AND hold.released IS NULL
                   AND (hold.kind NOT IN ('merged', 'waiting') OR EXISTS (SELECT 1 FROM asks WHERE asks.id = hold.ask
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
               SELECT 0, min(written), 'memories', 'memories', NULL, NULL,
                 count(*) || CASE count(*) WHEN 1 THEN ' memory candidate waits'
                   ELSE ' memory candidates wait' END || ' for a person: '
                   || group_concat(name, ', ') || '; see them with `priory memories`'
               FROM memory_candidates
               WHERE logged IS NOT NULL AND accepted IS NULL AND dropped IS NULL
               HAVING count(*) > 0
               UNION ALL
               SELECT 0, last, 'pattern', pattern, NULL, NULL,
                 workflow || '/' || step || ': ' || tasks || ' tasks stopped for one reason: ' || text
                   || '; see it with `priory patterns show ' || pattern || '`'
               FROM (SELECT json_extract(event, '$.data.pattern') AS pattern,
                   max(julianday(json_extract(event, '$.data.happened'))),
                   json_extract(event, '$.data.happened') AS last,
                   json_extract(event, '$.data.workflow') AS workflow,
                   json_extract(event, '$.data.step') AS step,
                   json_extract(event, '$.data.text') AS text,
                   count(DISTINCT json_extract(event, '$.data.task')) AS tasks
                 FROM trace WHERE type = 'factory.pattern.matched' GROUP BY 1) AS patterns
               WHERE tasks >= 2 AND NOT EXISTS (SELECT 1 FROM trace AS closed
                 WHERE closed.type = 'factory.pattern.closed'
                   AND json_extract(closed.event, '$.data.pattern') = patterns.pattern
                   AND (json_extract(closed.event, '$.data.dismissed')
                     OR closed.seq = (SELECT max(seq) FROM trace AS latest
                       WHERE latest.type = 'factory.pattern.closed'
                         AND json_extract(latest.event, '$.data.pattern') = patterns.pattern)
                       AND NOT EXISTS (SELECT 1 FROM trace AS later
                         WHERE later.type = 'factory.pattern.matched'
                           AND json_extract(later.event, '$.data.pattern') = patterns.pattern
                           AND later.seq > closed.seq
                           AND unixepoch(json_extract(later.event, '$.data.happened'))
                             >= unixepoch(closed.time))))
               UNION ALL
               SELECT 1, tasks.changed, 'queue', queue.place, tasks.id, tasks.lane,
                 coalesce(tasks.band, '-') || coalesce(': ' || tasks.reason, '')
               FROM tasks JOIN queue USING (id)"
);

fn asks_query() -> String {
    format!(
        "SELECT {ASK_ID} AS id, asks.task,
     coalesce(tasks.key, asks.key) AS key, asks.step, tasks.lane, tasks.band, tasks.repo, asks.run,
     CASE WHEN asks.withdrawn IS NOT NULL THEN 'withdrawn'
       WHEN asks.answer IS NOT NULL THEN 'answered' ELSE 'open' END AS state,
     asks.withdrawn_reason, asks.withdrawn_at,
     CASE asks.withdrawn_reason WHEN 'sibling-fired' THEN 'a wait of its run fired first'
       WHEN 'moved-on' THEN 'a later run called another tool'
       WHEN 'revived' THEN 'its task was revived'
       WHEN 'predecessor-revived' THEN 'the task it waits on was revived'
       WHEN 'dismissed' THEN 'dismissed' || coalesce(' by session ' || CASE WHEN asks.answered_by LIKE 'session:%'
         THEN substr(asks.answered_by, 9) END, '')
       ELSE asks.withdrawn_reason END AS withdrawn_why,
     asks.kind, asks.revision, asks.question, asks.options, asks.recommendation, asks.tradeoff, asks.answer,
     asks.answered AS answered_at, asks.answered_by, asks.routed AS routed_to, asks.created AS since, asks.title, asks.text,
     asks.confidence, asks.rationale,
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
     {IRREVERSIBLE} AS irreversible, {LETS} AS lets, tasks.merge_risk, tasks.grade,
     asks.kind != 'step' AND NOT EXISTS (SELECT 1 FROM holds
       WHERE holds.ask = asks.id AND holds.released IS NULL) AS free_text, {}
   FROM asks LEFT JOIN tasks ON tasks.id = asks.task
   LEFT JOIN snoozes ON snoozes.kind = 'ask' AND snoozes.item = asks.id
   WHERE (?1 IS NULL OR asks.task = ?1
       OR (asks.answer IS NULL AND asks.withdrawn IS NULL
         AND asks.id IN (SELECT ask FROM gates WHERE gates.task = ?1 AND gates.state = 'grading')))
     AND (?2 IS NULL OR asks.id = ?2)
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
           CASE type WHEN ?2 THEN json_extract(data, '$.line')
             WHEN ?3 THEN 'dropped by ' || json_extract(data, '$.script')
               || coalesce(': ' || coalesce(json_extract(data, '$.reason'),
                 json_extract(data, '$.failed')), '') END AS line, read
         FROM inbox WHERE seq > ?1 ORDER BY seq",
        params![since, post::REPORT, priory_intake::pre::DROPPED],
    )
}

/// The events after sequence number `since` that reached no task and changed nothing. `owned`
/// groups them by the resource each names first, newest first, when a task owned or subscribed to
/// it: with that task and the command that routes the next event. `other` counts the rest per
/// source, since no task ever owned them.
/// `unheard` groups by task what a task kept and no agent read: see `unheard_events`.
/// `outsider` lists the comments and reviews from outside the repository, newest first.
pub fn unrouted(db: &Connection, since: i64, now: Timestamp) -> rusqlite::Result<Value> {
    let mut owned: Vec<(String, Value, Vec<Value>)> = Vec::new();
    let mut other: Vec<(String, usize)> = Vec::new();
    let dropped = dropped_requests(db, since)?;
    let sources: Vec<&Value> = dropped.iter().map(|drop| &drop["event"]["seq"]).collect();
    for (resource, event) in unrouted_events(db, since, None)? {
        if let Some((_, _, events)) = owned.iter_mut().find(|(named, ..)| *named == resource) {
            events.push(event);
            continue;
        }
        if let Some(task) = owner(db, &resource)? {
            owned.push((resource, task, vec![event]));
            continue;
        }
        // A trigger wanted it: `dropped` lists it with the script's reason.
        if sources.contains(&&event["seq"]) {
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
    let mut unheard: Vec<(Value, bool, Vec<Value>)> = Vec::new();
    for (task, asking, event) in unheard_events(db, since, None, now)? {
        match unheard
            .iter_mut()
            .find(|(named, _, _)| named["id"] == task["id"])
        {
            Some((_, _, events)) => events.push(event),
            None => unheard.push((task, asking, vec![event])),
        }
    }
    let unheard: Vec<Value> = unheard
        .into_iter()
        .map(|(task, asking, events)| {
            let state = task["state"].as_str().unwrap_or_default();
            // A task at a step a person decides shows its events on that step's ask.
            let say_text =
                (["asleep", "blocked"].contains(&state) && !asking).then_some(UNHEARD_SAY);
            let request_text = (state == "done")
                .then(|| request_text(task["key"].as_str().unwrap_or_default(), &events))
                .flatten();
            let follow_up = unheard_follow_up(&task, say_text, request_text.as_deref());
            json!({
                "task": task, "events": events, "follow_up": follow_up,
                "say_text": say_text, "request_text": request_text,
            })
        })
        .collect();
    Ok(json!({
        "unrouted": { "owned": owned, "dropped": dropped, "other": other, "unheard": unheard },
        "outsider": outsider_events(db, since, None)?,
    }))
}

/// How long a kept event waits for an agent before `unheard_events` lists it: a few polls, so a
/// task about to start or wake is not listed.
const UNHEARD_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_mins(5);

/// Each event after `since`, and within `window` (from, to) when given, newest first, that a task
/// that is not running kept, itself or through a subscription, that no prompt carries and no wait took, written more than
/// `UNHEARD_AFTER` before `now` by someone other than the agent, with its task. Only what people
/// say, edit and close counts: checks and pushes stay with the checks gate, and an item a decision
/// model settled waits for the task's next prompt by design. Each comes with whether
/// its task waits on a step's ask.
fn unheard_events(
    db: &Connection,
    since: i64,
    window: Option<(&str, &str)>,
    now: Timestamp,
) -> rusqlite::Result<Vec<(Value, bool, Value)>> {
    let kinds = state::OWN_KEY
        .iter()
        .chain([&priory_intake::request::REQUESTED])
        .collect::<Vec<_>>();
    let kinds = serde_json::to_string(&kinds).expect("kinds serialize");
    let before = (now - UNHEARD_AFTER).to_string();
    let (from, to) = window.unzip();
    let mut statement = db.prepare(
        "WITH kept (seq, task) AS (
           SELECT seq, task FROM inbox WHERE outcome = 'kept' AND outbox IS NULL
           UNION ALL SELECT seq, task FROM subscription_deliveries
             WHERE outcome = 'kept' AND outbox IS NULL)
         SELECT inbox.seq, inbox.type, inbox.subject, inbox.time, inbox.data,
           tasks.id, tasks.key, tasks.state, tasks.workflow,
           EXISTS (SELECT 1 FROM asks WHERE asks.task = tasks.id AND kind = 'step'
             AND answer IS NULL AND withdrawn IS NULL)
         FROM kept JOIN inbox ON inbox.seq = kept.seq JOIN tasks ON tasks.id = kept.task
         WHERE tasks.state != 'running'
           AND inbox.seq > ?1
           AND (?2 IS NULL OR julianday(inbox.time) BETWEEN julianday(?2) AND julianday(?3))
           AND inbox.type IN (SELECT value FROM json_each(?4))
           AND julianday(inbox.time) <= julianday(?5)
           AND coalesce(json_extract(inbox.data, '$.own'), 0) != 1
           AND coalesce(json_extract(inbox.data, '$.decided.class'), '') != 'nothing-to-do'
           AND inbox.seq NOT IN (SELECT inbox FROM waits WHERE task = kept.task
             AND inbox IS NOT NULL)
         ORDER BY inbox.seq DESC",
    )?;
    let mut rows = statement.query(params![since, from, to, kinds, before])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let (_, event) = event_json(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            &row.get::<_, String>(4)?,
        );
        let task = json!({
            "id": row.get::<_, i64>(5)?, "key": row.get::<_, String>(6)?,
            "state": row.get::<_, String>(7)?, "workflow": row.get::<_, String>(8)?,
        });
        events.push((task, row.get(9)?, event));
    }
    Ok(events)
}

/// What `priory say` tells an asleep or blocked task that kept unheard events.
const UNHEARD_SAY: &str = "Read the events that came while you slept.";

/// The command that gets a task's unheard events to an agent: `say` with `say_text`, which wakes an
/// asleep task and reaches a blocked one when it resumes; for an ended task, what an unrouted event
/// of it would get. None for a task whose next run reads them anyway.
fn unheard_follow_up(
    task: &Value,
    say_text: Option<&str>,
    request_text: Option<&str>,
) -> Option<String> {
    if let Some(text) = say_text {
        return Some(format!("priory say {} {}", task["id"], quoted(text)));
    }
    match task["state"].as_str().unwrap_or_default() {
        "done" | "failed" | "dropped" => {
            follow_up(task, task["key"].as_str().unwrap_or_default(), request_text)
        }
        _ => None,
    }
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
        let data: String = row.get(4)?;
        let (resource, event) =
            event_json(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, &data);
        let Some(resource) = resource else {
            continue;
        };
        events.push((resource, event));
    }
    Ok(events)
}

/// Each request a trigger's `pre` script dropped after `since`, newest first, with its source event,
/// unless a task has its key since. A view entry is judged once, so nothing else lists it.
fn dropped_requests(db: &Connection, since: i64) -> rusqlite::Result<Vec<Value>> {
    let mut statement = db.prepare(
        "SELECT dropped.seq, dropped.time, dropped.data,
           source.seq, source.type, source.subject, source.time, source.data
         FROM inbox AS dropped
         LEFT JOIN inbox AS source ON source.id = json_extract(dropped.data, '$.source')
         WHERE dropped.type = ?1 AND dropped.seq > ?2
           AND NOT EXISTS (SELECT 1 FROM tasks WHERE tasks.key = json_extract(dropped.data, '$.key'))
         ORDER BY dropped.seq DESC",
    )?;
    let mut rows = statement.query(params![priory_intake::pre::DROPPED, since])?;
    let mut dropped = Vec::new();
    while let Some(row) = rows.next()? {
        let data: Value = serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or(Value::Null);
        let event = match row.get::<_, Option<i64>>(3)? {
            Some(seq) => {
                let data: String = row.get(7)?;
                event_json(seq, row.get(4)?, row.get(5)?, row.get(6)?, &data).1
            }
            None => Value::Null,
        };
        dropped.push(json!({
            "seq": row.get::<_, i64>(0)?, "time": row.get::<_, String>(1)?,
            "trigger": data["trigger"], "script": data["script"], "reason": data["reason"],
            "failed": data["failed"], "key": data["key"], "event": event,
        }));
    }
    Ok(dropped)
}

/// An inbox entry as the unrouted listing shows it, with the first resource it names.
fn event_json(
    seq: i64,
    kind: String,
    subject: Option<String>,
    time: String,
    data: &str,
) -> (Option<String>, Value) {
    let mut message = herdr_bus::Message::new(
        kind.as_str(),
        serde_json::from_str(data).unwrap_or(Value::Null),
    );
    message.subject = subject.clone();
    let resource = crate::route::resources(&message).into_iter().next();
    let said = ["body", "text", "title"]
        .iter()
        .find_map(|field| message.data[field].as_str())
        .and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
        .map(str::to_owned)
        .or_else(|| checked(&kind, &message.data));
    let event = json!({
        "seq": seq, "type": kind, "subject": subject,
        "time": time, "line": said, "url": message.data["url"].as_str(),
    });
    (resource, event)
}

/// A check run's name and conclusion, `test (ubuntu) failure`, as the line of its event.
fn checked(kind: &str, data: &Value) -> Option<String> {
    let name = data["name"].as_str().filter(|_| kind == CHECKED)?;
    Some(format!("{name} {}", data["conclusion"].as_str()?))
}

/// Every task in start order but parked ones, or every task with `all`, or the tasks of one lane,
/// of one grade, or both.
pub fn tasks(
    db: &Connection,
    lane: Option<&str>,
    grade: Option<&str>,
    all: bool,
) -> rusqlite::Result<Value> {
    tasks_since(db, lane, grade, None, all)
}

/// The tasks still going and those that ended since `since`, or every task without it, in start
/// order, as `tasks`, with the number of tasks left out as `older`.
pub fn recent_tasks(db: &Connection, since: Option<&str>) -> rusqlite::Result<Value> {
    let older: i64 = db.query_row(
        "SELECT count(*) FROM tasks WHERE state IN ('done', 'failed', 'dropped') AND changed < ?1",
        [since],
        |row| row.get(0),
    )?;
    Ok(json!({ "tasks": tasks_since(db, None, None, since, false)?, "older": older }))
}

/// The runs live or started since `since`, or every run without it, newest first, each with its
/// task, its result's outcome and the model and tokens its `factory.step.ended` event recorded,
/// as `runs`, with the number of runs left out as `older`.
pub fn recent_runs(db: &Connection, since: Option<&str>) -> rusqlite::Result<Value> {
    let older: i64 = db.query_row(
        "SELECT count(*) FROM runs WHERE ended IS NOT NULL AND started < ?1",
        [since],
        |row| row.get(0),
    )?;
    let mut runs = rows(
        db,
        "WITH ended AS MATERIALIZED (SELECT json_extract(event, '$.data.run') AS run, max(seq),
           json_extract(event, '$.data.model') AS model, json_extract(event, '$.data.tokens') AS tokens
           FROM trace WHERE type = 'factory.step.ended' AND (?1 IS NULL OR time >= ?1) GROUP BY 1)
         SELECT runs.id AS run, runs.task, tasks.key, runs.step, runs.agent, runs.started,
           runs.ended, runs.ended_as AS end, results.outcome, ended.model, ended.tokens
         FROM runs JOIN tasks ON tasks.id = runs.task
         LEFT JOIN results ON results.run = runs.id
         LEFT JOIN ended ON ended.run = CAST(runs.id AS TEXT)
         WHERE ?1 IS NULL OR runs.ended IS NULL OR runs.started >= ?1
         ORDER BY runs.id DESC",
        [since],
    )?;
    for run in runs.as_array_mut().expect("rows are an array") {
        if let Some(tokens) = run["tokens"].as_str() {
            run["tokens"] = serde_json::from_str(tokens).unwrap_or(Value::Null);
        }
    }
    Ok(json!({ "runs": runs, "older": older }))
}

fn tasks_since(
    db: &Connection,
    lane: Option<&str>,
    grade: Option<&str>,
    since: Option<&str>,
    all: bool,
) -> rusqlite::Result<Value> {
    let query = format!(
        "{} SELECT id, key, workflow, step, state, band, lane, reason, queue.place AS position,
           generation, retired, tldr, changed AS since, grade, grade_reason, progress, {CONDITIONS},
           {TITLE}, batch, added_by
         FROM tasks LEFT JOIN queue USING (id)
         WHERE (?1 IS NULL OR lane = ?1) AND (?2 IS NULL OR grade = ?2)
           AND (?3 IS NULL OR state NOT IN ('done', 'failed', 'dropped') OR changed >= ?3)
           AND (?4 OR {UNPARKED})
         ORDER BY {}",
        places(),
        state::START_ORDER
    );
    let mut tasks = rows(db, &query, params![lane, grade, since, all])?;
    for task in tasks.as_array_mut().expect("rows are an array") {
        parse_json(task);
        let id = task["id"].as_i64().expect("a task has an id");
        task["stack"] = crate::stack::json(db, id)?;
        task["relations"] = crate::relate::json(db, id)?;
    }
    // Another runner's task carries no lane or grade of its own, so a filter keeps this runner's.
    if lane.is_none() && grade.is_none() {
        crate::runners::with_remote_tasks(db, &mut tasks)?;
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
    for field in ["progress", "conditions", "waits", "stops", "protected"] {
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
    crate::runners::with_remote_asks(db, &mut asks, since)?;
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
    if let Some(asks) = asks.as_array_mut() {
        risk_estimates(db, asks);
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
        "{} SELECT position, band_position, id, key, title, workflow, step, state, band, aged, lane,
           reason, since, conditions, stops, protected FROM (
           SELECT 0 AS part, queue.place AS position, queue.band_place AS band_position, id, key,
             {TITLE}, workflow, step, state, band, {aged} AS aged, lane, reason, changed AS since,
             NULL AS conditions,
             (SELECT json_group_array(runs.task) FROM runs
               WHERE runs.preempted_for = tasks.id AND runs.ended IS NULL) AS stops,
             coalesce(tasks.protected, '[]') AS protected,
             queue.place AS sort
           FROM tasks JOIN queue USING (id)
           UNION ALL
           SELECT 1, NULL, NULL, tasks.id, key, {TITLE}, workflow, step, state, band, NULL, lane,
             tasks.reason,
             changed, {CONDITIONS}, '[]', '[]',
             (SELECT min(id) FROM holds WHERE holds.task = tasks.id AND holds.released IS NULL)
           FROM tasks WHERE tasks.state = 'held' AND {UNPARKED}
         ) ORDER BY part, sort",
        places(),
        aged = state::AGED_BAND
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

/// Gives each irreversible step of `flows` its `risk_estimate` from `risk`, or null when no state
/// was read.
pub fn with_risk_estimates(flows: &mut Value, risk: Option<&crate::risk::Risk>) {
    for step in flows["irreversible"].as_array_mut().into_iter().flatten() {
        step["risk_estimate"] = match (risk, step["workflow"].as_str(), step["step"].as_str()) {
            (Some(risk), Some(workflow), Some(id)) => risk.step_history(workflow, id),
            _ => Value::Null,
        };
    }
}

/// What `priory check --explain --json` prints and `GET /api/flows` serves, before
/// `with_risk_estimates`: the irreversible steps with their gates, the paths to one with no human
/// decision, the tools reached, and, for the drawing, each workflow's steps and edges.
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
            // A queued task's reason, such as waiting for room or its claim, says why it has not started.
            let why = match (
                row["reason"].as_str(),
                position,
                row["band_position"].as_i64(),
            ) {
                (Some(reason), _, _) => reason.to_owned(),
                (None, 1, _) => "first in the queue".to_owned(),
                (None, _, Some(1)) => "a band above it".to_owned(),
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
               CASE type WHEN 'factory.ask.auto_answered' THEN 'ask ' || json_extract(event, '$.data.ask')
                   || ' answered ' || json_quote(json_extract(event, '$.data.option')) || ' by auto ('
                   || json_extract(event, '$.data.confidence') || '): '
                   || json_extract(event, '$.data.rationale')
                 WHEN 'factory.ask.session_answered' THEN 'ask ' || json_extract(event, '$.data.ask')
                   || ' answered ' || json_quote(json_extract(event, '$.data.option'))
                   || ' by session ' || json_extract(event, '$.data.session')
                 WHEN 'factory.ask.session_dismissed' THEN 'ask ' || json_extract(event, '$.data.ask')
                   || ' dismissed by session ' || json_extract(event, '$.data.session')
                 ELSE type || coalesce(' ' || json_extract(event, '$.data.step'), '')
                   || coalesce(': ' || json_extract(event, '$.data.outcome'), '') END AS what,
               0 AS source, seq
             FROM trace WHERE julianday(time) >= (SELECT at FROM visited)
             ORDER BY julianday(time) DESC, seq DESC LIMIT 50)
           UNION ALL
           SELECT * FROM (SELECT time, task,
               CASE type WHEN ?2 THEN subject || ': ' || json_extract(data, '$.line')
                 ELSE type || coalesce(' ' || subject, '') || coalesce(': ' || outcome, '')
                   || CASE type WHEN ?3 THEN coalesce(', added by session '
                     || json_extract(data, '$.session'), '') ELSE '' END END,
               1, seq
             FROM inbox WHERE julianday(time) >= (SELECT at FROM visited)
             -- An event no task took would crowd out the rest in a busy repository.
             AND outcome IS NOT 'ignored'
             ORDER BY julianday(time) DESC, seq DESC LIMIT 50))
         SELECT time, task, (SELECT key FROM tasks WHERE tasks.id = events.task) AS key, what
         FROM events
         ORDER BY julianday(time) DESC, source DESC, seq DESC LIMIT 50",
        params![person, post::REPORT, crate::route::TASK_REQUESTED],
    )
}

/// Why an alarm ranks first.
pub(crate) const ALARM: &str = "work stopped and waits on a person";

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
    let mut asked = db.prepare(&format!(
        "SELECT asks.command, {LETS}
             FROM asks LEFT JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1"
    ))?;
    for item in items.iter_mut().filter(|item| item["kind"] == "ask") {
        let (command, lets) = match item["id"].as_i64() {
            Some(ask) => asked.query_row([ask], |row| Ok((row.get(0)?, row.get(1)?)))?,
            None => (None::<String>, None::<String>),
        };
        item["lets"] = json!(lets);
        let lets = lets
            .and_then(|lets| serde_json::from_str::<(String, String)>(&lets).ok())
            .map(|(_, step)| step);
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

/// Gives each `ask` item the lead it is routed to, and the confidence and rationale its agent
/// stated, each or null.
fn routed_to(db: &Connection, items: &mut [Value]) -> rusqlite::Result<()> {
    let mut routed = db.prepare("SELECT routed, confidence, rationale FROM asks WHERE id = ?1")?;
    for item in items.iter_mut().filter(|item| item["kind"] == "ask") {
        let [to, confidence, rationale] = match item["id"].as_i64() {
            Some(ask) => routed.query_row([ask], |row| {
                Ok([row.get::<_, Option<String>>(0)?, row.get(1)?, row.get(2)?])
            })?,
            None => [None, None, None],
        };
        item["routed_to"] = json!(to);
        item["confidence"] = json!(confidence);
        item["rationale"] = json!(rationale);
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
    risk_estimates(db, list);
    if let Some(me) = crate::runners::me(db)? {
        for item in list.iter_mut().filter(|item| item["kind"] == "ask") {
            item["runner"] = json!(me);
        }
        // Another runner's asks join what waits on a person, before the queue, oldest first.
        let remote = crate::runners::remote_desk_items(db)?;
        let waiting = list
            .iter()
            .take_while(|item| !item["urgency"].is_null())
            .count();
        let added = remote.len();
        list.splice(waiting..waiting, remote);
        list[..waiting + added].sort_by(|a, b| a["since"].as_str().cmp(&b["since"].as_str()));
    }
    for item in list.iter_mut().filter(|item| item["kind"] == "memories") {
        item["candidates"] = json!(crate::memories::list(db, false)?);
    }
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
    let unheard = unheard_events(db, 0, Some((day.0, &day.1)), now)?.len();
    let dropped = dropped_requests(db, 0)?
        .iter()
        .filter(|drop| drop["time"].as_str() >= Some(day.0))
        .count();
    let outside = [
        crate::link::banner(db)?,
        post::intake_banner(db)?,
        crate::kit::banner(db)?,
        sweep::fetch_banner(home),
    ]
    .into_iter()
    .flatten()
    .reduce(|first, next| format!("{first}; {next}"));
    let outsider = outsider_events(db, 0, Some((day.0, &day.1)))?
        .as_array()
        .map_or(0, Vec::len);
    Ok(json!({
        "paused": state::pause_banner(db)?,
        "caps": crate::spend::desk_lines(db)?,
        "reviews": crate::review::desk_lines(db)?,
        "banner": stall::banner(db)?,
        "intake": outside,
        "counts": {
            "asleep": asleep, "snoozed": snoozed, "unrouted": unrouted, "unheard": unheard,
            "dropped": dropped, "outsider": outsider,
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
/// `plan_file` is the plan document in the task's worktree under `home`, read now.
pub fn task(db: &Connection, home: &Path, id: i64) -> rusqlite::Result<Option<Value>> {
    let found = rows(
        db,
        &format!(
            "{} SELECT id, key, workflow, step, state, band, lane, reason, queue.place AS position,
               generation, retired, tldr, changed AS since, repo, branch, base_sha, grade, grade_reason,
               merge_risk, progress, plan_goal, plan_testing, plan_brief, verification, intervention, lead, auto_answer, batch, added_by,
               intent, noop, {CONDITIONS}
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
        (
            "scores",
            "SELECT run, step, scorer, kind, verdict, reason, time FROM (
               SELECT json_extract(event, '$.data.run') AS run,
                 json_extract(event, '$.data.step') AS step,
                 json_extract(event, '$.data.scorer') AS scorer,
                 json_extract(event, '$.data.kind') AS kind,
                 json_extract(event, '$.data.verdict') AS verdict,
                 json_extract(event, '$.data.reason') AS reason, time,
                 row_number() OVER (PARTITION BY json_extract(event, '$.data.run'),
                   json_extract(event, '$.data.scorer') ORDER BY seq DESC) AS latest
               FROM trace WHERE type = 'factory.run.scored'
                 AND json_extract(event, '$.data.task') = CAST(?1 AS TEXT))
             WHERE latest = 1 ORDER BY CAST(run AS INTEGER), scorer",
        ),
        (
            "held_scores",
            "SELECT json_extract(event, '$.data.scorer') AS scorer,
               json_extract(event, '$.data.verdict') AS verdict,
               json_extract(event, '$.data.evidence') AS reason,
               json_extract(event, '$.data.happened') AS happened, time
             FROM trace WHERE type = 'factory.step.labeled'
               AND json_extract(event, '$.data.task') = CAST(?1 AS TEXT)
               AND json_extract(event, '$.data.label') = 'scored'
             ORDER BY seq",
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
    task["consults"] = Value::from(crate::consult::of_task(db, id)?);
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
    crate::evidence::attach(db, id, &mut task["results"])?;
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
    let brief = latest(BRIEF_STEP);
    let plan = match latest(PLAN_STEP) {
        Value::Null => task["results"]
            .as_array()
            .and_then(|results| {
                results.iter().rev().find(|result| {
                    result["step"]
                        .as_str()
                        .is_some_and(|step| step.starts_with(PLAN_PREFIX))
                })
            })
            .cloned()
            .unwrap_or(Value::Null),
        plan => plan,
    };
    let noop = task
        .as_object_mut()
        .and_then(|fields| fields.remove("noop"))
        .and_then(|noop| serde_json::from_str::<Value>(noop.as_str()?).ok())
        .unwrap_or_else(|| json!([]));
    let intent = task
        .as_object_mut()
        .and_then(|fields| fields.remove("intent"))
        .unwrap_or(Value::Null);
    task["brief"] = json!({
        "title": request["title"],
        "request": request["request"],
        "result": brief,
        "intent": intent,
        "noop": noop,
    });
    task["plan"] = plan;
    task["plan_file"] = plan_file(home, &task).unwrap_or(Value::Null);
    task["pull_request"] = pull_request(db, id)?.unwrap_or(Value::Null);
    parse_json(&mut task);
    Ok(Some(task))
}

/// The step whose result is a task's brief, and the one whose result is its plan. A factory may
/// name its plan step `plan-<something>`, such as `plan-implementation`: with no `plan` result,
/// the latest result of a step starting with `PLAN_PREFIX` is the plan. The workflow marks no
/// step as the plan, and this reads only the database, so the rule goes by the step's name.
const BRIEF_STEP: &str = "groom";
const PLAN_STEP: &str = "plan";
const PLAN_PREFIX: &str = "plan-";

/// The plan documents an agent writes in its worktree, first found wins.
const PLAN_FILES: [&str; 2] = [".priory/plan.md", ".priory/brief.md"];
/// The most bytes of a plan document a task shows.
const PLAN_FILE_CAP: usize = 64 * 1024;

/// The task's plan document as `{ path, text, truncated }`: the first of `PLAN_FILES` in its
/// worktree, `home/worktrees/<repo>/<branch without priory/>` as `runtime::worktree` names it.
/// A file whose real path leaves the worktree, such as a symlink, is skipped.
fn plan_file(home: &Path, task: &Value) -> Option<Value> {
    let name = task["branch"].as_str()?.strip_prefix("priory/")?;
    let worktree = home
        .join("worktrees")
        .join(task["repo"].as_str()?)
        .join(name)
        .canonicalize()
        .ok()?;
    PLAN_FILES.iter().find_map(|file| {
        let path = worktree.join(file).canonicalize().ok()?;
        if !path.starts_with(&worktree) || !path.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .ok()?
            .take(PLAN_FILE_CAP as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        let truncated = bytes.len() > PLAN_FILE_CAP;
        bytes.truncate(PLAN_FILE_CAP);
        let text = String::from_utf8_lossy(&bytes);
        // A cut inside a character leaves one replacement character at the end; drop it.
        let text = if truncated {
            text.trim_end_matches('\u{FFFD}')
        } else {
            &text
        };
        Some(json!({ "path": path.to_string_lossy(), "text": text, "truncated": truncated }))
    })
}

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
           ORDER BY type LIKE 'github.pull_request%' DESC, seq DESC LIMIT 1),
         -- An event can come without the branches, so each takes the newest event that has it.
         known AS (SELECT
           (SELECT json_extract(data, '$.head_branch') FROM events
            WHERE json_extract(data, '$.head_branch') IS NOT NULL
            ORDER BY type LIKE 'github.pull_request%' DESC, seq DESC LIMIT 1) AS branch,
           (SELECT json_extract(data, '$.base_branch') FROM events
            WHERE json_extract(data, '$.base_branch') IS NOT NULL
            ORDER BY type LIKE 'github.pull_request%' DESC, seq DESC LIMIT 1) AS base,
           (SELECT json_extract(data, '$.head_sha') FROM events
            WHERE json_extract(data, '$.head_sha') IS NOT NULL
            ORDER BY type LIKE 'github.pull_request%' DESC, seq DESC LIMIT 1) AS head_sha)
         SELECT json_extract(head.data, '$.repo') AS repo,
           json_extract(head.data, '$.pull_request') AS number,
           (SELECT json_extract(data, '$.title') FROM events
            WHERE json_extract(data, '$.title') IS NOT NULL ORDER BY seq DESC LIMIT 1) AS title,
           'https://github.com/' || json_extract(head.data, '$.repo') || '/pull/'
             || json_extract(head.data, '$.pull_request') AS url,
           known.branch, known.base, known.head_sha,
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
                AND json_extract(data, '$.head_sha') = known.head_sha)
            WHERE nth = 1 ORDER BY name) AS checks
         FROM head, known",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A home holding task 1's worktree, `worktrees/acme/shop/7`, and a state where task 1 has
    /// that branch and no plan result.
    fn setup(name: &str) -> (PathBuf, PathBuf, Connection) {
        let home =
            std::env::temp_dir().join(format!("priory-plan-file-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let worktree = home.join("worktrees/acme/shop/7");
        fs::create_dir_all(worktree.join(".priory")).unwrap();
        let db = state::memory();
        db.execute(
            "INSERT INTO tasks
               (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered,
                repo, branch)
             VALUES ('cli:7', 'groom', 'approve-plan', 'asleep', 'now', 'cli', 0, 'cli', '7',
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'acme/shop', 'priory/7')",
            [],
        )
        .unwrap();
        (home, worktree, db)
    }

    fn plan_file_of(db: &Connection, home: &Path) -> Value {
        task(db, home, 1).unwrap().unwrap()["plan_file"].clone()
    }

    #[test]
    fn plan_file_is_the_brief_when_no_plan_step_ran() {
        let (home, worktree, db) = setup("brief");
        fs::write(
            worktree.join(".priory/brief.md"),
            "# Decisions\n\nD1: cursor.\n",
        )
        .unwrap();
        let file = plan_file_of(&db, &home);
        assert_eq!(file["text"], "# Decisions\n\nD1: cursor.\n");
        assert_eq!(file["truncated"], false);
        let path = worktree.canonicalize().unwrap().join(".priory/brief.md");
        assert_eq!(file["path"], path.to_str().unwrap());
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn plan_file_prefers_plan_md_over_brief_md() {
        let (home, worktree, db) = setup("prefers");
        fs::write(worktree.join(".priory/brief.md"), "the brief").unwrap();
        fs::write(worktree.join(".priory/plan.md"), "the plan").unwrap();
        assert_eq!(plan_file_of(&db, &home)["text"], "the plan");
        fs::remove_dir_all(&home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn plan_file_skips_a_symlink_out_of_the_worktree() {
        let (home, worktree, db) = setup("symlink");
        fs::write(home.join("secret.md"), "outside").unwrap();
        std::os::unix::fs::symlink(home.join("secret.md"), worktree.join(".priory/plan.md"))
            .unwrap();
        assert_eq!(plan_file_of(&db, &home), Value::Null);
        fs::write(worktree.join(".priory/brief.md"), "inside").unwrap();
        assert_eq!(plan_file_of(&db, &home)["text"], "inside");
        fs::remove_dir_all(worktree.join(".priory")).unwrap();
        std::os::unix::fs::symlink(&home, worktree.join(".priory")).unwrap();
        fs::write(home.join("plan.md"), "outside").unwrap();
        assert_eq!(plan_file_of(&db, &home), Value::Null);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn plan_file_past_the_cap_is_cut_at_a_character() {
        let (home, worktree, db) = setup("cap");
        // A 3-byte character straddles the cap.
        let text = format!("{}€{}", "a".repeat(PLAN_FILE_CAP - 1), "b".repeat(10));
        fs::write(worktree.join(".priory/plan.md"), text).unwrap();
        let file = plan_file_of(&db, &home);
        assert_eq!(file["truncated"], true);
        assert_eq!(file["text"], "a".repeat(PLAN_FILE_CAP - 1));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn plan_file_is_null_without_a_worktree() {
        let (home, _, db) = setup("none");
        assert_eq!(plan_file_of(&db, &home), Value::Null);
        fs::remove_dir_all(&home).unwrap();
    }
}
