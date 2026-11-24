//! A person's own session asks for a compaction through `priory mcp --outside`. Once the session's
//! `Stop` hook fires, the factory's `[asks] type` command types `/compact`, and its follow-up once
//! the compaction's `SessionStart` fires. A text is never typed twice.

use crate::enter::Entering;
use crate::outside::Call;
use crate::screen;
use crate::state::Signal;
use crate::trace::{self, Trace};
use priory_factory::factory::Factory;
use priory_runtime::script::{self, Script};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::Duration;

const FOCUS_LIMIT: usize = 1000;
const THEN_LIMIT: usize = 4000;
/// Keeps `then` from running as a `!`, `/` or `#` line, and says whose words it holds.
pub const PREFIX: &str =
    "Priory compacted this session, as it asked. The session's own note, not a person's words: ";
/// How long `check` may take: the calling tool waits `runtime::REPLY` for it.
const CHECK: Duration = Duration::from_secs(5);
/// How long `send` and `enter` may take: `hc send` alone waits up to a minute for a person's draft.
const TYPE: Duration = Duration::from_secs(180);
/// The exit of a `type` command that typed nothing and may be tried again.
const BUSY: &str = "exit 75";
const OPEN: &str = "state NOT IN ('done', 'failed')";
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
const UNREACHED: &str = "Priory cannot type into this session";
const PERSON: &str = "a person typed first";

/// A checked request of the `compact` tool.
struct Request {
    session: String,
    focus: String,
    then: String,
}

/// The `compact` tool's arguments as one request, or why it is refused.
fn read(call: &Call, factory: &Factory) -> Result<Request, String> {
    let (focus, then) = texts(&call.arguments, &screen::secrets(&factory.secret_envs()))?;
    let Some(session) = &call.session else {
        return Err(format!("{UNREACHED}: it runs in no hc terminal"));
    };
    if session.split_once(' ').is_none() {
        return Err(format!("{UNREACHED}: it names no Claude session"));
    }
    if factory.typing().is_none() {
        return Err(format!(
            "{UNREACHED}: the factory names no [asks] type command"
        ));
    }
    Ok(Request {
        session: session.clone(),
        focus,
        then,
    })
}

/// The focus and the follow-up, within their limits, with no secret and no control character.
fn texts(arguments: &Value, secrets: &[String]) -> Result<(String, String), String> {
    let fields = match arguments {
        Value::Object(fields) => fields,
        _ => return Err("compact takes an object of fields".to_owned()),
    };
    if let Some(unknown) = fields.keys().find(|key| *key != "focus" && *key != "then") {
        return Err(format!(
            "unknown field {unknown}: compact takes then and focus"
        ));
    }
    let then = match fields.get("then") {
        Some(Value::String(then)) if !then.trim().is_empty() => then.trim().to_owned(),
        _ => return Err("then must be a line of text: what to do after the compaction".to_owned()),
    };
    let focus = match fields.get("focus") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(focus)) => focus.split_whitespace().collect::<Vec<_>>().join(" "),
        _ => return Err("focus must be a string".to_owned()),
    };
    for (name, text, limit) in [("focus", &focus, FOCUS_LIMIT), ("then", &then, THEN_LIMIT)] {
        if text.chars().count() > limit {
            return Err(format!("{name} is longer than {limit} characters"));
        }
        if text
            .chars()
            .any(|char| char.is_control() && char != '\n' && char != '\t')
        {
            return Err(format!("{name} holds a control character"));
        }
        if screen::holds_any_secret(text, secrets) {
            return Err(format!(
                "{name} holds a secret: call compact again without it"
            ));
        }
    }
    Ok((focus, then))
}

enum Purpose {
    /// The tool's call waits for the reply.
    Check(Request, Sender<String>),
    Send(i64),
    Enter(i64),
}

/// The `type` commands that run.
#[derive(Default)]
pub(crate) struct Typing(Vec<(Purpose, Script)>);

