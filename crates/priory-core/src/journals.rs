//! Journals: the state a trigger's runs keep between them, on the log every runner reads. The
//! `journal_append`, `journal_read`, `journal_put` and `journal_get` tools, and the tables every
//! runner folds `priory.journal.appended`, `priory.journal.put` and `priory.journal.pruned` into,
//! in log order.
//!
//! Every runner must keep the same tables, so whether a message applies depends only on the
//! message and the log before it: the requests every runner recorded, and the writes it already
//! applied. A runner's own writes wait apart until it reads them back, and never count there.
//! Pruning by age reads the journal clock, the latest time of a journal message that applied, never
//! a runner's own clock, so every runner prunes the same entries.

use crate::tools::Call;
use crate::trace::Trace;
use herdr_bus::Message;
use priory_factory::factory::{Factory, Keep};
use priory_intake::intake::{self, JOURNAL_APPENDED, JOURNAL_PRUNED, JOURNAL_PUT};
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
        // `triggers/cli.toml` only sets defaults for a person's requests, which keep no journal.
        Some((key, trigger)) if trigger == "cli" || !factory.triggers.contains_key(&trigger) => {
            Err(format!(
                "task {key} has no trigger (it came from {trigger}), so it has no journal: only the runs of a factory trigger keep one"
            ))
        }
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
    // Every runner prunes by the keep the log carries, never by its own factory's.
    let keep = trace.factory.triggers[&trigger].value.keep().to_value();
    let mut data = json!({ "trigger": trigger, "body": body, "task": key, "runner": trace.runner, "keep": keep });
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

/// `priory journal import`: publishes `entries`, each of kind `kind`, in their order, and each of
/// `puts`, to trigger `trigger`'s journal as `person`'s writes. It checks every write before it
/// publishes the first, and refuses a journal that already holds entries, or one of the documents,
/// unless `again`.
#[allow(clippy::too_many_arguments)]
pub fn import(
    factory: &Factory,
    db: &Connection,
    trigger: &str,
    person: &str,
    kind: &str,
    entries: &[Value],
    puts: &[(String, Value)],
    again: bool,
) -> Result<String, String> {
    let fail = |error: rusqlite::Error| error.to_string();
    if !factory.triggers.contains_key(trigger) {
        return Err(format!("the factory has no trigger {trigger}"));
    }
    if !again {
        let held: i64 = db
            .query_row(
                "SELECT count(*) FROM journal_entries WHERE trigger = ?1",
                [trigger],
                |row| row.get(0),
            )
            .map_err(fail)?;
        if !entries.is_empty() && held > 0 {
            return Err(format!(
                "the journal of {trigger} already holds {held} entries: pass --again to append these after them"
            ));
        }
        for (name, _) in puts {
            if documents(db, trigger, Some(name)).map_err(fail)?.is_ok() {
                return Err(format!(
                    "the journal of {trigger} already holds document {name}: pass --again to replace it"
                ));
            }
        }
    }
    let keep = factory.triggers[trigger].value.keep().to_value();
    let writes = entries
        .iter()
        .map(|body| (true, kind, body))
        .chain(puts.iter().map(|(name, body)| (false, name.as_str(), body)));
    // shortcut: a document that --again replaces counts twice here, which only refuses early near 8 MB.
    let mut bytes = used(db, trigger, None, false).map_err(fail)?;
    let mut messages = Vec::new();
    for (n, (entry, label, body)) in writes.enumerate() {
        let text = body.to_string();
        let what = if entry {
            format!("entry {}", n + 1)
        } else {
            format!("document {label}")
        };
        if let Some(why) = refused(entry, label, text.len()) {
            return Err(format!("{what}: {why}"));
        }
        if crate::screen::holds_secret(&text) || crate::screen::holds_secret(label) {
            return Err(format!(
                "{what} looks like it holds a credential: keep secrets out of a journal"
            ));
        }
        bytes += text.len();
        let mut data = json!({
            "trigger": trigger, "body": body, "person": person, "runner": null, "keep": keep,
        });
        data[if entry { "kind" } else { "name" }] = json!(label);
        let kind = if entry { JOURNAL_APPENDED } else { JOURNAL_PUT };
        messages.push(Message::new(kind, data).subject(trigger.to_owned()));
    }
    if bytes > JOURNAL_LIMIT {
        return Err(full(trigger, bytes));
    }
    crate::runners::publish_all(factory.log(), &messages).map_err(|(sent, unsent)| {
        // An unconfirmed publish may have reached the log before it failed.
        let (reached, why) = match unsent {
            crate::runners::Unsent::Nothing(why) => (sent.to_string(), why),
            crate::runners::Unsent::Unconfirmed(why) => {
                (format!("between {sent} and {}", sent + 1), why)
            }
        };
        let next = if sent < entries.len() {
            format!(
                "Once `priory journal read --trigger {trigger}` shows the last entry that arrived, rerun with --again and --skip set to how many of the file's entries the journal holds"
            )
        } else {
            "Every entry reached it: rerun with only the --document options and --again".to_owned()
        };
        format!(
            "{why}: {reached} of the {} writes reached the log, entries first. {next}",
            messages.len()
        )
    })?;
    let mut said = format!(
        "imported {} entries into the journal of {trigger}",
        entries.len()
    );
    for (name, body) in puts {
        said += &format!("\nput document {name}, {} bytes", body.to_string().len());
    }
    Ok(said)
}

