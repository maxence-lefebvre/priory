use priory_factory::factory::{Confidence, Decider, Factory};
use priory_runtime::transcript::Usage;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::path::Path;

/// What every step record names besides the step: the factory, its revision and the machine. With
/// `usage`, which reads a harness transcript from a byte offset.
pub struct Trace<'a> {
    pub factory: &'a Factory,
    pub revision: Option<String>,
    pub machine: Option<String>,
    /// This daemon's runner, when the factory has several and so they own tasks through the log.
    pub runner: Option<String>,
    pub usage: fn(&Path, u64) -> Option<Usage>,
}

/// Records the run's transcript at its `SessionStart`. What it holds already belongs to earlier runs
/// of the session, so the run's turns start at its end. Returns the session's context size, if
/// its transcript holds a message.
pub(crate) fn begin(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    transcript: Option<&str>,
) -> rusqlite::Result<Option<u64>> {
    let Some(transcript) = transcript else {
        return Ok(None);
    };
    let Some(usage) = (trace.usage)(Path::new(transcript), 0) else {
        return Ok(None);
    };
    tx.execute(
        "UPDATE runs SET transcript = ?2, read = ?3 WHERE id = ?1",
        params![run, transcript, usage.end as i64],
    )?;
    Ok(usage.context)
}

/// Adds the tokens of the run's turns since its last read to its task's current step. A run with no
/// transcript recorded, as one live across the upgrade that added it, only starts reading there,
/// since its transcript may hold other runs' turns. An unreadable transcript keeps the offset.
/// Returns the session's context size after the last message read, if it read one.
pub(crate) fn account(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    transcript: Option<&str>,
) -> rusqlite::Result<Option<u64>> {
    let known = tx
        .query_row(
            "SELECT transcript, read FROM runs WHERE id = ?1",
            [run],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((recorded, read)) = known else {
        return Ok(None);
    };
    let Some(path) = recorded.as_deref().or(transcript) else {
        return Ok(None);
    };
    let from = if recorded.is_some() { read as u64 } else { 0 };
    let Some(usage) = (trace.usage)(Path::new(path), from) else {
        return Ok(None);
    };
    tx.execute(
        "UPDATE runs SET transcript = ?2, read = ?3 WHERE id = ?1",
        params![run, path, usage.end as i64],
    )?;
    if recorded.is_none() || usage.messages == 0 {
        return Ok(usage.context);
    }
    tx.execute(
        "UPDATE tasks SET model = coalesce(?2, model), tokens = json_object(
           'input', coalesce(json_extract(tokens, '$.input'), 0) + ?3,
           'output', coalesce(json_extract(tokens, '$.output'), 0) + ?4,
           'cache_read', coalesce(json_extract(tokens, '$.cache_read'), 0) + ?5,
           'cache_write', coalesce(json_extract(tokens, '$.cache_write'), 0) + ?6)
         WHERE id = (SELECT task FROM runs WHERE id = ?1)",
        params![
            run,
            usage.model,
            usage.input as i64,
            usage.output as i64,
            usage.cache_read as i64,
            usage.cache_write as i64
        ],
    )?;
    Ok(usage.context)
}

/// The dollars `tokens`, a step's token counts by kind, cost at `price`.
pub(crate) fn priced(tokens: &Value, price: &priory_factory::factory::Price) -> f64 {
    let count = |field: &str| tokens[field].as_u64().unwrap_or(0) as f64;
    (count("input") * price.input
        + count("output") * price.output
        + count("cache_read") * price.cache_read
        + count("cache_write") * price.cache_write)
        / 1_000_000.0
}

/// Appends a `factory.step.ended` record, as `docs/trace-schema.md` defines it, for the step `task`
/// is at, which `outcome` ends. `run` is the run whose result ended a model step. A field Priory
/// does not know stays absent.
pub(crate) fn ended(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    outcome: &str,
    run: Option<i64>,
) -> rusqlite::Result<()> {
    ended_with(tx, trace, task, outcome, run, None, None)
}

