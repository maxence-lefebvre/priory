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

pub(crate) const RULE: &str = "Before you act on the feedback above, sort each item by its URL as one of: trivial (one obvious answer inside this change's scope: fix it and reply, ending the reply with the line <!-- agent-comment -->), nothing-to-do (an approval or thanks) or persons-call (a behaviour, design or scope change, a disagreement or anything unclear: ask the person with the ask tool, naming the item's URL in the question, and change nothing for it). End the step with the result tool's feedback field: an object from each item's URL to its class.";

/// The URL naming the feedback item that `data` is: a comment, or a review that is not an
/// approval or a dismissal.
fn url<'a>(kind: &str, data: &'a Value) -> Option<&'a str> {
    let settled = matches!(data["state"].as_str(), Some("approved" | "dismissed"));
    let asks = FEEDBACK.contains(&kind) && !(kind == FEEDBACK[0] && settled);
    data["url"].as_str().filter(|url| asks && !url.is_empty())
}

/// The URL of an item the agent must sort. An item written by the login `gh api user` names, which
/// the intake runs as, is the person's own or an agent's reply: it goes to the person, not the agent.
/// An item a calibrated decision model sorted is no longer the agent's to sort.
pub(crate) fn item<'a>(kind: &str, data: &'a Value) -> Option<&'a str> {
    url(kind, data).filter(|_| data["own"] != true && crate::decision::class(data).is_none())
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
/// must be an object of known classes that, when `all` is true, names every item the agent was typed since the task's
/// last result and has not sorted, and each persons-call needs an ask of the task that names it. The person's own items are recorded as persons-calls; the agent's own replies are not recorded.
pub(crate) fn sorted(
    tx: &Transaction,
    task: i64,
    feedback: &Value,
    all: bool,
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
    let (mut own, mut missing) = (Vec::new(), Vec::new());
    // Items typed before the task's last result were that result's to sort, or predate sorting.
    // An own item marked `agent` is the agent's reply. One without `agent`, from before the marker,
    // is the agent's when written inside a run of the task. A comment in a run's first or last
    // second may have woken or ended it, so it stays the person's. Runs keep whole seconds, so the
    // comment's time is compared to the second.
    let mut statement = tx.prepare(
        "SELECT inbox.type, inbox.data, EXISTS (SELECT 1 FROM runs WHERE runs.task = ?1
           AND unixepoch(inbox.time) > unixepoch(runs.started)
           AND (runs.ended IS NULL OR unixepoch(inbox.time) < unixepoch(runs.ended)))
         FROM inbox JOIN outbox ON outbox.id = inbox.outbox
         WHERE inbox.task = ?1 AND outbox.state = 'confirmed'
         AND outbox.run > coalesce((SELECT max(run) FROM results WHERE task = ?1), 0)
         ORDER BY inbox.seq",
    )?;
    let mut rows = statement.query([task])?;
    while let Some(row) = rows.next()? {
        let data: Value = serde_json::from_str(&row.get::<_, String>(1)?).unwrap_or(Value::Null);
        let kind = row.get::<_, String>(0)?;
        if let Some(url) = url(&kind, &data).filter(|_| item(&kind, &data).is_none()) {
            if let Some(class) = crate::decision::class(&data) {
                // The agent's own class for it, as a correction, wins.
                if !given.contains_key(url) {
                    own.push((url.to_owned(), class.to_owned()));
                }
                continue;
            }
            let during = row.get::<_, bool>(2)?;
            let reply = data.get("agent").map_or(during, |agent| agent == true);
            if !reply {
                own.push((url.to_owned(), "persons-call".to_owned()));
            }
            continue;
        }
        let Some(url) = item(&kind, &data) else {
            continue;
        };
        let known: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM feedback_triage WHERE task = ?1 AND item = ?2)",
            params![task, url],
            |row| row.get(0),
        )?;
        if all && !known && !given.contains_key(url) && !missing.iter().any(|item| item == url) {
            missing.push(url.to_owned());
        }
    }
    match missing.as_slice() {
        [] => {}
        [url] => return Ok(Err(format!("feedback item {url} has no class: {invalid}"))),
        urls => {
            let urls = urls.join(", ");
            return Ok(Err(format!(
                "feedback items {urls} have no class: {invalid}"
            )));
        }
    }
    for (url, class) in &given {
        let asked: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM asks WHERE task = ?1
             AND (withdrawn IS NULL OR withdrawn_reason = 'dismissed')
             AND instr(question || ' ' || coalesce(title, '') || ' ' || coalesce(text, ''), ?2) > 0)",
            params![task, url],
            |row| row.get(0),
        )?;
        if class == "persons-call" && !asked {
            return Ok(Err(format!(
                "feedback item {url} is a persons-call but no ask of this task names it: raise one with the ask tool, naming the URL, then send the result"
            )));
        }
    }
    let mut classes: Vec<_> = given.into_iter().collect();
    classes.extend(own);
    Ok(Ok(classes))
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
