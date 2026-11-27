//! Journals: the state a trigger's runs keep between them, on the log every runner reads. The
//! `journal_append`, `journal_read`, `journal_put` and `journal_get` tools, and the tables every
//! runner folds `priory.journal.appended` and `priory.journal.put` into, in log order.
//!
//! Every runner must keep the same tables, so whether a message applies depends only on the
//! message and the log before it: the requests every runner recorded, and the writes it already
//! applied. A runner's own writes wait apart until it reads them back, and never count there.

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

/// The latest of this runner's puts of each name of trigger `?1` that it has yet to read back.
const PENDING: &str = "SELECT name, body, written, max(rowid) FROM journal_pending
    WHERE trigger = ?1 GROUP BY name";

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

/// The bytes trigger `trigger`'s journal holds apart from the document `name`, which a put
/// replaces. With `confirmed`, only what every runner applied from the log; else also this
/// runner's writes on their way, each pending put in place of the document it replaces.
fn used(
    db: &Connection,
    trigger: &str,
    name: Option<&str>,
    confirmed: bool,
) -> rusqlite::Result<usize> {
    let sql = if confirmed {
        "SELECT coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM journal_entries
                          WHERE trigger = ?1 AND seq IS NOT NULL), 0)
              + coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM journal_documents
                          WHERE trigger = ?1 AND name IS NOT ?2), 0)"
            .to_owned()
    } else {
        format!(
            "WITH pending AS ({PENDING})
             SELECT coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM journal_entries
                              WHERE trigger = ?1), 0)
                  + coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM journal_documents
                              WHERE trigger = ?1 AND name IS NOT ?2
                                AND name NOT IN (SELECT name FROM pending)), 0)
                  + coalesce((SELECT sum(length(CAST(body AS BLOB))) FROM pending
                              WHERE name IS NOT ?2), 0)"
        )
    };
    db.query_row(&sql, params![trigger, name], |row| row.get::<_, i64>(0))
        .map(|bytes| usize::try_from(bytes).unwrap_or(usize::MAX))
}

fn full(trigger: &str, bytes: usize) -> String {
    format!(
        "the journal of {trigger} holds at most {} MB: this write would take it to {bytes} bytes, so nothing was written",
        JOURNAL_LIMIT / 1024 / 1024
    )
}

/// Whether the log held, before inbox position `before` when given, a request of task `key` by
/// trigger `trigger`. Every runner records every request, so every runner answers alike, and a
/// message that names a task no trigger requested reaches no journal.
fn requested(
    db: &Connection,
    key: &str,
    trigger: &str,
    before: Option<i64>,
) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM inbox
           WHERE type = 'factory.task.requested' AND json_extract(data, '$.key') = ?1
             AND json_extract(data, '$.trigger') = ?2 AND (?3 IS NULL OR seq < ?3))",
        params![key, trigger, before],
        |row| row.get(0),
    )
}

/// The key and trigger of the task whose run has harness session `session`, or why it has no
/// journal: no run has the session, or no factory trigger requested its task.
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
        Some((key, trigger)) if !requested(db, &key, &trigger, None)? => Err(format!(
            "the log holds no request of task {key} by trigger {trigger}, so no runner would keep its journal"
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
    let bytes = used(&tx, &trigger, (!entry).then_some(label.as_str()), false)? + text.len();
    if bytes > JOURNAL_LIMIT {
        return Ok(Err(full(&trigger, bytes)));
    }
    let mut data = json!({ "trigger": trigger, "body": body, "task": key, "runner": trace.runner });
    data[if entry { "kind" } else { "name" }] = json!(label);
    let time = intake::now().unwrap_or_else(|_| jiff::Timestamp::now());
    let kind = if entry { JOURNAL_APPENDED } else { JOURNAL_PUT };
    let message = Message::new(kind, data).subject(trigger.clone()).time(time);
    let id = message.id.to_string();
    let write = Write::of(&id, kind, time, &message.data);
    if entry {
        write.entry(&tx, None)?;
    } else {
        tx.execute(
            "INSERT INTO journal_pending (id, trigger, name, body, task, runner, written)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                write.trigger,
                write.label,
                write.body,
                write.task,
                write.runner,
                write.written
            ],
        )?;
    }
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

/// One journal write, as a message or an inbox row holds it.
struct Write<'a> {
    id: &'a str,
    entry: bool,
    trigger: &'a str,
    label: &'a str,
    body: String,
    task: &'a str,
    runner: Option<&'a str>,
    written: String,
    at: i64,
}

