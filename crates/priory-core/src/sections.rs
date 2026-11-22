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
    /// Empty in a write: Priory fills it with the row's comment threads, oldest first. A row
    /// needs an `id` to take comments.
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
                    "section {:?}: comments must be empty; Priory fills them with the row's threads",
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

/// Stores `report` as `task`'s, keeping the choices already made on its picks and what the workflow
/// did with them when the write says nothing.
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
        chosen.extend(
            picks(&mut old)
                .map(|pick| (pick.id.clone(), (pick.chosen.take(), pick.applied.take()))),
        );
    }
    for pick in picks(&mut report) {
        let (kept, applied) = chosen.remove(&pick.id).unwrap_or_default();
        pick.chosen = kept;
        // An applied choice stands, so a write that leaves `applied` empty keeps it.
        if pick.chosen.is_some() && pick.applied.is_none() {
            pick.applied = applied;
        }
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
    found
        .map(|(task, revision, text)| {
            let mut report: Value =
                serde_json::from_str(&text).expect("a report is stored as JSON");
            report["task"] = task.into();
            report["revision"] = revision.into();
            threads(db, report)
        })
        .transpose()
}

/// Every task's report, newest first in the order `show` picks the latest: its task, date,
/// greeting and when it was written.
pub fn list(db: &Connection) -> rusqlite::Result<Value> {
    crate::listing::rows(
        db,
        "SELECT task, json_extract(report, '$.date') AS date,
           json_extract(report, '$.greeting') AS greeting, put
         FROM reports ORDER BY put DESC, task DESC",
        [],
    )
}

