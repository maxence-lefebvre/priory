use crate::room::{self, Share};
use crate::route::{self, Decision};
use crate::trace::Trace;
use herdr_bus::Message;
use priory_factory::factory::{Factory, Loaded, Session, Workflow};
use priory_factory::lanes::{Lanes, Subject};
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

const VERSION: i64 = 18;

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

const SCHEMA_4: &str = "
CREATE TABLE owns (
  task     INTEGER NOT NULL REFERENCES tasks (id),
  resource TEXT NOT NULL,
  PRIMARY KEY (task, resource)
);
CREATE INDEX owns_by_resource ON owns (resource);
INSERT INTO owns (task, resource) SELECT id, key FROM tasks;
CREATE TABLE waits (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  task     INTEGER NOT NULL REFERENCES tasks (id),
  run      INTEGER NOT NULL REFERENCES runs (id),
  kind     TEXT NOT NULL CHECK (kind IN ('pull_request', 'checks', 'review', 'time', 'task')),
  resource TEXT,
  until    TEXT,
  target   INTEGER REFERENCES tasks (id),
  head_sha TEXT,
  created  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  fired    TEXT,
  inbox    INTEGER REFERENCES inbox (seq)
);
PRAGMA user_version = 4;
";

const SCHEMA_5: &str = "
ALTER TABLE runs ADD COLUMN agent TEXT;
PRAGMA user_version = 5;
";

const SCHEMA_6: &str = "
ALTER TABLE waits ADD COLUMN daemon INTEGER NOT NULL DEFAULT 0;
ALTER TABLE results ADD COLUMN inbox INTEGER REFERENCES inbox (seq);
PRAGMA user_version = 6;
";

const SCHEMA_7: &str = "
ALTER TABLE runs ADD COLUMN turn TEXT;
ALTER TABLE runs ADD COLUMN prompted TEXT;
ALTER TABLE tasks ADD COLUMN changed TEXT;
UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now');
CREATE TRIGGER task_changed AFTER UPDATE OF state ON tasks WHEN NEW.state IS NOT OLD.state
  BEGIN UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = NEW.id; END;
PRAGMA user_version = 7;
";

const SCHEMA_8: &str = "
CREATE TABLE approvals (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  task       INTEGER NOT NULL REFERENCES tasks (id),
  step       TEXT NOT NULL,
  ask        INTEGER NOT NULL REFERENCES asks (id),
  revision   INTEGER NOT NULL,
  commit_sha TEXT,
  created    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
CREATE TABLE actions (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  task         INTEGER NOT NULL REFERENCES tasks (id),
  step         TEXT NOT NULL,
  action       TEXT NOT NULL,
  status       TEXT NOT NULL,
  pull_request TEXT,
  commit_sha   TEXT,
  error        TEXT,
  result       INTEGER REFERENCES results (run),
  inbox        INTEGER REFERENCES inbox (seq),
  created      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);
PRAGMA user_version = 8;
";

const SCHEMA_9: &str = "
CREATE TABLE shared_sessions (
  agent   TEXT PRIMARY KEY,
  session TEXT NOT NULL
);
CREATE UNIQUE INDEX one_live_run_per_session ON runs (session) WHERE ended IS NULL;
PRAGMA user_version = 9;
";

const SCHEMA_10: &str = "
ALTER TABLE tasks ADD COLUMN ending TEXT;
PRAGMA user_version = 10;
";

const SCHEMA_11: &str = "
CREATE TABLE trace (
  seq   INTEGER PRIMARY KEY AUTOINCREMENT,
  id    TEXT NOT NULL UNIQUE,
  type  TEXT NOT NULL,
  time  TEXT NOT NULL,
  event TEXT NOT NULL
);
ALTER TABLE tasks ADD COLUMN entered TEXT;
UPDATE tasks SET entered = changed;
ALTER TABLE tasks ADD COLUMN tokens TEXT;
ALTER TABLE tasks ADD COLUMN model TEXT;
ALTER TABLE runs ADD COLUMN transcript TEXT;
ALTER TABLE runs ADD COLUMN read INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 11;
";

const SCHEMA_12: &str = "
ALTER TABLE tasks ADD COLUMN position INTEGER;
CREATE TABLE bands (
  name TEXT PRIMARY KEY,
  rank INTEGER NOT NULL
);
PRAGMA user_version = 12;
";

const SCHEMA_13: &str = "
ALTER TABLE tasks ADD COLUMN team TEXT;
ALTER TABLE tasks ADD COLUMN project TEXT;
ALTER TABLE tasks ADD COLUMN lane_derived INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 13;
";

const SCHEMA_14: &str = "
ALTER TABLE runs ADD COLUMN thread TEXT;
PRAGMA user_version = 14;
";

const SCHEMA_15: &str = "
ALTER TABLE outbox ADD COLUMN nudge INTEGER NOT NULL DEFAULT 0;
ALTER TABLE runs ADD COLUMN nudgeable INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version = 15;
";

const SCHEMA_16: &str = "
ALTER TABLE tasks ADD COLUMN generation INTEGER NOT NULL DEFAULT 1;
ALTER TABLE asks ADD COLUMN withdrawn INTEGER;
ALTER TABLE waits ADD COLUMN withdrawn INTEGER;
PRAGMA user_version = 16;
";

const SCHEMA_17: &str = "
CREATE TABLE holds (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  task        INTEGER NOT NULL REFERENCES tasks (id),
  reason      TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('person', 'time', 'task', 'merge', 'blockers')),
  until       TEXT,
  target      INTEGER REFERENCES tasks (id),
  resource    TEXT,
  if_closed   TEXT CHECK (if_closed IN ('release', 'drop', 'ask')),
  resume      TEXT NOT NULL CHECK (resume IN ('queued', 'ready')),
  ask         INTEGER REFERENCES asks (id),
  created     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  released    TEXT,
  released_as TEXT
);
CREATE UNIQUE INDEX one_open_hold_per_task ON holds (task) WHERE released IS NULL;
PRAGMA user_version = 17;
";

