//! Batches: a planning task turns many issues into ordered tasks under one go.

use priory_factory::factory::Factory;
use priory_intake::events::BatchRequest;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The plan a planning task's result sends in `batch`, as the daemon stores it.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    entries: Vec<Entry>,
    #[serde(default)]
    close: Vec<Close>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    splits: Vec<Value>,
}

/// One task to be: an issue, its brief, and the entries it starts after.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    issue: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<i64>,
    title: String,
    brief: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    after: Vec<Edge>,
}

/// `ENTRY after OTHER at MOMENT`.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Edge {
    entry: String,
    moment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    why: Option<String>,
}

/// An issue the plan closes as already done.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Close {
    issue: String,
    why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence: Option<String>,
}

/// Records the batch of the planning task `task`, keyed `key`, that a `priory batch add` request
/// created.
pub(crate) fn created(
    tx: &Transaction,
    task: i64,
    key: &str,
    batch: &BatchRequest,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO batches (key, task, repo, band, lane, issues, entries_workflow)
         SELECT ?1, id, repo, band, lane, ?3, ?4 FROM tasks WHERE id = ?2",
        params![
            key,
            task,
            serde_json::to_string(&batch.issues).expect("issues serialize"),
            batch.entries
        ],
    )?;
    tx.execute(
        "UPDATE tasks SET batch = ?2 WHERE id = ?1",
        params![task, tx.last_insert_rowid()],
    )
    .map(drop)
}

/// An issue as the batch compares it: `linear:ENG-7` or `github:owner/name#42`, whichever way the
/// planner wrote it.
fn issue_key(issue: &str) -> String {
    if issue.starts_with("linear:") || issue.starts_with("github:") {
        return issue.to_owned();
    }
    if issue.contains('#') {
        return format!("github:{issue}");
    }
    format!("linear:{issue}")
}

/// Stores `batch`, a result's plan, as the next revision of the plan of `task`'s batch, or says
/// why the plan does not hold.
pub(crate) fn planned(
    tx: &Transaction,
    task: i64,
    batch: &Value,
    summary: &str,
) -> rusqlite::Result<Result<(), String>> {
    let found = tx
        .query_row(
            "SELECT id, issues FROM batches WHERE task = ?1",
            [task],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((id, issues)) = found else {
        return Ok(Err(format!(
            "task {task} plans no batch: only `priory batch add` starts one"
        )));
    };
    let issues: Vec<String> = serde_json::from_str(&issues).expect("issues are stored as JSON");
    let plan: Plan = match serde_json::from_value(batch.clone()) {
        Ok(plan) => plan,
        Err(error) => return Ok(Err(format!("batch: {error}"))),
    };
    if let Err(refused) = checked(&plan, &issues) {
        return Ok(Err(refused));
    }
    let named = plan
        .entries
        .iter()
        .map(|entry| &entry.issue)
        .chain(plan.close.iter().map(|close| &close.issue));
    for issue in named {
        let key = issue_key(issue);
        let active = tx
            .query_row(
                "SELECT id, state FROM tasks WHERE key = ?1
                 AND state NOT IN ('done', 'failed', 'dropped')",
                [&key],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((other, state)) = active {
            return Ok(Err(format!("{key} already has task {other} ({state})")));
        }
    }
    let title = plan.title.clone().unwrap_or_else(|| summary.to_owned());
    tx.execute(
        "UPDATE batches SET plan = ?2, title = ?3, plan_revision = plan_revision + 1 WHERE id = ?1",
        params![
            id,
            serde_json::to_string(&plan).expect("a plan serializes"),
            title
        ],
    )?;
    Ok(Ok(()))
}

/// Why `plan` does not hold for a batch of `issues`, if it does not.
fn checked(plan: &Plan, issues: &[String]) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for entry in &plan.entries {
        if !ids.insert(entry.id.as_str()) {
            return Err(format!("entry id {} is used twice", entry.id));
        }
    }
    let mut worked = BTreeSet::new();
    for entry in &plan.entries {
        let key = issue_key(&entry.issue);
        if !issues.contains(&key) {
            return Err(format!("{key} is not one of the batch's issues"));
        }
        if let Some(part) = entry.part {
            return Err(format!(
                "entry {} is part {part} of {key}; Priory does not split issues yet",
                entry.id
            ));
        }
        for edge in &entry.after {
            match edge.moment.as_str() {
                "merged" => {}
                "waiting" => {
                    return Err(format!(
                        "entry {} waits on {} at `waiting`, which Priory does not build yet; use merged",
                        entry.id, edge.entry
                    ));
                }
                other => return Err(format!("moment `{other}` is not merged or waiting")),
            }
            if !ids.contains(edge.entry.as_str()) {
                return Err(format!(
                    "entry {} waits on {}, which the plan does not have",
                    entry.id, edge.entry
                ));
            }
        }
        if !worked.insert(key.clone()) {
            return Err(format!("issue {key} is in two entries; give them parts"));
        }
    }
    let mut closed = BTreeSet::new();
    for close in &plan.close {
        let key = issue_key(&close.issue);
        if !issues.contains(&key) {
            return Err(format!("{key} is not one of the batch's issues"));
        }
        if worked.contains(&key) {
            return Err(format!("issue {key} is both an entry and a close"));
        }
        closed.insert(key);
    }
    if let Some(left) = issues
        .iter()
        .find(|issue| !worked.contains(*issue) && !closed.contains(*issue))
    {
        return Err(format!(
            "issue {left} of the batch is in no entry and no close"
        ));
    }
    let after: BTreeMap<&str, Vec<&str>> = plan
        .entries
        .iter()
        .map(|entry| {
            let others = entry.after.iter().map(|edge| edge.entry.as_str()).collect();
            (entry.id.as_str(), others)
        })
        .collect();
    let mut done = BTreeSet::new();
    for entry in &plan.entries {
        if let Some(cycle) = looped(&after, entry.id.as_str(), &mut vec![], &mut done) {
            return Err(format!("the plan has a loop: {}", cycle.join(", ")));
        }
    }
    Ok(())
}

/// The first loop through `after` reached from `at`, as the ids along it, if there is one.
fn looped<'a>(
    after: &BTreeMap<&'a str, Vec<&'a str>>,
    at: &'a str,
    path: &mut Vec<&'a str>,
    done: &mut BTreeSet<&'a str>,
) -> Option<Vec<&'a str>> {
    if let Some(start) = path.iter().position(|id| *id == at) {
        let mut cycle = path[start..].to_vec();
        cycle.push(at);
        return Some(cycle);
    }
    if !done.insert(at) {
        return None;
    }
    path.push(at);
    for other in after.get(at).into_iter().flatten() {
        if let Some(cycle) = looped(after, other, path, done) {
            return Some(cycle);
        }
    }
    path.pop();
    None
}

