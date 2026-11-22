//! Held scorers: a scorer of kind `held` grades a merged task once the moment its `after` names
//! comes, from what followed the merge, and appends one `factory.step.labeled` record per task.

use crate::score::Verdict;
use crate::trace::{self, SCORED, Trace};
use jiff::{SignedDuration, Timestamp};
use priory_factory::factory::{ScoreKind, Scorer};
use priory_intake::intake::PR_CLOSED;
use rusqlite::{Connection, Transaction, TransactionBehavior};

/// Something that followed a task's merge and can fail a `held` scorer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    /// One of the scorer's `fails`, such as `reverted` or `fixed`.
    pub name: String,
    /// When it happened, when Priory knows.
    pub at: Option<Timestamp>,
    /// The URL or task that shows it.
    pub evidence: String,
}

/// Grades a task with the signals `fails` names: it fails when one of them happened by
/// `deadline`, and passes otherwise. `after` is the scorer's key as written, for the reason.
pub fn grade(
    signals: &[Signal],
    fails: &[String],
    deadline: Timestamp,
    after: &str,
) -> (Verdict, String) {
    let found: Vec<String> = signals
        .iter()
        .filter(|signal| fails.contains(&signal.name))
        .filter(|signal| signal.at.is_none_or(|at| at <= deadline))
        .map(|signal| format!("{} {}", signal.name, signal.evidence))
        .collect();
    match found.is_empty() {
        true => (Verdict::Pass, format!("no {} by {after}", or(fails))),
        false => (Verdict::Fail, found.join("; ")),
    }
}

/// `a`, `a or b`, `a, b or c`.
fn or(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// Grades every merged task whose moment came by `now` with each `held` scorer of the factory
/// that has not graded it yet. Returns how many records it appended.
pub(crate) fn tick(db: &mut Connection, trace: &Trace, now: Timestamp) -> rusqlite::Result<usize> {
    let scorers: Vec<(&String, &Scorer)> = trace
        .factory
        .scorers
        .iter()
        .map(|(name, scorer)| (name, &scorer.value))
        .filter(|(_, scorer)| scorer.kind == ScoreKind::Held && scorer.after.is_some())
        .collect();
    if scorers.is_empty() {
        return Ok(0);
    }
    let merges = merges(db)?;
    let mut written = 0;
    for (task, merged) in merges {
        let Ok(merged) = merged.parse::<Timestamp>() else {
            continue;
        };
        let due: Vec<_> = scorers
            .iter()
            .filter_map(|(name, scorer)| {
                let after = scorer.after.as_ref()?;
                let delay = SignedDuration::try_from(after.delay).ok()?;
                let deadline = merged.checked_add(delay).ok()?;
                (deadline <= now).then_some((*name, *scorer, &after.text, deadline))
            })
            .collect();
        if due.is_empty() {
            continue;
        }
        // A deferred transaction that reads first cannot wait for another writer: it fails at once.
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let signals = signals(&tx, task)?;
        for (name, scorer, after, deadline) in due {
            if graded(&tx, task, name)? {
                continue;
            }
            let (verdict, reason) = grade(&signals, &scorer.fails, deadline, after);
            let happened = deadline.to_string();
            trace::held(&tx, trace, task, name, verdict.name(), &reason, &happened)?;
            written += 1;
        }
        tx.commit()?;
    }
    Ok(written)
}

/// Each task whose pull requests all merged, with when the last of them merged: the merged closes
/// of the pull requests in its stack and of those whose head branch it owns, as outcome labels
/// find a pull request's authors.
fn merges(db: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    db.prepare(
        "WITH merges AS (
           SELECT time,
             'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') AS pull,
             'github:' || json_extract(data, '$.repo') || '@' || json_extract(data, '$.head_branch') AS branch
           FROM inbox WHERE type = ?1 AND json_extract(data, '$.merged') = 1),
         authored AS (
           SELECT pull_requests.task, merges.time FROM merges
             JOIN pull_requests ON pull_requests.resource = merges.pull
           UNION ALL SELECT owns.task, merges.time FROM merges JOIN owns ON owns.resource = merges.branch)
         SELECT task, max(time) FROM authored
         WHERE NOT EXISTS (SELECT 1 FROM pull_requests AS stacked
           WHERE stacked.task = authored.task AND stacked.state != 'merged')
         GROUP BY task ORDER BY task",
    )?
    .query_map([PR_CLOSED], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// Whether `scorer` already graded `task`.
fn graded(tx: &Transaction, task: i64, scorer: &str) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM trace WHERE type = 'factory.step.labeled'
           AND json_extract(event, '$.data.task') = ?1 AND json_extract(event, '$.data.label') = ?2
           AND json_extract(event, '$.data.scorer') = ?3)",
        rusqlite::params![task.to_string(), SCORED, scorer],
        |row| row.get(0),
    )
}

