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

/// The oldest active task that owns issue `?1` or the key `?2` an entry takes, or that is a batch
/// part of `?1`, with its key.
const TAKEN: &str =
    "SELECT id, state, key FROM tasks WHERE state NOT IN ('done', 'failed', 'dropped')
  AND (id IN (SELECT task FROM owned WHERE resource IN (?1, ?2))
    OR batch_entry IS NOT NULL AND substr(key, 1, length(?1) + 1) = ?1 || '.')
  ORDER BY id LIMIT 1";

/// The key an entry's task takes: its issue's, or for a part of a split issue `<issue>.<part>`.
fn entry_key(entry: &Entry) -> String {
    let issue = issue_key(&entry.issue);
    match entry.part {
        Some(part) => format!("{issue}.{part}"),
        None => issue,
    }
}

/// The first key of `plan`, an entry's or a close's, that an active task holds, with that task's id
/// and state.
fn taken(tx: &Transaction, plan: &Plan) -> rusqlite::Result<Option<(String, i64, String)>> {
    let entries = plan
        .entries
        .iter()
        .map(|entry| (issue_key(&entry.issue), entry_key(entry)));
    let closes = plan
        .close
        .iter()
        .map(|close| (issue_key(&close.issue), issue_key(&close.issue)));
    for (issue, key) in entries.chain(closes) {
        let found = tx
            .query_row(TAKEN, [&issue, &key], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .optional()?;
        if let Some((other, state, theirs)) = found {
            let named = if theirs == key { key } else { issue };
            return Ok(Some((named, other, state)));
        }
    }
    Ok(None)
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
            "SELECT id, issues, state FROM batches WHERE task = ?1",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((id, issues, state)) = found else {
        return Ok(Err(format!(
            "task {task} plans no batch: only `priory batch add` starts one"
        )));
    };
    if state == "going" {
        return Ok(Err(format!("batch {id} went already; its plan is fixed")));
    }
    let issues: Vec<String> = serde_json::from_str(&issues).expect("issues are stored as JSON");
    let plan: Plan = match serde_json::from_value(batch.clone()) {
        Ok(plan) => plan,
        Err(error) => return Ok(Err(format!("batch: {error}"))),
    };
    if let Err(refused) = checked(&plan, &issues) {
        return Ok(Err(refused));
    }
    if let Some((key, other, state)) = taken(tx, &plan)? {
        return Ok(Err(format!("{key} already has task {other} ({state})")));
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
    let mut worked: BTreeMap<String, Vec<Option<i64>>> = BTreeMap::new();
    for entry in &plan.entries {
        let key = issue_key(&entry.issue);
        if !issues.contains(&key) {
            return Err(format!("{key} is not one of the batch's issues"));
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
        let parts = worked.entry(key.clone()).or_default();
        if parts.contains(&entry.part) && entry.part.is_some() {
            return Err(format!(
                "issue {key} has part {} twice",
                entry.part.unwrap_or_default()
            ));
        }
        if !parts.is_empty() && (entry.part.is_none() || parts.contains(&None)) {
            return Err(format!("issue {key} is in two entries; give them parts"));
        }
        parts.push(entry.part);
    }
    let mut closed = BTreeSet::new();
    for close in &plan.close {
        let key = issue_key(&close.issue);
        if !issues.contains(&key) {
            return Err(format!("{key} is not one of the batch's issues"));
        }
        if worked.contains_key(&key) {
            return Err(format!("issue {key} is both an entry and a close"));
        }
        closed.insert(key);
    }
    if let Some(left) = issues
        .iter()
        .find(|issue| !worked.contains_key(*issue) && !closed.contains(*issue))
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
    let mut earlier = BTreeSet::new();
    for entry in &plan.entries {
        if let Some(edge) = entry
            .after
            .iter()
            .find(|edge| !earlier.contains(edge.entry.as_str()))
        {
            return Err(format!(
                "entry {} waits on {}, which comes after it; put {} first",
                entry.id, edge.entry, edge.entry
            ));
        }
        earlier.insert(entry.id.as_str());
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
            .map(|edge| match &edge.why {
                Some(why) => format!("{} {} ({why})", edge.entry, edge.moment),
                None => format!("{} {}", edge.entry, edge.moment),
            })
            .collect();
        let after = match after.is_empty() {
            true => String::new(),
            false => format!(", after {}", after.join(" and ")),
        };
        text.push_str(&format!(
            "\n{}. {} {}: {}{after}\n",
            at + 1,
            entry.id,
            issue_key(&entry.issue),
            entry.title
        ));
        if !entry.files.is_empty() {
            text.push_str(&format!("   Files: {}\n", entry.files.join(", ")));
        }
        for line in entry.brief.lines() {
            text.push_str(&format!("   {line}\n"));
        }
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
            "SELECT id, plan, went IS NOT NULL, entries_workflow,
               plan_revision IS asked_revision, (SELECT band FROM tasks WHERE id = ?1)
             FROM batches WHERE task = ?1",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((id, plan, went, workflow, approved, band)) = found else {
        return Ok(Err(format!("task {task} plans no batch")));
    };
    if went {
        return Ok(Ok(()));
    }
    // A local transaction cannot stop another runner from claiming a request on an entry's issue.
    if factory.runners.len() > 1 {
        return Ok(Err(format!(
            "batches need one runner; this factory has {}",
            factory.runners.len()
        )));
    }
    let Some(plan) = plan.filter(|_| approved) else {
        return Ok(Err(format!("batch {id} has no plan the go approved")));
    };
    let plan: Plan = serde_json::from_str(&plan).expect("a stored plan parses");
    if let Some((key, other, state)) = taken(tx, &plan)? {
        return Ok(Err(format!(
            "{key} has task {other} ({state}) since the plan; drop the entry or the batch"
        )));
    }
    let Some(flow) = factory.workflows.get(&workflow) else {
        return Ok(Err(format!(
            "batch {id}'s entries workflow {workflow} is not in the factory"
        )));
    };
    let mut tasks = BTreeMap::new();
    let mut ordered = Vec::new();
    for entry in &plan.entries {
        let issue = issue_key(&entry.issue);
        let key = entry_key(entry);
        let source = uuid::Uuid::new_v4().to_string();
        // An entry takes the planning task's place and who answers its asks.
        tx.execute(
            "INSERT INTO tasks (key, workflow, step, state, band, lane, lane_derived, trigger,
               source, repo, intervention, lead, entered, batch, batch_entry)
             SELECT ?1, ?2, ?3, 'queued', band, lane, 0, trigger, ?4, repo, intervention, lead,
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?5, ?6
             FROM tasks WHERE id = ?7",
            params![key, workflow, flow.value.start, source, id, entry.id, task],
        )?;
        let created = tx.last_insert_rowid();
        tasks.insert(entry.id.as_str(), created);
        ordered.push(created);
        tx.execute(
            "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome)
             VALUES (?1, 'priory.batch.entry', ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?3,
               '[]', ?4, 'created')",
            params![
                source,
                key,
                json!({ "batch": id, "entry": entry.id, "issue": issue, "part": entry.part, "title": entry.title,
                        "text": entry.brief })
                .to_string(),
                created
            ],
        )?;
        tx.execute(
            "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
            params![created, key],
        )?;
        // A part does not own its issue: `state::owners` hands the issue's events to the oldest part.
        if let Some(resource) = crate::subscribe::resource(&key).filter(|_| entry.part.is_none()) {
            tx.execute(
                "INSERT OR IGNORE INTO subscriptions (task, resource) VALUES (?1, ?2)",
                params![created, resource],
            )?;
            tx.execute(
                "DELETE FROM subscriptions WHERE resource = ?2 AND task != ?1
                 AND task IN (SELECT id FROM tasks WHERE state = 'done')",
                params![created, resource],
            )?;
        }
        crate::relate::created(tx, created, &key, &[])?;
        for edge in &entry.after {
            let other = tasks[edge.entry.as_str()];
            crate::relate::insert(tx, created, "builds-on", other, "priory")?;
        }
        held(tx, id, created, &entry.after, &tasks)?;
    }
    number(tx, band.as_deref(), &ordered)?;
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

/// One edit of a plan while its go ask is open.
pub enum Edit {
    Drop {
        entry: String,
        with_successors: bool,
    },
    Move {
        entry: String,
        place: Place,
    },
    Moment {
        entry: String,
        other: String,
        moment: String,
    },
}

/// Where `priory batch edit --move` puts an entry in the plan's start order.
pub enum Place {
    Top,
    Before(String),
    After(String),
}

/// `POST /api/batches/{id}/edit`'s socket line: `{"batch_edit": {"batch": ..., "body": ...}}`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditLine {
    pub batch_edit: EditRequest,
}

/// The batch an edit names, by id or key, and the endpoint's body.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub batch: String,
    pub body: Value,
}

/// Applies the edit `request` names, as `priory batch edit` does.
pub fn apply(db: &mut Connection, request: &EditRequest) -> Result<String, String> {
    let (revision, change) = edit_of(&request.body)?;
    edit(db, &request.batch, revision, &change)
}

/// `POST /api/batches/{id}/edit`'s body, field by field.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditBody {
    revision: i64,
    drop: Option<String>,
    with_successors: Option<bool>,
    #[serde(rename = "move")]
    moves: Option<String>,
    top: Option<bool>,
    before: Option<String>,
    after: Option<String>,
    moment: Option<MomentBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MomentBody {
    entry: String,
    other: String,
    moment: String,
}

/// The edit and expected plan revision a `POST /api/batches/{id}/edit` body names, or why it
/// names none: it takes exactly one edit, with only that edit's fields.
pub fn edit_of(body: &Value) -> Result<(i64, Edit), String> {
    let body: EditBody = serde_json::from_value(body.clone()).map_err(|error| error.to_string())?;
    let placed = (body.top, body.before, body.after);
    let edit = match (body.drop, body.moves, body.moment) {
        (Some(entry), None, None) if matches!(placed, (None, None, None)) => Edit::Drop {
            entry,
            with_successors: body.with_successors.unwrap_or(false),
        },
        (None, Some(entry), None) if body.with_successors.is_none() => {
            let place = match placed {
                (Some(true), None, None) => Place::Top,
                (None, Some(other), None) => Place::Before(other),
                (None, None, Some(other)) => Place::After(other),
                _ => return Err("`move` takes one of `top: true`, `before` or `after`".to_owned()),
            };
            Edit::Move { entry, place }
        }
        (None, None, Some(moment))
            if body.with_successors.is_none() && matches!(placed, (None, None, None)) =>
        {
            Edit::Moment {
                entry: moment.entry,
                other: moment.other,
                moment: moment.moment,
            }
        }
        _ => {
            return Err(
                "the body takes one edit: `drop` with optional `with_successors`, `move` with one of `top: true`, `before` or `after`, or `moment` with `entry`, `other` and `moment`"
                    .to_owned(),
            );
        }
    };
    Ok((body.revision, edit))
}

/// Applies one edit to the plan of the batch `which` names, by id or key, as its next revision,
/// and asks its go again at that revision. Refused, it changes nothing.
pub fn edit(
    db: &mut Connection,
    which: &str,
    expected: i64,
    change: &Edit,
) -> Result<String, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(error)?;
    let found = tx
        .query_row(
            "SELECT id, task, plan_revision, plan, issues FROM batches
             WHERE CAST(id AS TEXT) = ?1 OR key = ?1",
            [which],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(error)?;
    let Some((id, task, current, plan, issues)) = found else {
        return Err(format!("there is no batch {which}"));
    };
    let state = one(&tx, id, false).map_err(error)?["state"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let ask: Option<i64> = tx
        .query_row(
            "SELECT max(id) FROM asks WHERE task = ?1 AND kind = 'step' AND answer IS NULL
             AND withdrawn IS NULL",
            [task],
            |row| row.get(0),
        )
        .map_err(error)?;
    let (Some(ask), Some(plan), "asked") = (ask, plan, state.as_str()) else {
        return Err(format!(
            "batch {id} is {state}; only an asked batch takes edits"
        ));
    };
    if current != expected {
        return Err(format!(
            "the plan of batch {id} is at revision {current}, not {expected}"
        ));
    }
    let mut plan: Plan = serde_json::from_str(&plan).expect("a stored plan parses");
    applied(&mut plan, id, change)?;
    let issues: Vec<String> = serde_json::from_str::<Vec<String>>(&issues)
        .expect("issues are stored as JSON")
        .into_iter()
        .filter(|issue| {
            let named = |named: &String| issue_key(named) == *issue;
            plan.entries.iter().any(|entry| named(&entry.issue))
                || plan.close.iter().any(|close| named(&close.issue))
        })
        .collect();
    checked(&plan, &issues)?;
    tx.execute(
        "UPDATE batches SET plan = ?2, issues = ?3, plan_revision = plan_revision + 1
         WHERE id = ?1",
        params![
            id,
            serde_json::to_string(&plan).expect("a plan serializes"),
            serde_json::to_string(&issues).expect("issues serialize")
        ],
    )
    .map_err(error)?;
    let (question, text) = go_ask(&tx, task)
        .map_err(error)?
        .expect("an asked batch has a plan");
    tx.execute(
        "UPDATE asks SET revision = revision + 1, question = ?2, text = ?3 WHERE id = ?1",
        params![ask, question, text],
    )
    .map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(format!("batch {id} is at plan revision {}", current + 1))
}

/// `names` as a reader lists them: `a`, `a and b`, `a, b and c`.
fn listed(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Applies `change` to `plan`, of batch `id`, or says why it does not apply.
fn applied(plan: &mut Plan, id: i64, change: &Edit) -> Result<(), String> {
    let at = |plan: &Plan, entry: &str| {
        plan.entries
            .iter()
            .position(|named| named.id == entry)
            .ok_or_else(|| format!("no entry {entry} in batch {id}"))
    };
    match change {
        Edit::Drop {
            entry,
            with_successors,
        } => {
            at(plan, entry)?;
            let waits_on = |named: &Entry, on: &BTreeSet<&str>| {
                named
                    .after
                    .iter()
                    .any(|edge| on.contains(edge.entry.as_str()))
            };
            let only = BTreeSet::from([entry.as_str()]);
            let successors: Vec<&str> = plan
                .entries
                .iter()
                .filter(|named| waits_on(named, &only))
                .map(|named| named.id.as_str())
                .collect();
            if !successors.is_empty() && !with_successors {
                return Err(format!(
                    "entry {entry} has successors {}; drop them first or add --with-successors",
                    listed(&successors)
                ));
            }
            // The entries run in start order, so each successor follows its predecessors.
            let mut dropped = only;
            for named in &plan.entries {
                if waits_on(named, &dropped) {
                    dropped.insert(named.id.as_str());
                }
            }
            let dropped: BTreeSet<String> = dropped.into_iter().map(str::to_owned).collect();
            plan.entries.retain(|named| !dropped.contains(&named.id));
        }
        Edit::Move { entry, place } => {
            let from = at(plan, entry)?;
            let to = match place {
                Place::Top => 0,
                Place::Before(other) | Place::After(other) if other == entry => {
                    return Err(format!("entry {entry} cannot move next to itself"));
                }
                Place::Before(other) => at(plan, other)?,
                Place::After(other) => at(plan, other)? + 1,
            };
            let moved = plan.entries.remove(from);
            plan.entries
                .insert(if to > from { to - 1 } else { to }, moved);
            let mut earlier = BTreeSet::new();
            for named in &plan.entries {
                if let Some(edge) = named
                    .after
                    .iter()
                    .find(|edge| !earlier.contains(edge.entry.as_str()))
                {
                    let other = &edge.entry;
                    return Err(match named.id == *entry {
                        true => {
                            format!("entry {entry} starts after {other}; move it after {other}")
                        }
                        false => format!(
                            "entry {} starts after {entry}; move {entry} before {}",
                            named.id, named.id
                        ),
                    });
                }
                earlier.insert(named.id.as_str());
            }
        }
        Edit::Moment {
            entry,
            other,
            moment,
        } => {
            let at = at(plan, entry)?;
            let edge = plan.entries[at]
                .after
                .iter_mut()
                .find(|edge| edge.entry == *other)
                .ok_or_else(|| format!("entry {entry} does not wait on {other}"))?;
            edge.moment.clone_from(moment);
        }
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
    let plan: Option<Plan> =
        plan.map(|plan| serde_json::from_str(&plan).expect("a stored plan parses"));
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
    // An entry that is done with its pull request open has not merged, so the batch goes on.
    let open = entries.iter().any(|entry| {
        !["merged", "failed", "dropped"].contains(&entry["stage"].as_str().unwrap_or_default())
    });
    if batch["state"] == "ended" && open {
        batch["state"] = json!("going");
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
    batch["splits"] = json!(plan.map(|plan| plan.splits).unwrap_or_default());
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
        after
            .push(json!({ "entry": edge.entry, "task": other, "moment": edge.moment, "met": met }));
    }
    Ok(json!({
        "entry": entry.id,
        "position": at + 1,
        "issue": entry.issue,
        "part": entry.part,
        "title": entry.title,
        "files": entry.files,
        "task": task.as_ref().map(|task| task.0),
        "key": task.as_ref().map(|task| &task.1),
        "state": task.as_ref().map(|task| &task.2),
        "stage": stage,
        "after": after,
    }))
}

/// The desk's line for going batch `id`: its title, its counts and which entries start next, or
/// none once it ended.
pub(crate) fn desk_line(db: &Connection, id: i64) -> rusqlite::Result<Option<String>> {
    let batch = one(db, id, true)?;
    if batch["state"] != "going" {
        return Ok(None);
    }
    let counts: Vec<String> = [
        "merged", "started", "waiting", "queued", "blocked", "planned", "failed", "dropped",
    ]
    .iter()
    .filter_map(|stage| {
        let count = batch["counts"][stage].as_i64().unwrap_or_default();
        (count > 0).then(|| format!("{count} {stage}"))
    })
    .collect();
    let mut line = format!(
        "{}: {}.",
        batch["title"].as_str().unwrap_or_default(),
        counts.join(", ")
    );
    // The entries whose one unmet edge is on the same task, for the first such task in plan order.
    let mut next: Vec<(i64, Vec<i64>)> = Vec::new();
    for entry in batch["entries"].as_array().into_iter().flatten() {
        let (Some(task), "held") = (
            entry["task"].as_i64(),
            entry["state"].as_str().unwrap_or(""),
        ) else {
            continue;
        };
        let unmet: BTreeSet<i64> = entry["after"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|edge| edge["met"].is_null())
            .filter_map(|edge| edge["task"].as_i64())
            .collect();
        let [on] = unmet.into_iter().collect::<Vec<_>>()[..] else {
            continue;
        };
        match next.iter_mut().find(|(other, _)| *other == on) {
            Some((_, tasks)) => tasks.push(task),
            None => next.push((on, vec![task])),
        }
    }
    if let Some((on, tasks)) = next.first() {
        let named: Vec<String> = tasks.iter().map(i64::to_string).collect();
        let named: Vec<&str> = named.iter().map(String::as_str).collect();
        let (tasks, start) = match tasks.len() {
            1 => ("task", "starts"),
            _ => ("tasks", "start"),
        };
        line.push_str(&format!(
            " Next: {tasks} {} {start} when task {on} merges.",
            listed(&named)
        ));
    }
    Ok(Some(line))
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
            |row| {
                Ok((
                    row.get(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            },
        )
        .optional()?;
    let at = reported.and_then(|(evidence, at)| reports_closed(&evidence, &key).then_some(at));
    Ok(json!({ "issue": close.issue, "why": close.why, "evidence": close.evidence, "closed": at }))
}

/// Whether `evidence` has a line `<issue>: closed`, where the issue is `key` written either way.
fn reports_closed(evidence: &str, key: &str) -> bool {
    evidence.lines().any(|line| {
        let mut words = line
            .split_whitespace()
            .map(|word| word.trim_matches(|c: char| ":,.;".contains(c)));
        let issue = words.next().map(issue_key);
        issue.as_deref() == Some(key) && words.next() == Some("closed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            "(e[0-9]|ENG-[0-9]|merged|waiting|[a-z#/:-]{0,8})".prop_map(Value::from),
        ];
        leaf.prop_recursive(4, 64, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
                proptest::collection::btree_map(
                    "(id|issue|part|title|brief|files|after|entry|moment|why|entries|close|evidence|title)",
                    inner,
                    0..6
                )
                .prop_map(|map| Value::Object(map.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn a_plan_of_any_shape_is_checked_or_refused_without_panicking(batch in value()) {
            let issues = ["linear:ENG-1".to_owned(), "linear:ENG-2".to_owned()];
            if let Ok(plan) = serde_json::from_value::<Plan>(batch) {
                let _ = checked(&plan, &issues);
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn an_edit_body_of_any_shape_is_read_or_refused_without_panicking(body in value()) {
            let _ = edit_of(&body);
        }
    }

    #[test]
    fn an_edit_body_names_exactly_one_edit_with_only_its_own_fields() {
        let refused = [
            json!({ "revision": 1, "drop": "e3", "move": "e2", "top": true }),
            json!({ "revision": 1, "drop": "e3", "with_successors": "yes" }),
            json!({ "revision": 1, "drop": "e3", "brief": "x" }),
            json!({ "revision": 1, "drop": "e3", "top": true }),
            json!({ "revision": 1, "move": "e3", "top": true, "with_successors": true }),
            json!({ "revision": 1, "move": "e3", "top": false }),
            json!({ "revision": 1 }),
        ];
        for body in refused {
            assert!(edit_of(&body).is_err(), "{body}");
        }
        assert!(matches!(
            edit_of(&json!({ "revision": 2, "drop": "e3", "with_successors": true })),
            Ok((
                2,
                Edit::Drop {
                    with_successors: true,
                    ..
                }
            ))
        ));
    }

    #[test]
    fn a_close_is_reported_by_its_own_line_only() {
        let evidence = "linear:ENG-40: closed, fixed by #198\nENG-41: closed\nENG-4: failed, could not be closed\nENG-5 skipped: changed";
        let closed = |key| reports_closed(evidence, key);
        assert_eq!(
            [
                "linear:ENG-40",
                "linear:ENG-41",
                "linear:ENG-4",
                "linear:ENG-5"
            ]
            .map(closed),
            [true, true, false, false]
        );
    }

    #[test]
    fn a_loop_is_named_from_the_first_entry_on_it() {
        let after = BTreeMap::from([("e1", vec!["e2"]), ("e2", vec!["e3"]), ("e3", vec!["e2"])]);
        assert_eq!(
            looped(&after, "e1", &mut vec![], &mut BTreeSet::new()),
            Some(vec!["e2", "e3", "e2"])
        );
    }
}