/// One journal write, as a message or an inbox row holds it.
struct Write<'a> {
    id: &'a str,
    entry: bool,
    trigger: &'a str,
    label: &'a str,
    body: String,
    task: &'a str,
    /// The person who imported it, for a write no task made.
    person: &'a str,
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
            person: text("person"),
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
            && (if self.task.is_empty() {
                !self.person.trim().is_empty()
            } else {
                requested(tx, self.task, self.trigger, before)?
            })
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

/// Handles a journal message read from the log: applies its entry, document or prune, or refuses
/// it. `None` for any other message.
pub(crate) fn handle(tx: &Transaction, message: &Message) -> rusqlite::Result<Option<()>> {
    if ![JOURNAL_APPENDED, JOURNAL_PUT, JOURNAL_PRUNED].contains(&message.kind.as_str()) {
        return Ok(None);
    }
    let id = message.id.to_string();
    fold(
        tx,
        &id,
        &message.kind,
        message.time,
        &message.data,
        None,
        |outcome| crate::state::record(tx, message, None, outcome, None),
    )
    .map(Some)
}

/// What every runner does with the journal message `id` at its place in the log, before inbox
/// position `before` when given: it applies or refuses it, then records the outcome with `record`,
/// which returns the message's inbox position. Every decision reads the message and the log before
/// it alone, never this runner's factory or clock.
fn fold(
    tx: &Transaction,
    id: &str,
    kind: &str,
    time: jiff::Timestamp,
    data: &Value,
    before: Option<i64>,
    record: impl FnOnce(&str) -> rusqlite::Result<i64>,
) -> rusqlite::Result<()> {
    // Anyone can publish on the log, so a malformed or foreign message is refused, never fatal.
    let keep = data.get("keep").map(Keep::parse);
    if kind == JOURNAL_PRUNED {
        let text = |field: &str| data[field].as_str().filter(|text| !text.trim().is_empty());
        let cutoff = data.get("before").map(|time| {
            time.as_str()
                .and_then(|time| time.parse::<jiff::Timestamp>().ok())
        });
        let (Some(trigger), Some(_), None | Some(Some(_))) =
            (text("trigger"), text("person"), cutoff)
        else {
            return record("refused").map(drop);
        };
        if matches!(keep, Some(None)) {
            return record("refused").map(drop);
        }
        if let Some(Some(cutoff)) = cutoff {
            tx.execute(
                "DELETE FROM journal_entries WHERE trigger = ?1 AND seq IS NOT NULL AND at < ?2",
                params![trigger, cutoff.as_millisecond()],
            )?;
        }
        settle(tx, trigger, keep.flatten(), time)?;
        return record("noted").map(drop);
    }
    let write = Write::of(id, kind, time, data);
    // Pruned first, so the size check sees the entries every runner still holds here.
    prune_one(tx, write.trigger)?;
    if matches!(keep, Some(None)) || !write.applies(tx, before)? {
        record("refused")?;
        return write.forget_own(tx);
    }
    let seq = record("noted")?;
    write.apply(tx, seq)?;
    settle(tx, write.trigger, keep.flatten(), time)
}

