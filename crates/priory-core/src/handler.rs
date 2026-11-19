use crate::stall::Group;
use crate::tools;
use crate::trace::{self, Handled, Trace};
use priory_factory::factory::Factory;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};
use serde_json::Value;

/// The most handler calls one run gets.
pub(crate) const CALLS: i64 = 3;

/// The model of a handler agent that names none.
pub(crate) const MODEL: &str = "haiku";

/// Why a run cannot take a nudge: its earlier prompt may still sit in its input.
const BLOCKED: &str = "EXISTS (SELECT 1 FROM outbox JOIN runs AS typed ON typed.id = outbox.run
  WHERE outbox.task = runs.task AND outbox.state IN ('attempted', 'uncertain')
  AND typed.ended IS NULL)";

/// A live run that might be stalled: it reached `SessionStart`, shows no permission prompt, and
/// sleeps on no open ask or wait.
pub(crate) struct Quiet {
    pub run: i64,
    pub tmux: String,
    pub session: String,
    /// Seconds since its last hook event, or its start.
    pub silent: i64,
    /// Seconds since its last handler call, if it had one.
    pub handled: Option<i64>,
    /// The screen its last handler call read, when no hook event came since.
    pub left: Option<String>,
    pub calls: i64,
}

pub(crate) fn quiet(db: &Connection) -> rusqlite::Result<Vec<Quiet>> {
    let mut statement = db.prepare(
        "SELECT runs.id, runs.tmux, runs.session, unixepoch('now') - unixepoch(coalesce(runs.signaled, runs.started)),
         unixepoch('now') - unixepoch(last.called),
         iif(runs.signaled IS NULL OR runs.signaled < last.called, last.screen, NULL),
         (SELECT count(*) FROM handlers WHERE run = runs.id)
         FROM runs LEFT JOIN handlers AS last
           ON last.id = (SELECT max(id) FROM handlers WHERE run = runs.id)
         WHERE runs.ended IS NULL AND runs.ready AND runs.prompted IS NULL
         AND NOT EXISTS (SELECT 1 FROM asks WHERE asks.run = runs.id AND asks.answer IS NULL)
         AND NOT EXISTS (SELECT 1 FROM waits WHERE waits.run = runs.id AND waits.fired IS NULL
           AND waits.withdrawn IS NULL)
         ORDER BY runs.id",
    )?;
    statement
        .query_map([], |row| {
            Ok(Quiet {
                run: row.get(0)?,
                tmux: row.get(1)?,
                session: row.get(2)?,
                silent: row.get(3)?,
                handled: row.get(4)?,
                left: row.get(5)?,
                calls: row.get(6)?,
            })
        })?
        .collect()
}

/// The model the handler calls: the `handler` agent's, else `haiku`. A factory with no `handler`
/// calls none.
pub(crate) fn model(factory: &Factory) -> Option<&str> {
    let name = factory.settings.as_ref()?.value.handler.as_ref()?;
    let agent = factory.agents.get(name)?;
    Some(agent.value.model.as_deref().unwrap_or(MODEL))
}

const ASK: &str = r#"You are Priory's handler. Priory runs coding agents unattended in tmux. The agent below seems stalled: its run sent no hook event and its screen did not change for a while. Read its screen and answer with one JSON object and nothing else, one of:
{"action": "nudge", "text": "<a short message Priory types to the agent>"} when a short message would get it going again;
{"action": "restart", "why": "<one line>"} when its process is wedged and only a new process on the same conversation can help;
{"action": "leave", "why": "<one line>"} when it still works, or waits on something a message cannot fix.
Never answer a question the agent puts to a person, and never approve, deny or pick an option of a permission prompt or a menu: answer leave instead."#;

const NO_NUDGE: &str = "An earlier prompt may still sit in its input, so a nudge cannot reach it: answer restart or leave.";