/// The question and Markdown text of the go ask of `task`'s batch, which marks the batch asked at
/// its plan's revision; none when the task plans no batch or its plan never came.
pub(crate) fn go_ask(tx: &Transaction, task: i64) -> rusqlite::Result<Option<(String, String)>> {
    let found = tx
        .query_row(
            "SELECT id, title, issues, plan FROM batches WHERE task = ?1 AND plan IS NOT NULL
             AND went IS NULL",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((id, title, issues, plan)) = found else {
        return Ok(None);
    };
    tx.execute(
        "UPDATE batches SET state = 'asked', asked_revision = plan_revision WHERE id = ?1",
        [id],
    )?;
    let issues: Vec<String> = serde_json::from_str(&issues).expect("issues are stored as JSON");
    let plan: Plan = serde_json::from_str(&plan).expect("a stored plan parses");
    let count = |n: usize, one: &str| match n {
        1 => format!("1 {one}"),
        n => format!("{n} {one}s"),
    };
    let closes = match plan.close.len() {
        0 => String::new(),
        n => format!(", and close {n}"),
    };
    let question = format!(
        "Start batch {id}: {} from {}{closes}?",
        count(plan.entries.len(), "task"),
        count(issues.len(), "issue")
    );
    let mut text = format!("Batch {id}: {title}\n");
    for (at, entry) in plan.entries.iter().enumerate() {
        let after: Vec<String> = entry
            .after
            .iter()
            .map(|edge| format!("{} {}", edge.entry, edge.moment))
            .collect();
        let after = match after.is_empty() {
            true => String::new(),
            false => format!(", after {}", after.join(" and ")),
        };
        text.push_str(&format!(
            "\n{}. {} {}: {}{after}",
            at + 1,
            entry.id,
            issue_key(&entry.issue),
            entry.title
        ));
    }
    if !plan.close.is_empty() {
        text.push_str("\n\nClose:\n");
        for close in &plan.close {
            let evidence = close
                .evidence
                .as_deref()
                .map_or(String::new(), |evidence| format!(" ({evidence})"));
            text.push_str(&format!(
                "\n- {}: {}{evidence}",
                issue_key(&close.issue),
                close.why
            ));
        }
    }
    Ok(Some((question, text)))
}

