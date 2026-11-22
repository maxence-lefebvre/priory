//! Memories that agents write on the log: the `remember`, `forget` and `memory` tools, the
//! candidates every runner records in log order, and a person's accept or drop (R11.1).

use crate::tools::Call;
use crate::trace::Trace;
use herdr_bus::Message;
use priory_factory::factory::{self, EventLog, Factory, Kit, MEMORY_LIMIT, Memory};
use priory_intake::intake::{self, MEMORY_ACCEPTED, MEMORY_DROPPED, MEMORY_WRITTEN};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// A memory's name has lowercase letters, digits and hyphens, and starts with a letter or a digit,
/// as `priory check` asks of `memories/<name>.md`.
fn named(name: &str) -> bool {
    name.bytes()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// The file a candidate's memory is, as the run's snapshot holds it.
fn file(name: &str, description: &str, text: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {}\n---\n{text}\n",
        serde_json::to_string(description).expect("a string serializes")
    )
}

/// Why a candidate is refused, if it is: the rules of `remember` and `forget`, which a runner also
/// applies to every candidate it reads, since anyone who can publish on the log can write one.
fn refused(name: &str, kept: Option<(&str, &str)>) -> Option<String> {
    if !named(name) {
        return Some(
            "a memory's name has lowercase letters, digits and hyphens, and starts with a letter or a digit"
                .to_owned(),
        );
    }
    let (description, text) = kept?;
    if description.trim().is_empty() {
        return Some(
            "a memory needs a description, which tells an agent when to read it".to_owned(),
        );
    }
    if text.trim().is_empty() {
        return Some("a memory needs a text: the fact itself".to_owned());
    }
    let bytes = file(name, description, text).len();
    if bytes > MEMORY_LIMIT {
        return Some(format!(
            "a memory holds one fact in at most {MEMORY_LIMIT} bytes: this one is {bytes} bytes"
        ));
    }
    if [name, description, text]
        .into_iter()
        .any(crate::screen::holds_secret)
    {
        return Some(
            "the text looks like it holds a credential: a memory reaches every later run, so keep secrets out of it"
                .to_owned(),
        );
    }
    None
}

fn argument<'a>(call: &'a Call, name: &str) -> Result<&'a str, String> {
    call.arguments[name]
        .as_str()
        .ok_or_else(|| format!("{name} must be a string"))
}