impl<'a> Write<'a> {
    fn of(id: &'a str, kind: &str, time: jiff::Timestamp, data: &'a Value) -> Self {
        let entry = kind == JOURNAL_APPENDED;
        let text = |field: &str| data[field].as_str().unwrap_or_default();
        Write {
            id,
            entry,
            trigger: text("trigger"),
            label: text(if entry { "kind" } else { "name" }),
            body: if data["body"].is_null() {
                String::new()
            } else {
                data["body"].to_string()
            },
            task: text("task"),
            runner: data["runner"].as_str(),
            written: time.to_string(),
            at: time.as_millisecond(),
        }
    }

    /// Whether every runner applies it, from the log before inbox position `before` alone.
    fn applies(&self, tx: &Transaction, before: Option<i64>) -> rusqlite::Result<bool> {
        Ok(!self.trigger.is_empty()
            && !self.body.is_empty()
            && refused(self.entry, self.label, self.body.len()).is_none()
            && requested(tx, self.task, self.trigger, before)?
            && used(tx, self.trigger, (!self.entry).then_some(self.label), true)? + self.body.len()
                <= JOURNAL_LIMIT)
    }

    /// Keeps it as an entry, read back at `seq` or on its way without.
    fn entry(&self, tx: &Transaction, seq: Option<i64>) -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO journal_entries (id, trigger, kind, body, task, runner, written, at, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (id) DO UPDATE SET seq = excluded.seq WHERE seq IS NULL",
            params![
                self.id,
                self.trigger,
                self.label,
                self.body,
                self.task,
                self.runner,
                self.written,
                self.at,
                seq
            ],
        )
        .map(drop)
    }

    /// Applies it, read from the log at inbox position `seq`: an entry joins the journal, and a
    /// document replaces the one of its name. Either way this runner's own copy stops waiting.
    fn apply(&self, tx: &Transaction, seq: i64) -> rusqlite::Result<()> {
        if self.entry {
            return self.entry(tx, Some(seq));
        }
        tx.execute(
            "INSERT INTO journal_documents (trigger, name, id, body, task, runner, written, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (trigger, name) DO UPDATE SET id = excluded.id, body = excluded.body,
               task = excluded.task, runner = excluded.runner, written = excluded.written,
               seq = excluded.seq",
            params![
                self.trigger,
                self.label,
                self.id,
                self.body,
                self.task,
                self.runner,
                self.written,
                seq
            ],
        )?;
        self.forget_own(tx)
    }

    /// Drops this runner's own copy on its way, if it wrote it: refused, or now applied.
    fn forget_own(&self, tx: &Transaction) -> rusqlite::Result<()> {
        tx.execute(
            "DELETE FROM journal_entries WHERE id = ?1 AND seq IS NULL",
            [self.id],
        )?;
        tx.execute("DELETE FROM journal_pending WHERE id = ?1", [self.id])
            .map(drop)
    }
}

/// Handles a journal message read from the log: applies its entry or document, or refuses it.
/// `None` for any other message.
pub(crate) fn handle(tx: &Transaction, message: &Message) -> rusqlite::Result<Option<()>> {
    if message.kind != JOURNAL_APPENDED && message.kind != JOURNAL_PUT {
        return Ok(None);
    }
    let id = message.id.to_string();
    let write = Write::of(&id, &message.kind, message.time, &message.data);
    // Anyone can publish on the log, so a malformed or foreign message is refused, never fatal.
    if write.applies(tx, None)? {
        let seq = crate::state::record(tx, message, None, "noted", None)?;
        write.apply(tx, seq)?;
    } else {
        crate::state::record(tx, message, None, "refused", None)?;
        write.forget_own(tx)?;
    }
    Ok(Some(()))
}

