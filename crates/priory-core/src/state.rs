use crate::factory::{Loaded, Workflow};
use crate::route::{self, Decision};
use herdr_bus::Message;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::time::Duration;

const VERSION: i64 = 3;

const SCHEMA: &str = "
CREATE TABLE tasks (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  key      TEXT NOT NULL,
  workflow TEXT NOT NULL,
  step     TEXT NOT NULL,
  state    TEXT NOT NULL,
  band     TEXT,
  lane     TEXT,
  trigger  TEXT NOT NULL,
  source   TEXT NOT NULL
);
CREATE UNIQUE INDEX one_active_task_per_key ON tasks (key)
  WHERE state NOT IN ('done', 'failed', 'dropped');
CREATE TABLE inbox (
  seq     INTEGER PRIMARY KEY AUTOINCREMENT,
  id      TEXT NOT NULL UNIQUE,
  type    TEXT NOT NULL,
  subject TEXT,
  time    TEXT NOT NULL,
  data    TEXT NOT NULL,
  links   TEXT NOT NULL,
  task    INTEGER REFERENCES tasks (id),
  outcome TEXT NOT NULL
);
PRAGMA user_version = 1;
";

const SCHEMA_2: &str = "
CREATE TABLE runs (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  task     INTEGER NOT NULL REFERENCES tasks (id),
  step     TEXT NOT NULL,
  tmux     TEXT NOT NULL,
  session  TEXT NOT NULL,
  idle     INTEGER NOT NULL DEFAULT 0,
  ready    INTEGER NOT NULL DEFAULT 0,
  started  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  ended    TEXT,
  ended_as TEXT
);
CREATE UNIQUE INDEX one_live_run_per_task ON runs (task) WHERE ended IS NULL;
CREATE TABLE outbox (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  task      INTEGER NOT NULL REFERENCES tasks (id),
  run       INTEGER REFERENCES runs (id),
  text      TEXT NOT NULL,
  state     TEXT NOT NULL DEFAULT 'queued',
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  attempted TEXT,
  confirmed TEXT
);
ALTER TABLE tasks ADD COLUMN branch TEXT;
ALTER TABLE tasks ADD COLUMN worktree TEXT;
ALTER TABLE tasks ADD COLUMN reason TEXT;
ALTER TABLE inbox ADD COLUMN outbox INTEGER REFERENCES outbox (id);
PRAGMA user_version = 2;
";