/// The task of the run whose harness session is `session`, with its key.
fn caller(db: &Connection, session: &str) -> rusqlite::Result<Option<(i64, String)>> {
    db.query_row(
        "SELECT tasks.id, tasks.key FROM runs JOIN tasks ON tasks.id = runs.task
         WHERE runs.session = ?1 ORDER BY runs.id DESC LIMIT 1",
        [session],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// `remember(name, description, text)` and `forget(name)`: stores a candidate for the caller's
/// task and queues its `priory.memory.written` for the log.
pub(crate) fn write(
    db: &mut Connection,
    call: &Call,
    trace: &Trace,
    kit: Option<&crate::kit::Fetched>,
) -> rusqlite::Result<Result<String, String>> {
    let fields = (|| {
        let name = argument(call, "name")?.trim().to_owned();
        let kept = match call.tool.as_str() {
            "remember" => Some((
                argument(call, "description")?.trim().to_owned(),
                argument(call, "text")?.trim().to_owned(),
            )),
            _ => None,
        };
        match refused(&name, kept.as_ref().map(|(d, t)| (d.as_str(), t.as_str()))) {
            Some(why) => Err(why),
            None => Ok((name, kept)),
        }
    })();
    let (name, kept) = match fields {
        Ok(fields) => fields,
        Err(why) => return Ok(Err(why)),
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some((task, key)) = caller(&tx, &call.session)? else {
        return Ok(Err(format!("no run has session {}", call.session)));
    };
    if let Some((description, text)) = &kept
        && let Some(why) = repeat(&tx, trace.factory, kit, task, &name, description, text)?
    {
        return Ok(Err(why));
    }
    let mut data = json!({ "name": name, "task": key, "runner": trace.runner });
    match &kept {
        Some((description, text)) => {
            data["description"] = json!(description);
            data["text"] = json!(text);
        }
        None => data["forget"] = json!(true),
    }
    let time = intake::now().unwrap_or_else(|_| jiff::Timestamp::now());
    let message = Message::new(MEMORY_WRITTEN, data)
        .subject(name.clone())
        .time(time);
    let id = message.id.to_string();
    tx.execute(
        "INSERT INTO memory_candidates (id, name, description, text, key, runner, task, written)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            name,
            kept.as_ref().map(|(description, _)| description),
            kept.as_ref().map(|(_, text)| text),
            key,
            trace.runner,
            task,
            time.to_string()
        ],
    )?;
    tx.execute(
        "INSERT INTO sends (message) VALUES (?1)",
        [serde_json::to_string(&message).expect("a message serializes")],
    )?;
    tx.commit()?;
    let what = if kept.is_some() {
        format!("remembered {name}")
    } else {
        format!("forgot {name}")
    };
    Ok(Ok(format!(
        "{what} as candidate {id}: your task's later runs see it now, and other tasks' runs once a person accepts it"
    )))
}

/// Why a `remember` of `name` repeats what exists, if it does: the memory the task sees under that
/// name, a candidate of that name that waits for a person, or one a person dropped, with the same
/// description and text. A changed text is a correction, never a repeat.
fn repeat(
    tx: &Transaction,
    factory: &Factory,
    kit: Option<&crate::kit::Fetched>,
    task: i64,
    name: &str,
    description: &str,
    text: &str,
) -> rusqlite::Result<Option<String>> {
    let layer = layer(tx, Some(task))?;
    let sources = crate::kit::sources(factory, kit, &layer);
    if sources.memory(name).is_some_and(|memory| {
        memory.description.trim() == description && memory.text().trim() == text
    }) {
        return Ok(Some(format!("{name} already holds this text")));
    }
    let same = "name = ?1 AND description = ?2 AND text = ?3 AND accepted IS NULL";
    let pending: Option<String> = tx
        .query_row(
            &format!("SELECT id FROM memory_candidates WHERE {same} AND dropped IS NULL LIMIT 1"),
            params![name, description, text],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = pending {
        return Ok(Some(format!(
            "candidate {id} already proposes this text for {name}, and waits for a person"
        )));
    }
    let dropped: Option<String> = tx
        .query_row(
            &format!(
                "SELECT substr(decided, 1, 10) FROM memory_candidates WHERE {same}
                 AND dropped IS NOT NULL ORDER BY dropped DESC LIMIT 1"
            ),
            params![name, description, text],
            |row| row.get(0),
        )
        .optional()?;
    Ok(dropped.map(|date| {
        format!(
            "a person dropped this text for {name} on {date}: propose it again only with new evidence"
        )
    }))
}

/// Handles a memory message read from the log: records a candidate, or a person's accept or drop.
/// `None` for any other message.
pub(crate) fn handle(tx: &Transaction, message: &Message) -> rusqlite::Result<Option<()>> {
    let kind = message.kind.as_str();
    if ![MEMORY_WRITTEN, MEMORY_ACCEPTED, MEMORY_DROPPED].contains(&kind) {
        return Ok(None);
    }
    let data = &message.data;
    let text = |field: &str| data[field].as_str();
    // Anyone can publish on the log, so a malformed message is refused, never fatal.
    let applied = if kind == MEMORY_WRITTEN {
        let forget = data["forget"] == true;
        let kept = text("description").zip(text("text"));
        let name = text("name").unwrap_or_default();
        let well_formed =
            text("task").is_some() && forget == kept.is_none() && refused(name, kept).is_none();
        if well_formed {
            let seq = crate::state::record(tx, message, None, "noted", None)?;
            tx.execute(
                "INSERT INTO memory_candidates (id, name, description, text, key, runner, written, logged)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET logged = excluded.logged WHERE logged IS NULL",
                params![
                    message.id.to_string(),
                    name,
                    text("description"),
                    text("text"),
                    text("task"),
                    text("runner"),
                    message.time.to_string(),
                    seq
                ],
            )?;
            return Ok(Some(()));
        }
        false
    } else {
        let column = if kind == MEMORY_ACCEPTED {
            "accepted"
        } else {
            "dropped"
        };
        // A retire drops an accepted candidate; a plain drop that comes after an accept does not.
        let decides = if kind == MEMORY_DROPPED && data["retire"] == true {
            "accepted IS NOT NULL"
        } else {
            "accepted IS NULL"
        };
        let seq = crate::state::record(tx, message, None, "noted", None)?;
        let decided = tx.execute(
            &format!(
                "UPDATE memory_candidates SET {column} = ?2, decided = ?3
                 WHERE id = ?1 AND logged IS NOT NULL AND {decides} AND dropped IS NULL"
            ),
            params![text("candidate"), seq, message.time.to_string()],
        )?;
        decided == 1
    };
    if !applied {
        crate::state::record(tx, message, None, "refused", None)?;
    }
    Ok(Some(()))
}

/// The candidates a run of `task` gets over the kit's and the factory's memories, in the order they
/// apply: each accepted one at its accept, and each of the task's own at its write. A forget is
/// `None`. Without a task, only the accepted ones.
pub(crate) fn layer(
    db: &Connection,
    task: Option<i64>,
) -> rusqlite::Result<Vec<(String, Option<Memory>)>> {
    db.prepare(
        "SELECT name, description, text FROM (
           SELECT name, description, text, accepted AS at, rowid AS written
           FROM memory_candidates WHERE accepted IS NOT NULL AND dropped IS NULL
           UNION ALL
           SELECT name, description, text, coalesce(logged, 9223372036854775807), rowid
           FROM memory_candidates WHERE task = ?1 AND dropped IS NULL)
         ORDER BY at, written",
    )?
    .query_map([task], |row| {
        let name: String = row.get(0)?;
        let kept: Option<(String, String)> = row.get::<_, Option<String>>(1)?.zip(row.get(2)?);
        let memory = kept.map(|(description, text)| Memory {
            bytes: file(&name, &description, &text).into_bytes(),
            description,
        });
        Ok((name, memory))
    })?
    .collect()
}

/// `memory(name)`: the file of the memory `name` as the caller's task sees it now.
pub(crate) fn recall(
    db: &Connection,
    call: &Call,
    factory: &Factory,
    kit: Option<&crate::kit::Fetched>,
) -> rusqlite::Result<Result<String, String>> {
    let name = match argument(call, "name") {
        Ok(name) => name.trim(),
        Err(why) => return Ok(Err(why)),
    };
    let Some((task, _)) = caller(db, &call.session)? else {
        return Ok(Err(format!("no run has session {}", call.session)));
    };
    let layer = layer(db, Some(task))?;
    let sources = crate::kit::sources(factory, kit, &layer);
    Ok(match sources.memory(name) {
        Some(memory) => Ok(String::from_utf8_lossy(&memory.bytes).into_owned()),
        None => Err(format!(
            "no memory {name} reaches your task: your instructions' index lists those that do"
        )),
    })
}

const STATE: &str = "CASE WHEN accepted IS NOT NULL AND dropped IS NOT NULL THEN 'retired'
  WHEN accepted IS NOT NULL THEN 'accepted' WHEN dropped IS NOT NULL THEN 'dropped' WHEN logged IS NULL THEN 'sending' ELSE 'pending' END";

/// The candidates this runner read, oldest first: only those that wait for a person, or with `all`
/// every one.
pub fn list(db: &Connection, all: bool) -> rusqlite::Result<Vec<Value>> {
    db.prepare(&format!(
        "SELECT id, name, description, text, key, runner, written, decided, {STATE}
         FROM memory_candidates WHERE ?1 OR (logged IS NOT NULL AND accepted IS NULL
           AND dropped IS NULL)
         ORDER BY coalesce(logged, 9223372036854775807), rowid"
    ))?
    .query_map([all], |row| {
        let text: Option<String> = row.get(3)?;
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "name": row.get::<_, String>(1)?,
            "description": row.get::<_, Option<String>>(2)?,
            "forget": text.is_none(),
            "text": text,
            "task": row.get::<_, String>(4)?,
            "runner": row.get::<_, Option<String>>(5)?,
            "written": row.get::<_, String>(6)?,
            "decided": row.get::<_, Option<String>>(7)?,
            "state": row.get::<_, String>(8)?,
        }))
    })?
    .collect()
}