/// As `ended`, naming `decider` in place of the step's own, and with a debate step's `debate`
/// field. A debate's record carries no tokens, harness or model: each turn's own record does.
pub(crate) fn ended_with(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    outcome: &str,
    run: Option<i64>,
    decider: Option<&str>,
    debate: Option<Value>,
) -> rusqlite::Result<()> {
    let ended = tx.query_row(
        "SELECT workflow, step, key, entered, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
         CAST(round((julianday('now') - julianday(entered)) * 86400000) AS INTEGER), tokens, model, generation,
         lane
         FROM tasks WHERE id = ?1",
        [task],
        |row| {
            Ok(Ended {
                workflow: row.get(0)?,
                step: row.get(1)?,
                key: row.get(2)?,
                started: row.get(3)?,
                time: row.get(4)?,
                duration: row.get(5)?,
                tokens: row.get(6)?,
                model: row.get(7)?,
                generation: row.get(8)?,
                lane: row.get(9)?,
            })
        },
    )?;
    let Some(step) = trace
        .factory
        .workflows
        .get(&ended.workflow)
        .and_then(|flow| flow.value.step(&ended.step))
    else {
        return Ok(());
    };
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": ended.workflow,
        "task": task.to_string(),
        "key": ended.key,
        "run": run.map(|run| run.to_string()),
        "generation": ended.generation,
        "step": ended.step,
        "decider": decider.unwrap_or(step.decides.name()),
        "effect": step.effect,
        "outcome": outcome,
        "machine": trace.machine,
        "started": ended.started,
        "duration_ms": ended.duration.max(0),
    });
    if let Some(lane) = ended.lane {
        data["lane"] = json!(lane);
    }
    if let Some(debate) = debate {
        data["debate"] = debate;
    }
    if let Some((consult, asker)) = crate::consult::of_investigation(tx, task)? {
        data["consult"] = json!({ "id": consult, "task": asker.to_string() });
    }
    let agent = step
        .agent
        .as_ref()
        .and_then(|name| trace.factory.agents.get(name))
        .filter(|_| step.decides == Decider::Model && step.debate.is_none());
    if agent.is_some() && crate::tools::GRADES.contains(&outcome) {
        data["grade"] = json!(outcome);
    }
    if let Some(agent) = agent {
        if let Some(run) = run {
            let (hash, commit, memories, sandbox, start) = tx
                .query_row(
                    "SELECT kit_hash, kit_commit, memories_commit, sandbox,
                       CAST(round((julianday(session_started) - julianday(started)) * 86400000)
                         AS INTEGER)
                     FROM runs WHERE id = ?1",
                    [run],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                        ))
                    },
                )
                .optional()?
                .unwrap_or_default();
            if let Some(sandbox) = sandbox {
                data["sandbox"] = Value::from(sandbox);
            }
            if let Some(start) = start {
                data["session_start_ms"] = Value::from(start.max(0));
            }
            data["kit_hash"] = hash.map_or(Value::Null, Value::from);
            if let Some(commit) = commit {
                data["kit_commit"] = Value::from(commit);
            }
            if let Some(commit) = memories {
                data["memories_commit"] = Value::from(commit);
            }
        }
        data["harness"] = json!(agent.value.harness);
        data["model"] = json!(ended.model.or_else(|| agent.value.model.clone()));
        data["tokens"] = ended
            .tokens
            .and_then(|tokens| serde_json::from_str(&tokens).ok())
            .unwrap_or(Value::Null);
        let price = data["model"]
            .as_str()
            .and_then(|model| trace.factory.prices.get(model));
        if let (Some(budget), Some(price)) = (step.budget(), price)
            && data["tokens"].is_object()
        {
            let spent = priced(&data["tokens"], price);
            if spent > budget {
                data["overrun"] = json!({ "budget_usd": budget, "spent_usd": spent });
            }
        }
    }
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.step.ended", &ended.time, data)
}

/// The type of the record each debate turn appends.
pub const DEBATE_TURN: &str = "factory.debate.turn_ended";