const SCHEMA_18: &str = "
CREATE TABLE waits_18 (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  task      INTEGER NOT NULL REFERENCES tasks (id),
  run       INTEGER NOT NULL REFERENCES runs (id),
  kind      TEXT NOT NULL CHECK (kind IN ('pull_request', 'checks', 'review', 'time', 'task',
              'linear_comment', 'linear_state', 'slack_reply')),
  resource  TEXT,
  until     TEXT,
  target    INTEGER REFERENCES tasks (id),
  head_sha  TEXT,
  created   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
  fired     TEXT,
  inbox     INTEGER REFERENCES inbox (seq),
  daemon    INTEGER NOT NULL DEFAULT 0,
  withdrawn INTEGER,
  state     TEXT,
  since     TEXT
);
INSERT INTO waits_18 (id, task, run, kind, resource, until, target, head_sha, created, fired, inbox,
  daemon, withdrawn)
  SELECT id, task, run, kind, resource, until, target, head_sha, created, fired, inbox, daemon,
  withdrawn FROM waits;
DROP TABLE waits;
ALTER TABLE waits_18 RENAME TO waits;
PRAGMA user_version = 18;
";

/// Each schema change in order: a file at version `n` applies those from index `n` on.
const MIGRATIONS: [&str; VERSION as usize] = [
    SCHEMA, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5, SCHEMA_6, SCHEMA_7, SCHEMA_8, SCHEMA_9,
    SCHEMA_10, SCHEMA_11, SCHEMA_12, SCHEMA_13, SCHEMA_14, SCHEMA_15, SCHEMA_16, SCHEMA_17,
    SCHEMA_18,
];

/// The order the queue starts tasks in: band, then a person's order, then age. A band the factory
/// no longer has comes after the last one.
pub const START_ORDER: &str = "coalesce((SELECT rank FROM bands WHERE bands.name = tasks.band),
  (SELECT count(*) FROM bands)), tasks.position IS NULL, tasks.position, tasks.id";

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

pub(crate) struct Task {
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
    let Some(pending) = usize::try_from(version)
        .ok()
        .and_then(|done| MIGRATIONS.get(done..))
    else {
        return Err(format!(
            "{}: state schema {version} is newer than this priory knows ({VERSION})",
            path.display()
        ));
    };
    tx.execute_batch(&pending.concat()).map_err(at)?;
    tx.commit().map_err(at)?;
    Ok(db)
}