/// What followed `task`: each outcome label no clear withdrew, and each later task that fixes it.
fn signals(tx: &Transaction, task: i64) -> rusqlite::Result<Vec<Signal>> {
    let at = |text: Option<String>| text.and_then(|text| text.parse::<Timestamp>().ok());
    let mut signals: Vec<Signal> = tx
        .prepare(
            "SELECT json_extract(event, '$.data.label'),
               coalesce(json_extract(event, '$.data.happened'), time),
               coalesce(json_extract(event, '$.data.evidence'), '')
             FROM trace AS labeled WHERE type = 'factory.step.labeled'
             AND json_extract(event, '$.data.task') = ?1 AND json_extract(event, '$.data.label') != ?2
             AND NOT EXISTS (SELECT 1 FROM trace AS cleared WHERE cleared.type = 'factory.step.unlabeled'
               AND cleared.seq > labeled.seq
               AND json_extract(cleared.event, '$.data.task') = ?1
               AND json_extract(cleared.event, '$.data.label') = json_extract(labeled.event, '$.data.label'))
             ORDER BY seq",
        )?
        .query_map(rusqlite::params![task.to_string(), SCORED], |row| {
            Ok(Signal {
                name: row.get(0)?,
                at: at(row.get(1)?),
                evidence: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let fixes: Vec<Signal> = tx
        .prepare(
            "SELECT task, created FROM relations WHERE kind = 'fixes' AND other = ?1 ORDER BY task",
        )?
        .query_map([task], |row| {
            Ok(Signal {
                name: "fixed".to_owned(),
                at: at(row.get(1)?),
                evidence: format!("by task {}", row.get::<_, i64>(0)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    signals.extend(fixes);
    Ok(signals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(name: &str, at: &str) -> Signal {
        Signal {
            name: name.to_owned(),
            at: Some(at.parse().unwrap()),
            evidence: "https://github.com/o/r/pull/9".to_owned(),
        }
    }

    #[test]
    fn a_signal_by_the_deadline_fails_and_one_after_it_or_not_named_does_not() {
        let deadline: Timestamp = "2026-11-08T00:00:00Z".parse().unwrap();
        let fails = vec!["reverted".to_owned(), "fixed".to_owned()];
        let signals = [
            signal("reverted", "2026-11-09T00:00:00Z"),
            signal("red-after-merge", "2026-11-02T00:00:00Z"),
        ];
        assert_eq!(
            grade(&signals, &fails, deadline, "merged + 7d"),
            (
                Verdict::Pass,
                "no reverted or fixed by merged + 7d".to_owned()
            )
        );
        let signals = [signal("reverted", "2026-11-08T00:00:00Z")];
        assert_eq!(
            grade(&signals, &fails, deadline, "merged + 7d"),
            (
                Verdict::Fail,
                "reverted https://github.com/o/r/pull/9".to_owned()
            )
        );
    }

    #[test]
    fn a_signal_with_no_time_counts() {
        let deadline: Timestamp = "2026-11-08T00:00:00Z".parse().unwrap();
        let signals = [Signal {
            name: "fixed".to_owned(),
            at: None,
            evidence: "by task 4".to_owned(),
        }];
        let fails = ["fixed".to_owned()];
        assert_eq!(
            grade(&signals, &fails, deadline, "merged + 1h").0,
            Verdict::Fail
        );
    }
}
