//! Spend caps: what a task, a trigger, a lane or the factory may spend, per task, local day or
//! week, priced from `prices.toml` and the steps' token counts. See `docs/daemon/spend.md`.

use crate::trace::Trace;
use priory_factory::factory::{Factory, Limits, Price};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Task,
    Day,
    Week,
}

impl Period {
    fn name(self) -> &'static str {
        match self {
            Period::Task => "task",
            Period::Day => "day",
            Period::Week => "week",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Scope {
    Factory,
    Trigger(String),
    Lane(String),
}

/// One cap of the factory's files.
#[derive(Clone)]
pub struct Cap {
    pub scope: Scope,
    pub period: Period,
    pub usd: f64,
}

impl Cap {
    /// What the state stores the cap as, such as `lane:core/intake.day`.
    fn id(&self) -> String {
        let period = self.period.name();
        match &self.scope {
            Scope::Factory => format!("factory.{period}"),
            Scope::Trigger(name) => format!("trigger:{name}.{period}"),
            Scope::Lane(lane) => format!("lane:{lane}.{period}"),
        }
    }

    /// The scope as `priory costs` names it: `factory`, `trigger <name>` or `lane <lane>`.
    fn scope_name(&self) -> String {
        match &self.scope {
            Scope::Factory => "factory".to_owned(),
            Scope::Trigger(name) => format!("trigger {name}"),
            Scope::Lane(lane) => format!("lane {lane}"),
        }
    }

    /// Such as `the factory's task cap of $50.00` or `trigger agent-label's day cap of $10.00`.
    fn named(&self) -> String {
        let owner = match &self.scope {
            Scope::Factory => "the factory's".to_owned(),
            Scope::Trigger(name) => format!("trigger {name}'s"),
            Scope::Lane(lane) => format!("lane {lane}'s"),
        };
        format!("{owner} {} cap of {}", self.period.name(), usd(self.usd))
    }

    /// Whether the cap counts toward the task with `trigger` and `lane`.
    fn covers(&self, trigger: &str, lane: Option<&str>) -> bool {
        match &self.scope {
            Scope::Factory => true,
            Scope::Trigger(name) => name == trigger,
            Scope::Lane(name) => Some(name.as_str()) == lane,
        }
    }
}

fn usd(dollars: f64) -> String {
    format!("${dollars:.2}")
}

/// Every cap of the factory: its own, then each trigger's, then each lane's, each by task, day
/// and week.
pub fn caps(factory: &Factory) -> Vec<Cap> {
    let mut caps = Vec::new();
    let mut add = |scope: Scope, limits: &Limits| {
        for (period, usd) in [
            (Period::Task, limits.task),
            (Period::Day, limits.day),
            (Period::Week, limits.week),
        ] {
            if let Some(usd) = usd {
                caps.push(Cap {
                    scope: scope.clone(),
                    period,
                    usd,
                });
            }
        }
    };
    if let Some(own) = factory
        .settings
        .as_ref()
        .and_then(|settings| settings.value.caps.as_ref())
    {
        let limits = Limits {
            task: own.task,
            day: own.day,
            week: own.week,
        };
        add(Scope::Factory, &limits);
    }
    for (name, trigger) in &factory.triggers {
        if let Some(limits) = &trigger.value.caps {
            add(Scope::Trigger(name.clone()), limits);
        }
    }
    if let Some(own) = factory
        .settings
        .as_ref()
        .and_then(|settings| settings.value.caps.as_ref())
    {
        for (lane, limits) in &own.lanes {
            add(Scope::Lane(lane.clone()), limits);
        }
    }
    caps
}

/// What the spend of a task or a window is read with: the caps, the prices, and the model of each
/// model step's agent, for a step whose transcript named none.
#[derive(Clone, Default)]
pub struct Book {
    caps: Vec<Cap>,
    prices: BTreeMap<String, Price>,
    models: BTreeMap<(String, String), String>,
}

impl Book {
    pub fn new(factory: &Factory) -> Self {
        let mut models = BTreeMap::new();
        for (name, workflow) in &factory.workflows {
            for step in &workflow.value.steps {
                let model = step
                    .agent
                    .as_ref()
                    .and_then(|agent| factory.agents.get(agent))
                    .and_then(|agent| agent.value.model.clone());
                if let Some(model) = model {
                    models.insert((name.clone(), step.id.clone()), model);
                }
            }
        }
        Book {
            caps: caps(factory),
            prices: factory.prices.clone(),
            models,
        }
    }
}

/// The records a cost counts: model steps, debate turns, handler calls and summarizer calls, as
/// `priory costs`. A debate step's own record carries no tokens.
const PRICED: &str = "(trace.type IN ('factory.handler.called', 'factory.summarizer.called',
    'factory.debate.turn_ended')
  OR (trace.type = 'factory.step.ended' AND json_extract(trace.event, '$.data.decider') = 'model'))";

