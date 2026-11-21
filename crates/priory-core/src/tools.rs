use crate::command;
use crate::route;
use crate::trace::{self, Trace};
use priory_factory::explain;
use priory_factory::factory::{
    Decider, Effect, Factory, Harness, Intervention, Loaded, Step, StepAfter, Workflow,
    lead_workflow,
};
use priory_intake::intake::{
    CHECK_RUNS, CHECKED, COMMENTED, ISSUE_CLOSED, ISSUE_COMMENTED, PASSING, PR_CLOSED as CLOSED,
    REVIEWED, THREAD_COMMENTED,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

type Workflows = BTreeMap<String, Loaded<Workflow>>;

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

/// A tool call from a run's `priory mcp`, as it arrives on the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Call {
    pub tool: String,
    pub session: String,
    #[serde(default)]
    pub arguments: Value,
}

/// A person's answer to an ask, as `priory answer` sends it on the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Answer {
    pub ask: i64,
    pub text: String,
    pub revision: i64,
    /// The person confirmed that this answer runs the ask's command.
    #[serde(default)]
    pub run: bool,
    /// Where the answer came from: `cli` or `serve`.
    #[serde(default)]
    pub by: String,
}

/// A live run with its task's workflow.
struct Run {
    id: i64,
    task: i64,
    step: String,
    agent: Option<String>,
    workflow: String,
    tmux: String,
    session: String,
}

/// The live run of the harness session `session`.
fn run(tx: &Transaction, session: &str) -> rusqlite::Result<Option<Run>> {
    tx.query_row(
        "SELECT runs.id, runs.task, runs.step, runs.agent, tasks.workflow, runs.tmux, runs.session
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL AND runs.session = ?1",
        [session],
        |row| {
            Ok(Run {
                id: row.get(0)?,
                task: row.get(1)?,
                step: row.get(2)?,
                agent: row.get(3)?,
                workflow: row.get(4)?,
                tmux: row.get(5)?,
                session: row.get(6)?,
            })
        },
    )
    .optional()
}

fn step<'a>(workflows: &'a Workflows, workflow: &str, id: &str) -> Option<&'a Step> {
    workflows.get(workflow)?.value.step(id)
}

fn text(arguments: &Value, name: &str) -> Result<String, String> {
    arguments[name]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{name} must be a string"))
}

/// The fields that describe an ask beyond its question: title, text, what waits and next action.
fn described(arguments: &Value) -> Result<[Option<String>; 4], String> {
    Ok([
        optional(arguments, "title")?,
        optional(arguments, "text")?,
        optional(arguments, "whatWaits")?,
        optional(arguments, "nextAction")?,
    ])
}

/// `commit` in lower case, as GitHub names a pull request head; an abbreviated SHA could never
/// match one.
fn full_sha(commit: &str) -> Result<String, String> {
    if matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(commit.to_ascii_lowercase());
    }
    Err(format!(
        "commit {commit:?} is not a full commit SHA: send the 40 characters that git rev-parse HEAD prints"
    ))
}

/// A result's `branches`, bottom first: the task's own branch last, each lower one named after it,
/// so `open-pr` never pushes a branch of another task or the trunk.
fn branches(value: &Value, own: Option<&str>) -> Result<Option<Vec<String>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let wrong = || {
        format!(
            "branches must list the stack's branches bottom first, each once, ending with the task's branch {}, each lower one named {}-<name>",
            own.unwrap_or("-"),
            own.unwrap_or("-")
        )
    };
    let branches: Vec<String> = serde_json::from_value(value.clone()).map_err(|_| wrong())?;
    let Some((top, lower)) = branches.split_last() else {
        return Err(wrong());
    };
    let named = |branch: &String| {
        own.and_then(|own| branch.strip_prefix(own)?.strip_prefix('-'))
            .is_some_and(|name| !name.is_empty())
    };
    let unique = branches
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == branches.len();
    if Some(top.as_str()) != own || !lower.iter().all(named) || !unique {
        return Err(wrong());
    }
    Ok(Some(branches))
}

fn optional(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    match &arguments[name] {
        Value::Null => Ok(None),
        _ => text(arguments, name).map(Some),
    }
}

/// Applies a tool call from the run whose harness session is `call.session`. The inner error is
/// the tool's error, returned to the agent with nothing stored. Also returns the tmux session of a
/// run the call ended, to kill once the agent has its reply.
pub(crate) fn call(
    db: &mut Connection,
    call: &Call,
    trace: &Trace,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<(Result<String, String>, Option<String>)> {
    let factory = trace.factory;
    let workflows = &factory.workflows;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = run(&tx, &call.session)? else {
        return Ok((
            Err(format!("no live run has session {}", call.session)),
            None,
        ));
    };
    if let Some(outcome) = tx
        .query_row(
            "SELECT outcome FROM results WHERE run = ?1",
            [run.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok((
            Err(format!(
                "run {} already has a result, {outcome}; a result is final",
                run.id
            )),
            None,
        ));
    }
    // An ask and a wait may sit side by side: the first answer or event wins.
    let open_ask = tx
        .query_row(
            "SELECT id FROM asks WHERE run = ?1 AND answer IS NULL AND withdrawn IS NULL",
            [run.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let open_wait = open_wait(&tx, run.id)?;
    let refused = match (call.tool.as_str(), open_ask, open_wait) {
        ("ask", Some(ask), _) | ("result" | "compact", Some(ask), _) => {
            Some(format!("run {} already waits on ask {ask}", run.id))
        }
        ("wait", _, Some(wait)) | ("result" | "compact", _, Some(wait)) => {
            Some(format!("run {} already waits on wait {wait}", run.id))
        }
        ("ask" | "wait", ..) if woken(&tx, &run)? => {
            Some("an answer or event for this run is queued: end your turn to read it".to_owned())
        }
        // A result or ask would end or sleep the task over feedback it has not read yet.
        ("result" | "ask", ..) if kept(&tx, run.task)? => {
            Some("feedback for this run is queued: end your turn to read it".to_owned())
        }
        // Its end would wake the task at whatever step the result moved it to.
        ("result", ..) if commanding(&tx, run.task)? => Some(
            "an answer's command for this task runs: end your turn, and its end wakes the task"
                .to_owned(),
        ),
        _ => None,
    };
    if let Some(refused) = refused {
        return Ok((Err(refused), None));
    }
    // The run moves on from the asks and waits of earlier runs. A refused call rolls this back.
    tx.execute(
        "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'moved-on' WHERE task = ?1
         AND run IS NOT NULL AND run != ?2 AND answer IS NULL AND withdrawn IS NULL",
        params![run.task, run.id],
    )?;
    tx.execute(
        "UPDATE waits SET withdrawn = 1, withdrawn_reason = 'moved-on' WHERE task = ?1 AND run != ?2
         AND fired IS NULL AND withdrawn IS NULL AND NOT daemon",
        params![run.task, run.id],
    )?;
    let mut kill = None;
    let stored = match call.tool.as_str() {
        // A result moves the task, maybe to another agent's conversation, before the compact is typed.
        "result" if compact_queued(&tx, run.task)? => Err(
            "a compact is queued for this run; end your turn and call result after the compaction"
                .to_owned(),
        ),
        "result" => match lead_answer(&tx, &run, &call.arguments, trace, head)? {
            Ok(()) => result(&tx, &run, &call.arguments, workflows, head)?,
            Err(refused) => Err(refused),
        },
        "ask" => ask(&tx, &run, &call.arguments, factory)?,
        "wait" => {
            let (stored, ended) = wait(&tx, &run, &call.arguments, factory)?;
            kill = ended;
            stored
        }
        "compact" => compact(&tx, &run, &call.arguments, factory)?,
        tool => Err(format!("unknown tool {tool}")),
    };
    if stored.is_ok() {
        tx.execute("UPDATE runs SET nudgeable = 0 WHERE id = ?1", [run.id])?;
        tx.commit()?;
        return Ok((stored, kill));
    }
    Ok((stored, None))
}

fn result(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
    workflows: &Workflows,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Result<String, String>> {
    let fields = (|| {
        Ok::<_, String>((
            text(arguments, "outcome")?,
            text(arguments, "summary")?,
            optional(arguments, "evidence")?,
            optional(arguments, "commit")?,
            optional(arguments, "reason")?,
            PLAN_FIELDS
                .map(|(name, _)| optional(arguments, name))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?,
        ))
    })();
    let (outcome, summary, evidence, commit, reason, plan) = match fields {
        Ok(fields) => fields,
        Err(error) => return Ok(Err(error)),
    };
    let commit = match commit.as_deref().map(full_sha).transpose() {
        Ok(commit) => commit,
        Err(error) => return Ok(Err(error)),
    };
    let own: Option<String> = tx.query_row(
        "SELECT branch FROM tasks WHERE id = ?1",
        [run.task],
        |row| row.get(0),
    )?;
    let branches = match branches(&arguments["branches"], own.as_deref()) {
        Ok(branches) => branches,
        Err(error) => return Ok(Err(error)),
    };
    let merge_risk = match merge_risk(&outcome, &arguments["risk"]) {
        Ok(merge_risk) => merge_risk,
        Err(error) => return Ok(Err(error)),
    };
    let triage = match crate::triage::sorted(tx, run.task, &arguments["feedback"])? {
        Ok(triage) => triage,
        Err(error) => return Ok(Err(error)),
    };
    let Some(step) = step(workflows, &run.workflow, &run.step) else {
        return Ok(Err(format!(
            "step {} is not in workflow {}",
            run.step, run.workflow
        )));
    };
    let Some(next) = step.next.get(&outcome) else {
        let valid: Vec<_> = step.next.keys().map(String::as_str).collect();
        return Ok(Err(format!(
            "outcome {outcome:?} is not one of step {}'s outcomes: {}",
            step.id,
            valid.join(", ")
        )));
    };
    if let Some(refused) = crate::proof::refused(tx, run.task, step, &outcome, head)? {
        return Ok(Err(refused));
    }
    let gated = next == "done"
        || workflows
            .get(&run.workflow)
            .and_then(|flow| flow.value.step(next))
            .is_some_and(|s| s.after == Some(StepAfter::Merged));
    if gated
        && commit.is_none()
        && !all_merged(tx, run.task)?
        && let Some(resource) = pull_request(tx, run.task)?
    {
        return Ok(Err(format!(
            "task {} owns {resource}: send the result with the commit its head must match",
            run.task
        )));
    }
    if !arguments["batch"].is_null()
        && let Err(refused) = crate::batch::planned(tx, run.task, &arguments["batch"], &summary)?
    {
        return Ok(Err(refused));
    }
    tx.execute(
        "INSERT INTO results (run, task, step, outcome, summary, evidence, commit_sha, reason, merge_risk, inbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, (SELECT max(seq) FROM inbox))",
        params![
            run.id, run.task, run.step, outcome, summary, evidence, commit, reason, merge_risk
        ],
    )?;
    crate::triage::store(tx, run.task, run.id, &triage)?;
    for ((_, column), value) in PLAN_FIELDS.iter().zip(&plan) {
        tx.execute(
            &format!("UPDATE tasks SET {column} = coalesce(?2, {column}) WHERE id = ?1"),
            params![run.task, value],
        )?;
    }
    if let Some(branches) = branches {
        tx.execute(
            "INSERT INTO result_branches (run, branches) VALUES (?1, ?2)",
            params![run.id, Value::from(branches).to_string()],
        )?;
    }
    Ok(Ok(format!(
        "result stored: the task moves to {next} when this turn ends"
    )))
}

/// The outcomes of a risk step, lowest first: its verdict on the finished change.
pub const RISK_LEVELS: [&str; 3] = ["low", "medium", "high"];

/// What a risk step grades the change on, one line each.
pub const RISK_DIMENSIONS: [&str; 5] = [
    "ask",
    "blast_radius",
    "verification",
    "rollback",
    "detection",
];

/// The merge risk a result with `outcome` and `risk` stores, as JSON: the verdict and the line of
/// each dimension. `risk` needs the outcome its highest line has; without it, nothing is stored.
fn merge_risk(outcome: &str, risk: &Value) -> Result<Option<String>, String> {
    if risk.is_null() {
        return Ok(None);
    }
    let Some(verdict) = RISK_LEVELS.iter().position(|level| *level == outcome) else {
        return Err("risk needs a low, medium or high outcome: its verdict".to_owned());
    };
    let shape = format!(
        "risk must be an object with one line per dimension, {}, each its level, a colon and why, on one line",
        RISK_DIMENSIONS.join(", ")
    );
    let Some(lines) = risk.as_object() else {
        return Err(shape);
    };
    if lines.len() != RISK_DIMENSIONS.len() {
        return Err(shape);
    }
    let mut highest = 0;
    for dimension in RISK_DIMENSIONS {
        let level = lines
            .get(dimension)
            .and_then(Value::as_str)
            .filter(|line| !line.contains(['\n', '\r', '\t']))
            .and_then(|line| line.split_once(": "))
            .filter(|(_, why)| !why.trim().is_empty())
            .and_then(|(level, _)| RISK_LEVELS.iter().position(|known| *known == level));
        let Some(level) = level else {
            return Err(shape);
        };
        highest = highest.max(level);
    }
    if highest != verdict {
        return Err(format!(
            "outcome {outcome} must be the highest level of risk's lines: {}",
            RISK_LEVELS[highest]
        ));
    }
    let mut stored = lines.clone();
    stored.insert("verdict".to_owned(), outcome.into());
    Ok(Some(Value::Object(stored).to_string()))
}

/// The task's plan brief and verification, then its latest merge risk, as an ask's Markdown body.
fn ask_text(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    let parts = [plan_text(tx, task)?, risk_text(tx, task)?];
    let parts: Vec<_> = parts.into_iter().flatten().collect();
    Ok((!parts.is_empty()).then(|| parts.join("\n\n")))
}

/// The fields a plan result may carry beside its summary, with the task column each is kept in, in
/// the order an approval shows them.
const PLAN_FIELDS: [(&str, &str); 4] = [
    ("goal", "plan_goal"),
    ("testing", "plan_testing"),
    ("brief", "plan_brief"),
    ("verification", "verification"),
];

/// The plan fields a plan result proposed, as Markdown, if it sent any.
fn plan_text(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    let columns: Vec<_> = PLAN_FIELDS.iter().map(|(_, column)| *column).collect();
    let stored: Vec<Option<String>> = tx.query_row(
        &format!("SELECT {} FROM tasks WHERE id = ?1", columns.join(", ")),
        [task],
        |row| (0..columns.len()).map(|at| row.get(at)).collect(),
    )?;
    let lines: Vec<_> = PLAN_FIELDS
        .iter()
        .zip(stored)
        .filter_map(|((name, _), text)| {
            let (first, rest) = name.split_at(1);
            Some(format!("{}{rest}: {}", first.to_uppercase(), text?))
        })
        .collect();
    Ok((!lines.is_empty()).then(|| lines.join("\n\n")))
}

/// The prompt that entering `step` queues: the step's own, then the verification the plan promised.
fn entry_prompt(tx: &Transaction, task: i64, step: &Step) -> rusqlite::Result<String> {
    let verification: Option<String> = tx.query_row(
        "SELECT verification FROM tasks WHERE id = ?1",
        [task],
        |row| row.get(0),
    )?;
    let prompt = step_prompt(step);
    Ok(match verification {
        Some(verification) => format!("{prompt}\nProve the change with: {verification}"),
        None => prompt,
    })
}

/// The task's latest merge risk as an ask's Markdown body, if it has one.
fn risk_text(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    let stored: Option<String> = tx.query_row(
        "SELECT merge_risk FROM tasks WHERE id = ?1",
        [task],
        |row| row.get(0),
    )?;
    Ok(stored
        .and_then(|stored| serde_json::from_str::<Value>(&stored).ok())
        .map(|risk| {
            let lines = RISK_DIMENSIONS.map(|dimension| {
                format!(
                    "- {dimension}: {}",
                    risk[dimension].as_str().unwrap_or_default()
                )
            });
            format!(
                "Merge risk: {}\n\n{}",
                risk["verdict"].as_str().unwrap_or_default(),
                lines.join("\n")
            )
        }))
}

/// A typed compact needs no check, since an agent mid-compaction calls no tool.
fn compact_queued(tx: &Transaction, task: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM outbox WHERE task = ?1 AND compact = 1 AND state = 'queued')",
        [task],
        |row| row.get(0),
    )
}

/// Queues `/compact` for the outbox to type once the run is idle. Codex's `/compact` takes no focus.
fn compact(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let focus = match optional(arguments, "focus") {
        Ok(focus) => focus,
        Err(error) => return Ok(Err(error)),
    };
    if compact_queued(tx, run.task)? {
        return Ok(Err(
            "a compact is already queued for this run; end your turn".to_owned(),
        ));
    }
    let codex = run
        .agent
        .as_ref()
        .and_then(|agent| factory.agents.get(agent))
        .is_some_and(|agent| matches!(agent.value.harness, Harness::Codex));
    let text = match focus {
        Some(focus) if !codex => {
            format!(
                "/compact {}",
                focus.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        }
        _ => "/compact".to_owned(),
    };
    tx.execute(
        "INSERT INTO outbox (task, text, compact) VALUES (?1, ?2, 1)",
        params![run.task, text.trim_end()],
    )?;
    Ok(Ok(format!(
        "Queued {text}. End your turn now: Priory types it once you are idle, then a prompt to carry on."
    )))
}

/// Queues `/compact` once per crossing when the session's `context` reaches its agent's
/// `compactAt`. Every run of the session shares the crossing. A run with a result is skipped, since
/// the result may move the task to another conversation, and so is a task whose answer's command
/// runs. Says whether the compact it queued for the crossing is still queued, so the run must not
/// sleep yet. A reading below `compactAt` withdraws that compact if it was never typed.
pub(crate) fn overflow(
    tx: &Transaction,
    session: &str,
    context: Option<u64>,
    factory: &Factory,
) -> rusqlite::Result<bool> {
    let Some(run) = run(tx, session)? else {
        return Ok(false);
    };
    let Some(limit) = run
        .agent
        .as_ref()
        .and_then(|agent| factory.agents.get(agent))
        .and_then(|agent| agent.value.compact_at)
    else {
        return Ok(false);
    };
    if context.is_some_and(|context| context < limit.get()) {
        tx.execute(
            "DELETE FROM outbox WHERE state = 'queued'
             AND id IN (SELECT crossed FROM runs WHERE session = ?1)",
            [session],
        )?;
        tx.execute("UPDATE runs SET crossed = 0 WHERE session = ?1", [session])?;
        return Ok(false);
    }
    if commanding(tx, run.task)? {
        return Ok(false);
    }
    let (crossed, pending): (bool, bool) = tx.query_row(
        "SELECT max(crossed) > 0, EXISTS (SELECT 1 FROM outbox WHERE state = 'queued'
           AND id = (SELECT max(crossed) FROM runs WHERE session = ?1))
         FROM runs WHERE session = ?1",
        [session],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if crossed {
        return Ok(pending);
    }
    let result: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM results WHERE run = ?1)",
        [run.id],
        |row| row.get(0),
    )?;
    if context.is_none() || result || compact_queued(tx, run.task)? {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO outbox (task, text, compact) VALUES (?1, '/compact', 1)",
        [run.task],
    )?;
    tx.execute(
        "UPDATE runs SET crossed = ?2 WHERE session = ?1",
        params![session, tx.last_insert_rowid()],
    )?;
    Ok(true)
}

/// Ends a `/compact` Priory typed into `run`; a compaction it did not type changes nothing.
pub(crate) fn compacted(tx: &Transaction, run: i64) -> rusqlite::Result<()> {
    let Some((id, task, step)) = tx
        .query_row(
            "SELECT outbox.id, runs.task, tasks.step FROM outbox
             JOIN runs ON runs.id = outbox.run JOIN tasks ON tasks.id = runs.task
             WHERE outbox.run = ?1 AND outbox.compact = 1 AND outbox.state IN ('attempted', 'uncertain')",
            [run],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()?
    else {
        return Ok(());
    };
    tx.execute(
        &format!("UPDATE outbox SET state = 'confirmed', confirmed = {NOW} WHERE id = ?1"),
        [id],
    )?;
    tx.execute(
        "UPDATE runs SET idle = 1, turn = NULL, nudgeable = 0 WHERE id = ?1",
        [run],
    )?;
    wake(
        tx,
        task,
        &format!(
            "Priory compacted this conversation, as you asked. You are on task {task}, at step {step}. Carry on with the step."
        ),
    )
    .map(drop)
}

fn ask(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let worktree: Option<String> = tx.query_row(
        "SELECT worktree FROM tasks WHERE id = ?1",
        [run.task],
        |row| row.get(0),
    )?;
    let fields = (|| {
        let options = arguments["options"]
            .as_array()
            .and_then(|options| {
                options
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or("options must be an array of strings")?;
        let recommendation = text(arguments, "recommendation")?;
        let command = match &arguments["command"] {
            Value::Null => None,
            command => {
                let worktree = worktree.as_deref();
                let parsed = command::parse(command, &options, &recommendation, worktree, factory)?;
                Some(serde_json::to_string(&parsed).expect("a command serializes"))
            }
        };
        Ok::<_, String>((
            text(arguments, "question")?,
            serde_json::to_string(&options).expect("options serialize"),
            recommendation,
            text(arguments, "tradeoff")?,
            described(arguments)?,
            command,
        ))
    })();
    let (question, options, recommendation, tradeoff, described, command) = match fields {
        Ok(fields) => fields,
        Err(error) => return Ok(Err(error)),
    };
    let [title, body, what_waits, next_action] = described;
    let lead = lead_of(
        tx,
        run.task,
        &run.step,
        command.is_some(),
        &factory.workflows,
    )?;
    tx.execute(
        "INSERT INTO asks (task, run, step, kind, question, options, recommendation, tradeoff,
           title, text, what_waits, next_action, command, routed)
         VALUES (?1, ?2, ?3, 'agent', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            run.task,
            run.id,
            run.step,
            question,
            options,
            recommendation,
            tradeoff,
            title,
            body,
            what_waits,
            next_action,
            command,
            lead
        ],
    )?;
    let id = tx.last_insert_rowid();
    if let Some(lead) = &lead {
        call_lead(tx, id, lead)?;
    }
    Ok(Ok(format!(
        "ask {id} stored: end your turn now; the answer arrives as your next prompt"
    )))
}

/// The fields `priory ask` stores, as Fleet's `priory ask` names them.
const OUTSIDE_FIELDS: [&str; 13] = [
    "question",
    "options",
    "recommendation",
    "tradeoff",
    "title",
    "text",
    "whatWaits",
    "nextAction",
    "command",
    "coordinator",
    "key",
    "runId",
    "session",
];

/// Stores an ask from a session Priory did not start, under `name`, and returns the fields it
/// ignored. Asking again with other fields replaces them all and starts a new revision, which
/// reopens an answered or dismissed ask; the same fields change nothing.
pub fn ask_outside(
    db: &mut Connection,
    name: &str,
    arguments: &Value,
) -> rusqlite::Result<Result<Vec<String>, String>> {
    let named = (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && !name.bytes().all(|byte| byte.is_ascii_digit())
        && !name.bytes().all(|byte| byte == b'.');
    if !named {
        return Ok(Err(format!(
            "not an ask id: {name:?}; use 1 to 100 letters, digits, '.', '_' or '-', not only digits or dots"
        )));
    }
    let Some(fields) = arguments.as_object() else {
        return Ok(Err("ask fields must be a JSON object".to_owned()));
    };
    let parsed = (|| {
        let options = match &arguments["options"] {
            Value::Null => Vec::new(),
            options => options
                .as_array()
                .and_then(|options| {
                    options
                        .iter()
                        .map(Value::as_str)
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or("options must be an array of strings")?,
        };
        let mut texts = Vec::new();
        for field in OUTSIDE_FIELDS.iter().skip(2) {
            texts.push(optional(arguments, field)?);
        }
        // A Fleet-shaped ask names its question `title`.
        let question = optional(arguments, "question")?
            .or(optional(arguments, "title")?)
            .ok_or("question or title must be a string")?;
        Ok::<_, String>((
            question,
            serde_json::to_string(&options).expect("options serialize"),
            texts,
        ))
    })();
    let (question, options, texts) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => return Ok(Err(error)),
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO asks (kind, name, question, options, recommendation, tradeoff, title, text,
           what_waits, next_action, command, coordinator, key, run_id, session)
         VALUES ('outside', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT (name) DO UPDATE SET revision = revision + 1, answer = NULL, answered = NULL, answered_by = NULL,
           withdrawn = NULL, withdrawn_reason = NULL, question = excluded.question, options = excluded.options,
           recommendation = excluded.recommendation, tradeoff = excluded.tradeoff,
           title = excluded.title, text = excluded.text, what_waits = excluded.what_waits,
           next_action = excluded.next_action, command = excluded.command,
           coordinator = excluded.coordinator, key = excluded.key, run_id = excluded.run_id,
           session = excluded.session
         WHERE (question, options, recommendation, tradeoff, title, text, what_waits, next_action,
           command, coordinator, key, run_id, session)
           IS NOT (excluded.question, excluded.options, excluded.recommendation,
           excluded.tradeoff, excluded.title, excluded.text, excluded.what_waits,
           excluded.next_action, excluded.command, excluded.coordinator, excluded.key,
           excluded.run_id, excluded.session)",
        rusqlite::params_from_iter(
            [Some(name.to_owned()), Some(question), Some(options)]
                .into_iter()
                .chain(texts),
        ),
    )?;
    tx.commit()?;
    Ok(Ok(fields
        .keys()
        .filter(|field| !OUTSIDE_FIELDS.contains(&field.as_str()))
        .cloned()
        .collect()))
}

/// The number of the outside ask stored under `name`.
pub fn ask_named(db: &Connection, name: &str) -> Result<i64, String> {
    db.query_row("SELECT id FROM asks WHERE name = ?1", [name], |row| {
        row.get(0)
    })
    .optional()
    .map_err(|error| error.to_string())?
    .ok_or_else(|| format!("there is no ask {name}"))
}

/// Whether an answer, dismissal or fired wait of `run` waits in a queued prompt: a new ask or wait
/// would put the task to sleep over it.
fn woken(tx: &Transaction, run: &Run) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT (EXISTS (SELECT 1 FROM waits WHERE run = ?1 AND fired IS NOT NULL)
           OR EXISTS (SELECT 1 FROM asks WHERE run = ?1
             AND (answer IS NOT NULL OR withdrawn_reason = 'dismissed')))
         AND EXISTS (SELECT 1 FROM outbox WHERE task = ?2 AND state = 'queued')
         OR EXISTS (SELECT 1 FROM actions WHERE task = ?2 AND action = 'answer' AND ended IS NULL)",
        params![run.id, run.task],
        |row| row.get(0),
    )
}

/// Whether a person's feedback the task kept while it ran waits in a queued prompt.
fn kept(tx: &Transaction, task: i64) -> rusqlite::Result<bool> {
    let mut statement = tx.prepare(
        "SELECT inbox.type, inbox.data FROM inbox JOIN outbox ON outbox.id = inbox.outbox
         WHERE inbox.task = ?1 AND inbox.outcome = 'kept' AND outbox.state = 'queued'",
    )?;
    let mut rows = statement.query([task])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let data: Value = serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or_default();
        if feedback_on(&kind, &data).is_some() && !settles(&kind, &data) && data["own"] != true {
            return Ok(true);
        }
    }
    Ok(false)
}

fn commanding(tx: &Transaction, task: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM actions WHERE task = ?1 AND action = 'answer' AND ended IS NULL)",
        [task],
        |row| row.get(0),
    )
}

fn open_wait(tx: &Transaction, run: i64) -> rusqlite::Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM waits WHERE run = ?1 AND fired IS NULL AND withdrawn IS NULL",
        [run],
        |row| row.get(0),
    )
    .optional()
}

