//! Reports of ordered sections that a workflow writes for a person, such as Bonjour's, and the
//! picks a person makes on their rows.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A report a workflow writes for its task: ordered sections of rows. The latest one per task is
/// kept, with a revision that each write and each pick raises.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// The day the report is for, such as 2026-11-20.
    pub date: String,
    /// The RFC 3339 instant the report covers changes from.
    pub since: String,
    /// The line the report opens with.
    pub greeting: String,
    /// The sections, in the order a front end draws them.
    pub sections: Vec<Section>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// Unique in the report.
    pub id: String,
    pub title: String,
    /// The number a front end shows beside the title, when it is not the number of rows.
    #[serde(default)]
    pub count: Option<u64>,
    pub rows: Vec<Row>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Row {
    #[serde(default)]
    pub id: Option<String>,
    /// An issue or pull request key, such as FND-2409.
    #[serde(default)]
    pub key: Option<String>,
    pub text: String,
    /// A second line, such as 41 events since 18:00.
    #[serde(default)]
    pub meta: Option<String>,
    /// A task state for the row's tile.
    #[serde(default)]
    pub state: Option<String>,
    /// Who the row waits on, such as "on Max".
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub pick: Option<Pick>,
    /// Always empty: comments on rows come in a later version.
    #[serde(default)]
    #[schemars(length(max = 0))]
    pub comments: Vec<Value>,
}

/// A choice a person makes with `priory report pick`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pick {
    /// Unique in the report, such as 2026-11-20_sentry_fnd-2409.
    pub id: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub recommended: Option<String>,
    /// The person's choice. Only `priory report pick` sets it: a write keeps the stored one.
    #[serde(default)]
    pub chosen: Option<String>,
    /// What the workflow did with the choice.
    #[serde(default)]
    pub applied: Option<String>,
    /// Options that request a task instead of waking the report's task, by option.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requests: BTreeMap<String, priory_intake::request::Request>,
}

/// The report in `text`, or why its shape is refused.
pub fn parse(text: &str) -> Result<Report, String> {
    let report: Report =
        serde_json::from_str(text).map_err(|error| format!("the report is refused: {error}"))?;
    check(&report).map_err(|error| format!("the report is refused: {error}"))?;
    Ok(report)
}

fn check(report: &Report) -> Result<(), String> {
    report
        .date
        .parse::<jiff::civil::Date>()
        .map_err(|error| format!("date {:?}: {error}", report.date))?;
    report
        .since
        .parse::<jiff::Timestamp>()
        .map_err(|error| format!("since {:?}: {error}", report.since))?;
    let (mut sections, mut picks) = (HashSet::new(), HashSet::new());
    for section in &report.sections {
        if !sections.insert(&section.id) {
            return Err(format!("two sections have the id {:?}", section.id));
        }
        for row in &section.rows {
            if !row.comments.is_empty() {
                return Err(format!(
                    "section {:?}: comments must be empty; they come in a later version",
                    section.id
                ));
            }
            let Some(pick) = &row.pick else { continue };
            if !picks.insert(&pick.id) {
                return Err(format!("two picks have the id {:?}", pick.id));
            }
            if pick.options.is_empty() {
                return Err(format!("pick {:?} has no options", pick.id));
            }
            let named = pick.recommended.iter().chain(pick.requests.keys());
            if let Some(stray) = named.into_iter().find(|name| !pick.options.contains(name)) {
                return Err(format!(
                    "pick {:?}: {stray:?} is not one of its options {:?}",
                    pick.id, pick.options
                ));
            }
        }
    }
    Ok(())
}

fn picks(report: &mut Report) -> impl Iterator<Item = &mut Pick> {
    report
        .sections
        .iter_mut()
        .flat_map(|section| section.rows.iter_mut())
        .filter_map(|row| row.pick.as_mut())
}

