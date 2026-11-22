//! The daemon's answer to the tool guard: whether a run may make an irreversible call.

use crate::trace::{self, Trace};
use priory_factory::factory::Effect;
use priory_factory::guard::{NEED_APPROVAL, SIGN_IN, UNREADABLE};
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

/// The hook's line: `{"guard": {...}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Line {
    pub guard: Call,
}

/// A guarded call a run is about to make.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub session: String,
    pub guard: String,
    pub call: String,
    pub tool: String,
    /// The same tool call also moves the branch's commit, as `git commit && gh pr merge` does.
    #[serde(default)]
    pub moves_head: bool,
}

struct Run {
    id: i64,
    task: i64,
    workflow: String,
    step: String,
    worktree: Option<String>,
    branch: Option<String>,
    agent: Option<String>,
}

/// Allows the call when its workflow allows the guard, or when the task's step is irreversible,
/// names the guard in its own `guard.allow`, and is approved at the task's current commit or
/// autonomous; `NEED_APPROVAL` guards need the approval. A denial is the reason the agent
/// reads, and it is recorded on the task as `factory.tool.denied`.
pub(crate) fn decide(
    db: &mut Connection,
    call: &Call,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let denied = |why: &str| {
        format!(
            "Priory denied `{}`: it is irreversible (guard `{}`) and {why}. Do not try it another way. If a person must do or approve it, call the ask tool of the priory MCP server; otherwise end the step with the result tool.",
            call.call, call.guard
        )
    };
    let run = db
        .query_row(
            "SELECT runs.id, runs.task, tasks.workflow, tasks.step, tasks.worktree, tasks.branch, runs.agent
             FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.session = ?1 AND runs.ended IS NULL ORDER BY runs.id DESC LIMIT 1",
            [&call.session],
            |row| {
                Ok(Run {
                    id: row.get(0)?,
                    task: row.get(1)?,
                    workflow: row.get(2)?,
                    step: row.get(3)?,
                    worktree: row.get(4)?,
                    branch: row.get(5)?,
                    agent: row.get(6)?,
                })
            },
        )
        .optional()?;
    let Some(run) = run else {
        return Ok(Err(denied("this session runs no task step")));
    };
    // Fail closed: what the guard cannot read, nothing grants.
    if call.guard == UNREADABLE {
        let reason = denied("is too deeply nested for the guard to read");
        let tx = db.transaction()?;
        trace::tool_denied(&tx, trace, run.task, run.id, call, &reason)?;
        tx.commit()?;
        return Ok(Err(reason));
    }
    if call.guard == SIGN_IN {
        return sign_in(db, call, &run, trace);
    }
    let workflow = trace
        .factory
        .workflows
        .get(&run.workflow)
        .map(|flow| &flow.value);
    if workflow.is_some_and(|flow| flow.guard.allow.contains(&call.guard)) {
        return Ok(Ok(format!(
            "workflow {} allows {}",
            run.workflow, call.guard
        )));
    }
    let step = workflow
        .and_then(|flow| flow.step(&run.step))
        .filter(|step| step.effect == Effect::Irreversible);
    let why = match step {
        None => format!(
            "step `{}` is not irreversible, so no person approved a call like this",
            run.step
        ),
        Some(step) if !step.guard.allow.contains(&call.guard) => format!(
            "step `{}` does not name `{}` in its `guard.allow`",
            run.step, call.guard
        ),
        Some(step) if step.autonomous.is_some() => {
            if !NEED_APPROVAL.contains(&call.guard.as_str()) {
                return Ok(Ok(format!("step {} is autonomous", run.step)));
            }
            format!(
                "only a person's approval grants it, and step `{}` is autonomous",
                run.step
            )
        }
        Some(_) => {
            let head = run
                .worktree
                .as_deref()
                .zip(run.branch.as_deref())
                .and_then(|(worktree, branch)| runtime::head(Path::new(worktree), branch));
            let approved: Option<Option<String>> = db
                .query_row(
                    "SELECT commit_sha FROM approvals WHERE task = ?1 AND step = ?2
                     AND generation IS (SELECT generation FROM tasks WHERE id = ?1)
                     ORDER BY id DESC LIMIT 1",
                    params![run.task, run.step],
                    |row| row.get(0),
                )
                .optional()?;
            match approved {
                Some(commit) if head.is_some() && commit == head && !call.moves_head => {
                    return Ok(Ok(format!("step {} is approved", run.step)));
                }
                Some(_) if call.moves_head => format!(
                    "the same call also moves the branch's commit, which step `{}`'s approval is bound to: run it in a call of its own",
                    run.step
                ),
                Some(commit) => format!(
                    "step `{}` was approved at commit {}, but the task's branch is now at {}",
                    run.step,
                    commit.as_deref().unwrap_or("-"),
                    head.as_deref().unwrap_or("-")
                ),
                None => format!("no person approved step `{}`", run.step),
            }
        }
    };
    let reason = denied(&why);
    let tx = db.transaction()?;
    trace::tool_denied(&tx, trace, run.task, run.id, call, &reason)?;
    tx.commit()?;
    Ok(Err(reason))
}

/// An unattended run never signs in: the denial tells it to go on and records the connector as a
/// to-do for a person, unless the run's agent opted in with `connectorSignIn`.
fn sign_in(
    db: &mut Connection,
    call: &Call,
    run: &Run,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let opted_in = run
        .agent
        .as_deref()
        .and_then(|agent| trace.factory.agents.get(agent))
        .is_some_and(|agent| agent.value.connector_sign_in);
    if opted_in {
        return Ok(Ok(format!(
            "agent {} may sign in",
            run.agent.as_deref().unwrap_or("")
        )));
    }
    let connector = connector(&call.tool);
    let reason = format!(
        "Priory denied `{}`: an unattended run never signs in to `{connector}`, because signing in clears the token every later run needs. Carry on with your other sources, and name `{connector}` in your result as a connector a person must sign in to.",
        call.call
    );
    let tx = db.transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO todos (text, connector) VALUES (?1, ?2)",
        params![format!("Sign in to {connector}"), connector],
    )?;
    crate::consult::signed_in(&tx, run.task, connector)?;
    trace::tool_denied(&tx, trace, run.task, run.id, call, &reason)?;
    tx.commit()?;
    Ok(Err(reason))
}

/// The MCP server of a tool named `mcp__<server>__<tool>`.
pub(crate) fn connector(tool: &str) -> &str {
    let name = tool.strip_prefix("mcp__").unwrap_or(tool);
    name.rsplit_once("__").map_or(name, |(server, _)| server)
}