/// The memories a run gets before any candidate of its own task, as `mark` reads them.
pub struct Held<'a> {
    /// The factory's `[kit]`, whose commit the state file names.
    pub kit: Option<&'a Kit>,
    /// The factory's own memories, each name's text.
    pub factory: BTreeMap<String, String>,
}

impl<'a> Held<'a> {
    /// The kit settings and the memories of `factory`.
    pub fn of(factory: &'a Factory) -> Self {
        Held {
            kit: factory.settings.as_ref().and_then(|s| s.value.kit.as_ref()),
            factory: factory
                .memories
                .iter()
                .map(|(name, memory)| (name.clone(), memory.value.text()))
                .collect(),
        }
    }

    /// Each memory's text by name: the kit's at the commit runs get now, the factory's, then the
    /// accepted candidates in their order. And whether the kit's are in it: false when the factory
    /// has a kit whose commit is not checked out here, as on a machine that only views.
    fn texts(
        &self,
        db: &Connection,
        home: &Path,
    ) -> rusqlite::Result<(BTreeMap<String, String>, bool)> {
        let mut texts = BTreeMap::new();
        let commit: Option<String> = db
            .query_row("SELECT kit_commit FROM kit", [], |row| row.get(0))
            .optional()?
            .flatten();
        let checkout = commit
            .map(|commit| home.join("kit").join(commit))
            .filter(|dir| dir.is_dir());
        let complete = self.kit.is_none() || checkout.is_some();
        if let Some((kit, dir)) = self.kit.zip(checkout) {
            let (files, _) = factory::load_kit(&dir, kit);
            texts.extend(
                files
                    .memories
                    .iter()
                    .map(|(n, m)| (n.clone(), m.value.text())),
            );
        }
        texts.extend(self.factory.clone());
        for (name, memory) in layer(db, None)? {
            match memory {
                Some(memory) => texts.insert(name, memory.text()),
                None => texts.remove(&name),
            };
        }
        Ok((texts, complete))
    }