/// Refuses a go on a plan that changed since its ask showed it, and marks the batch of `task`
/// going on a go, so no edit lands between the answer and the start.
pub(crate) fn answering(
    tx: &Transaction,
    task: i64,
    go: bool,
    by: &str,
) -> rusqlite::Result<Result<(), String>> {
    let found = tx
        .query_row(
            "SELECT id, plan_revision, asked_revision FROM batches
             WHERE task = ?1 AND state = 'asked'",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((id, current, asked)) = found else {
        return Ok(Ok(()));
    };
    if Some(current) != asked {
        return Ok(Err(format!(
            "the plan of batch {id} is at revision {current}, not {}; answer its new ask",
            asked.unwrap_or_default()
        )));
    }
    if go {
        tx.execute(
            "UPDATE batches SET state = 'going', went_by = ?2 WHERE id = ?1",
            params![id, by],
        )?;
    }
    Ok(Ok(()))
}

/// Starts the batch of `task` as its approved plan says: one task per entry, held on its edges,
/// related to the entries it builds on and numbered in plan order in its band. A batch that went
/// already starts nothing again. An issue an active task took since the plan fails the whole start.
pub(crate) fn start(
    tx: &Transaction,
    task: i64,
    factory: &Factory,
) -> rusqlite::Result<Result<(), String>> {
    let found = tx
        .query_row(
            "SELECT batches.id, batches.plan, batches.went IS NOT NULL, batches.entries_workflow,
               tasks.band, tasks.lane, tasks.repo, tasks.trigger
             FROM batches JOIN tasks ON tasks.id = batches.task WHERE batches.task = ?1",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((id, plan, went, workflow, band, lane, repo, trigger)) = found else {
        return Ok(Err(format!("task {task} plans no batch")));
    };
    if went {
        return Ok(Ok(()));
    }
    let Some(plan) = plan else {
        return Ok(Err(format!("batch {id} has no plan")));
    };
    let plan: Plan = serde_json::from_str(&plan).expect("a stored plan parses");
    for entry in &plan.entries {
        let key = issue_key(&entry.issue);
        let taken = tx
            .query_row(
                "SELECT id, state FROM tasks WHERE key = ?1
                 AND state NOT IN ('done', 'failed', 'dropped')",
                [&key],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((other, state)) = taken {
            return Ok(Err(format!(
                "{key} has task {other} ({state}) since the plan; drop the entry or the batch"
            )));
        }
    }
    let Some(flow) = factory.workflows.get(&workflow) else {
        return Ok(Err(format!("batch {id}'s entries workflow {workflow} is not in the factory")));
    };
    let mut tasks = BTreeMap::new();
    for entry in &plan.entries {
        let key = issue_key(&entry.issue);
        let source = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO tasks (key, workflow, step, state, band, lane, lane_derived, trigger,
               source, repo, entered, batch, batch_entry)
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5, 0, ?6, ?7, ?8,
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?9, ?10)",
            params![
                key,
                workflow,
                flow.value.start,
                band,
                lane,
                trigger,
                source,
                repo,
                id,
                entry.id
            ],
        )?;
        let created = tx.last_insert_rowid();
        tasks.insert(entry.id.as_str(), created);
        tx.execute(
            "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome)
             VALUES (?1, 'priory.batch.entry', ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?3,
               '[]', ?4, 'created')",
            params![
                source,
                key,
                json!({ "batch": id, "entry": entry.id, "issue": key, "title": entry.title,
                        "text": entry.brief })
                .to_string(),
                created
            ],
        )?;
        tx.execute(
            "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
            params![created, key],
        )?;
        if let Some(resource) = crate::subscribe::resource(&key) {
            tx.execute(
                "INSERT OR IGNORE INTO subscriptions (task, resource) VALUES (?1, ?2)",
                params![created, resource],
            )?;
        }
        crate::relate::created(tx, created, &key, &[])?;
        for edge in &entry.after {
            crate::relate::insert(tx, created, "builds-on", tasks[edge.entry.as_str()], "priory")?;
        }
        held(tx, id, created, &entry.after, &tasks)?;
    }
    number(tx, band.as_deref(), &tasks.values().copied().collect::<Vec<_>>())?;
    tx.execute(
        "UPDATE batches SET state = 'going', went = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        [id],
    )?;
    Ok(Ok(()))
}