/// What the handler reads for the stalled run `run`: its step's prompt, the prompts it received
/// last, its task's recent trace and the last lines of its screen.
pub(crate) fn prompt(
    db: &Connection,
    factory: &Factory,
    run: i64,
    stalled: i64,
    screen: &[String],
) -> rusqlite::Result<String> {
    let (task, workflow, step, blocked): (i64, String, String, bool) = db.query_row(
        &format!(
            "SELECT runs.task, tasks.workflow, runs.step, {BLOCKED} FROM runs
             JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1"
        ),
        [run],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let step_prompt = factory
        .workflows
        .get(&workflow)
        .and_then(|workflow| workflow.value.step(&step))
        .map_or_else(String::new, tools::step_prompt);
    let latest = |sql: &str| -> rusqlite::Result<String> {
        let mut statement = db.prepare(sql)?;
        let rows = statement
            .query_map([task], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().rev().collect::<Vec<_>>().join("\n\n"))
    };
    let prompts = latest(
        "SELECT text FROM outbox WHERE task = ?1 AND state NOT IN ('queued', 'withdrawn') ORDER BY id DESC LIMIT 3",
    )?;
    let records = latest(
        "SELECT json_extract(event, '$.data') FROM trace
         WHERE json_extract(event, '$.data.task') = CAST(?1 AS TEXT) ORDER BY seq DESC LIMIT 3",
    )?;
    let no_nudge = if blocked { NO_NUDGE } else { "" };
    Ok(format!(
        "{ASK}\n{no_nudge}\n\nIt has been stalled for {stalled}s.\n\nIts step's prompt:\n{step_prompt}\n\n\
         The last prompts it received:\n{prompts}\n\nIts task's recent trace:\n{records}\n\n\
         The last lines of its screen:\n{}\n",
        screen.join("\n")
    ))
}

/// Records a handler call on `run`, which read `screen`, and returns its id.
pub(crate) fn called(db: &Connection, run: i64, screen: &[String]) -> rusqlite::Result<i64> {
    db.query_row(
        "INSERT INTO handlers (run, screen) VALUES (?1, ?2) RETURNING id",
        params![run, screen.join("\n")],
        |row| row.get(0),
    )
}

/// Records the process group of handler call `id` and its start time, so a restarted daemon kills
/// it and never a process that reused its pid.
pub(crate) fn spawned(
    db: &Connection,
    id: i64,
    pid: u32,
    started: Option<i64>,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE handlers SET pid = ?2, pid_started = ?3 WHERE id = ?1",
        params![id, pid, started],
    )
    .map(drop)
}

/// Forgets the screen the last handler call on `run` read, once the run showed another: a stall that
/// comes back to that screen later is a new one.
pub(crate) fn redrawn(db: &Connection, run: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE handlers SET screen = '' WHERE id = (SELECT max(id) FROM handlers WHERE run = ?1)",
        [run],
    )
    .map(drop)
}

/// The handler calls a stopped daemon left unanswered, with their process group and its start time.
pub(crate) fn orphans(db: &Connection) -> rusqlite::Result<Vec<(i64, Option<Group>)>> {
    let mut statement =
        db.prepare("SELECT id, pid, pid_started FROM handlers WHERE answered IS NULL")?;
    statement
        .query_map([], |row| {
            let pid: Option<u32> = row.get(1)?;
            Ok((row.get(0)?, pid.zip(row.get(2)?)))
        })?
        .collect()
}

/// What the handler chose, as its JSON names it.
#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
enum Choice {
    Nudge { text: String },
    Restart { why: String },
    Leave { why: String },
}

/// What a handler call answered, read from Claude's `--output-format json`.
struct Answer {
    choice: Choice,
    usage: Usage,
}

/// What a one-shot `claude -p` call reported about itself.
pub(crate) struct Usage {
    pub model: Option<String>,
    pub tokens: Option<Value>,
    pub cost_usd: Option<f64>,
}

/// A one-shot call's output, if it parses, with its usage.
pub(crate) fn parse(output: Option<&str>) -> (Value, Usage) {
    let parsed: Value = output
        .and_then(|output| serde_json::from_str(output).ok())
        .unwrap_or(Value::Null);
    let usage = &parsed["usage"];
    let tokens = usage.is_object().then(|| {
        serde_json::json!({
            "input": usage["input_tokens"].as_i64().unwrap_or(0),
            "output": usage["output_tokens"].as_i64().unwrap_or(0),
            "cache_read": usage["cache_read_input_tokens"].as_i64().unwrap_or(0),
            "cache_write": usage["cache_creation_input_tokens"].as_i64().unwrap_or(0),
        })
    });
    let usage = Usage {
        model: parsed["modelUsage"]
            .as_object()
            .and_then(|models| models.keys().next().cloned()),
        tokens,
        cost_usd: parsed["total_cost_usd"].as_f64(),
    };
    (parsed, usage)
}

/// Whether a one-shot call that ended with `status` answered successfully.
pub(crate) fn succeeded(parsed: &Value, status: &str) -> bool {
    matches!(status, "exit 0" | INTERRUPTED)
        && parsed["is_error"] == false
        && parsed["subtype"] == "success"
}

