//! Autopilot: for a while a person chose, the daemon answers the eligible asks with their
//! recommendation, through the code `auto_answer` uses, and accounts for each (docs/daemon/waits.md).

use crate::listing::rows;
use crate::runners::{self, Unsent};
use crate::tools::{self, AUTOPILOT, Answer};
use crate::trace::{self, Trace};
use herdr_bus::Message;
use jiff::{SignedDuration, Timestamp, civil, tz::TimeZone};
use priory_factory::explain;
use priory_factory::factory::{Confidence, Factory, Workflow};
use priory_intake::intake::AUTOPILOT_SET;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::Path;

/// The longest a person can leave autopilot on.
const LONGEST: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Reads the commit of a branch checked out in a worktree.
type Head<'a> = &'a dyn Fn(&Path, &str) -> Option<String>;

/// When autopilot ends: `span` from `now`, or at the time `at`, an RFC 3339 instant or the next
/// `HH:MM` in `zone`; the next 08:00 there with neither.
pub fn until(
    span: Option<&str>,
    at: Option<&str>,
    now: Timestamp,
    zone: TimeZone,
) -> Result<Timestamp, String> {
    let now_there = now.to_zoned(zone.clone());
    let end = match (span, at) {
        (Some(span), _) => span
            .parse::<jiff::Span>()
            .and_then(|span| now_there.checked_add(span))
            .map_err(|error| format!("{span}: {error}"))?
            .timestamp(),
        (None, at) => {
            let at = at.unwrap_or("08:00");
            match civil::Time::strptime("%H:%M", at) {
                Ok(time) => next(&now_there, time).map_err(|error| format!("{at}: {error}"))?,
                Err(_) => at.parse::<Timestamp>().map_err(|_| {
                    format!("{at} is not a time such as 08:00 or 2026-11-25T08:00:00+01:00")
                })?,
            }
        }
    };
    let end = Timestamp::from_second(end.as_second()).map_err(|error| error.to_string())?;
    if end <= now {
        return Err(format!("{end} is not in the future"));
    }
    if now.duration_until(end) > LONGEST {
        return Err(format!("{end} is more than 7 days away"));
    }
    Ok(end)
}

/// The first `time` of a day after `now`, in its zone.
fn next(now: &jiff::Zoned, time: civil::Time) -> Result<Timestamp, jiff::Error> {
    let today = now
        .date()
        .to_datetime(time)
        .to_zoned(now.time_zone().clone())?;
    let next = if today > *now {
        today
    } else {
        now.date()
            .tomorrow()?
            .to_datetime(time)
            .to_zoned(now.time_zone().clone())?
    };
    Ok(next.timestamp())
}

/// What `priory autopilot on` sets, or `off` without one.
pub struct Setting {
    pub until: Timestamp,
    pub level: Confidence,
    pub lanes: Vec<String>,
}

/// Turns autopilot on with `setting`, or off without one, in this state file. Returns the message
/// that tells the other runners.
pub fn set(
    db: &mut Connection,
    setting: Option<&Setting>,
    queued: bool,
) -> rusqlite::Result<Message> {
    let data = setting.map_or_else(
        || json!({}),
        |on| json!({ "until": on.until.to_string(), "level": on.level, "lanes": on.lanes }),
    );
    let message = Message::new(AUTOPILOT_SET, data).subject("autopilot");
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    apply(&tx, &message.id.to_string(), setting, Timestamp::now())?;
    // Queued with the change, so the log reads a runner's settings in the order it made them.
    if queued {
        runners::send(&tx, &message)?;
    }
    // An off waits for its echo even before `tell`, so no older `on` from the log undoes it.
    let awaited = queued || setting.is_none();
    tx.execute(
        "UPDATE autopilot_state SET unread = ?1",
        [awaited.then(|| message.id.to_string())],
    )?;
    tx.commit()?;
    Ok(message)
}

/// Tells the other runners the `off` that `message` is, as this machine's `role` in `factory` does.
pub fn tell(
    db: &mut Connection,
    factory: &Factory,
    role: Role,
    message: &Message,
) -> Result<(), String> {
    match role {
        Role::Viewer => publish(factory, message),
        Role::Runner => db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .and_then(|tx| {
                runners::send(&tx, message)?;
                tx.commit()
            })
            .map_err(|error| error.to_string()),
        // With no other runner, no echo comes.
        Role::Alone => db
            .execute("UPDATE autopilot_state SET unread = NULL", [])
            .map(drop)
            .map_err(|error| error.to_string()),
    }
}