/// Stores `report` as `task`'s, keeping the choices already made on its picks.
pub fn put(db: &mut Connection, task: i64, mut report: Report) -> Result<String, String> {
    let at = |error: rusqlite::Error| error.to_string();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(at)?;
    let exists = tx
        .query_row("SELECT 1 FROM tasks WHERE id = ?1", [task], |_| Ok(()))
        .optional()
        .map_err(at)?;
    if exists.is_none() {
        return Err(format!("there is no task {task}"));
    }
    let stored: Option<String> = tx
        .query_row(
            "SELECT report FROM reports WHERE task = ?1",
            [task],
            |row| row.get(0),
        )
        .optional()
        .map_err(at)?;
    let mut chosen = HashMap::new();
    if let Some(mut old) = stored.and_then(|text| serde_json::from_str::<Report>(&text).ok()) {
        chosen.extend(picks(&mut old).map(|pick| (pick.id.clone(), pick.chosen.take())));
    }
    for pick in picks(&mut report) {
        pick.chosen = chosen.remove(&pick.id).flatten();
        if let Some(kept) = pick
            .chosen
            .as_ref()
            .filter(|kept| !pick.options.contains(kept))
        {
            return Err(format!(
                "the report is refused: pick {:?} was chosen as {kept:?}, which its options drop",
                pick.id
            ));
        }
        let owner: Option<i64> = tx
            .query_row(
                "SELECT task FROM reports WHERE task != ?1 AND EXISTS (
                   SELECT 1 FROM json_each(report, '$.sections') AS section,
                     json_each(section.value, '$.rows') AS row
                   WHERE json_extract(row.value, '$.pick.id') = ?2)",
                params![task, pick.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(at)?;
        if let Some(owner) = owner {
            return Err(format!(
                "the report is refused: task {owner}'s report already has the pick {:?}",
                pick.id
            ));
        }
    }
    let text = serde_json::to_string(&report).expect("a report serializes");
    let revision: i64 = tx
        .query_row(
            "INSERT INTO reports (task, revision, report, put)
             VALUES (?1, 1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT (task) DO UPDATE SET revision = revision + 1, report = excluded.report,
               put = excluded.put
             RETURNING revision",
            params![task, text],
            |row| row.get(0),
        )
        .map_err(at)?;
    tx.commit().map_err(at)?;
    Ok(format!("task {task}'s report is at revision {revision}"))
}

/// `task`'s report, or the one written last, with its task and revision; none when there is none.
pub fn show(db: &Connection, task: Option<i64>) -> rusqlite::Result<Option<Value>> {
    let found = db
        .query_row(
            "SELECT task, revision, report FROM reports WHERE ?1 IS NULL OR task = ?1
             ORDER BY put DESC, task DESC LIMIT 1",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(found.map(|(task, revision, text)| {
        let mut report: Value = serde_json::from_str(&text).expect("a report is stored as JSON");
        report["task"] = task.into();
        report["revision"] = revision.into();
        report
    }))
}

