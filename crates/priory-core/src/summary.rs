use crate::handler::{self, INTERRUPTED};
use crate::search;
use crate::stall::Group;
use crate::trace::{self, Trace};
use priory_factory::factory::Factory;
use rusqlite::{Connection, TransactionBehavior, params};

/// The most summarizer calls in flight at once.
pub(crate) const CALLS: usize = 2;

/// The model the summarizer calls: the `[search] tldr` agent's, else `haiku`. A factory with no
/// summarizer calls none.
pub(crate) fn model(factory: &Factory) -> Option<&str> {
    let agent = factory.agents.get(factory.search().tldr.as_ref()?)?;
    Some(agent.value.model.as_deref().unwrap_or(handler::MODEL))
}

/// The tasks that have no TL;DR and were never summarized, oldest first, with their request's title
/// and text: at most `room` of them.
pub(crate) fn due(db: &Connection, room: usize) -> rusqlite::Result<Vec<(i64, String, String)>> {
    let mut statement = db.prepare(
        "SELECT task, title, request FROM task_text JOIN tasks ON tasks.id = task_text.task
         WHERE tasks.tldr IS NULL AND NOT EXISTS (SELECT 1 FROM summaries WHERE summaries.task = tasks.id)
         ORDER BY task LIMIT ?1",
    )?;
    statement
        .query_map([room as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect()
}

pub(crate) fn skipped(db: &Connection, task: i64, why: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO summaries (task, answered, status)
         VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?2)",
        params![task, why],
    )
    .map(drop)
}

pub(crate) fn called(db: &Connection, task: i64) -> rusqlite::Result<i64> {
    db.query_row(
        "INSERT INTO summaries (task) VALUES (?1) RETURNING id",
        [task],
        |row| row.get(0),
    )
}

pub(crate) fn spawned(
    db: &Connection,
    id: i64,
    pid: u32,
    started: Option<i64>,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE summaries SET pid = ?2, pid_started = ?3 WHERE id = ?1",
        params![id, pid, started],
    )
    .map(drop)
}

/// The summarizer calls a stopped daemon left unanswered, with their process group.
pub(crate) fn orphans(db: &Connection) -> rusqlite::Result<Vec<(i64, Option<Group>)>> {
    let mut statement =
        db.prepare("SELECT id, pid, pid_started FROM summaries WHERE answered IS NULL")?;
    statement
        .query_map([], |row| {
            let pid: Option<u32> = row.get(1)?;
            Ok((row.get(0)?, pid.zip(row.get(2)?)))
        })?
        .collect()
}

/// What the summarizer reads: the request alone.
pub(crate) fn prompt(title: &str, request: &str) -> String {
    format!(
        "You write the TL;DR of a task for Priory, which runs coding agents. People and agents search \
         tasks by it. Read the request below and answer with the TL;DR alone: what the task is about, \
         in plain words, in at most {} words, with no preamble.\n\nThe request:\n{title}\n{request}\n",
        search::WORDS
    )
}

/// Records summarizer call `id`'s answer, from its JSON `output` and process `status`. A successful
/// answer, cut at the limit, becomes the task's TL;DR unless another writer set one meanwhile.
pub(crate) fn answered(
    db: &mut Connection,
    trace: &Trace,
    id: i64,
    output: Option<&str>,
    status: &str,
    counter: Option<&crate::embed::Counter>,
) -> rusqlite::Result<()> {
    let (parsed, usage) = handler::parse(output);
    let text = parsed["result"]
        .as_str()
        .map(|text| search::fit(text, counter))
        .unwrap_or_default();
    let status = match (handler::succeeded(&parsed, status), text.is_empty()) {
        (true, false) => "written".to_owned(),
        (true, true) => "the summarizer gave no text".to_owned(),
        (false, _) if status == INTERRUPTED => status.to_owned(),
        (false, _) => format!("the summarizer call failed: {status}"),
    };
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task: i64 = tx.query_row("SELECT task FROM summaries WHERE id = ?1", [id], |row| {
        row.get(0)
    })?;
    if status == "written" {
        tx.execute(
            "UPDATE tasks SET tldr = ?2 WHERE id = ?1 AND tldr IS NULL",
            params![task, text],
        )?;
    }
    tx.execute(
        "UPDATE summaries SET answered = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), status = ?2
         WHERE id = ?1",
        params![id, status],
    )?;
    let model = model(trace.factory).unwrap_or(handler::MODEL);
    trace::summarized(&tx, trace, task, &status, model, usage)?;
    tx.commit()
}
