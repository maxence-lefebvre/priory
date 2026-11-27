//! Journals: the state a trigger's runs keep between them, on the log every runner reads. The
//! `journal_append`, `journal_read`, `journal_put` and `journal_get` tools, and the tables every
//! runner folds `priory.journal.appended` and `priory.journal.put` into, in log order.

use crate::tools::Call;
use crate::trace::Trace;
use herdr_bus::Message;
use priory_factory::factory::Factory;
use priory_intake::intake::{self, JOURNAL_APPENDED, JOURNAL_PUT};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

/// The most bytes an entry's body holds, as JSON.
pub const ENTRY_LIMIT: usize = 16 * 1024;
/// The most bytes a document's body holds, as JSON.
pub const DOCUMENT_LIMIT: usize = 256 * 1024;
/// The most bytes a trigger's live entries and documents hold together.
pub const JOURNAL_LIMIT: usize = 8 * 1024 * 1024;
/// How many entries `journal_read` returns without `last`, and at most.
const READ_DEFAULT: i64 = 20;
const READ_MAX: i64 = 200;

/// Why an entry of `kind`, or a document of `name`, with a body of `bytes` is refused, if it is.
fn refused(entry: bool, label: &str, bytes: usize) -> Option<String> {
    let what = if entry {
        "an entry's kind"
    } else {
        "a document's name"
    };
    if label.trim().is_empty() || label.len() > 200 || label.chars().any(char::is_control) {
        return Some(format!("{what} is one line of at most 200 bytes"));
    }
    let (limit, shape) = if entry {
        (ENTRY_LIMIT, "an entry")
    } else {
        (DOCUMENT_LIMIT, "a document")
    };
    (bytes > limit).then(|| {
        format!(
            "{shape} holds at most {} KB: this one is {bytes} bytes, so nothing was written",
            limit / 1024
        )
    })
}

/// The bytes trigger `trigger`'s journal holds, leaving out the row of id `id` and the document
/// `name`, which a write replaces.
fn used(db: &Connection, trigger: &str, id: &str, name: Option<&str>) -> rusqlite::Result<usize> {
    db.query_row(
        "SELECT coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM journal_entries
                          WHERE trigger = ?1 AND id != ?2), 0)
              + coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM journal_documents
                          WHERE trigger = ?1 AND id != ?2 AND name IS NOT ?3), 0)",
        params![trigger, id, name],
        |row| row.get::<_, i64>(0),
    )
    .map(|bytes| usize::try_from(bytes).unwrap_or(usize::MAX))
}

fn full(trigger: &str, bytes: usize) -> String {
    format!(
        "the journal of {trigger} holds at most {} MB: this write would take it to {bytes} bytes, so nothing was written",
        JOURNAL_LIMIT / 1024 / 1024
    )
}

/// The key and trigger of the task whose run has harness session `session`, or why it has no
/// journal: no run has the session, or no factory trigger started its task.
fn caller(
    db: &Connection,
    session: &str,
    factory: &Factory,
) -> rusqlite::Result<Result<(String, String), String>> {
    let found: Option<(String, String)> = db
        .query_row(
            "SELECT tasks.key, tasks.trigger FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.session = ?1 ORDER BY runs.id DESC LIMIT 1",
            [session],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match found {
        None => Err(format!("no run has session {session}")),
        Some((key, trigger)) if !factory.triggers.contains_key(&trigger) => Err(format!(
            "task {key} has no trigger (it came from {trigger}), so it has no journal: only the runs of a factory trigger keep one"
        )),
        Some(found) => Ok(found),
    })
}

fn argument<'a>(call: &'a Call, name: &str) -> Result<&'a str, String> {
    call.arguments[name]
        .as_str()
        .ok_or_else(|| format!("{name} must be a string"))
}

