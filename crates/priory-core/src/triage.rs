//! The sorting of review feedback: the rule a wake carries, and the class each item gets in a result.

use rusqlite::{Transaction, params};
use serde_json::Value;
use std::collections::BTreeMap;

const CLASSES: [&str; 3] = ["trivial", "nothing-to-do", "persons-call"];

const FEEDBACK: [&str; 5] = [
    "github.pull_request_review.submitted",
    "github.pull_request.comment",
    "github.pull_request_review_comment",
    "github.issues.comment",
    "linear.comment.created",
];

pub(crate) const RULE: &str = "Before you act on the feedback above, sort each item by its URL as one of: trivial (one obvious answer inside this change's scope: fix it and reply), nothing-to-do (an approval or thanks) or persons-call (a behaviour, design or scope change, a disagreement or anything unclear: ask the person and change nothing for it). End the step with the result tool's feedback field: an object from each item's URL to its class.";

/// The URL naming the feedback item that `data` is, when an agent must sort it: a comment, or a
/// review that is not an approval or a dismissal.
pub(crate) fn item<'a>(kind: &str, data: &'a Value) -> Option<&'a str> {
    let settled = matches!(data["state"].as_str(), Some("approved" | "dismissed"));
    let asks = FEEDBACK.contains(&kind) && !(kind == FEEDBACK[0] && settled);
    data["url"].as_str().filter(|url| asks && !url.is_empty())
}

/// Whether any of the inbox events `seqs` is an item to sort.
pub(crate) fn any(tx: &Transaction, seqs: &[i64]) -> rusqlite::Result<bool> {
    for seq in seqs {
        let (kind, data): (String, String) = tx.query_row(
            "SELECT type, data FROM inbox WHERE seq = ?1",
            [seq],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let data = serde_json::from_str(&data).unwrap_or(Value::Null);
        if item(&kind, &data).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The classes a result's `feedback` gives, as pairs of URL and class, or why it is refused: it
/// must be an object of known classes that names every item the agent was typed and has not sorted.
pub(crate) fn sorted(
    tx: &Transaction,
    task: i64,
    feedback: &Value,
) -> rusqlite::Result<Result<Vec<(String, String)>, String>> {
    let invalid =
        "feedback must be an object from each item's URL to trivial, nothing-to-do or persons-call";
    let given: BTreeMap<String, String> = match feedback {
        Value::Null => BTreeMap::new(),
        Value::Object(map) => {
            let classes = map
                .iter()
                .map(|(url, class)| Some((url.clone(), class.as_str()?.to_owned())))
                .collect::<Option<BTreeMap<_, _>>>();
            match classes.filter(|c| c.values().all(|class| CLASSES.contains(&class.as_str()))) {
                Some(classes) => classes,
                None => return Ok(Err(invalid.to_owned())),
            }
        }
        _ => return Ok(Err(invalid.to_owned())),
    };
    let mut statement = tx.prepare(
        "SELECT inbox.type, inbox.data FROM inbox JOIN outbox ON outbox.id = inbox.outbox
         WHERE inbox.task = ?1 AND outbox.state = 'confirmed'
         ORDER BY inbox.seq",
    )?;
    let mut rows = statement.query([task])?;
    while let Some(row) = rows.next()? {
        let data: Value = serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or(Value::Null);
        let Some(url) = item(&row.get::<_, String>(0)?, &data) else {
            continue;
        };
        let known: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM feedback_triage WHERE task = ?1 AND item = ?2)",
            params![task, url],
            |row| row.get(0),
        )?;
        if !known && !given.contains_key(url) {
            return Ok(Err(format!("feedback item {url} has no class: {invalid}")));
        }
    }
    Ok(Ok(given.into_iter().collect()))
}

/// Records the classes that run `run` of `task` gave.
pub(crate) fn store(
    tx: &Transaction,
    task: i64,
    run: i64,
    classes: &[(String, String)],
) -> rusqlite::Result<()> {
    for (item, class) in classes {
        tx.execute(
            "INSERT INTO feedback_triage (task, item, class, run) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (task, item) DO UPDATE SET class = excluded.class, run = excluded.run",
            params![task, item, class, run],
        )?;
    }
    Ok(())
}