fn start(
    factory: &Factory,
    home: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Script, String> {
    let typing = factory
        .typing()
        .ok_or("the factory names no [asks] type command")?;
    let mut command = Command::new(typing);
    command.args(args);
    let log = home
        .join("compactions")
        .join(format!("{}.log", uuid::Uuid::new_v4()));
    script::spawn_command(command, &factory.dir, &log, timeout)
}

fn line(reply: Result<String, String>) -> String {
    serde_json::to_string(&reply).expect("a result serializes")
}

/// Starts the check of a `compact` call, whose reply waits for it, or refuses the call.
pub(crate) fn call(
    db: &mut Connection,
    call: &Call,
    trace: &Trace,
    home: &Path,
    running: &mut Typing,
    reply: Sender<String>,
) -> rusqlite::Result<()> {
    let checked = read(call, trace.factory).and_then(|request| {
        start(trace.factory, home, &["check", &request.session], CHECK)
            .map(|script| (request, script))
            .map_err(|error| format!("{UNREACHED}: {error}"))
    });
    match checked {
        Ok((request, script)) => running.0.push((Purpose::Check(request, reply), script)),
        Err(why) => {
            refuse(db, trace, call.session.as_deref(), &why)?;
            let _ = reply.send(line(Err(why)));
        }
    }
    Ok(())
}

fn refuse(
    db: &mut Connection,
    trace: &Trace,
    session: Option<&str>,
    why: &str,
) -> rusqlite::Result<()> {
    let Some(session) = session else {
        return Ok(());
    };
    let tx = db.transaction()?;
    trace::compaction(&tx, trace, "failed", session, None, Some(why))?;
    tx.commit()
}

/// Stores a request whose check passed, unless the session has an open one.
fn store(
    db: &mut Connection,
    trace: &Trace,
    request: &Request,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let claude = request.session.split_once(' ').map(|(_, claude)| claude);
    let open: bool = tx.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM outside_compactions WHERE claude = ?1 AND {OPEN})"),
        [claude],
        |row| row.get(0),
    )?;
    if open {
        return Ok(Err(
            "a compact is already queued for this session; end your turn".to_owned(),
        ));
    }
    tx.execute(
        "INSERT INTO outside_compactions (session, claude, focus, follow_up) VALUES (?1, ?2, ?3, ?4)",
        params![request.session, claude, request.focus, request.then],
    )?;
    step(&tx, trace, tx.last_insert_rowid(), "waiting", None)?;
    tx.commit()?;
    let text = command(&request.focus);
    Ok(Ok(format!(
        "Queued {text}. End your turn now: Priory types it once this session is idle, then your note. It gives up after 1 hour without the plugin's Stop hook."
    )))
}

fn command(focus: &str) -> String {
    format!("/compact {focus}").trim_end().to_owned()
}

/// Moves request `id` to `state`, and records the states a reader of the trace needs.
fn step(
    tx: &Transaction,
    trace: &Trace,
    id: i64,
    state: &str,
    reason: Option<&str>,
) -> rusqlite::Result<()> {
    tx.execute(
        &format!(
            "UPDATE outside_compactions SET state = ?2, reason = ?3, interrupted = 0, changed = {NOW}
             WHERE id = ?1"
        ),
        params![id, state, reason],
    )?;
    let kind = match state {
        "waiting" => "requested",
        "typed" => "typed",
        "compacted" => "compacted",
        "done" => "prompted",
        "failed" => "failed",
        _ => return Ok(()),
    };
    let (session, focus, then): (String, String, String) = tx.query_row(
        "SELECT session, focus, follow_up FROM outside_compactions WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let sizes = (focus.chars().count(), then.chars().count());
    trace::compaction(tx, trace, kind, &session, Some(sizes), reason)
}

/// Sets `state` with no record: a retry, or a step the trace does not name.
fn set(tx: &Transaction, id: i64, state: &str) -> rusqlite::Result<()> {
    tx.execute(
        &format!(
            "UPDATE outside_compactions SET state = ?2, interrupted = 0, changed = {NOW} WHERE id = ?1"
        ),
        params![id, state],
    )
    .map(drop)
}

/// The hook line of a person's own session: `{"outside_signal": {"event": ..., "session": ...}}`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub outside_signal: Signal,
}