/// The `report` tool, which `priory mcp` answers itself: stores the report in `arguments` as the
/// task's of the live run with harness session `session`. `None` for another tool.
pub fn tool(session: &str, tool: &str, arguments: &Value) -> Option<Result<String, String>> {
    if tool != "report" {
        return None;
    }
    let report = parse(&arguments.to_string());
    Some(report.and_then(|report| {
        let mut db = crate::state::open()?;
        let task = db
            .query_row(
                "SELECT task FROM runs WHERE ended IS NULL AND session = ?1",
                [session],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no live run has session {session}"))?;
        put(&mut db, task, report)
    }))
}

/// A person's choice on a pick, as `priory report pick` sends it on the socket.
#[derive(Serialize, Deserialize)]
pub struct Choice {
    pub pick: String,
    pub option: String,
    /// The report's revision the person saw.
    pub expected: i64,
}

/// Records a person's choice on the report that holds its pick, which one report at most holds. The
/// first choice stands. An option with a request publishes it and fills `applied`; any other tells
/// the report's agent, as `priory say` does.
pub(crate) fn pick(
    db: &mut Connection,
    choice: &Choice,
    factory: &priory_factory::factory::Factory,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found = tx
        .query_row(
            "SELECT reports.task, reports.revision, reports.report, tasks.state
             FROM reports JOIN tasks ON tasks.id = reports.task
             WHERE EXISTS (SELECT 1 FROM json_each(reports.report, '$.sections') AS section,
               json_each(section.value, '$.rows') AS row
               WHERE json_extract(row.value, '$.pick.id') = ?1)
             LIMIT 1",
            [&choice.pick],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((task, revision, text, state)) = found else {
        return Ok(Err(format!("no report has the pick {}", choice.pick)));
    };
    if choice.expected != revision {
        return Ok(Err(format!(
            "task {task}'s report is at revision {revision}, not {}",
            choice.expected
        )));
    }
    let mut report: Report = serde_json::from_str(&text).expect("a stored report parses");
    let pick = picks(&mut report)
        .find(|pick| pick.id == choice.pick)
        .expect("the query found the pick");
    if let Some(chosen) = &pick.chosen {
        return Ok(Err(format!("pick {} is already {chosen}", pick.id)));
    }
    if !pick.options.contains(&choice.option) {
        return Ok(Err(format!(
            "pick {} takes one of: {}",
            pick.id,
            pick.options.join(", ")
        )));
    }
    match pick.requests.get(&choice.option) {
        Some(request) => {
            if let Err(error) = priory_intake::request::check(factory, request) {
                return Ok(Err(format!("pick {}: {error}", pick.id)));
            }
            tx.execute(
                "INSERT INTO dispatches (pick, request) VALUES (?1, ?2)",
                params![
                    pick.id,
                    serde_json::to_string(request).expect("a request serializes")
                ],
            )?;
            pick.applied = Some(format!("requested task cli:{}", request.key()));
        }
        None => {
            let say = crate::tools::Act {
                act: "say".to_owned(),
                task,
                step: None,
                hold: None,
                text: Some(format!(
                    "The person picked {} on {} in your report: apply it, then write the report again with the pick's applied filled.",
                    choice.option, pick.id
                )),
                expected: None,
            };
            if let Err(error) = crate::tools::say(&tx, &say, &state, &factory.workflows)? {
                return Ok(Err(format!("pick {}: {error}", pick.id)));
            }
        }
    }
    pick.chosen = Some(choice.option.clone());
    tx.execute(
        "UPDATE reports SET revision = revision + 1, report = ?2 WHERE task = ?1",
        params![
            task,
            serde_json::to_string(&report).expect("a report serializes")
        ],
    )?;
    tx.commit()?;
    let recorded = format!(
        "pick {} is {} at revision {}",
        choice.pick,
        choice.option,
        revision + 1
    );
    Ok(match dispatch(db, factory, Some(&choice.pick))?.pop() {
        None => Ok(recorded),
        Some(error) => Err(format!(
            "{recorded}, but {error}; the daemon publishes it again when it starts"
        )),
    })
}

/// Publishes the pending dispatch of `only`, or every one, under its pick's id, so publishing one
/// twice adds nothing. A daemon start publishes those a crash or a failed publish left. Returns
/// why each one that failed is not published.
pub(crate) fn dispatch(
    db: &Connection,
    factory: &priory_factory::factory::Factory,
    only: Option<&str>,
) -> rusqlite::Result<Vec<String>> {
    let pending = db
        .prepare(
            "SELECT pick, request FROM dispatches WHERE ?1 IS NULL OR pick = ?1 ORDER BY rowid",
        )?
        .query_map([only], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut failed = Vec::new();
    for (pick, text) in pending {
        let request: priory_intake::request::Request =
            serde_json::from_str(&text).expect("a stored request parses");
        match priory_intake::request::run(factory, &request, Some(&pick)) {
            Ok(()) => {
                db.execute("DELETE FROM dispatches WHERE pick = ?1", [&pick])?;
            }
            Err(error) => failed.push(format!("pick {pick}'s request is not published: {error}")),
        }
    }
    Ok(failed)
}