/// `report` with each row's `comments` filled with its threads, oldest first.
fn threads(db: &Connection, mut report: Value) -> rusqlite::Result<Value> {
    let mut by_row: HashMap<String, Vec<Value>> = HashMap::new();
    let mut statement = db.prepare(
        "SELECT id, row, reply_to, author, text, created, resolved FROM row_comments
         WHERE task = ?1 ORDER BY id",
    )?;
    let mut rows = statement.query([report["task"].as_i64()])?;
    while let Some(found) = rows.next()? {
        let comment = serde_json::json!({
            "id": found.get::<_, i64>(0)?,
            "author": found.get::<_, String>(3)?,
            "text": found.get::<_, String>(4)?,
            "created": found.get::<_, String>(5)?,
        });
        let thread = by_row.entry(found.get(1)?).or_default();
        match found.get::<_, Option<i64>>(2)? {
            Some(root) => {
                let root = thread.iter_mut().find(|thread| thread["id"] == root);
                let root = root.expect("a reply's thread is older than the reply");
                root["replies"]
                    .as_array_mut()
                    .expect("a thread has replies")
                    .push(comment);
            }
            None => {
                let mut comment = comment;
                comment["resolved"] = found.get::<_, bool>(6)?.into();
                comment["replies"] = serde_json::json!([]);
                thread.push(comment);
            }
        }
    }
    let sections = report["sections"].as_array_mut().into_iter().flatten();
    for row in sections.flat_map(|section| section["rows"].as_array_mut().into_iter().flatten()) {
        if let Some(threads) = row["id"].as_str().and_then(|id| by_row.get(id)) {
            row["comments"] = threads.clone().into();
        }
    }
    Ok(report)
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

/// Records a person's choice on the report that holds its pick, which one report at most holds. A
/// choice changes until the workflow applies it. An option with a request publishes it and fills
/// `applied`; any other tells the report's agent, as `priory say` does.
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
    // A choice stands once the workflow applied it; before that a person can change it.
    if let Some(chosen) = pick
        .chosen
        .as_ref()
        .filter(|chosen| pick.applied.is_some() || **chosen == choice.option)
    {
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
            // The agent was told to apply the first choice, and may still be at it.
            if let Some(before) = &pick.chosen {
                let text = format!(
                    "The person changed their pick on {} in your report from {before} to {}: do not apply {before}. Priory requested {}'s task and filled the pick's applied.",
                    pick.id, choice.option, choice.option
                );
                if let Err(error) = crate::tools::say(&tx, task, &text, &state, &factory.workflows)?
                {
                    return Ok(Err(format!("pick {}: {error}", pick.id)));
                }
            }
            tx.execute(
                "INSERT INTO dispatches (pick, request) VALUES (?1, ?2)",
                params![
                    pick.id,
                    serde_json::to_string(request).expect("a request serializes")
                ],
            )?;
            pick.applied = Some(format!("requested {}", request.summary()));
        }
        None => {
            let text = match &pick.chosen {
                Some(before) => format!(
                    "The person changed their pick on {} in your report from {before} to {}: apply {} instead, then write the report again with the pick's applied filled.",
                    pick.id, choice.option, choice.option
                ),
                None => format!(
                    "The person picked {} on {} in your report: apply it, then write the report again with the pick's applied filled.",
                    choice.option, pick.id
                ),
            };
            if let Err(error) = crate::tools::say(&tx, task, &text, &state, &factory.workflows)? {
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

/// What a comment line does: a new thread on a row, a reply in a thread, or a thread's resolve or
/// reopen.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Remark {
    Comment,
    Reply,
    Resolve,
    Reopen,
}

/// A comment line, as `priory report comment|reply|resolve|reopen` sends it on the socket.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comment {
    pub comment: Remark,
    /// The row's id for a new thread, else a comment's id.
    pub on: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The report's revision the person saw, when they send one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<i64>,
    /// The run session of the agent that sends it, from `PRIORY_SESSION`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// Applies a comment line to the report that holds its row: the written last of those that have
/// a row with that id. A comment or reply from anyone but a live run of the report's own task
/// tells its agent, as `priory say` does, and is refused when `say` would refuse it.
pub(crate) fn comment(
    db: &mut Connection,
    comment: &Comment,
    factory: &priory_factory::factory::Factory,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found = match comment.comment {
        Remark::Comment => tx
            .query_row(
                "SELECT reports.task, reports.revision, tasks.state, NULL, 0
                 FROM reports JOIN tasks ON tasks.id = reports.task
                 WHERE EXISTS (SELECT 1 FROM json_each(reports.report, '$.sections') AS section,
                   json_each(section.value, '$.rows') AS row
                   WHERE json_extract(row.value, '$.id') = ?1)
                 ORDER BY reports.put DESC, reports.task DESC LIMIT 1",
                [&comment.on],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, comment.on.clone())),
            )
            .optional()?
            .ok_or_else(|| format!("no report has the row {}", comment.on)),
        _ => tx
            .query_row(
                "SELECT reports.task, reports.revision, tasks.state, root.id, root.resolved, root.row
                 FROM row_comments AS remark
                 JOIN row_comments AS root ON root.id = coalesce(remark.reply_to, remark.id)
                 JOIN reports ON reports.task = remark.task JOIN tasks ON tasks.id = remark.task
                 WHERE remark.id = ?1",
                [comment.on.parse::<i64>().unwrap_or(-1)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .optional()?
            .ok_or_else(|| format!("there is no comment {}", comment.on)),
    };
    let (task, revision, state, root, resolved, row): (
        i64,
        i64,
        String,
        Option<i64>,
        bool,
        String,
    ) = match found {
        Ok(found) => found,
        Err(error) => return Ok(Err(error)),
    };
    if let Some(expected) = comment.expected.filter(|expected| *expected != revision) {
        return Ok(Err(format!(
            "task {task}'s report is at revision {revision}, not {expected}"
        )));
    }
    let revised = revision + 1;
    let done = match (comment.comment, root) {
        (Remark::Resolve | Remark::Reopen, Some(root)) => {
            let resolve = comment.comment == Remark::Resolve;
            tx.execute(
                "UPDATE row_comments SET resolved = ?2 WHERE id = ?1",
                params![root, resolve],
            )?;
            let now = if resolve { "resolved" } else { "open" };
            format!("thread {root} is {now} at revision {revised}")
        }
        (Remark::Resolve | Remark::Reopen, None) => unreachable!("a new thread is a comment"),
        (_, root) => {
            let Some(text) = comment
                .text
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            else {
                return Ok(Err("a comment needs a text".to_owned()));
            };
            if resolved {
                return Ok(Err(format!(
                    "thread {} is resolved: reopen it to reply",
                    root.unwrap_or_default()
                )));
            }
            let agent: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM runs WHERE task = ?1 AND ended IS NULL AND session = ?2)",
                params![task, comment.session],
                |row| row.get(0),
            )?;
            let id: i64 = tx.query_row(
                "INSERT INTO row_comments (task, row, reply_to, author, text)
                 VALUES (?1, ?2, ?3, ?4, ?5) RETURNING id",
                params![
                    task,
                    row,
                    root,
                    if agent { "agent" } else { "person" },
                    text
                ],
                |row| row.get(0),
            )?;
            let thread = root.unwrap_or(id);
            if !agent {
                let wrote = match root {
                    None => format!("commented on row {row}"),
                    Some(root) => format!("replied in thread {root} on row {row}"),
                };
                let line = format!(
                    "The person {wrote} in your report (reply with `priory report reply {thread} TEXT`): {text}"
                );
                if let Err(error) = crate::tools::say(&tx, task, &line, &state, &factory.workflows)?
                {
                    return Ok(Err(format!("row {row}: {error}")));
                }
            }
            match root {
                None => format!("comment {id} on row {row} at revision {revised}"),
                Some(root) => format!("reply {id} in thread {root} at revision {revised}"),
            }
        }
    };
    tx.execute(
        "UPDATE reports SET revision = revision + 1 WHERE task = ?1",
        [task],
    )?;
    tx.commit()?;
    Ok(Ok(done))
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const VALID: &str = r#"{"date":"2026-11-21","since":"2026-11-20T00:00:00Z","greeting":"hi","sections":[{"id":"s","title":"t","rows":[{"id":"r","text":"x","pick":{"id":"p","options":["a","b"],"recommended":"a"}}]}]}"#;

    #[test]
    fn the_valid_report_is_accepted() {
        assert!(parse(VALID).is_ok());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn text_of_any_shape_is_parsed_or_refused_without_panicking(
            text in prop_oneof![
                "\\PC{0,300}",
                (0..VALID.len(), "\\PC{0,20}").prop_map(|(at, junk)| {
                    let at = (0..=at).rev().find(|at| VALID.is_char_boundary(*at)).unwrap();
                    format!("{}{junk}{}", &VALID[..at], &VALID[at..])
                }),
                (0..VALID.len()).prop_map(|at| {
                    let at = (0..=at).rev().find(|at| VALID.is_char_boundary(*at)).unwrap();
                    VALID[..at].to_owned()
                }),
                (1..20_000usize).prop_map(|depth| "[".repeat(depth)),
                (1..20_000usize).prop_map(|depth| format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth))),
            ],
        ) {
            let _ = parse(&text);
        }
    }
}