/// `journal_append(kind, body)` and `journal_put(name, body)`: stores the write for the caller's
/// trigger and queues its message for the log.
pub(crate) fn write(
    db: &mut Connection,
    call: &Call,
    trace: &Trace,
) -> rusqlite::Result<Result<String, String>> {
    let entry = call.tool == "journal_append";
    let label = match argument(call, if entry { "kind" } else { "name" }) {
        Ok(label) => label.trim().to_owned(),
        Err(why) => return Ok(Err(why)),
    };
    let body = &call.arguments["body"];
    if body.is_null() {
        return Ok(Err(
            "body must be a JSON value or a string of Markdown".to_owned()
        ));
    }
    let text = body.to_string();
    if let Some(why) = refused(entry, &label, text.len()) {
        return Ok(Err(why));
    }
    if crate::screen::holds_secret(&text) || crate::screen::holds_secret(&label) {
        return Ok(Err(
            "the body looks like it holds a credential: keep secrets out of a journal".to_owned(),
        ));
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (key, trigger) = match caller(&tx, &call.session, trace.factory)? {
        Ok(found) => found,
        Err(why) => return Ok(Err(why)),
    };
    let bytes = used(&tx, &trigger, "", (!entry).then_some(label.as_str()))? + text.len();
    if bytes > JOURNAL_LIMIT {
        return Ok(Err(full(&trigger, bytes)));
    }
    let mut data = json!({ "trigger": trigger, "body": body, "task": key, "runner": trace.runner });
    data[if entry { "kind" } else { "name" }] = json!(label);
    let time = intake::now().unwrap_or_else(|_| jiff::Timestamp::now());
    let kind = if entry { JOURNAL_APPENDED } else { JOURNAL_PUT };
    let message = Message::new(kind, data).subject(trigger.clone()).time(time);
    let id = message.id.to_string();
    store(&tx, &message, None)?;
    tx.execute(
        "INSERT INTO sends (message) VALUES (?1)",
        [serde_json::to_string(&message).expect("a message serializes")],
    )?;
    tx.commit()?;
    Ok(Ok(if entry {
        format!("appended entry {id} to the journal of {trigger}")
    } else {
        format!("put document {label} in the journal of {trigger}")
    }))
}

/// Writes `message`'s entry or document into the tables, at log position `seq` once read back.
/// A document replaces the one of its name unless that one is this runner's own put that the log
/// has yet to order, which comes later.
fn store(tx: &Transaction, message: &Message, seq: Option<i64>) -> rusqlite::Result<()> {
    let data = &message.data;
    let text = |field: &str| data[field].as_str();
    let values = params![
        message.id.to_string(),
        text("trigger"),
        text(if message.kind == JOURNAL_APPENDED {
            "kind"
        } else {
            "name"
        }),
        data["body"].to_string(),
        text("task"),
        text("runner"),
        message.time.to_string(),
        seq
    ];
    if message.kind == JOURNAL_APPENDED {
        tx.execute(
            "INSERT INTO journal_entries (id, trigger, kind, body, task, runner, written, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (id) DO UPDATE SET seq = excluded.seq WHERE seq IS NULL",
            values,
        )?;
    } else {
        tx.execute(
            "INSERT INTO journal_documents (id, trigger, name, body, task, runner, written, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (trigger, name) DO UPDATE SET id = excluded.id, body = excluded.body,
               task = excluded.task, runner = excluded.runner, written = excluded.written,
               seq = excluded.seq
             WHERE journal_documents.id = excluded.id OR journal_documents.seq IS NOT NULL",
            values,
        )?;
    }
    Ok(())
}

/// Whether this runner knows the task `key` came from triggers other than `trigger` alone: from a
/// task it holds or a request it read.
fn foreign(tx: &Transaction, key: &str, trigger: &str) -> rusqlite::Result<bool> {
    tx.query_row(
        "WITH known (trigger) AS (
           SELECT trigger FROM tasks WHERE key = ?1
           UNION ALL
           SELECT json_extract(data, '$.trigger') FROM inbox
           WHERE type = 'factory.task.requested' AND json_extract(data, '$.key') = ?1)
         SELECT EXISTS (SELECT 1 FROM known) AND NOT EXISTS (SELECT 1 FROM known WHERE trigger = ?2)",
        params![key, trigger],
        |row| row.get(0),
    )
}