/// Applies, in log order, the journal messages a daemon older than journals recorded in the inbox
/// without reading them, as `handle` would have when it read each.
pub(crate) fn backfill(tx: &Transaction) -> rusqlite::Result<()> {
    let rows: Vec<(i64, String, String, String, String)> = tx
        .prepare(
            "SELECT seq, id, type, time, data FROM inbox
             WHERE type IN ('priory.journal.appended', 'priory.journal.put') ORDER BY seq",
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
    for (seq, id, kind, time, data) in rows {
        let data: Value = serde_json::from_str(&data).unwrap_or_default();
        let time = time.parse().unwrap_or_default();
        let write = Write::of(&id, &kind, time, &data);
        let outcome = if write.applies(tx, Some(seq))? {
            write.apply(tx, seq)?;
            "noted"
        } else {
            "refused"
        };
        tx.execute(
            "UPDATE inbox SET outcome = ?2 WHERE seq = ?1",
            params![seq, outcome],
        )?;
    }
    Ok(())
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
        Some(Ok(time)) => Some(time.as_millisecond()),
        None => None,
    };
    let last = last.unwrap_or(READ_DEFAULT).clamp(1, READ_MAX);
    let mut found = db
        .prepare(
            "SELECT id, kind, body, task, runner, written FROM journal_entries
             WHERE trigger = ?1 AND (?2 IS NULL OR kind = ?2) AND (?3 IS NULL OR at >= ?3)
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

/// Trigger `trigger`'s document `name`, or the name, size and time of each of its documents. This
/// runner's own put of a name, until it reads it back, stands in for the document it replaces.
pub fn documents(
    db: &Connection,
    trigger: &str,
    name: Option<&str>,
) -> rusqlite::Result<Result<Value, String>> {
    let current = format!(
        "WITH pending AS ({PENDING})
         SELECT name, body, written FROM pending
         UNION ALL
         SELECT name, body, written FROM journal_documents
         WHERE trigger = ?1 AND name NOT IN (SELECT name FROM pending)"
    );
    let rows: Vec<(String, String, String)> = db
        .prepare(&format!(
            "SELECT * FROM ({current}) WHERE ?2 IS NULL OR name = ?2 ORDER BY name"
        ))?
        .query_map(params![trigger, name], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let Some(name) = name else {
        let listed: Vec<Value> = rows
            .iter()
            .map(|(name, body, written)| json!({ "name": name, "bytes": body.len(), "written": written }))
            .collect();
        return Ok(Ok(json!({ "trigger": trigger, "documents": listed })));
    };
    Ok(match rows.into_iter().next() {
        Some((_, body, written)) => Ok(json!({
            "trigger": trigger,
            "name": name,
            "body": serde_json::from_str::<Value>(&body).unwrap_or(Value::Null),
            "written": written,
        })),
        None => Err(format!("the journal of {trigger} holds no document {name}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "github:owner/repo#1";

    fn request() -> Message {
        Message::new(
            "factory.task.requested",
            json!({ "key": KEY, "trigger": "daily", "workflow": "w" }),
        )
    }

    fn write(kind: &str, label: &str, body: Value) -> Message {
        let field = if kind == JOURNAL_APPENDED {
            "kind"
        } else {
            "name"
        };
        Message::new(
            kind,
            json!({ "trigger": "daily", field: label, "body": body, "task": KEY, "runner": "a" }),
        )
    }

    /// A body of `bytes` bytes as JSON.
    fn sized(bytes: usize) -> Value {
        json!("x".repeat(bytes - 2))
    }

    /// Reads `log` in order, as a runner's tick does.
    fn fold(db: &mut Connection, log: &[Message]) {
        for message in log {
            let tx = db.transaction().unwrap();
            if handle(&tx, message).unwrap().is_none() {
                crate::state::record(&tx, message, None, "ignored", None).unwrap();
            }
            tx.commit().unwrap();
        }
    }

    /// Queues `message` as this runner's own write, before the log has it.
    fn own(db: &mut Connection, message: &Message) {
        let tx = db.transaction().unwrap();
        let id = message.id.to_string();
        let write = Write::of(&id, &message.kind, message.time, &message.data);
        if write.entry {
            write.entry(&tx, None).unwrap();
        } else {
            tx.execute(
                "INSERT INTO journal_pending (id, trigger, name, body, task, runner, written)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id,
                    write.trigger,
                    write.label,
                    write.body,
                    write.task,
                    write.runner,
                    write.written
                ],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }

    /// What every runner must agree on: the applied entries and documents.
    fn tables(db: &Connection) -> (Vec<String>, Vec<(String, String)>) {
        let entries = db
            .prepare("SELECT id FROM journal_entries WHERE seq IS NOT NULL ORDER BY seq")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let documents = db
            .prepare("SELECT name, id FROM journal_documents ORDER BY name")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        (entries, documents)
    }

    fn document(db: &Connection, name: &str) -> Value {
        documents(db, "daily", Some(name)).unwrap().unwrap()["body"].clone()
    }

    #[test]
    fn runners_that_append_near_the_limit_at_once_and_one_that_reads_the_log_later_keep_equal_tables()
     {
        // 31 full documents and one that leaves 20,000 bytes: room for one of two 15,000-byte entries.
        let mut log = vec![request()];
        log.extend((0..31).map(|n| write(JOURNAL_PUT, &format!("doc-{n}"), sized(DOCUMENT_LIMIT))));
        log.push(write(
            JOURNAL_PUT,
            "rest",
            sized(JOURNAL_LIMIT - 31 * DOCUMENT_LIMIT - 20_000),
        ));
        let (from_a, from_b) = (
            write(JOURNAL_APPENDED, "run", sized(15_000)),
            write(JOURNAL_APPENDED, "run", sized(15_000)),
        );
        let (mut a, mut b) = (crate::state::memory(), crate::state::memory());
        fold(&mut a, &log);
        fold(&mut b, &log);
        own(&mut a, &from_a);
        own(&mut b, &from_b);
        // b's entry reaches the log first, while a's still waits on a.
        log.extend([from_b.clone(), from_a.clone()]);
        let tail = &log[log.len() - 2..];
        fold(&mut a, tail);
        fold(&mut b, tail);
        let mut later = crate::state::memory();
        fold(&mut later, &log);

        assert_eq!(tables(&a), tables(&b));
        assert_eq!(tables(&a), tables(&later));
        assert_eq!(tables(&a).0, [from_b.id.to_string()]);
        let pending: i64 = a
            .query_row(
                "SELECT count(*) FROM journal_entries WHERE seq IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0, "a's refused entry left its tables");
    }

    #[test]
    fn a_refused_own_put_keeps_the_version_every_runner_holds_and_a_put_applied_meanwhile() {
        let mut a = crate::state::memory();
        fold(
            &mut a,
            &[request(), write(JOURNAL_PUT, "history", json!("v1"))],
        );
        let mut refused = write(JOURNAL_PUT, "history", json!("v2"));
        // No trigger requested task 9, so every runner refuses the put once the log has it.
        refused.data["task"] = json!("github:owner/repo#9");
        own(&mut a, &refused);
        assert_eq!(
            document(&a, "history"),
            "v2",
            "a run reads its own put at once"
        );

        let from_b = write(JOURNAL_PUT, "history", json!("v3"));
        fold(&mut a, &[from_b]);
        assert_eq!(
            document(&a, "history"),
            "v2",
            "its own later put still stands in"
        );
        fold(&mut a, &[refused]);
        assert_eq!(document(&a, "history"), "v3");
    }

    #[test]
    fn a_second_own_put_stands_in_until_the_log_has_it_too() {
        let mut a = crate::state::memory();
        fold(&mut a, &[request()]);
        let (first, second) = (
            write(JOURNAL_PUT, "history", json!(1)),
            write(JOURNAL_PUT, "history", json!(2)),
        );
        own(&mut a, &first);
        own(&mut a, &second);
        assert_eq!(document(&a, "history"), 2);
        fold(&mut a, std::slice::from_ref(&first));
        assert_eq!(document(&a, "history"), 2);
        fold(&mut a, &[second]);
        assert_eq!(document(&a, "history"), 2);
        let pending: i64 = a
            .query_row("SELECT count(*) FROM journal_pending", [], |row| row.get(0))
            .unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn a_message_for_a_task_no_trigger_of_its_own_requested_reaches_no_journal() {
        let mut a = crate::state::memory();
        let mut other = write(JOURNAL_APPENDED, "run", json!(1));
        other.data["trigger"] = json!("weekly");
        // A request the log holds only after a write comes too late for it.
        let early = write(JOURNAL_APPENDED, "run", json!(2));
        fold(&mut a, &[other, early, request()]);
        fold(&mut a, &[write(JOURNAL_APPENDED, "run", json!(3))]);
        assert_eq!(tables(&a).0.len(), 1);
        let read = entries(&a, "daily", None, None, None).unwrap().unwrap();
        assert_eq!(read["entries"][0]["body"], 3);
        assert_eq!(
            entries(&a, "weekly", None, None, None).unwrap().unwrap()["entries"],
            json!([])
        );
    }

    #[test]
    fn since_compares_times_not_their_text() {
        let mut a = crate::state::memory();
        let at = |text: &str| text.parse::<jiff::Timestamp>().unwrap();
        let early =
            write(JOURNAL_APPENDED, "run", json!("early")).time(at("2026-11-26T10:00:00.5Z"));
        let late = write(JOURNAL_APPENDED, "run", json!("late")).time(at("2026-11-26T10:00:01Z"));
        fold(&mut a, &[request(), early, late]);
        let read = |since: &str| {
            entries(&a, "daily", None, Some(since), None)
                .unwrap()
                .unwrap()["entries"]
                .as_array()
                .unwrap()
                .len()
        };
        assert_eq!(read("2026-11-26T10:00:00.9Z"), 1);
        assert_eq!(read("2026-11-26T11:00:00+01:00"), 2);
    }
}