    /// Marks each candidate with `replaces`, whether runs get a memory of its name now, and
    /// `current_text`, that memory's text or null. `replaces` is null when only the kit, which is
    /// not checked out here, could hold the name.
    pub fn mark(
        &self,
        db: &Connection,
        home: &Path,
        candidates: &mut [Value],
    ) -> rusqlite::Result<()> {
        if candidates.is_empty() {
            return Ok(());
        }
        let (texts, complete) = self.texts(db, home)?;
        for candidate in candidates {
            let current = candidate["name"].as_str().and_then(|name| texts.get(name));
            candidate["replaces"] = match current {
                None if !complete => Value::Null,
                _ => json!(current.is_some()),
            };
            candidate["current_text"] = json!(current);
        }
        Ok(())
    }
}

/// Why a person's accept or drop of a candidate was not published.
pub enum Refused {
    /// This runner never read the candidate.
    Unknown(String),
    /// The candidate is not on the log yet, or a person already decided it.
    Stale(String),
    /// The log, or the state file, could not be reached.
    Unsent(String),
    /// The publish failed, after which the log may still hold the verdict.
    Unconfirmed(String),
}

impl From<Refused> for String {
    fn from(refused: Refused) -> String {
        match refused {
            Refused::Unknown(why)
            | Refused::Stale(why)
            | Refused::Unsent(why)
            | Refused::Unconfirmed(why) => why,
        }
    }
}

