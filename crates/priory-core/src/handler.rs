use crate::tools;
use crate::trace::{self, Handled, Trace};
use priory_factory::factory::Factory;
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::Value;

/// The most handler calls one run gets.
pub(crate) const CALLS: i64 = 3;

/// The model of a handler agent that names none, and of the built-in handler.
pub(crate) const MODEL: &str = "haiku";

/// A live run that might be stalled: it reached `SessionStart`, shows no permission prompt, and
/// sleeps on no open ask or wait.
pub(crate) struct Quiet {
    pub run: i64,
    pub tmux: String,
    /// Seconds since its last hook event, or its start.
    pub silent: i64,
    /// Seconds since its last handler call, if it had one.
    pub handled: Option<i64>,
    pub calls: i64,
}

pub(crate) fn quiet(db: &Connection) -> rusqlite::Result<Vec<Quiet>> {
    let mut statement = db.prepare(
        "SELECT runs.id, runs.tmux, unixepoch('now') - unixepoch(coalesce(runs.signaled, runs.started)),
         (SELECT unixepoch('now') - unixepoch(max(called)) FROM handlers WHERE run = runs.id),
         (SELECT count(*) FROM handlers WHERE run = runs.id)
         FROM runs WHERE runs.ended IS NULL AND runs.ready AND runs.prompted IS NULL
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
                silent: row.get(2)?,
                handled: row.get(3)?,
                calls: row.get(4)?,
            })
        })?
        .collect()
}

/// The model the handler calls: the `handler` agent's, else `haiku`.
pub(crate) fn model(factory: &Factory) -> &str {
    factory
        .settings
        .as_ref()
        .and_then(|settings| settings.value.handler.as_ref())
        .and_then(|name| factory.agents.get(name))
        .and_then(|agent| agent.value.model.as_deref())
        .unwrap_or(MODEL)
}

const ASK: &str = r#"You are Priory's handler. Priory runs coding agents unattended in tmux. The agent below seems stalled: its run sent no hook event and its screen did not change for a while. Read its screen and answer with one JSON object and nothing else, one of:
{"action": "nudge", "text": "<a short message Priory types to the agent>"} when a short message would get it going again;
{"action": "restart", "why": "<one line>"} when its process is wedged and only a new process on the same conversation can help;
{"action": "leave", "why": "<one line>"} when it still works, or waits on something a message cannot fix.
Never answer a question the agent puts to a person, and never approve, deny or pick an option of a permission prompt or a menu: answer leave instead."#;

/// What the handler reads for the stalled run `run`: its step's prompt, the prompts it received
/// last, its task's recent trace and the last lines of its screen.
pub(crate) fn prompt(
    db: &Connection,
    factory: &Factory,
    run: i64,
    stalled: i64,
    screen: &[String],
) -> rusqlite::Result<String> {
    let (task, workflow, step): (i64, String, String) = db.query_row(
        "SELECT runs.task, tasks.workflow, runs.step FROM runs JOIN tasks ON tasks.id = runs.task
         WHERE runs.id = ?1",
        [run],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
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
        "SELECT text FROM outbox WHERE task = ?1 AND state != 'queued' ORDER BY id DESC LIMIT 3",
    )?;
    let records = latest(
        "SELECT json_extract(event, '$.data') FROM trace
         WHERE json_extract(event, '$.data.task') = CAST(?1 AS TEXT) ORDER BY seq DESC LIMIT 3",
    )?;
    Ok(format!(
        "{ASK}\n\nIt has been stalled for {stalled}s.\n\nIts step's prompt:\n{step_prompt}\n\n\
         The last prompts it received:\n{prompts}\n\nIts task's recent trace:\n{records}\n\n\
         The last lines of its screen:\n{}\n",
        screen.join("\n")
    ))
}

/// Records a handler call on `run` and returns its id.
pub(crate) fn called(db: &Connection, run: i64) -> rusqlite::Result<i64> {
    db.query_row(
        "INSERT INTO handlers (run) VALUES (?1) RETURNING id",
        [run],
        |row| row.get(0),
    )
}

