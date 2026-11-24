//! Outcome labels, which judge a graded task by what happened to its pull requests and gates, and
//! `priory grades`, which compares each grade with them.

use crate::trace::{self, Trace};
use herdr_bus::Message;
use priory_intake::intake::{CHECKED, PASSING, PR_CLOSED, Passed, REVIEWED, Red, RedCheck};
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

/// How the report line of a merge that turned out red or was reverted starts; the desk ranks it as
/// an alarm.
pub(crate) const LANDED: &str = "landed wrong: ";

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
                .filter(|_| !PASSING.contains(&conclusion) && data["superseded_by"].is_null())
            else {
                return Ok(());
            };
            let base = data["base_branch"].as_str().unwrap_or("its base branch");
            let name = data["name"].as_str().unwrap_or("a check");
            for (number, branch) in merged_at(tx, repo, head)? {
                let tasks = authors(tx, repo, number as u64, branch.as_deref())?;
                for &task in &tasks {
                    trace::labeled(tx, trace, task, "red-after-merge", url, happened)?;
                }
                // One alarm per merge commit, however many of its checks fail.
                let red = format!("{LANDED}{repo}#{number} of {} is red on {base}", of(&tasks));
                if !tasks.is_empty() && !reported(tx, &red)? {
                    let line = format!("{red}: {name} {conclusion} {url}");
                    report(tx, &line)?;
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
        let tasks = authors(tx, repo, reverted as u64, branch.as_deref())?;
        for &task in &tasks {
            trace::labeled(tx, trace, task, "reverted", url, happened)?;
        }
        let line = format!(
            "{LANDED}{repo}#{reverted} of {} was reverted by #{number}: {url}",
            of(&tasks)
        );
        if !tasks.is_empty() && !reported(tx, &line)? {
            report(tx, &line)?;
        }
    }
    Ok(())
}

/// `task 5`, or `tasks 5, 6`.
fn of(tasks: &[i64]) -> String {
    let ids: Vec<String> = tasks.iter().map(i64::to_string).collect();
    match ids.len() {
        1 => format!("task {}", ids[0]),
        _ => format!("tasks {}", ids.join(", ")),
    }
}

/// Whether a report line from Priory starts with `start`.
fn reported(tx: &Transaction, start: &str) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM inbox WHERE type = ?1 AND subject = 'priory'
           AND substr(json_extract(data, '$.line'), 1, length(?2)) = ?2)",
        params![crate::post::REPORT, start],
        |row| row.get(0),
    )
}

fn report(tx: &Transaction, line: &str) -> rusqlite::Result<()> {
    crate::post::report(tx, "priory", line)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))
}

