use priory_factory::factory::{Decider, Factory};
use priory_runtime::transcript::Usage;
use rusqlite::{OptionalExtension, Transaction, params};
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
        "decider": step.decides.name(),
        "effect": step.effect,
        "outcome": outcome,
        "machine": trace.machine,
        "started": ended.started,
        "duration_ms": ended.duration.max(0),
    });
    if let Some(lane) = ended.lane {
        data["lane"] = json!(lane);
    }
    let agent = step
        .agent
        .as_ref()
        .and_then(|name| trace.factory.agents.get(name))
        .filter(|_| step.decides == Decider::Model);
    if agent.is_some() && crate::tools::GRADES.contains(&outcome) {
        data["grade"] = json!(outcome);
    }
    if let Some(agent) = agent {
        if let Some(run) = run {
            data["kit_hash"] = tx
                .query_row("SELECT kit_hash FROM runs WHERE id = ?1", [run], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten()
                .map_or(Value::Null, Value::from);
        }
        data["harness"] = json!(agent.value.harness);
        data["model"] = json!(ended.model.or_else(|| agent.value.model.clone()));
        data["tokens"] = ended
            .tokens
            .and_then(|tokens| serde_json::from_str(&tokens).ok())
            .unwrap_or(Value::Null);
    }
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.step.ended", &ended.time, data)
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
/// source event: `happened`, when the event happened, else now, and `evidence`. It names the record
/// of the step that graded the task, when one did, and its grade.
pub(crate) fn labeled(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    label: &str,
    evidence: &str,
    happened: Option<&str>,
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
             AND json_extract(event, '$.data.happened') = ?4",
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
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    append(tx, trace, "factory.step.labeled", &time, data)
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

/// The `source` of every record this factory writes on this machine.
pub(crate) fn source(trace: &Trace) -> String {
    match &trace.machine {
        Some(machine) => format!("factory://{}/{machine}", trace.factory.name),
        None => format!("factory://{}", trace.factory.name),
    }
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