/// Appends a `factory.debate.turn_ended` record for the turn `run` took in `debate`, with the
/// tokens read since the last one, priced at its own agent's model, and clears them from the task,
/// so the step's record never sums two models.
pub(crate) fn turned(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    debate: i64,
) -> rusqlite::Result<()> {
    let turn = tx.query_row(
        "SELECT tasks.id, tasks.workflow, runs.step, tasks.key, tasks.generation, tasks.lane,
         tasks.tokens, tasks.model, runs.agent, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
         (SELECT verdict FROM positions WHERE positions.run = runs.id),
         (SELECT count(*) FROM runs AS earlier WHERE earlier.debate = ?2 AND earlier.id < runs.id)
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1",
        params![run, debate],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, i64>(11)?,
            ))
        },
    )?;
    let (task, workflow, step, key, generation, lane, tokens, model, agent, time, verdict, earlier) =
        turn;
    let Some(tokens) = tokens else {
        return Ok(());
    };
    let defined = agent
        .as_ref()
        .and_then(|agent| trace.factory.agents.get(agent));
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "lane": lane,
        "run": run.to_string(),
        "generation": generation,
        "step": step,
        "debate": debate.to_string(),
        "turn": earlier + 1,
        "agent": agent,
        "verdict": verdict,
        "harness": defined.map(|agent| json!(agent.value.harness)),
        "model": model.or_else(|| defined.and_then(|agent| agent.value.model.clone())),
        "machine": trace.machine,
        "tokens": serde_json::from_str::<Value>(&tokens).ok(),
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, DEBATE_TURN, &time, data)?;
    tx.execute(
        "UPDATE tasks SET tokens = NULL, model = NULL WHERE id = ?1",
        [task],
    )
    .map(drop)
}

/// What a handler call of a stalled run answered and cost.
pub(crate) struct Handled<'a> {
    pub run: i64,
    pub action: &'a str,
    pub text: &'a str,
    pub model: &'a str,
    pub tokens: Option<Value>,
    pub cost_usd: Option<f64>,
}