/// After a message of trigger `trigger` applied: keeps the `keep` it carried as the trigger's,
/// moves the journal clock to its `time`, and prunes the trigger's entries.
fn settle(
    tx: &Connection,
    trigger: &str,
    keep: Option<Keep>,
    time: jiff::Timestamp,
) -> rusqlite::Result<()> {
    if let Some(keep) = keep {
        tx.execute(
            "INSERT INTO journal_keeps (trigger, keep) VALUES (?1, ?2)
             ON CONFLICT (trigger) DO UPDATE SET keep = excluded.keep",
            params![trigger, keep.to_value().to_string()],
        )?;
    }
    advance(tx, time)?;
    prune_one(tx, trigger).map(drop)
}

/// How far one message moves the journal clock at most.
const CLOCK_STEP: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);

/// Moves the journal clock to `time` when that is later, by at most `CLOCK_STEP`, so one writer
/// whose clock runs ahead cannot age every journal at once. Only applied journal messages move it,
/// in log order, so every runner's clock is the same at the same place in the log.
fn advance(tx: &Connection, time: jiff::Timestamp) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO journal_clock (one, at) VALUES (1, ?1)
         ON CONFLICT (one) DO UPDATE SET at = max(at, min(excluded.at, at + ?2))",
        params![time.as_millisecond(), CLOCK_STEP.as_millis() as i64],
    )
    .map(drop)
}

/// Deletes the applied entries of trigger `trigger` past the `keep` its last write on the log
/// carried, or the default: all but the newest `Count` in log order, or those written more than `For`
/// before the journal clock. This runner's own entries on their way stay until it reads them back.
fn prune_one(db: &Connection, trigger: &str) -> rusqlite::Result<usize> {
    let carried: Option<String> = db
        .query_row(
            "SELECT keep FROM journal_keeps WHERE trigger = ?1",
            [trigger],
            |row| row.get(0),
        )
        .optional()?;
    let keep = carried
        .and_then(|text| serde_json::from_str(&text).ok())
        .and_then(|value: Value| Keep::parse(&value))
        .unwrap_or(Keep::DEFAULT);
    match keep {
        Keep::Count(count) => db.execute(
            "DELETE FROM journal_entries WHERE trigger = ?1 AND seq IS NOT NULL
               AND seq NOT IN (SELECT seq FROM journal_entries
                               WHERE trigger = ?1 AND seq IS NOT NULL ORDER BY seq DESC LIMIT ?2)",
            params![trigger, count],
        ),
        Keep::For(kept) => db.execute(
            "DELETE FROM journal_entries WHERE trigger = ?1 AND seq IS NOT NULL
               AND at < (SELECT at FROM journal_clock) - ?2",
            params![trigger, i64::try_from(kept.as_millis()).unwrap_or(i64::MAX)],
        ),
    }
}

/// On each tick: deletes every journal's entries past its `keep`, as of the journal clock. Both
/// come from the log, so this deletes only what the next write of each trigger would.
pub(crate) fn prune(db: &Connection) -> rusqlite::Result<()> {
    let triggers: Vec<String> = db
        .prepare("SELECT DISTINCT trigger FROM journal_entries WHERE seq IS NOT NULL")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for trigger in triggers {
        prune_one(db, &trigger)?;
    }
    Ok(())
}

/// `priory journal prune`: publishes `priory.journal.pruned` as `person`'s, with trigger
/// `trigger`'s `keep` from `factory`, so every runner applies that keep at once. A duration's message
/// also drops the entries written before `now` minus the keep.
pub fn prune_now(
    factory: &Factory,
    trigger: &str,
    person: &str,
    now: jiff::Timestamp,
) -> Result<String, String> {
    let Some(found) = factory.triggers.get(trigger) else {
        return Err(format!("the factory has no trigger {trigger}"));
    };
    let keep = found.value.keep();
    let mut data = json!({ "trigger": trigger, "person": person, "keep": keep.to_value() });
    let said = match keep {
        Keep::For(kept) => {
            let before = jiff::SignedDuration::try_from(kept)
                .ok()
                .and_then(|kept| now.checked_sub(kept).ok())
                .unwrap_or(jiff::Timestamp::MIN);
            data["before"] = json!(before.to_string());
            format!("every runner drops its entries written before {before}")
        }
        Keep::Count(count) => format!("every runner keeps its newest {count} entries"),
    };
    let message = Message::new(JOURNAL_PRUNED, data)
        .subject(trigger.to_owned())
        .time(now);
    crate::runners::publish_all(factory.log(), &[message]).map_err(|(_, unsent)| match unsent {
        crate::runners::Unsent::Nothing(why) | crate::runners::Unsent::Unconfirmed(why) => why,
    })?;
    Ok(format!("pruned the journal of {trigger}: {said}"))
}

