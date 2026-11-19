use rusqlite::{Connection, params};

/// The most words a TL;DR holds when no tokenizer counts it.
pub const WORDS: usize = 180;

/// How many tasks a search prints when the caller sets no limit.
pub const LIMIT: usize = 10;

/// The length of `text` and the limit it must stay under, in the unit that limit counts.
pub struct Measure {
    pub count: usize,
    pub limit: usize,
    pub unit: &'static str,
}

/// Counts `text` against the TL;DR limit.
pub fn measure(text: &str) -> Measure {
    Measure {
        count: text.split_whitespace().count(),
        limit: WORDS,
        unit: "words",
    }
}

/// `text` cut to the TL;DR limit.
pub fn cut(text: &str) -> String {
    text.split_whitespace()
        .take(WORDS)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Sets task `task`'s TL;DR, refusing one over the limit with its count.
pub fn set(db: &Connection, task: i64, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a TL;DR needs text".to_owned());
    }
    let Measure { count, limit, unit } = measure(text);
    if count > limit {
        return Err(format!(
            "the TL;DR has {count} {unit}, over the limit of {limit}: shorten it"
        ));
    }
    let changed = db
        .execute(
            "UPDATE tasks SET tldr = ?2 WHERE id = ?1",
            params![task, text],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err(format!("no task {task}"));
    }
    Ok(format!("task {task} has its TL;DR"))
}

/// The FTS5 query for a person's words: any of them, each quoted, so no word reads as syntax.
fn any_word(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// The tasks matching `query`, best first, in `states` when it names any, as a table with a header.
pub fn search(
    db: &Connection,
    query: &str,
    states: &[String],
    limit: usize,
) -> Result<String, String> {
    let mut table = String::from("task\tkey\tstate\tstep\ttldr\n");
    let Some(matched) = any_word(query) else {
        return Ok(table);
    };
    let states = serde_json::to_string(states).expect("states serialize");
    let mut statement = db
        .prepare(
            "SELECT tasks.id, tasks.key, tasks.state, tasks.step, tasks.tldr, task_text.title,
               task_text.request
             FROM search JOIN tasks ON tasks.id = search.rowid
             JOIN task_text ON task_text.task = tasks.id
             WHERE search MATCH ?1
             AND (json_array_length(?2) = 0 OR tasks.state IN (SELECT value FROM json_each(?2)))
             ORDER BY bm25(search) LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![matched, states, limit as i64], |row| {
            let tldr: Option<String> = row.get(4)?;
            let shown = match tldr {
                Some(tldr) => tldr,
                None => fallback(&row.get::<_, String>(5)?, &row.get::<_, String>(6)?),
            };
            Ok([
                row.get::<_, i64>(0)?.to_string(),
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                shown.replace(['\n', '\t'], " "),
            ])
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<_>>>)
        .map_err(|error| error.to_string())?;
    for row in rows {
        table.push_str(&row.join("\t"));
        table.push('\n');
    }
    Ok(table)
}

/// What stands for a task's TL;DR until it has one: its request's title and opening text.
fn fallback(title: &str, request: &str) -> String {
    cut(&format!("{title} {request}"))
}