const CONDITIONS: &str =
    "pull_request, checks, review, time, task, linear_comment, linear_state, slack_reply or disk";

/// What a resource wait names: its resource, and the head or the state it wants.
type Named = (String, Option<String>, Option<String>);

/// The resource `on` names for a resource wait of `kind`, with its head or state.
fn named(kind: &str, on: &Value) -> Result<Named, String> {
    let text = |field: &str| on[field].as_str().filter(|value| !value.is_empty());
    match kind {
        "pull_request" | "checks" | "review" => {
            let (Some(repo), Some(number)) = (text("repo"), on["pull_request"].as_u64()) else {
                return Err(format!(
                    "a {kind} wait needs repo, as owner/name, and pull_request, a number"
                ));
            };
            let head = match &on["head_sha"] {
                Value::Null => None,
                Value::String(head) if kind == "checks" => Some(full_sha(head)?),
                _ => return Err("head_sha must be a commit, on a checks wait only".into()),
            };
            Ok((route::pull_request(repo, number), head, None))
        }
        "linear_comment" | "linear_state" => {
            let state = text("state").filter(|_| kind == "linear_state");
            match (text("issue"), state) {
                (Some(issue), None) if kind == "linear_comment" => {
                    Ok((format!("linear:{issue}"), None, None))
                }
                (Some(issue), Some(state)) => {
                    Ok((format!("linear:{issue}"), None, Some(state.to_owned())))
                }
                _ if kind == "linear_comment" => {
                    Err("a linear_comment wait needs issue, such as ENG-7".into())
                }
                _ => Err(
                    "a linear_state wait needs issue, such as ENG-7, and state, such as Done"
                        .into(),
                ),
            }
        }
        _ => match (text("channel"), text("thread_ts")) {
            (Some(channel), Some(thread)) => Ok((format!("slack:{channel}:{thread}"), None, None)),
            _ => Err(
                "a slack_reply wait needs channel, such as C0123, and thread_ts, the ts of the thread's first message"
                    .into(),
            ),
        },
    }
}

/// The resource a `subscribe` or `unsubscribe` condition names: a pull request, a Linear issue or
/// a Slack thread.
pub(crate) fn resource(on: &Value) -> Result<String, String> {
    match on["type"].as_str().unwrap_or_default() {
        kind @ ("pull_request" | "checks" | "review" | "linear_comment" | "slack_reply") => {
            named(kind, on).map(|(resource, ..)| resource)
        }
        "linear_state" => named("linear_comment", on).map(|(resource, ..)| resource),
        kind => Err(format!(
            "cannot subscribe to {kind:?}: subscribe to pull_request, checks, review, linear_comment, linear_state or slack_reply"
        )),
    }
}

/// Stores a wait on each condition of `on` or `any`, and fires the first that already holds: an
/// event since the agent's last prompt, a passed time or an ended task. The daemon reads the
/// current states it can afterwards, outside this call (see `readable`). Also returns the tmux
/// session of a run that a merge the catch-up judged ended, to kill once the agent has its reply.
fn wait(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
    factory: &Factory,
) -> rusqlite::Result<(Result<String, String>, Option<String>)> {
    let conditions = match conditions(arguments) {
        Ok(conditions) => conditions,
        Err(error) => return Ok((Err(error), None)),
    };
    let mut ids = Vec::new();
    for on in conditions {
        match condition(tx, run, on)? {
            Ok(id) => ids.push(id.to_string()),
            Err(error) => return Ok((Err(error), None)),
        }
    }
    let ids = ids.join(", ");
    let (caught, kill) = catch_up(tx, run.task, factory)?;
    if !caught {
        due(tx)?;
    }
    let fired = tx
        .query_row(
            &format!("SELECT id FROM waits WHERE id IN ({ids}) AND fired IS NOT NULL"),
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let reply = match fired {
        Some(id) if caught => format!(
            "wait {id} already holds: end your turn; the event that satisfies it is your next prompt"
        ),
        Some(id) => format!("wait {id} already holds: end your turn; your next prompt says so"),
        None if !ids.contains(',') => format!(
            "wait {ids} stored: end your turn now; Priory wakes you with the event that satisfies it"
        ),
        None => format!(
            "waits {ids} stored: end your turn now; Priory wakes you with the first event that satisfies one of them"
        ),
    };
    Ok((Ok(reply), kill))
}

/// The conditions of a wait call: `on`, one condition, or `any`, a list of them.
fn conditions(arguments: &Value) -> Result<&[Value], String> {
    match (&arguments["on"], &arguments["any"]) {
        (on, Value::Null) => Ok(std::slice::from_ref(on)),
        (Value::Null, Value::Array(any)) if !any.is_empty() => Ok(any),
        _ => Err(
            "wait takes either on, one condition, or any, a non-empty list of conditions".into(),
        ),
    }
}

/// The open waits whose current state the daemon can read: a Linear state, as `linear:<issue>`,
/// and a pull request, as `github:<owner/name>#<number>`. Each comes with its id and kind.
pub(crate) fn readable(db: &Connection) -> rusqlite::Result<Vec<(i64, String, String)>> {
    db.prepare(
        "SELECT waits.id, waits.kind, waits.resource FROM waits JOIN tasks ON tasks.id = waits.task
         WHERE waits.fired IS NULL AND waits.withdrawn IS NULL AND NOT waits.daemon
         AND waits.kind IN ('linear_state', 'pull_request')
         AND tasks.state NOT IN ('done', 'failed', 'dropped') ORDER BY waits.id",
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
    .collect()
}

/// Fires the wait `wait` if `current`, the state the daemon read for its condition, already
/// satisfies it: a Linear issue in the wanted state, or a pull request merged or closed. A wait
/// that fired or was withdrawn meanwhile stays as it is.
pub(crate) fn read(db: &mut Connection, wait: i64, current: &str) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found = tx
        .query_row(
            "SELECT waits.task, waits.kind, waits.resource, waits.state FROM waits
             JOIN tasks ON tasks.id = waits.task
             WHERE waits.id = ?1 AND tasks.state NOT IN ('done', 'failed', 'dropped')",
            [wait],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((task, kind, resource, state)) = found else {
        return Ok(());
    };
    let line = match kind.as_str() {
        "linear_state" if state.as_deref() == Some(current) => {
            format!(
                "{} is already {current}",
                resource.trim_start_matches("linear:")
            )
        }
        "pull_request" if current != "OPEN" => {
            format!("{resource} is already {}", current.to_lowercase())
        }
        _ => return Ok(()),
    };
    fire(&tx, wait, task, &format!("wait {wait} fired: {line}"), None)?;
    tx.commit()
}

/// Stores a wait on the one condition `on` and returns its id.
fn condition(tx: &Transaction, run: &Run, on: &Value) -> rusqlite::Result<Result<i64, String>> {
    let kind = on["type"].as_str().unwrap_or_default();
    match kind {
        "pull_request" | "checks" | "review" | "linear_comment" | "linear_state"
        | "slack_reply" => {
            let (resource, head, state) = match named(kind, on) {
                Ok(named) => named,
                Err(error) => return Ok(Err(error)),
            };
            if let Some(refused) = crate::state::taken(tx, run.task, &resource)? {
                return Ok(Err(refused));
            }
            tx.execute(
                "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
                params![run.task, resource],
            )?;
            // A Linear or Slack event reaches the task only once it waits, so the agent's last
            // prompt bounds it: an event at or after it counts, one before it does not.
            tx.execute(
                "INSERT INTO waits (task, run, kind, resource, head_sha, state, since)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, iif(?3 IN ('linear_comment', 'linear_state', 'slack_reply'),
                   coalesce((SELECT max(confirmed) FROM outbox WHERE task = ?1),
                     (SELECT started FROM runs WHERE id = ?2)), NULL))",
                params![run.task, run.id, kind, resource, head, state],
            )?;
            let id = tx.last_insert_rowid();
            // No task owned the resource before: its events since the last prompt were ignored.
            tx.execute(
                "UPDATE inbox SET task = ?1, outcome = 'kept'
                 WHERE outcome = 'ignored' AND task IS NULL
                 AND julianday(time) >= julianday((SELECT since FROM waits WHERE id = ?2))
                 AND ?3 = CASE
                   WHEN type LIKE 'linear.%' THEN 'linear:' || json_extract(data, '$.issue')
                   WHEN type LIKE 'slack.%' THEN 'slack:' || json_extract(data, '$.channel') || ':'
                     || coalesce(json_extract(data, '$.thread_ts'), json_extract(data, '$.ts'))
                 END",
                params![run.task, id, resource],
            )?;
            return Ok(Ok(id));
        }
        "time" => {
            let until = on["until"].as_str().unwrap_or_default();
            let at = match instant(tx, until)? {
                Ok(at) => at,
                Err(error) => return Ok(Err(error)),
            };
            tx.execute(
                "INSERT INTO waits (task, run, kind, until) VALUES (?1, ?2, 'time', ?3)",
                params![run.task, run.id, at],
            )?;
        }
        "task" => {
            let target = on["task"].as_i64().unwrap_or_default();
            let known = tx
                .query_row("SELECT 1 FROM tasks WHERE id = ?1", [target], |_| Ok(()))
                .optional()?;
            if known.is_none() || target == run.task {
                return Ok(Err(format!(
                    "task must name another task's id, not {}",
                    on["task"]
                )));
            }
            tx.execute(
                "INSERT INTO waits (task, run, kind, target) VALUES (?1, ?2, 'task', ?3)",
                params![run.task, run.id, target],
            )?;
        }
        "disk" => {
            let Some(gb) = on["gb"].as_f64().filter(|gb| gb.is_finite() && *gb > 0.0) else {
                return Ok(Err(
                    "a disk wait needs gb, the free disk in GB to wait for".into()
                ));
            };
            tx.execute(
                "INSERT INTO waits (task, run, kind, bytes) VALUES (?1, ?2, 'disk', ?3)",
                params![run.task, run.id, (gb * GB) as i64],
            )?;
        }
        other => {
            return Ok(Err(format!(
                "unknown condition {other:?}: wait on {CONDITIONS}"
            )));
        }
    }
    Ok(Ok(tx.last_insert_rowid()))
}

