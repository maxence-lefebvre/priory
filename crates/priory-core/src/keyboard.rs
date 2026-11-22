//! A person's keyboard on a run's pane: a grant taken from the web view, and the text and keys sent
//! under it. The daemon applies each line; the secret never reaches it, only its hash.

use crate::state;
use crate::trace::{self, Trace};
use priory_factory::factory::Terminal;
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The keys a person may press, each a name tmux's `send-keys` reads as that one key.
pub const KEYS: &[&str] = &[
    "Enter", "Escape", "Tab", "BTab", "Up", "Down", "Left", "Right", "PageUp", "PageDown", "C-c",
    "C-d", "y", "n", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];
/// The most bytes one text may hold.
const MOST: usize = 1024;
/// The most bytes a request id may hold.
const REQUEST: usize = 64;
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";
/// A grant on run ?1's pane: of ?1, or of the run that carried ?1's process into the next step.
const ON_PANE: &str = "run IN (SELECT pane.id FROM runs AS pane JOIN runs AS asked ON asked.id = ?1
  WHERE pane.task = asked.task AND pane.tmux = asked.tmux AND pane.session IS asked.session)";

/// What one request sends: printable text, pasted as is, or one key of `KEYS`.
#[derive(serde::Serialize, serde::Deserialize, Debug)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Input {
    Text(String),
    Key(String),
}

impl Input {
    /// What the trace keeps of it: `text`, never the text, or the key's name.
    fn kind(&self) -> &str {
        match self {
            Input::Text(_) => "text",
            Input::Key(key) => key,
        }
    }
}

/// Refuses a text that is empty, over 1 KiB or holds a control character, newline and Escape
/// included, so text cannot do what only a listed key may; and a key off the list.
pub fn check(input: &Input) -> Result<(), String> {
    match input {
        Input::Text(text) if text.is_empty() => Err("text is empty".to_owned()),
        Input::Text(text) if text.len() > MOST => {
            Err(format!("text holds {} bytes, over {MOST}", text.len()))
        }
        Input::Text(text) if text.chars().any(char::is_control) => Err(
            "text holds a control character: send Enter, Escape, Tab or C-c as a key".to_owned(),
        ),
        Input::Key(key) if !KEYS.contains(&key.as_str()) => Err(format!(
            "`{key}` is not a key Priory sends; send one of {}",
            KEYS.join(", ")
        )),
        _ => Ok(()),
    }
}

/// Refuses a request id that is empty or over 64 bytes.
pub fn check_request(request: &str) -> Result<(), String> {
    if request.is_empty() || request.len() > REQUEST {
        return Err(format!("request is an id of 1 to {REQUEST} bytes"));
    }
    Ok(())
}

/// The hash a grant is stored and found by.
pub fn hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `POST /api/runs/{id}/keyboard`'s line: take run's keyboard.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeLine {
    pub keyboard: Take,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    pub run: i64,
    pub label: String,
    pub hash: String,
}

/// `POST /api/runs/{id}/keyboard/return`'s line.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnLine {
    pub keyboard_return: Held,
}

/// A grant of `run`, named by its secret's hash.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Held {
    pub run: i64,
    pub hash: String,
}

/// `POST /api/runs/{id}/keys`'s line.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeysLine {
    pub keys: Keys,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keys {
    pub run: i64,
    pub hash: String,
    pub request: String,
    pub input: Input,
}

fn typing(trace: &Trace) -> Result<(), String> {
    match trace.factory.serve() {
        Some(serve) if serve.terminal == Terminal::Type => Ok(()),
        _ => Err("[serve] terminal is not \"type\", so no one takes a run's keyboard".to_owned()),
    }
}