/// Records `message` in the inbox and applies its route, in one transaction. A message
/// already in the inbox changes nothing.
pub(crate) fn handle(
    db: &mut Connection,
    message: &Message,
    factory: &Factory,
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
    let resources = route::resources(message);
    let owner = owner(&tx, &resources)?;
    let (task, outcome) = match route::route(message, owner.as_ref(), factory)? {
        Decision::Create(new) => {
            tx.execute(
                "INSERT INTO tasks
                   (key, workflow, step, state, band, lane, team, project, lane_derived, trigger, source, entered)
                 VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?8, ?9, ?10, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    new.key,
                    new.workflow,
                    new.step,
                    new.band,
                    new.lane,
                    new.team,
                    new.project,
                    new.lane_derived,
                    new.trigger,
                    new.source
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
                params![id, new.key],
            )?;
            if let Some(hold) = &new.hold {
                crate::hold::filed(&tx, id, hold, factory)?;
            }
            (Some(id), "created")
        }
        Decision::Keep(id) if route::tick(message) => {
            // A human step or a daemon's checks gate has no agent turn for a tick to wake.
            let asleep_at: Option<(String, String)> = tx
                .query_row(
                    "SELECT workflow, step FROM tasks WHERE id = ?1 AND state = 'asleep'
                     AND NOT EXISTS (SELECT 1 FROM waits WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL AND daemon)",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let wakes = asleep_at.is_some_and(|(workflow, step)| {
                factory
                    .workflows
                    .get(&workflow)
                    .and_then(|workflow| workflow.value.step(&step))
                    .is_some_and(|step| step.decides == priory_factory::factory::Decider::Model)
            });
            (Some(id), if wakes { "woke" } else { "coalesced" })
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
    let seq = tx.last_insert_rowid();
    crate::hold::event(&tx, &resources)?;
    match (task, outcome) {
        (Some(task), "kept") => drop(crate::tools::event(&tx, task, seq)?),
        (Some(task), "woke") => {
            let line = line(
                &message.kind,
                message.subject.as_deref(),
                &message.data.to_string(),
            );
            let text = message.data["prompt"].as_str().unwrap_or(&line);
            crate::tools::wake_on(&tx, task, text, Some(seq))?;
        }
        _ => {}
    }
    Ok(tx.commit()?)
}

/// Records the factory's bands, highest first, so listings and moves read them without the factory.
pub(crate) fn bands(db: &mut Connection, bands: &[&str]) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM bands", [])?;
    for (rank, band) in bands.iter().enumerate() {
        tx.execute(
            "INSERT INTO bands (name, rank) VALUES (?1, ?2)",
            params![band, rank as i64],
        )?;
    }
    // Tasks from before bands have none: the last band is what a request without one gets.
    tx.execute(
        "UPDATE tasks SET band = ?1 WHERE band IS NULL",
        [bands.last()],
    )?;
    tx.commit()
}

/// Fires every time wait whose instant passed and task wait whose task ended.
pub(crate) fn due(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    crate::tools::due(&tx)?;
    crate::hold::due(&tx)?;
    tx.commit()
}

/// The oldest active task that owns any of `resources`.
pub(crate) fn owner(tx: &Transaction, resources: &[String]) -> rusqlite::Result<Option<Task>> {
    let resources = serde_json::to_string(resources).expect("resources serialize");
    tx.query_row(
        "SELECT tasks.id FROM owns JOIN tasks ON tasks.id = owns.task
         WHERE owns.resource IN (SELECT value FROM json_each(?1))
         AND tasks.state NOT IN ('done', 'failed', 'dropped') ORDER BY tasks.id LIMIT 1",
        [resources],
        |row| Ok(Task { id: row.get(0)? }),
    )
    .optional()
}

/// The rows of `query`, each column as text and `-` for null, tab-separated after `header`.
pub fn print(
    db: &Connection,
    header: &str,
    query: &str,
    params: impl rusqlite::Params,
) -> rusqlite::Result<()> {
    emit(header);
    let mut statement = db.prepare(query)?;
    let columns = statement.column_count();
    let mut rows = statement.query(params)?;
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
        emit(&cells.join("\t"));
    }
    Ok(())
}