/// Applies a hook of a person's own session to its open request, if it has one.
pub(crate) fn signal(db: &mut Connection, signal: &Signal, trace: &Trace) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found: Option<(i64, String, String)> = tx
        .query_row(
            &format!(
                "SELECT id, state, follow_up FROM outside_compactions WHERE claude = ?1 AND {OPEN}"
            ),
            [&signal.session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((id, state, follow_up)) = found else {
        return Ok(());
    };
    let compacted = signal.source.as_deref() == Some("compact");
    match (signal.event.as_str(), state.as_str()) {
        ("Stop", "waiting") => set(&tx, id, "due")?,
        // The person's turn ended while a busy `send` still waits to type.
        ("Stop", "typing") => {
            tx.execute(
                "UPDATE outside_compactions SET interrupted = 0 WHERE id = ?1",
                [id],
            )?;
        }
        ("SessionStart", "typing") if compacted => {
            step(&tx, trace, id, "typed", None)?;
            step(&tx, trace, id, "compacted", None)?;
        }
        ("SessionStart", "typed") if compacted => step(&tx, trace, id, "compacted", None)?,
        ("UserPromptSubmit", "due") => set(&tx, id, "waiting")?,
        ("UserPromptSubmit", "prompting" | "prompted")
            if signal.prompt.as_deref().map(str::trim) == Some(note(&follow_up).trim()) =>
        {
            step(&tx, trace, id, "done", None)?;
        }
        // Its command still runs, and only its exit says whether the text was typed.
        ("UserPromptSubmit", "typing" | "prompting") => {
            tx.execute(
                "UPDATE outside_compactions SET interrupted = 1 WHERE id = ?1",
                [id],
            )?;
        }
        ("UserPromptSubmit", "typed" | "compacted" | "prompted") => {
            step(&tx, trace, id, "failed", Some(PERSON))?;
        }
        _ => {}
    }
    tx.commit()
}

fn note(follow_up: &str) -> String {
    format!("{PREFIX}{follow_up}")
}

/// Whether the typed follow-up of request `id` still waits for its `UserPromptSubmit`.
pub(crate) fn unsubmitted(db: &Connection, id: i64) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM outside_compactions WHERE id = ?1 AND state = 'prompted')",
        [id],
        |row| row.get(0),
    )
}

/// Fails the requests whose command a stopped daemon left running: its text may have been typed.
pub(crate) fn adopt(db: &mut Connection, trace: &Trace) -> rusqlite::Result<()> {
    fail_where(
        db,
        trace,
        "state IN ('typing', 'prompting')",
        "a daemon restart interrupted its command, which may have typed",
    )
}

fn fail_where(db: &mut Connection, trace: &Trace, which: &str, why: &str) -> rusqlite::Result<()> {
    let ids: Vec<i64> = db
        .prepare(&format!(
            "SELECT id FROM outside_compactions WHERE {OPEN} AND {which}"
        ))?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in ids {
        let tx = db.transaction()?;
        step(&tx, trace, id, "failed", Some(why))?;
        tx.commit()?;
    }
    Ok(())
}

/// Settles the `type` commands that ended, fails the requests that waited too long, presses the
/// Enters that are due, and types the next text of each session that is ready for it.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    running: &mut Typing,
    entering: &mut Entering,
) -> rusqlite::Result<()> {
    for (purpose, mut script) in std::mem::take(&mut running.0) {
        match (script.poll(), purpose) {
            (None, purpose) => running.0.push((purpose, script)),
            (Some((status, output)), Purpose::Check(request, reply)) => {
                let result = if status == "exit 0" {
                    store(db, trace, &request)?
                } else {
                    Err(format!("{UNREACHED}: {}", said(&status, &output)))
                };
                if let Err(why) = &result {
                    refuse(db, trace, Some(&request.session), why)?;
                }
                let _ = reply.send(line(result));
            }
            (Some((status, output)), Purpose::Send(id)) => {
                sent(db, trace, id, &status, &output, entering)?;
            }
            (Some((status, output)), Purpose::Enter(id)) if status != "exit 0" => {
                eprintln!(
                    "priory daemon: pressing Enter for compaction {id}: {}",
                    said(&status, &output)
                );
            }
            (Some(_), Purpose::Enter(_)) => {}
        }
    }
    for (which, why) in [
        (
            "state IN ('waiting', 'due') AND julianday(requested) <= julianday('now', '-1 hours')",
            "the session was not idle within 1 hour",
        ),
        (
            "state = 'typed' AND julianday(changed) <= julianday('now', '-10 minutes')",
            "no compaction ended within 10 minutes of its /compact",
        ),
        (
            "state = 'compacted' AND julianday(requested) <= julianday('now', '-2 hours')",
            "the session was busy after its compaction",
        ),
        // A restart loses the Enter that would have settled it sooner.
        (
            "state = 'prompted' AND julianday(changed) <= julianday('now', '-10 minutes')",
            UNCONFIRMED,
        ),
    ] {
        fail_where(db, trace, which, why)?;
    }
    for id in entering.unconfirmed() {
        let tx = db.transaction()?;
        step(&tx, trace, id, "failed", Some(UNCONFIRMED))?;
        tx.commit()?;
    }
    for id in entering.enters() {
        let session: String = db.query_row(
            "SELECT session FROM outside_compactions WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        match start(trace.factory, home, &["enter", &session], TYPE) {
            Ok(script) => running.0.push((Purpose::Enter(id), script)),
            Err(error) => eprintln!("priory daemon: pressing Enter for compaction {id}: {error}"),
        }
    }
    send(db, trace, home, running)
}

