use crate::tools::Call;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

/// Subscribes `task` to `resource`: the task owns it until it closes, the task retires, or
/// `unsubscribe`. Refused while another active task owns it; a done task's subscription yields.
pub fn subscribe(
    db: &mut Connection,
    task: i64,
    resource: &str,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let state: Option<String> = tx
        .query_row("SELECT state FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })
        .optional()?;
    let refused = match state.as_deref() {
        None => Some(format!("no task {task}")),
        Some(state @ ("failed" | "dropped")) => Some(format!(
            "task {task} is {state}: a retired task subscribes to nothing"
        )),
        Some(_) => crate::state::taken(&tx, task, resource)?,
    };
    if let Some(refused) = refused {
        return Ok(Err(refused));
    }
    tx.execute(
        "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
        params![task, resource],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO subscriptions (task, resource) VALUES (?1, ?2)",
        params![task, resource],
    )?;
    if state.as_deref() != Some("done") {
        tx.execute(
            "DELETE FROM subscriptions WHERE resource = ?2 AND task != ?1
             AND task IN (SELECT id FROM tasks WHERE state = 'done')",
            params![task, resource],
        )?;
    }
    tx.commit()?;
    Ok(Ok(format!("task {task} subscribes to {resource}")))
}

/// Ends `task`'s subscription to `resource`, and its ownership unless the resource is its key or
/// an open wait of it names the resource.
pub fn unsubscribe(
    db: &mut Connection,
    task: i64,
    resource: &str,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ended = tx.execute(
        "DELETE FROM subscriptions WHERE task = ?1 AND resource = ?2",
        params![task, resource],
    )?;
    if ended == 0 {
        return Ok(Err(format!("task {task} does not subscribe to {resource}")));
    }
    tx.execute(
        "DELETE FROM owns WHERE task = ?1 AND resource = ?2
         AND resource IS NOT (SELECT key FROM tasks WHERE id = ?1)
         AND NOT EXISTS (SELECT 1 FROM waits WHERE task = ?1 AND resource = ?2
           AND fired IS NULL AND withdrawn IS NULL)",
        params![task, resource],
    )?;
    tx.commit()?;
    Ok(Ok(format!("task {task} unsubscribed from {resource}")))
}

/// The `subscribe` and `unsubscribe` tools. They skip `tools::call`, since they do not end the
/// turn: they stay open after an ask, a wait or a result, and withdraw nothing.
pub(crate) fn tool(db: &mut Connection, call: &Call) -> rusqlite::Result<Result<String, String>> {
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
    let resource = match crate::tools::resource(&call.arguments["on"]) {
        Ok(resource) => resource,
        Err(error) => return Ok(Err(error)),
    };
    match call.tool.as_str() {
        "subscribe" => subscribe(db, task, &resource),
        _ => unsubscribe(db, task, &resource),
    }
}

/// Whether `resource`, as `priory subscribe` takes it, names a pull request or issue, a Linear
/// issue or a Slack thread.
pub fn known(resource: &str) -> bool {
    let github = resource
        .strip_prefix("github:")
        .and_then(|pull| pull.rsplit_once('#'))
        .is_some_and(|(repo, number)| repo.contains('/') && number.parse::<u64>().is_ok());
    let linear = resource
        .strip_prefix("linear:")
        .is_some_and(|issue| !issue.is_empty());
    let slack = resource
        .strip_prefix("slack:")
        .and_then(|thread| thread.split_once(':'))
        .is_some_and(|(channel, ts)| !channel.is_empty() && !ts.is_empty());
    github || linear || slack
}
