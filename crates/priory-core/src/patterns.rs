//! `priory patterns`: failure patterns, the tasks that stopped at one step for the same reason.
//! `find` groups the window's failed, blocked, repaired and marked-wrong tasks by workflow, step and
//! the shape of their reason, and appends one `factory.pattern.matched` record per new piece of
//! evidence. A pattern is the records of one group, so it needs no table and its count only grows.

use crate::trace::{self, Trace};
use jiff::Timestamp;
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

/// How far back `priory patterns find` reads when it names no `--since`.
pub const SINCE: &str = "24h";
/// The fewest tasks a pattern holds before it shows on the desk. The desk's query in
/// `listing.rs` holds the same number.
pub const LEAST: usize = 2;
/// The text of a person's `priory label TASK wrong`, which carries a URL and no reason.
const MARKED: &str = "marked wrong by a person";
/// The most characters of a signature.
const LONGEST: usize = 120;

/// The shape of a failure's text, which groups failures: its first line, lower case, with each
/// number, such as `41` or `10.5`, and each hex word of 7 or more characters with a digit as `#`,
/// runs of space as one, and at most 120 characters.
pub fn signature(text: &str) -> String {
    static HEX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b[0-9a-f]{7,}\b").expect("the hex pattern compiles"));
    static DIGITS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"[0-9]+(?:[.,][0-9]+)*").expect("the digits pattern compiles")
    });
    static SPACE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s+").expect("the space pattern compiles"));
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .to_lowercase();
    let line = HEX.replace_all(&line, |found: &regex::Captures| {
        if found[0].bytes().any(|byte| byte.is_ascii_digit()) {
            "#".to_owned()
        } else {
            found[0].to_owned()
        }
    });
    let line = DIGITS.replace_all(&line, "#");
    let line = SPACE.replace_all(&line, " ");
    line.trim()
        .chars()
        .take(LONGEST)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// A pattern's id: the first 8 hex digits of the SHA-256 of its workflow, step and signature.
pub fn id(workflow: &str, step: &str, signature: &str) -> String {
    let digest = Sha256::digest(format!("{workflow}\n{step}\n{signature}"));
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One piece of evidence: a task that stopped, was repaired or was marked wrong at a step.
struct Evidence {
    task: i64,
    key: String,
    generation: i64,
    workflow: String,
    step: String,
    kind: &'static str,
    text: String,
    happened: String,
    source: String,
}

/// `text` on one line with each credential shape and each of `secrets` replaced by `[redacted]`.
fn redacted(text: &str, secrets: &[String]) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    crate::screen::redact(line, secrets, None)
        .map_or_else(|| "[redacted]".to_owned(), |(line, _)| line)
        .replace(['\n', '\r', '\t'], " ")
}

