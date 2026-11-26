//! A cloud run's grant on its runner: made as the run starts, revoked once the run ends, and the
//! calls the diocese's gateway hands over through the log, applied as calls on the socket.

use crate::diocese::{CALLED, GRANTED, REPLIED, REVOKED};
use crate::trace::Trace;
use herdr_bus::Message;
use priory_intake::intake;
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

type Error = Box<dyn std::error::Error>;

/// Makes run `run`'s grant for `seconds` and queues its hash for the log. Returns the MCP servers
/// its session loads, which hold the token, or none when the factory has no gateway.
pub(crate) fn make(
    tx: &Transaction,
    trace: &Trace,
    run: i64,
    task: i64,
    seconds: u64,
) -> Result<Option<String>, Error> {
    let Some(url) = trace.factory.cloud_gateway() else {
        return Ok(None);
    };
    let token = crate::serve::new_secret()?;
    let id = uuid::Uuid::now_v7().to_string();
    let hash = crate::diocese::hash(&token);
    let key: Option<String> =
        tx.query_row("SELECT key FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    tx.execute(
        "INSERT INTO run_grants (run, grant_id, hash) VALUES (?1, ?2, ?3)",
        params![run, id, hash],
    )?;
    let data = json!({ "grant": id, "hash": hash, "seconds": seconds, "task": key, "run": run,
        "runner": trace.runner });
    let message = Message::new(GRANTED, data)
        .subject(&id)
        .time(intake::now()?);
    crate::runners::send(tx, &message)?;
    let servers = json!({ "mcpServers": { "priory": { "type": "http", "url": url,
        "headers": { "Authorization": format!("Bearer {token}") } } } });
    Ok(Some(servers.to_string()))
}