/// Publishes a person's accept or drop of candidate `id` on `log`, once this runner's copy shows
/// it waiting, or a drop of an accepted one, which retires it. Every runner applies it as it reads
/// the log.
pub fn decide(log: &EventLog, db: &Connection, accept: bool, id: &str) -> Result<String, Refused> {
    let found = db
        .query_row(
            &format!("SELECT name, {STATE} FROM memory_candidates WHERE id = ?1"),
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| Refused::Unsent(error.to_string()))?;
    let (name, retire) = match found {
        None => return Err(Refused::Unknown(format!("no memory candidate {id}"))),
        Some((name, state)) if state == "pending" => (name, false),
        Some((name, state)) if state == "accepted" && !accept => (name, true),
        Some((_, state)) if state == "sending" => {
            return Err(Refused::Stale(format!(
                "candidate {id} is not on the log yet: try again once its runner reaches the log"
            )));
        }
        Some((_, state)) => return Err(Refused::Stale(format!("candidate {id} is {state}"))),
    };
    crate::link::refuse_while_unreachable(db).map_err(Refused::Unsent)?;
    let (kind, verb) = match (accept, retire) {
        (true, _) => (MEMORY_ACCEPTED, "accepted"),
        (false, false) => (MEMORY_DROPPED, "dropped"),
        (false, true) => (MEMORY_DROPPED, "retired"),
    };
    let mut data = json!({ "candidate": id });
    if retire {
        data["retire"] = json!(true);
    }
    let message = Message::new(kind, data).subject(name.clone());
    crate::runners::publish(log, &message).map_err(|unsent| match unsent {
        crate::runners::Unsent::Nothing(why) => Refused::Unsent(why),
        crate::runners::Unsent::Unconfirmed(why) => Refused::Unconfirmed(why),
    })?;
    Ok(format!(
        "{name} {verb}: every runner applies it as it reads the log"
    ))
}

/// The kit repository's branch that the daily pull request of accepted memories comes from.
pub(crate) const BRANCH: &str = "priory/memories";

/// An accepted candidate that decides its name: the last accepted in log order.
struct Winner {
    id: String,
    name: String,
    /// The memory's file, or none for a forget.
    file: Option<Vec<u8>>,
    key: String,
    runner: Option<String>,
    decided: String,
}

