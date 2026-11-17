use crate::factory::{Loaded, Workflow};
use crate::route::{self, Decision};
use herdr_bus::Message;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::BTreeMap;
use std::time::Duration;

const VERSION: i64 = 1;

const SCHEMA: &str = "
BEGIN;
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
COMMIT;
";

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
    let db = Connection::open(&path).map_err(at)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(at)?;
    db.pragma_update(None, "journal_mode", "WAL").map_err(at)?;
    db.pragma_update(None, "foreign_keys", true).map_err(at)?;
    let version: i64 = db
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(at)?;
    match version {
        0 => db.execute_batch(SCHEMA).map_err(at)?,
        VERSION => {}
        _ => {
            return Err(format!(
                "{}: state schema {version} is newer than this priory knows ({VERSION})",
                path.display()
            ));
        }
    }
    Ok(db)
}

/// Records `message` in the inbox and applies its route, in one transaction. A message
/// already in the inbox changes nothing.
pub fn handle(
    db: &mut Connection,
    message: &Message,
    workflows: &BTreeMap<String, Loaded<Workflow>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx = db.transaction()?;
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
    tx.execute(
        "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id.to_string(),
            message.kind,
            message.subject,
            message.time.to_string(),
            message.data.to_string(),
            serde_json::to_string(&message.links).expect("links serialize"),
            task,
            outcome
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