/// The window's evidence, oldest first.
fn evidence(
    db: &Connection,
    start: &str,
    now: &str,
    secrets: &[String],
) -> rusqlite::Result<Vec<Evidence>> {
    let within = |time: &str| {
        format!("julianday(?1) <= julianday({time}) AND julianday({time}) <= julianday(?2)")
    };
    let mut found = Vec::new();
    let mut stopped = db.prepare(&format!(
        "SELECT id, key, generation, workflow, step, state, coalesce(reason, state), changed
         FROM tasks WHERE state IN ('failed', 'blocked') AND {} ORDER BY id",
        within("changed")
    ))?;
    let rows = stopped.query_map([start, now], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
        ))
    })?;
    for row in rows {
        let (task, key, generation, workflow, step, state, reason, changed) = row?;
        let kind = if state == "failed" {
            "failed"
        } else {
            "blocked"
        };
        // A stop is the same evidence until the task's next generation or another reason.
        let source = format!(
            "task {task} generation {generation} {kind} at {step}: {}",
            signature(&redacted(&reason, secrets))
        );
        found.push(Evidence {
            task,
            key,
            generation,
            workflow,
            step,
            kind,
            text: reason,
            happened: changed,
            source,
        });
    }

    let mut repaired = db.prepare(&format!(
        "SELECT id, time, event FROM trace WHERE type = 'factory.task.repaired' AND {} ORDER BY seq",
        within("time")
    ))?;
    let rows = repaired.query_map([start, now], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (source, happened, event) = row?;
        let data = serde_json::from_str::<Value>(&event).unwrap_or_default()["data"].clone();
        let text = |field: &str| data[field].as_str().map(str::to_owned);
        let (Some(task), Some(workflow), Some(step), Some(cause)) = (
            text("task").and_then(|task| task.parse().ok()),
            text("workflow"),
            text("step"),
            text("cause"),
        ) else {
            continue;
        };
        found.push(Evidence {
            task,
            key: text("key").unwrap_or_default(),
            generation: data["generation"].as_i64().unwrap_or(1),
            workflow,
            step,
            kind: "repaired",
            text: cause,
            happened,
            source,
        });
    }

    let mut marked = db.prepare(&format!(
        "SELECT labeled.id, labeled.time, json_extract(labeled.event, '$.data.task'),
           tasks.key, tasks.generation, tasks.workflow,
           coalesce((SELECT json_extract(ended.event, '$.data.step') FROM trace AS ended
             WHERE ended.id = json_extract(labeled.event, '$.data.step_event')), tasks.step)
         FROM trace AS labeled
         JOIN tasks ON tasks.id = CAST(json_extract(labeled.event, '$.data.task') AS INTEGER)
         WHERE labeled.type = 'factory.step.labeled'
           AND json_extract(labeled.event, '$.data.label') = ?3 AND {}
           AND NOT EXISTS (SELECT 1 FROM trace AS cleared WHERE cleared.type = 'factory.step.unlabeled'
             AND json_extract(cleared.event, '$.data.task') = json_extract(labeled.event, '$.data.task')
             AND json_extract(cleared.event, '$.data.label') = ?3 AND cleared.seq > labeled.seq)
         ORDER BY labeled.seq",
        within("labeled.time")
    ))?;
    let rows = marked.query_map(params![start, now, crate::label::MARKED_WRONG], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;
    for row in rows {
        let (source, happened, task, key, generation, workflow, step) = row?;
        let Ok(task) = task.parse() else {
            continue;
        };
        found.push(Evidence {
            task,
            key,
            generation,
            workflow,
            step,
            kind: "marked-wrong",
            text: MARKED.to_owned(),
            happened,
            source,
        });
    }
    found.sort_by(|a, b| a.happened.cmp(&b.happened));
    Ok(found)
}

/// What `find` recorded: the new evidence of each pattern, by pattern id.
pub struct Found {
    pub matched: BTreeMap<String, usize>,
}

/// Appends a `factory.pattern.matched` record for each piece of the window's evidence that no
/// earlier record holds, so running it twice over one window records nothing new.
pub fn find(db: &mut Connection, trace: &Trace, start: &str, now: &str) -> Result<Found, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let mut matched = BTreeMap::new();
    let secrets = crate::screen::secrets(&trace.factory.secret_envs());
    for found in evidence(&tx, start, now, &secrets).map_err(error)? {
        let known = tx
            .query_row(
                "SELECT 1 FROM trace WHERE type = 'factory.pattern.matched'
                 AND json_extract(event, '$.data.source') = ?1",
                [&found.source],
                |_| Ok(()),
            )
            .optional()
            .map_err(error)?;
        if known.is_some() {
            continue;
        }
        // The trace keeps the signature, so it takes the redacted line, as `text` does.
        let shape = signature(&redacted(&found.text, &secrets));
        let pattern = id(&found.workflow, &found.step, &shape);
        let data = json!({
            "factory": trace.factory.name,
            "factory_revision": trace.revision,
            "machine": trace.machine,
            "pattern": pattern,
            "workflow": found.workflow,
            "step": found.step,
            "signature": shape,
            "text": redacted(&found.text, &secrets),
            "task": found.task.to_string(),
            "key": found.key,
            "generation": found.generation,
            "kind": found.kind,
            "happened": found.happened,
            "source": found.source,
        });
        let mut data = data;
        if let Value::Object(fields) = &mut data {
            fields.retain(|_, value| !value.is_null());
        }
        trace::record(&tx, trace, "factory.pattern.matched", data).map_err(error)?;
        *matched.entry(pattern).or_insert(0) += 1;
    }
    tx.commit().map_err(error)?;
    Ok(Found { matched })
}