/// Revokes each grant whose run ended: by its result, a failure, a cancel, a move or a give.
pub(crate) fn revoke_ended(db: &mut Connection) -> Result<(), Error> {
    let tx = db.transaction()?;
    let ended: Vec<String> = tx
        .prepare(
            "SELECT grant_id FROM run_grants JOIN runs ON runs.id = run_grants.run
             WHERE NOT run_grants.revoked AND runs.ended IS NOT NULL",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in ended {
        let message = Message::new(REVOKED, json!({ "grant": id }))
            .subject(&id)
            .time(intake::now()?);
        crate::runners::send(&tx, &message)?;
        tx.execute(
            "UPDATE run_grants SET revoked = 1 WHERE grant_id = ?1",
            [&id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Takes a call that the gateway handed over, and says whether `message` is one. A call of a grant
/// this runner did not make is another runner's. A call id seen before adds nothing.
pub(crate) fn called(tx: &Transaction, message: &Message) -> Result<bool, Error> {
    if message.kind != CALLED {
        return Ok(false);
    }
    let text = |field: &str| message.data[field].as_str().unwrap_or_default();
    let (call, grant) = (text("call"), text("grant"));
    let live: Option<bool> = tx
        .query_row(
            "SELECT NOT run_grants.revoked AND runs.ended IS NULL FROM run_grants
             JOIN runs ON runs.id = run_grants.run WHERE grant_id = ?1",
            [grant],
            |row| row.get(0),
        )
        .optional()?;
    let Some(live) = live else {
        crate::state::record(tx, message, None, "elsewhere", None)?;
        return Ok(true);
    };
    crate::state::record(tx, message, None, "called", None)?;
    let added = tx.execute(
        "INSERT OR IGNORE INTO run_calls (call, grant_id, tool, arguments) VALUES (?1, ?2, ?3, ?4)",
        params![
            call,
            grant,
            text("tool"),
            message.data["arguments"].to_string()
        ],
    )?;
    if added == 1 && !live {
        answer(
            tx,
            call,
            Err(format!("run grant {grant} is revoked: its run ended")),
        )?;
    }
    Ok(true)
}

/// Applies each call the gateway handed over that has no reply yet, as a call on the socket of
/// its run's session, and queues the reply for the log.
pub(crate) fn apply(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
) -> Result<(), Error> {
    let pending: Vec<(String, String, String, i64, String)> = db
        .prepare(
            "SELECT run_calls.call, run_calls.tool, run_calls.arguments, runs.id, runs.session
             FROM run_calls JOIN run_grants USING (grant_id) JOIN runs ON runs.id = run_grants.run
             WHERE run_calls.reply IS NULL ORDER BY run_calls.rowid",
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
    for (id, tool, arguments, run, session) in pending {
        let arguments: Value = serde_json::from_str(&arguments).unwrap_or(Value::Null);
        // shortcut: the tool and its reply commit apart, so a crash between them applies the call
        // again; move the reply into the tool's transaction if that ever shows.
        let reply = match tool.as_str() {
            "result" => report(db, trace, run, &arguments)?,
            "ask" | "wait" => {
                let call = crate::tools::Call {
                    tool: tool.clone(),
                    session,
                    arguments,
                };
                let (reply, kill) =
                    crate::tools::call(db, &call, trace, kit, &|worktree, branch| {
                        runtime::head(worktree, branch)
                    })?;
                if let Some(kill) = kill {
                    runtime::kill(&kill);
                }
                if reply.is_ok() {
                    db.execute("UPDATE run_grants SET wake = 1 WHERE run = ?1", [run])?;
                }
                reply
            }
            _ => Err(format!(
                "the gateway serves ask, wait and result, not {tool}"
            )),
        };
        let tx = db.transaction()?;
        answer(&tx, &id, reply)?;
        tx.commit()?;
    }
    Ok(())
}

/// Keeps the result a session reported, which ends its step once Priory looked for its push.
fn report(
    db: &Connection,
    trace: &Trace,
    run: i64,
    arguments: &Value,
) -> Result<Result<String, String>, Error> {
    let (workflow, step): (String, String) = db.query_row(
        "SELECT tasks.workflow, runs.step FROM runs JOIN tasks ON tasks.id = runs.task
         WHERE runs.id = ?1",
        [run],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let Some(step) = trace
        .factory
        .workflows
        .get(&workflow)
        .and_then(|workflow| workflow.value.step(&step))
    else {
        return Ok(Err(format!("step {step} is not in workflow {workflow}")));
    };
    let outcome = arguments["outcome"].as_str().unwrap_or_default();
    if !step.next.contains_key(outcome) {
        let valid: Vec<_> = step.next.keys().map(String::as_str).collect();
        return Ok(Err(format!(
            "outcome {outcome:?} is not one of this step's: {}",
            valid.join(", ")
        )));
    }
    if arguments["summary"].as_str().is_none_or(str::is_empty) {
        return Ok(Err("a result needs a summary".to_owned()));
    }
    db.execute(
        "UPDATE run_grants SET reported = ?2 WHERE run = ?1",
        params![run, arguments.to_string()],
    )?;
    Ok(Ok(format!(
        "result {outcome} recorded. Priory ends the step once it looked for your push: it takes the \
         pushed commit, or ends the step without one when the branch has no new commit. End your turn."
    )))
}

/// Stores the reply to call `id` and queues it for the gateway.
fn answer(tx: &Transaction, id: &str, reply: Result<String, String>) -> Result<(), Error> {
    let (text, failed) = match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    let grant: String = tx.query_row(
        "UPDATE run_calls SET reply = ?2, failed = ?3 WHERE call = ?1 RETURNING grant_id",
        params![id, text, failed],
        |row| row.get(0),
    )?;
    let data = json!({ "call": id, "grant": grant, "text": text, "error": failed });
    let message = Message::new(REPLIED, data).subject(id).time(intake::now()?);
    crate::runners::send(tx, &message)?;
    Ok(())
}

/// The result a session reported through the gateway, for run `run`.
pub(crate) fn reported(db: &Connection, run: i64) -> rusqlite::Result<Option<Value>> {
    let text: Option<Option<String>> = db
        .query_row(
            "SELECT reported FROM run_grants WHERE run = ?1",
            [run],
            |row| row.get(0),
        )
        .optional()?;
    Ok(text
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok()))
}

/// Wakes each live cloud session that asked or waited once something is queued for its task: an
/// answer, an event or feedback. It reads them as its next turn's message.
pub(crate) fn wake(db: &mut Connection) -> Result<(), Error> {
    let due: Vec<(i64, String)> = db
        .prepare(
            "SELECT run_grants.run, runs.thread FROM run_grants JOIN runs ON runs.id = run_grants.run
             WHERE run_grants.wake AND NOT run_grants.revoked AND runs.ended IS NULL
               AND runs.thread IS NOT NULL",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (run, session) in due {
        let mut said = Vec::new();
        while let Some(text) = crate::state::claim(db, run)? {
            said.push(text);
        }
        if said.is_empty() {
            continue;
        }
        // shortcut: a failed wake is only logged, and the desk still shows the answer; retry it
        // if sessions miss wakes.
        db.execute(
            "UPDATE outbox SET state = 'confirmed', confirmed = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE run = ?1 AND state = 'attempted'",
            [run],
        )?;
        db.execute("UPDATE run_grants SET wake = 0 WHERE run = ?1", [run])?;
        let text = said.join("\n\n");
        std::thread::spawn(move || {
            if let Err(error) = priory_runtime::cloud::wake(&session, &text) {
                eprintln!("priory daemon: wake cloud session {session}: {error}");
            }
        });
    }
    Ok(())
}