/// The rows a query of `(model, input, output, cache_read, cache_write)` returns, priced. A model
/// with no price counts nothing.
fn priced(
    db: &Connection,
    prices: &BTreeMap<String, Price>,
    sql: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<f64> {
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map(params, |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            [
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ],
        ))
    })?;
    let mut dollars = 0.0;
    for row in rows {
        let (model, tokens) = row?;
        if let Some(price) = model.and_then(|model| prices.get(&model)) {
            dollars += crate::report::dollars(&tokens, price);
        }
    }
    Ok(dollars)
}

const TOKENS: &str = "coalesce(json_extract(trace.event, '$.data.tokens.input'), 0),
  coalesce(json_extract(trace.event, '$.data.tokens.output'), 0),
  coalesce(json_extract(trace.event, '$.data.tokens.cache_read'), 0),
  coalesce(json_extract(trace.event, '$.data.tokens.cache_write'), 0)";

const OPEN_TOKENS: &str = "coalesce(json_extract(tasks.tokens, '$.input'), 0),
  coalesce(json_extract(tasks.tokens, '$.output'), 0),
  coalesce(json_extract(tasks.tokens, '$.cache_read'), 0),
  coalesce(json_extract(tasks.tokens, '$.cache_write'), 0)";

/// What `task` spent: its recorded steps and calls, and the tokens of the step it is at.
fn spent_by_task(db: &Connection, book: &Book, task: i64) -> rusqlite::Result<f64> {
    let recorded = priced(
        db,
        &book.prices,
        &format!(
            "SELECT json_extract(trace.event, '$.data.model'), {TOKENS} FROM trace
             WHERE CAST(json_extract(trace.event, '$.data.task') AS INTEGER) = ?1 AND {PRICED}"
        ),
        [task],
    )?;
    Ok(recorded + open_step(db, book, Some(task), None)?)
}

/// The priced tokens of the steps active tasks are at, of `task` alone or of those `cap` covers.
fn open_step(
    db: &Connection,
    book: &Book,
    task: Option<i64>,
    cap: Option<&Cap>,
) -> rusqlite::Result<f64> {
    let mut statement = db.prepare(&format!(
        "SELECT id, workflow, step, model, trigger, lane, {OPEN_TOKENS} FROM tasks
         WHERE tokens IS NOT NULL AND state NOT IN ('done', 'failed', 'dropped')
         AND (?1 IS NULL OR id = ?1)"
    ))?;
    let rows = statement.query_map([task], |row| {
        Ok((
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            [
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ],
        ))
    })?;
    let mut dollars = 0.0;
    for row in rows {
        let (workflow, step, model, trigger, lane, tokens) = row?;
        if cap.is_some_and(|cap| !cap.covers(&trigger, lane.as_deref())) {
            continue;
        }
        // The record of the step names the agent's model when the transcript named none.
        let model = model.or_else(|| book.models.get(&(workflow, step)).cloned());
        if let Some(price) = model.and_then(|model| book.prices.get(&model)) {
            dollars += crate::report::dollars(&tokens, price);
        }
    }
    Ok(dollars)
}

/// When the window of `period` that holds `now` started: the local day, or Monday 00:00 local.
fn window_start(db: &Connection, period: Period, now: &str) -> rusqlite::Result<String> {
    let sql = match period {
        Period::Week => {
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, 'localtime', 'start of day', '-6 days',
               'weekday 1', 'utc')"
        }
        _ => "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, 'localtime', 'start of day', 'utc')",
    };
    db.query_row(sql, [now], |row| row.get(0))
}