/// Sends `message` to the runners from a machine that runs no task.
pub fn publish(factory: &Factory, message: &Message) -> Result<(), String> {
    runners::publish(factory.log(), message).map_err(|unsent| match unsent {
        Unsent::Nothing(why) | Unsent::Unconfirmed(why) => why,
    })
}

/// Whether this machine runs tasks of a factory of several runners, which take the setting from
/// its state file, or only views them, and so publishes it.
pub fn role(factory: &Factory, host: Option<&str>) -> Result<Role, String> {
    if factory.runners.len() < 2 {
        return Ok(Role::Alone);
    }
    match runners::runner(factory, host) {
        Ok(_) => Ok(Role::Runner),
        Err(_) => runners::viewer(factory, host).map(|_| Role::Viewer),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Role {
    Alone,
    Runner,
    Viewer,
}

/// Makes `setting` the window in force under the id `id`, ending the one before it.
fn apply(
    tx: &Transaction,
    id: &str,
    setting: Option<&Setting>,
    now: Timestamp,
) -> rusqlite::Result<()> {
    let Some(on) = setting else {
        tx.execute(
            "UPDATE autopilot_windows SET ended = ?1, why = 'off' WHERE ended IS NULL",
            [stamp(now)],
        )?;
        // An `on` read back from the log after a local `off` stays off.
        tx.execute(
            "UPDATE autopilot_windows SET why = 'off' WHERE why = 'replaced' AND until > ?1",
            [stamp(now)],
        )?;
        return Ok(());
    };
    let known = tx
        .query_row(
            "SELECT coalesce(why, 'on') FROM autopilot_windows WHERE setting = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if known.as_deref().is_some_and(|why| why != "replaced") || on.until <= now {
        return Ok(());
    }
    tx.execute(
        "UPDATE autopilot_windows SET ended = ?1, why = 'replaced' WHERE ended IS NULL",
        [stamp(now)],
    )?;
    tx.execute("UPDATE autopilot_state SET clock = ?1", [stamp(now)])?;
    tx.execute(
        "INSERT INTO autopilot_windows (setting, started, until, level, lanes)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (setting) DO UPDATE SET ended = NULL, why = NULL, reported = NULL",
        params![
            id,
            stamp(now),
            stamp(on.until),
            on.level.as_str(),
            json!(on.lanes).to_string()
        ],
    )?;
    Ok(())
}

/// As SQLite's `strftime('%Y-%m-%dT%H:%M:%SZ')` writes it, so the two compare as text.
fn stamp(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Applies an autopilot message read from the log, or leaves another message to routing. Refused in
/// a factory of one runner, whose state file alone turns it on, and when it is malformed.
pub(crate) fn handle(
    tx: &Transaction,
    message: &Message,
    several: bool,
) -> rusqlite::Result<Option<()>> {
    if message.kind != AUTOPILOT_SET {
        return Ok(None);
    }
    let now = Timestamp::now();
    let id = message.id.to_string();
    let unread: Option<String> =
        tx.query_row("SELECT unread FROM autopilot_state", [], |row| row.get(0))?;
    let echo = unread.as_ref() == Some(&id);
    if echo {
        tx.execute("UPDATE autopilot_state SET unread = NULL", [])?;
    }
    let outcome = match parse(&message.data, now) {
        Ok(setting) if several => {
            // This runner's own setting comes after an older one on the log, and wins.
            if echo || unread.is_none() {
                apply(tx, &id, setting.as_ref(), now)?;
            }
            "noted"
        }
        _ => "refused",
    };
    crate::state::record(tx, message, None, outcome, None)?;
    Ok(Some(()))
}

/// The setting a message carries: `None` to turn it off. Anyone can publish on the log, so a
/// malformed one is refused, never fatal.
fn parse(data: &Value, now: Timestamp) -> Result<Option<Setting>, ()> {
    let Some(fields) = data.as_object() else {
        return Err(());
    };
    if fields.is_empty() {
        return Ok(None);
    }
    let until: Timestamp = data["until"]
        .as_str()
        .and_then(|at| at.parse().ok())
        .ok_or(())?;
    let level = data["level"]
        .as_str()
        .and_then(Confidence::parse)
        .filter(|level| *level != Confidence::Low)
        .ok_or(())?;
    let lanes = data["lanes"]
        .as_array()
        .ok_or(())?
        .iter()
        .map(|lane| lane.as_str().map(str::to_owned).ok_or(()))
        .collect::<Result<Vec<_>, ()>>()?;
    if now.duration_until(until) > LONGEST {
        return Err(());
    }
    Ok(Some(Setting {
        until,
        level,
        lanes,
    }))
}

/// The time autopilot is on for, by its row's id.
pub(crate) struct Window {
    id: i64,
    pub level: Confidence,
    lanes: Vec<String>,
}

impl Window {
    fn covers(&self, lane: Option<&str>) -> bool {
        self.lanes.is_empty() || lane.is_some_and(|lane| self.lanes.iter().any(|on| on == lane))
    }
}

/// Whether the clock was set back by over a minute while autopilot was on, in SQL. The stored clock
/// is up to 10 seconds old and never moves back.
const SET_BACK: &str = "(SELECT clock FROM autopilot_state)
  > strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+50 seconds')";

/// The window in force now, if any. Its end is read here, so no restart or pause extends it.
pub(crate) fn window(db: &Connection) -> rusqlite::Result<Option<Window>> {
    db.query_row(
        &format!(
            "SELECT id, level, lanes FROM autopilot_windows
             WHERE ended IS NULL AND until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             AND NOT {SET_BACK}"
        ),
        [],
        |row| {
            Ok(Window {
                id: row.get(0)?,
                level: Confidence::parse(&row.get::<_, String>(1)?).unwrap_or(Confidence::High),
                lanes: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
            })
        },
    )
    .optional()
}

/// The window that answers an agent's new ask of `task` at step `at`, if it does.
pub(crate) fn answers(
    tx: &Transaction,
    workflow: &Workflow,
    task: i64,
    at: &str,
    command: bool,
    confidence: Option<Confidence>,
) -> rusqlite::Result<Option<Window>> {
    let Some(window) = window(tx)? else {
        return Ok(None);
    };
    let lane: Option<String> =
        tx.query_row("SELECT lane FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    let refused = explain::autopilot_refusal(workflow, at, command, window.level, confidence);
    let answers = window.covers(lane.as_deref()) && refused.is_none() && !held(tx, task)?;
    Ok(answers.then_some(window))
}

/// Whether autopilot may approve the plan `task` grades in `workflow`, so the critic reads it first.
pub(crate) fn approves_plans(
    tx: &Transaction,
    factory: &Factory,
    task: i64,
    workflow: &str,
) -> rusqlite::Result<bool> {
    if !factory.approve_plans().iter().any(|name| name == workflow) {
        return Ok(false);
    }
    let lane: Option<String> =
        tx.query_row("SELECT lane FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    Ok(window(tx)?.is_some_and(|window| window.covers(lane.as_deref())))
}

/// An open ask, as the tick judges it.
struct Open {
    id: i64,
    task: Option<i64>,
    kind: String,
    run: Option<i64>,
    step: String,
    revision: i64,
    options: Vec<String>,
    recommendation: Option<String>,
    command: bool,
    routed: Option<String>,
    confidence: Option<Confidence>,
    rationale: Option<String>,
    workflow: String,
    lane: Option<String>,
    grade: Option<String>,
    snoozed: bool,
}

fn open(db: &Connection, ask: i64) -> rusqlite::Result<Option<Open>> {
    db.query_row(
        "SELECT asks.id, asks.task, asks.kind, asks.run, coalesce(asks.step, ''), asks.revision,
           asks.options, asks.recommendation, asks.command IS NOT NULL, asks.routed, asks.confidence,
           asks.rationale, coalesce(tasks.workflow, ''), tasks.lane, tasks.grade,
           EXISTS (SELECT 1 FROM snoozes WHERE snoozes.kind = 'ask' AND snoozes.item = asks.id
             AND snoozes.back IS NULL AND snoozes.until > strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         FROM asks LEFT JOIN tasks ON tasks.id = asks.task
         WHERE asks.id = ?1 AND asks.answer IS NULL AND asks.withdrawn IS NULL
           AND coalesce(tasks.state, '') NOT IN ('done', 'failed', 'dropped')",
        [ask],
        |row| {
            Ok(Open {
                id: row.get(0)?,
                task: row.get(1)?,
                kind: row.get(2)?,
                run: row.get(3)?,
                step: row.get(4)?,
                revision: row.get(5)?,
                options: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
                recommendation: row.get(7)?,
                command: row.get(8)?,
                routed: row.get(9)?,
                confidence: row
                    .get::<_, Option<String>>(10)?
                    .as_deref()
                    .and_then(Confidence::parse),
                rationale: row.get(11)?,
                workflow: row.get(12)?,
                lane: row.get(13)?,
                grade: row.get(14)?,
                snoozed: row.get(15)?,
            })
        },
    )
    .optional()
}

/// Whether `task` is held, or is to be at the end of its turn: a person keeps it from going on.
fn held(tx: &Transaction, task: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM holds WHERE task = ?1 AND released IS NULL)",
        [task],
        |row| row.get(0),
    )
}

/// The option autopilot answers `ask` with and why it may, or why it leaves the ask.
fn verdict(
    tx: &Transaction,
    ask: &Open,
    window: &Window,
    factory: &Factory,
) -> rusqlite::Result<Result<(String, String), String>> {
    let Some(task) = ask.task else {
        return Ok(Err("it was asked outside Priory".to_owned()));
    };
    if !window.covers(ask.lane.as_deref()) {
        return Ok(Err("its lane is not covered".to_owned()));
    }
    if ask.snoozed {
        return Ok(Err("it is snoozed".to_owned()));
    }
    if let Some(lead) = &ask.routed {
        return Ok(Err(format!("it is with {lead}")));
    }
    if crate::hold::of_ask(tx, ask.id)?.is_some() || (ask.kind == "agent" && ask.run.is_none()) {
        return Ok(Err("Priory asked it, not an agent".to_owned()));
    }
    if held(tx, task)? {
        return Ok(Err("its task is held".to_owned()));
    }
    let Some(flow) = factory.workflows.get(&ask.workflow) else {
        return Ok(Err(format!(
            "its workflow {} is no longer in the factory",
            ask.workflow
        )));
    };
    if ask.kind == "step" {
        return plan(tx, ask, task, &flow.value, factory);
    }
    if let Some(why) = explain::autopilot_refusal(
        &flow.value,
        &ask.step,
        ask.command,
        window.level,
        ask.confidence,
    ) {
        return Ok(Err(why));
    }
    let Some(option) = ask
        .recommendation
        .clone()
        .filter(|option| ask.options.contains(option))
    else {
        return Ok(Err(
            "its recommendation is not one of its options".to_owned()
        ));
    };
    let why = format!(
        "its confidence, {}, meets {}, and {}",
        ask.confidence.map_or("none", Confidence::as_str),
        window.level.as_str(),
        explain::delegable_because(&flow.value, &ask.step)
    );
    Ok(Ok((option, why)))
}

/// As `verdict`, for a human step's ask: only a green plan of a workflow the factory names.
fn plan(
    tx: &Transaction,
    ask: &Open,
    task: i64,
    workflow: &Workflow,
    factory: &Factory,
) -> rusqlite::Result<Result<(String, String), String>> {
    let step = workflow.step(&ask.step);
    if step.is_some_and(|step| step.debate.is_some()) {
        return Ok(Err("it picks a debate's answer".to_owned()));
    }
    let graded = step.is_some_and(|step| step.decides == priory_factory::factory::Decider::Model);
    if !graded {
        return Ok(Err(
            match explain::undelegable(workflow, &ask.step, ask.command) {
                Some(why) => why,
                None => format!("a person decides at step {}", ask.step),
            },
        ));
    }
    if !factory.approve_plans().contains(&ask.workflow) {
        return Ok(Err(format!(
            "it approves a plan, and approve_plans does not name {}",
            ask.workflow
        )));
    }
    if let Some(why) = explain::plan_refusal(workflow, &ask.step) {
        return Ok(Err(why));
    }
    if ask.grade.as_deref() != Some("green") {
        return Ok(Err(format!(
            "its plan is graded {}, not green",
            ask.grade.as_deref().unwrap_or("nothing")
        )));
    }
    // A later revision shows a plan, or a word, that the critic never read.
    if ask.revision > 1 {
        return Ok(Err("its ask changed after its plan was graded".to_owned()));
    }
    let Some(critic) = factory.critic() else {
        return Ok(Err("the factory has no critic to read its plan".to_owned()));
    };
    let read: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM gates WHERE gates.task = ?1 AND gates.step = ?2
           AND gates.state = 'passed' AND gates.outcome = 'green'
           AND gates.run = (SELECT max(id) FROM runs WHERE runs.task = ?1 AND runs.step = ?2)
           AND EXISTS (SELECT 1 FROM json_each(gates.scorers) WHERE value = ?3))",
        params![task, ask.step, critic],
        |row| row.get(0),
    )?;
    if !read {
        return Ok(Err(format!("the critic {critic} did not read its plan")));
    }
    let why = format!(
        "its plan is green, approve_plans names {}, and {}",
        ask.workflow,
        explain::delegable_because(workflow, &ask.step)
    );
    Ok(Ok(("approved".to_owned(), why)))
}

/// Answers the open ask `id` as autopilot when `window` covers it, or notes why it is left.
pub(crate) fn judge(
    tx: &Transaction,
    id: i64,
    window: &Window,
    trace: &Trace,
    head: Head,
) -> rusqlite::Result<bool> {
    let Some(ask) = open(tx, id)? else {
        return Ok(false);
    };
    let (option, why) = match verdict(tx, &ask, window, trace.factory)? {
        Ok(answered) => answered,
        Err(why) => {
            note(tx, window, &ask, "left", None, &why)?;
            return Ok(false);
        }
    };
    let answer = Answer {
        ask: id,
        text: option.clone(),
        revision: ask.revision,
        run: false,
        by: AUTOPILOT.to_owned(),
    };
    // A batch's go and any other refusal leaves the ask to the person, with no refusal noted on it.
    if let Err(why) = tools::answer_in(tx, &answer, trace, head)? {
        note(tx, window, &ask, "left", None, &why)?;
        return Ok(false);
    }
    note(tx, window, &ask, "answered", Some(&option), &why)?;
    let mut fields = json!({
        "ask": id.to_string(),
        "revision": ask.revision,
        "option": option,
        "why": why,
        "level": window.level,
        "window": window.id,
        "run": ask.run.map(|run| run.to_string()),
    });
    match ask.kind.as_str() {
        "step" => fields["grade"] = json!(ask.grade),
        _ => {
            fields["confidence"] = json!(ask.confidence);
            fields["rationale"] = json!(ask.rationale);
        }
    }
    trace::autopilot(tx, trace, "factory.ask.autopilot_answered", id, fields)?;
    Ok(true)
}

/// Records what became of `ask` in `window`, writing only a change, which bumps the revision.
fn note(
    tx: &Transaction,
    window: &Window,
    ask: &Open,
    verdict: &str,
    option: Option<&str>,
    why: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO autopilot_asks (window, ask, revision, verdict, option, why)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (window, ask) DO UPDATE SET revision = excluded.revision,
           verdict = excluded.verdict, option = excluded.option, why = excluded.why,
           noted = excluded.noted
         WHERE (revision, verdict, option, why)
           IS NOT (excluded.revision, excluded.verdict, excluded.option, excluded.why)",
        params![window.id, ask.id, ask.revision, verdict, option, why],
    )?;
    Ok(())
}

/// Judges every open ask while autopilot is on, and posts one report line for each window that
/// ended.
pub(crate) fn tick(db: &mut Connection, trace: &Trace, head: Head) -> rusqlite::Result<()> {
    finish(db)?;
    if window(db)?.is_none() {
        return Ok(());
    }
    let asks = db
        .prepare(
            "SELECT asks.id FROM asks LEFT JOIN tasks ON tasks.id = asks.task
             WHERE asks.answer IS NULL AND asks.withdrawn IS NULL
               AND coalesce(tasks.state, '') NOT IN ('done', 'failed', 'dropped')
             ORDER BY asks.id",
        )?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for ask in asks {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Read again in the transaction: a person may have turned it off or answered meanwhile.
        if let Some(window) = window(&tx)? {
            judge(&tx, ask, &window, trace, head)?;
        }
        tx.commit()?;
    }
    Ok(())
}

/// Ends the window whose time passed or whose clock was set back, and reports each that ended.
fn finish(db: &mut Connection) -> rusqlite::Result<()> {
    // Read first: a write transaction on each tick would hold up every other writer.
    let due: bool = db.query_row(
        &format!(
            "SELECT EXISTS (SELECT 1 FROM autopilot_windows WHERE reported IS NULL AND (ended IS NOT NULL
               OR until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now') OR {SET_BACK}
               OR (SELECT clock FROM autopilot_state)
                 < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 seconds')))"
        ),
        [],
        |row| row.get(0),
    )?;
    if !due {
        return Ok(());
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE autopilot_windows SET ended = until, why = 'time'
         WHERE ended IS NULL AND until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        [],
    )?;
    // Its end would come later than the person meant.
    tx.execute(
        &format!(
            "UPDATE autopilot_windows SET ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), why = 'clock'
             WHERE ended IS NULL AND {SET_BACK}"
        ),
        [],
    )?;
    tx.execute(
        "UPDATE autopilot_state SET clock = max(clock, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        [],
    )?;
    let ended = tx
        .prepare(
            "UPDATE autopilot_windows SET reported = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE ended IS NOT NULL AND reported IS NULL RETURNING id",
        )?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for window in ended {
        let (answered, left) = counts(&tx, window)?;
        let line = format!(
            "autopilot ended: it answered {answered} and left {left}. See `priory autopilot report {window}`"
        );
        crate::post::report(&tx, "autopilot", &line)
            .map_err(|why| rusqlite::Error::ToSqlConversionFailure(why.into()))?;
    }
    tx.commit()
}

fn counts(db: &Connection, window: i64) -> rusqlite::Result<(i64, i64)> {
    db.query_row(
        "SELECT count(*) FILTER (WHERE verdict = 'answered'), count(*) FILTER (WHERE verdict = 'left')
         FROM autopilot_asks WHERE window = ?1",
        [window],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// The window in force as the desk and `priory autopilot status` show it, or null.
pub fn status(db: &Connection) -> rusqlite::Result<Value> {
    let Some(window) = window(db)? else {
        return Ok(Value::Null);
    };
    let (answered, left) = counts(db, window.id)?;
    let (started, until): (String, String) = db.query_row(
        "SELECT started, until FROM autopilot_windows WHERE id = ?1",
        [window.id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(json!({
        "window": window.id, "since": started, "until": until, "level": window.level,
        "lanes": window.lanes, "answered": answered, "left": left,
    }))
}

/// `status` as one line.
pub fn line(status: &Value) -> Option<String> {
    let lanes: Vec<&str> = status["lanes"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let lanes = match lanes.is_empty() {
        true => "every lane".to_owned(),
        false => lanes.join(", "),
    };
    Some(format!(
        "autopilot is on until {} ({}, {lanes}): answered {}, left {}. Turn it off with `priory autopilot off`",
        status["until"].as_str()?,
        status["level"].as_str()?,
        status["answered"],
        status["left"],
    ))
}

/// What window `id`, the latest without one, answered and left, why, and how to reverse each, in
/// `factory` when it is known. `None` when there is no such window.
pub fn report(
    db: &Connection,
    id: Option<i64>,
    factory: Option<&Factory>,
) -> rusqlite::Result<Option<Value>> {
    let found = rows(
        db,
        "SELECT id AS window, started AS since, until, level, lanes, coalesce(ended,
           CASE WHEN until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now') THEN until END) AS ended,
           coalesce(why, CASE WHEN until <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now') THEN 'time' END) AS why
         FROM autopilot_windows WHERE ?1 IS NULL OR id = ?1 ORDER BY id DESC LIMIT 1",
        [id],
    )?;
    let Some(mut report) = found.as_array().and_then(|rows| rows.first()).cloned() else {
        return Ok(None);
    };
    if let Some(lanes) = report["lanes"].as_str() {
        report["lanes"] = serde_json::from_str(lanes).unwrap_or(Value::Null);
    }
    let noted = rows(
        db,
        "SELECT coalesce(asks.name, asks.id) AS ask, asks.task, noted.revision, asks.step, asks.question,
           noted.verdict, noted.option, noted.why, noted.noted AS at, asks.kind,
           CASE WHEN asks.withdrawn IS NOT NULL THEN 'withdrawn' WHEN asks.answer IS NOT NULL
             THEN 'answered' ELSE 'open' END AS state, asks.answer, asks.answered_by,
           tasks.state AS task_state, tasks.key, tasks.workflow, EXISTS (SELECT 1 FROM outbox WHERE outbox.task = asks.task
             AND outbox.state = 'queued'
             AND instr(outbox.text, 'Answer to ask ' || asks.id || ' (revision ') > 0) AS unsent
         FROM autopilot_asks AS noted JOIN asks ON asks.id = noted.ask
         LEFT JOIN tasks ON tasks.id = asks.task
         WHERE noted.window = ?1 ORDER BY noted.ask",
        [report["window"].as_i64()],
    )?;
    let (mut answered, mut left) = (Vec::new(), Vec::new());
    for mut ask in noted.as_array().cloned().unwrap_or_default() {
        match reverse(db, &ask, factory)? {
            Some(Ok(command)) => ask["reverse"] = json!(command),
            Some(Err(why)) => {
                ask["reverse"] = Value::Null;
                ask["no_reverse"] = json!(why);
            }
            None => ask["reverse"] = Value::Null,
        }
        let fields = ask.as_object_mut().expect("rows are objects");
        fields.remove("unsent");
        fields.remove("key");
        fields.remove("workflow");
        match fields
            .remove("verdict")
            .and_then(|it| it.as_str().map(str::to_owned))
        {
            Some(verdict) if verdict == "answered" => answered.push(ask),
            _ => left.push(ask),
        }
    }
    let window = (
        report["since"].as_str().unwrap_or_default().to_owned(),
        report["ended"].as_str().map(str::to_owned),
    );
    for ask in runners::remote_rows(db, None)? {
        remote(db, &ask, &window, &mut answered, &mut left)?;
    }
    report["answered"] = json!(answered);
    report["left"] = json!(left);
    Ok(Some(report))
}

/// Adds another runner's `ask` as its last post shows it, when autopilot answered it between the
/// window's start and end, or left it open.
fn remote(
    db: &Connection,
    ask: &Value,
    (since, ended): &(String, Option<String>),
    answered: &mut Vec<Value>,
    left: &mut Vec<Value>,
) -> rusqlite::Result<()> {
    let mut row = json!({
        "ask": ask["id"], "task": ask["task"], "revision": ask["revision"], "step": ask["step"],
        "question": ask["question"], "state": ask["state"], "answer": ask["answer"],
        "answered_by": ask["answered_by"], "kind": ask["kind"], "runner": ask["runner"],
    });
    let at = ask["answered_at"].as_str().unwrap_or_default();
    let during = at >= since.as_str() && ended.as_deref().is_none_or(|ended| at <= ended);
    if ask["answered_by"] == AUTOPILOT && during {
        row["option"] = ask["answer"].clone();
        row["why"] = ask["autopilot_why"].clone();
        row["at"] = ask["answered_at"].clone();
        let runner = ask["runner"].as_str().unwrap_or_default();
        let key = ask["key"].as_str().unwrap_or_default();
        let over: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM claims WHERE key = ?1 AND runner = ?2
               AND coalesce(published, hold) = 'ended')",
            [key, runner],
            |row| row.get(0),
        )?;
        row["reverse"] = match over {
            true => Value::Null,
            false => json!(format!("priory say {runner}:{key} TEXT")),
        };
        if over {
            row["no_reverse"] = json!(format!("its task ended on {runner}: see it there"));
        }
        answered.push(row);
    } else if ask["state"] == "open" && !ask["autopilot_left"].is_null() {
        row["why"] = ask["autopilot_left"].clone();
        row["reverse"] = json!(null);
        left.push(row);
    }
    Ok(())
}

/// The command that reverses an answer autopilot gave, or why none does: another answer while its
/// prompt is unsent, else word to the agent, or for a plan a cancel and a revive at the graded step.
fn reverse(
    db: &Connection,
    ask: &Value,
    factory: Option<&Factory>,
) -> rusqlite::Result<Option<Result<String, String>>> {
    if ask["verdict"] != "answered" || ask["answered_by"] != AUTOPILOT {
        return Ok(None);
    }
    let (Some(task), Some(step), Some(state)) = (
        ask["task"].as_i64(),
        ask["step"].as_str(),
        ask["task_state"].as_str(),
    ) else {
        return Ok(None);
    };
    let tx = db.unchecked_transaction()?;
    let over = ["done", "failed", "dropped"].contains(&state);
    if over && let Some(rival) = tools::rival(&tx, task, ask["key"].as_str().unwrap_or_default())? {
        return Ok(Some(Err(format!("task {task} is {state}, and {rival}"))));
    }
    let workflow = ask["workflow"].as_str().unwrap_or_default();
    let revives = state != "done" && (over || ask["kind"] == "step");
    if let Some(factory) = factory
        && revives
        && factory
            .workflows
            .get(workflow)
            .is_none_or(|flow| flow.value.step(step).is_none())
    {
        return Ok(Some(Err(format!(
            "task {task} cannot revive at step {step}, which is no longer in workflow {workflow}"
        ))));
    }
    Ok(Some(match (state, ask["kind"] == "step") {
        ("done", _) => match tools::says_reopens(&tx, task, factory)? {
            Ok(()) => Ok(format!("priory say {task} TEXT")),
            Err(why) => Err(format!(
                "task {task} is done and {why}: ask for the change in a new task, with `priory add TEXT`"
            )),
        },
        _ if over => Ok(format!("priory revive {task} --step {step}")),
        (_, true) => Ok(format!(
            "priory cancel {task} && priory revive {task} --step {step}"
        )),
        (_, false) if ask["unsent"] == 1 => Ok(format!(
            "priory answer --revision {} {} OPTION",
            ask["revision"], ask["ask"]
        )),
        (_, false) => Ok(format!("priory say {task} TEXT")),
    }))
}

/// Prints `report` as `priory autopilot report` does.
pub fn print(report: &Value) {
    let text = |value: &Value| match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.replace(['\n', '\t'], " "),
        value => value.to_string(),
    };
    let ended = match report["ended"].as_str() {
        Some(ended) => format!("to {ended}"),
        None => format!("on until {}", text(&report["until"])),
    };
    crate::state::emit(&format!(
        "autopilot {} from {} {ended}, at {}",
        report["window"],
        text(&report["since"]),
        text(&report["level"])
    ));
    for (title, key) in [("Answered", "answered"), ("Left", "left")] {
        let asks = report[key].as_array().cloned().unwrap_or_default();
        crate::state::emit(&format!("\n{title}: {}", asks.len()));
        for ask in asks {
            let answer = match key {
                "answered" => format!(" -> {}", text(&ask["option"])),
                _ => String::new(),
            };
            crate::state::emit(&format!(
                "ask {} of task {}: {}{answer}",
                text(&ask["ask"]),
                text(&ask["task"]),
                text(&ask["question"])
            ));
            crate::state::emit(&format!("  why: {}", text(&ask["why"])));
            if let Some(why) = ask["no_reverse"].as_str() {
                crate::state::emit(&format!("  reverse: none. {why}"));
                continue;
            }
            match (&ask["reverse"], ask["state"].as_str()) {
                (Value::String(command), _) => crate::state::emit(&format!("  reverse: {command}")),
                (_, Some("answered")) => crate::state::emit(&format!(
                    "  now: answered {} by {}",
                    text(&ask["answer"]),
                    text(&ask["answered_by"])
                )),
                (_, Some(state)) => crate::state::emit(&format!("  now: {state}")),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn paris() -> TimeZone {
        TimeZone::get("Europe/Paris").unwrap()
    }

    #[test]
    fn the_end_is_the_next_eight_in_the_zone_a_clock_time_a_span_or_an_instant() {
        let night = at("2026-11-24T20:00:00Z");
        let end = |span, when| until(span, when, night, paris()).map(|end| end.to_string());
        assert_eq!(end(None, None).unwrap(), "2026-11-25T07:00:00Z");
        assert_eq!(end(None, Some("21:30")).unwrap(), "2026-11-24T20:30:00Z");
        assert_eq!(end(None, Some("20:59")).unwrap(), "2026-11-25T19:59:00Z");
        assert_eq!(end(Some("90m"), None).unwrap(), "2026-11-24T21:30:00Z");
        assert_eq!(
            end(None, Some("2026-11-25T06:00:00+01:00")).unwrap(),
            "2026-11-25T05:00:00Z"
        );
        assert!(end(Some("8 days"), None).unwrap_err().contains("7 days"));
        assert!(end(Some("-1h"), None).unwrap_err().contains("future"));
        assert!(end(None, Some("soon")).is_err());
    }

    #[test]
    fn a_message_that_is_malformed_low_or_too_long_sets_nothing() {
        let now = at("2026-11-24T20:00:00Z");
        let on = json!({ "until": "2026-11-25T07:00:00Z", "level": "high", "lanes": [] });
        assert!(matches!(parse(&on, now), Ok(Some(_))));
        assert!(matches!(parse(&json!({}), now), Ok(None)));
        for (field, value) in [
            ("until", json!("2026-12-25T07:00:00Z")),
            ("until", json!(7)),
            ("level", json!("low")),
            ("lanes", json!(["core", 3])),
            ("lanes", json!(null)),
        ] {
            let mut bad = on.clone();
            bad[field] = value;
            assert!(parse(&bad, now).is_err(), "{bad}");
        }
    }

    proptest! {
        #[test]
        fn until_never_panics(span in proptest::option::of(".{0,40}"), when in proptest::option::of(".{0,40}")) {
            let _ = until(span.as_deref(), when.as_deref(), at("2026-11-24T20:00:00Z"), paris());
        }
    }
}
