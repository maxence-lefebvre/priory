//! Outcome labels, which judge a graded task by what happened to its pull requests and gates, and
//! `priory grades`, which compares each grade with them.

use crate::trace::{self, Trace};
use herdr_bus::Message;
use priory_intake::intake::{CHECKED, PASSING, PR_CLOSED, REVIEWED};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

/// The labels that settle a task's outcome, so its grade counts as calibrated.
const FINAL: [&str; 4] = [
    "approved-clean",
    "changes-requested",
    "approval-rejected",
    "closed-unmerged",
];

/// Every label, in the order a report shows them.
const LABELS: [&str; 6] = [
    "approved-clean",
    "changes-requested",
    "approval-rejected",
    "closed-unmerged",
    "red-after-merge",
    "reverted",
];

/// The share of green tasks with a final outcome that should be `approved-clean`.
const TARGET: f64 = 0.9;

/// Labels the tasks that own the pull request `message` names, by what the event tells, once the
/// inbox holds it.
pub(crate) fn label(tx: &Transaction, trace: &Trace, message: &Message) -> rusqlite::Result<()> {
    let data = &message.data;
    let (Some(repo), Some(number)) = (data["repo"].as_str(), data["pull_request"].as_u64()) else {
        return Ok(());
    };
    let url = data["url"].as_str().unwrap_or_default();
    let happened = message.time.to_string();
    let happened = Some(happened.as_str());
    match message.kind.as_str() {
        REVIEWED if human(data) => {
            // A review that arrives after the merge can name an older commit than the merged one.
            let evidence = match data["state"] == "changes_requested" {
                true => Some(url.to_owned()),
                false => pushed_after_review(tx, &crate::route::pull_request(repo, number))?,
            };
            let Some(evidence) = evidence else {
                return Ok(());
            };
            for task in authors(tx, repo, number, data["head_branch"].as_str())? {
                trace::labeled(tx, trace, task, "changes-requested", &evidence, happened)?;
            }
        }
        PR_CLOSED if data["merged"] == true => {
            for task in authors(tx, repo, number, data["head_branch"].as_str())? {
                merged(tx, trace, task, repo, number, url, happened)?;
            }
            reverts(tx, trace, repo, number, data, happened)?;
        }
        PR_CLOSED if data["merged"] == false => {
            for task in authors(tx, repo, number, data["head_branch"].as_str())? {
                trace::labeled(tx, trace, task, "closed-unmerged", url, happened)?;
            }
        }
        CHECKED => {
            let conclusion = data["conclusion"].as_str().unwrap_or_default();
            let Some(head) = data["head_sha"]
                .as_str()
                .filter(|_| !PASSING.contains(&conclusion))
            else {
                return Ok(());
            };
            for (number, branch) in merged_at(tx, repo, head)? {
                for task in authors(tx, repo, number as u64, branch.as_deref())? {
                    trace::labeled(tx, trace, task, "red-after-merge", url, happened)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Labels `approval-rejected` the task whose human step `answer` ends, when it rejects: its option
/// starts with `reject`, or it sends the task to `stop`.
pub(crate) fn answered(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    step: &str,
    answer: &str,
    target: &str,
) -> rusqlite::Result<()> {
    if target != "stop" && !answer.to_lowercase().starts_with("reject") {
        return Ok(());
    }
    trace::labeled(
        tx,
        trace,
        task,
        "approval-rejected",
        &format!("{step}: {answer}"),
        None,
    )
}

fn human(data: &Value) -> bool {
    !data["actor"]
        .as_str()
        .is_some_and(|actor| actor.ends_with("[bot]"))
}

/// The tasks that wrote pull request `number`: those whose stack holds it, and those that own its
/// head branch. A task keyed by the pull request, such as a review task, did not write it.
fn authors(
    tx: &Transaction,
    repo: &str,
    number: u64,
    head: Option<&str>,
) -> rusqlite::Result<Vec<i64>> {
    let branch = head.map(|head| crate::route::branch(repo, head));
    tx.prepare(
        "SELECT task FROM pull_requests WHERE resource = ?1
         UNION SELECT task FROM owns WHERE resource = ?2",
    )?
    .query_map(
        params![crate::route::pull_request(repo, number), branch],
        |row| row.get(0),
    )?
    .collect()
}

/// Labels `task` once its last open pull request merged: `approved-clean` when no human review
/// came before a later push, else `changes-requested`. A task that already had changes requested
/// keeps that label alone.
fn merged(
    tx: &Transaction,
    trace: &Trace,
    task: i64,
    repo: &str,
    number: u64,
    url: &str,
    happened: Option<&str>,
) -> rusqlite::Result<()> {
    let stack: Vec<(String, String)> = tx
        .prepare("SELECT resource, state FROM pull_requests WHERE task = ?1")?
        .query_map([task], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if stack.iter().any(|(_, state)| state != "merged") {
        return Ok(());
    }
    let pulls = match stack.is_empty() {
        true => vec![crate::route::pull_request(repo, number)],
        false => stack.into_iter().map(|(resource, _)| resource).collect(),
    };
    let requested = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM trace WHERE type = 'factory.step.labeled'
           AND json_extract(event, '$.data.task') = ?1
           AND json_extract(event, '$.data.label') = 'changes-requested')",
        [task.to_string()],
        |row| row.get::<_, bool>(0),
    )?;
    if requested {
        return Ok(());
    }
    for pull in &pulls {
        if let Some(pushed) = pushed_after_review(tx, pull)? {
            return trace::labeled(tx, trace, task, "changes-requested", &pushed, happened);
        }
    }
    trace::labeled(tx, trace, task, "approved-clean", url, happened)
}

/// The merged head of `pull` when it is not the commit of its first human review, as evidence of a
/// push after that review. Events count by when they happened, not when they arrived.
fn pushed_after_review(tx: &Transaction, pull: &str) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "WITH events AS (SELECT time, type, data FROM inbox
           WHERE 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1),
         reviewed AS (SELECT coalesce(json_extract(data, '$.commit'), json_extract(data, '$.head_sha')) AS head
           FROM events WHERE type = ?2 AND coalesce(json_extract(data, '$.actor'), '') NOT LIKE '%[bot]'
           AND coalesce(json_extract(data, '$.commit'), json_extract(data, '$.head_sha')) IS NOT NULL
           ORDER BY time LIMIT 1),
         merged AS (SELECT json_extract(data, '$.head_sha') AS head FROM events
           WHERE type = ?3 AND json_extract(data, '$.merged') = 1 ORDER BY time DESC LIMIT 1)
         SELECT 'merged ' || merged.head || ' after the review of ' || reviewed.head
         FROM reviewed, merged WHERE merged.head != reviewed.head",
        params![pull, REVIEWED, PR_CLOSED],
        |row| row.get(0),
    )
    .optional()
}

/// The pull requests of `repo` whose merge commit is `head`, with their head branches.
fn merged_at(
    tx: &Transaction,
    repo: &str,
    head: &str,
) -> rusqlite::Result<Vec<(i64, Option<String>)>> {
    tx.prepare(
        "SELECT DISTINCT json_extract(data, '$.pull_request'), json_extract(data, '$.head_branch')
         FROM inbox
         WHERE type = ?1 AND json_extract(data, '$.merged') = 1
         AND json_extract(data, '$.repo') = ?2 AND json_extract(data, '$.merge_commit') = ?3",
    )?
    .query_map(params![PR_CLOSED, repo, head], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?
    .collect()
}

/// Labels `reverted` the authors of each earlier merged pull request of `repo` that the merged
/// pull request `number` reverts: its title or body says `reverts commit <sha>` of that one's merge
/// commit, as `git revert` writes, or `Reverts owner/name#N`, as GitHub's revert button writes.
fn reverts(
    tx: &Transaction,
    trace: &Trace,
    repo: &str,
    number: u64,
    data: &Value,
    happened: Option<&str>,
) -> rusqlite::Result<()> {
    let text = format!(
        "{}\n{}",
        data["title"].as_str().unwrap_or_default(),
        data["body"].as_str().unwrap_or_default()
    )
    .to_lowercase();
    let earlier: Vec<(i64, Option<String>, Option<String>)> = tx
        .prepare(
            "SELECT DISTINCT json_extract(data, '$.pull_request'), json_extract(data, '$.merge_commit'),
               json_extract(data, '$.head_branch')
             FROM inbox WHERE type = ?1 AND json_extract(data, '$.merged') = 1
             AND json_extract(data, '$.repo') = ?2 AND json_extract(data, '$.pull_request') != ?3",
        )?
        .query_map(params![PR_CLOSED, repo, number as i64], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let url = data["url"].as_str().unwrap_or_default();
    for (reverted, merge_commit, branch) in earlier {
        let named = merge_commit
            .is_some_and(|sha| text.contains(&format!("reverts commit {}", sha.to_lowercase())));
        if !named
            && !mentions(
                &text,
                &format!("reverts {}#{reverted}", repo.to_lowercase()),
            )
        {
            continue;
        }
        for task in authors(tx, repo, reverted as u64, branch.as_deref())? {
            trace::labeled(tx, trace, task, "reverted", url, happened)?;
        }
    }
    Ok(())
}

/// Whether `text` holds `reference` not followed by another digit, so `#5` is not `#57`.
fn mentions(text: &str, reference: &str) -> bool {
    text.match_indices(reference).any(|(at, _)| {
        !text[at + reference.len()..].starts_with(|next: char| next.is_ascii_digit())
    })
}

/// Each grade's tasks and their outcome labels.
pub struct Grades {
    since: Option<String>,
    rows: Vec<Row>,
}

struct Row {
    grade: &'static str,
    tasks: i64,
    outcomes: i64,
    labels: [i64; LABELS.len()],
}

impl Row {
    fn share(&self, count: i64) -> Option<f64> {
        (self.outcomes > 0).then(|| count as f64 / self.outcomes as f64)
    }
}

/// The tasks created since the instant `since`, or all of them, with a grade and their labels.
pub fn grades(db: &Connection, since: Option<String>) -> Result<Grades, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let mut statement = db
        .prepare(
            "SELECT tasks.grade, group_concat(json_extract(trace.event, '$.data.label'), ' ')
             FROM tasks LEFT JOIN trace ON trace.type = 'factory.step.labeled'
               AND json_extract(trace.event, '$.data.task') = CAST(tasks.id AS TEXT)
             WHERE tasks.grade IS NOT NULL AND (?1 IS NULL OR tasks.created >= ?1)
             GROUP BY tasks.id",
        )
        .map_err(error)?;
    let tasks: Vec<(String, Option<String>)> = statement
        .query_map([&since], |row| Ok((row.get(0)?, row.get(1)?)))
        .and_then(Iterator::collect)
        .map_err(error)?;
    let rows = crate::tools::GRADES
        .into_iter()
        .map(|grade| {
            let mut row = Row {
                grade,
                tasks: 0,
                outcomes: 0,
                labels: [0; LABELS.len()],
            };
            for (_, labels) in tasks.iter().filter(|(graded, _)| graded == grade) {
                let mut labels: Vec<&str> =
                    labels.as_deref().unwrap_or_default().split(' ').collect();
                // A review that arrived after the merge still requested a change.
                if labels.contains(&"changes-requested") {
                    labels.retain(|label| *label != "approved-clean");
                }
                row.tasks += 1;
                row.outcomes += i64::from(FINAL.iter().any(|label| labels.contains(label)));
                for (count, label) in row.labels.iter_mut().zip(LABELS) {
                    *count += i64::from(labels.contains(&label));
                }
            }
            row
        })
        .collect();
    Ok(Grades { since, rows })
}

impl Grades {
    fn green(&self) -> &Row {
        self.rows
            .iter()
            .find(|row| row.grade == "green")
            .expect("green is a grade")
    }

    pub fn json(&self) -> Value {
        let green = self.green();
        json!({
            "since": self.since,
            "one_shot": {
                "rate": green.share(green.labels[0]),
                "clean": green.labels[0],
                "outcomes": green.outcomes,
                "target": TARGET,
            },
            "grades": self.rows.iter().map(|row| json!({
                "grade": row.grade,
                "tasks": row.tasks,
                "outcomes": row.outcomes,
                "labels": LABELS.iter().zip(row.labels).map(|(label, count)| {
                    (label.to_string(), json!({ "count": count, "share": row.share(count) }))
                }).collect::<serde_json::Map<_, _>>(),
            })).collect::<Vec<_>>(),
        })
    }

    pub fn print(&self) {
        let green = self.green();
        let percent = |share: Option<f64>| {
            share.map_or_else(|| "-".to_owned(), |share| format!("{:.1}%", share * 100.0))
        };
        crate::state::emit(&format!(
            "green one-shot: {} ({} of {}), target {:.0}%",
            percent(green.share(green.labels[0])),
            green.labels[0],
            green.outcomes,
            TARGET * 100.0
        ));
        crate::state::emit("");
        crate::state::emit(&format!("grade\ttasks\toutcomes\t{}", LABELS.join("\t")));
        for row in &self.rows {
            let cells: Vec<String> = row
                .labels
                .iter()
                .map(|&count| format!("{count} ({})", percent(row.share(count))))
                .collect();
            crate::state::emit(&format!(
                "{}\t{}\t{}\t{}",
                row.grade,
                row.tasks,
                row.outcomes,
                cells.join("\t")
            ));
        }
    }
}