/// The status of a call a stopped daemon left: its output, if any, is read but never applied.
pub(crate) const INTERRUPTED: &str = "interrupted by a daemon restart";

/// Reads the call's output. Only a call that exited 0, or was interrupted after it wrote, with a
/// successful result counts; any other becomes `leave`, keeping its usage.
fn read(output: Option<&str>, status: &str) -> Answer {
    let (parsed, usage) = parse(output);
    let result = parsed["result"].as_str().unwrap_or_default();
    let succeeded = succeeded(&parsed, status);
    // A model may wrap its JSON in prose or a code fence.
    let chosen = result
        .find('{')
        .zip(result.rfind('}'))
        .and_then(|(start, end)| serde_json::from_str::<Choice>(result.get(start..=end)?).ok())
        .filter(|choice| {
            let (Choice::Nudge { text }
            | Choice::Restart { why: text }
            | Choice::Leave { why: text }) = choice;
            !text.trim().is_empty()
        });
    let choice = match chosen {
        Some(choice) if succeeded => choice,
        _ if !succeeded => Choice::Leave {
            why: format!("the handler call failed: {status}"),
        },
        _ => Choice::Leave {
            why: format!("the handler gave no action: {result}"),
        },
    };
    Answer { choice, usage }
}

/// Why the answer to call `id` must not act on its run, if it must not: the run ended, sent a hook
/// event, stored a result, ask or wait, or took a prompt since the call.
fn moved_on(tx: &Transaction, id: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT runs.ended IS NOT NULL OR runs.signaled >= handlers.called
         OR EXISTS (SELECT 1 FROM results WHERE run = runs.id)
         OR EXISTS (SELECT 1 FROM asks WHERE run = runs.id AND created >= handlers.called)
         OR EXISTS (SELECT 1 FROM waits WHERE run = runs.id AND created >= handlers.called)
         OR EXISTS (SELECT 1 FROM outbox WHERE run = runs.id AND attempted >= handlers.called)
         FROM handlers JOIN runs ON runs.id = handlers.run WHERE handlers.id = ?1",
        [id],
        |row| {
            row.get::<_, Option<bool>>(0)
                .map(|moved| moved.unwrap_or(false))
        },
    )
}

/// Records handler call `id`'s answer, from its JSON `output` and process `status`, and applies it
/// unless the run `moved` on its screen or otherwise since the call: a nudge goes through the
/// outbox, a restart ends the run so its task resumes the session in a new one. Returns a tmux
/// session to kill.
pub(crate) fn answered(
    db: &mut Connection,
    trace: &Trace,
    id: i64,
    output: Option<&str>,
    status: &str,
    moved: bool,
) -> rusqlite::Result<Option<String>> {
    let answer = read(output, status);
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (run, blocked): (i64, bool) = tx.query_row(
        &format!(
            "SELECT runs.id, {BLOCKED} FROM handlers JOIN runs ON runs.id = handlers.run
             WHERE handlers.id = ?1"
        ),
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let skipped = if moved || moved_on(&tx, id)? {
        Some("the run moved on")
    } else if blocked && matches!(answer.choice, Choice::Nudge { .. }) {
        Some("an earlier prompt still waits for its confirmation")
    } else {
        None
    };
    let (action, mut text, kill) = match (&answer.choice, skipped) {
        (Choice::Leave { why }, _) => ("leave", why.clone(), None),
        (Choice::Nudge { text }, None) => {
            tools::nudge_stalled(&tx, run, text)?;
            ("nudge", text.clone(), None)
        }
        (Choice::Restart { why }, None) => {
            let tmux = tools::restart(&tx, trace, run, why)?;
            ("restart", why.clone(), Some(tmux))
        }
        (Choice::Nudge { text }, Some(_)) => ("nudge", text.clone(), None),
        (Choice::Restart { why }, Some(_)) => ("restart", why.clone(), None),
    };
    if let (Some(why), false) = (skipped, action == "leave") {
        text.push_str(&format!(" (not applied: {why})"));
    }
    tx.execute(
        "UPDATE handlers SET answered = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), action = ?2,
         text = ?3 WHERE id = ?1",
        params![id, action, text],
    )?;
    trace::handled(
        &tx,
        trace,
        &Handled {
            run,
            action,
            text: &text,
            model: answer
                .usage
                .model
                .as_deref()
                .or_else(|| model(trace.factory))
                .unwrap_or(MODEL),
            tokens: answer.usage.tokens,
            cost_usd: answer.usage.cost_usd,
        },
    )?;
    tx.commit()?;
    Ok(kill)
}