/// The instant `until` names: an RFC 3339 instant, or a duration from now such as `2 hours`.
pub(crate) fn instant(tx: &Transaction, until: &str) -> rusqlite::Result<Result<String, String>> {
    let at: Option<String> = match until.parse::<herdr_bus::Timestamp>() {
        Ok(instant) => tx.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1)",
            [instant.to_string()],
            |row| row.get(0),
        )?,
        // The `+` refuses the SQLite modifiers that are not durations, such as `start of day`.
        Err(_) => tx.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?1)",
            [until],
            |row| row.get(0),
        )?,
    };
    Ok(at.ok_or_else(|| {
        format!(
            "until {until:?} is neither an RFC 3339 instant nor a duration such as 30 seconds, 5 minutes or 2 hours"
        )
    }))
}

/// Fires the task's open wait on the first event since the agent's last prompt that satisfies it,
/// on which none of its waits fired: one it kept and was not typed yet, or one another task kept
/// since that prompt. Says whether one did, with the tmux session of a run its firing ended.
fn catch_up(tx: &Transaction, task: i64, factory: &Factory) -> rusqlite::Result<Fired> {
    let mut candidates = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT inbox.seq FROM inbox LEFT JOIN outbox ON outbox.id = inbox.outbox
             WHERE inbox.outcome = 'kept'
             AND (inbox.task = ?1 AND (inbox.outbox IS NULL OR outbox.state = 'queued')
               OR inbox.task IS NOT ?1 AND julianday(inbox.time) >= julianday(coalesce(
                 (SELECT max(confirmed) FROM outbox WHERE task = ?1),
                 (SELECT max(started) FROM runs WHERE task = ?1))))
             AND inbox.seq NOT IN (SELECT inbox FROM waits WHERE task = ?1 AND inbox IS NOT NULL)
             ORDER BY inbox.seq",
        )?;
        let mut rows = statement.query([task])?;
        while let Some(row) = rows.next()? {
            candidates.push(row.get::<_, i64>(0)?);
        }
    }
    for seq in candidates {
        let fired = event(tx, task, seq, factory)?;
        if fired.0 {
            return Ok(fired);
        }
    }
    Ok((false, None))
}

/// Whether a wait fired, and the tmux session of a run its firing ended, to kill once committed.
pub(crate) type Fired = (bool, Option<String>);

/// Fires the first of the task's open waits that the inbox entry `seq` satisfies, and says whether
/// one did.
pub(crate) fn event(
    tx: &Transaction,
    task: i64,
    seq: i64,
    factory: &Factory,
) -> rusqlite::Result<Fired> {
    let open = tx
        .prepare(
            "SELECT waits.id, waits.kind, waits.resource, waits.head_sha, waits.state,
               waits.since, waits.daemon, waits.created FROM waits
             JOIN tasks ON tasks.id = waits.task
             WHERE waits.task = ?1 AND waits.fired IS NULL AND waits.withdrawn IS NULL
             AND tasks.state NOT IN ('done', 'failed', 'dropped') ORDER BY waits.id",
        )?
        .query_map([task], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (wait, kind, resource, head, state, since, daemon, created) in open {
        let Some(resource) = resource else {
            continue;
        };
        let (kind_of_event, subject, data, after, kept, later): (
            String,
            Option<String>,
            String,
            bool,
            bool,
            bool,
        ) = tx.query_row(
            "SELECT type, subject, data, coalesce(julianday(time) >= julianday(?2), 1), task IS ?3,
               julianday(time) >= julianday(?4)
                 FROM inbox WHERE seq = ?1",
            params![seq, since, task, created],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        let mut line = crate::state::line(&kind_of_event, subject.as_deref(), &data);
        let data: Value = serde_json::from_str(&data).expect("inbox data is JSON");
        let on = data["repo"]
            .as_str()
            .zip(data["pull_request"].as_u64())
            .map(|(repo, number)| route::pull_request(repo, number));
        // Each pull request of a stack has its own gate, which only its events decide.
        if daemon && on.as_deref() != Some(resource.as_str()) {
            continue;
        }
        // A comment written before the wait, such as the agent's own, wakes nothing.
        let place = crate::stack::place(tx, task, &resource)?;
        let said = feedback(&resource, place.as_deref(), &kind_of_event, &data)
            .filter(|_| later || kind_of_event == REVIEWED || data["own"] == false);
        if daemon {
            let Some(said) = said.filter(|_| !settles(&kind_of_event, &data)) else {
                return judge(tx, wait, factory);
            };
            fire(
                tx,
                wait,
                task,
                &format!("wait {wait} fired: {said}"),
                Some(seq),
            )?;
            return Ok((true, None));
        }
        let wanted = head.as_deref().or(state.as_deref());
        let woken = match &said {
            Some(_) => ["pull_request", "review"].contains(&kind.as_str()),
            // A comment or reply written before the wait, such as the agent's own question, wakes nothing.
            None => {
                !FEEDBACK.contains(&kind_of_event.as_str())
                    && (later || !["linear_comment", "slack_reply"].contains(&kind.as_str()))
                    && route::satisfies(&kind, &resource, wanted, &kind_of_event, &data)
            }
        };
        if !after || !woken {
            continue;
        }
        if let Some(said) = said {
            line = format!("wait {wait} fired: {said}");
        }
        if !kept {
            // Another task kept the event and its prompt: this one gets the line in a prompt of its own.
            mark_fired(tx, wait, Some(seq))?;
            wake_on(tx, task, &line, None)?;
            return Ok((true, None));
        }
        fire(tx, wait, task, &line, Some(seq))?;
        return Ok((true, None));
    }
    Ok((false, None))
}

/// The events of a person's feedback on a pull request.
const FEEDBACK: [&str; 3] = [REVIEWED, COMMENTED, THREAD_COMMENTED];

/// The resource that feedback of `kind` is on: its pull request for pull request feedback, its
/// issue, Linear issue or Slack thread for a comment there; none for any other event.
fn feedback_on(kind: &str, data: &Value) -> Option<String> {
    let text = |field: &str| data[field].as_str();
    match kind {
        _ if FEEDBACK.contains(&kind) => Some(route::pull_request(
            text("repo")?,
            data["pull_request"].as_u64()?,
        )),
        ISSUE_COMMENTED => Some(route::pull_request(text("repo")?, data["issue"].as_u64()?)),
        "linear.comment.created" => Some(format!("linear:{}", text("issue")?)),
        "slack.thread.replied" => {
            Some(format!("slack:{}:{}", text("channel")?, text("thread_ts")?))
        }
        _ => None,
    }
}

/// Fires the checks gate of `task` on a comment or reply in `message` on its issue, Linear issue
/// or Slack thread, as pull request feedback fires it, since a merge would end the task unheard.
pub(crate) fn gate_feedback(
    tx: &Transaction,
    task: i64,
    message: &herdr_bus::Message,
    seq: Option<i64>,
) -> rusqlite::Result<bool> {
    let Some(resource) = feedback_on(&message.kind, &message.data)
        .filter(|_| !FEEDBACK.contains(&message.kind.as_str()))
    else {
        return Ok(false);
    };
    let gate: Option<i64> = tx.query_row(
        "SELECT min(waits.id) FROM waits JOIN tasks ON tasks.id = waits.task
         WHERE waits.task = ?1 AND waits.daemon AND waits.fired IS NULL AND waits.withdrawn IS NULL
         AND tasks.state = 'asleep'",
        [task],
        |row| row.get(0),
    )?;
    let Some(gate) = gate else {
        return Ok(false);
    };
    let said = reopened_by(&resource, None, &message.kind, &message.data);
    let mut text = format!("wait {gate} fired: {said}");
    // Another task kept the event, so its whole line goes with the reason.
    if seq.is_none() {
        let data = message.data.to_string();
        let line = crate::state::line(&message.kind, message.subject.as_deref(), &data);
        text = format!("{text}\n{line}");
    }
    fire(tx, gate, task, &text, seq)?;
    Ok(true)
}

/// Who said what on `resource`, for the prompt of the done task that feedback of `kind` reopens.
fn reopened_by(resource: &str, place: Option<&str>, kind: &str, data: &Value) -> String {
    if let Some(said) = feedback(resource, place, kind, data) {
        return said;
    }
    let who = data["actor"].as_str().or(data["user"].as_str());
    let who = who.unwrap_or("a deleted user");
    let body = data["body"].as_str().or(data["text"].as_str());
    let first = body.unwrap_or_default().lines().next().unwrap_or_default();
    let url = data["url"].as_str().unwrap_or_default();
    let verb = match kind {
        "slack.thread.replied" => "replied in",
        _ => "commented on",
    };
    format!("{who} {verb} {resource}: {first} {url}")
        .trim_end()
        .to_owned()
}

/// Whether `data` is a review that asks the agent for nothing: an approval or a dismissal.
fn settles(kind: &str, data: &Value) -> bool {
    kind == REVIEWED && matches!(data["state"].as_str(), Some("approved" | "dismissed"))
}

/// Who said what and where, when `data` is feedback on the pull request `resource`, at `place`
/// in its task's stack.
fn feedback(resource: &str, place: Option<&str>, kind: &str, data: &Value) -> Option<String> {
    let on = route::pull_request(data["repo"].as_str()?, data["pull_request"].as_u64()?);
    if on != resource || !FEEDBACK.contains(&kind) {
        return None;
    }
    let resource = match place {
        Some(place) => format!("{resource} ({place})"),
        None => resource.to_owned(),
    };
    let who = data["actor"].as_str().unwrap_or("a deleted user");
    let url = data["url"].as_str().unwrap_or_default();
    let body = data["body"].as_str().unwrap_or_default();
    let first = body.lines().next().unwrap_or_default();
    let said = match kind {
        REVIEWED => {
            let state = data["state"].as_str().unwrap_or("submitted");
            match first {
                "" => format!("{who} reviewed {resource}: {state}"),
                _ => format!("{who} reviewed {resource}: {state}: {first}"),
            }
        }
        COMMENTED => format!("{who} commented on {resource}: {first}"),
        _ => {
            let path = data["path"].as_str().unwrap_or_default();
            let at = data["line"]
                .as_u64()
                .map_or(String::new(), |line| format!(":{line}"));
            format!("{who} commented on {path}{at} of {resource}: {first}")
        }
    };
    Some(format!("{said} {url}").trim_end().to_owned())
}

/// An event the task kept on the pull request of a daemon's checks wait.
struct Seen {
    seq: i64,
    kind: String,
    head: String,
    data: Value,
}

impl Seen {
    fn red(&self) -> bool {
        self.kind == CHECKED
            && !PASSING.contains(&self.data["conclusion"].as_str().unwrap_or("none"))
    }
}

/// Judges the daemon's checks wait `wait` on every event the inbox kept, ignored or woke on about its pull
/// request, so that events the agent already read, or an earlier owner kept, count too. The newest event tells the head: one after the
/// result that names another head wakes the agent. The newest check at the result's commit then
/// decides: red wakes the agent, and green with none pending or failing marks the task done. With
/// no check at the commit, a newest snapshot of no check runs marks it done too.
fn judge(tx: &Transaction, wait: i64, factory: &Factory) -> rusqlite::Result<Fired> {
    type Gate = (i64, String, Option<String>, Option<i64>, bool);
    let (task, resource, pinned, mark, after): Gate = tx.query_row(
        "SELECT waits.task, waits.resource, waits.head_sha, coalesce(
           (SELECT max(inbox) FROM actions WHERE result = waits.run AND commit_sha = waits.head_sha),
           results.inbox), waits.after IS NOT NULL
         FROM waits JOIN results ON results.run = waits.run WHERE waits.id = ?1",
        [wait],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    let mut seen = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT seq, type, data FROM inbox
             WHERE outcome IN ('kept', 'ignored', 'woke')
             AND 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1
             AND (json_extract(data, '$.head_sha') IS NOT NULL OR type = ?2) ORDER BY seq DESC",
        )?;
        let mut rows = statement.query(params![resource, CLOSED])?;
        while let Some(row) = rows.next()? {
            let data: Value =
                serde_json::from_str(&row.get::<_, String>(2)?).expect("inbox data is JSON");
            seen.push(Seen {
                seq: row.get(0)?,
                kind: row.get(1)?,
                head: data["head_sha"].as_str().unwrap_or_default().to_owned(),
                data,
            });
        }
    }
    let Some(newest) = seen.first() else {
        return Ok((false, None));
    };
    let stack = crate::stack::of(tx, task)?;
    let stacked = stack.len() > 1;
    if newest.kind == CLOSED {
        if newest.data["merged"] == true && stacked {
            return merged(tx, wait, task, &resource, newest, &stack, factory);
        }
        if newest.data["merged"] == true {
            return done(tx, wait, task, newest.seq, factory);
        }
        let at = stack.iter().position(|pull| pull.resource == resource);
        if let Some(asked) = at.filter(|_| stacked).map(|at| stranded(task, &stack, at))
            && !answered(tx, &asked.name)?
        {
            return Ok((ask_stack(tx, task, asked)?, None));
        }
        let reason = match newest.data["merged"] {
            Value::Bool(false) => format!("wait {wait} fired: {resource} closed without a merge"),
            _ => format!(
                "wait {wait} fired: {resource} closed, and GitHub no longer says whether it merged"
            ),
        };
        fire(tx, wait, task, &reason, Some(newest.seq))?;
        return Ok((true, None));
    }
    if let Some(asked) = retargeted(task, &stack, &resource, newest)
        && !answered(tx, &asked.name)?
    {
        return Ok((ask_stack(tx, task, asked)?, None));
    }
    // A lower pull request of a stack is judged at its newest head, since a restack moves it.
    let commit = pinned.unwrap_or_else(|| newest.head.clone());
    if newest.head != commit {
        // A head seen before the result may be one the agent's commit replaced since.
        if newest.seq <= mark.unwrap_or_default() {
            return Ok((false, None));
        }
        let reason = format!(
            "wait {wait} fired: the head of {resource} moved to {}, away from your result's commit {commit}",
            newest.head
        );
        fire(tx, wait, task, &reason, Some(newest.seq))?;
        return Ok((true, None));
    }
    let mut checks = seen
        .iter()
        .filter(|event| event.kind == CHECKED && event.head == commit);
    // A snapshot decides only before any check completed at the commit.
    let snapshot = || {
        seen.iter()
            .find(|event| event.kind == CHECK_RUNS && event.head == commit)
    };
    let Some(check) = checks.next().or_else(snapshot) else {
        // A stack's gate at a new head with no check yet is not green any more.
        if stacked {
            return Ok((stack_green(tx, wait, task, false, &stack)?, None));
        }
        return Ok((false, None));
    };
    let failing = check.data["failing"].as_u64();
    let red = match failing {
        _ if check.red() => Some(check),
        // The red check counted here came earlier, or its event follows from the same poll.
        Some(1..) => checks.find(|event| event.red()),
        _ => None,
    };
    if let Some(red) = red {
        let url = red.data["url"]
            .as_str()
            .filter(|url| !url.is_empty())
            .map(|url| format!(" {url}"))
            .unwrap_or_default();
        let reason = format!(
            "wait {wait} fired: check {} at {commit} concluded {}{url}",
            red.data["name"].as_str().unwrap_or("-"),
            red.data["conclusion"].as_str().unwrap_or("none"),
        );
        fire(tx, wait, task, &reason, Some(red.seq))?;
        return Ok((true, None));
    }
    let green = match check.kind.as_str() {
        CHECK_RUNS => check.data["runs"].as_u64() == Some(0),
        _ => check.data["pending"].as_u64() == Some(0) && failing == Some(0),
    };
    // A stack is done once every pull request merged, so green checks leave it waiting.
    if stacked {
        return Ok((stack_green(tx, wait, task, green, &stack)?, None));
    }
    // A gate before a step that waits for the merge stays open on green checks.
    if !green || after {
        return Ok((false, None));
    }
    done(tx, wait, task, check.seq, factory)
}

/// Records whether gate `wait` of `task`'s stack is green. When that makes every open gate green,
/// posts one report naming the pull request to merge first, since only a person merges a stack.
fn stack_green(
    tx: &Transaction,
    wait: i64,
    task: i64,
    green: bool,
    stack: &[crate::stack::Pull],
) -> rusqlite::Result<bool> {
    let all_green = || -> rusqlite::Result<bool> {
        tx.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM waits WHERE task = ?1 AND daemon
               AND fired IS NULL AND withdrawn IS NULL AND green IS NOT 1)",
            [task],
            |row| row.get(0),
        )
    };
    let before = all_green()?;
    tx.execute(
        "UPDATE waits SET green = ?2 WHERE id = ?1",
        params![wait, green],
    )?;
    let Some(bottom) = stack.iter().find(|pull| pull.state == "open") else {
        return Ok(false);
    };
    if before || !all_green()? {
        return Ok(false);
    }
    let key: String = tx.query_row("SELECT key FROM tasks WHERE id = ?1", [task], |row| {
        row.get(0)
    })?;
    let line = format!(
        "the stack of task {task} ({key}) is green: merge #{} first",
        bottom.number()
    );
    crate::post::report(tx, "priory", &line)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?;
    Ok(false)
}

/// A daemon's ask about a stack: its stable name, so a repeated event asks once, and its question.
struct StackAsk {
    name: String,
    question: String,
    onto: String,
}