/// The accepted candidate that decides each name, by name, and the time of the last accept.
fn winners(db: &Connection) -> rusqlite::Result<(Vec<Winner>, Option<String>)> {
    let mut by_name = std::collections::BTreeMap::new();
    let mut last = None;
    let rows = db
        .prepare(
            "SELECT id, name, description, text, key, runner, decided FROM memory_candidates
             WHERE accepted IS NOT NULL AND dropped IS NULL ORDER BY accepted",
        )?
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let kept: Option<(String, String)> = row.get::<_, Option<String>>(2)?.zip(row.get(3)?);
            Ok(Winner {
                id: row.get(0)?,
                file: kept.map(|(description, text)| file(&name, &description, &text).into_bytes()),
                name,
                key: row.get(4)?,
                runner: row.get(5)?,
                decided: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for winner in rows {
        last = Some(winner.decided.clone());
        by_name.insert(winner.name.clone(), winner);
    }
    Ok((by_name.into_values().collect(), last))
}

/// The `open-memories-pr` action: commits the file of each accepted memory, and removes the file
/// of each accepted forget, under the kit's memories folder on top of its ref, pushes the commit to
/// the kit's `priory/memories` branch, and opens or updates one pull request from it on `repo`.
/// `unchanged` when the kit already holds them.
pub(crate) fn open_pull_request(
    db: &Connection,
    factory: &Factory,
    home: &std::path::Path,
    repo: Option<&str>,
) -> Result<crate::tools::Done, Box<dyn std::error::Error>> {
    let failed = crate::daemon::failed;
    let Some(settings) = factory.settings.as_ref().and_then(|s| s.value.kit.as_ref()) else {
        return Ok(failed("the factory names no [kit]".to_owned()));
    };
    let Some(repo) = repo else {
        return Ok(failed(
            "the task names no repo: the kit's GitHub owner/name".to_owned(),
        ));
    };
    let (winners, last) = winners(db)?;
    let Some(last) = last else {
        return Ok(unchanged());
    };
    let [_, _, (_, folder)] = settings.paths();
    let folder = folder.trim_start_matches("./").trim_end_matches('/');
    let path = |name: &str| match folder {
        "" | "." => format!("{name}.md"),
        folder => format!("{folder}/{name}.md"),
    };
    let files: Vec<(String, Option<Vec<u8>>)> = winners
        .iter()
        .map(|winner| (path(&winner.name), winner.file.clone()))
        .collect();
    let date = last
        .parse::<jiff::Timestamp>()
        .map(|time| format!("@{} +0000", time.as_second()))
        .map_err(|error| format!("candidate time {last:?}: {error}"))?;
    let (url, mirror) = crate::kit::place(settings, factory, home)?;
    let reference = settings.reference();
    let message = "Move the memories accepted on Priory into the kit\n";
    let made =
        match priory_runtime::runtime::kit_commit(&mirror, &url, reference, &files, message, &date)
        {
            Ok(Some(made)) => made,
            Ok(None) => return Ok(unchanged()),
            Err(error) => return Ok(failed(error)),
        };
    let mut lines = Vec::new();
    for (status, changed) in &made.changed {
        let Some(winner) = winners.iter().find(|winner| path(&winner.name) == *changed) else {
            continue;
        };
        let what = match status {
            'A' => "added",
            'D' => "removed",
            _ => "changed",
        };
        let runner = winner
            .runner
            .as_deref()
            .map(|runner| format!(" on runner {runner}"))
            .unwrap_or_default();
        lines.push(format!(
            "- `{}`: {what}. Candidate {}, written by task {}{runner}, accepted {}.",
            winner.name, winner.id, winner.key, winner.decided
        ));
    }
    let base = (reference != "HEAD").then_some(reference);
    let body = format!(
        "A person accepted these memories on Priory's log, so every run already gets them. Merging \
         this pull request moves them into the kit's `{folder}` folder.\n\n{}\n\nPriory rebuilds \
         this branch from `{}` each day, so a commit pushed to it is lost.",
        lines.join("\n"),
        base.unwrap_or("the kit's default branch"),
    );
    let pushed =
        priory_runtime::runtime::kit_push(&mirror, &url, &made.commit, BRANCH).and_then(|()| {
            priory_runtime::runtime::kit_pull_request(
                repo,
                BRANCH,
                base,
                "Accepted memories from Priory",
                &body,
            )
        });
    Ok(match pushed {
        Ok(number) => crate::tools::Done {
            status: "opened",
            pull_request: None,
            commit: Some(made.commit),
            error: None,
            stdout: Some(format!(
                "pull request {repo}#{number} from {BRANCH}: {} file(s) changed",
                made.changed.len()
            )),
            stack: Vec::new(),
        },
        Err(error) => crate::tools::Done {
            commit: Some(made.commit),
            ..failed(error)
        },
    })
}

fn unchanged() -> crate::tools::Done {
    crate::tools::Done {
        status: "unchanged",
        pull_request: None,
        commit: None,
        error: None,
        stdout: None,
        stack: Vec::new(),
    }
}