/// Appends a `factory.handler.called` record for a handler call on `handled.run`.
pub(crate) fn handled(tx: &Transaction, trace: &Trace, handled: &Handled) -> rusqlite::Result<()> {
    let (task, workflow, step, key, generation, time): (i64, String, String, String, i64, String) =
        tx.query_row(
            "SELECT tasks.id, tasks.workflow, runs.step, tasks.key, tasks.generation,
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1",
            [handled.run],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "run": handled.run.to_string(),
        "generation": generation,
        "step": step,
        "action": handled.action,
        "text": handled.text,
        "harness": "claude-code",
        "model": handled.model,
        "machine": trace.machine,
        "tokens": handled.tokens,
        "cost_usd": handled.cost_usd,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.handler.called", &time, data)
}

/// Appends a `factory.summarizer.called` record for the summarizer call that wrote, or failed to
/// write, `task`'s TL;DR.
pub(crate) fn summarized(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    status: &str,
    model: &str,
    usage: crate::handler::Usage,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "status": status,
        "harness": "claude-code",
        "model": usage.model.as_deref().unwrap_or(model),
        "machine": trace.machine,
        "tokens": usage.tokens,
        "cost_usd": usage.cost_usd,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.summarizer.called", &time, data)
}

/// Whether Priory recorded a harness's trust for `run` before it started: each of its paths granted
/// or present. direnv's grants do not count.
pub(crate) fn granted(db: &Connection, run: i64) -> bool {
    db.query_row(
        "SELECT coalesce(min(json_extract(event, '$.data.outcome') IN ('granted', 'present')), 0)
         FROM trace WHERE type = 'factory.run.trusted'
           AND json_extract(event, '$.data.run') = ?1
           AND json_extract(event, '$.data.harness') <> 'direnv'",
        [run.to_string()],
        |row| row.get(0),
    )
    .unwrap_or(false)
}

/// Appends a `factory.run.trusted` record: what Priory did about `harness`'s trust in one path
/// for `run`, with the harness's `version` when it asked anyway.
pub(crate) fn trusted(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    harness: &str,
    grant: &priory_runtime::trust::Grant,
    version: Option<&str>,
) -> rusqlite::Result<()> {
    let (task, key, time): (i64, String, String) = tx.query_row(
        "SELECT tasks.id, tasks.key, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1",
        [run],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let file = (!grant.file.as_os_str().is_empty()).then(|| grant.file.display().to_string());
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "task": task.to_string(),
        "key": key,
        "run": run.to_string(),
        "machine": trace.machine,
        "harness": harness,
        "path": grant.path.display().to_string(),
        "file": file,
        "outcome": grant.outcome.as_str(),
        "why": grant.why,
        "version": version,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.run.trusted", &time, data)
}

/// Appends a `factory.run.attach_failed` record: the factory's `[runs] attach` command did not open
/// run `run`, for `reason`, after printing `output`.
pub(crate) fn attach_failed(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    reason: &str,
    output: &str,
) -> rusqlite::Result<()> {
    let (task, key, step, time): (i64, String, String, String) = tx.query_row(
        "SELECT tasks.id, tasks.key, runs.step, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1",
        [run],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "task": task.to_string(),
        "key": key,
        "run": run.to_string(),
        "step": step,
        "machine": trace.machine,
        "reason": reason,
    });
    if !output.is_empty() {
        let output = crate::screen::plain(output);
        let secrets = crate::screen::secrets(&trace.factory.secret_envs());
        data["output"] = json!(
            crate::screen::redact(&output, &secrets, None)
                .map_or_else(|| "[redacted]".to_owned(), |(shown, _)| shown)
        );
    }
    append(tx, trace, "factory.run.attach_failed", &time, data)
}

/// Appends a `factory.run.preempted` record: `run` ended at its turn end to make room for the
/// head task `head`, in band `head_band`, `waited` milliseconds after it was marked.
pub(crate) fn preempted(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    head: i64,
    head_band: &str,
    waited: Option<i64>,
) -> rusqlite::Result<()> {
    let (task, workflow, step, key, generation, band, time): (
        i64,
        String,
        String,
        String,
        i64,
        Option<String>,
        String,
    ) = tx.query_row(
        "SELECT tasks.id, tasks.workflow, runs.step, tasks.key, tasks.generation, tasks.band,
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1",
        [run],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    let band = band.or_else(|| trace.factory.bands().last().map(|band| (*band).to_owned()));
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "run": run.to_string(),
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "band": band,
        "head_task": head.to_string(),
        "head_band": head_band,
        "waited_ms": waited,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.run.preempted", &time, data)
}

/// Appends a `factory.ask.auto_answered` record: the daemon answered ask `ask` of `run`, at its
/// `revision`, with its recommended `option`, since its stated confidence met its task's level.
#[allow(clippy::too_many_arguments)]
pub(crate) fn auto_answered(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    ask: i64,
    revision: i64,
    option: &str,
    [confidence, level]: [Confidence; 2],
    rationale: &str,
) -> rusqlite::Result<()> {
    let (task, workflow, step, key, generation, time): (i64, String, String, String, i64, String) =
        tx.query_row(
            "SELECT tasks.id, tasks.workflow, asks.step, tasks.key, tasks.generation,
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
            [ask],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "run": run.to_string(),
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "ask": ask.to_string(),
        "revision": revision,
        "option": option,
        "confidence": confidence.as_str(),
        "rationale": rationale,
        "level": level.as_str(),
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.ask.auto_answered", &time, data)
}

/// Appends a record of type `kind` about autopilot's answer to ask `ask` of a task, with `fields`:
/// `factory.ask.autopilot_answered` when the daemon gave it, and `factory.ask.autopilot_replaced`
/// when a person's answer took its place.
pub(crate) fn autopilot(
    tx: &Transaction,
    trace: &Trace,
    kind: &str,
    ask: i64,
    fields: serde_json::Value,
) -> rusqlite::Result<()> {
    let (task, workflow, step, key, generation, time): (i64, String, String, String, i64, String) =
        tx.query_row(
            "SELECT tasks.id, tasks.workflow, asks.step, tasks.key, tasks.generation,
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM asks JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
            [ask],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
    });
    if let (Value::Object(data), Value::Object(fields)) = (&mut data, fields) {
        data.extend(fields);
        data.retain(|_, value| !value.is_null());
    }
    append(tx, trace, kind, &time, data)
}

/// Appends a `factory.ask.session_answered` record, or `factory.ask.session_dismissed` when
/// `option` is `None`: a person's own session decided ask `ask` at `revision` through `priory mcp
/// --outside`. An outside ask has no task, so its record has no task fields.
pub(crate) fn session_decided(
    tx: &Transaction,
    trace: &Trace,
    ask: i64,
    revision: i64,
    session: &str,
    option: Option<&str>,
) -> rusqlite::Result<()> {
    type Row = (
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        String,
    );
    let (task, workflow, step, key, generation, name, time): Row = tx.query_row(
        "SELECT tasks.id, tasks.workflow, asks.step, coalesce(tasks.key, asks.key), tasks.generation,
           asks.name, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM asks LEFT JOIN tasks ON tasks.id = asks.task WHERE asks.id = ?1",
        [ask],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.map(|task| task.to_string()),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "ask": name.unwrap_or_else(|| ask.to_string()),
        "revision": revision,
        "session": session,
        "option": option,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    let kind = match option {
        Some(_) => "factory.ask.session_answered",
        None => "factory.ask.session_dismissed",
    };
    append(tx, trace, kind, &time, data)
}

/// Appends a `factory.task.repaired` record for the repair of `task` after `run` failed.
pub(crate) fn repaired(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    run: i64,
    repair: [&str; 3],
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let [cause, action, outcome] = repair;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "run": run.to_string(),
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "cause": cause,
        "action": action,
        "outcome": outcome,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.task.repaired", &time, data)
}

/// Appends a `factory.worktree.recovered` record: the start of `task` found its worktree off its
/// branch and put it back, as `recovery` says.
pub(crate) fn worktree_recovered(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    worktree: &Path,
    recovery: &priory_runtime::runtime::Recovery,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "worktree": worktree.display().to_string(),
        "head": recovery.head,
        "from_branch": recovery.from_branch,
        "branch": recovery.branch,
        "kept_branch": recovery.kept_branch,
        "kept_commits": recovery.kept_commits,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.worktree.recovered", &time, data)
}

/// Appends a `factory.worktree.reclaimed` record: the sweep deleted `bytes` of build output at
/// `path` in the worktree of idle `task`.
pub(crate) fn worktree_reclaimed(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    path: &Path,
    bytes: u64,
) -> rusqlite::Result<()> {
    let (workflow, key, time): (String, String, String) = tx.query_row(
        "SELECT workflow, key, strftime('%Y-%m-%dT%H:%M:%fZ', 'now') FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "machine": trace.machine,
        "path": path.display().to_string(),
        "bytes": bytes,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.worktree.reclaimed", &time, data)
}

/// Appends a `factory.task.snoozed` record for a person's snooze of a desk item of `task` until
/// `until`, or a `factory.task.unsnoozed` one when `until` is `None`.
pub(crate) fn snoozed(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    kind: &str,
    item: i64,
    until: Option<&str>,
) -> rusqlite::Result<()> {
    let kind_of = match until {
        Some(_) => "factory.task.snoozed",
        None => "factory.task.unsnoozed",
    };
    snooze_record(
        tx,
        trace,
        kind_of,
        (task, kind, item),
        json!({ "until": until }),
    )
}

/// Appends a `factory.task.snooze_ended` record for a snooze that ended by itself: at its time,
/// as `reason` `time`, or early on a change, with the change's reason.
pub(crate) fn snooze_ended(
    tx: &Transaction,
    trace: &Trace,
    (task, kind, item): (i64, &str, i64),
    reason: &str,
) -> rusqlite::Result<()> {
    let fields = json!({ "reason": reason });
    snooze_record(
        tx,
        trace,
        "factory.task.snooze_ended",
        (task, kind, item),
        fields,
    )
}

fn snooze_record(
    tx: &Transaction,
    trace: &Trace,
    kind_of: &str,
    (task, kind, item): (i64, &str, i64),
    fields: Value,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "item": { "kind": kind, "id": item.to_string() },
    });
    if let (Value::Object(data), Value::Object(fields)) = (&mut data, fields) {
        data.extend(fields);
        data.retain(|_, value| !value.is_null());
    }
    append(tx, trace, kind_of, &time, data)
}

/// Appends a `factory.task.progressed` record with `task`'s progress after a change.
pub(crate) fn progressed(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    progress: &Value,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "progress": progress,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.task.progressed", &time, data)
}

/// Appends a `factory.task.reopened` record for `task`, which feedback on `pull_request` moved back
/// from done as its next generation.
pub(crate) fn reopened(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    resource: &str,
    pull_request: Option<&str>,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "resource": resource,
        "pull_request": pull_request,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.task.reopened", &time, data)
}

/// Appends a `factory.step.labeled` record that judges `task` with `label`, once per task, label and
/// source event since the label's last withdrawal: `happened`, when the event happened, else now,
/// and `evidence`. It names the record of the step that graded the task, when one did, and its
/// grade.
pub(crate) fn labeled(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    label: &str,
    evidence: &str,
    happened: Option<&str>,
) -> rusqlite::Result<()> {
    labeled_with(tx, trace, task, label, evidence, happened, json!({}))
}

/// Appends the `factory.step.labeled` record of a `held` scorer that graded `task`: label `scored`,
/// with the scorer, its verdict, and its reason as the evidence. `happened` is the moment its
/// `after` names.
pub(crate) fn held(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    scorer: &str,
    verdict: &str,
    reason: &str,
    happened: &str,
) -> rusqlite::Result<()> {
    let extra = json!({ "scorer": scorer, "kind": "held", "verdict": verdict });
    labeled_with(tx, trace, task, SCORED, reason, Some(happened), extra)
}

/// The label of a `held` scorer's record.
pub(crate) const SCORED: &str = "scored";

fn labeled_with(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    label: &str,
    evidence: &str,
    happened: Option<&str>,
    extra: Value,
) -> rusqlite::Result<()> {
    let (workflow, key, generation, grade, time): (String, String, i64, Option<String>, String) =
        tx.query_row(
            "SELECT workflow, key, generation, grade, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let happened = happened.unwrap_or(&time);
    let known = tx
        .query_row(
            "SELECT 1 FROM trace WHERE type = 'factory.step.labeled'
             AND json_extract(event, '$.data.task') = ?1 AND json_extract(event, '$.data.label') = ?2
             AND json_extract(event, '$.data.evidence') = ?3
             AND json_extract(event, '$.data.happened') = ?4
             AND seq > coalesce((SELECT max(seq) FROM trace WHERE type = 'factory.step.unlabeled'
               AND json_extract(event, '$.data.task') = ?1
               AND json_extract(event, '$.data.label') = ?2), 0)",
            params![task.to_string(), label, evidence, happened],
            |_| Ok(()),
        )
        .optional()?;
    if known.is_some() {
        return Ok(());
    }
    let step_event: Option<String> = tx
        .query_row(
            "SELECT id FROM trace WHERE type = 'factory.step.ended'
             AND json_extract(event, '$.data.task') = ?1
             AND json_extract(event, '$.data.grade') IS NOT NULL ORDER BY seq DESC LIMIT 1",
            [task.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "machine": trace.machine,
        "step_event": step_event,
        "grade": grade,
        "label": label,
        "evidence": evidence,
        "happened": happened,
    });
    if let (Value::Object(fields), Value::Object(extra)) = (&mut data, extra) {
        fields.extend(extra);
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.step.labeled", &time, data)
}

/// Appends a `factory.step.unlabeled` record that withdraws every `label` record of `task` before
/// it, as a person who cleared a mark wrote at `time`.
pub(crate) fn unlabeled(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    label: &str,
    time: &str,
) -> rusqlite::Result<()> {
    let (workflow, key, generation): (String, String, i64) = tx.query_row(
        "SELECT workflow, key, generation FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "machine": trace.machine,
        "label": label,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.step.unlabeled", time, data)
}

/// Appends a `factory.tool.denied` record: the tool guard denied `call` to `run` of `task`.
pub(crate) fn tool_denied(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    run: i64,
    call: &crate::guard::Call,
    reason: &str,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "run": run.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "tool": call.tool,
        "guard": call.guard,
        "call": call.call,
        "reason": reason,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.tool.denied", &time, data)
}

/// Appends a `factory.gate.ran` record: `priory gate` ran the factory's gate for `run` of `task`
/// at `commit`, and it exited `exit` after `duration_ms`.
pub(crate) fn gate_ran(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    run: i64,
    commit: &str,
    exit: i32,
    duration_ms: u64,
) -> rusqlite::Result<()> {
    let (workflow, step, key, generation, time): (String, String, String, i64, String) = tx
        .query_row(
            "SELECT workflow, step, key, generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM tasks WHERE id = ?1",
            [task],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "run": run.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "machine": trace.machine,
        "commit": commit,
        "exit": exit,
        "duration_ms": duration_ms,
    });
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.gate.ran", &time, data)
}

/// Appends a `factory.run.scored` record: a scorer graded `score.run` of `task`.
pub(crate) fn scored(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    score: &crate::score::Score,
    gate: Option<&str>,
) -> rusqlite::Result<()> {
    // A run of an earlier generation keeps the generation its step record named.
    let (workflow, key, generation, time): (String, Option<String>, i64, String) = tx.query_row(
        "SELECT workflow, key,
           coalesce((SELECT json_extract(event, '$.data.generation') FROM trace
             WHERE type = 'factory.step.ended' AND json_extract(event, '$.data.run') = ?2
             ORDER BY seq DESC LIMIT 1), generation),
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM tasks WHERE id = ?1",
        params![task, score.run.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "run": score.run.to_string(),
        "key": key,
        "generation": generation,
        "step": score.step,
        "machine": trace.machine,
        "scorer": score.scorer,
        "kind": score.kind.name(),
        "verdict": score.verdict.name(),
        "reason": score.reason,
        "gate": gate,
    });
    if let (Some(judged), Value::Object(fields)) = (&score.judged, &mut data) {
        fields.insert("model".to_owned(), json!(judged.model));
        fields.insert(
            "agreement".to_owned(),
            json!((judged.calibration.agreement() * 10_000.0).round() / 10_000.0),
        );
        fields.insert("cases".to_owned(), json!(judged.calibration.cases));
    }
    if let (Some(Value::Object(intent)), Value::Object(fields)) = (&score.intent, &mut data) {
        fields.extend(intent.clone());
    }
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.run.scored", &time, data)
}

/// Appends a `factory.scorer.calibrated` record: a judge scorer ran on every one of its labeled
/// cases, and `data` holds its agreement with the hashes of its prompt and cases.
pub(crate) fn calibrated(tx: &Transaction, trace: &Trace, data: Value) -> rusqlite::Result<()> {
    let time: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get(0)
    })?;
    let mut data = data;
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.scorer.calibrated", &time, data)
}

/// Appends a record of `kind` now, with `data`.
pub(crate) fn record(
    tx: &Transaction,
    trace: &Trace,
    kind: &str,
    data: Value,
) -> rusqlite::Result<()> {
    let time: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get(0)
    })?;
    append(tx, trace, kind, &time, data)
}