/// Applies, in log order, the journal messages a daemon older than journals recorded in the inbox
/// without reading them, as `handle` would have when it read each.
pub(crate) fn backfill(tx: &Transaction) -> rusqlite::Result<()> {
    for (seq, id, kind, time, data) in journal_rows(tx, false)? {
        let data: Value = serde_json::from_str(&data).unwrap_or_default();
        let time = time.parse().unwrap_or_default();
        fold(tx, &id, &kind, time, &data, Some(seq), |outcome| {
            tx.execute(
                "UPDATE inbox SET outcome = ?2 WHERE seq = ?1",
                params![seq, outcome],
            )?;
            Ok(seq)
        })?;
    }
    Ok(())
}

/// Sets the journal clock as a runner that read the whole log would: from the times of the journal
/// messages that applied, in log order.
pub(crate) fn replay_clock(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM journal_clock", [])?;
    for (_, _, _, time, _) in journal_rows(tx, true)? {
        if let Ok(time) = time.parse() {
            advance(tx, time)?;
        }
    }
    Ok(())
}

/// An inbox row's `seq`, `id`, `type`, `time` and `data`.
type Row = (i64, String, String, String, String);

/// The inbox's journal messages in log order, only those that applied with `noted`.
fn journal_rows(tx: &Transaction, noted: bool) -> rusqlite::Result<Vec<Row>> {
    tx.prepare(
        "SELECT seq, id, type, time, data FROM inbox
         WHERE type IN ('priory.journal.appended', 'priory.journal.put', 'priory.journal.pruned')
           AND (NOT ?1 OR outcome = 'noted')
         ORDER BY seq",
    )?
    .query_map([noted], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?
    .collect()
}

/// `journal_read` and `journal_get`: the entries or documents of the caller's trigger, or of the
/// trigger `trigger` when the caller's lists it in `[journal] reads`, as JSON.
pub(crate) fn read(
    db: &Connection,
    call: &Call,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let own = match caller(db, &call.session, factory)? {
        Ok((_, trigger)) => trigger,
        Err(why) => return Ok(Err(why)),
    };
    let arguments = &call.arguments;
    let text = |name: &str| arguments[name].as_str();
    let trigger = match text("trigger") {
        None => own,
        Some(other) if other == own => own,
        Some(other)
            if factory.triggers[&own]
                .value
                .reads()
                .iter()
                .any(|read| read == other) =>
        {
            other.to_owned()
        }
        Some(other) => {
            return Ok(Err(format!(
                "trigger {own} does not list {other} in [journal] reads, so its runs cannot read the journal of {other}"
            )));
        }
    };
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

    /// A factory with the triggers `daily` and `cli`, which `triggers/cli.toml` makes one.
    fn factory() -> Factory {
        factory_with("plain", "")
    }

    /// The factory of `factory()`, with `journal` after `daily`'s keys, and the triggers `weekly`,
    /// which reads `daily`, and `other`. Its folder is `name`'s.
    fn factory_with(name: &str, journal: &str) -> Factory {
        let dir =
            std::env::temp_dir().join(format!("priory-journals-{}-{name}", std::process::id()));
        let trigger =
            "source = \"schedule\"\nrepo = \"owner/repo\"\nat = \"07:00\"\nworkflow = \"w\"\n";
        let daily = format!("{trigger}{journal}");
        let weekly = format!("{trigger}[journal]\nreads = [\"daily\"]\n");
        for (path, text) in [
            ("factory.toml", "name = \"demo\"\n"),
            ("triggers/daily.toml", daily.as_str()),
            ("triggers/cli.toml", trigger),
            ("triggers/weekly.toml", weekly.as_str()),
            ("triggers/other.toml", trigger),
        ] {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        let factory = priory_factory::factory::load(&dir).0;
        assert!(factory.triggers.contains_key("daily") && factory.triggers.contains_key("cli"));
        factory
    }

    fn request(trigger: &str) -> Message {
        Message::new(
            "factory.task.requested",
            json!({ "key": KEY, "trigger": trigger, "workflow": "w" }),
        )
    }

    /// A message another runner wrote, as the log carries it.
    fn written(kind: &str, label: &str, body: Value) -> Message {
        let field = if kind == JOURNAL_APPENDED {
            "kind"
        } else {
            "name"
        };
        Message::new(
            kind,
            json!({ "trigger": "daily", field: label, "body": body, "task": KEY, "runner": "b" }),
        )
    }

    /// A body of `bytes` bytes as JSON.
    fn sized(bytes: usize) -> Value {
        json!("x".repeat(bytes - 2))
    }

    /// A runner whose task of `KEY`, which trigger `trigger` requested, has the live run `s`.
    fn runner(trigger: &str) -> Connection {
        let mut db = crate::state::memory();
        db.execute(
            "INSERT INTO tasks (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
             VALUES (?1, 'w', 'work', 'running', 'now', 'l', 0, ?2, 's', 'now')",
            params![KEY, trigger],
        )
        .unwrap();
        db.execute(
            "INSERT INTO runs (task, step, tmux, session) VALUES (1, 'work', 'priory-1', 's')",
            [],
        )
        .unwrap();
        fold(&mut db, &[request(trigger)]);
        db
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

    /// The run's `tool` call with `arguments`, and the message it queued for the log, if any.
    fn call(
        db: &mut Connection,
        factory: &Factory,
        tool: &str,
        arguments: Value,
    ) -> (Result<String, String>, Option<Message>) {
        let trace = Trace {
            factory,
            revision: None,
            machine: None,
            runner: Some("a".to_owned()),
            usage: |_, _| None,
        };
        let call = Call {
            tool: tool.to_owned(),
            session: "s".to_owned(),
            arguments,
        };
        let count = |db: &Connection| -> i64 {
            db.query_row("SELECT count(*) FROM sends", [], |row| row.get(0))
                .unwrap()
        };
        let before = count(db);
        let reply = write(db, &call, &trace).unwrap();
        let queued: Option<String> = (count(db) > before).then(|| {
            db.query_row(
                "SELECT message FROM sends ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
        });
        (
            reply,
            queued.map(|text| serde_json::from_str(&text).unwrap()),
        )
    }

    /// The message of the run's write, which must succeed.
    fn own(db: &mut Connection, factory: &Factory, tool: &str, arguments: Value) -> Message {
        let (reply, queued) = call(db, factory, tool, arguments);
        assert!(reply.is_ok(), "{reply:?}");
        queued.unwrap()
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

    /// Puts from another runner that leave `free` bytes of the journal.
    fn filled(free: usize) -> Vec<Message> {
        let mut log: Vec<Message> = (0..31)
            .map(|n| written(JOURNAL_PUT, &format!("doc-{n}"), sized(DOCUMENT_LIMIT)))
            .collect();
        log.push(written(
            JOURNAL_PUT,
            "rest",
            sized(JOURNAL_LIMIT - 31 * DOCUMENT_LIMIT - free),
        ));
        log
    }

    #[test]
    fn an_import_applies_on_a_runner_with_no_request_and_a_write_with_neither_task_nor_person_does_not()
     {
        let imported = |person: Option<&str>, body: &str| {
            let mut message = written(JOURNAL_APPENDED, "run", json!({ "line": body }));
            let data = message.data.as_object_mut().unwrap();
            data.remove("task");
            data["runner"] = Value::Null;
            if let Some(person) = person {
                data.insert("person".to_owned(), json!(person));
            }
            message
        };
        let log = [
            imported(Some("a-person"), "1"),
            imported(None, "anon"),
            imported(Some("a-person"), "2"),
        ];
        let mut later = crate::state::memory();
        fold(&mut later, &log);
        let read = entries(&later, "daily", None, None, None).unwrap().unwrap();
        let lines: Vec<&Value> = read["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| &e["body"]["line"])
            .collect();
        assert_eq!(lines, [&json!("1"), &json!("2")]);
    }

    #[test]
    fn runners_that_append_near_the_limit_at_once_and_one_that_reads_the_log_later_keep_equal_tables()
     {
        let factory = factory();
        let (mut a, mut b) = (runner("daily"), runner("daily"));
        let mut log = vec![request("daily")];
        // Room for one of two 15,000-byte entries.
        let fill = filled(20_000);
        fold(&mut a, &fill);
        fold(&mut b, &fill);
        log.extend(fill);
        let entry = json!({ "kind": "run", "body": sized(15_000) });
        let from_a = own(&mut a, &factory, "journal_append", entry.clone());
        let from_b = own(&mut b, &factory, "journal_append", entry.clone());
        let (refused, _) = call(&mut a, &factory, "journal_append", entry);
        assert!(
            refused.is_err_and(|why| why.contains("holds at most 8 MB")),
            "a's own entry on its way counts for a's next write"
        );
        // b's entry reaches the log first, while a's still waits on a.
        let tail = [from_b.clone(), from_a];
        fold(&mut a, &tail);
        fold(&mut b, &tail);
        log.extend(tail);
        let mut later = crate::state::memory();
        fold(&mut later, &log);

        assert_eq!(tables(&a), tables(&b));
        assert_eq!(tables(&a), tables(&later));
        assert_eq!(tables(&a).0, [from_b.id.to_string()]);
        let waiting: i64 = a
            .query_row(
                "SELECT count(*) FROM journal_entries WHERE seq IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(waiting, 0, "a's refused entry left its tables");
    }

    #[test]
    fn a_refused_own_put_keeps_the_version_every_runner_holds_and_a_put_applied_meanwhile() {
        let factory = factory();
        let mut a = runner("daily");
        fold(&mut a, &[written(JOURNAL_PUT, "history", json!("v1"))]);
        let big = json!({ "name": "history", "body": sized(150_000) });
        let from_a = own(&mut a, &factory, "journal_put", big);
        assert_eq!(document(&a, "history").as_str().unwrap().len(), 149_998);

        // Before a's put reaches the log, b fills the journal and puts its own version.
        let mut from_b = filled(100_000);
        from_b.push(written(JOURNAL_PUT, "history", json!("v3")));
        fold(&mut a, &from_b);
        assert_eq!(
            document(&a, "history").as_str().unwrap().len(),
            149_998,
            "a's own later put still stands in"
        );
        fold(&mut a, &[from_a]);
        assert_eq!(document(&a, "history"), "v3");
    }

    #[test]
    fn a_second_own_put_stands_in_until_the_log_has_it_too() {
        let factory = factory();
        let mut a = runner("daily");
        let first = own(
            &mut a,
            &factory,
            "journal_put",
            json!({ "name": "history", "body": 1 }),
        );
        let second = own(
            &mut a,
            &factory,
            "journal_put",
            json!({ "name": "history", "body": 2 }),
        );
        assert_eq!(document(&a, "history"), 2);
        fold(&mut a, &[first]);
        assert_eq!(document(&a, "history"), 2);
        fold(&mut a, &[second]);
        assert_eq!(document(&a, "history"), 2);
        let pending: i64 = a
            .query_row("SELECT count(*) FROM journal_pending", [], |row| row.get(0))
            .unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn a_persons_request_and_a_batch_entry_keep_no_journal_even_with_a_cli_trigger_file() {
        let factory = factory();
        // A batch's entries copy their planning task's trigger, which `priory batch add` sets to cli.
        let mut a = runner("cli");
        let (reply, queued) = call(
            &mut a,
            &factory,
            "journal_append",
            json!({ "kind": "run", "body": 1 }),
        );
        assert!(
            reply.is_err_and(|why| why.ends_with("has no trigger (it came from cli), so it has no journal: only the runs of a factory trigger keep one")),
        );
        assert!(queued.is_none());
    }

    #[test]
    fn a_message_for_a_task_no_trigger_of_its_own_requested_reaches_no_journal() {
        let mut a = crate::state::memory();
        let mut other = written(JOURNAL_APPENDED, "run", json!(1));
        other.data["trigger"] = json!("weekly");
        // A request the log holds only after a write comes too late for it.
        let early = written(JOURNAL_APPENDED, "run", json!(2));
        fold(&mut a, &[other, early, request("daily")]);
        fold(&mut a, &[written(JOURNAL_APPENDED, "run", json!(3))]);
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
            written(JOURNAL_APPENDED, "run", json!("early")).time(at("2026-11-26T10:00:00.5Z"));
        let late = written(JOURNAL_APPENDED, "run", json!("late")).time(at("2026-11-26T10:00:01Z"));
        fold(&mut a, &[request("daily"), early, late]);
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

    /// The bodies of trigger `trigger`'s entries, oldest first.
    fn bodies(db: &Connection, trigger: &str) -> Vec<Value> {
        entries(db, trigger, None, None, None).unwrap().unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["body"].clone())
            .collect()
    }

    fn at(text: &str) -> jiff::Timestamp {
        text.parse().unwrap()
    }

    /// `message` as a writer whose factory gives its trigger `keep` publishes it.
    fn keeping(mut message: Message, keep: Value) -> Message {
        message.data["keep"] = keep;
        message
    }

    #[test]
    fn entries_past_a_count_keep_leave_every_runners_tables() {
        let factory = factory_with("count", "[journal]\nkeep = 2\n");
        let (mut a, mut b) = (runner("daily"), runner("daily"));
        let from_a = own(
            &mut a,
            &factory,
            "journal_append",
            json!({ "kind": "run", "body": 0 }),
        );
        assert_eq!(from_a.data["keep"], 2, "a write carries its factory's keep");
        let mut log = vec![request("daily"), from_a];
        log.extend((1..=3).map(|n| keeping(written(JOURNAL_APPENDED, "run", json!(n)), json!(2))));
        fold(&mut a, &log[1..]);
        // b ticks between its reads; a reads every message at once.
        fold(&mut b, &log[1..3]);
        prune(&b).unwrap();
        fold(&mut b, &log[3..]);
        let mut later = crate::state::memory();
        fold(&mut later, &log);

        assert_eq!(bodies(&a, "daily"), [json!(2), json!(3)]);
        assert_eq!(tables(&a), tables(&b));
        assert_eq!(tables(&a), tables(&later));
    }

    #[test]
    fn entries_older_than_a_duration_keep_leave_every_runners_tables() {
        let entry = |body: i64, time: &str| {
            keeping(written(JOURNAL_APPENDED, "run", json!(body)), json!("1d")).time(at(time))
        };
        let log = vec![
            request("daily"),
            entry(1, "2026-11-20T07:00:00Z"),
            entry(2, "2026-11-20T08:00:00Z"),
            entry(3, "2026-11-21T07:30:00Z"),
            entry(4, "2026-11-22T07:00:00Z"),
        ];
        let (mut a, mut b) = (crate::state::memory(), crate::state::memory());
        fold(&mut a, &log);
        for message in &log {
            fold(&mut b, std::slice::from_ref(message));
            prune(&b).unwrap();
        }
        // A runner's own clock is days later, and moves nothing.
        prune(&a).unwrap();
        let mut later = crate::state::memory();
        fold(&mut later, &log);

        assert_eq!(bodies(&a, "daily"), [json!(3), json!(4)]);
        assert_eq!(tables(&a), tables(&b));
        assert_eq!(tables(&a), tables(&later));
    }

    #[test]
    fn runners_whose_factories_keep_differently_fold_a_log_near_the_limit_into_equal_tables() {
        // a's factory keeps one entry; b's keeps the default 90 days.
        let (one, default) = (factory_with("one", "[journal]\nkeep = 1\n"), factory());
        let (mut a, mut b) = (runner("daily"), runner("daily"));
        let mut log = vec![request("daily")];
        // Room for one of two 15,000-byte entries, unless the first is pruned.
        let fill = filled(20_000);
        fold(&mut a, &fill);
        fold(&mut b, &fill);
        log.extend(fill);
        let entry = json!({ "kind": "run", "body": sized(15_000) });
        let from_b = own(&mut b, &default, "journal_append", entry.clone());
        let from_a = own(&mut a, &one, "journal_append", entry);
        let tail = [from_b.clone(), from_a.clone()];
        fold(&mut a, &tail);
        fold(&mut b, &tail);
        log.extend(tail.clone());
        let mut later = crate::state::memory();
        fold(&mut later, &log);

        assert_eq!(tables(&a), tables(&b));
        assert_eq!(tables(&a), tables(&later));
        // b's entry came first and carried 90 days, so a's entry found no room on any runner, even
        // on a, whose own factory would have pruned b's to make it.
        assert_eq!(tables(&a).0, [from_b.id.to_string()]);
        let outcome = |id: &Message| -> String {
            a.query_row(
                "SELECT outcome FROM inbox WHERE id = ?1",
                [id.id.to_string()],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            (outcome(&from_b), outcome(&from_a)),
            ("noted".to_owned(), "refused".to_owned())
        );
    }

    #[test]
    fn a_message_stamped_a_year_ahead_prunes_no_entry_within_keep() {
        let entry = |body: i64, time: &str| {
            keeping(written(JOURNAL_APPENDED, "run", json!(body)), json!("30d")).time(at(time))
        };
        let mut a = crate::state::memory();
        fold(
            &mut a,
            &[
                request("daily"),
                entry(1, "2026-11-20T07:00:00Z"),
                entry(2, "2026-11-21T07:00:00Z"),
                entry(3, "2027-11-21T07:00:00Z"),
            ],
        );
        prune(&a).unwrap();
        assert_eq!(bodies(&a, "daily"), [json!(1), json!(2), json!(3)]);
        // Each later message moves the clock a day at most, until real time catches up.
        fold(&mut a, &[entry(4, "2026-11-22T07:00:00Z")]);
        assert_eq!(bodies(&a, "daily").len(), 4);
    }

    #[test]
    fn a_persons_prune_applies_and_one_that_names_no_person_does_not() {
        let entry =
            |body: i64, time: &str| written(JOURNAL_APPENDED, "run", json!(body)).time(at(time));
        let prune = |before: &str, person: Option<&str>| {
            let mut data = json!({ "trigger": "daily", "before": before });
            if let Some(person) = person {
                data["person"] = json!(person);
            }
            Message::new(JOURNAL_PRUNED, data).subject("daily")
        };
        let mut a = crate::state::memory();
        fold(
            &mut a,
            &[
                request("daily"),
                entry(1, "2026-11-20T07:00:00Z"),
                entry(2, "2026-11-20T09:00:00Z"),
                entry(3, "2026-11-20T11:00:00Z"),
                prune("2026-11-20T12:00:00Z", None),
                prune("2026-11-20T08:00:00Z", Some("a-person")),
            ],
        );
        assert_eq!(bodies(&a, "daily"), [json!(2), json!(3)]);
        // A count carries no time: the keep it carries prunes, and later entries keep to it.
        let count = |keep: Value| {
            Message::new(
                JOURNAL_PRUNED,
                json!({ "trigger": "daily", "person": "a-person", "keep": keep }),
            )
        };
        fold(&mut a, &[count(json!(0)), count(json!(1))]);
        assert_eq!(bodies(&a, "daily"), [json!(3)]);
        fold(&mut a, &[entry(4, "2026-11-20T13:00:00Z")]);
        assert_eq!(bodies(&a, "daily"), [json!(4)]);

        let outcomes: Vec<String> = a
            .prepare("SELECT outcome FROM inbox WHERE type = ?1 ORDER BY seq")
            .unwrap()
            .query_map([JOURNAL_PRUNED], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(outcomes, ["refused", "noted", "refused", "noted"]);
    }

    #[test]
    fn a_run_reads_a_journal_its_trigger_lists_and_is_refused_one_it_does_not() {
        let factory = factory();
        let daily = [
            request("daily"),
            written(JOURNAL_APPENDED, "run", json!(1)),
            written(JOURNAL_PUT, "history", json!("v1")),
        ];
        let reply = |db: &Connection, tool: &str, arguments: Value| {
            let call = Call {
                tool: tool.to_owned(),
                session: "s".to_owned(),
                arguments,
            };
            read(db, &call, &factory).unwrap()
        };
        let mut weekly = runner("weekly");
        fold(&mut weekly, &daily);
        let mut other = runner("other");
        fold(&mut other, &daily);

        let read: Value = serde_json::from_str(
            &reply(&weekly, "journal_read", json!({ "trigger": "daily" })).unwrap(),
        )
        .unwrap();
        assert_eq!(read["trigger"], "daily");
        assert_eq!(read["entries"][0]["body"], 1);
        let got: Value = serde_json::from_str(
            &reply(
                &weekly,
                "journal_get",
                json!({ "trigger": "daily", "name": "history" }),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(got["body"], "v1");
        let own: Value =
            serde_json::from_str(&reply(&weekly, "journal_read", json!({})).unwrap()).unwrap();
        assert_eq!(own["trigger"], "weekly");
        assert_eq!(
            reply(&other, "journal_read", json!({ "trigger": "daily" })),
            Err("trigger other does not list daily in [journal] reads, so its runs cannot read the journal of daily".to_owned())
        );
        assert!(
            reply(&weekly, "journal_get", json!({ "trigger": "other" }))
                .is_err_and(|why| why.starts_with("trigger weekly does not list other"))
        );
    }
}
