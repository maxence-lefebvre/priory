use crate::route;
use priory_factory::factory::{Decider, Effect, Loaded, Step, Workflow};
use priory_intake::intake::{CHECK_RUNS, CHECKED, PASSING};
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
    if let Some(wait) = open_wait(&tx, run.id)? {
        return Ok(Err(format!("run {} already waits on wait {wait}", run.id)));
    }
    let stored = match call.tool.as_str() {
        "result" => result(&tx, &run, &call.arguments, workflows)?,
        "ask" => ask(&tx, &run, &call.arguments)?,
        "wait" => wait(&tx, &run, &call.arguments)?,
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
    if next == "done"
        && commit.is_none()
        && let Some(resource) = pull_request(tx, run.task)?
    {
        return Ok(Err(format!(
            "task {} owns {resource}: send the result with the commit its head must match",
            run.task
        )));
    }
    tx.execute(
        "INSERT INTO results (run, task, step, outcome, summary, evidence, commit_sha, inbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, (SELECT max(seq) FROM inbox))",
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

fn open_wait(tx: &Transaction, run: i64) -> rusqlite::Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM waits WHERE run = ?1 AND fired IS NULL",
        [run],
        |row| row.get(0),
    )
    .optional()
}

const CONDITIONS: &str = "pull_request, checks, review, time or task";