/// Appends a `factory.bench.compared` record: a comparison of two commits on a benchmark ended.
pub(crate) fn compared(tx: &Transaction, trace: &Trace, data: Value) -> rusqlite::Result<()> {
    let time: String = tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get(0)
    })?;
    append(tx, trace, "factory.bench.compared", &time, data)
}

/// The earlier comparisons of `base` and `head` on `benchmark`: how many, and whether one of them
/// was complete.
pub(crate) fn comparisons(
    db: &Connection,
    benchmark: &str,
    base: &str,
    head: &str,
) -> rusqlite::Result<(usize, bool)> {
    db.query_row(
        "SELECT count(*), coalesce(max(json_extract(event, '$.data.complete')), 0) FROM trace
         WHERE type = 'factory.bench.compared'
           AND json_extract(event, '$.data.benchmark') = ?1
           AND json_extract(event, '$.data.base') = ?2
           AND json_extract(event, '$.data.head') = ?3",
        params![benchmark, base, head],
        |row| Ok((row.get::<_, i64>(0)? as usize, row.get::<_, i64>(1)? != 0)),
    )
}

/// How many earlier comparisons on `benchmark` this machine posted to the pull request `pull`.
pub(crate) fn comparisons_of(
    db: &Connection,
    benchmark: &str,
    pull: &str,
) -> rusqlite::Result<usize> {
    db.query_row(
        "SELECT count(*) FROM trace
         WHERE type = 'factory.bench.compared'
           AND json_extract(event, '$.data.benchmark') = ?1
           AND json_extract(event, '$.data.pull_request') = ?2",
        params![benchmark, pull],
        |row| Ok(row.get::<_, i64>(0)? as usize),
    )
}