/// What the tasks `cap`, a day or week cap, covers spent in its window up to `now`: their records
/// in the window and the steps active tasks are at.
fn spent_in(db: &Connection, book: &Book, cap: &Cap, now: &str) -> rusqlite::Result<f64> {
    let since = window_start(db, cap.period, now)?;
    let (filter, value) = match &cap.scope {
        Scope::Factory => ("1", None),
        Scope::Trigger(name) => ("tasks.trigger IS ?3", Some(name.as_str())),
        Scope::Lane(lane) => ("tasks.lane IS ?3", Some(lane.as_str())),
    };
    let recorded = priced(
        db,
        &book.prices,
        &format!(
            "SELECT json_extract(trace.event, '$.data.model'), {TOKENS} FROM trace
             LEFT JOIN tasks ON tasks.id = CAST(json_extract(trace.event, '$.data.task') AS INTEGER)
             WHERE trace.time >= ?1 AND trace.time <= ?2 AND {PRICED} AND (?3 IS NULL OR {filter})"
        ),
        params![since, now, value],
    )?;
    Ok(recorded + open_step(db, book, None, Some(cap))?)
}

/// What a task may spend under `cap` before it is held: the cap, or what a person raised it to.
fn limit(db: &Connection, task: i64, cap: &Cap) -> rusqlite::Result<f64> {
    let raised: Option<f64> = db.query_row(
        "SELECT max(raised_to) FROM spend_holds WHERE task = ?1 AND cap = ?2",
        params![task, cap.id()],
        |row| row.get(0),
    )?;
    Ok(raised.map_or(cap.usd, |raised| raised.max(cap.usd)))
}

/// Why a day or week cap holds new runs: `spend cap: lane core/intake spent $12.00 of its day cap
/// of $10.00`.
fn reached_line(cap: &Cap, spent: f64) -> String {
    format!(
        "spend cap: {} spent {} of its {} cap of {}",
        cap.scope_name(),
        usd(spent),
        cap.period.name(),
        usd(cap.usd)
    )
}

/// A task the tick checks: its id, trigger, lane and state, and its live run and tmux session.
type Candidate = (i64, String, Option<String>, String, Option<(i64, String)>);

/// Holds each task past a cap, once its turn ended, with one ask to raise the cap or stop the task;
/// and records the day and week caps reached, which hold new runs. Returns the tmux sessions of
/// the runs it ended, to kill.
pub(crate) fn tick(db: &mut Connection, trace: &Trace, now: &str) -> rusqlite::Result<Vec<String>> {
    let factory = trace.factory;
    let book = Book::new(factory);
    let caps = &book.caps;
    let windows: Vec<(Cap, f64)> = caps
        .iter()
        .filter(|cap| cap.period != Period::Task)
        .map(|cap| Ok((cap.clone(), spent_in(db, &book, cap, now)?)))
        .collect::<rusqlite::Result<_>>()?;
    {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM spend_reached", [])?;
        for (cap, spent) in windows.iter().filter(|(cap, spent)| *spent >= cap.usd) {
            let until = match cap.period {
                Period::Week => "next week",
                _ => "the next day",
            };
            let line = format!(
                "{} reached: {} spent {} of its {} cap of {}; its new runs wait for {until}",
                "spend cap",
                cap.scope_name(),
                usd(*spent),
                cap.period.name(),
                usd(cap.usd)
            );
            tx.execute(
                "INSERT INTO spend_reached (cap, line) VALUES (?1, ?2)",
                params![cap.id(), line],
            )?;
        }
        tx.commit()?;
    }
    if caps.is_empty() {
        return Ok(Vec::new());
    }
    // A running task between turns, or one about to start.
    let candidates: Vec<Candidate> = db
        .prepare(
            "SELECT tasks.id, tasks.trigger, tasks.lane, tasks.state, runs.id, runs.tmux FROM tasks
             LEFT JOIN runs ON runs.task = tasks.id AND runs.ended IS NULL
             WHERE (tasks.state IN ('queued', 'ready')
               -- With several runners, only a task this runner may start, as the queue reads it.
               AND (?1 IS NULL OR tasks.trigger = ?2 OR EXISTS (SELECT 1 FROM claims
                 WHERE claims.key = tasks.key AND claims.runner = ?1)))
             OR (tasks.state = 'running' AND runs.ready AND runs.idle = 1 AND NOT EXISTS (
               SELECT 1 FROM outbox WHERE outbox.run = runs.id
               AND outbox.state IN ('attempted', 'uncertain')))
             ORDER BY tasks.id",
        )?
        .query_map(params![trace.runner, crate::tools::LEAD_TRIGGER], |row| {
            let run = match row.get::<_, Option<i64>>(4)? {
                Some(run) => Some((run, row.get(5)?)),
                None => None,
            };
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, run))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut killed = Vec::new();
    for (task, trigger, lane, state, run) in candidates {
        let running = state == "running";
        if !running && !at_model_step(db, factory, task)? {
            continue;
        }
        let mut crossed = None;
        for cap in caps
            .iter()
            .filter(|cap| cap.covers(&trigger, lane.as_deref()))
        {
            let spent = match cap.period {
                Period::Task => spent_by_task(db, &book, task)?,
                // A day or week cap holds only new runs of a lane, and of a task not yet running.
                _ if !running || matches!(cap.scope, Scope::Lane(_)) => continue,
                _ => windows
                    .iter()
                    .find(|(window, _)| window.id() == cap.id())
                    .map_or(0.0, |(_, spent)| *spent),
            };
            if spent >= limit(db, task, cap)? {
                crossed = Some((cap, spent));
                break;
            }
        }
        let Some((cap, spent)) = crossed else {
            continue;
        };
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((run, tmux)) = &run {
            crate::trace::account(&tx, trace, *run, None)?;
            tx.execute(
                &format!("UPDATE runs SET ended = {NOW}, ended_as = 'capped' WHERE id = ?1"),
                [run],
            )?;
            killed.push(tmux.clone());
        }
        hold(
            &tx,
            task,
            if running { "ready" } else { &state },
            cap,
            spent,
        )?;
        tx.commit()?;
    }
    Ok(killed)
}