/// The ask about the pull request at `at` of `stack`, which closed without a merge.
fn stranded(task: i64, stack: &[crate::stack::Pull], at: usize) -> StackAsk {
    let pull = &stack[at];
    let above = open_above(stack, at);
    let based = match above.len() {
        0 => String::new(),
        1 => format!(", and {} is based on it", listed(&above)),
        _ => format!(", and {} are based on it", listed(&above)),
    };
    StackAsk {
        name: format!("stack-{task}-closed-{}", pull.number()),
        question: format!(
            "#{} ({} of {}) closed without a merge{based}. What should the worker do?",
            pull.number(),
            at + 1,
            stack.len()
        ),
        onto: pull.base.clone(),
    }
}

/// Whether a person answered the daemon's ask `name`: their decision stands, so the same event
/// never asks again.
fn answered(tx: &Transaction, name: &str) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM asks WHERE name = ?1 AND answer IS NOT NULL)",
        [name],
        |row| row.get(0),
    )
}

/// The ask when `resource`, the bottom of `stack`, is now based on another branch than its trunk,
/// as when someone retargets it by hand: Priory cannot tell where the stack should land.
fn retargeted(
    task: i64,
    stack: &[crate::stack::Pull],
    resource: &str,
    seen: &Seen,
) -> Option<StackAsk> {
    let bottom = stack.first().filter(|_| stack.len() > 1)?;
    let base = seen.data["base_branch"].as_str()?;
    if bottom.resource != resource || base == bottom.base {
        return None;
    }
    Some(StackAsk {
        name: format!("stack-{task}-retargeted-{}", bottom.number()),
        question: format!(
            "#{}, the bottom of the stack, is now based on {base}, not {}. What should the worker do?",
            bottom.number(),
            bottom.base
        ),
        onto: bottom.base.clone(),
    })
}

/// The open pull requests above position `at` of `stack`, as `#8`.
fn open_above(stack: &[crate::stack::Pull], at: usize) -> Vec<String> {
    stack[at + 1..]
        .iter()
        .filter(|pull| pull.state == "open")
        .map(|pull| format!("#{}", pull.number()))
        .collect()
}

/// `#8`, `#8 and #9`, or `#8, #9 and #10`.
fn listed(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Puts `asked` to a person for `task`, once by its name, and ends the task's gates: the answer
/// wakes the agent. A dismissed or withdrawn ask of that name is asked again as its next revision.
fn ask_stack(tx: &Transaction, task: i64, asked: StackAsk) -> rusqlite::Result<bool> {
    let options = serde_json::json!([format!("restack onto {}", asked.onto), "stop"]).to_string();
    tx.execute(
        "INSERT INTO asks (task, step, kind, name, question, options)
         SELECT id, step, 'agent', ?2, ?3, ?4 FROM tasks WHERE id = ?1
         ON CONFLICT (name) DO UPDATE SET revision = revision + 1, withdrawn = NULL,
           withdrawn_reason = NULL,
           step = excluded.step, question = excluded.question, options = excluded.options
         WHERE withdrawn IS NOT NULL AND answer IS NULL",
        params![task, asked.name, asked.question, options],
    )?;
    tx.execute(
        "UPDATE waits SET withdrawn = 1, withdrawn_reason = 'asked' WHERE task = ?1 AND daemon
         AND fired IS NULL AND withdrawn IS NULL",
        [task],
    )?;
    Ok(true)
}

/// Fires the gate `wait` of a stack's pull request that merged: the task is done once every pull
/// request merged, and wakes to restack the ones still open above it.
fn merged(
    tx: &Transaction,
    wait: i64,
    task: i64,
    resource: &str,
    close: &Seen,
    stack: &[crate::stack::Pull],
    factory: &Factory,
) -> rusqlite::Result<Fired> {
    if stack.iter().all(|pull| pull.state == "merged") {
        return done(tx, wait, task, close.seq, factory);
    }
    let above = stack
        .iter()
        .position(|pull| pull.resource == resource)
        .map(|at| open_above(stack, at))
        .unwrap_or_default();
    if above.is_empty() {
        mark_fired(tx, wait, Some(close.seq))?;
        return Ok((true, None));
    }
    let onto = close.data["base_branch"].as_str().unwrap_or("its base");
    let number = resource.rsplit_once('#').map_or("", |(_, number)| number);
    let text = format!("#{number} merged; rebase {} onto {onto}", listed(&above));
    fire(tx, wait, task, &text, Some(close.seq))?;
    Ok((true, None))
}

/// Tells each task stacked on the merged pull request in `close`, the event `seq`, to rebase its
/// open pull requests onto the merged one's base. A stacked task expects that retarget, so no
/// person is asked: the line fires its gate, or reaches it while it runs.
pub(crate) fn restack_stacked(tx: &Transaction, seq: i64, close: &Value) -> rusqlite::Result<()> {
    let (Some(repo), Some(number), Some(branch)) = (
        close["repo"].as_str(),
        close["pull_request"].as_u64(),
        close["head_branch"].as_str(),
    ) else {
        return Ok(());
    };
    let resource = route::pull_request(repo, number);
    let stacked: Vec<(i64, String)> = tx
        .prepare(
            "SELECT tasks.id, tasks.state FROM tasks JOIN pull_requests AS base
               ON base.task = tasks.base_task AND base.resource = ?1
             WHERE tasks.state NOT IN ('done', 'failed', 'dropped') ORDER BY tasks.id",
        )?
        .query_map([&resource], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let onto = close["base_branch"].as_str().unwrap_or("its base");
    for (task, state) in stacked {
        let stack = crate::stack::of(tx, task)?;
        if stack.first().is_none_or(|bottom| bottom.base != branch) {
            continue;
        }
        let open: Vec<String> = stack
            .iter()
            .filter(|pull| pull.state == "open")
            .map(|pull| format!("#{}", pull.number()))
            .collect();
        if open.is_empty() {
            continue;
        }
        let text = format!("#{number} merged; rebase {} onto {onto}", listed(&open));
        let gate: Option<i64> = tx
            .query_row(
                "SELECT id FROM waits WHERE task = ?1 AND daemon AND fired IS NULL
                 AND withdrawn IS NULL ORDER BY id LIMIT 1",
                [task],
                |row| row.get(0),
            )
            .optional()?;
        match gate {
            Some(gate) => fire(tx, gate, task, &text, Some(seq))?,
            None if state == "running" => {
                tx.execute(
                    "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
                    params![task, text],
                )?;
            }
            None => {}
        }
    }
    Ok(())
}

/// Fires the daemon's checks wait `wait` on the event `seq` and marks its task done, or enters
/// the step the gate waited to enter. A run a person's `say` started ends when the task is done or
/// that step does.
fn done(
    tx: &Transaction,
    wait: i64,
    task: i64,
    seq: i64,
    factory: &Factory,
) -> rusqlite::Result<Fired> {
    mark_fired(tx, wait, Some(seq))?;
    let (workflow, after): (String, Option<String>) = tx.query_row(
        "SELECT tasks.workflow, waits.after FROM waits JOIN tasks ON tasks.id = waits.task
         WHERE waits.id = ?1",
        [wait],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    // A person's `say` may have started a run while the task slept on its gate.
    let live: Option<String> = tx
        .query_row(
            "SELECT session FROM runs WHERE task = ?1 AND ended IS NULL",
            [task],
            |row| row.get(0),
        )
        .optional()?;
    let from = match live {
        Some(session) => run(tx, &session)?,
        None => None,
    };
    let Some(after) = after else {
        set(tx, task, "done", None, None)?;
        let Some(from) = from else {
            return Ok((true, None));
        };
        end(tx, from.id, "done")?;
        return Ok((true, Some(from.tmux)));
    };
    let kill = step_in(
        tx,
        task,
        &workflow,
        &after,
        from.as_ref(),
        "",
        factory,
        false,
    )?;
    Ok((true, kill))
}

/// Whether `task` owns a pull request and every one of its stack merged.
pub(crate) fn all_merged(tx: &Connection, task: i64) -> rusqlite::Result<bool> {
    let stack = crate::stack::of(tx, task)?;
    if !stack.is_empty() {
        return Ok(stack.iter().all(|pull| pull.state == "merged"));
    }
    let Some(resource) = pull_request(tx, task)? else {
        return Ok(false);
    };
    let merged = tx
        .query_row(
            "SELECT json_extract(data, '$.merged') FROM inbox WHERE type = ?2
             AND 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1
             ORDER BY seq DESC LIMIT 1",
            params![resource, CLOSED],
            |row| row.get::<_, Option<bool>>(0),
        )
        .optional()?
        .flatten();
    Ok(merged == Some(true))
}

/// Whether a pull request `task` owns closed without a merge: one of its stack, or its only one.
pub(crate) fn closed_unmerged(db: &Connection, task: i64) -> rusqlite::Result<bool> {
    let stack = crate::stack::of(db, task)?;
    if !stack.is_empty() {
        return Ok(stack.iter().any(|pull| pull.state == "closed"));
    }
    let Some(resource) = pull_request(db, task)? else {
        return Ok(false);
    };
    let merged = db
        .query_row(
            "SELECT json_extract(data, '$.merged') FROM inbox WHERE type = ?2
             AND 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1
             ORDER BY seq DESC LIMIT 1",
            params![resource, CLOSED],
            |row| row.get::<_, Option<bool>>(0),
        )
        .optional()?
        .flatten();
    Ok(merged == Some(false))
}

/// Fires every open time wait whose instant passed and task wait whose task ended.
pub(crate) fn due(tx: &Transaction) -> rusqlite::Result<()> {
    let mut fired = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT waits.id, waits.task, waits.until, waits.target, target.state FROM waits
             JOIN tasks ON tasks.id = waits.task LEFT JOIN tasks AS target ON target.id = waits.target
             WHERE waits.fired IS NULL AND waits.withdrawn IS NULL AND tasks.state NOT IN ('done', 'failed', 'dropped')
             AND ((waits.kind = 'time' AND waits.until <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
               OR (waits.kind = 'task' AND target.state IN ('done', 'failed', 'dropped')))
             ORDER BY waits.id",
        )?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let text = match row.get::<_, Option<String>>(2)? {
                Some(until) => format!("wait {id} fired: {until} passed"),
                None => format!(
                    "wait {id} fired: task {} is {}",
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?
                ),
            };
            fired.push((id, row.get::<_, i64>(1)?, text));
        }
    }
    for (id, task, text) in &fired {
        fire(tx, *id, *task, text, None)?;
    }
    Ok(())
}

/// Readies each queued or ready task's step before a run or action takes it: a task requested
/// before its workflow existed starts at the workflow's start once it does, a step its workflow
/// lost blocks it, and a human step gets its ask.
pub(crate) fn enter_queued(db: &mut Connection, factory: &Factory) -> rusqlite::Result<()> {
    let workflows = &factory.workflows;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let tasks = tx
        .prepare(
            "SELECT id, workflow, step, state,
               EXISTS (SELECT 1 FROM asks WHERE task = tasks.id AND answer IS NULL AND withdrawn IS NULL)
             FROM tasks WHERE state IN ('queued', 'ready')",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (task, workflow, at, state, asked) in tasks {
        let Some(flow) = workflows.get(&workflow) else {
            let reason = format!("waits for workflow {workflow}, which the factory lacks");
            tx.execute(
                "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
                params![task, reason],
            )?;
            continue;
        };
        let at = if at.is_empty() {
            set(&tx, task, &state, Some(&flow.value.start), None)?;
            flow.value.start.clone()
        } else {
            at
        };
        match flow.value.step(&at) {
            None => {
                let reason = format!("step {at} is not in workflow {workflow}");
                set(&tx, task, "blocked", None, Some(&reason))?;
            }
            Some(step) if step.decides == Decider::Human && !asked => {
                enter(&tx, task, &workflow, &at, None, "", factory)?;
            }
            Some(_) => {}
        }
    }
    tx.commit()
}

const GB: f64 = (1u64 << 30) as f64;

/// Fires every open disk wait that `free` bytes of free disk satisfy.
pub(crate) fn disk_due(tx: &Transaction, free: u64) -> rusqlite::Result<()> {
    let fired = tx
        .prepare(
            "SELECT waits.id, waits.task FROM waits JOIN tasks ON tasks.id = waits.task
             WHERE waits.fired IS NULL AND waits.withdrawn IS NULL AND waits.kind = 'disk'
             AND tasks.state NOT IN ('done', 'failed', 'dropped') AND waits.bytes <= ?1 ORDER BY waits.id",
        )?
        .query_map([free as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let text = format!("{:.1} GB of disk free", free as f64 / GB);
    for (id, task) in fired {
        fire(tx, id, task, &format!("wait {id} fired: {text}"), None)?;
    }
    Ok(())
}

/// Fires `wait` and wakes its task with `text`.
fn fire(
    tx: &Transaction,
    wait: i64,
    task: i64,
    text: &str,
    seq: Option<i64>,
) -> rusqlite::Result<()> {
    if mark_fired(tx, wait, seq)? {
        // The other gates of a stack end with this wake: the next `done` stores fresh ones.
        tx.execute(
            "UPDATE waits SET withdrawn = 1, withdrawn_reason = 'sibling-fired' WHERE task = ?2
             AND daemon AND fired IS NULL AND withdrawn IS NULL
             AND (SELECT daemon FROM waits WHERE id = ?1)",
            params![wait, task],
        )?;
        wake_on(tx, task, text, seq)?;
    }
    Ok(())
}

/// Marks `wait` fired on the event `seq`, unless it fired or was withdrawn already, and withdraws
/// the other open waits and the open ask of its run. Says whether it fired now.
fn mark_fired(tx: &Transaction, wait: i64, seq: Option<i64>) -> rusqlite::Result<bool> {
    let fired = tx.execute(
        &format!(
            "UPDATE waits SET fired = {NOW}, inbox = ?2 WHERE id = ?1 AND fired IS NULL AND withdrawn IS NULL"
        ),
        params![wait, seq],
    )?;
    if fired == 0 {
        return Ok(false);
    }
    tx.execute(
        "UPDATE waits SET withdrawn = 1, withdrawn_reason = 'sibling-fired' WHERE id != ?1
         AND fired IS NULL AND withdrawn IS NULL AND NOT daemon
         AND run = (SELECT run FROM waits WHERE id = ?1 AND NOT daemon)",
        [wait],
    )?;
    tx.execute(
        "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'sibling-fired' WHERE answer IS NULL
         AND withdrawn IS NULL AND run = (SELECT run FROM waits WHERE id = ?1 AND NOT daemon)",
        [wait],
    )?;
    Ok(true)
}

/// Withdraws the open waits of the run that stored the ask `ask`, now that a person settled it as
/// `reason` says.
fn withdraw_waits(tx: &Transaction, ask: i64, reason: &str) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE waits SET withdrawn = 1, withdrawn_reason = ?2 WHERE fired IS NULL
         AND withdrawn IS NULL AND NOT daemon AND run = (SELECT run FROM asks WHERE id = ?1)",
        params![ask, reason],
    )
    .map(drop)
}

/// Queues `text`, then the event `seq` unless `text` is its line or another prompt carries it, then
/// the kept events no prompt carries, as the task's next prompt, and makes an asleep task ready.
pub(crate) fn wake_on(
    tx: &Transaction,
    task: i64,
    text: &str,
    seq: Option<i64>,
) -> rusqlite::Result<()> {
    let mut carried = Vec::new();
    let mut text = text.to_owned();
    if let Some(seq) = seq {
        let (prompt, own, line): (Option<i64>, bool, String) = tx.query_row(
            "SELECT inbox.outbox, coalesce(outbox.state = 'queued' AND substr(outbox.text, 1, length(line)) = line, 0), line
             FROM (SELECT seq, outbox, type || ' ' || coalesce(subject, '-') || ' ' || data AS line
                   FROM inbox WHERE seq = ?1) AS inbox
             LEFT JOIN outbox ON outbox.id = inbox.outbox",
            [seq],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        match (prompt, own) {
            // A running task's event has a queued prompt of its own: take it out so it comes first.
            (Some(prompt), true) => {
                tx.execute("UPDATE inbox SET outbox = NULL WHERE seq = ?1", [seq])?;
                tx.execute("DELETE FROM outbox WHERE id = ?1", [prompt])?;
                carried.push(seq);
            }
            // Another prompt carries the event already: only a wake reason of the daemon adds to it.
            (Some(_), false) if text == line => return set_ready(tx, task),
            (Some(_), false) => {}
            (None, _) => carried.push(seq),
        }
        if carried.contains(&seq) && text != line {
            text = format!("{text}\n{line}");
        }
    }
    {
        let mut statement = tx.prepare(
            "SELECT seq, type, subject, data FROM inbox
             WHERE task = ?1 AND outcome = 'kept' AND outbox IS NULL AND seq IS NOT ?2 ORDER BY seq",
        )?;
        let mut rows = statement.query(params![task, seq])?;
        while let Some(row) = rows.next()? {
            carried.push(row.get(0)?);
            text.push('\n');
            text.push_str(&crate::state::line(
                &row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.as_deref(),
                &row.get::<_, String>(3)?,
            ));
        }
    }
    if crate::triage::any(tx, &carried)? {
        text = format!("{text}\n{}", crate::triage::RULE);
    }
    let id = wake(tx, task, &text)?;
    for seq in carried {
        tx.execute("UPDATE inbox SET outbox = ?1 WHERE seq = ?2", [id, seq])?;
    }
    set_ready(tx, task)
}

fn set_ready(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE tasks SET state = 'ready', reason = NULL WHERE id = ?1 AND state = 'asleep'",
        [task],
    )
    .map(drop)
}

/// After a run's turn ends: moves its task by its result, or puts it to sleep on its open ask or wait.
/// Returns the tmux session to kill once the hook has its reply.
pub(crate) fn stop(
    tx: &Transaction,
    session: &str,
    trace: &Trace,
) -> rusqlite::Result<Option<String>> {
    let Some(run) = run(tx, session)? else {
        return Ok(None);
    };
    if let Some(kill) = settle(tx, &run, trace)? {
        return Ok(kill);
    }
    nudge(tx, run.id, run.task)?;
    Ok(None)
}

/// Measure a change to it first with `nudge.eval.toml` (docs/evaluating-a-prompt.md).
const NUDGE: &str = include_str!("nudge.txt");

/// Queues one nudge, ahead of any queued event, after a silent turn that Priory typed.
fn nudge(tx: &Transaction, run: i64, task: i64) -> rusqlite::Result<()> {
    let (nudgeable, ending): (bool, Option<String>) = tx.query_row(
        "SELECT runs.nudgeable, tasks.ending FROM runs JOIN tasks ON tasks.id = runs.task
         WHERE runs.id = ?1",
        [run],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !nudgeable {
        return Ok(());
    }
    let text = match ending {
        Some(ending) => format!("{}\n{ending}", NUDGE.trim_end()),
        None => NUDGE.trim_end().to_owned(),
    };
    let id = queue_first(tx, task, &text)?;
    tx.execute("UPDATE outbox SET nudge = 1 WHERE id = ?1", [id])
        .map(drop)
}

/// Settles the live run of `session`, whose agent process is gone, as its `Stop` would have, when a
/// result, ask or wait ended its turn. Says whether one did; otherwise nothing changes.
pub(crate) fn gone(db: &mut Connection, session: &str, trace: &Trace) -> rusqlite::Result<bool> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(mut run) = run(&tx, session)? else {
        return Ok(false);
    };
    // No process carries on, so even a next step of the same agent needs a new run.
    run.agent = None;
    trace::account(&tx, trace, run.id, None)?;
    let settled = settle(&tx, &run, trace)?.is_some();
    tx.commit()?;
    Ok(settled)
}

/// The outcomes that grade a task.
pub const GRADES: [&str; 3] = ["red", "yellow", "green"];

/// Moves the run's task by its result, or puts it to sleep on its open ask or wait. `None` when
/// none of them ended the turn; otherwise the tmux session to kill, if any.
fn settle(tx: &Transaction, run: &Run, trace: &Trace) -> rusqlite::Result<Option<Option<String>>> {
    let workflows = &trace.factory.workflows;
    let outcome = tx
        .query_row(
            "SELECT outcome, summary, reason FROM results WHERE run = ?1",
            [run.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((outcome, summary, reason)) = outcome {
        if GRADES.contains(&outcome.as_str()) {
            let reason = reason.as_deref().unwrap_or(&summary);
            tx.execute(
                "UPDATE tasks SET grade = ?2, grade_reason = ?3 WHERE id = ?1",
                // A tab would split the reason across `priory tasks` columns.
                params![
                    run.task,
                    outcome,
                    reason.lines().next().unwrap_or_default().replace('\t', " ")
                ],
            )?;
        }
        tx.execute(
            "UPDATE tasks SET merge_risk = (SELECT merge_risk FROM results WHERE run = ?2)
             WHERE id = ?1 AND EXISTS (SELECT 1 FROM results WHERE run = ?2 AND merge_risk IS NOT NULL)",
            params![run.task, run.id],
        )?;
        let target = step(workflows, &run.workflow, &run.step)
            .and_then(|step| step.next.get(&outcome))
            .cloned()
            .unwrap_or_else(|| format!("{}?{outcome}", run.step));
        trace::ended(tx, trace, run.task, &outcome, Some(run.id))?;
        if gates_plan(tx, run, &outcome, &target, workflows)? {
            tx.execute(
                "INSERT INTO asks (task, step, kind, question, options, text) VALUES (?1, ?2, 'step', ?3, ?4, ?5)",
                params![
                    run.task,
                    run.step,
                    format!("{}: approve the plan graded {outcome}? {summary}", run.step),
                    serde_json::to_string(&PLAN_OPTIONS).expect("options serialize"),
                    ask_text(tx, run.task)?
                ],
            )?;
            end(tx, run.id, "asleep")?;
            set(tx, run.task, "asleep", None, None)?;
            return Ok(Some(Some(run.tmux.clone())));
        }
        let tmux = enter(
            tx,
            run.task,
            &run.workflow,
            &target,
            Some(run),
            &summary,
            trace.factory,
        )?;
        if target == "stop" {
            stopped(tx, run, &outcome, &summary)?;
        }
        return Ok(Some(tmux));
    }
    // A schedule's tick wakes a task with its ask or wait still open: this run sleeps on them again.
    tx.execute(
        "UPDATE asks SET run = ?2 WHERE task = ?1 AND run IS NOT NULL AND answer IS NULL AND withdrawn IS NULL",
        params![run.task, run.id],
    )?;
    tx.execute(
        "UPDATE waits SET run = ?2 WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL AND NOT daemon",
        params![run.task, run.id],
    )?;
    // An answer whose command still runs wakes the task with its outcome; until then the task stays
    // `running`, so the sweep leaves its worktree alone.
    if commanding(tx, run.task)? {
        end(tx, run.id, "asleep")?;
        return Ok(Some(Some(run.tmux.clone())));
    }
    let asked = tx
        .query_row(
            "SELECT 1 FROM asks WHERE run = ?1 AND answer IS NULL AND withdrawn IS NULL",
            [run.id],
            |_| Ok(()),
        )
        .optional()?;
    if asked.is_none() && open_wait(tx, run.id)?.is_none() {
        return Ok(None);
    }
    // A person's `say` that beat the Stop is the run's next prompt, as an answer would be.
    let said = tx
        .query_row(
            "SELECT 1 FROM outbox WHERE task = ?1 AND state = 'queued' AND wakes",
            [run.task],
            |_| Ok(()),
        )
        .optional()?;
    if said.is_some() {
        return Ok(None);
    }
    end(tx, run.id, "asleep")?;
    set(tx, run.task, "asleep", None, None)?;
    Ok(Some(Some(run.tmux.clone())))
}

/// The trigger a lead task names. No trigger file has a `/` in its name, so none collides.
pub const LEAD_TRIGGER: &str = "priory/lead";

/// The lead a new ask of `task` at step `at` goes to: a `hands-off` task's lead, unless the ask
/// runs a command, its answer can let an irreversible step run, or its step approves one.
fn lead_of(
    tx: &Transaction,
    task: i64,
    at: &str,
    command: bool,
    workflows: &Workflows,
) -> rusqlite::Result<Option<String>> {
    let (workflow, intervention, lead): (String, Option<String>, Option<String>) = tx.query_row(
        "SELECT workflow, intervention, lead FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if command || intervention.as_deref() != Some(Intervention::HandsOff.as_str()) {
        return Ok(None);
    }
    let Some(flow) = workflows.get(&workflow) else {
        return Ok(None);
    };
    // A lead no trigger names any more has no workflow to answer in.
    if lead
        .as_ref()
        .is_none_or(|lead| !workflows.contains_key(&lead_workflow(lead)))
    {
        return Ok(None);
    }
    let approves = flow
        .value
        .step(at)
        .is_some_and(|step| !step.approves.is_empty());
    if approves || explain::answer_lets_irreversible_run(&flow.value, at) {
        return Ok(None);
    }
    Ok(lead)
}

/// Queues a lead task that answers `ask` in the person's place. Its source event carries the ask,
/// so its first prompt names it.
fn call_lead(tx: &Transaction, ask: i64, lead: &str) -> rusqlite::Result<()> {
    let source = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO tasks
           (key, workflow, step, state, band, lane, lane_derived, trigger, source, repo, entered)
         SELECT 'priory:ask:' || asks.id || ':' || asks.revision, ?2, 'answer', 'queued',
           tasks.band, tasks.lane, 0, ?3, ?4, tasks.repo, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
        params![ask, lead_workflow(lead), LEAD_TRIGGER, source],
    )?;
    let task = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO owns (task, resource) SELECT id, key FROM tasks WHERE id = ?1",
        [task],
    )?;
    tx.execute(
        "INSERT INTO inbox (id, type, time, data, links, task, outcome)
         SELECT ?2, 'priory.ask.routed', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
           json_object('task', asks.task, 'key', tasks.key, 'ask', asks.id,
             'revision', asks.revision, 'question', asks.question, 'options', json(asks.options),
             'recommendation', asks.recommendation, 'tradeoff', asks.tradeoff,
             'title', asks.title, 'text', asks.text),
           '[]', ?3, 'created'
         FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
        params![ask, source, task],
    )?;
    Ok(())
}

/// Applies a lead task's `answered` result to the ask it was called for, as answered by its agent.
fn lead_answer(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
    trace: &Trace,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Result<(), String>> {
    if arguments["outcome"] != "answered" {
        return Ok(Ok(()));
    }
    let called = tx
        .query_row(
            "SELECT json_extract(inbox.data, '$.ask'), json_extract(inbox.data, '$.revision')
             FROM tasks JOIN inbox ON inbox.id = tasks.source
             WHERE tasks.id = ?1 AND tasks.trigger = ?2 AND inbox.type = 'priory.ask.routed'",
            params![run.task, LEAD_TRIGGER],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((ask, revision)) = called else {
        return Ok(Ok(()));
    };
    let text = arguments["summary"].as_str().unwrap_or_default().trim();
    let options: String = tx.query_row("SELECT options FROM asks WHERE id = ?1", [ask], |row| {
        row.get(0)
    })?;
    let options: Vec<String> = serde_json::from_str(&options).expect("options are stored as JSON");
    // A person may answer an agent's ask in free text; a lead picks one of its options.
    if !options.is_empty() && !options.iter().any(|option| option == text) {
        return Ok(Err(format!(
            "ask {ask} takes one of: {}",
            options.join(", ")
        )));
    }
    let answer = Answer {
        ask,
        text: text.to_owned(),
        revision,
        run: false,
        by: format!("agent:{}", run.agent.as_deref().unwrap_or_default()),
    };
    Ok(answer_in(tx, &answer, trace, head)?.map(drop))
}

/// Drops the lead task still queued for `ask`, which a person answered or dismissed first.
fn drop_lead(tx: &Transaction, ask: i64, why: &str) -> rusqlite::Result<()> {
    let queued = tx
        .prepare(
            "SELECT id FROM tasks WHERE trigger = ?1 AND state = 'queued'
             AND key LIKE 'priory:ask:' || ?2 || ':%'",
        )?
        .query_map(params![LEAD_TRIGGER, ask], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for task in queued {
        set(tx, task, "dropped", None, Some(why))?;
    }
    Ok(())
}

/// The options of the ask a `plan` task's grading step waits on.
const PLAN_OPTIONS: [&str; 2] = ["approved", "rejected"];

/// Whether a `plan` task's grade waits for a person: the grade leads to a model or code step,
/// so no human step of the workflow approves the plan.
fn gates_plan(
    tx: &Transaction,
    run: &Run,
    outcome: &str,
    target: &str,
    workflows: &Workflows,
) -> rusqlite::Result<bool> {
    if !GRADES.contains(&outcome) {
        return Ok(false);
    }
    let unattended =
        step(workflows, &run.workflow, target).is_some_and(|step| step.decides != Decider::Human);
    if !unattended {
        return Ok(false);
    }
    let intervention: Option<String> = tx.query_row(
        "SELECT intervention FROM tasks WHERE id = ?1",
        [run.task],
        |row| row.get(0),
    )?;
    Ok(intervention.as_deref() == Some(Intervention::Plan.as_str()))
}

const RESUMED: &str = "A person resumed this task";
const CONTINUE: &str = "Check the worktree, then continue.";

/// Takes the line of an earlier resume that no run took out of the task's queued prompt.
fn unresume(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    let queued = tx
        .query_row(
            "SELECT id, text FROM outbox WHERE task = ?1 AND state = 'queued' ORDER BY id LIMIT 1",
            [task],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((id, text)) = queued else {
        return Ok(());
    };
    // A shared session's task intro may come first.
    let Some(start) = (text.starts_with(RESUMED))
        .then_some(0)
        .or_else(|| text.find(&format!("\n{RESUMED}")).map(|at| at + 1))
    else {
        return Ok(());
    };
    let end = format!("{CONTINUE}\n");
    let Some(length) = text[start..].find(&end) else {
        return Ok(());
    };
    let rest = format!("{}{}", &text[..start], &text[start + length + end.len()..]);
    tx.execute(
        "UPDATE outbox SET text = ?2 WHERE id = ?1",
        params![id, rest],
    )
    .map(drop)
}

/// Queues `text` as a wake prompt: `text`, then how to end the step the task is in, if it is a
/// model step and no queued prompt says so already.
fn wake(tx: &Transaction, task: i64, text: &str) -> rusqlite::Result<i64> {
    let ending: Option<String> = tx
        .query_row(
            "SELECT ending FROM tasks WHERE id = ?1 AND NOT EXISTS (
           SELECT 1 FROM outbox WHERE task = ?1 AND state = 'queued'
           AND instr(char(10) || text || char(10), char(10) || ending || char(10)) > 0)",
            [task],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    match ending {
        Some(ending) => queue_first(tx, task, &format!("{text}\n{ending}")),
        None => queue_first(tx, task, text),
    }
}

/// Queues `text` as the task's next prompt, ahead of the prompts already queued, which it carries
/// after it: an event kept during the last turn must not reach the agent before its new instructions.
fn queue_first(tx: &Transaction, task: i64, text: &str) -> rusqlite::Result<i64> {
    let mut merged = text.to_owned();
    let mut queued = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT id, text FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0
             ORDER BY id",
        )?;
        let mut rows = statement.query([task])?;
        while let Some(row) = rows.next()? {
            queued.push(row.get::<_, i64>(0)?);
            merged.push('\n');
            merged.push_str(&row.get::<_, String>(1)?);
        }
    }
    tx.execute(
        "INSERT INTO outbox (task, text, wakes) VALUES (?1, ?2, (SELECT coalesce(max(wakes), 0)
           FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0))",
        params![task, merged],
    )?;
    let id = tx.last_insert_rowid();
    for old in queued {
        tx.execute("UPDATE inbox SET outbox = ?1 WHERE outbox = ?2", [id, old])?;
        tx.execute(
            "UPDATE subscription_deliveries SET outbox = ?1 WHERE outbox = ?2",
            [id, old],
        )?;
        tx.execute("DELETE FROM outbox WHERE id = ?1", [old])?;
    }
    // A queued `/compact` must be typed alone, so it follows the new prompt instead of joining it.
    let moved = tx.execute(
        "INSERT INTO outbox (task, text, compact) SELECT task, text, 1 FROM outbox
         WHERE task = ?1 AND state = 'queued' AND compact = 1 AND id < ?2",
        [task, id],
    )?;
    if moved > 0 {
        tx.execute(
            "UPDATE runs SET crossed = ?3 WHERE crossed IN (SELECT id FROM outbox
             WHERE task = ?1 AND state = 'queued' AND compact = 1 AND id < ?2)",
            params![task, id, tx.last_insert_rowid()],
        )?;
    }
    tx.execute(
        "DELETE FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 1 AND id < ?2",
        [task, id],
    )?;
    Ok(id)
}