/// The `source` of every record this factory writes on this machine.
pub(crate) fn source(trace: &Trace) -> String {
    match &trace.machine {
        Some(machine) => format!("factory://{}/{machine}", trace.factory.name),
        None => format!("factory://{}", trace.factory.name),
    }
}

/// Appends a `factory.terminal.*` record of `kind` for `run`, with `fields` beside its task's.
pub(crate) fn terminal(
    tx: &Transaction,
    trace: &Trace,
    kind: &str,
    run: i64,
    fields: Value,
) -> rusqlite::Result<()> {
    let (task, workflow, step, key, generation, time): (i64, String, String, String, i64, String) =
        tx.query_row(
            "SELECT tasks.id, tasks.workflow, runs.step, tasks.key, tasks.generation, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1",
            [run],
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
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "workflow": workflow,
        "task": task.to_string(),
        "key": key,
        "generation": generation,
        "step": step,
        "run": run.to_string(),
        "machine": trace.machine,
    });
    if let (Value::Object(data), Value::Object(fields)) = (&mut data, fields) {
        data.extend(fields);
        data.retain(|_, value| !value.is_null());
    }
    append(tx, trace, kind, &time, data)
}

/// Appends one CloudEvents record of `kind` to the trace.
fn append(
    tx: &Transaction,
    trace: &Trace,
    kind: &str,
    time: &str,
    data: Value,
) -> rusqlite::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let source = source(trace);
    let event = json!({
        "specversion": "1.0",
        "id": id,
        "source": source,
        "type": kind,
        "time": time,
        "datacontenttype": "application/json",
        "data": data,
    });
    tx.execute(
        "INSERT INTO trace (id, type, time, event) VALUES (?1, ?2, ?3, ?4)",
        params![id, kind, time, event.to_string()],
    )?;
    Ok(())
}

/// The task's step as it ends, read in one query.
struct Ended {
    workflow: String,
    step: String,
    key: String,
    started: String,
    time: String,
    duration: i64,
    tokens: Option<String>,
    model: Option<String>,
    generation: i64,
    lane: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_trusted_only_when_each_grant_of_its_harness_holds() {
        let db = crate::state::memory();
        for (run, harness, outcome) in [
            (1, "claude-code", "granted"),
            (1, "claude-code", "present"),
            (1, "direnv", "refused"),
            (2, "claude-code", "granted"),
            (2, "claude-code", "refused"),
            (3, "direnv", "granted"),
        ] {
            let event =
                json!({"data": {"run": run.to_string(), "harness": harness, "outcome": outcome}});
            db.execute(
                "INSERT INTO trace (id, type, time, event)
                 VALUES (hex(randomblob(8)), 'factory.run.trusted', '2026-11-24T00:00:00Z', ?1)",
                [event.to_string()],
            )
            .unwrap();
        }

        assert_eq!(
            [1, 2, 3, 4].map(|run| granted(&db, run)),
            [true, false, false, false]
        );
    }
}