const SCHEMA_3: &str = "
CREATE TABLE results (
  run        INTEGER PRIMARY KEY REFERENCES runs (id),
  task       INTEGER NOT NULL REFERENCES tasks (id),
  step       TEXT NOT NULL,
  outcome    TEXT NOT NULL,
  summary    TEXT NOT NULL,
  evidence   TEXT,
  commit_sha TEXT,
  created    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TRIGGER results_are_immutable BEFORE UPDATE ON results
  BEGIN SELECT RAISE(ABORT, 'results are immutable'); END;
CREATE TRIGGER results_stay BEFORE DELETE ON results
  BEGIN SELECT RAISE(ABORT, 'results are immutable'); END;
CREATE TABLE asks (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  task           INTEGER NOT NULL REFERENCES tasks (id),
  run            INTEGER REFERENCES runs (id),
  step           TEXT NOT NULL,
  kind           TEXT NOT NULL CHECK (kind IN ('step', 'agent')),
  revision       INTEGER NOT NULL DEFAULT 1,
  question       TEXT NOT NULL,
  options        TEXT NOT NULL,
  recommendation TEXT,
  tradeoff       TEXT,
  answer         TEXT,
  answered       TEXT,
  created        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
PRAGMA user_version = 3;
";

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

pub struct Task {
    pub id: i64,
}

/// Opens the state file `PRIORY_DB`, creating its schema on first use.
pub fn open() -> Result<Connection, String> {
    let path = std::env::var_os("PRIORY_DB")
        // SQLite keeps nothing on disk for "", ":memory:" or a URI such as "file::memory:".
        .filter(|path| {
            let name = path.to_string_lossy();
            !name.is_empty() && name != ":memory:" && !name.starts_with("file:")
        })
        .ok_or("PRIORY_DB is not set to a file: name the state file")?;
    let at = |error: rusqlite::Error| format!("{}: {error}", path.display());
    let mut db = Connection::open(&path).map_err(at)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(at)?;
    // Setting the mode takes a lock that a crashed daemon's WAL recovery can hold past the timeout.
    let mode: String = db
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(at)?;
    if mode != "wal" {
        db.pragma_update(None, "journal_mode", "WAL").map_err(at)?;
    }
    db.pragma_update(None, "foreign_keys", true).map_err(at)?;
    let version = |db: &Connection| -> rusqlite::Result<i64> {
        db.pragma_query_value(None, "user_version", |row| row.get(0))
    };
    if version(&db).map_err(at)? == VERSION {
        return Ok(db);
    }
    // Immediate, so two processes opening a new file do not both create its schema.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(at)?;
    let version = version(&tx).map_err(at)?;
    match version {
        0 => tx
            .execute_batch(&format!("{SCHEMA}{SCHEMA_2}{SCHEMA_3}"))
            .map_err(at)?,
        1 => tx
            .execute_batch(&format!("{SCHEMA_2}{SCHEMA_3}"))
            .map_err(at)?,
        2 => tx.execute_batch(SCHEMA_3).map_err(at)?,
        VERSION => {}
        _ => {
            return Err(format!(
                "{}: state schema {version} is newer than this priory knows ({VERSION})",
                path.display()
            ));
        }
    }
    tx.commit().map_err(at)?;
    Ok(db)
}

/// Records `message` in the inbox and applies its route, in one transaction. A message
/// already in the inbox changes nothing.
pub fn handle(
    db: &mut Connection,
    message: &Message,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let seen = tx
        .query_row(
            "SELECT 1 FROM inbox WHERE id = ?1",
            [message.id.to_string()],
            |_| Ok(()),
        )
        .optional()?;
    if seen.is_some() {
        return Ok(());
    }
    let owner = match route::key(message) {
        Some(key) => active(&tx, &key)?,
        None => None,
    };
    let (task, outcome) = match route::route(message, owner.as_ref(), workflows)? {
        Decision::Create(new) => {
            tx.execute(
                "INSERT INTO tasks (key, workflow, step, state, band, lane, trigger, source)
                 VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7)",
                params![
                    new.key,
                    new.workflow,
                    new.step,
                    new.band,
                    new.lane,
                    new.trigger,
                    new.source
                ],
            )?;
            (Some(tx.last_insert_rowid()), "created")
        }
        Decision::Keep(id) => (Some(id), "kept"),
        Decision::Ignore => (None, "ignored"),
    };
    let running = match task {
        Some(id) => tx
            .query_row(
                "SELECT 1 FROM tasks WHERE id = ?1 AND state = 'running'",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some(),
        None => false,
    };
    let outbox = if running && outcome == "kept" {
        let text = line(
            &message.kind,
            message.subject.as_deref(),
            &message.data.to_string(),
        );
        tx.execute(
            "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
            params![task, text],
        )?;
        Some(tx.last_insert_rowid())
    } else {
        None
    };
    tx.execute(
        "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome, outbox)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            message.id.to_string(),
            message.kind,
            message.subject,
            message.time.to_string(),
            message.data.to_string(),
            serde_json::to_string(&message.links).expect("links serialize"),
            task,
            outcome,
            outbox
        ],
    )?;
    Ok(tx.commit()?)
}

fn active(tx: &Transaction, key: &str) -> rusqlite::Result<Option<Task>> {
    tx.query_row(
        "SELECT id FROM tasks WHERE key = ?1 AND state NOT IN ('done', 'failed', 'dropped')",
        [key],
        |row| Ok(Task { id: row.get(0)? }),
    )
    .optional()
}

/// The rows of `query`, each column as text and `-` for null, tab-separated after `header`.
pub fn print(db: &Connection, header: &str, query: &str) -> rusqlite::Result<()> {
    println!("{header}");
    let mut statement = db.prepare(query)?;
    let columns = statement.column_count();
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let cells = (0..columns)
            .map(|i| {
                Ok(match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => "-".to_owned(),
                    rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                    value => value.as_str()?.to_owned(),
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        println!("{}", cells.join("\t"));
    }
    Ok(())
}

/// One event as an agent reads it: type, subject and data on one line.
fn line(kind: &str, subject: Option<&str>, data: &str) -> String {
    format!("{kind} {} {data}", subject.unwrap_or("-"))
}

/// A queued task the daemon may start a run for.
pub struct Admission {
    pub task: i64,
    pub workflow: String,
    pub step: String,
    pub trigger: String,
}

/// The oldest queued or ready task at a model step, when no run is live.
pub fn admissible(
    db: &Connection,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> rusqlite::Result<Option<Admission>> {
    // shortcut: one live run at a time stands in for R4.3's room check; replace it with that check.
    let live = db
        .query_row("SELECT 1 FROM runs WHERE ended IS NULL", [], |_| Ok(()))
        .optional()?;
    if live.is_some() {
        return Ok(None);
    }
    let mut statement = db.prepare(
        "SELECT id, workflow, step, trigger FROM tasks WHERE state IN ('queued', 'ready') ORDER BY id",
    )?;
    let queued = statement.query_map([], |row| {
        Ok(Admission {
            task: row.get(0)?,
            workflow: row.get(1)?,
            step: row.get(2)?,
            trigger: row.get(3)?,
        })
    })?;
    for admission in queued {
        let admission = admission?;
        let model = workflows.get(&admission.workflow).is_some_and(|workflow| {
            workflow.value.steps.iter().any(|step| {
                step.id == admission.step && step.decides == crate::factory::Decider::Model
            })
        });
        if model {
            return Ok(Some(admission));
        }
    }
    Ok(None)
}

/// What a new run needs recorded before its tmux session starts.
pub struct NewRun<'a> {
    pub task: i64,
    pub step: &'a str,
    pub tmux: &'a str,
    pub session: &'a str,
    pub branch: &'a str,
    pub worktree: &'a str,
    /// The step's prompt, for a new session. A resumed session has its waking prompt queued.
    pub prompt: Option<&'a str>,
}

/// The harness session of the task's latest run that Claude started, which a new run resumes.
pub fn resumable(db: &Connection, task: i64) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT session FROM runs WHERE task = ?1 AND ready = 1 ORDER BY id DESC LIMIT 1",
        [task],
        |row| row.get(0),
    )
    .optional()
}

/// Records a run and its first prompt: the step's prompt, if any, then the task's source event and
/// every event it kept that no prompt carried yet. Returns the run id.
pub fn start(db: &mut Connection, run: &NewRun) -> rusqlite::Result<i64> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO runs (task, step, tmux, session) VALUES (?1, ?2, ?3, ?4)",
        params![run.task, run.step, run.tmux, run.session],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE tasks SET state = 'running', branch = ?2, worktree = ?3, reason = NULL WHERE id = ?1",
        params![run.task, run.branch, run.worktree],
    )?;
    let mut events = Vec::new();
    let mut text = run.prompt.unwrap_or_default().to_owned();
    {
        let mut statement = tx.prepare(
            "SELECT seq, type, subject, data FROM inbox WHERE outbox IS NULL
             AND ((task = ?1 AND outcome = 'kept') OR id = (SELECT source FROM tasks WHERE id = ?1))
             ORDER BY seq",
        )?;
        let mut rows = statement.query([run.task])?;
        while let Some(row) = rows.next()? {
            events.push(row.get::<_, i64>(0)?);
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&line(
                &row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.as_deref(),
                &row.get::<_, String>(3)?,
            ));
        }
    }
    if text.is_empty() {
        tx.commit()?;
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
        params![run.task, text],
    )?;
    let outbox = tx.last_insert_rowid();
    for seq in events {
        tx.execute(
            "UPDATE inbox SET outbox = ?1 WHERE seq = ?2",
            params![outbox, seq],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

/// Blocks `task` with `reason`, ending its live run first as `ended_as`, if it has one.
pub fn block(db: &mut Connection, task: i64, ended_as: &str, reason: &str) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        &format!("UPDATE runs SET ended = {NOW}, ended_as = ?2 WHERE task = ?1 AND ended IS NULL"),
        params![task, ended_as],
    )?;
    tx.execute(
        "UPDATE tasks SET state = 'blocked', reason = ?2 WHERE id = ?1",
        params![task, reason],
    )?;
    tx.commit()
}