/// Handles a journal message read from the log: keeps its entry or document, or refuses it. `None`
/// for any other message.
pub(crate) fn handle(tx: &Transaction, message: &Message) -> rusqlite::Result<Option<()>> {
    let entry = message.kind == JOURNAL_APPENDED;
    if !entry && message.kind != JOURNAL_PUT {
        return Ok(None);
    }
    let data = &message.data;
    let text = |field: &str| data[field].as_str().unwrap_or_default();
    let (trigger, key) = (text("trigger"), text("task"));
    let label = text(if entry { "kind" } else { "name" });
    let body = data["body"].to_string();
    let id = message.id.to_string();
    // Anyone can publish on the log, so a malformed or foreign message is refused, never fatal.
    let applies = !trigger.is_empty()
        && !key.is_empty()
        && !data["body"].is_null()
        && refused(entry, label, body.len()).is_none()
        && !foreign(tx, key, trigger)?
        && used(tx, trigger, &id, (!entry).then_some(label))? + body.len() <= JOURNAL_LIMIT;
    if !applies {
        crate::state::record(tx, message, None, "refused", None)?;
        // This runner's own write, refused on its way back, leaves its tables as every runner's.
        tx.execute("DELETE FROM journal_entries WHERE id = ?1", [&id])?;
        tx.execute("DELETE FROM journal_documents WHERE id = ?1", [&id])?;
        return Ok(Some(()));
    }
    let seq = crate::state::record(tx, message, None, "noted", None)?;
    store(tx, message, Some(seq))?;
    Ok(Some(()))
}

/// `journal_read` and `journal_get`: the caller's trigger's entries or documents, as JSON.
pub(crate) fn read(
    db: &Connection,
    call: &Call,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let trigger = match caller(db, &call.session, factory)? {
        Ok((_, trigger)) => trigger,
        Err(why) => return Ok(Err(why)),
    };
    let arguments = &call.arguments;
    let text = |name: &str| arguments[name].as_str();
    let read = if call.tool == "journal_read" {
        entries(
            db,
            &trigger,
            arguments["last"].as_i64(),
            text("since"),
            text("kind"),
        )?
    } else {
        documents(db, &trigger, text("name"))?
    };
    Ok(read.map(|value| value.to_string()))
}

/// Trigger `trigger`'s entries, oldest first: the last `last` (20 by default, at most 200), of kind
/// `kind` when given, written at or after the RFC 3339 time `since` when given.
pub fn entries(
    db: &Connection,
    trigger: &str,
    last: Option<i64>,
    since: Option<&str>,
    kind: Option<&str>,
) -> rusqlite::Result<Result<Value, String>> {
    let since = match since.map(str::parse::<jiff::Timestamp>) {
        Some(Err(error)) => return Ok(Err(format!("since is no RFC 3339 time: {error}"))),
        Some(Ok(time)) => Some(time.to_string()),
        None => None,
    };
    let last = last.unwrap_or(READ_DEFAULT).clamp(1, READ_MAX);
    let mut found = db
        .prepare(
            "SELECT id, kind, body, task, runner, written FROM journal_entries
             WHERE trigger = ?1 AND (?2 IS NULL OR kind = ?2) AND (?3 IS NULL OR written >= ?3)
             ORDER BY seq IS NULL DESC, seq DESC, rowid DESC LIMIT ?4",
        )?
        .query_map(params![trigger, kind, since, last], |row| {
            let body: String = row.get(2)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "kind": row.get::<_, String>(1)?,
                "body": serde_json::from_str::<Value>(&body).unwrap_or(Value::Null),
                "task": row.get::<_, String>(3)?,
                "runner": row.get::<_, Option<String>>(4)?,
                "written": row.get::<_, String>(5)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    found.reverse();
    Ok(Ok(json!({ "trigger": trigger, "entries": found })))
}

/// Trigger `trigger`'s document `name`, or the name, size and time of each of its documents.
pub fn documents(
    db: &Connection,
    trigger: &str,
    name: Option<&str>,
) -> rusqlite::Result<Result<Value, String>> {
    let Some(name) = name else {
        let listed = db
            .prepare(
                "SELECT name, length(CAST(body AS BLOB)), written FROM journal_documents
                 WHERE trigger = ?1 ORDER BY name",
            )?
            .query_map([trigger], |row| {
                Ok(json!({
                    "name": row.get::<_, String>(0)?,
                    "bytes": row.get::<_, i64>(1)?,
                    "written": row.get::<_, String>(2)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        return Ok(Ok(json!({ "trigger": trigger, "documents": listed })));
    };
    let found: Option<(String, String)> = db
        .query_row(
            "SELECT body, written FROM journal_documents WHERE trigger = ?1 AND name = ?2",
            params![trigger, name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match found {
        Some((body, written)) => Ok(json!({
            "trigger": trigger,
            "name": name,
            "body": serde_json::from_str::<Value>(&body).unwrap_or(Value::Null),
            "written": written,
        })),
        None => Err(format!("the journal of {trigger} holds no document {name}")),
    })
}