/// One match of a pattern: a task and how it stopped.
#[derive(Clone, serde::Serialize)]
pub struct Match {
    pub task: i64,
    pub key: String,
    pub kind: String,
    pub text: String,
    pub happened: String,
}

/// A failure pattern, from its records.
#[derive(Clone, serde::Serialize)]
pub struct Pattern {
    pub id: String,
    pub workflow: String,
    pub step: String,
    /// The latest match's text.
    pub text: String,
    pub signature: String,
    /// `open`, `closed` or `dismissed`.
    pub state: String,
    /// The distinct tasks of its matches, in the order they first matched.
    pub tasks: Vec<i64>,
    pub first: String,
    pub last: String,
    /// When a person last closed or dismissed it.
    pub closed: Option<String>,
    pub matches: Vec<Match>,
}

fn instant(text: &str) -> Option<Timestamp> {
    text.parse().ok()
}

/// Every pattern, the latest match first.
pub fn patterns(db: &Connection) -> rusqlite::Result<Vec<Pattern>> {
    let mut by_id: BTreeMap<String, Pattern> = BTreeMap::new();
    let mut matched = db.prepare(
        "SELECT seq, event FROM trace WHERE type = 'factory.pattern.matched' ORDER BY seq",
    )?;
    let rows = matched.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut seqs: BTreeMap<String, Vec<(i64, Option<Timestamp>)>> = BTreeMap::new();
    for row in rows {
        let (seq, event) = row?;
        let data = serde_json::from_str::<Value>(&event).unwrap_or_default()["data"].clone();
        let text = |field: &str| data[field].as_str().unwrap_or_default().to_owned();
        let (Some(id), Ok(task)) = (data["pattern"].as_str(), text("task").parse::<i64>()) else {
            continue;
        };
        let found = Match {
            task,
            key: text("key"),
            kind: text("kind"),
            text: text("text"),
            happened: text("happened"),
        };
        let pattern = by_id.entry(id.to_owned()).or_insert_with(|| Pattern {
            id: id.to_owned(),
            workflow: text("workflow"),
            step: text("step"),
            text: String::new(),
            signature: text("signature"),
            state: "open".to_owned(),
            tasks: Vec::new(),
            first: found.happened.clone(),
            last: found.happened.clone(),
            closed: None,
            matches: Vec::new(),
        });
        if !pattern.tasks.contains(&task) {
            pattern.tasks.push(task);
        }
        if instant(&found.happened) >= instant(&pattern.last) {
            pattern.last = found.happened.clone();
            pattern.text = found.text.clone();
        }
        if instant(&found.happened) < instant(&pattern.first) {
            pattern.first = found.happened.clone();
        }
        seqs.entry(id.to_owned())
            .or_default()
            .push((seq, instant(&found.happened)));
        pattern.matches.push(found);
    }
    let mut closed = db.prepare(
        "SELECT seq, time, json_extract(event, '$.data.pattern'),
           coalesce(json_extract(event, '$.data.dismissed'), 0)
         FROM trace WHERE type = 'factory.pattern.closed' ORDER BY seq",
    )?;
    let rows = closed.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, bool>(3)?,
        ))
    })?;
    let mut closes: BTreeMap<String, (i64, Option<Timestamp>)> = BTreeMap::new();
    for row in rows {
        let (seq, time, id, dismissed) = row?;
        let Some(pattern) = by_id.get_mut(&id) else {
            continue;
        };
        closes.insert(id, (seq, instant(&time)));
        pattern.closed = Some(time);
        if dismissed {
            pattern.state = "dismissed".to_owned();
        }
    }
    for (id, (closed, at)) in closes {
        let second = |at: Option<Timestamp>| at.map(|at| at.as_second());
        let reopened = seqs.get(&id).is_some_and(|matches| {
            matches
                .iter()
                .any(|(seq, happened)| *seq > closed && second(*happened) >= second(at))
        });
        if let Some(pattern) = by_id
            .get_mut(&id)
            .filter(|pattern| pattern.state == "open" && !reopened)
        {
            pattern.state = "closed".to_owned();
        }
    }
    let mut all: Vec<Pattern> = by_id.into_values().collect();
    all.sort_by(|a, b| {
        instant(&b.last)
            .cmp(&instant(&a.last))
            .then(a.id.cmp(&b.id))
    });
    Ok(all)
}