/// Whether `task` is at a model step, which starts a run that spends.
fn at_model_step(db: &Connection, factory: &Factory, task: i64) -> rusqlite::Result<bool> {
    let (workflow, step): (String, String) = db.query_row(
        "SELECT workflow, step FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(factory
        .workflows
        .get(&workflow)
        .and_then(|flow| flow.value.step(&step))
        .is_some_and(|step| step.decides == priory_factory::factory::Decider::Model))
}

/// Holds `task`, which spent `spent` under `cap`, until a person answers its ask: `raise` or `stop`.
fn hold(tx: &Transaction, task: i64, resume: &str, cap: &Cap, spent: f64) -> rusqlite::Result<()> {
    let past = format!("spent {} of {}", usd(spent), cap.named());
    tx.execute(
        "INSERT INTO holds (task, reason, kind, resume) VALUES (?1, ?2, 'person', ?3)",
        params![task, format!("spend cap: {past}"), resume],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE tasks SET state = 'held', reason = ?2 WHERE id = ?1",
        params![
            task,
            format!("held: {past}, until a person raises the cap or stops the task")
        ],
    )?;
    let question = format!(
        "Task {task} {past}. Raise the cap for this task by {}, or stop the task?",
        usd(cap.usd)
    );
    tx.execute(
        "INSERT INTO asks (task, step, kind, question, options)
         SELECT id, step, 'agent', ?2, '[\"raise\",\"stop\"]' FROM tasks WHERE id = ?1",
        params![task, question],
    )?;
    tx.execute(
        "UPDATE holds SET ask = ?2 WHERE id = ?1",
        params![id, tx.last_insert_rowid()],
    )?;
    tx.execute(
        "INSERT INTO spend_holds (hold, task, cap, cap_usd, spent_usd) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, task, cap.id(), cap.usd, spent],
    )
    .map(drop)
}

