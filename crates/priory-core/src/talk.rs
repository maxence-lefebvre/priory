//! Two tasks on the same work, such as a duplicate and the issue it duplicates, agree which one
//! continues: `message` reaches the other task's agent, and `hand_over` gives it the work and drops
//! the caller (docs/daemon/routing.md#two-tasks-on-the-same-work).

use priory_factory::factory::Factory;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::Value;

/// The caller's task and its key, from its live run.
fn caller(tx: &Transaction, session: &str) -> rusqlite::Result<Option<(i64, String)>> {
    tx.query_row(
        "SELECT tasks.id, tasks.key FROM runs JOIN tasks ON tasks.id = runs.task
         WHERE runs.session = ?1 AND runs.ended IS NULL ORDER BY runs.id DESC LIMIT 1",
        [session],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// The other task's key and state, or why it cannot hear `task`.
fn other(
    tx: &Transaction,
    task: i64,
    other: i64,
) -> rusqlite::Result<Result<(String, String), String>> {
    if task == other {
        return Ok(Err("a task does not message itself".to_owned()));
    }
    let found: Option<(String, String)> = tx
        .query_row(
            "SELECT key, state FROM tasks WHERE id = ?1",
            [other],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match found {
        None => Err(format!("no task {other}")),
        Some((_, state)) if ["done", "failed", "dropped"].contains(&state.as_str()) => {
            Err(format!("task {other} is {state}"))
        }
        Some(found) => Ok(found),
    })
}

/// The task id and text of `arguments`, whose text field is `field`, or why they are wrong.
fn read(arguments: &Value, tool: &str, field: &str) -> Result<(i64, String), String> {
    if let Some(name) = arguments.as_object().and_then(|fields| {
        fields
            .keys()
            .find(|name| !["task", field].contains(&name.as_str()))
    }) {
        return Err(format!("{tool} takes task and {field}, not {name}"));
    }
    let task = arguments["task"].as_i64().ok_or("task must be a task id")?;
    let text = arguments[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{field} must be text"))?;
    Ok((task, text.to_owned()))
}

/// Puts `text` before the other task's agent: during its turn when it runs, as its next prompt,
/// or on its open step ask while a person decides.
fn deliver(
    tx: &Transaction,
    other: i64,
    state: &str,
    text: &str,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let said = crate::tools::say_or_hold(tx, other, text, state, &factory.workflows)?;
    if said.is_ok() {
        crate::news::tell(tx, other, None, text)?;
    }
    Ok(said)
}

/// The `message` tool: `text` from the caller's task to the agent of another active task.
pub(crate) fn message(
    db: &mut Connection,
    session: &str,
    arguments: &Value,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let (to, text) = match read(arguments, "message", "text") {
        Ok(read) => read,
        Err(refused) => return Ok(Err(refused)),
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some((task, key)) = caller(&tx, session)? else {
        return Ok(Err(format!("no live run has session {session}")));
    };
    let state = match other(&tx, task, to)? {
        Ok((_, state)) => state,
        Err(refused) => return Ok(Err(refused)),
    };
    let text = format!(
        "Task {task} ({key}) says: {text}\nAnswer it with the message tool, giving task {task}."
    );
    let said = deliver(&tx, to, &state, &text, factory)?;
    if said.is_ok() {
        tx.commit()?;
    }
    Ok(said.map(|_| format!("task {to} has your message")))
}

/// The `hand_over` tool: tells the other task what it takes over, records that it takes up the
/// caller's task, and drops the caller's task with a link to it. Returns the caller's tmux session
/// to kill once it has the reply.
pub(crate) fn hand_over(
    tx: &Transaction,
    task: i64,
    arguments: &Value,
    factory: &Factory,
) -> rusqlite::Result<(Result<String, String>, Option<String>)> {
    let (to, summary) = match read(arguments, "hand_over", "summary") {
        Ok(read) => read,
        Err(refused) => return Ok((Err(refused), None)),
    };
    let (to_key, state) = match other(tx, task, to)? {
        Ok(found) => found,
        Err(refused) => return Ok((Err(refused), None)),
    };
    let key: String = tx.query_row("SELECT key FROM tasks WHERE id = ?1", [task], |row| {
        row.get(0)
    })?;
    let text = format!("Task {task} ({key}) stopped and handed its work over to you: {summary}");
    if let Err(refused) = deliver(tx, to, &state, &text, factory)? {
        return Ok((Err(refused), None));
    }
    crate::relate::insert(tx, to, "takes-up", task, "agent")?;
    let tmux = crate::tools::cancel(tx, task, &format!("handed over to task {to} ({to_key})"))?;
    Ok((
        Ok(format!("task {task} dropped: task {to} continues it")),
        tmux,
    ))
}