pub struct LiveRun {
    pub id: i64,
    pub task: i64,
    pub tmux: String,
    pub session: String,
    pub worktree: Option<String>,
    /// No `SessionStart` arrived within `READY_WITHIN` of the start.
    pub late: bool,
}

pub const READY_WITHIN: &str = "60 seconds";

pub fn live(db: &Connection) -> rusqlite::Result<Vec<LiveRun>> {
    let mut statement = db.prepare(&format!(
        "SELECT runs.id, runs.task, runs.tmux, runs.session, tasks.worktree,
         NOT runs.ready AND runs.started <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{READY_WITHIN}')
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL ORDER BY runs.id"
    ))?;
    statement
        .query_map([], |row| {
            Ok(LiveRun {
                id: row.get(0)?,
                task: row.get(1)?,
                tmux: row.get(2)?,
                session: row.get(3)?,
                worktree: row.get(4)?,
                late: row.get(5)?,
            })
        })?
        .collect()
}

/// A signal from a run's harness, as `priory hook` sends it on the socket.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Signal {
    pub event: String,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// Applies a hook signal: the run is idle after `SessionStart` or `Stop`, and busy after
/// `UserPromptSubmit`, which also confirms its attempted prompt when the submitted text matches.
/// After `Stop`, the task moves on by the run's result or ask. Returns a tmux session to kill.
pub fn signal(
    db: &mut Connection,
    signal: &Signal,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> rusqlite::Result<Option<String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(run) = tx
        .query_row(
            "SELECT id FROM runs WHERE session = ?1 AND ended IS NULL",
            [&signal.session],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut kill = None;
    match signal.event.as_str() {
        "SessionStart" => {
            tx.execute("UPDATE runs SET idle = 1, ready = 1 WHERE id = ?1", [run])?;
        }
        "Stop" => {
            tx.execute("UPDATE runs SET idle = 1 WHERE id = ?1", [run])?;
            kill = crate::tools::stop(&tx, &signal.session, workflows)?;
        }
        "UserPromptSubmit" => {
            tx.execute("UPDATE runs SET idle = 0 WHERE id = ?1", [run])?;
            let attempted = tx
                .query_row(
                    "SELECT id, text FROM outbox WHERE run = ?1 AND state = 'attempted'",
                    [run],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            if let Some((id, text)) = attempted
                && signal
                    .prompt
                    .as_deref()
                    .is_some_and(|prompt| prompt.split_whitespace().eq(text.split_whitespace()))
            {
                tx.execute(
                    &format!(
                        "UPDATE outbox SET state = 'confirmed', confirmed = {NOW} WHERE id = ?1"
                    ),
                    [id],
                )?;
            }
        }
        _ => {}
    }
    tx.commit()?;
    Ok(kill)
}

/// Marks the oldest queued prompt of an idle live run `attempted` and the run busy, in one
/// transaction, before anything is typed. A run with an attempted or uncertain prompt gets none.
pub fn attempt(db: &mut Connection) -> rusqlite::Result<Option<(String, String, String)>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next = tx
        .query_row(
            "SELECT outbox.id, runs.id, runs.tmux, runs.session, outbox.text FROM runs
             JOIN outbox ON outbox.task = runs.task AND outbox.state = 'queued'
             WHERE runs.ended IS NULL AND runs.idle = 1 AND NOT EXISTS (
               SELECT 1 FROM outbox AS open WHERE open.task = runs.task
               AND open.state IN ('attempted', 'uncertain'))
             ORDER BY outbox.id LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((outbox, run, tmux, session, text)) = next else {
        return Ok(None);
    };
    tx.execute(
        &format!(
            "UPDATE outbox SET state = 'attempted', run = ?2, attempted = {NOW} WHERE id = ?1"
        ),
        params![outbox, run],
    )?;
    tx.execute("UPDATE runs SET idle = 0 WHERE id = ?1", [run])?;
    tx.commit()?;
    Ok(Some((tmux, session, text)))
}

/// The harness sessions of ended runs that no live run carries on.
pub fn ended_sessions(db: &Connection) -> rusqlite::Result<std::collections::BTreeSet<String>> {
    let mut statement = db.prepare(
        "SELECT DISTINCT session FROM runs WHERE ended IS NOT NULL
         AND session NOT IN (SELECT session FROM runs WHERE ended IS NULL)",
    )?;
    statement.query_map([], |row| row.get(0))?.collect()
}

/// After a restart, a prompt attempted but never confirmed may or may not have been typed.
pub fn uncertain(db: &Connection) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE outbox SET state = 'uncertain' WHERE state = 'attempted'",
        [],
    )
}