/// Holds the new task `task` on one `merged` condition per edge in `after`.
fn held(
    tx: &Transaction,
    batch: i64,
    task: i64,
    after: &[Edge],
    tasks: &BTreeMap<&str, i64>,
) -> rusqlite::Result<()> {
    if after.is_empty() {
        return Ok(());
    }
    let reason = format!("batch {batch}");
    let mut texts = Vec::new();
    for edge in after {
        let other = tasks[edge.entry.as_str()];
        tx.execute(
            "INSERT INTO holds (task, reason, kind, target, resume)
             VALUES (?1, ?2, 'merged', ?3, 'queued')",
            params![task, reason, other],
        )?;
        texts.push(format!("until task {other} merges"));
    }
    tx.execute(
        "UPDATE tasks SET state = 'held', reason = ?2 WHERE id = ?1",
        params![task, format!("held: {reason}, {}", texts.join(" and "))],
    )
    .map(drop)
}

/// Numbers `entries` after the other active tasks of `band`, in plan order, as `priory move`
/// numbers a band.
fn number(tx: &Transaction, band: Option<&str>, entries: &[i64]) -> rusqlite::Result<()> {
    let others: Vec<i64> = tx
        .prepare(&format!(
            "SELECT id FROM tasks WHERE coalesce(band, '') = ?1
             AND state NOT IN ('done', 'failed', 'dropped')
             AND id NOT IN (SELECT value FROM json_each(?2)) ORDER BY {}",
            crate::state::START_ORDER
        ))?
        .query_map(
            params![band.unwrap_or_default(), json!(entries).to_string()],
            |row| row.get(0),
        )?
        .collect::<rusqlite::Result<_>>()?;
    for (position, id) in others.iter().chain(entries).enumerate() {
        tx.execute(
            "UPDATE tasks SET position = ?2 WHERE id = ?1",
            params![id, position as i64 + 1],
        )?;
    }
    Ok(())
}

/// Every batch, newest first, or the one `which` names by id or key with its entries and closes,
/// as `priory batch show --json` prints it.
pub fn show(db: &Connection, which: Option<&str>) -> Result<Value, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let revision: i64 = db
        .query_row("SELECT value FROM revision", [], |row| row.get(0))
        .map_err(error)?;
    let ids: Vec<i64> = db
        .prepare(
            "SELECT id FROM batches WHERE ?1 IS NULL OR CAST(id AS TEXT) = ?1 OR key = ?1
             ORDER BY id DESC",
        )
        .and_then(|mut statement| {
            statement
                .query_map([which], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()
        })
        .map_err(error)?;
    let mut batches = Vec::new();
    for id in ids {
        batches.push(one(db, id, which.is_some()).map_err(error)?);
    }
    let Some(which) = which else {
        return Ok(json!({ "revision": revision, "batches": batches }));
    };
    let mut batch = batches
        .pop()
        .ok_or_else(|| format!("there is no batch {which}"))?;
    batch["revision"] = json!(revision);
    Ok(batch)
}

