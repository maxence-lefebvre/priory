use crate::tools::Call;
use crate::trace::{self, Trace};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `priory progress TASK JSON`: the fields to merge into task `task`'s progress.
#[derive(Serialize, Deserialize)]
pub struct Put {
    pub task: i64,
    pub progress: Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Fields {
    #[serde(skip_serializing_if = "Option::is_none")]
    goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    acceptance_criteria: Option<Vec<Criterion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_change: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_action: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Criterion {
    id: String,
    description: String,
    required: bool,
}

/// The fields of `progress` a call sets, or why its shape is refused.
fn checked(progress: &Value) -> Result<Map<String, Value>, String> {
    // Serde also reads a struct from an array, so refuse positional fields first.
    let mut criteria = progress["acceptanceCriteria"]
        .as_array()
        .into_iter()
        .flatten();
    if !progress.is_object() || criteria.any(|criterion| !criterion.is_object()) {
        return Err(
            "progress must be an object, and each acceptance criterion {id, description, required}"
                .to_owned(),
        );
    }
    let fields = serde_json::from_value::<Fields>(progress.clone())
        .map_err(|error| format!("progress: {error}"))?;
    let Ok(Value::Object(set)) = serde_json::to_value(fields) else {
        unreachable!("progress fields serialize to an object");
    };
    if set.is_empty() {
        return Err(
            "progress needs one of goal, acceptanceCriteria, latestChange or nextAction".to_owned(),
        );
    }
    Ok(set)
}

/// Merges `put.progress` into its task's progress, and records the result in the trace.
pub(crate) fn put(
    db: &mut Connection,
    put: &Put,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let set = match checked(&put.progress) {
        Ok(set) => set,
        Err(error) => return Ok(Err(error)),
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = tx
        .query_row(
            "SELECT progress FROM tasks WHERE id = ?1",
            [put.task],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    let Some(stored) = stored else {
        return Ok(Err(format!("no task {}", put.task)));
    };
    let mut progress: Map<String, Value> = stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    progress.extend(set);
    let progress = Value::Object(progress);
    tx.execute(
        "UPDATE tasks SET progress = ?2 WHERE id = ?1",
        rusqlite::params![put.task, progress.to_string()],
    )?;
    trace::progressed(&tx, trace, put.task, &progress)?;
    tx.commit()?;
    Ok(Ok(format!("task {} has its progress", put.task)))
}

/// The `progress` tool. It skips `tools::call`, since it does not end the turn: it stays open after
/// an ask, a wait or a result, and withdraws nothing.
pub(crate) fn tool(
    db: &mut Connection,
    call: &Call,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let task = db
        .query_row(
            "SELECT task FROM runs WHERE ended IS NULL AND session = ?1",
            [&call.session],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(task) = task else {
        return Ok(Err(format!("no live run has session {}", call.session)));
    };
    let progress = call.arguments.clone();
    put(db, &Put { task, progress }, trace)
}