/// Applies a person's answer to the ask of the spend hold `id`, if it is one: `raise` lets the task
/// spend one more cap from what it spent, and `stop` drops it. Returns false for another hold.
pub(crate) fn answered(tx: &Transaction, id: i64, answer: &str) -> rusqlite::Result<bool> {
    let held = tx
        .query_row(
            "SELECT spend_holds.task, holds.resume FROM spend_holds
             JOIN holds ON holds.id = spend_holds.hold WHERE spend_holds.hold = ?1",
            [id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((task, resume)) = held else {
        return Ok(false);
    };
    match answer {
        "raise" => {
            tx.execute(
                "UPDATE spend_holds SET raised_to = spent_usd + cap_usd WHERE hold = ?1",
                [id],
            )?;
            if resume == "ready" {
                crate::tools::queue_first(
                    tx,
                    task,
                    "A person raised the spend cap this task reached. Check the worktree, then continue.",
                )?;
            }
            crate::hold::meet(tx, id, "the cap was raised by a person")?;
        }
        _ => crate::hold::drop_task(tx, task, "stopped by a person at a spend cap")?,
    }
    Ok(true)
}

/// Why a day or week cap holds the queued or ready `task` from starting, if one does.
pub(crate) fn waits(db: &Connection, book: &Book, task: i64) -> rusqlite::Result<Option<String>> {
    let reached: Vec<String> = db
        .prepare("SELECT cap FROM spend_reached")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if reached.is_empty() {
        return Ok(None);
    }
    let (trigger, lane): (String, Option<String>) = db.query_row(
        "SELECT trigger, lane FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let now = db.query_row(&format!("SELECT {NOW}"), [], |row| row.get::<_, String>(0))?;
    for cap in book
        .caps
        .iter()
        .filter(|cap| cap.period != Period::Task && cap.covers(&trigger, lane.as_deref()))
        .filter(|cap| reached.contains(&cap.id()))
    {
        let spent = spent_in(db, book, cap, &now)?;
        if spent >= limit(db, task, cap)? {
            return Ok(Some(format!("waits: {}", reached_line(cap, spent))));
        }
    }
    Ok(None)
}

/// The desk's lines for the day and week caps reached now.
pub fn desk_lines(db: &Connection) -> rusqlite::Result<Vec<String>> {
    db.prepare("SELECT line FROM spend_reached ORDER BY cap")?
        .query_map([], |row| row.get(0))?
        .collect()
}

/// Each cap with what its window spent up to `now`, or, for a task cap, the most an active task
/// under it spent, as `priory costs` shows them.
pub fn report(db: &Connection, book: &Book, now: &str) -> rusqlite::Result<Vec<Value>> {
    let mut rows = Vec::new();
    let active: Vec<(i64, String, Option<String>)> = db
        .prepare(
            "SELECT id, trigger, lane FROM tasks WHERE state NOT IN ('done', 'failed', 'dropped')
             ORDER BY id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for cap in &book.caps {
        let (spent, task) = match cap.period {
            Period::Task => {
                let mut most: Option<(f64, i64)> = None;
                for (task, trigger, lane) in &active {
                    if !cap.covers(trigger, lane.as_deref()) {
                        continue;
                    }
                    let spent = spent_by_task(db, book, *task)?;
                    if most.is_none_or(|(top, _)| spent > top) {
                        most = Some((spent, *task));
                    }
                }
                (most.map(|(spent, _)| spent), most.map(|(_, task)| task))
            }
            _ => (Some(spent_in(db, book, cap, now)?), None),
        };
        rows.push(json!({
            "scope": cap.scope_name(),
            "period": cap.period.name(),
            "cap_usd": cap.usd,
            "spent_usd": spent,
            "task": task,
        }));
    }
    Ok(rows)
}

/// Each task held at a spend cap now, with the cap it reached and what it had spent, by task.
pub fn held(db: &Connection) -> rusqlite::Result<Vec<Value>> {
    db.prepare(
        "SELECT tasks.id, tasks.key, spend_holds.cap, spend_holds.cap_usd, spend_holds.spent_usd
         FROM spend_holds JOIN holds ON holds.id = spend_holds.hold
         JOIN tasks ON tasks.id = spend_holds.task
         WHERE tasks.state = 'held' AND holds.met IS NULL AND holds.released IS NULL
         ORDER BY tasks.id, holds.id",
    )?
    .query_map([], |row| {
        let cap: String = row.get(2)?;
        // The stored id, such as `lane:core/intake.day`, as `report` names its scope and period.
        let (scope, period) = cap.rsplit_once('.').unwrap_or((&cap, ""));
        Ok(json!({
            "task": row.get::<_, i64>(0)?,
            "key": row.get::<_, String>(1)?,
            "scope": scope.replacen(':', " ", 1),
            "period": period,
            "cap_usd": row.get::<_, f64>(3)?,
            "spent_usd": row.get::<_, f64>(4)?,
        }))
    })?
    .collect()
}