const UNCONFIRMED: &str = "the typed follow-up was not confirmed by a UserPromptSubmit";

fn said(status: &str, output: &str) -> String {
    match output {
        "" => status.to_owned(),
        output => format!("{status}: {output}"),
    }
}

/// Starts typing the next text of each session that settled a second ago, or whose last try was
/// busy a second ago.
fn send(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    running: &mut Typing,
) -> rusqlite::Result<()> {
    type Next = (i64, String, String, String, String);
    let next: Vec<Next> = db
        .prepare(
            "SELECT id, session, state, focus, follow_up FROM outside_compactions
             WHERE state IN ('due', 'compacted')
               AND julianday(changed) <= julianday('now', '-1 seconds') ORDER BY id",
        )?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, session, state, focus, follow_up) in next {
        // A compaction that ended before its `send` did: the follow-up waits for that command.
        if running
            .0
            .iter()
            .any(|(purpose, _)| matches!(purpose, Purpose::Send(other) if *other == id))
        {
            continue;
        }
        let (typing, text) = match state.as_str() {
            "due" => ("typing", command(&focus)),
            _ => ("prompting", note(&follow_up)),
        };
        // Committed before the command starts, so a crash never types the text twice.
        let tx = db.transaction()?;
        set(&tx, id, typing)?;
        tx.commit()?;
        match start(trace.factory, home, &["send", &session, &text], TYPE) {
            Ok(script) => running.0.push((Purpose::Send(id), script)),
            Err(error) => {
                let tx = db.transaction()?;
                step(&tx, trace, id, "failed", Some(&error))?;
                tx.commit()?;
            }
        }
    }
    Ok(())
}

/// Records how the `send` of request `id` ended.
fn sent(
    db: &mut Connection,
    trace: &Trace,
    id: i64,
    status: &str,
    output: &str,
    entering: &mut Entering,
) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    let (state, interrupted): (String, bool) = tx.query_row(
        "SELECT state, interrupted FROM outside_compactions WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    match (state.as_str(), status, interrupted) {
        ("typing", BUSY, true) => set(&tx, id, "waiting")?,
        ("typing", BUSY, false) => set(&tx, id, "due")?,
        ("prompting", BUSY, false) => set(&tx, id, "compacted")?,
        ("typing" | "prompting", BUSY | "exit 0", true) => {
            step(&tx, trace, id, "failed", Some(PERSON))?;
        }
        ("typing", "exit 0", false) => step(&tx, trace, id, "typed", None)?,
        ("prompting", "exit 0", false) => {
            set(&tx, id, "prompted")?;
            entering.sent(id);
        }
        ("typing" | "prompting", ..) => {
            step(&tx, trace, id, "failed", Some(&said(status, output)))?;
        }
        // A hook moved it on before its command ended.
        _ => {}
    }
    tx.commit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn json() -> impl Strategy<Value = Value> {
        let text = prop_oneof![".*", "(?s).*", "ghp_[a-z0-9]{0,40}"];
        let field = prop_oneof![
            text.prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            Just(Value::Null),
        ];
        proptest::collection::hash_map("focus|then|other", field, 0..3)
            .prop_map(|fields| Value::Object(fields.into_iter().collect()))
    }

    proptest! {
        #[test]
        fn any_arguments_are_refused_or_safe_to_type(arguments in json()) {
            if let Ok((focus, then)) = texts(&arguments, &[]) {
                for (text, limit) in [(&focus, FOCUS_LIMIT), (&then, THEN_LIMIT)] {
                    prop_assert!(text.chars().count() <= limit);
                    let escapes = text.chars().any(|char| char == '\x1b');
                    prop_assert!(!escapes && !screen::holds_secret(text));
                }
                prop_assert!(!focus.contains('\n') && !then.is_empty());
            }
        }
    }
}