/// The pattern `id`, or why there is none.
pub fn pattern(db: &Connection, id: &str) -> Result<Pattern, String> {
    patterns(db)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|pattern| pattern.id == id)
        .ok_or_else(|| format!("no pattern {id}: list them with `priory patterns --all`"))
}

/// Whether the desk shows `pattern`: open, with at least `LEAST` tasks.
pub fn shown(pattern: &Pattern) -> bool {
    pattern.state == "open" && pattern.tasks.len() >= LEAST
}

/// A person closes pattern `id` as fixed, or dismisses it as no pattern. A closed pattern opens
/// again when `find` records a task that stopped after the close; a dismissed one never does.
pub fn close(
    db: &mut Connection,
    trace: &Trace,
    id: &str,
    dismissed: bool,
) -> Result<String, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    let pattern = pattern(&tx, id)?;
    match pattern.state.as_str() {
        "dismissed" => return Err(format!("pattern {id} is already dismissed")),
        "closed" if !dismissed => return Err(format!("pattern {id} is already closed")),
        _ => {}
    }
    let data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "machine": trace.machine,
        "pattern": id,
        "workflow": pattern.workflow,
        "step": pattern.step,
        "tasks": pattern.tasks.len(),
        "dismissed": dismissed,
    });
    let mut data = data;
    if let Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    trace::record(&tx, trace, "factory.pattern.closed", data).map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(if dismissed {
        format!("pattern {id} dismissed: no task brings it back")
    } else {
        format!("pattern {id} closed: the next task that matches it opens it again")
    })
}

/// The pattern's distinct tasks, for its capture.
pub fn tasks(pattern: &Pattern) -> BTreeSet<i64> {
    pattern.tasks.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_signature_drops_numbers_hashes_case_and_spacing() {
        assert_eq!(
            signature("cargo test failed:  3 tests failed in 41.2s at 4c1e2b7f\nmore"),
            signature("Cargo test failed: 12 tests failed in 9s at 999daf01"),
        );
        assert_eq!(
            signature("cargo test failed: 3 tests failed in 41.2s at 4c1e2b7f"),
            "cargo test failed: # tests failed in #s at #"
        );
        assert_eq!(signature("\n\n  disk full  "), "disk full");
        assert_ne!(signature("disk full"), signature("permission denied"));
        assert_eq!(signature("a deadbeef word"), "a deadbeef word");
    }

    #[test]
    fn an_id_names_its_workflow_step_and_signature() {
        let one = id("implement", "plan", "disk full");
        assert_eq!(one.len(), 8);
        assert_eq!(one, id("implement", "plan", "disk full"));
        assert_ne!(one, id("implement", "test", "disk full"));
    }

    proptest! {
        #[test]
        fn a_signature_never_panics_and_is_its_own_signature(text in any::<String>()) {
            let once = signature(&text);
            prop_assert!(once.chars().count() <= LONGEST);
            prop_assert_eq!(signature(&once), once);
        }
    }
}
