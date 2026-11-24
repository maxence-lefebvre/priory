//! Two tasks on the same work, such as a duplicate and the issue it duplicates, agree which one
//! continues: `message` reaches the other task's agent, and `hand_over` gives it the work and drops
//! the caller (docs/daemon/routing.md#two-tasks-on-the-same-work). Only a pair that a duplicate's
//! event made may talk, so a run steered by an issue's text cannot reach any other task.

use priory_factory::factory::Factory;
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use std::path::Path;

/// How many messages a pair of tasks may send each other in an hour, so two agents cannot wake
/// each other without end.
const MESSAGES_AN_HOUR: i64 = 6;

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

/// The other task's key and state, or why `task` cannot reach it: it is itself, missing, ended, or
/// no duplicate paired the two. A relation does not count: an agent can record one with its tool or
/// `priory relate`, and a request's text with a link.
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
    let paired: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM pairs
           WHERE (task = ?1 AND other = ?2) OR (task = ?2 AND other = ?1))",
        params![task, other],
        |row| row.get(0),
    )?;
    Ok(match found {
        None => Err(format!("no task {other}")),
        Some((_, state)) if ["done", "failed", "dropped"].contains(&state.as_str()) => {
            Err(format!("task {other} is {state}"))
        }
        Some(_) if !paired => Err(format!(
            "task {other} is not paired with task {task}: an agent may reach only the task that a duplicate's news names"
        )),
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

/// Puts `text` before the other task's agent once. A running agent reads it on its next tool call,
/// and else after its turn from a prompt of its own, which that call withdraws. Otherwise it wakes
/// the task, or goes on its open step ask while a person decides.
fn deliver(
    tx: &Transaction,
    other: i64,
    state: &str,
    text: &str,
    factory: &Factory,
) -> rusqlite::Result<Result<(), String>> {
    if state != "running" {
        return crate::tools::say_or_hold(tx, other, text, state, &factory.workflows)
            .map(|said| said.map(drop));
    }
    tx.execute(
        "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
        params![other, text],
    )?;
    let prompt = tx.last_insert_rowid();
    crate::news::tell(tx, other, None, text, Some(prompt))?;
    Ok(Ok(()))
}

/// The `message` tool: `text` from the caller's task to the agent of its paired task.
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
    let sent: i64 = tx.query_row(
        "SELECT count(*) FROM messages WHERE ((task = ?1 AND other = ?2) OR (task = ?2 AND other = ?1))
         AND julianday(at) > julianday('now', '-1 hour')",
        params![task, to],
        |row| row.get(0),
    )?;
    if sent >= MESSAGES_AN_HOUR {
        return Ok(Err(format!(
            "task {task} and task {to} sent each other {sent} messages in the last hour: if you still disagree, ask the person with the ask tool"
        )));
    }
    let text = format!(
        "Task {task} ({key}) says: {text}\nAnswer it with the message tool, giving task {task}."
    );
    if let Err(refused) = deliver(&tx, to, &state, &text, factory)? {
        return Ok(Err(refused));
    }
    tx.execute(
        "INSERT INTO messages (task, other, at) VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![task, to],
    )?;
    tx.commit()?;
    Ok(Ok(format!("task {to} has your message")))
}

/// Why the branch of `task` holds commits its remote lacks, if it does: the other task takes over
/// only what is pushed.
fn unpushed(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    let found: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT worktree, branch FROM tasks WHERE id = ?1",
            [task],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((Some(worktree), Some(branch))) = found else {
        return Ok(None);
    };
    let home = runtime::home();
    let local = runtime::kept_head(&home, Path::new(&worktree), &branch);
    let pushed = runtime::pushed_head(&home, Path::new(&worktree), &branch);
    Ok((local.is_some() && local != pushed).then(|| {
        format!("branch {branch} has commits origin lacks: push it first, since the other task takes over only what is pushed")
    }))
}

/// The `hand_over` tool: tells the related task what it takes over, records that it takes up the
/// caller's task, and drops the caller's task with a link to it. Returns the caller's tmux session
/// to kill once it has the reply.
pub(crate) fn hand_over(
    tx: &Transaction,
    run: &crate::tools::Run,
    arguments: &Value,
    factory: &Factory,
) -> rusqlite::Result<(Result<String, String>, Option<String>)> {
    let task = run.task;
    if run.debate.is_some() {
        return Ok((Err(crate::debate::ENDS_WITH_POSITION.to_owned()), None));
    }
    let (to, summary) = match read(arguments, "hand_over", "summary") {
        Ok(read) => read,
        Err(refused) => return Ok((Err(refused), None)),
    };
    let (to_key, state) = match other(tx, task, to)? {
        Ok(found) => found,
        Err(refused) => return Ok((Err(refused), None)),
    };
    if let Some(refused) = unpushed(tx, task)? {
        return Ok((Err(refused), None));
    }
    let (key, branch): (String, Option<String>) = tx.query_row(
        "SELECT key, branch FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let branch = branch.map_or_else(String::new, |branch| format!(" Its branch is {branch}."));
    let text = format!(
        "Task {task} ({key}) stopped and handed its work over to you.{branch} It says: {summary}"
    );
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
