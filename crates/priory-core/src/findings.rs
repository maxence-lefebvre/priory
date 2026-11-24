use crate::tools::Call;
use priory_factory::factory::Factory;
use priory_intake::request::{Add, Refused, Relation, add_to};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

/// How many runs in a row a finding is reported before its fix task moves up one band.
pub(crate) const STREAK: i64 = 3;

fn text(call: &Call, name: &str) -> Result<Option<String>, String> {
    match &call.arguments[name] {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(value) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_owned()))
        }
        _ => Err(format!("{name} must be a non-empty string")),
    }
}

/// The fix task's key: the generation comes before the finding's key, so no two findings collide.
fn fix_key(task: i64, generation: i64, key: &str) -> String {
    format!("finding:{task}:{generation}:{key}")
}

/// The `finding` tool: hands one problem of the live run to a fix task. A finding is one problem
/// by its key, so the same key in the next run opens no second task, and a key that skipped a run
/// and returns opens a new one. It does not end the turn.
pub(crate) fn tool(
    db: &mut Connection,
    call: &Call,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let fields = (|| {
        let key = text(call, "key")?.ok_or("key must be a non-empty string")?;
        let title = text(call, "title")?.ok_or("title must be a non-empty string")?;
        Ok::<_, String>((key, title, text(call, "details")?))
    })();
    let (key, title, details) = match fields {
        Ok(fields) => fields,
        Err(error) => return Ok(Err(error)),
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let run = tx
        .query_row(
            "SELECT id, task, (SELECT repo FROM tasks WHERE tasks.id = runs.task), (SELECT max(id) FROM runs earlier WHERE earlier.task = runs.task AND earlier.id < runs.id)
             FROM runs WHERE ended IS NULL AND session = ?1",
            [&call.session],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<i64>>(3)?)),
        )
        .optional()?;
    let Some((run, task, repo, previous)) = run else {
        return Ok(Err(format!("no live run has session {}", call.session)));
    };
    let stored: Option<(i64, i64, i64, bool)> = tx
        .query_row(
            "SELECT generation, streak, run, escalated FROM findings WHERE task = ?1 AND key = ?2",
            params![task, key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (generation, streak, escalated) = match stored {
        Some((_, _, seen, _)) if seen == run => {
            return Ok(Ok(format!("finding {key} is already reported in this run")));
        }
        Some((generation, streak, seen, escalated)) if Some(seen) == previous => {
            (generation, streak + 1, escalated)
        }
        Some((generation, ..)) => (generation + 1, 1, false),
        None => (1, 1, false),
    };
    let fix = fix_key(task, generation, &key);
    if streak == 1 {
        let mut body = title.clone();
        if let Some(details) = &details {
            body = format!("{body}\n\n{details}");
        }
        let add = Add {
            text: format!("{body}\n\nFound by task {task} in run {run} as finding {key}."),
            workflow: None,
            band: None,
            lane: None,
            repo,
            relates: vec![Relation {
                kind: "relates".to_owned(),
                task,
            }],
            intervention: None,
            session: None,
            auto_answer: None,
            labels: Vec::new(),
        };
        match add_to(factory, &add, &fix) {
            Ok(_) | Err(Refused::Repeated(_)) => {}
            Err(refused) => return Ok(Err(refused.to_string())),
        }
    }
    let moved = if streak >= STREAK && !escalated {
        tx.execute(
            "UPDATE tasks SET band = (SELECT name FROM bands WHERE rank =
               (SELECT rank FROM bands WHERE name = tasks.band) - 1), position = NULL
             WHERE key = ?1 AND state NOT IN ('done', 'failed', 'dropped')
               AND coalesce((SELECT rank FROM bands WHERE name = tasks.band), 0) > 0",
            [&fix],
        )? > 0
    } else {
        false
    };
    tx.execute(
        "INSERT INTO findings (task, key, title, generation, streak, run, escalated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (task, key) DO UPDATE SET title = ?3, generation = ?4, streak = ?5, run = ?6,
           escalated = ?7",
        params![
            task,
            key,
            title,
            generation,
            streak,
            run,
            escalated || moved
        ],
    )?;
    tx.commit()?;
    let said = match (streak, moved) {
        (1, _) => format!("finding {key} opened fix task {fix}"),
        (_, true) => {
            format!("finding {key} was reported {streak} runs in a row, so {fix} moved up a band")
        }
        _ => format!("finding {key} was reported {streak} runs in a row; {fix} is already open"),
    };
    Ok(Ok(said))
}

/// What a woken scan reads of its earlier runs: each finding, how long it has been reported and
/// the fix task it has.
pub(crate) fn memory(db: &Connection, task: i64) -> rusqlite::Result<Option<String>> {
    let mut statement = db.prepare(
        "SELECT key, title, streak, run = (SELECT max(id) FROM runs WHERE task = ?1), generation
         FROM findings WHERE task = ?1 ORDER BY key",
    )?;
    let rows = statement
        .query_map([task], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let lines: Vec<String> = rows
        .into_iter()
        .map(|(key, title, streak, last, generation)| {
            let fix = fix_key(task, generation, &key);
            let seen = if last {
                format!("reported in the last {streak} runs in a row")
            } else {
                "not reported in the last run, so it looks fixed".to_owned()
            };
            format!("- {key}: {title} ({seen}; fix task {fix})")
        })
        .collect();
    Ok(Some(format!(
        "Findings from your earlier runs. Report a finding again with the same key, and no second fix task opens:\n{}",
        lines.join("\n")
    )))
}