/// The unread reports of merges since `since` that are red on their base branch, with each check that
/// failed at the merge commit. A revert's report is none, nor is one with a failing run without an id.
pub fn red(db: &Connection, since: &str) -> rusqlite::Result<Vec<Red>> {
    let reds: Vec<Red> = db
        .prepare(
            "SELECT DISTINCT report.seq, json_extract(closed.data, '$.repo'),
               json_extract(closed.data, '$.base_branch'), json_extract(closed.data, '$.merge_commit')
             FROM inbox AS report CROSS JOIN inbox AS closed
               ON closed.type = ?1 AND julianday(closed.time) >= julianday(?4)
               AND json_extract(closed.data, '$.merged') = 1
               AND json_extract(closed.data, '$.merge_commit') IS NOT NULL
               AND json_extract(closed.data, '$.base_branch') IS NOT NULL
               AND instr(json_extract(report.data, '$.line'), ?2 || json_extract(closed.data, '$.repo')
                 || '#' || json_extract(closed.data, '$.pull_request') || ' of task') = 1
             WHERE report.type = ?3 AND report.subject = 'priory' AND report.read IS NULL
             AND instr(json_extract(report.data, '$.line'), ' is red on ') > 0",
        )?
        .query_map(params![PR_CLOSED, LANDED, crate::post::REPORT, since], |row| {
            Ok(Red {
                report: row.get(0)?,
                repo: row.get(1)?,
                base_branch: row.get(2)?,
                commit: row.get(3)?,
                checks: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut complete = Vec::new();
    for mut red in reds {
        let Some(checks) = red_checks(db, &red.repo, &red.commit)? else {
            continue;
        };
        red.checks = checks;
        for check in &mut red.checks {
            check.passed = passed(
                db,
                &red.repo,
                &red.base_branch,
                &red.commit,
                since,
                &check.name,
                check.app.as_deref(),
                check.id,
            )?;
        }
        complete.push(red);
    }
    Ok(complete)
}

/// The checks of `repo` that failed at `commit`, each with its latest failing run. None when a
/// failing run has no id, since no run can then be later than it.
fn red_checks(
    db: &Connection,
    repo: &str,
    commit: &str,
) -> rusqlite::Result<Option<Vec<RedCheck>>> {
    db.prepare(
        "SELECT coalesce(json_extract(data, '$.name'), ''), json_extract(data, '$.app'),
           max(json_extract(data, '$.github_id')), coalesce(json_extract(data, '$.url'), ''),
           count(*) = count(json_extract(data, '$.github_id'))
         FROM inbox WHERE type = ?1 AND json_extract(data, '$.repo') = ?2
         AND json_extract(data, '$.head_sha') = ?3
         AND coalesce(json_extract(data, '$.conclusion'), '') NOT IN ('success', 'neutral', 'skipped')
         AND json_extract(data, '$.superseded_by') IS NULL
         GROUP BY 1, 2",
    )?
    .query_map(params![CHECKED, repo, commit], |row| {
        if !row.get::<_, bool>(4)? {
            return Ok(None);
        }
        Ok(Some(RedCheck {
            name: row.get(0)?,
            app: row.get(1)?,
            id: row.get::<_, i64>(2)? as u64,
            url: row.get(3)?,
            passed: Vec::new(),
        }))
    })?
    .collect()
}

/// The merge commits of `repo` into `base` since `since`, other than `commit`, whose checks all
/// completed and where every run of the check `name` of `app` after the run `id` succeeded, the 5
/// newest first.
#[allow(clippy::too_many_arguments)]
fn passed(
    db: &Connection,
    repo: &str,
    base: &str,
    commit: &str,
    since: &str,
    name: &str,
    app: Option<&str>,
    id: u64,
) -> rusqlite::Result<Vec<Passed>> {
    // A crash may have stored the passing events of a commit and not the failing ones.
    db.prepare(
        "WITH landed AS MATERIALIZED (
           SELECT DISTINCT json_extract(data, '$.merge_commit') AS sha FROM inbox
           WHERE type = ?1 AND julianday(time) >= julianday(?2)
           AND json_extract(data, '$.merged') = 1 AND json_extract(data, '$.repo') = ?3
           AND json_extract(data, '$.base_branch') = ?4
           AND json_extract(data, '$.merge_commit') != ?5),
         runs AS MATERIALIZED (
           SELECT json_extract(data, '$.head_sha') AS sha, json_extract(data, '$.github_id') AS id,
             json_extract(data, '$.conclusion') AS conclusion, json_extract(data, '$.url') AS url,
             json_extract(data, '$.pending') AS pending, json_extract(data, '$.failing') AS failing,
             json_extract(data, '$.superseded_by') AS superseded,
             json_extract(data, '$.name') = ?6 AND json_extract(data, '$.app') IS ?7
               AND coalesce(json_extract(data, '$.github_id') > ?8, 1) AS later
           FROM inbox WHERE type = ?9 AND julianday(time) >= julianday(?2)
           AND json_extract(data, '$.repo') = ?3)
         SELECT sha, json_group_array(url) FILTER (WHERE later) FROM landed JOIN runs USING (sha)
         GROUP BY sha
         HAVING min(conclusion IS 'success' AND id IS NOT NULL) FILTER (WHERE later) = 1
         AND min(CASE WHEN pending = 0 THEN failing END) <= sum(superseded IS NULL
           AND coalesce(conclusion, '') NOT IN ('success', 'neutral', 'skipped'))
         ORDER BY max(id) FILTER (WHERE later) DESC LIMIT 5",
    )?
    .query_map(
        params![
            PR_CLOSED, since, repo, base, commit, name, app, id as i64, CHECKED
        ],
        |row| {
            let urls: String = row.get(1)?;
            Ok(Passed {
                sha: row.get(0)?,
                urls: serde_json::from_str(&urls).unwrap_or_default(),
            })
        },
    )?
    .collect()
}

/// Whether `text` holds `reference` not followed by another digit, so `#5` is not `#57`.
fn mentions(text: &str, reference: &str) -> bool {
    text.match_indices(reference).any(|(at, _)| {
        !text[at + reference.len()..].starts_with(|next: char| next.is_ascii_digit())
    })
}

/// Each grade's tasks and their outcome labels, and each lane's with `--by lane`.
pub struct Grades {
    since: Option<String>,
    by_lane: bool,
    rows: Vec<Row>,
}

struct Row {
    /// The lane its tasks have now, with `--by lane`; `-` for a task with none.
    lane: Option<String>,
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
/// With `by_lane`, each grade splits into one row per lane its tasks have now.
pub fn grades(db: &Connection, since: Option<String>, by_lane: bool) -> Result<Grades, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let mut statement = db
        .prepare(
            "SELECT coalesce(tasks.lane, '-'), tasks.grade,
               group_concat(json_extract(trace.event, '$.data.label'), ' ')
             FROM tasks LEFT JOIN trace ON trace.type = 'factory.step.labeled'
               AND json_extract(trace.event, '$.data.task') = CAST(tasks.id AS TEXT)
             WHERE tasks.grade IS NOT NULL AND (?1 IS NULL OR tasks.created >= ?1)
             GROUP BY tasks.id",
        )
        .map_err(error)?;
    let tasks: Vec<(String, String, Option<String>)> = statement
        .query_map([&since], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .and_then(Iterator::collect)
        .map_err(error)?;
    let lanes: Vec<Option<&str>> = match by_lane {
        true => tasks
            .iter()
            .map(|(lane, ..)| lane.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(Some)
            .collect(),
        false => vec![None],
    };
    let mut rows = Vec::new();
    for lane in lanes {
        for grade in crate::tools::GRADES {
            let mut row = Row {
                lane: lane.map(str::to_owned),
                grade,
                tasks: 0,
                outcomes: 0,
                labels: [0; LABELS.len()],
            };
            for (_, _, labels) in tasks.iter().filter(|(task_lane, graded, _)| {
                graded == grade && lane.is_none_or(|lane| lane == task_lane)
            }) {
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
            rows.push(row);
        }
    }
    Ok(Grades {
        since,
        by_lane,
        rows,
    })
}

impl Grades {
    /// Green's tasks over every lane.
    fn green(&self) -> Row {
        let mut green = Row {
            lane: None,
            grade: "green",
            tasks: 0,
            outcomes: 0,
            labels: [0; LABELS.len()],
        };
        for row in self.rows.iter().filter(|row| row.grade == "green") {
            green.tasks += row.tasks;
            green.outcomes += row.outcomes;
            for (sum, count) in green.labels.iter_mut().zip(row.labels) {
                *sum += count;
            }
        }
        green
    }

    pub fn json(&self) -> Value {
        let green = self.green();
        json!({
            "since": self.since,
            "by": self.by_lane.then_some("lane"),
            "one_shot": {
                "rate": green.share(green.labels[0]),
                "clean": green.labels[0],
                "outcomes": green.outcomes,
                "target": TARGET,
            },
            "grades": self.rows.iter().map(|row| {
                let mut value = json!({
                    "grade": row.grade,
                    "tasks": row.tasks,
                    "outcomes": row.outcomes,
                    "labels": LABELS.iter().zip(row.labels).map(|(label, count)| {
                        (label.to_string(), json!({ "count": count, "share": row.share(count) }))
                    }).collect::<serde_json::Map<_, _>>(),
                });
                if let Some(lane) = &row.lane {
                    value["lane"] = json!(lane);
                }
                value
            }).collect::<Vec<_>>(),
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
        let lane = |row: &Row| {
            row.lane
                .as_ref()
                .map_or_else(String::new, |lane| format!("{lane}\t"))
        };
        crate::state::emit(&format!(
            "{}grade\ttasks\toutcomes\t{}",
            if self.by_lane { "lane\t" } else { "" },
            LABELS.join("\t")
        ));
        for row in &self.rows {
            let cells: Vec<String> = row
                .labels
                .iter()
                .map(|&count| format!("{count} ({})", percent(row.share(count))))
                .collect();
            crate::state::emit(&format!(
                "{}{}\t{}\t{}\t{}",
                lane(row),
                row.grade,
                row.tasks,
                row.outcomes,
                cells.join("\t")
            ));
        }
    }
}
