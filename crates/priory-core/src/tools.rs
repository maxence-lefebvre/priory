use priory_factory::factory::{Decider, Loaded, Step, Workflow};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use std::collections::BTreeMap;

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
}

/// A live run with its task's workflow.
struct Run {
    id: i64,
    task: i64,
    step: String,
    workflow: String,
    tmux: String,
    session: String,
}

/// The live run of the harness session `session`.
fn run(tx: &Transaction, session: &str) -> rusqlite::Result<Option<Run>> {
    tx.query_row(
        "SELECT runs.id, runs.task, runs.step, tasks.workflow, runs.tmux, runs.session
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL AND runs.session = ?1",
        [session],
        |row| {
            Ok(Run {
                id: row.get(0)?,
                task: row.get(1)?,
                step: row.get(2)?,
                workflow: row.get(3)?,
                tmux: row.get(4)?,
                session: row.get(5)?,
            })
        },
    )
    .optional()
}

fn step<'a>(workflows: &'a Workflows, workflow: &str, id: &str) -> Option<&'a Step> {
    workflows
        .get(workflow)?
        .value
        .steps
        .iter()
        .find(|step| step.id == id)
}

fn text(arguments: &Value, name: &str) -> Result<String, String> {
    arguments[name]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn optional(arguments: &Value, name: &str) -> Result<Option<String>, String> {
    match &arguments[name] {
        Value::Null => Ok(None),
        _ => text(arguments, name).map(Some),
    }
}

/// Applies a tool call from the run whose harness session is `call.session`. The inner error is
/// the tool's error, returned to the agent with nothing stored.
pub(crate) fn call(
    db: &mut Connection,
    call: &Call,
    workflows: &Workflows,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = run(&tx, &call.session)? else {
        return Ok(Err(format!("no live run has session {}", call.session)));
    };
    if let Some(outcome) = tx
        .query_row(
            "SELECT outcome FROM results WHERE run = ?1",
            [run.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(Err(format!(
            "run {} already has a result, {outcome}; a result is final",
            run.id
        )));
    }
    if let Some(ask) = tx
        .query_row(
            "SELECT id FROM asks WHERE run = ?1 AND answer IS NULL",
            [run.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(Err(format!("run {} already waits on ask {ask}", run.id)));
    }
    let stored = match call.tool.as_str() {
        "result" => result(&tx, &run, &call.arguments, workflows)?,
        "ask" => ask(&tx, &run, &call.arguments)?,
        tool => Err(format!("unknown tool {tool}")),
    };
    if stored.is_ok() {
        tx.commit()?;
    }
    Ok(stored)
}

fn result(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
    workflows: &Workflows,
) -> rusqlite::Result<Result<String, String>> {
    let fields = (|| {
        Ok::<_, String>((
            text(arguments, "outcome")?,
            text(arguments, "summary")?,
            optional(arguments, "evidence")?,
            optional(arguments, "commit")?,
        ))
    })();
    let (outcome, summary, evidence, commit) = match fields {
        Ok(fields) => fields,
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
    tx.execute(
        "INSERT INTO results (run, task, step, outcome, summary, evidence, commit_sha)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            run.id, run.task, run.step, outcome, summary, evidence, commit
        ],
    )?;
    Ok(Ok(format!(
        "result stored: the task moves to {next} when this turn ends"
    )))
}

fn ask(tx: &Transaction, run: &Run, arguments: &Value) -> rusqlite::Result<Result<String, String>> {
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
        Ok::<_, String>((
            text(arguments, "question")?,
            serde_json::to_string(&options).expect("options serialize"),
            text(arguments, "recommendation")?,
            text(arguments, "tradeoff")?,
        ))
    })();
    let (question, options, recommendation, tradeoff) = match fields {
        Ok(fields) => fields,
        Err(error) => return Ok(Err(error)),
    };
    tx.execute(
        "INSERT INTO asks (task, run, step, kind, question, options, recommendation, tradeoff)
         VALUES (?1, ?2, ?3, 'agent', ?4, ?5, ?6, ?7)",
        params![
            run.task,
            run.id,
            run.step,
            question,
            options,
            recommendation,
            tradeoff
        ],
    )?;
    Ok(Ok(format!(
        "ask {} stored: end your turn now; the answer arrives as your next prompt",
        tx.last_insert_rowid()
    )))
}

/// After a run's turn ends: moves its task by its result, or puts it to sleep on its open ask.
/// Returns the tmux session to kill once the hook has its reply.
pub(crate) fn stop(
    tx: &Transaction,
    session: &str,
    workflows: &Workflows,
) -> rusqlite::Result<Option<String>> {
    let Some(run) = run(tx, session)? else {
        return Ok(None);
    };
    let outcome = tx
        .query_row(
            "SELECT outcome, summary FROM results WHERE run = ?1",
            [run.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((outcome, summary)) = outcome {
        let target = step(workflows, &run.workflow, &run.step)
            .and_then(|step| step.next.get(&outcome))
            .cloned()
            .unwrap_or_else(|| format!("{}?{outcome}", run.step));
        return enter(
            tx,
            run.task,
            &run.workflow,
            &target,
            Some(&run),
            &summary,
            workflows,
        );
    }
    let asked = tx
        .query_row(
            "SELECT 1 FROM asks WHERE run = ?1 AND answer IS NULL",
            [run.id],
            |_| Ok(()),
        )
        .optional()?;
    if asked.is_none() {
        return Ok(None);
    }
    end(tx, run.id, "asleep")?;
    set(tx, run.task, "asleep", None, None)?;
    Ok(Some(run.tmux))
}

/// Queues `text` as the task's next prompt, ahead of the prompts already queued, which it carries
/// after it: an event kept during the last turn must not reach the agent before its new instructions.
fn queue_first(tx: &Transaction, task: i64, text: &str) -> rusqlite::Result<()> {
    let mut merged = text.to_owned();
    let mut queued = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT id, text FROM outbox WHERE task = ?1 AND state = 'queued' ORDER BY id",
        )?;
        let mut rows = statement.query([task])?;
        while let Some(row) = rows.next()? {
            queued.push(row.get::<_, i64>(0)?);
            merged.push('\n');
            merged.push_str(&row.get::<_, String>(1)?);
        }
    }
    tx.execute(
        "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
        params![task, merged],
    )?;
    let id = tx.last_insert_rowid();
    for old in queued {
        tx.execute("UPDATE inbox SET outbox = ?1 WHERE outbox = ?2", [id, old])?;
        tx.execute("DELETE FROM outbox WHERE id = ?1", [old])?;
    }
    Ok(())
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
        "UPDATE tasks SET state = ?2, step = coalesce(?3, step), reason = ?4 WHERE id = ?1",
        params![task, state, step, reason],
    )
    .map(drop)
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
    workflows: &Workflows,
) -> rusqlite::Result<Option<String>> {
    let finish = |ended_as: &str| -> rusqlite::Result<Option<String>> {
        let Some(run) = from else {
            return Ok(None);
        };
        end(tx, run.id, ended_as)?;
        Ok(Some(run.tmux.clone()))
    };
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
    match step.decides {
        Decider::Model => {
            queue_first(tx, task, step.prompt.as_deref().unwrap_or_default())?;
            let Some(run) = from else {
                set(tx, task, "ready", Some(target), None)?;
                return Ok(None);
            };
            end(tx, run.id, "moved")?;
            tx.execute(
                "INSERT INTO runs (task, step, tmux, session, idle, ready) VALUES (?1, ?2, ?3, ?4, 1, 1)",
                params![task, target, run.tmux, run.session],
            )?;
            set(tx, task, "running", Some(target), None)?;
            Ok(None)
        }
        Decider::Human => {
            let question = if context.is_empty() {
                target.to_owned()
            } else {
                format!("{target}: {context}")
            };
            tx.execute(
                "INSERT INTO asks (task, step, kind, question, options) VALUES (?1, ?2, 'step', ?3, ?4)",
                params![
                    task,
                    target,
                    question,
                    serde_json::to_string(&step.options).expect("options serialize")
                ],
            )?;
            set(tx, task, "asleep", Some(target), None)?;
            finish("asleep")
        }
        Decider::Code => {
            set(
                tx,
                task,
                "blocked",
                Some(target),
                Some("code steps are not built yet"),
            )?;
            finish("blocked")
        }
    }
}