/// Batch `id`, with its entries and closes when `full`.
fn one(db: &Connection, id: i64, full: bool) -> rusqlite::Result<Value> {
    let (mut batch, plan): (Value, Option<String>) = db.query_row(
        "SELECT json_object('id', batches.id, 'key', batches.key, 'task', batches.task,
           'title', batches.title,
           'state', CASE
             WHEN batches.went IS NULL AND tasks.state IN ('done', 'failed', 'dropped') THEN 'dropped'
             WHEN batches.went IS NOT NULL AND tasks.state IN ('done', 'failed', 'dropped')
               AND NOT EXISTS (SELECT 1 FROM tasks AS entry WHERE entry.batch = batches.id
                 AND entry.id != batches.task AND entry.state NOT IN ('done', 'failed', 'dropped'))
               THEN 'ended'
             ELSE batches.state END,
           'repo', batches.repo, 'band', batches.band, 'lane', batches.lane,
           'plan_revision', batches.plan_revision,
           'ask', (SELECT max(asks.id) FROM asks WHERE asks.task = batches.task AND asks.kind = 'step'),
           'went', batches.went, 'went_by', batches.went_by), batches.plan
         FROM batches JOIN tasks ON tasks.id = batches.task WHERE batches.id = ?1",
        [id],
        |row| {
            let batch: String = row.get(0)?;
            Ok((
                serde_json::from_str(&batch).expect("SQLite writes JSON"),
                row.get(1)?,
            ))
        },
    )?;
    let plan: Option<Plan> = plan.map(|plan| serde_json::from_str(&plan).expect("a stored plan parses"));
    let mut entries = Vec::new();
    let mut counts: BTreeMap<&str, i64> = STAGES.iter().map(|stage| (*stage, 0)).collect();
    for (at, entry) in plan.iter().flat_map(|plan| plan.entries.iter().enumerate()) {
        let shown = entry_of(db, id, at, entry)?;
        let stage = shown["stage"].as_str().unwrap_or("planned");
        if let Some(count) = counts.iter_mut().find(|(name, _)| **name == stage) {
            *count.1 += 1;
        }
        entries.push(shown);
    }
    batch["counts"] = json!(counts);
    if !full {
        return Ok(batch);
    }
    batch["entries"] = json!(entries);
    let closes: Vec<Value> = match &plan {
        Some(plan) => plan
            .close
            .iter()
            .map(|close| closed(db, batch["task"].as_i64().unwrap_or_default(), close))
            .collect::<rusqlite::Result<_>>()?,
        None => Vec::new(),
    };
    batch["closes"] = json!(closes);
    Ok(batch)
}

/// The batch's words for an entry's place, in the order its counts show them.
const STAGES: [&str; 8] = [
    "planned", "queued", "blocked", "started", "waiting", "merged", "failed", "dropped",
];

/// One entry of batch `id`, at plan position `at`, with its task once the batch went.
fn entry_of(db: &Connection, id: i64, at: usize, entry: &Entry) -> rusqlite::Result<Value> {
    let task = db
        .query_row(
            "SELECT id, key, state FROM tasks WHERE batch = ?1 AND batch_entry = ?2",
            params![id, entry.id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let stage = match &task {
        None => "planned",
        Some((own, _, state)) => match state.as_str() {
            "held" => "planned",
            "queued" => "queued",
            "blocked" => "blocked",
            "failed" => "failed",
            "dropped" => "dropped",
            _ if crate::tools::all_merged(db, *own)? => "merged",
            "done" if crate::tools::pull_request(db, *own)?.is_none() => "merged",
            _ => "started",
        },
    };
    let mut after = Vec::new();
    for edge in &entry.after {
        let met = match &task {
            Some((own, ..)) => db
                .query_row(
                    "SELECT holds.met FROM holds JOIN tasks ON tasks.id = holds.target
                     WHERE holds.task = ?1 AND holds.kind = 'merged' AND tasks.batch_entry = ?2
                     ORDER BY holds.id DESC LIMIT 1",
                    params![own, edge.entry],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten(),
            None => None,
        };
        let other: Option<i64> = db
            .query_row(
                "SELECT id FROM tasks WHERE batch = ?1 AND batch_entry = ?2",
                params![id, edge.entry],
                |row| row.get(0),
            )
            .optional()?;
        after.push(json!({ "entry": edge.entry, "task": other, "moment": edge.moment, "met": met }));
    }
    Ok(json!({
        "entry": entry.id,
        "position": at + 1,
        "issue": entry.issue,
        "part": entry.part,
        "title": entry.title,
        "task": task.as_ref().map(|task| task.0),
        "key": task.as_ref().map(|task| &task.1),
        "state": task.as_ref().map(|task| &task.2),
        "stage": stage,
        "after": after,
    }))
}

/// A close of the plan, with when the planning task's `close` step reported it closed.
fn closed(db: &Connection, task: i64, close: &Close) -> rusqlite::Result<Value> {
    let key = issue_key(&close.issue);
    let reported: Option<(String, String)> = db
        .query_row(
            "SELECT coalesce(results.evidence, ''), runs.ended FROM results
             JOIN runs ON runs.id = results.run
             WHERE results.task = ?1 AND results.step = 'close' ORDER BY results.run DESC LIMIT 1",
            [task],
            |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?.unwrap_or_default())),
        )
        .optional()?;
    let at = reported.and_then(|(evidence, at)| {
        evidence
            .lines()
            .any(|line| line.contains(&key) && line.contains("closed"))
            .then_some(at)
    });
    Ok(json!({ "issue": close.issue, "why": close.why, "evidence": close.evidence, "closed": at }))
}
