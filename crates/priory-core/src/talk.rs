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

/// The longest text a task may send another, in characters.
const TEXT_LIMIT: usize = crate::debate::ARGUMENT_LIMIT;

/// The caller's live run, its task, the task's key and whether the run is a debate turn.
fn caller(tx: &Transaction, session: &str) -> rusqlite::Result<Option<(i64, i64, String, bool)>> {
    tx.query_row(
        "SELECT runs.id, tasks.id, tasks.key, runs.debate IS NOT NULL
         FROM runs JOIN tasks ON tasks.id = runs.task
         WHERE runs.session = ?1 AND runs.ended IS NULL ORDER BY runs.id DESC LIMIT 1",
        [session],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .optional()
}

/// The outcome the live run of `task` already ended its step with, if it did.
fn stored(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    let run: Option<i64> = tx.query_row(
        "SELECT max(id) FROM runs WHERE task = ?1 AND ended IS NULL",
        [task],
        |row| row.get(0),
    )?;
    let Some(run) = run else {
        return Ok(None);
    };
    let result = tx
        .query_row("SELECT outcome FROM results WHERE run = ?1", [run], |row| {
            row.get(0)
        })
        .optional()?;
    match result {
        Some(outcome) => Ok(Some(outcome)),
        None => crate::gates::grading(tx, run),
    }
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
    let paired = crate::news::paired(tx, task, other)?;
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
    let length = text.chars().count();
    if length > TEXT_LIMIT {
        return Err(format!(
            "{field} is {length} characters; it is at most {TEXT_LIMIT}"
        ));
    }
    Ok((task, text.to_owned()))
}

/// Queues `text` for the other task's agent as a person's `say` is: typed after a running agent's
/// turn, waking a sleeping task, held during a debate, or on its open step ask while a person
/// decides. A running agent also reads it on its next tool call. A held call that reads it withdraws
/// the prompt, unless that prompt carries more.
fn deliver(
    tx: &Transaction,
    other: i64,
    state: &str,
    text: &str,
    factory: &Factory,
) -> rusqlite::Result<Result<(), String>> {
    let queued = |tx: &Transaction| -> rusqlite::Result<Option<i64>> {
        tx.query_row(
            "SELECT max(id) FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0",
            [other],
            |row| row.get(0),
        )
    };
    let before = queued(tx)?;
    if let Err(refused) = crate::tools::say_or_hold(tx, other, text, state, &factory.workflows)? {
        return Ok(Err(refused));
    }
    if state == "running" {
        let alone = match before {
            None => queued(tx)?,
            Some(_) => None,
        };
        crate::news::tell(tx, other, None, text, alone)?;
    }
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
    let Some((run, task, key, debater)) = caller(&tx, session)? else {
        return Ok(Err(format!("no live run has session {session}")));
    };
    if debater {
        return Ok(Err(crate::debate::ENDS_WITH_POSITION.to_owned()));
    }
    if let Some(outcome) = stored(&tx, task)? {
        return Ok(Err(format!(
            "run {run} already has a result, {outcome}; a result is final"
        )));
    }
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
        "Task {task} ({key}) says:\n{}\n{}\nAnswer it with the message tool, giving task {task}.",
        crate::consult::DATA,
        crate::consult::quoted(&text)
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

/// The branches of `task`, its own first, then those named after it and those of
/// its open pull requests, and why the other
/// task could not take them all over from origin, if it could not.
fn branches(tx: &Transaction, task: i64) -> rusqlite::Result<Result<Vec<String>, String>> {
    let found: (Option<String>, Option<String>) = tx.query_row(
        "SELECT worktree, branch FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (Some(worktree), Some(own)) = found else {
        return Ok(Ok(Vec::new()));
    };
    let (home, worktree) = (runtime::home(), Path::new(&worktree));
    let pulls = crate::stack::of(tx, task)?;
    // A merged or closed pull request's branch may be gone from origin for good.
    let over = |branch: &String| {
        pulls
            .iter()
            .any(|pull| pull.branch == *branch && pull.state != "open")
    };
    let stacked = match runtime::stacked(&home, worktree, &own) {
        Ok(stacked) => stacked,
        Err(error) => return Ok(Err(error)),
    };
    let mut branches = vec![own];
    branches.extend(stacked.into_iter().filter(|branch| !over(branch)));
    for pull in pulls.iter().filter(|pull| pull.state == "open") {
        if !branches.contains(&pull.branch) {
            branches.push(pull.branch.clone());
        }
    }
    Ok(runtime::unpushed(&home, worktree, &branches))
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
    let (to, summary) = match read(arguments, "hand_over", "summary") {
        Ok(read) => read,
        Err(refused) => return Ok((Err(refused), None)),
    };
    let (to_key, state) = match other(tx, task, to)? {
        Ok(found) => found,
        Err(refused) => return Ok((Err(refused), None)),
    };
    if ["held", "blocked"].contains(&state.as_str()) {
        return Ok((
            Err(format!(
                "task {to} is {state}, so it would not continue the work: ask the person with the ask tool"
            )),
            None,
        ));
    }
    if let Some(outcome) = stored(tx, to)? {
        return Ok((
            Err(format!(
                "task {to} already ended its step with {outcome}, so it may never read this: message it, or ask the person with the ask tool"
            )),
            None,
        ));
    }
    let branches = match branches(tx, task)? {
        Ok(branches) => branches,
        Err(refused) => return Ok((Err(refused), None)),
    };
    let key: String = tx.query_row("SELECT key FROM tasks WHERE id = ?1", [task], |row| {
        row.get(0)
    })?;
    let pushed = match branches.as_slice() {
        [] => "It pushed no branch.".to_owned(),
        branches => format!("On origin: {}.", branches.join(", ")),
    };
    let text = format!(
        "Task {task} ({key}) stopped and handed its work over to you. {pushed} It says:\n{}\n{}",
        crate::consult::DATA,
        crate::consult::quoted(&summary)
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