/// Applies a person's answer. The first answer to an ask's current revision stands. A step's ask
/// moves the task by the chosen option; an agent's ask queues the answer as its next prompt.
pub(crate) fn answer(
    db: &mut Connection,
    answer: &Answer,
    workflows: &Workflows,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ask = tx
        .query_row(
            "SELECT asks.task, asks.kind, asks.step, asks.revision, asks.options, asks.answer,
             tasks.workflow, tasks.state
             FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
            [answer.ask],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((task, kind, at, revision, options, answered, workflow, state)) = ask else {
        return Ok(Err(format!("there is no ask {}", answer.ask)));
    };
    if let Some(answered) = answered {
        return Ok(Err(format!(
            "ask {} is already answered: {answered}",
            answer.ask
        )));
    }
    if answer.revision != revision {
        return Ok(Err(format!(
            "ask {} is at revision {revision}, not {}",
            answer.ask, answer.revision
        )));
    }
    let options: Vec<String> = serde_json::from_str(&options).expect("options are stored as JSON");
    if kind == "step" && !options.contains(&answer.text) {
        return Ok(Err(format!(
            "ask {} takes one of: {}",
            answer.ask,
            options.join(", ")
        )));
    }
    tx.execute(
        &format!("UPDATE asks SET answer = ?2, answered = {NOW} WHERE id = ?1"),
        params![answer.ask, answer.text],
    )?;
    if kind == "step" {
        let target = step(workflows, &workflow, &at)
            .and_then(|step| step.next.get(&answer.text))
            .cloned()
            .unwrap_or_else(|| format!("{at}?{}", answer.text));
        enter(&tx, task, &workflow, &target, None, "", workflows)?;
    } else {
        queue_first(&tx, task, &answer.text)?;
        // An answer that beats the run's Stop leaves it running: the answer is its next prompt.
        if state == "asleep" {
            set(&tx, task, "ready", None, None)?;
        }
    }
    tx.commit()?;
    Ok(Ok(format!("ask {} answered", answer.ask)))
}