/// Stores the wait and fires it at once if an event since the agent's last prompt satisfies it.
fn wait(
    tx: &Transaction,
    run: &Run,
    arguments: &Value,
) -> rusqlite::Result<Result<String, String>> {
    let on = &arguments["on"];
    let kind = on["type"].as_str().unwrap_or_default();
    match kind {
        "pull_request" | "checks" | "review" => {
            let (Some(repo), Some(number)) = (on["repo"].as_str(), on["pull_request"].as_u64())
            else {
                return Ok(Err(format!(
                    "a {kind} wait needs repo, as owner/name, and pull_request, a number"
                )));
            };
            let resource = route::pull_request(repo, number);
            if let Some(other) = crate::state::owner(tx, std::slice::from_ref(&resource))?
                && other.id != run.task
            {
                return Ok(Err(format!(
                    "{resource} belongs to task {}, not this one",
                    other.id
                )));
            }
            tx.execute(
                "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
                params![run.task, resource],
            )?;
            let head = match &on["head_sha"] {
                Value::Null => None,
                Value::String(head) if kind == "checks" => Some(head.as_str()),
                _ => {
                    return Ok(Err(
                        "head_sha must be a commit, on a checks wait only".into()
                    ));
                }
            };
            tx.execute(
                "INSERT INTO waits (task, run, kind, resource, head_sha) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![run.task, run.id, kind, resource, head],
            )?;
        }
        "time" => {
            let until = on["until"].as_str().unwrap_or_default();
            let instant = until.parse::<herdr_bus::Timestamp>().ok();
            // The `+` refuses the SQLite modifiers that are not durations, such as `start of day`.
            let at: Option<String> = match instant {
                Some(instant) => tx.query_row(
                    "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1)",
                    [instant.to_string()],
                    |row| row.get(0),
                )?,
                None => tx.query_row(
                    "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+' || ?1)",
                    [until],
                    |row| row.get(0),
                )?,
            };
            let Some(at) = at else {
                return Ok(Err(format!(
                    "until {until:?} is neither an RFC 3339 instant nor a duration such as 30 seconds, 5 minutes or 2 hours"
                )));
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
        other => {
            return Ok(Err(format!(
                "unknown condition {other:?}: wait on {CONDITIONS}"
            )));
        }
    }
    let id = tx.last_insert_rowid();
    if catch_up(tx, run.task)? {
        return Ok(Ok(format!(
            "wait {id} already holds: end your turn; the event that satisfies it is your next prompt"
        )));
    }
    due(tx)?;
    let fired = tx.query_row(
        "SELECT fired IS NOT NULL FROM waits WHERE id = ?1",
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    if fired {
        return Ok(Ok(format!(
            "wait {id} already holds: end your turn; your next prompt says so"
        )));
    }
    Ok(Ok(format!(
        "wait {id} stored: end your turn now; Priory wakes you with the event that satisfies it"
    )))
}

/// Fires the task's open wait on the first event since the agent's last prompt that satisfies it:
/// one not typed yet, on which no wait fired. Says whether one did.
fn catch_up(tx: &Transaction, task: i64) -> rusqlite::Result<bool> {
    let mut candidates = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT inbox.seq FROM inbox LEFT JOIN outbox ON outbox.id = inbox.outbox
             WHERE inbox.task = ?1 AND inbox.outcome = 'kept'
             AND (inbox.outbox IS NULL OR outbox.state = 'queued')
             AND inbox.seq NOT IN (SELECT inbox FROM waits WHERE inbox IS NOT NULL)
             ORDER BY inbox.seq",
        )?;
        let mut rows = statement.query([task])?;
        while let Some(row) = rows.next()? {
            candidates.push(row.get::<_, i64>(0)?);
        }
    }
    for seq in candidates {
        if event(tx, task, seq)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Fires the task's open wait if the inbox entry `seq` satisfies it, and says whether it did.
pub(crate) fn event(tx: &Transaction, task: i64, seq: i64) -> rusqlite::Result<bool> {
    let open = tx
        .query_row(
            "SELECT waits.id, waits.kind, waits.resource, waits.head_sha, waits.daemon FROM waits
             JOIN tasks ON tasks.id = waits.task
             WHERE waits.task = ?1 AND waits.fired IS NULL AND tasks.state IN ('running', 'asleep')",
            [task],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((wait, kind, Some(resource), head, daemon)) = open else {
        return Ok(false);
    };
    if daemon {
        return judge(tx, wait);
    }
    let (kind_of_event, subject, data): (String, Option<String>, String) = tx.query_row(
        "SELECT type, subject, data FROM inbox WHERE seq = ?1",
        [seq],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let line = crate::state::line(&kind_of_event, subject.as_deref(), &data);
    let data: Value = serde_json::from_str(&data).expect("inbox data is JSON");
    if !route::satisfies(&kind, &resource, head.as_deref(), &kind_of_event, &data) {
        return Ok(false);
    }
    fire(tx, wait, task, &line, Some(seq))?;
    Ok(true)
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

/// Judges the daemon's checks wait `wait` on every event the inbox kept or ignored about its pull
/// request, so that events the agent already read, or an earlier owner kept, count too. The newest event tells the head: one after the
/// result that names another head wakes the agent. The newest check at the result's commit then
/// decides: red wakes the agent, and green with none pending or failing marks the task done. With
/// no check at the commit, a newest snapshot of no check runs marks it done too.
fn judge(tx: &Transaction, wait: i64) -> rusqlite::Result<bool> {
    let (task, resource, commit, mark): (i64, String, String, Option<i64>) = tx.query_row(
        "SELECT waits.task, waits.resource, waits.head_sha, coalesce(
           (SELECT max(inbox) FROM actions WHERE result = waits.run AND commit_sha = waits.head_sha),
           results.inbox)
         FROM waits JOIN results ON results.run = waits.run WHERE waits.id = ?1",
        [wait],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let mut seen = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT seq, type, data FROM inbox
             WHERE outcome IN ('kept', 'ignored')
             AND 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1
             AND json_extract(data, '$.head_sha') IS NOT NULL ORDER BY seq DESC",
        )?;
        let mut rows = statement.query(params![resource])?;
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
        return Ok(false);
    };
    if newest.head != commit {
        // A head seen before the result may be one the agent's commit replaced since.
        if newest.seq <= mark.unwrap_or_default() {
            return Ok(false);
        }
        let reason = format!(
            "wait {wait} fired: the head of {resource} moved to {}, away from your result's commit {commit}",
            newest.head
        );
        fire(tx, wait, task, &reason, Some(newest.seq))?;
        return Ok(true);
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
        return Ok(false);
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
        return Ok(true);
    }
    let green = match check.kind.as_str() {
        CHECK_RUNS => check.data["runs"].as_u64() == Some(0),
        _ => check.data["pending"].as_u64() == Some(0) && failing == Some(0),
    };
    if !green {
        return Ok(false);
    }
    tx.execute(
        &format!("UPDATE waits SET fired = {NOW}, inbox = ?2 WHERE id = ?1"),
        params![wait, check.seq],
    )?;
    set(tx, task, "done", None, None)?;
    Ok(true)
}

/// Fires every open time wait whose instant passed and task wait whose task ended.
pub(crate) fn due(tx: &Transaction) -> rusqlite::Result<()> {
    let mut fired = Vec::new();
    {
        let mut statement = tx.prepare(
            "SELECT waits.id, waits.task, waits.until, waits.target, target.state FROM waits
             JOIN tasks ON tasks.id = waits.task LEFT JOIN tasks AS target ON target.id = waits.target
             WHERE waits.fired IS NULL AND tasks.state IN ('running', 'asleep')
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

/// Queues `text`, then the event `seq` unless `text` is its line or another prompt carries it, then
/// the kept events no prompt carries, as the task's next prompt.
fn fire(
    tx: &Transaction,
    wait: i64,
    task: i64,
    text: &str,
    seq: Option<i64>,
) -> rusqlite::Result<()> {
    tx.execute(
        &format!("UPDATE waits SET fired = {NOW}, inbox = ?2 WHERE id = ?1"),
        params![wait, seq],
    )?;
    let mut carried = Vec::new();
    let mut text = text.to_owned();
    if let Some(seq) = seq {
        let (prompt, own, line): (Option<i64>, bool, String) = tx.query_row(
            "SELECT inbox.outbox, coalesce(outbox.state = 'queued' AND outbox.text = line, 0), line
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
    let id = queue_first(tx, task, &text)?;
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
    workflows: &Workflows,
) -> rusqlite::Result<Option<String>> {
    let Some(run) = run(tx, session)? else {
        return Ok(None);
    };
    Ok(settle(tx, &run, workflows)?.flatten())
}

/// Settles the live run of `session`, whose agent process is gone, as its `Stop` would have, when a
/// result, ask or wait ended its turn. Says whether one did; otherwise nothing changes.
pub(crate) fn gone(
    db: &mut Connection,
    session: &str,
    workflows: &Workflows,
) -> rusqlite::Result<bool> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(mut run) = run(&tx, session)? else {
        return Ok(false);
    };
    // No process carries on, so even a next step of the same agent needs a new run.
    run.agent = None;
    let settled = settle(&tx, &run, workflows)?.is_some();
    tx.commit()?;
    Ok(settled)
}

/// Moves the run's task by its result, or puts it to sleep on its open ask or wait. `None` when
/// none of them ended the turn; otherwise the tmux session to kill, if any.
fn settle(
    tx: &Transaction,
    run: &Run,
    workflows: &Workflows,
) -> rusqlite::Result<Option<Option<String>>> {
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
            Some(run),
            &summary,
            workflows,
        )
        .map(Some);
    }
    let asked = tx
        .query_row(
            "SELECT 1 FROM asks WHERE run = ?1 AND answer IS NULL",
            [run.id],
            |_| Ok(()),
        )
        .optional()?;
    if asked.is_none() && open_wait(tx, run.id)?.is_none() {
        return Ok(None);
    }
    end(tx, run.id, "asleep")?;
    set(tx, run.task, "asleep", None, None)?;
    Ok(Some(Some(run.tmux.clone())))
}

/// Queues `text` as the task's next prompt, ahead of the prompts already queued, which it carries
/// after it: an event kept during the last turn must not reach the agent before its new instructions.
fn queue_first(tx: &Transaction, task: i64, text: &str) -> rusqlite::Result<i64> {
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
    Ok(id)
}

/// The pull request the task owns: the latest one its actions opened, else the one its latest wait
/// named, else the latest one opened from its branch, else its key when a pull request event created it.
fn pull_request(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT resource FROM (
           SELECT pull_request AS resource, 3 AS rank, id AS at FROM actions
             WHERE task = ?1 AND pull_request IS NOT NULL
           UNION ALL
           SELECT resource, 2, id FROM waits WHERE task = ?1 AND resource IS NOT NULL
           UNION ALL
           SELECT 'github:' || json_extract(inbox.data, '$.repo') || '#' || json_extract(inbox.data, '$.pull_request'), 1, inbox.seq
             FROM inbox JOIN tasks ON tasks.id = inbox.task
             WHERE inbox.task = ?1 AND inbox.outcome = 'kept'
             AND json_extract(inbox.data, '$.pull_request') IS NOT NULL
             AND json_extract(inbox.data, '$.head_branch') = tasks.branch
           UNION ALL
           SELECT tasks.key, 0, 0 FROM tasks JOIN inbox ON inbox.id = tasks.source
             WHERE tasks.id = ?1 AND inbox.type LIKE 'github.pull_request.%'
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
    if target == "done"
        && let Some(resource) = pull_request(tx, task)?
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
                format!("task reached done owning {resource}, but no result names a commit");
            set(tx, task, "blocked", None, Some(&reason))?;
            return finish("blocked");
        };
        tx.execute(
            "INSERT INTO waits (task, run, kind, resource, head_sha, daemon)
             VALUES (?1, ?2, 'checks', ?3, ?4, 1)",
            params![task, result_run, resource, commit],
        )?;
        let wait = tx.last_insert_rowid();
        // The agent of the result's step fixes what the checks find, even after a human step.
        set(tx, task, "asleep", Some(&at), None)?;
        judge(tx, wait)?;
        return finish("asleep");
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
    match step.decides {
        Decider::Model => {
            queue_first(tx, task, step.prompt.as_deref().unwrap_or_default())?;
            // An irreversible step starts a new run, whose start checks its approval.
            let carry = |run: &&Run| run.agent == step.agent && step.effect != Effect::Irreversible;
            let Some(run) = from.filter(carry) else {
                set(tx, task, "ready", Some(target), None)?;
                return finish("moved");
            };
            end(tx, run.id, "moved")?;
            tx.execute(
                "INSERT INTO runs (task, step, agent, tmux, session, idle, ready)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, 1)",
                params![task, target, run.agent, run.tmux, run.session],
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
            set(tx, task, "ready", Some(target), None)?;
            finish("moved")
        }
    }
}

/// Applies a person's answer. The first answer to an ask's current revision stands. A step's ask
/// moves the task by the chosen option; an agent's ask queues the answer as its next prompt.
/// An answer toward a step the asked step `approves` records an approval at the task's commit, which
/// `head` reads from its worktree.
pub(crate) fn answer(
    db: &mut Connection,
    answer: &Answer,
    workflows: &Workflows,
    head: &dyn Fn(&Path, &str) -> Option<String>,
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
    if ["done", "failed", "dropped"].contains(&state.as_str()) {
        return Ok(Err(format!("task {task} is {state}")));
    }
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
        let approved: Vec<&String> = workflows
            .get(&workflow)
            .and_then(|flow| Some((&flow.value, flow.value.step(&at)?)))
            .into_iter()
            .flat_map(|(flow, asked)| {
                asked
                    .approves
                    .iter()
                    .filter(|approved| flow.reaches(&target, approved))
            })
            .collect();
        approve(&tx, task, answer, &approved, head)?;
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

/// A person's desk action on a task, as `priory resume` or `priory cancel` sends it on the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Act {
    pub act: String,
    pub task: i64,
}

/// Applies a desk action. `resume` makes a blocked task `ready`, with a prompt that says why it
/// stopped. `cancel` drops an active task and ends its live run. Returns the reply and a tmux
/// session to kill.
pub(crate) fn act(
    db: &mut Connection,
    act: &Act,
) -> rusqlite::Result<(Result<String, String>, Option<String>)> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
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
            let text = format!(
                "A person resumed this task after: {}. Check the worktree, then continue.",
                reason.as_deref().unwrap_or("no reason")
            );
            queue_first(&tx, act.task, &text)?;
            set(&tx, act.task, "ready", None, None)?;
            tx.commit()?;
            Ok((Ok(format!("task {} is ready", act.task)), None))
        }
        "resume" => Ok((
            Err(format!("task {} is {state}, not blocked", act.task)),
            None,
        )),
        "cancel" if ["done", "failed", "dropped"].contains(&state.as_str()) => {
            Ok((Err(format!("task {} is already {state}", act.task)), None))
        }
        "cancel" => {
            let tmux = tx
                .query_row(
                    "SELECT tmux FROM runs WHERE task = ?1 AND ended IS NULL",
                    [act.task],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            tx.execute(
                &format!(
                    "UPDATE runs SET ended = {NOW}, ended_as = 'cancelled' WHERE task = ?1 AND ended IS NULL"
                ),
                [act.task],
            )?;
            set(
                &tx,
                act.task,
                "dropped",
                None,
                Some("cancelled from the desk"),
            )?;
            tx.commit()?;
            Ok((Ok(format!("task {} dropped", act.task)), tmux))
        }
        other => Ok((Err(format!("unknown action {other}")), None)),
    }
}

/// Ends the live run of `session`, whose tmux session was gone when the daemon started, as `lost`.
/// Its task becomes `ready` with a prompt that says so, unless the task's previous run was lost too
/// and this one stored no ask or wait: then the task blocks. A result would have settled it instead.
pub(crate) fn lost(db: &mut Connection, session: &str, tmux: &str) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = run(&tx, session)? else {
        return Ok(());
    };
    end(&tx, run.id, "lost")?;
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
        queue_first(&tx, run.task, &text)?;
        set(&tx, run.task, "ready", None, None)?;
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
            "INSERT INTO approvals (task, step, ask, revision, commit_sha) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task, step, answer.ask, answer.revision, commit],
        )?;
    }
    Ok(())
}

/// Whether the irreversible step `at` may run at the task's commit `head`. With no commit or no
/// approval, the task blocks. With an approval given at another commit, its ask gets a new revision
/// and the task sleeps at the approving step until a person answers it again.
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
            "SELECT approvals.ask, approvals.commit_sha, asks.step FROM approvals
             JOIN asks ON asks.id = approvals.ask
             WHERE approvals.task = ?1 AND approvals.step = ?2 ORDER BY approvals.id DESC LIMIT 1",
            params![task, at.id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((ask, commit, approving)) = approval else {
        let reason = format!("step {} is irreversible and no answer approved it", at.id);
        set(&tx, task, "blocked", None, Some(&reason))?;
        tx.commit()?;
        return Ok(false);
    };
    if commit.as_deref() == Some(head) {
        return Ok(true);
    }
    let moved = format!(
        "\nasked again: the branch moved from {} to {head} since this was answered",
        commit.as_deref().unwrap_or("-"),
    );
    tx.execute(
        "UPDATE asks SET revision = revision + 1, answer = NULL, answered = NULL,
         question = question || ?2 WHERE id = ?1",
        params![ask, moved],
    )?;
    if let Some(prompt) = at.prompt.as_deref() {
        unqueue(&tx, task, prompt)?;
    }
    set(&tx, task, "asleep", Some(&approving), None)?;
    tx.commit()?;
    Ok(false)
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
}

/// Records the run of the code step `at`'s action, makes the task own the pull request it opened,
/// and moves the task by `next[status]`.
pub(crate) fn performed(
    db: &mut Connection,
    task: i64,
    workflow: &str,
    at: &Step,
    mut done: Done,
    workflows: &Workflows,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(resource) = done.pull_request.take() {
        match crate::state::owner(&tx, std::slice::from_ref(&resource))? {
            Some(other) if other.id != task => {
                done.status = "failed";
                done.error = Some(format!(
                    "{resource} belongs to task {}, not this one",
                    other.id
                ));
            }
            _ => done.pull_request = Some(resource),
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
    if let Some(resource) = &done.pull_request {
        tx.execute(
            "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
            params![task, resource],
        )?;
    }
    let target = at
        .next
        .get(done.status)
        .cloned()
        .unwrap_or_else(|| format!("{}?{}", at.id, done.status));
    let context = done.error.as_deref().unwrap_or_default();
    enter(&tx, task, workflow, &target, None, context, workflows)?;
    tx.commit()
}
