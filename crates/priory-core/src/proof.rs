//! Step proofs: what the daemon checks before it accepts a model step's outcome.

use crate::trace::{self, Trace};
use priory_factory::factory::{Proof, Step};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use std::path::Path;

/// A line from `priory gate`, which runs the factory's gate in a run's task worktree.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "gate", rename_all = "lowercase")]
pub enum Line {
    /// Asks for the gate command, the worktree to run it in and the commit it runs at.
    Start { session: String },
    /// Records one gate run of the run whose session is `session`.
    Ran {
        session: String,
        commit: String,
        exit: i32,
        duration_ms: u64,
    },
}

/// What `priory gate` runs: `command` with `sh -c` in `worktree`, at `commit`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Started {
    pub command: String,
    pub worktree: String,
    pub commit: String,
}

/// Applies `line`: replies to a start with [`Started`] as JSON, and records a gate run in the
/// trace.
pub(crate) fn gate(
    db: &mut rusqlite::Connection,
    line: &Line,
    trace: &Trace,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session = match line {
        Line::Start { session } | Line::Ran { session, .. } => session,
    };
    let Some((run, task, worktree, branch)) = tx
        .query_row(
            "SELECT runs.id, runs.task, tasks.worktree, tasks.branch
             FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.ended IS NULL AND runs.session = ?1",
            [session],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(Err(format!("no live run has session {session}")));
    };
    match line {
        Line::Start { .. } => {
            let Some(command) = trace.factory.gate() else {
                return Ok(Err("factory.toml sets no `gate`".to_owned()));
            };
            let (Some(worktree), Some(branch)) = (worktree, branch) else {
                return Ok(Err(format!("task {task} has no worktree")));
            };
            let Some(commit) = head(Path::new(&worktree), &branch) else {
                return Ok(Err(format!("branch {branch} of task {task} has no commit")));
            };
            let started = Started {
                command: command.to_owned(),
                worktree,
                commit,
            };
            Ok(Ok(
                serde_json::to_string(&started).expect("a start serializes")
            ))
        }
        Line::Ran {
            commit,
            exit,
            duration_ms,
            ..
        } => {
            trace::gate_ran(&tx, trace, task, run, commit, *exit, *duration_ms)?;
            tx.commit()?;
            Ok(Ok(format!(
                "gate run recorded for task {task}: exit {exit} at {commit}"
            )))
        }
    }
}

/// Why `step` cannot take `outcome` for `task` yet, if its proof does not hold.
pub(crate) fn refused(
    tx: &Transaction,
    task: i64,
    step: &Step,
    outcome: &str,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Option<String>> {
    let Some(Proof::GatePassed) = step.proof.get(outcome) else {
        return Ok(None);
    };
    let (worktree, branch): (Option<String>, Option<String>) = tx.query_row(
        "SELECT worktree, branch FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let Some(at) = worktree
        .zip(branch)
        .and_then(|(worktree, branch)| head(Path::new(&worktree), &branch))
    else {
        return Ok(Some(format!(
            "outcome {outcome} needs a gate that passed at the task's head, and task {task} has no commit"
        )));
    };
    let needs = format!("outcome {outcome} needs a gate that passed at the head {at}");
    let again = "run `priory gate` in the worktree, then send the result again";
    let last = |at_head: bool| {
        tx.query_row(
            "SELECT json_extract(event, '$.data.commit'), json_extract(event, '$.data.exit')
             FROM trace WHERE type = 'factory.gate.ran' AND json_extract(event, '$.data.task') = ?1
               AND (NOT ?2 OR json_extract(event, '$.data.commit') = ?3)
             ORDER BY seq DESC LIMIT 1",
            params![task.to_string(), at_head, at],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
    };
    Ok(match (last(true)?, last(false)?) {
        (Some((_, 0)), _) => None,
        (Some((_, exit)), _) => Some(format!(
            "{needs}, and the last gate run at it exited {exit}: {again}"
        )),
        (None, Some((commit, _))) => Some(format!(
            "{needs}, and the last gate run for task {task} was at another commit, {commit}: {again}"
        )),
        (None, None) => Some(format!("{needs}, and no gate ran for task {task}: {again}")),
    })
}