/// Stores a grant of the live run's keyboard for 15 minutes, ending the one it had, and replies
/// with its id and end.
pub(crate) fn take(
    db: &mut Connection,
    take: &Take,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    if let Err(refused) = typing(trace) {
        return Ok(Err(refused));
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // The live run is all a take needs: writes elsewhere move the state's revision every second.
    if let Err(refused) = state::live_run(&tx, &state::Attach::Run(take.run)) {
        return Ok(Err(refused));
    }
    let open: Option<i64> = tx
        .query_row(
            "SELECT id FROM keyboard_grants WHERE run = ?1 AND ended IS NULL",
            [take.run],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(grant) = open {
        end(&tx, trace, grant, "replaced")?;
    }
    let (grant, ends): (i64, String) = tx.query_row(
        &format!(
            "INSERT INTO keyboard_grants (run, hash, label, started, ends)
             VALUES (?1, ?2, ?3, {NOW}, strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+15 minutes'))
             RETURNING id, ends"
        ),
        params![take.run, take.hash, take.label],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let fields = json!({ "grant": grant.to_string(), "label": take.label, "ends": ends });
    trace::terminal(&tx, trace, "factory.terminal.taken", take.run, fields)?;
    tx.commit()?;
    Ok(Ok(json!({ "grant": grant, "ends": ends }).to_string()))
}

/// Ends the open grant `held` names, as the person gave it back.
pub(crate) fn give_back(
    db: &mut Connection,
    held: &Held,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let grant = match open(&tx, held)? {
        Ok((grant, _)) => grant,
        Err(refused) => return Ok(Err(refused)),
    };
    end(&tx, trace, grant, "returned")?;
    tx.commit()?;
    Ok(Ok(format!("run {}'s keyboard returned", held.run)))
}

/// The open grant `held` names and the run it is on now, or why there is none.
fn open(db: &Connection, held: &Held) -> rusqlite::Result<Result<(i64, i64), String>> {
    let found: Option<(i64, i64, Option<String>)> = db
        .query_row(
            &format!(
                "SELECT id, run, CASE WHEN why IS NULL AND julianday(ends) <= julianday('now')
                   THEN 'expired' ELSE why END
                 FROM keyboard_grants WHERE hash = ?2 AND {ON_PANE}"
            ),
            params![held.run, held.hash],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(match found {
        None => Err(format!(
            "no keyboard grant of run {} has this secret: take the keyboard",
            held.run
        )),
        Some((_, _, Some(why))) => Err(format!(
            "your keyboard grant of run {} ended ({why}): take the keyboard again",
            held.run
        )),
        Some((grant, run, None)) => Ok((grant, run)),
    })
}

/// Sends one text or key to the run's pane under an open grant, once per request id: a repeat
/// gets the first outcome and sends nothing. The row is stored before the send and its outcome
/// after, with its trace record, so a crash between leaves it to `adopt`.
pub(crate) fn keys(
    db: &mut Connection,
    keys: &Keys,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    if let Err(refused) = typing(trace)
        .and_then(|()| check(&keys.input))
        .and_then(|()| check_request(&keys.request))
    {
        return Ok(Err(refused));
    }
    sweep(db, trace)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let held = Held {
        run: keys.run,
        hash: keys.hash.clone(),
    };
    let any: Option<i64> = tx
        .query_row(
            &format!("SELECT id FROM keyboard_grants WHERE hash = ?2 AND {ON_PANE}"),
            params![held.run, held.hash],
            |row| row.get(0),
        )
        .optional()?;
    let repeated: Option<(Option<String>, Option<String>)> = tx
        .query_row(
            "SELECT outcome, error FROM keyboard_keys WHERE grant_id = ?1 AND request = ?2",
            params![any, keys.request],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((outcome, error)) = repeated {
        let outcome = outcome.unwrap_or_else(|| "uncertain".to_owned());
        return Ok(Ok(reply(&outcome, error.as_deref(), true)));
    }
    let (grant, on) = match open(&tx, &held)? {
        Ok(found) => found,
        Err(refused) => return Ok(Err(refused)),
    };
    let run = match state::live_run(&tx, &state::Attach::Run(on)) {
        Ok(run) => run,
        Err(refused) => return Ok(Err(refused)),
    };
    let length = match &keys.input {
        Input::Text(text) => Some(text.chars().count() as i64),
        Input::Key(_) => None,
    };
    tx.execute(
        "INSERT INTO keyboard_keys (grant_id, request, input, length) VALUES (?1, ?2, ?3, ?4)",
        params![grant, keys.request, keys.input.kind(), length],
    )?;
    tx.commit()?;
    let sent = runtime::pane(&run.tmux, &run.session)
        .ok_or_else(|| state::no_pane(&run))
        .and_then(|pane| match &keys.input {
            Input::Text(text) => runtime::paste(&pane, text),
            Input::Key(key) => runtime::press(&pane, key),
        });
    let (outcome, error) = match &sent {
        Ok(()) => ("sent", None),
        // A tmux client killed at its deadline may have sent already.
        Err(error) if error.contains("timed out after") => ("uncertain", Some(error.as_str())),
        Err(error) => ("failed", Some(error.as_str())),
    };
    let tx = db.transaction()?;
    settle(&tx, trace, grant, &keys.request, outcome, error)?;
    tx.commit()?;
    Ok(Ok(reply(outcome, error, false)))
}

fn reply(outcome: &str, error: Option<&str>, repeated: bool) -> String {
    let mut reply = json!({ "outcome": outcome });
    if let Some(error) = error {
        reply["error"] = json!(error);
    }
    if repeated {
        reply["repeated"] = json!(true);
    }
    reply.to_string()
}

/// Stores a request's outcome and appends its one `factory.terminal.keys` record.
fn settle(
    tx: &rusqlite::Transaction,
    trace: &Trace,
    grant: i64,
    request: &str,
    outcome: &str,
    error: Option<&str>,
) -> rusqlite::Result<()> {
    let (run, input, length): (i64, String, Option<i64>) = tx.query_row(
        "UPDATE keyboard_keys SET outcome = ?3, error = ?4 WHERE grant_id = ?1 AND request = ?2
         RETURNING (SELECT run FROM keyboard_grants WHERE id = ?1), input, length",
        params![grant, request, outcome, error],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let fields = json!({
        "grant": grant.to_string(),
        "request": request,
        "input": input,
        "length": length,
        "outcome": outcome,
        "error": error,
    });
    trace::terminal(tx, trace, "factory.terminal.keys", run, fields)
}

/// Ends grant `grant` as `why` and appends its `factory.terminal.returned` record.
fn end(tx: &rusqlite::Transaction, trace: &Trace, grant: i64, why: &str) -> rusqlite::Result<()> {
    let run: i64 = tx.query_row(
        &format!("UPDATE keyboard_grants SET ended = {NOW}, why = ?2 WHERE id = ?1 RETURNING run"),
        params![grant, why],
        |row| row.get(0),
    )?;
    let fields = json!({ "grant": grant.to_string(), "reason": why });
    trace::terminal(tx, trace, "factory.terminal.returned", run, fields)
}

/// Ends each open grant past its end, of a run that ended, or any once the factory's terminal is
/// not `"type"`.
pub(crate) fn sweep(db: &mut Connection, trace: &Trace) -> rusqlite::Result<()> {
    let typing = typing(trace).is_ok();
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ending: Vec<(i64, String)> = tx
        .prepare(
            "SELECT keyboard_grants.id, CASE
               WHEN NOT ?1 THEN 'terminal not \"type\"'
               WHEN runs.ended IS NOT NULL THEN 'run ended'
               ELSE 'expired' END
             FROM keyboard_grants JOIN runs ON runs.id = keyboard_grants.run
             WHERE keyboard_grants.ended IS NULL AND (NOT ?1 OR runs.ended IS NOT NULL
               OR julianday(keyboard_grants.ends) <= julianday('now'))",
        )?
        .query_map([typing], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (grant, why) in ending {
        end(&tx, trace, grant, &why)?;
    }
    tx.commit()
}

/// Marks each request a crash cut between its row and its outcome `uncertain`: it is never sent
/// again, since it may have reached the pane.
pub(crate) fn adopt(db: &mut Connection, trace: &Trace) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cut: Vec<(i64, String)> = tx
        .prepare("SELECT grant_id, request FROM keyboard_keys WHERE outcome IS NULL")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (grant, request) in cut {
        settle(&tx, trace, grant, &request, "uncertain", None)?;
    }
    tx.commit()
}

/// `priory attach`'s line: a read-write attach command was issued for `run`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachLine {
    pub attach_issued: Attach,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attach {
    pub run: i64,
    pub printed: bool,
}

/// Appends a `factory.terminal.attach_issued` record for `attach`.
pub(crate) fn attach_issued(
    db: &mut Connection,
    attach: &Attach,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction()?;
    let known: Option<i64> = tx
        .query_row("SELECT id FROM runs WHERE id = ?1", [attach.run], |row| {
            row.get(0)
        })
        .optional()?;
    if known.is_none() {
        return Ok(Err(format!("no run {}", attach.run)));
    }
    let fields: Value = json!({ "printed": attach.printed });
    trace::terminal(
        &tx,
        trace,
        "factory.terminal.attach_issued",
        attach.run,
        fields,
    )?;
    tx.commit()?;
    Ok(Ok(String::new()))
}

#[cfg(test)]
mod tests {
    use super::{Input, KEYS, check};
    use proptest::prelude::*;

    #[test]
    fn text_up_to_1_kib_passes_and_one_byte_more_is_refused() {
        assert!(check(&Input::Text("é".repeat(512))).is_ok());
        assert!(check(&Input::Text(format!("{}a", "é".repeat(512)))).is_err());
    }

    proptest! {
        #[test]
        fn a_text_passes_only_without_control_characters_and_a_key_only_from_the_list(
            text in "\\PC{0,8}[\\x00-\\x1f\\x7f\\u{80}-\\u{9f}]?\\PC{0,8}",
            key in "[A-Za-z0-9-]{1,9}",
        ) {
            let clean = !text.is_empty() && !text.chars().any(char::is_control);
            prop_assert_eq!(check(&Input::Text(text)).is_ok(), clean);
            prop_assert_eq!(check(&Input::Key(key.clone())).is_ok(), KEYS.contains(&key.as_str()));
        }
    }
}