/// A daemon that stopped left its handler calls unanswered; their runs keep the call in their count.
pub(crate) fn interrupted(db: &Connection) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE handlers SET answered = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), action = 'leave',
         text = 'interrupted by a daemon restart' WHERE answered IS NULL",
        [],
    )
}

/// What a handler call answered, read from Claude's `--output-format json`.
struct Answer {
    action: &'static str,
    text: String,
    model: Option<String>,
    tokens: Option<Value>,
    cost_usd: Option<f64>,
}

fn read(output: Option<&str>, status: &str) -> Answer {
    let parsed: Option<Value> = output.and_then(|output| serde_json::from_str(output).ok());
    let failed = |text: String| Answer {
        action: "leave",
        text,
        model: None,
        tokens: None,
        cost_usd: None,
    };
    let Some(parsed) = parsed else {
        return failed(format!("the handler call gave no answer: {status}"));
    };
    let usage = &parsed["usage"];
    let tokens = usage.is_object().then(|| {
        serde_json::json!({
            "input": usage["input_tokens"].as_i64().unwrap_or(0),
            "output": usage["output_tokens"].as_i64().unwrap_or(0),
            "cache_read": usage["cache_read_input_tokens"].as_i64().unwrap_or(0),
            "cache_write": usage["cache_creation_input_tokens"].as_i64().unwrap_or(0),
        })
    });
    let model = parsed["modelUsage"]
        .as_object()
        .and_then(|models| models.keys().next().cloned());
    let result = parsed["result"].as_str().unwrap_or_default();
    // A model may wrap its JSON in prose or a code fence.
    let chosen: Option<Value> = result
        .find('{')
        .zip(result.rfind('}'))
        .and_then(|(start, end)| serde_json::from_str(result.get(start..=end)?).ok());
    let field = |name: &str| {
        chosen
            .as_ref()
            .and_then(|chosen| chosen[name].as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let (action, text) = match field("action").as_deref() {
        Some("nudge") => match field("text") {
            Some(text) => ("nudge", text),
            None => ("leave", "the handler nudged with no text".to_owned()),
        },
        Some("restart") => ("restart", field("why").unwrap_or_default()),
        Some("leave") => ("leave", field("why").unwrap_or_default()),
        _ => ("leave", format!("the handler gave no action: {result}")),
    };
    Answer {
        action,
        text,
        model,
        tokens,
        cost_usd: parsed["total_cost_usd"].as_f64(),
    }
}

/// Records handler call `id`'s answer, from its JSON `output`, and applies it unless its run ended
/// or `moved` on since the call: a nudge goes through the outbox, a restart ends the run so its
/// task resumes the session in a new one. Returns a tmux session to kill.
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
    let (run, live): (i64, bool) = tx.query_row(
        "SELECT handlers.run, runs.ended IS NULL AND (runs.signaled IS NULL
           OR runs.signaled < handlers.called)
         FROM handlers JOIN runs ON runs.id = handlers.run WHERE handlers.id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut text = answer.text.clone();
    let mut kill = None;
    if !live || moved {
        if answer.action != "leave" {
            text.push_str(" (not applied: the run moved on)");
        }
    } else if answer.action == "nudge" {
        tools::nudge_stalled(&tx, run, &answer.text)?;
    } else if answer.action == "restart" {
        kill = Some(tools::restart(&tx, trace, run, &answer.text)?);
    }
    tx.execute(
        "UPDATE handlers SET answered = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), action = ?2,
         text = ?3 WHERE id = ?1",
        params![id, answer.action, text],
    )?;
    trace::handled(
        &tx,
        trace,
        &Handled {
            run,
            action: answer.action,
            text: &text,
            model: answer
                .model
                .as_deref()
                .unwrap_or_else(|| model(trace.factory)),
            tokens: answer.tokens,
            cost_usd: answer.cost_usd,
        },
    )?;
    tx.commit()?;
    Ok(kill)
}