/// The pull request the task owns: the latest one its actions opened, else the one its latest wait
/// named, else the latest one opened from its branch, else its key when a pull request event created
/// it. A review request's task reviews the pull request and does not own it.
pub(crate) fn pull_request(tx: &Connection, task: i64) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT resource FROM (
           SELECT pull_request AS resource, 3 AS rank, id AS at FROM actions
             WHERE task = ?1 AND pull_request IS NOT NULL
           UNION ALL
           SELECT resource, 2, id FROM waits
             WHERE task = ?1 AND kind IN ('pull_request', 'checks', 'review')
           UNION ALL
           SELECT 'github:' || json_extract(inbox.data, '$.repo') || '#' || json_extract(inbox.data, '$.pull_request'), 1, inbox.seq
             FROM inbox JOIN tasks ON tasks.id = inbox.task
             WHERE inbox.task = ?1 AND inbox.outcome IN ('kept', 'woke')
             AND json_extract(inbox.data, '$.pull_request') IS NOT NULL
             AND json_extract(inbox.data, '$.head_branch') = tasks.branch
           UNION ALL
           SELECT tasks.key, 0, 0 FROM tasks JOIN inbox ON inbox.id = tasks.source
             WHERE tasks.id = ?1 AND inbox.type LIKE 'github.pull_request.%'
             AND inbox.type NOT LIKE 'github.pull_request.review_request%'
         ) ORDER BY rank DESC, at DESC LIMIT 1",
        [task],
        |row| row.get(0),
    )
    .optional()
}

fn end(tx: &Transaction, run: i64, ended_as: &str) -> rusqlite::Result<()> {
    tx.execute(
        &format!("UPDATE runs SET ended = {NOW}, ended_as = ?2 WHERE id = ?1"),
        params![run, ended_as],
    )
    .map(drop)
}

fn set(
    tx: &Transaction,
    task: i64,
    state: &str,
    step: Option<&str>,
    reason: Option<&str>,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE tasks SET state = ?2, step = coalesce(?3, step), reason = ?4,
         entered = iif(?3 IS NULL, entered, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
         tokens = iif(?3 IS NULL, tokens, NULL), model = iif(?3 IS NULL, model, NULL) WHERE id = ?1",
        params![task, state, step, reason],
    )
    .map(drop)
}

/// Keeps the first line of the summary of the result that stopped the run's task as the task's
/// reason, and posts it as a report, so the desk shows why the work ended.
fn stopped(tx: &Transaction, run: &Run, outcome: &str, summary: &str) -> rusqlite::Result<()> {
    // A tab would split the reason across `priory tasks` columns.
    let line = summary
        .lines()
        .next()
        .unwrap_or_default()
        .replace('\t', " ");
    tx.execute(
        "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND state = 'dropped'",
        params![run.task, line],
    )?;
    let key: String = tx.query_row("SELECT key FROM tasks WHERE id = ?1", [run.task], |row| {
        row.get(0)
    })?;
    let line = format!(
        "task {} ({key}) stopped at {}, {outcome}: {line}",
        run.task, run.step
    );
    crate::post::report(tx, "priory", &line)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))
}

/// Moves `task` to `target`: a step id, `done` or `stop`. `from` is the run whose result moved it,
/// still live; an answer moves an asleep task with none. Returns the tmux session to kill.
fn enter(
    tx: &Transaction,
    task: i64,
    workflow: &str,
    target: &str,
    from: Option<&Run>,
    context: &str,
    factory: &Factory,
) -> rusqlite::Result<Option<String>> {
    step_in(tx, task, workflow, target, from, context, factory, true)
}

