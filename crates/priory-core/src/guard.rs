//! The daemon's answer to the tool guard: whether a run may make an irreversible call.

use crate::trace::{self, Trace};
use priory_factory::factory::Effect;
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
}

/// Allows the call when its workflow allows the guard, or when the task's step is irreversible
/// and autonomous or approved at the task's current commit. A denial is the reason the agent
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
            "SELECT runs.id, runs.task, tasks.workflow, tasks.step, tasks.worktree, tasks.branch
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
                })
            },
        )
        .optional()?;
    let Some(run) = run else {
        return Ok(Err(denied("this session runs no task step")));
    };
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
    let step = workflow.and_then(|flow| flow.step(&run.step));
    let why = match step {
        Some(step) if step.effect == Effect::Irreversible && step.autonomous.is_some() => {
            return Ok(Ok(format!("step {} is autonomous", run.step)));
        }
        Some(step) if step.effect == Effect::Irreversible => {
            let head = run
                .worktree
                .as_deref()
                .zip(run.branch.as_deref())
                .and_then(|(worktree, branch)| runtime::head(Path::new(worktree), branch));
            let approved: Option<Option<String>> = db
                .query_row(
                    "SELECT commit_sha FROM approvals WHERE task = ?1 AND step = ?2
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
        _ => format!(
            "step `{}` is not irreversible, so no person approved a call like this",
            run.step
        ),
    };
    let reason = denied(&why);
    let tx = db.transaction()?;
    trace::tool_denied(&tx, trace, run.task, run.id, call, &reason)?;
    tx.commit()?;
    Ok(Err(reason))
}
