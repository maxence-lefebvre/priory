use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;

/// The most words a TL;DR holds when no tokenizer counts it.
pub const WORDS: usize = 180;

/// How many tasks a search prints when the caller sets no limit.
pub const LIMIT: usize = 10;

/// Reciprocal rank fusion's constant: a match's score is `1 / (K + rank)` in each ranking.
const K: f64 = 60.0;

/// The most matches each ranking hands to the fusion.
const RANKED: usize = 200;

// shortcut: one fixed floor for every model; make it a `[search]` field if a model needs another.
/// The least cosine similarity a task needs to match by vector.
const SIMILAR: f32 = 0.3;

/// The length of `text` and the limit it must stay under, in the unit that limit counts.
pub struct Measure {
    pub count: usize,
    pub limit: usize,
    pub unit: &'static str,
}

/// Counts `text` against the TL;DR limit: in `tokens` when a tokenizer counted them, else in words.
pub fn measure(text: &str, tokens: Option<(usize, usize)>) -> Measure {
    match tokens {
        Some((count, limit)) => Measure {
            count,
            limit,
            unit: "tokens",
        },
        None => Measure {
            count: text.split_whitespace().count(),
            limit: WORDS,
            unit: "words",
        },
    }
}

/// `text` cut to the word limit.
pub fn cut(text: &str) -> String {
    text.split_whitespace()
        .take(WORDS)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `text` cut to the TL;DR limit: the word limit, then the token limit `tokens` gives, if any.
pub fn fit(text: &str, tokens: &mut dyn FnMut(&str) -> Option<(usize, usize)>) -> String {
    let mut words: Vec<&str> = text.split_whitespace().take(WORDS).collect();
    while !words.is_empty() && tokens(&words.join(" ")).is_some_and(|(count, limit)| count > limit)
    {
        words.pop();
    }
    words.join(" ")
}

/// The token count and limit of `text` for the factory's embedder, when embeddings are on and it
/// has a tokenizer.
pub fn tokens(
    db: &Connection,
    embedders: &crate::embed::Shared,
    text: &str,
) -> Option<(usize, usize)> {
    let (name, _) = embedder(db).ok()??;
    crate::embed::tokens_shared(embedders, &name, text)
}

/// Sets task `task`'s TL;DR, refusing one over the limit with its count. `tokens` counts its tokens
/// and gives the limit, when the embedding model's tokenizer is at hand.
pub fn set(
    db: &Connection,
    task: i64,
    text: &str,
    tokens: Option<(usize, usize)>,
) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a TL;DR needs text".to_owned());
    }
    let Measure { count, limit, unit } = measure(text, tokens);
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

/// Records the factory's embedder, by name, or that embeddings are off. A new embedder forgets the
/// last one's error.
pub fn configure(db: &Connection, name: Option<&str>) -> rusqlite::Result<()> {
    match name {
        Some(name) => db.execute(
            "INSERT INTO embedder (id, name) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET name = ?1, error = NULL WHERE name IS NOT ?1",
            [name],
        ),
        None => db.execute("DELETE FROM embedder", []),
    }
    .map(drop)
}

/// The factory's embedder and why it is unavailable, if it is; `None` with embeddings off.
pub fn embedder(db: &Connection) -> rusqlite::Result<Option<(String, Option<String>)>> {
    db.query_row("SELECT name, error FROM embedder", [], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })
    .optional()
}

/// Records why the embedder `name` failed, or `None` once it works. Returns whether that changed.
pub fn failed(db: &Connection, name: &str, error: Option<&str>) -> rusqlite::Result<bool> {
    db.execute(
        "UPDATE embedder SET error = ?2 WHERE name = ?1 AND error IS NOT ?2",
        params![name, error],
    )
    .map(|changed| changed > 0)
}

/// Each task whose vector is missing, or was made from another text or by another embedder than
/// `name`, with the text to embed: its TL;DR, else its request's title and opening text.
pub fn stale(db: &Connection, name: &str) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut statement = db.prepare(
        "SELECT task_text.task, tasks.tldr, task_text.title, task_text.request, vectors.text,
           vectors.embedder IS ?1
         FROM task_text JOIN tasks ON tasks.id = task_text.task
         LEFT JOIN vectors ON vectors.task = task_text.task ORDER BY task_text.task",
    )?;
    let rows = statement.query_map([name], |row| {
        let tldr: Option<String> = row.get(1)?;
        let text = match tldr {
            Some(tldr) => tldr,
            None => fallback(&row.get::<_, String>(2)?, &row.get::<_, String>(3)?),
        };
        let current = row.get::<_, Option<String>>(4)?.as_deref() == Some(text.as_str())
            && row.get::<_, bool>(5)?;
        Ok((row.get(0)?, text, current))
    })?;
    let mut stale = Vec::new();
    for row in rows {
        let (task, text, current) = row?;
        if !current && !text.is_empty() {
            stale.push((task, text));
        }
    }
    Ok(stale)
}