/// As `enter`. `gate` false enters a step after the merge without its gate, as the merge that
/// fired the gate does.
#[allow(clippy::too_many_arguments)]
fn step_in(
    tx: &Transaction,
    task: i64,
    workflow: &str,
    target: &str,
    from: Option<&Run>,
    context: &str,
    factory: &Factory,
    gate: bool,
) -> rusqlite::Result<Option<String>> {
    let workflows = &factory.workflows;
    let finish = |ended_as: &str| -> rusqlite::Result<Option<String>> {
        let Some(run) = from else {
            return Ok(None);
        };
        end(tx, run.id, ended_as)?;
        Ok(Some(run.tmux.clone()))
    };
    let merge_step =
        step(workflows, workflow, target).filter(|s| s.after == Some(StepAfter::Merged));
    let after = merge_step.filter(|_| gate).map(|_| target);
    let owned = pull_request(tx, task)?;
    if after.is_some() && owned.is_none() {
        let reason = format!("step {target} waits for a merge, but the task owns no pull request");
        set(tx, task, "blocked", None, Some(&reason))?;
        return finish("blocked");
    }
    if (target == "done" || after.is_some())
        && !all_merged(tx, task)?
        && let Some(resource) = owned
    {
        let last = tx
            .query_row(
                "SELECT results.run, results.step, coalesce(results.commit_sha,
                   (SELECT commit_sha FROM actions WHERE result = results.run AND status = 'opened'
                    ORDER BY id DESC LIMIT 1))
                 FROM results WHERE task = ?1 ORDER BY run DESC LIMIT 1",
                [task],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((result_run, at, Some(commit))) = last else {
            let reason =
                format!("task reached {target} owning {resource}, but no result names a commit");
            set(tx, task, "blocked", None, Some(&reason))?;
            return finish("blocked");
        };
        let stack = crate::stack::of(tx, task)?;
        // A closed pull request of a stack waits for a person's decision before anything else.
        for closed in (0..stack.len()).filter(|&at| stack.len() > 1 && stack[at].state == "closed")
        {
            let asked = stranded(task, &stack, closed);
            if answered(tx, &asked.name)? {
                continue;
            }
            ask_stack(tx, task, asked)?;
            set(tx, task, "asleep", Some(&at), None)?;
            set_ending(tx, task, step(workflows, workflow, &at))?;
            return finish("asleep");
        }
        // A stack gates each open pull request, its top at the result's commit; one is `resource`.
        let gates: Vec<(String, Option<&str>)> = match stack.split_last() {
            Some((top, lower)) if !lower.is_empty() => stack
                .iter()
                .filter(|pull| pull.state == "open")
                .map(|pull| {
                    let pinned = (pull.resource == top.resource).then_some(commit.as_str());
                    (pull.resource.clone(), pinned)
                })
                .collect(),
            _ => vec![(resource, Some(commit.as_str()))],
        };
        // A stack with no open pull request left is past its gates.
        if !gates.is_empty() {
            let mut waits = Vec::new();
            for (resource, pinned) in gates {
                tx.execute(
                    "INSERT INTO waits (task, run, kind, resource, head_sha, daemon, after)
                 VALUES (?1, ?2, 'checks', ?3, ?4, 1, ?5)",
                    params![task, result_run, resource, pinned, after],
                )?;
                waits.push(tx.last_insert_rowid());
            }
            // The agent of the result's step fixes what the checks find, even after a human step.
            set(tx, task, "asleep", Some(&at), None)?;
            set_ending(tx, task, step(workflows, workflow, &at))?;
            for wait in waits {
                judge(tx, wait, factory)?;
            }
            return finish("asleep");
        }
        // Answered asks about closed pull requests left no gate, and no merge.
        if after.is_some() {
            let reason = format!(
                "step {target} waits for a merge, but a pull request of the stack closed without one"
            );
            set(tx, task, "blocked", None, Some(&reason))?;
            return finish("blocked");
        }
    }
    if target == "done" || target == "stop" {
        let state = if target == "done" { "done" } else { "dropped" };
        set(tx, task, state, None, None)?;
        return finish(state);
    }
    let Some(step) = step(workflows, workflow, target) else {
        let reason = format!("step {target} is not in workflow {workflow}");
        set(tx, task, "blocked", None, Some(&reason))?;
        return finish("blocked");
    };
    drop_ending(tx, task)?;
    set_ending(tx, task, Some(step))?;
    match step.decides {
        Decider::Model => {
            queue_first(tx, task, &entry_prompt(tx, task, step)?)?;
            // An irreversible step starts a new run, whose start checks its approval.
            let carry = |run: &&Run| run.agent == step.agent && step.effect != Effect::Irreversible;
            let Some(run) = from.filter(carry) else {
                set(tx, task, "ready", Some(target), None)?;
                return finish("moved");
            };
            end(tx, run.id, "moved")?;
            tx.execute(
                "INSERT INTO runs (task, step, agent, tmux, session, idle, ready, transcript, read, thread)
                 SELECT ?1, ?2, ?3, ?4, ?5, 1, 1, transcript, read, thread FROM runs WHERE id = ?6",
                params![task, target, run.agent, run.tmux, run.session, run.id],
            )?;
            // Its process carries on, so an unconfirmed prompt may still sit in its input.
            tx.execute(
                "UPDATE outbox SET run = ?2 WHERE run = ?1 AND state IN ('attempted', 'uncertain')",
                params![run.id, tx.last_insert_rowid()],
            )?;
            set(tx, task, "running", Some(target), None)?;
            Ok(None)
        }
        Decider::Human => {
            let go = crate::batch::go_ask(tx, task)?;
            // A result's summary is no news on a go; a failed start's error is.
            let question = match (&go, context.is_empty()) {
                (Some((asked, _)), _) if context.is_empty() || from.is_some() => asked.clone(),
                (Some((asked, _)), _) => format!("{context}. {asked}"),
                (None, true) => target.to_owned(),
                (None, false) => format!("{target}: {context}"),
            };
            let text = match go {
                Some((_, plan)) => Some(plan),
                None => ask_text(tx, task)?,
            };
            let command = match step_command(tx, task, step, factory)? {
                Ok(command) => command,
                Err(error) => {
                    let reason = format!("step {target}: {error}");
                    set(tx, task, "blocked", Some(target), Some(&reason))?;
                    return finish("blocked");
                }
            };
            let lead = lead_of(tx, task, target, command.is_some(), workflows)?;
            tx.execute(
                "INSERT INTO asks (task, step, kind, question, options, routed, command, text)
                 VALUES (?1, ?2, 'step', ?3, ?4, ?5, ?6, ?7)",
                params![
                    task,
                    target,
                    question,
                    serde_json::to_string(&step.options).expect("options serialize"),
                    lead,
                    command,
                    text
                ],
            )?;
            if let Some(lead) = &lead {
                call_lead(tx, tx.last_insert_rowid(), lead)?;
            }
            set(tx, task, "asleep", Some(target), None)?;
            finish("asleep")
        }
        Decider::Code => {
            set(tx, task, "ready", Some(target), None)?;
            finish("moved")
        }
    }
}

/// Applies a person's answer, or records why it was refused on the ask it names.
pub(crate) fn answer(
    db: &mut Connection,
    answer: &Answer,
    trace: &Trace,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let applied = answer_in_tx(&tx, answer, trace, head)?;
    tx.commit()?;
    Ok(applied)
}

/// Applies a person's answer in `tx`, or records why it was refused. The first answer to an ask's
/// current revision stands. A step's ask moves the task by the chosen option; an agent's ask queues
/// the answer as its next prompt. An answer toward a step the asked step `approves` records an
/// approval at the task's commit, which `head` reads from its worktree.
pub(crate) fn answer_in_tx(
    tx: &Transaction,
    answer: &Answer,
    trace: &Trace,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Result<String, String>> {
    let applied = answer_in(tx, answer, trace, head)?;
    if let Err(why) = &applied {
        tx.execute(
            "INSERT INTO ask_refusals (ask, revision, answer, by, why)
             SELECT id, ?2, ?3, ?4, ?5 FROM asks WHERE id = ?1",
            params![answer.ask, answer.revision, answer.text, answer.by, why],
        )?;
    }
    Ok(applied)
}

/// Applies `answer` in `tx`, writing nothing when it refuses it.
fn answer_in(
    tx: &Transaction,
    answer: &Answer,
    trace: &Trace,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Result<String, String>> {
    let workflows = &trace.factory.workflows;
    let OpenAsk {
        label,
        task,
        kind,
        step: at,
        options,
        workflow,
        state,
        command,
    } = match open_ask(tx, answer.ask, Some(answer.revision))? {
        Ok(ask) => ask,
        Err(error) => return Ok(Err(error)),
    };
    let options: Vec<String> = serde_json::from_str(&options).expect("options are stored as JSON");
    let hold = crate::hold::of_ask(tx, answer.ask)?;
    let (chosen, note) = match answer.text.split_once(':') {
        Some((option, note))
            if kind == "step"
                && !options.contains(&answer.text)
                && options.iter().any(|known| known == option.trim())
                && !note.trim().is_empty() =>
        {
            (option.trim().to_owned(), Some(note.trim()))
        }
        _ => (answer.text.clone(), None),
    };
    let runs = command
        .as_deref()
        .filter(|_| kind == "agent" || kind == "step")
        .and_then(|command| serde_json::from_str::<command::Command>(command).ok())
        .filter(|command| command.runs.contains(&chosen));
    if (kind == "step" || hold.is_some()) && !options.contains(&chosen) {
        return Ok(Err(format!(
            "ask {label} takes one of: {}",
            options.join(", ")
        )));
    }
    let asked = step(workflows, &workflow, &at);
    let target = match (kind == "step", asked) {
        (false, _) => None,
        // A `plan` task's grading step: an approval moves it by its grade.
        (true, Some(asked)) if asked.decides == Decider::Model => Some(match chosen.as_str() {
            "approved" => {
                let grade: Option<String> = match task {
                    Some(task) => {
                        tx.query_row("SELECT grade FROM tasks WHERE id = ?1", [task], |row| {
                            row.get(0)
                        })?
                    }
                    None => None,
                };
                grade
                    .as_deref()
                    .and_then(|grade| asked.next.get(grade))
                    .cloned()
                    .unwrap_or_else(|| format!("{at}?{}", grade.unwrap_or_default()))
            }
            _ => "stop".to_owned(),
        }),
        (true, _) => Some(answered_target(workflows, &workflow, &at, &chosen)),
    };
    // A note goes where someone reads it: the next model step's prompt, the next human step's
    // question, or the dropped task's reason.
    let reads_note = |target: &str| {
        target == "stop"
            || step(workflows, &workflow, target)
                .is_some_and(|next| matches!(next.decides, Decider::Model | Decider::Human))
    };
    if let (Some(_), Some(target)) = (note, target.as_deref())
        && (runs.is_some() || !reads_note(target))
    {
        return Ok(Err(format!(
            "ask {label} answered {chosen} leads to {target}, which reads no note: answer {chosen} alone"
        )));
    }
    if let Some(runs) = &runs
        && !answer.run
    {
        return Ok(Err(format!(
            "ask {label} runs {} in {} (target {}), bound to revision {}, when answered {:?}: answer again with --run to run it",
            runs.shown(),
            runs.dir.display(),
            runs.target,
            answer.revision,
            answer.text
        )));
    }
    if kind == "step"
        && let Some(task) = task
    {
        let target = answered_target(workflows, &workflow, &at, &answer.text);
        let go = step(workflows, &workflow, &target)
            .is_some_and(|starts| starts.action.as_deref() == Some("start-batch"));
        if let Err(refused) = crate::batch::answering(tx, task, go, &answer.by)? {
            return Ok(Err(refused));
        }
    }
    tx.execute(
        &format!("UPDATE asks SET answer = ?2, answered = {NOW}, answered_by = ?3 WHERE id = ?1"),
        params![answer.ask, answer.text, answer.by],
    )?;
    drop_lead(tx, answer.ask, &format!("ask {label} was answered first"))?;
    let Some(task) = task else {
        crate::deliver::queue(tx, answer.ask, Some(&answer.text), trace.factory)?;
        if command.is_some() {
            return Ok(Ok(format!(
                "ask {label} answered; its command does not run: Priory runs only the commands of its own runs' asks"
            )));
        }
        return Ok(Ok(format!("ask {label} answered")));
    };
    if let Some(runs) = runs {
        if kind == "step" {
            let target = answered_target(workflows, &workflow, &at, &answer.text);
            approve(
                tx,
                task,
                answer,
                &approvals(workflows, &workflow, &at, &target),
                head,
            )?;
        }
        withdraw_waits(tx, answer.ask, "answered-elsewhere")?;
        tx.execute(
            "INSERT INTO actions (task, step, action, status, ask, revision, command, generation)
             VALUES (?1, ?2, 'answer', 'running', ?3, ?4, ?5,
               (SELECT generation FROM tasks WHERE id = ?1))",
            params![
                task,
                at,
                answer.ask,
                answer.revision,
                serde_json::to_string(&runs).expect("a command serializes")
            ],
        )?;
        // A running task keeps the sweep from removing the worktree the command runs in.
        tx.execute(
            "UPDATE tasks SET state = 'running' WHERE id = ?1 AND state = 'asleep'",
            [task],
        )?;
        return Ok(Ok(format!(
            "ask {label} answered: running {} in {}",
            runs.shown(),
            runs.dir.display()
        )));
    }
    if let Some(hold) = hold {
        crate::hold::answered(tx, hold, &answer.text)?;
    } else if let Some(target) = target {
        if !asked.is_some_and(|asked| asked.decides == Decider::Model) {
            approve(
                tx,
                task,
                answer,
                &approvals(workflows, &workflow, &at, &target),
                head,
            )?;
            trace::ended(tx, trace, task, &chosen, None)?;
            crate::grades::answered(tx, trace, task, &at, &chosen, &target)?;
        }
        let note = note
            .map(|note| format!("A person answered {chosen} on step {at}, with a note: {note}"));
        let next = step(workflows, &workflow, &target).map(|next| &next.decides);
        if let (Some(note), Some(Decider::Model)) = (&note, next) {
            queue_first(tx, task, note)?;
        }
        let context = note.as_deref().filter(|_| next == Some(&Decider::Human));
        enter(
            tx,
            task,
            &workflow,
            &target,
            None,
            context.unwrap_or_default(),
            trace.factory,
        )?;
        if let (Some(note), "stop") = (&note, target.as_str()) {
            tx.execute(
                "UPDATE tasks SET reason = ?2 WHERE id = ?1",
                params![task, note],
            )?;
        }
    } else {
        withdraw_waits(tx, answer.ask, "answered-elsewhere")?;
        wake(
            tx,
            task,
            &format!(
                "Answer to ask {label} (revision {}): {}",
                answer.revision, answer.text
            ),
        )?;
        // An answer that beats the run's Stop leaves it running: the answer is its next prompt.
        if state == "asleep" {
            set(tx, task, "ready", None, None)?;
        }
    }
    Ok(Ok(format!("ask {label} answered")))
}

/// Where answer `answer` to the ask of step `at` leads.
fn answered_target(workflows: &Workflows, workflow: &str, at: &str, answer: &str) -> String {
    step(workflows, workflow, at)
        .and_then(|step| step.next.get(answer))
        .cloned()
        .unwrap_or_else(|| format!("{at}?{answer}"))
}

/// The irreversible steps that step `at` approves and `target` leads toward.
fn approvals<'a>(
    workflows: &'a Workflows,
    workflow: &str,
    at: &str,
    target: &str,
) -> Vec<&'a String> {
    workflows
        .get(workflow)
        .and_then(|flow| Some((&flow.value, flow.value.step(at)?)))
        .into_iter()
        .flat_map(|(flow, asked)| {
            asked
                .approves
                .iter()
                .filter(|approved| flow.reaches(target, approved))
        })
        .collect()
}