/// Prints `line`. A reader that closed the pipe, as `head` does, ends the listing without a message.
pub fn emit(line: &str) {
    let Err(error) = writeln!(std::io::stdout(), "{line}") else {
        return;
    };
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("priory: stdout: {error}");
    std::process::exit(1);
}

/// Every trace record, oldest first, as CloudEvents JSON.
pub fn events(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare("SELECT event FROM trace ORDER BY seq")?;
    statement.query_map([], |row| row.get(0))?.collect()
}

/// One event as an agent reads it: type, subject and data on one line.
pub(crate) fn line(kind: &str, subject: Option<&str>, data: &str) -> String {
    format!("{kind} {} {data}", subject.unwrap_or("-"))
}

/// A queued task the daemon may start a run for.
pub(crate) struct Admission {
    pub task: i64,
    pub workflow: String,
    pub step: String,
    pub trigger: String,
    /// Queued, not ready: its run is the task's first and gets the step's prompt.
    pub fresh: bool,
}

/// The first queued or ready task at a model step, in start order, that fits in the runner's room.
/// Each one before it gets the reason it waits.
pub(crate) fn admissible<'r>(
    db: &Connection,
    factory: &Factory,
    share: &Share,
    room: impl Fn() -> &'r Result<Room, String>,
) -> rusqlite::Result<Option<Admission>> {
    let mut statement = db.prepare(&format!(
        "SELECT id, workflow, step, trigger, state = 'queued' FROM tasks
             WHERE state IN ('queued', 'ready') ORDER BY {START_ORDER}"
    ))?;
    let queued = statement
        .query_map([], |row| {
            Ok(Admission {
                task: row.get(0)?,
                workflow: row.get(1)?,
                step: row.get(2)?,
                trigger: row.get(3)?,
                fresh: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let promised = promised(db, factory)?;
    // The first task that waits for room: no task behind it starts, so it is never starved.
    let mut head = None;
    for admission in queued {
        let Some(step) = factory
            .workflows
            .get(&admission.workflow)
            .and_then(|workflow| workflow.value.step(&admission.step))
            .filter(|step| step.decides == priory_factory::factory::Decider::Model)
        else {
            continue;
        };
        let agent = step
            .agent
            .as_ref()
            .and_then(|name| Some((name, &factory.agents.get(name)?.value)));
        let busy = match agent {
            Some((name, agent)) if matches!(agent.session, Session::Shared) => serving(db, name)?
                .map(|task| {
                    format!("waits for agent {name}: its shared session serves task {task}")
                }),
            _ => None,
        };
        let waits = match (busy, head) {
            (Some(reason), _) => Some(reason),
            (None, Some(head)) => Some(format!("waits behind task {head}, which waits for room")),
            (None, None) => {
                let need = room::need(agent.map(|(_, agent)| agent));
                let (reason, holds) = match room() {
                    Ok(measured) => (
                        room::wait(measured, share, promised, need),
                        room::fits_ever(measured, share, need),
                    ),
                    Err(error) => (
                        Some(format!("waits for room: cannot measure it: {error}")),
                        true,
                    ),
                };
                // A task that can never fit would hold the queue forever.
                if reason.is_some() && holds {
                    head = Some(admission.task);
                }
                reason
            }
        };
        let Some(reason) = waits else {
            return Ok(Some(admission));
        };
        db.execute(
            "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
            params![admission.task, reason],
        )?;
    }
    Ok(None)
}

/// The task whose live run holds `agent`'s shared session, which runs one turn at a time.
fn serving(db: &Connection, agent: &str) -> rusqlite::Result<Option<i64>> {
    db.query_row(
        "SELECT task FROM runs WHERE agent = ?1 AND ended IS NULL",
        [agent],
        |row| row.get(0),
    )
    .optional()
}

/// The memory declared by the agents of live runs. No run reports its own use yet, so each one
/// keeps its whole declaration until it ends.
fn promised(db: &Connection, factory: &Factory) -> rusqlite::Result<u64> {
    let mut statement = db.prepare("SELECT agent FROM runs WHERE ended IS NULL")?;
    // A run recorded before schema 5 has no agent.
    let mut agents = statement.query_map([], |row| row.get::<_, Option<String>>(0))?;
    agents.try_fold(0, |sum: u64, agent| {
        let agent = agent?.and_then(|name| factory.agents.get(&name));
        Ok(sum.saturating_add(room::need(agent.map(|agent| &agent.value))))
    })
}

/// A task at a code step, whose action the daemon runs.
pub(crate) struct Due {
    pub task: i64,
    pub workflow: String,
    pub step: String,
    pub trigger: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

/// The oldest queued or ready task at a code step.
pub(crate) fn actionable(
    db: &Connection,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> rusqlite::Result<Option<Due>> {
    let mut statement = db.prepare(
        "SELECT id, workflow, step, trigger, branch, worktree FROM tasks
         WHERE state IN ('queued', 'ready') ORDER BY id",
    )?;
    let tasks = statement.query_map([], |row| {
        Ok(Due {
            task: row.get(0)?,
            workflow: row.get(1)?,
            step: row.get(2)?,
            trigger: row.get(3)?,
            branch: row.get(4)?,
            worktree: row.get(5)?,
        })
    })?;
    for due in tasks {
        let due = due?;
        let code = workflows
            .get(&due.workflow)
            .and_then(|workflow| workflow.value.step(&due.step))
            .is_some_and(|step| step.decides == priory_factory::factory::Decider::Code);
        if code {
            return Ok(Some(due));
        }
    }
    Ok(None)
}

/// The summary and evidence of the task's latest result.
pub(crate) fn last_result(
    db: &Connection,
    task: i64,
) -> rusqlite::Result<Option<(String, Option<String>)>> {
    db.query_row(
        "SELECT summary, evidence FROM results WHERE task = ?1 ORDER BY run DESC LIMIT 1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// What a new run needs recorded before its tmux session starts.
pub(crate) struct NewRun<'a> {
    pub task: i64,
    pub step: &'a str,
    pub agent: &'a str,
    pub tmux: &'a str,
    pub session: &'a str,
    pub repo: &'a str,
    pub branch: &'a str,
    pub worktree: &'a str,
    /// The step's prompt, for a fresh task. A ready task has its prompt queued.
    pub prompt: Option<&'a str>,
    /// How the step ends, which every prompt that wakes the task repeats.
    pub ending: &'a str,
    /// The session is the agent's shared one, which other tasks' runs resume too.
    pub shared: bool,
    /// Derives the lane of a task again, once, now that its branch is known.
    pub lanes: &'a Lanes,
}

/// The agent's shared session, once a run of it reached `SessionStart`, which every task resumes.
pub(crate) fn shared(db: &Connection, agent: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT session FROM shared_sessions WHERE agent = ?1
         AND EXISTS (SELECT 1 FROM runs WHERE runs.session = shared_sessions.session AND ready = 1)",
        [agent],
        |row| row.get(0),
    )
    .optional()
}

/// The harness session of the task's latest run of `agent` that Claude started, which a new run of
/// that agent resumes.
pub(crate) fn resumable(
    db: &Connection,
    task: i64,
    agent: &str,
) -> rusqlite::Result<Option<String>> {
    db.query_row(
        // A session the agent once shared stays shared, so two tasks cannot both resume it, even
        // after a harness change gave the agent another shared session.
        "SELECT session FROM runs WHERE task = ?1 AND agent = ?2 AND ready = 1
         AND session NOT IN (SELECT session FROM shared_sessions)
         AND session NOT IN (SELECT session FROM runs GROUP BY session HAVING count(DISTINCT task) > 1)
         ORDER BY id DESC LIMIT 1",
        params![task, agent],
        |row| row.get(0),
    )
    .optional()
}

/// The harness's own id for `session`, when it differs from Priory's, as Codex's thread id does.
pub(crate) fn thread(db: &Connection, session: &str) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT thread FROM runs WHERE session = ?1 AND thread IS NOT NULL ORDER BY id DESC LIMIT 1",
        [session],
        |row| row.get(0),
    )
    .optional()
}

/// Records a run and its first prompt: the step's prompt, if any, then the task's source event and
/// every event it kept that no prompt carried yet. Returns the run id.
pub(crate) fn start(db: &mut Connection, run: &NewRun) -> rusqlite::Result<i64> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "INSERT INTO runs (task, step, agent, tmux, session) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![run.task, run.step, run.agent, run.tmux, run.session],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "UPDATE tasks SET state = 'running', branch = ?2, worktree = ?3, reason = NULL, ending = ?4
         WHERE id = ?1",
        params![run.task, run.branch, run.worktree, run.ending],
    )?;
    // A schedule's tick wakes a task with its ask or wait still open: this run sleeps on them again.
    tx.execute(
        "UPDATE asks SET run = ?2 WHERE task = ?1 AND run IS NOT NULL AND answer IS NULL AND withdrawn IS NULL",
        params![run.task, id],
    )?;
    tx.execute(
        "UPDATE waits SET run = ?2 WHERE task = ?1 AND fired IS NULL AND withdrawn IS NULL AND NOT daemon",
        params![run.task, id],
    )?;
    let (key, team, project, derived): (String, Option<String>, Option<String>, bool) = tx
        .query_row(
            "SELECT key, team, project, lane_derived FROM tasks WHERE id = ?1",
            [run.task],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if derived {
        let lane = run.lanes.lane(&Subject {
            key: &key,
            team: team.as_deref(),
            project: project.as_deref(),
            branch: Some(run.branch),
        });
        tx.execute(
            "UPDATE tasks SET lane = ?2, lane_derived = 0 WHERE id = ?1",
            params![run.task, lane],
        )?;
    }
    tx.execute(
        "INSERT OR IGNORE INTO owns (task, resource) VALUES (?1, ?2)",
        params![run.task, route::branch(run.repo, run.branch)],
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
    if !text.is_empty() {
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
    }
    if run.shared {
        share(&tx, run)?;
    }
    tx.commit()?;
    Ok(id)
}

/// Records the run's session as its agent's shared one, and names the task in its first prompt.
fn share(tx: &Transaction, run: &NewRun) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO shared_sessions (agent, session) VALUES (?1, ?2)
         ON CONFLICT (agent) DO UPDATE SET session = excluded.session",
        params![run.agent, run.session],
    )?;
    let (key, trigger): (String, String) = tx.query_row(
        "SELECT key, trigger FROM tasks WHERE id = ?1",
        [run.task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let intro = format!(
        "This is task {} ({key}), from trigger {trigger}, in the worktree {}. Earlier turns of this conversation may belong to other tasks.",
        run.task, run.worktree
    );
    let prepended = tx.execute(
        "UPDATE outbox SET text = ?2 || char(10) || text WHERE id =
         (SELECT min(id) FROM outbox WHERE task = ?1 AND state = 'queued')",
        params![run.task, intro],
    )?;
    if prepended == 0 {
        tx.execute(
            "INSERT INTO outbox (task, text) VALUES (?1, ?2)",
            params![run.task, intro],
        )?;
    }
    Ok(())
}

/// Blocks `task` with `reason`, ending its live run first as `ended_as`, if it has one, after reading
/// its transcript.
pub(crate) fn block(
    db: &mut Connection,
    trace: &Trace,
    task: i64,
    ended_as: &str,
    reason: &str,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let live = tx
        .query_row(
            "SELECT id FROM runs WHERE task = ?1 AND ended IS NULL",
            [task],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(run) = live {
        // The step goes on after a resume, so the turn cut short still counts toward it.
        crate::trace::account(&tx, trace, run, None)?;
    }
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

pub(crate) struct LiveRun {
    pub id: i64,
    pub task: i64,
    pub step: String,
    pub agent: String,
    pub workflow: String,
    pub tmux: String,
    pub session: String,
    pub worktree: Option<String>,
    /// No `SessionStart` arrived within `READY_WITHIN` of the start.
    pub late: bool,
    /// How long its current turn has run, in seconds.
    pub turn: Option<i64>,
    /// Claude's message for the permission prompt it shows, if any.
    pub prompted: Option<String>,
}

pub(crate) const READY_WITHIN: &str = "60 seconds";

pub(crate) fn live(db: &Connection) -> rusqlite::Result<Vec<LiveRun>> {
    let mut statement = db.prepare(&format!(
        "SELECT runs.id, runs.task, runs.step, tasks.workflow, runs.tmux, runs.session, tasks.worktree,
         NOT runs.ready AND runs.started <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{READY_WITHIN}'),
         unixepoch('now') - unixepoch(runs.turn), runs.prompted, coalesce(runs.agent, '')
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL ORDER BY runs.id"
    ))?;
    statement
        .query_map([], |row| {
            Ok(LiveRun {
                id: row.get(0)?,
                task: row.get(1)?,
                step: row.get(2)?,
                workflow: row.get(3)?,
                tmux: row.get(4)?,
                session: row.get(5)?,
                worktree: row.get(6)?,
                late: row.get(7)?,
                turn: row.get(8)?,
                prompted: row.get(9)?,
                agent: row.get(10)?,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The session transcript Claude names in every hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// The harness's own session id, when it chose one apart from `session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// Applies a hook signal: the run is idle after `SessionStart` or `Stop`, and busy after
/// `UserPromptSubmit`, which also confirms its attempted prompt when the submitted text matches.
/// After `Stop`, the task moves on by the run's result or ask. Returns a tmux session to kill.
pub(crate) fn signal(
    db: &mut Connection,
    signal: &Signal,
    trace: &Trace,
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
            tx.execute(
                "UPDATE runs SET idle = 1, ready = 1, turn = NULL, nudgeable = 0, thread = coalesce(?2, thread)
                 WHERE id = ?1",
                params![run, signal.thread],
            )?;
            crate::trace::begin(&tx, trace, run, signal.transcript.as_deref())?;
        }
        "Stop" => {
            tx.execute("UPDATE runs SET idle = 1, turn = NULL WHERE id = ?1", [run])?;
            crate::trace::account(&tx, trace, run, signal.transcript.as_deref())?;
            kill = crate::tools::stop(&tx, &signal.session, trace)?;
            tx.execute("UPDATE runs SET nudgeable = 0 WHERE id = ?1", [run])?;
        }
        "UserPromptSubmit" => {
            tx.execute(
                &format!("UPDATE runs SET idle = 0, turn = {NOW}, nudgeable = 0 WHERE id = ?1"),
                [run],
            )?;
            let attempted = tx
                .query_row(
                    "SELECT id, text, nudge FROM outbox WHERE run = ?1 AND state = 'attempted'",
                    [run],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, bool>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((id, text, nudge)) = attempted
                && signal.prompt.as_deref().is_some_and(|prompt| {
                    pasted(prompt)
                        .split_whitespace()
                        .eq(text.split_whitespace())
                })
            {
                tx.execute(
                    &format!(
                        "UPDATE outbox SET state = 'confirmed', confirmed = {NOW} WHERE id = ?1"
                    ),
                    [id],
                )?;
                tx.execute(
                    "UPDATE runs SET nudgeable = ?2 WHERE id = ?1",
                    params![run, !nudge],
                )?;
            }
        }
        "PermissionPrompt" => {
            tx.execute(
                "UPDATE runs SET prompted = coalesce(?2, 'a permission prompt') WHERE id = ?1",
                params![run, signal.message],
            )?;
        }
        _ => {}
    }
    tx.commit()?;
    Ok(kill)
}

/// The prompt inside the `pasted_content` tag that Claude Code 2.1.328 wraps a paste in, before its
/// hook sees it; `prompt` itself when it has no such tag.
fn pasted(prompt: &str) -> &str {
    prompt
        .trim()
        .strip_prefix("<pasted_content")
        .and_then(|rest| rest.split_once('>'))
        .and_then(|(attributes, body)| body.strip_suffix(&format!("</pasted_content{attributes}>")))
        .unwrap_or(prompt)
}

/// Marks the oldest queued prompt of an idle live run `attempted` and the run busy, in one
/// transaction, before anything is typed. A task with an attempted or uncertain prompt gets none
/// while the process it went to may still hold its text: its run is live or moved on.
pub(crate) fn attempt(db: &mut Connection) -> rusqlite::Result<Option<(String, String, String)>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next = tx
        .query_row(
            "SELECT outbox.id, runs.id, runs.tmux, runs.session, outbox.text FROM runs
             JOIN outbox ON outbox.task = runs.task AND outbox.state = 'queued'
             WHERE runs.ended IS NULL AND runs.idle = 1 AND NOT EXISTS (
               SELECT 1 FROM outbox AS open JOIN runs AS typed ON typed.id = open.run
               WHERE open.task = runs.task AND open.state IN ('attempted', 'uncertain')
               AND (typed.ended IS NULL OR typed.ended_as = 'moved'))
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

/// The tmux sessions and harness sessions of ended runs that no live run carries on in that tmux
/// session. A shared harness session lives on in another task's tmux session.
pub(crate) fn ended_sessions(
    db: &Connection,
) -> rusqlite::Result<std::collections::BTreeSet<(String, String)>> {
    let mut statement = db.prepare(
        "SELECT DISTINCT tmux, session FROM runs WHERE ended IS NOT NULL
         AND (tmux, session) NOT IN (SELECT tmux, session FROM runs WHERE ended IS NULL)",
    )?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// After a restart, a prompt attempted but never confirmed may or may not have been typed.
pub(crate) fn uncertain(db: &Connection) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE outbox SET state = 'uncertain' WHERE state = 'attempted'",
        [],
    )
}

/// Where `priory move` puts a task in its band's person order.
pub enum Place {
    Top,
    Before(i64),
    After(i64),
}

/// Moves the active `task` to `band`, at `place` in that band's person order. Without a band it
/// stays in its own, or joins the band of the task it goes before or after. Without a place it
/// goes after the tasks a person ordered, by age.
pub fn move_task(
    db: &mut Connection,
    task: i64,
    band: Option<&str>,
    place: Option<Place>,
) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    moved(&tx, task, band, place)?;
    tx.commit().map_err(|error| error.to_string())
}

fn moved(
    tx: &Transaction,
    task: i64,
    band: Option<&str>,
    place: Option<Place>,
) -> Result<(), String> {
    let error = |error: rusqlite::Error| error.to_string();
    let band_of = |id: i64| -> Result<String, String> {
        tx.query_row(
            "SELECT coalesce(band, '') FROM tasks WHERE id = ?1
             AND state NOT IN ('done', 'failed', 'dropped')",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(error)?
        .ok_or_else(|| format!("task {id} is not active"))
    };
    let own = band_of(task)?;
    if let Some(band) = band {
        let bands: Vec<String> = tx
            .prepare("SELECT name FROM bands ORDER BY rank")
            .and_then(|mut statement| statement.query_map([], |row| row.get(0))?.collect())
            .map_err(error)?;
        if !bands.iter().any(|known| known == band) {
            return Err(format!("band `{band}` is not one of {}", bands.join(", ")));
        }
    }
    let other = match place {
        Some(Place::Before(other) | Place::After(other)) if other == task => {
            return Err(format!("task {task} cannot move next to itself"));
        }
        Some(Place::Before(other) | Place::After(other)) => Some((other, band_of(other)?)),
        _ => None,
    };
    let target = match (band, &other) {
        (Some(band), Some((other, theirs))) if band != theirs => {
            return Err(format!("task {other} is in band `{theirs}`, not `{band}`"));
        }
        (Some(band), _) => band.to_owned(),
        (None, Some((_, theirs))) => theirs.clone(),
        (None, None) => own,
    };
    let Some(place) = place else {
        tx.execute(
            "UPDATE tasks SET band = ?2, position = NULL WHERE id = ?1",
            params![task, target],
        )
        .map_err(error)?;
        return Ok(());
    };
    let mut order: Vec<i64> = tx
        .prepare(&format!(
            "SELECT id FROM tasks WHERE coalesce(band, '') = ?1 AND id != ?2
             AND state NOT IN ('done', 'failed', 'dropped') ORDER BY {START_ORDER}"
        ))
        .and_then(|mut statement| {
            statement
                .query_map(params![target, task], |row| row.get(0))?
                .collect()
        })
        .map_err(error)?;
    let at = match place {
        Place::Top => 0,
        Place::Before(other) | Place::After(other) => {
            let at = order
                .iter()
                .position(|id| *id == other)
                .expect("other is active in the band");
            at + usize::from(matches!(place, Place::After(_)))
        }
    };
    order.insert(at, task);
    tx.execute(
        "UPDATE tasks SET band = ?2 WHERE id = ?1",
        params![task, target],
    )
    .map_err(error)?;
    for (position, id) in order.iter().enumerate() {
        tx.execute(
            "UPDATE tasks SET position = ?2 WHERE id = ?1",
            params![id, position as i64 + 1],
        )
        .map_err(error)?;
    }
    Ok(())
}