/// Stores task `task`'s vector of `text`, made by the embedder `name`.
pub fn store(
    db: &Connection,
    task: i64,
    name: &str,
    text: &str,
    vector: &[f32],
) -> rusqlite::Result<()> {
    let bytes: Vec<u8> = vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    db.execute(
        "INSERT INTO vectors (task, embedder, text, vector) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (task) DO UPDATE SET embedder = ?2, text = ?3, vector = ?4",
        params![task, name, text, bytes],
    )
    .map(drop)
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

const IN_STATES: &str =
    "(json_array_length(?2) = 0 OR tasks.state IN (SELECT value FROM json_each(?2)))";

/// What a search found, and why it compared no vectors when embeddings are on but unavailable.
pub struct Found {
    pub table: String,
    pub unavailable: Option<String>,
}

/// Embeds a text with the embedder a name stands for.
pub type Embed<'a> = dyn FnMut(&str, &str) -> Result<Vec<f32>, String> + 'a;

/// The tasks matching `query`, best first, in `states` when it names any, as a table with a header.
/// With embeddings on, `embed` turns the query into a vector, and the keyword and vector rankings
/// merge by reciprocal rank fusion. Tasks the embedder has not reached match by keyword alone.
pub fn search(
    db: &Connection,
    query: &str,
    states: &[String],
    limit: usize,
    embed: &mut Embed,
) -> Result<Found, String> {
    let states = serde_json::to_string(states).expect("states serialize");
    let keyword = match any_word(query) {
        Some(words) => keyword_ranking(db, &words, &states).map_err(|error| error.to_string())?,
        None => Vec::new(),
    };
    let (vector, unavailable) = match embedder(db).map_err(|error| error.to_string())? {
        None => (Vec::new(), None),
        Some((_, Some(error))) => (Vec::new(), Some(error)),
        Some((name, None)) => match embed(&name, query) {
            Ok(query) => (
                vector_ranking(db, &name, &query, &states).map_err(|error| error.to_string())?,
                None,
            ),
            Err(error) => (Vec::new(), Some(error)),
        },
    };
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for ranking in [&keyword, &vector] {
        for (rank, task) in ranking.iter().enumerate() {
            *scores.entry(*task).or_default() += 1.0 / (K + rank as f64 + 1.0);
        }
    }
    let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(limit);
    let mut table = String::from("task\tkey\tstate\tstep\ttldr\n");
    for (task, _) in ranked {
        let row = db
            .query_row(
                "SELECT tasks.key, tasks.state, tasks.step, tasks.tldr, task_text.title, task_text.request
                 FROM tasks JOIN task_text ON task_text.task = tasks.id WHERE tasks.id = ?1",
                [task],
                |row| {
                    let tldr: Option<String> = row.get(3)?;
                    let shown = match tldr {
                        Some(tldr) => tldr,
                        None => fallback(&row.get::<_, String>(4)?, &row.get::<_, String>(5)?),
                    };
                    Ok([
                        task.to_string(),
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        shown.replace(['\n', '\t'], " "),
                    ])
                },
            )
            .map_err(|error| error.to_string())?;
        table.push_str(&row.join("\t"));
        table.push('\n');
    }
    Ok(Found { table, unavailable })
}

/// The tasks whose text matches `words`, best bm25 score first.
fn keyword_ranking(db: &Connection, words: &str, states: &str) -> rusqlite::Result<Vec<i64>> {
    let mut statement = db.prepare(&format!(
        "SELECT tasks.id FROM search JOIN tasks ON tasks.id = search.rowid
         WHERE search MATCH ?1 AND {IN_STATES} ORDER BY bm25(search), tasks.id LIMIT ?3"
    ))?;
    statement
        .query_map(params![words, states, RANKED as i64], |row| row.get(0))?
        .collect()
}

/// The tasks whose vector by the embedder `name` is similar enough to `query`, most similar first.
fn vector_ranking(
    db: &Connection,
    name: &str,
    query: &[f32],
    states: &str,
) -> rusqlite::Result<Vec<i64>> {
    let mut statement = db.prepare(&format!(
        "SELECT tasks.id, vectors.vector FROM vectors JOIN tasks ON tasks.id = vectors.task
         WHERE vectors.embedder = ?1 AND {IN_STATES}"
    ))?;
    let mut similar: Vec<(i64, f32)> = statement
        .query_map(params![name, states], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?
        .filter_map(|row| {
            let (task, bytes) = row.ok()?;
            let vector: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("4 bytes")))
                .collect();
            Some((task, cosine(query, &vector)?))
        })
        .filter(|(_, similarity)| *similarity >= SIMILAR)
        .collect();
    similar.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    similar.truncate(RANKED);
    Ok(similar.into_iter().map(|(task, _)| task).collect())
}

/// The cosine similarity of two vectors of one length, if neither is zero.
fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norms = norm(a) * norm(b);
    (norms > 0.0).then(|| dot / norms)
}

/// What stands for a task's TL;DR until it has one: its request's title and opening text.
pub(crate) fn fallback(title: &str, request: &str) -> String {
    cut(&format!("{title} {request}"))
}
