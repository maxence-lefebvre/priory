//! To-dos: items a person owns on the desk, with no task or agent attached.

use priory_factory::factory::Factory;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

/// A to-do write on the daemon's socket: `{"todo": {"add": {...}}}` or `{"todo": {"done": ID}}`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub todo: Change,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Change {
    Add(Add),
    Done(i64),
}

/// `priory todo add`'s fields, and `POST /api/todos`'s body. `due` is a UTC time.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

/// A hook's report that a run called a tool of this MCP server: `{"connector_used": "linear"}`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorLine {
    pub connector_used: String,
}

/// `PRIORY_CONNECTORS`: the connectors with an open sign-in to-do, which a run's hook watches for.
pub(crate) fn connectors_env(db: &Connection) -> rusqlite::Result<Option<String>> {
    let mut statement =
        db.prepare("SELECT connector FROM todos WHERE connector IS NOT NULL AND done IS NULL")?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((!names.is_empty()).then(|| format!("PRIORY_CONNECTORS={}", names.join(","))))
}

/// A run used `connector` without a denial, so its sign-in to-do is done.
pub(crate) fn connector_used(
    db: &Connection,
    connector: &str,
) -> rusqlite::Result<Result<String, String>> {
    db.execute(
        "UPDATE todos SET done = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE connector = ?1 AND done IS NULL",
        [connector],
    )?;
    Ok(Ok(String::new()))
}

/// Adds a to-do and replies with its id, or marks one done.
pub(crate) fn change(
    db: &Connection,
    line: &Line,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    match &line.todo {
        Change::Add(add) => add_todo(db, add, factory),
        Change::Done(id) => done(db, *id),
    }
}

fn add_todo(
    db: &Connection,
    add: &Add,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let text = add.text.trim();
    if text.is_empty() || text.contains('\n') {
        return Ok(Err("a to-do is one line of text".to_owned()));
    }
    let bands = factory.bands();
    if let Some(band) = add.band.as_deref().filter(|band| !bands.contains(band)) {
        return Ok(Err(format!(
            "band `{band}` is not one of {}",
            bands.join(", ")
        )));
    }
    let due = match crate::snooze::until(None, add.due.as_deref()) {
        Ok(due) => due,
        Err(error) => return Ok(Err(error)),
    };
    let id: i64 = db.query_row(
        "INSERT INTO todos (text, band, due) VALUES (?1, ?2, ?3) RETURNING id",
        params![text, add.band, due],
        |row| row.get(0),
    )?;
    Ok(Ok(id.to_string()))
}

fn done(db: &Connection, id: i64) -> rusqlite::Result<Result<String, String>> {
    let found: Option<bool> = db
        .query_row(
            "SELECT done IS NOT NULL FROM todos WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    match found {
        None => Ok(Err(format!("there is no to-do {id}"))),
        Some(true) => Ok(Err(format!("to-do {id} is already done"))),
        Some(false) => {
            db.execute(
                "UPDATE todos SET done = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
                [id],
            )?;
            Ok(Ok(format!("to-do {id} is done")))
        }
    }
}

/// Every to-do, open ones first, each in the order it was added.
pub fn list(db: &Connection) -> rusqlite::Result<Value> {
    crate::listing::rows(
        db,
        "SELECT id, text, band, due, created, done FROM todos ORDER BY done IS NOT NULL, id",
        [],
    )
}