/// The command `step`'s ask stores as JSON, its target resolved to a folder as an agent's ask
/// command is, or why it cannot run for the task.
fn step_command(
    tx: &Transaction,
    task: i64,
    step: &Step,
    factory: &Factory,
) -> rusqlite::Result<Result<Option<String>, String>> {
    let Some(command) = &step.command else {
        return Ok(Ok(None));
    };
    let worktree: Option<String> =
        tx.query_row("SELECT worktree FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    let options: Vec<&str> = step.options.iter().map(String::as_str).collect();
    let asked = serde_json::to_value(command).expect("a step command serializes");
    Ok(
        command::parse(&asked, &options, "", worktree.as_deref(), factory)
            .map(|parsed| Some(serde_json::to_string(&parsed).expect("a command serializes"))),
    )
}

/// An open ask, as `open_ask` finds it. An outside ask has no task, and an empty step, workflow
/// and state.
struct OpenAsk {
    label: String,
    task: Option<i64>,
    kind: String,
    step: String,
    options: String,
    workflow: String,
    state: String,
    command: Option<String>,
}

/// The ask `id` if a person can still answer or dismiss it at `revision`, else why not.
fn open_ask(
    tx: &Transaction,
    id: i64,
    revision: Option<i64>,
) -> rusqlite::Result<Result<OpenAsk, String>> {
    let found = tx
        .query_row(
            "SELECT coalesce(asks.name, CAST(asks.id AS TEXT)), asks.task, asks.kind, coalesce(asks.step, ''),
               asks.revision, asks.options, asks.answer, coalesce(tasks.workflow, ''),
               coalesce(tasks.state, ''), asks.withdrawn_reason, asks.command
             FROM asks LEFT JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
            [id],
            |row| {
                Ok((
                    OpenAsk {
                        label: row.get(0)?,
                        task: row.get(1)?,
                        kind: row.get(2)?,
                        step: row.get(3)?,
                        options: row.get(5)?,
                        workflow: row.get(7)?,
                        state: row.get(8)?,
                        command: row.get(10)?,
                    },
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((ask, at, answered, withdrawn_reason)) = found else {
        return Ok(Err(format!("there is no ask {id}")));
    };
    let (label, task) = (&ask.label, ask.task.unwrap_or_default());
    let refused = match withdrawn_reason.as_deref() {
        Some("dismissed") => Some(format!("ask {label} was dismissed")),
        Some("sibling-fired") => Some(format!(
            "ask {label} was withdrawn when a wait of its run fired first"
        )),
        Some("moved-on") => Some(format!(
            "ask {label} was withdrawn when a later run of task {task} called another tool"
        )),
        Some("revived") => Some(format!(
            "ask {label} was withdrawn when task {task} was revived"
        )),
        Some(reason) => Some(format!("ask {label} was withdrawn: {reason}")),
        None if ["done", "failed", "dropped"].contains(&ask.state.as_str()) => {
            Some(format!("task {task} is {}", ask.state))
        }
        None => answered.map(|answered| format!("ask {label} is already answered: {answered}")),
    };
    if let Some(refused) = refused {
        return Ok(Err(refused));
    }
    if let Some(revision) = revision.filter(|revision| *revision != at) {
        return Ok(Err(format!(
            "ask {label} is at revision {at}, not {revision}"
        )));
    }
    Ok(Ok(ask))
}

/// A person's `priory dismiss`: it closes an open ask with no answer, at `revision` if given.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Dismiss {
    pub dismiss: i64,
    pub revision: Option<i64>,
}

/// Closes an ask with no answer. An agent's asleep task wakes with a line that says so; a step's
/// ask, which only its options move on, is refused.
pub(crate) fn dismiss(
    db: &mut Connection,
    dismiss: &Dismiss,
    factory: &priory_factory::factory::Factory,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ask = match open_ask(&tx, dismiss.dismiss, dismiss.revision)? {
        Ok(ask) => ask,
        Err(error) => return Ok(Err(error)),
    };
    let label = &ask.label;
    if ask.kind == "step" || crate::hold::of_ask(&tx, dismiss.dismiss)?.is_some() {
        return Ok(Err(format!(
            "ask {label} moves its task on by its options: answer it instead"
        )));
    }
    tx.execute(
        "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'dismissed' WHERE id = ?1",
        [dismiss.dismiss],
    )?;
    withdraw_waits(&tx, dismiss.dismiss, "dismissed")?;
    drop_lead(&tx, dismiss.dismiss, &format!("ask {label} was dismissed"))?;
    crate::deliver::queue(&tx, dismiss.dismiss, None, factory)?;
    if let Some(task) = ask.task {
        wake(
            &tx,
            task,
            &format!("A person dismissed ask {label} without an answer. Continue your task."),
        )?;
        if ask.state == "asleep" {
            set(&tx, task, "ready", None, None)?;
        }
    }
    tx.commit()?;
    Ok(Ok(format!("ask {label} dismissed")))
}

/// A person's desk action on a task, as `priory resume`, `cancel`, `revive`, `hold` or `release`
/// sends it on the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Act {
    pub act: String,
    pub task: i64,
    /// The step `revive` restarts at, instead of the one the task ended on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<priory_factory::factory::Hold>,
    /// What `say` tells the task's agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The state revision a person's `hold`, `release` or `cancel` was made on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<i64>,
}

/// Applies a desk action. `resume` makes a blocked task `ready`, with a prompt that says why it
/// stopped. `cancel` drops an active task and ends its live run. `revive` restarts a failed or
/// dropped task as its next generation. `hold` and `release` hold a queued or ready task and let it
/// go. Returns the reply and a tmux session to kill.
pub(crate) fn act(
    db: &mut Connection,
    act: &Act,
    factory: &priory_factory::factory::Factory,
) -> rusqlite::Result<(Result<String, String>, Option<String>)> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Err(stale) = crate::state::expect_revision(&tx, act.expected) {
        return Ok((Err(stale), None));
    }
    let held = match (act.act.as_str(), &act.hold) {
        ("hold", Some(hold)) => Some(crate::hold::hold(&tx, act.task, hold, factory)?),
        ("release", _) => Some(crate::hold::release_task(&tx, act.task)?),
        _ => None,
    };
    if let Some(reply) = held {
        if reply.is_ok() {
            tx.commit()?;
        }
        return Ok((reply, None));
    }
    let Some((state, reason)) = tx
        .query_row(
            "SELECT state, reason FROM tasks WHERE id = ?1",
            [act.task],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
    else {
        return Ok((Err(format!("there is no task {}", act.task)), None));
    };
    match act.act.as_str() {
        "resume" if state == "blocked" => {
            // No run ever took its step prompt: it starts as new.
            let unstarted: bool = tx.query_row(
                "SELECT NOT EXISTS (SELECT 1 FROM runs WHERE task = ?1)
                 AND NOT EXISTS (SELECT 1 FROM outbox WHERE task = ?1 AND state = 'queued')",
                [act.task],
                |row| row.get(0),
            )?;
            if unstarted {
                set(&tx, act.task, "queued", None, None)?;
                tx.commit()?;
                return Ok((Ok(format!("task {} is queued", act.task)), None));
            }
            // A reason from a run that never started describes the launch, not the agent's work.
            let started: bool = tx.query_row(
                "SELECT coalesce((SELECT ready FROM runs WHERE task = ?1 ORDER BY id DESC LIMIT 1), 0)",
                [act.task],
                |row| row.get(0),
            )?;
            let text = match reason.as_deref().filter(|_| started) {
                Some(reason) => format!("{RESUMED} after: {reason}. {CONTINUE}"),
                None => format!("{RESUMED}. {CONTINUE}"),
            };
            unresume(&tx, act.task)?;
            wake(&tx, act.task, &text)?;
            set(&tx, act.task, "ready", None, None)?;
            tx.commit()?;
            Ok((Ok(format!("task {} is ready", act.task)), None))
        }
        "resume" => Ok((
            Err(format!("task {} is {state}, not blocked", act.task)),
            None,
        )),
        "say" => {
            let Some(text) = act.text.as_deref().filter(|text| !text.trim().is_empty()) else {
                return Ok((Err("say needs a text".to_owned()), None));
            };
            // A fixed first line keeps a text starting with `/` from running as a slash command.
            let text = format!("{SAYS}{text}");
            let said = say(&tx, act.task, &text, &state, &factory.workflows)?;
            if said.is_ok() {
                tx.commit()?;
            }
            Ok((said, None))
        }
        "cancel" if ["done", "failed", "dropped"].contains(&state.as_str()) => {
            Ok((Err(format!("task {} is already {state}", act.task)), None))
        }
        "cancel" => {
            let tmux = cancel(&tx, act.task, "cancelled from the desk")?;
            tx.commit()?;
            Ok((Ok(format!("task {} dropped", act.task)), tmux))
        }
        "revive" => {
            let revived = revive(&tx, act, factory)?;
            if revived.is_ok() {
                tx.commit()?;
            }
            Ok((revived, None))
        }
        other => Ok((Err(format!("unknown action {other}")), None)),
    }
}

/// Drops an active task with `reason`, ends its live run and its holds, and returns the run's tmux
/// session to kill once the transaction commits.
pub(crate) fn cancel(
    tx: &Transaction,
    task: i64,
    reason: &str,
) -> rusqlite::Result<Option<String>> {
    let tmux = tx
        .query_row(
            "SELECT tmux FROM runs WHERE task = ?1 AND ended IS NULL",
            [task],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    tx.execute(
        &format!(
            "UPDATE runs SET ended = {NOW}, ended_as = 'cancelled' WHERE task = ?1 AND ended IS NULL"
        ),
        [task],
    )?;
    crate::hold::close_asks(tx, task)?;
    tx.execute(
        &format!(
            "UPDATE holds SET released = {NOW}, released_as = 'cancelled' WHERE task = ?1 AND released IS NULL"
        ),
        [task],
    )?;
    set(tx, task, "dropped", None, Some(reason))?;
    Ok(tmux)
}

/// The first words of a person's `say` text in the agent's prompt.
pub(crate) const SAYS: &str = "A person says: ";

/// Queues `text` as the next prompt of the agent at the task's model step, as an answer
/// is: typed once its run is idle, or waking an asleep task into a new run that resumes the session.
pub(crate) fn say(
    tx: &Transaction,
    task: i64,
    text: &str,
    state: &str,
    workflows: &Workflows,
) -> rusqlite::Result<Result<String, String>> {
    if ["done", "failed", "dropped"].contains(&state) {
        return Ok(Err(format!("task {task} is {state}")));
    }
    let (workflow, at): (String, String) = tx.query_row(
        "SELECT workflow, step FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if step(workflows, &workflow, &at).is_none_or(|step| step.decides != Decider::Model) {
        return Ok(Err(format!(
            "task {task} is at step {at}, which no agent runs"
        )));
    }
    // A task with no run and no queued prompt still has its step's prompt to build at its first
    // run, which a queued text would skip.
    let unstarted: bool = tx.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM runs WHERE task = ?1)
         AND NOT EXISTS (SELECT 1 FROM outbox WHERE task = ?1 AND state = 'queued')",
        [task],
        |row| row.get(0),
    )?;
    if unstarted {
        return Ok(Err(format!(
            "task {task} has no run yet: say it once its agent started"
        )));
    }
    let prompt = wake(tx, task, text)?;
    tx.execute("UPDATE outbox SET wakes = 1 WHERE id = ?1", [prompt])?;
    if state == "asleep" {
        set(tx, task, "ready", None, None)?;
    }
    Ok(Ok(format!("task {task} will hear it")))
}

/// Moves a failed or dropped task back into its step, or `act.step`, as its next generation, and
/// withdraws the open asks and waits of the earlier ones so that no late answer or event reaches it.
fn revive(
    tx: &Transaction,
    act: &Act,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let workflows = &factory.workflows;
    let (key, workflow, at, state, reason, generation) = tx.query_row(
        "SELECT key, workflow, step, state, reason, generation FROM tasks WHERE id = ?1",
        [act.task],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    )?;
    if !["failed", "dropped"].contains(&state.as_str()) {
        return Ok(Err(format!(
            "task {} is {state}, not failed or dropped",
            act.task
        )));
    }
    let target = act.step.as_deref().unwrap_or(&at);
    let Some(entered) = step(workflows, &workflow, target) else {
        return Ok(Err(format!("step {target} is not in workflow {workflow}")));
    };
    let active = tx
        .query_row(
            "SELECT id FROM tasks WHERE key = ?1 AND state NOT IN ('done', 'failed', 'dropped')",
            [&key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(active) = active {
        return Ok(Err(format!("task {active} is already active on key {key}")));
    }
    let shared = tx
        .query_row(
            "SELECT owned.resource, tasks.id FROM owned JOIN owned AS mine ON mine.resource = owned.resource
             JOIN tasks ON tasks.id = owned.task
             WHERE mine.task = ?1 AND owned.task != ?1 AND tasks.state NOT IN ('done', 'failed', 'dropped')
             ORDER BY tasks.id LIMIT 1",
            [act.task],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    if let Some((resource, active)) = shared {
        return Ok(Err(format!(
            "task {active} is already active on {resource}"
        )));
    }
    let generation = generation + 1;
    regenerate(tx, act.task, generation)?;
    enter(tx, act.task, &workflow, target, None, "", factory)?;
    if entered.decides == Decider::Model {
        let after = reason.map_or(state.clone(), |reason| format!("{state}: {reason}"));
        let text = format!(
            "A person revived this task as generation {generation} after it was {after}. The asks and waits of its earlier generation are withdrawn. Check the worktree, then continue."
        );
        queue_first(tx, act.task, &text)?;
    }
    let now: String = tx.query_row("SELECT state FROM tasks WHERE id = ?1", [act.task], |row| {
        row.get(0)
    })?;
    Ok(Ok(format!(
        "task {} is {now} at {target}, generation {generation}",
        act.task
    )))
}

/// Makes `generation` the task's, withdrawing the open asks, waits and queued prompts of the
/// earlier ones so that no late answer or event reaches it.
fn regenerate(tx: &Transaction, task: i64, generation: i64) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'revived' WHERE task = ?1
         AND answer IS NULL AND withdrawn IS NULL",
        [task],
    )?;
    tx.execute(
        "UPDATE waits SET withdrawn = 1, withdrawn_reason = 'revived' WHERE task = ?1
         AND fired IS NULL AND withdrawn IS NULL",
        [task],
    )?;
    tx.execute(
        "UPDATE outbox SET state = 'withdrawn' WHERE task = ?1 AND state = 'queued'",
        [task],
    )?;
    tx.execute(
        "UPDATE tasks SET generation = ?2 WHERE id = ?1",
        [task, generation],
    )
    .map(drop)
}

/// The done task that feedback in `message` reopens: the newest that owns the pull request, issue,
/// Linear issue or Slack thread it is on, done before the feedback was written, with no close of
/// that pull request in the inbox, no merged pull request and no active task on its key.
pub(crate) fn reopenable(
    tx: &Transaction,
    message: &herdr_bus::Message,
) -> rusqlite::Result<Option<i64>> {
    let data = &message.data;
    let Some(resource) = feedback_on(&message.kind, data) else {
        return Ok(None);
    };
    // A pull request the intake read as closed may reopen nothing before its close arrives.
    if settles(&message.kind, data) || data["open"] == false {
        return Ok(None);
    }
    // Pull request feedback reaches what the task owns; a comment elsewhere, its subscriptions only.
    let owner = match FEEDBACK.contains(&message.kind.as_str()) {
        true => "owned",
        false => "subscriptions",
    };
    tx.query_row(
        &format!(
            "SELECT tasks.id FROM {owner} AS held JOIN tasks ON tasks.id = held.task
             WHERE held.resource = ?1 AND tasks.state = 'done'
             AND julianday(?2) >= julianday(tasks.changed)
             AND NOT EXISTS (SELECT 1 FROM inbox WHERE type IN (?3, ?4) AND 'github:'
               || json_extract(data, '$.repo') || '#'
               || coalesce(json_extract(data, '$.pull_request'), json_extract(data, '$.issue')) = ?1)
             AND NOT EXISTS (SELECT 1 FROM tasks AS other WHERE other.key = tasks.key
               AND other.state NOT IN ('done', 'failed', 'dropped'))
             AND NOT EXISTS (SELECT 1 FROM pull_requests WHERE task = tasks.id AND state = 'merged')
             ORDER BY tasks.id DESC LIMIT 1"
        ),
        params![resource, message.time.to_string(), CLOSED, ISSUE_CLOSED],
        |row| row.get(0),
    )
    .optional()
}

/// The report a comment in `message` posts when it lands on the own key of the newest task on that
/// key, which failed or was dropped before the comment was written, with no active task on the key:
/// nothing reopens such a task, so a person decides whether to revive it.
pub(crate) fn unheard(
    tx: &Transaction,
    message: &herdr_bus::Message,
) -> rusqlite::Result<Option<String>> {
    let data = &message.data;
    let Some(resource) = feedback_on(&message.kind, data).filter(|_| data["own"] != true) else {
        return Ok(None);
    };
    tx.query_row(
        "SELECT id, state FROM tasks WHERE key = ?1 AND state IN ('failed', 'dropped')
         AND julianday(?2) >= julianday(changed)
         AND id = (SELECT max(id) FROM tasks AS newest WHERE newest.key = ?1)",
        params![resource, message.time.to_string()],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
    )
    .optional()
    .map(|task| {
        task.map(|(task, state)| {
            format!("{resource} has a new comment; its task {task} {state}: priory revive {task}")
        })
    })
}

/// Moves the done `task` back, as its next generation, to its workflow's `reopen` step, else the
/// step that opened its pull request, else its last step, with the feedback of the inbox entry
/// `seq` as its prompt.
pub(crate) fn reopen(tx: &Transaction, trace: &Trace, task: i64, seq: i64) -> rusqlite::Result<()> {
    let workflows = &trace.factory.workflows;
    let (workflow, at, generation, kind, data): (String, String, i64, String, String) = tx
        .query_row(
            "SELECT tasks.workflow, tasks.step, tasks.generation, inbox.type, inbox.data
             FROM tasks, inbox WHERE tasks.id = ?1 AND inbox.seq = ?2",
            params![task, seq],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
    let data: Value = serde_json::from_str(&data).expect("inbox data is JSON");
    let resource = feedback_on(&kind, &data).unwrap_or_default();
    let opened: Option<String> = tx
        .query_row(
            "SELECT results.step FROM actions JOIN results ON results.run = actions.result
             WHERE actions.task = ?1 AND actions.pull_request = ?2 AND actions.status = 'opened'
             ORDER BY actions.id DESC LIMIT 1",
            params![task, resource],
            |row| row.get(0),
        )
        .optional()?;
    let named = workflows
        .get(&workflow)
        .and_then(|flow| flow.value.reopen.clone());
    let target = named.or(opened).unwrap_or(at);
    let generation = generation + 1;
    regenerate(tx, task, generation)?;
    enter(tx, task, &workflow, &target, None, "", trace.factory)?;
    let place = crate::stack::place(tx, task, &resource)?;
    let said = reopened_by(&resource, place.as_deref(), &kind, &data);
    let text = format!("Feedback reopened this done task as generation {generation}: {said}");
    wake_on(tx, task, &text, Some(seq))?;
    let pull_request = FEEDBACK
        .contains(&kind.as_str())
        .then_some(resource.as_str());
    trace::reopened(tx, trace, task, &resource, pull_request)
}

/// Ends the live run of `session`, whose tmux session was gone when the daemon started, as `lost`.
/// Its task becomes `ready` with a prompt that says so, unless the task's previous run was lost too
/// and this one stored no ask or wait: then the task blocks. A result would have settled it instead.
pub(crate) fn lost(
    db: &mut Connection,
    trace: &Trace,
    session: &str,
    tmux: &str,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = run(&tx, session)? else {
        return Ok(());
    };
    trace::account(&tx, trace, run.id, None)?;
    end(&tx, run.id, "lost")?;
    // A run that never reached SessionStart never took the prompt it was launched with.
    tx.execute(
        "UPDATE outbox SET state = 'queued', run = NULL, attempted = NULL
         WHERE run = ?1 AND state IN ('attempted', 'uncertain') AND (SELECT NOT ready FROM runs WHERE id = ?1)",
        [run.id],
    )?;
    let again = tx
        .query_row(
            "SELECT ended_as = 'lost' FROM runs WHERE task = ?1 AND id < ?2 ORDER BY id DESC LIMIT 1",
            params![run.task, run.id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false)
        && tx.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM asks WHERE run = ?1)
             AND NOT EXISTS (SELECT 1 FROM waits WHERE run = ?1)",
            [run.id],
            |row| row.get::<_, bool>(0),
        )?;
    if again {
        let reason = format!(
            "run {} was lost again before it made progress: tmux session {tmux} is gone",
            run.id
        );
        set(&tx, run.task, "blocked", None, Some(&reason))?;
    } else {
        let text = format!(
            "Priory restarted and found tmux session {tmux} gone, as after a reboot, so this run was interrupted. Check the worktree, then continue."
        );
        wake(&tx, run.task, &text)?;
        set(&tx, run.task, "ready", None, None)?;
    }
    tx.commit()
}

/// Ends the stalled run `run` as `restarted` and makes its task ready, so its next run resumes the
/// same session. Returns the tmux session to kill.
pub(crate) fn restart(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    why: &str,
) -> rusqlite::Result<String> {
    let (task, tmux): (i64, String) =
        tx.query_row("SELECT task, tmux FROM runs WHERE id = ?1", [run], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
    trace::account(tx, trace, run, None)?;
    end(tx, run, "restarted")?;
    let text = format!(
        "Priory's handler restarted this run after it stalled: {why}. Check the worktree, then continue."
    );
    wake(tx, task, &text)?;
    set(tx, task, "ready", None, None)?;
    Ok(tmux)
}

/// Queues the handler's `text` for the stalled run `run`, ahead of the task's other prompts, and
/// marks the run idle: the handler read on its screen that it waits for input.
pub(crate) fn nudge_stalled(tx: &Transaction, run: i64, text: &str) -> rusqlite::Result<()> {
    let task: i64 = tx.query_row("SELECT task FROM runs WHERE id = ?1", [run], |row| {
        row.get(0)
    })?;
    let id = queue_first(
        tx,
        task,
        &format!("Priory's handler read your screen after this run stalled: {text}"),
    )?;
    tx.execute("UPDATE outbox SET nudge = 1 WHERE id = ?1", [id])?;
    tx.execute("UPDATE runs SET idle = 1 WHERE id = ?1", [run])
        .map(drop)
}

/// What a repair found about a failed run, what it did, and whether the agent may start again.
pub(crate) struct Repair {
    pub cause: String,
    pub action: &'static str,
    pub gone: bool,
}

/// Whether `run` followed a repair of this generation and stored no result, ask or wait.
pub(crate) fn spent(db: &Connection, run: i64) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM trace WHERE type = 'factory.task.repaired'
           AND json_extract(event, '$.data.outcome') = 'ready'
           AND json_extract(event, '$.data.generation') = (SELECT generation FROM tasks
             WHERE id = (SELECT task FROM runs WHERE id = ?1))
           AND json_extract(event, '$.data.run') = (SELECT CAST(max(id) AS TEXT) FROM runs
             WHERE task = (SELECT task FROM runs WHERE id = ?1) AND id < ?1))
         AND NOT EXISTS (SELECT 1 FROM results WHERE run = ?1)
         AND NOT EXISTS (SELECT 1 FROM asks WHERE run = ?1)
         AND NOT EXISTS (SELECT 1 FROM waits WHERE run = ?1)",
        [run],
        |row| row.get(0),
    )
}

/// Fails the blocked task of `run`, then makes it ready with the cause in its prompt when `repair` allows.
pub(crate) fn fail(
    db: &mut Connection,
    trace: &Trace,
    task: i64,
    run: i64,
    reason: &str,
    repair: Option<&Repair>,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let blocked: bool = tx.query_row(
        "SELECT state = 'blocked' FROM tasks WHERE id = ?1",
        [task],
        |row| row.get(0),
    )?;
    if !blocked {
        return Ok(());
    }
    let Some(repair) = repair else {
        let reason = format!(
            "{reason}; it failed again after its repair, so Priory did not restart the agent"
        );
        set(&tx, task, "failed", None, Some(&reason))?;
        return tx.commit();
    };
    let outcome = if repair.gone { "ready" } else { "failed" };
    trace::repaired(
        &tx,
        trace,
        task,
        run,
        [&repair.cause, repair.action, outcome],
    )?;
    let found = format!("Cause: {}. Action: {}.", repair.cause, repair.action);
    if repair.gone {
        let text = format!(
            "Priory repaired this task after: {reason}. {found} Check the worktree, then continue."
        );
        wake(&tx, task, &text)?;
        set(&tx, task, "ready", None, None)?;
    } else {
        let reason =
            format!("{reason}. {found} The cause persists, so Priory did not restart the agent.");
        set(&tx, task, "failed", None, Some(&reason))?;
    }
    tx.commit()
}

/// Records `answer` as the approval of each step in `approved`, at the task's commit.
fn approve(
    tx: &Transaction,
    task: i64,
    answer: &Answer,
    approved: &[&String],
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<()> {
    if approved.is_empty() {
        return Ok(());
    }
    let (worktree, branch): (Option<String>, Option<String>) = tx.query_row(
        "SELECT worktree, branch FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let commit = worktree
        .zip(branch)
        .and_then(|(worktree, branch)| head(Path::new(&worktree), &branch));
    for step in approved {
        tx.execute(
            "INSERT INTO approvals (task, step, ask, revision, commit_sha, generation)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT generation FROM tasks WHERE id = ?1))",
            params![task, step, answer.ask, answer.revision, commit],
        )?;
    }
    Ok(())
}

/// Whether the irreversible step `at` may run at the task's commit `head`. With no commit or no
/// approval, the task blocks. With an approval given at another commit or in an earlier generation,
/// its ask gets a new revision and the task sleeps at the approving step until a person answers it
/// again.
pub(crate) fn approved(
    db: &mut Connection,
    task: i64,
    at: &Step,
    head: Option<&str>,
) -> rusqlite::Result<bool> {
    if at.effect != Effect::Irreversible || at.autonomous.is_some() {
        return Ok(true);
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(head) = head else {
        let reason = format!(
            "step {} is irreversible and the task's branch has no commit",
            at.id
        );
        set(&tx, task, "blocked", None, Some(&reason))?;
        tx.commit()?;
        return Ok(false);
    };
    let approval = tx
        .query_row(
            "SELECT approvals.ask, approvals.commit_sha, asks.step,
               approvals.generation IS tasks.generation FROM approvals
             JOIN asks ON asks.id = approvals.ask JOIN tasks ON tasks.id = approvals.task
             WHERE approvals.task = ?1 AND approvals.step = ?2 ORDER BY approvals.id DESC LIMIT 1",
            params![task, at.id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((ask, commit, approving, current)) = approval else {
        let reason = format!("step {} is irreversible and no answer approved it", at.id);
        set(&tx, task, "blocked", None, Some(&reason))?;
        tx.commit()?;
        return Ok(false);
    };
    if current && commit.as_deref() == Some(head) {
        return Ok(true);
    }
    let moved = if current {
        format!(
            "\nasked again: the branch moved from {} to {head} since this was answered",
            commit.as_deref().unwrap_or("-"),
        )
    } else {
        "\nasked again: this was answered in an earlier generation of the task, or before Priory kept generations".to_owned()
    };
    tx.execute(
        // Asked again, an ask that a revive withdrew belongs to the current generation.
        "UPDATE asks SET revision = revision + 1, answer = NULL, answered = NULL, answered_by = NULL, withdrawn = NULL,
         withdrawn_reason = NULL, question = question || ?2, text = ?3 WHERE id = ?1",
        params![ask, moved, ask_text(&tx, task)?],
    )?;
    unqueue(&tx, task, &entry_prompt(&tx, task, at)?)?;
    set(&tx, task, "asleep", Some(&approving), None)?;
    tx.commit()?;
    Ok(false)
}

/// Stores how `step` ends, or nothing when it is no model step, for the wake prompts of `task`.
pub(crate) fn set_ending(tx: &Transaction, task: i64, step: Option<&Step>) -> rusqlite::Result<()> {
    let ending = step
        .filter(|step| step.decides == Decider::Model)
        .map(ending);
    tx.execute(
        "UPDATE tasks SET ending = ?2 WHERE id = ?1",
        params![task, ending],
    )
    .map(drop)
}

/// Removes the line that ends the task's current step from its queued prompts, as before it enters
/// another step: a prompt queued for the old step, such as a person's `say`, must not name its
/// outcomes.
fn drop_ending(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    let ending: Option<String> =
        tx.query_row("SELECT ending FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    let Some(ending) = ending else {
        return Ok(());
    };
    let queued = tx
        .prepare(
            "SELECT id, text FROM outbox WHERE task = ?1 AND state = 'queued' AND compact = 0",
        )?
        .query_map([task], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, text) in queued {
        let kept: Vec<&str> = text.lines().filter(|line| *line != ending).collect();
        // A prompt always says something before the ending, so none is left empty.
        if kept.len() != text.lines().count() && !kept.is_empty() {
            tx.execute(
                "UPDATE outbox SET text = ?2 WHERE id = ?1",
                params![id, kept.join("\n")],
            )?;
        }
    }
    Ok(())
}

/// Fills `ending` for tasks stored before it existed.
pub fn backfill_endings(db: &mut Connection, workflows: &Workflows) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let tasks: Vec<(i64, String, String)> = {
        let mut statement = tx.prepare(
            "SELECT id, workflow, step FROM tasks
             WHERE ending IS NULL AND state NOT IN ('done', 'failed', 'dropped')",
        )?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (task, workflow, at) in tasks {
        set_ending(&tx, task, step(workflows, &workflow, &at))?;
    }
    tx.commit()
}

/// The line that tells a model step's agent how to end the step.
pub(crate) fn ending(step: &Step) -> String {
    let outcomes: Vec<_> = step.next.keys().map(String::as_str).collect();
    let end = format!(
        "End this step by calling the result tool of the priory MCP server with one of these outcomes: {}.",
        outcomes.join(", ")
    );
    let proved: Vec<_> = step.proof.keys().map(String::as_str).collect();
    if proved.is_empty() {
        return end;
    }
    format!(
        "{end} Before outcome {}, run `priory gate` in the worktree: Priory takes it only once the gate passed at the head commit.",
        proved.join(" or ")
    )
}

/// What a model step's run reads first: the step's prompt, then how to end the step.
pub(crate) fn step_prompt(step: &Step) -> String {
    let end = ending(step);
    match step.prompt.as_deref() {
        Some(prompt) if !prompt.is_empty() => format!("{prompt}\n{end}"),
        _ => end,
    }
}

/// Takes `prompt`, which entering a step queued first, out of the task's queued prompt, keeping the
/// events it carries after it.
fn unqueue(tx: &Transaction, task: i64, prompt: &str) -> rusqlite::Result<()> {
    let queued = tx
        .query_row(
            "SELECT id, text FROM outbox WHERE task = ?1 AND state = 'queued' ORDER BY id LIMIT 1",
            [task],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((id, text)) = queued else {
        return Ok(());
    };
    match text.strip_prefix(prompt) {
        Some("") => {
            tx.execute("UPDATE inbox SET outbox = NULL WHERE outbox = ?1", [id])?;
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
        Some(rest) if rest.starts_with('\n') => {
            tx.execute(
                "UPDATE outbox SET text = ?2 WHERE id = ?1",
                params![id, &rest[1..]],
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// What an action did: its status, a key of the step's `next`, and what it opened or why it failed.
pub(crate) struct Done {
    pub status: &'static str,
    pub pull_request: Option<String>,
    pub commit: Option<String>,
    pub error: Option<String>,
    /// The pull requests `open-pr` opened, bottom first; the last is `pull_request`.
    pub stack: Vec<Stacked>,
}

/// One pull request of a task's stack.
pub(crate) struct Stacked {
    pub resource: String,
    pub branch: String,
    pub base: String,
}

/// Stores the `run` action of the code step `at` as `running` before its command starts, and the
/// task as `running` with it. Returns the action's id.
pub(crate) fn launched(db: &mut Connection, task: i64, at: &Step) -> rusqlite::Result<i64> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO actions (task, step, action, status, result, inbox)
         VALUES (?1, ?2, 'run', 'running',
           (SELECT max(run) FROM results WHERE task = ?1), (SELECT max(seq) FROM inbox))",
        params![task, at.id],
    )?;
    let id = tx.last_insert_rowid();
    set(&tx, task, "running", None, None)?;
    tx.commit()?;
    Ok(id)
}

/// Records how the command of `run` action `id` ended and moves its task by the outcome: the last
/// line of stdout when `next` names it and the command exited, else `ok` on exit 0 and `failed`
/// otherwise. A task the action no longer runs for, as after a cancel, stays where it is.
pub(crate) fn ran(
    db: &mut Connection,
    trace: &Trace,
    id: i64,
    status: &str,
    stdout: &str,
    stderr: &str,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let still: bool = tx.query_row(crate::state::CURRENT_COMMAND, [id], |row| row.get(0))?;
    let kind: String = tx.query_row("SELECT action FROM actions WHERE id = ?1", [id], |row| {
        row.get(0)
    })?;
    if kind == "answer" {
        answer_ran(&tx, trace, id, still, status, stdout, stderr)?;
        return tx.commit();
    }
    let (task, workflow, at): (i64, String, String) = tx.query_row(
        "SELECT task, workflow, actions.step
         FROM actions JOIN tasks ON tasks.id = actions.task WHERE actions.id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let next = step(&trace.factory.workflows, &workflow, &at).map(|step| &step.next);
    let exit = status.strip_prefix("exit ");
    let named = stdout
        .lines()
        .last()
        .map(str::trim)
        .filter(|line| exit.is_some() && next.is_some_and(|next| next.contains_key(*line)));
    let outcome = named.unwrap_or(if exit == Some("0") { "ok" } else { "failed" });
    let error = (exit != Some("0")).then_some(status);
    tx.execute(
        &format!(
            "UPDATE actions SET status = ?2, error = ?3, stdout = ?4, stderr = ?5, ended = {NOW}
             WHERE id = ?1"
        ),
        params![id, outcome, error, stdout, stderr],
    )?;
    if still {
        let target = next
            .and_then(|next| next.get(outcome).cloned())
            .unwrap_or_else(|| format!("{at}?{outcome}"));
        trace::ended(&tx, trace, task, outcome, None)?;
        enter(
            &tx,
            task,
            &workflow,
            &target,
            None,
            error.unwrap_or_default(),
            trace.factory,
        )?;
    }
    tx.commit()
}

/// The status of an answer's command that a daemon restart interrupted.
pub(crate) const UNCERTAIN: &str = "uncertain";

/// Records how the command of answer action `id` ended, `done` on exit 0, and wakes its task with
/// the exit and the tail of its output, unless the task moved on, as a cancel does. A step's ask
/// moves its task by the answer once the command is done, else asks the step again.
fn answer_ran(
    tx: &Transaction,
    trace: &Trace,
    id: i64,
    still: bool,
    status: &str,
    stdout: &str,
    stderr: &str,
) -> rusqlite::Result<()> {
    let (task, ask, revision, answer, ran): (i64, i64, i64, String, String) = tx.query_row(
        "SELECT actions.task, actions.ask, actions.revision, asks.answer, actions.command
         FROM actions JOIN asks ON asks.id = actions.ask WHERE actions.id = ?1",
        [id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let ran: command::Command = serde_json::from_str(&ran).expect("a stored command parses");
    let (outcome, error, how) = match status {
        "exit 0" => ("done", None, ": exit 0.".to_owned()),
        UNCERTAIN => (
            UNCERTAIN,
            Some("Priory stopped while it ran"),
            ", and Priory stopped while it ran, so whether it finished is uncertain. Check the target before you act on it."
                .to_owned(),
        ),
        status => ("failed", Some(status), format!(": {status}.")),
    };
    tx.execute(
        &format!(
            "UPDATE actions SET status = ?2, error = ?3, stdout = ?4, stderr = ?5, ended = {NOW}
             WHERE id = ?1"
        ),
        params![id, outcome, error, stdout, stderr],
    )?;
    // An approval holds only for the answer whose command ran.
    if outcome != "done" {
        tx.execute(
            "DELETE FROM approvals WHERE ask = ?1 AND revision = ?2",
            params![ask, revision],
        )?;
    }
    if !still {
        return Ok(());
    }
    let (kind, workflow, at): (String, String, String) = tx.query_row(
        "SELECT asks.kind, tasks.workflow, coalesce(asks.step, '')
         FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
        [ask],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if kind == "step" {
        let workflows = &trace.factory.workflows;
        if outcome == "done" {
            let target = answered_target(workflows, &workflow, &at, &answer);
            crate::grades::answered(tx, trace, task, &at, &answer, &target)?;
            trace::ended(tx, trace, task, &answer, None)?;
            enter(tx, task, &workflow, &target, None, "", trace.factory)?;
            return Ok(());
        }
        let context = format!(
            "answer {answer:?} ran {} in {}{how}",
            ran.shown(),
            ran.dir.display()
        );
        enter(tx, task, &workflow, &at, None, &context, trace.factory)?;
        return Ok(());
    }
    let answer = serde_json::to_string(&answer).expect("an answer serializes");
    wake(
        tx,
        task,
        &format!(
            "Ask {ask} was answered {answer} at revision {revision}, which ran {} in {}{how}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            ran.shown(),
            ran.dir.display()
        ),
    )?;
    tx.execute(
        "UPDATE tasks SET state = 'ready', reason = NULL WHERE id = ?1
         AND state IN ('running', 'asleep')
         AND NOT EXISTS (SELECT 1 FROM runs WHERE task = ?1 AND ended IS NULL)",
        [task],
    )
    .map(drop)
}

/// Records the run of the code step `at`'s action, makes the task own the pull request it opened,
/// and moves the task by `next[status]`.
pub(crate) fn performed(
    db: &mut Connection,
    task: i64,
    workflow: &str,
    at: &Step,
    done: Done,
    trace: &Trace,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    performed_in(&tx, task, workflow, at, done, trace)?;
    tx.commit()
}

/// As `performed`, in `tx`.
pub(crate) fn performed_in(
    tx: &Transaction,
    task: i64,
    workflow: &str,
    at: &Step,
    mut done: Done,
    trace: &Trace,
) -> rusqlite::Result<()> {
    for pull in &done.stack {
        if let Some(refused) = crate::state::taken(tx, task, &pull.resource)? {
            done.status = "failed";
            done.error = Some(refused);
            done.pull_request = None;
            done.stack.clear();
            break;
        }
    }
    tx.execute(
        "INSERT INTO actions (task, step, action, status, pull_request, commit_sha, error, result, inbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
           (SELECT max(run) FROM results WHERE task = ?1), (SELECT max(seq) FROM inbox))",
        params![
            task,
            at.id,
            at.action,
            done.status,
            done.pull_request,
            done.commit,
            done.error
        ],
    )?;
    for pull in &done.stack {
        tx.execute(
            "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
            params![task, pull.resource],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO subscriptions (task, resource) VALUES (?1, ?2)",
            params![task, pull.resource],
        )?;
        crate::stack::record(tx, task, &pull.resource, &pull.branch, &pull.base)?;
    }
    let target = at
        .next
        .get(done.status)
        .cloned()
        .unwrap_or_else(|| format!("{}?{}", at.id, done.status));
    let context = done.error.as_deref().unwrap_or_default();
    trace::ended(tx, trace, task, done.status, None)?;
    enter(tx, task, workflow, &target, None, context, trace.factory)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn risk_lines_of_any_shape_are_stored_or_refused_without_panicking(
            outcome in "(low|medium|high|\\PC{0,8})",
            lines in proptest::collection::vec(
                prop_oneof![
                    "(low|medium|high|): \\PC{0,20}",
                    "\\PC{0,30}",
                    "(low|medium|high): [a-z]{1,5}[\\t\\n\\r][a-z]{0,5}",
                ],
                0..7,
            ),
        ) {
            let risk: serde_json::Map<String, Value> = RISK_DIMENSIONS
                .iter()
                .zip(lines)
                .map(|(dimension, line)| ((*dimension).to_owned(), line.into()))
                .collect();
            if let Ok(Some(stored)) = merge_risk(&outcome, &Value::Object(risk)) {
                let stored: Value = serde_json::from_str(&stored).unwrap();
                prop_assert_eq!(stored["verdict"].as_str(), Some(outcome.as_str()));
            }
        }
    }
}
