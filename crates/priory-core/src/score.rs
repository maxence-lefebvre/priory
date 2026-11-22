//! `priory score`: grades the latest finished run of each step of a task with the factory's
//! scorers, and appends one `factory.run.scored` trace record per score.

use crate::trace::{self, Trace};
use priory_factory::factory::{Factory, Loaded, ScoreKind, Scorer};
use priory_runtime::{eval, room, runtime, transcript};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

/// What a scorer answered for one run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    NotApplicable,
    Error,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "fail",
            Verdict::NotApplicable => "not applicable",
            Verdict::Error => "error",
        }
    }
}

/// One scorer's verdict on one run, with its line of why.
pub struct Score {
    pub run: i64,
    pub step: String,
    pub scorer: String,
    pub kind: ScoreKind,
    pub verdict: Verdict,
    pub reason: String,
}

impl Score {
    pub fn json(&self) -> Value {
        json!({
            "run": self.run,
            "step": self.step,
            "scorer": self.scorer,
            "verdict": self.verdict.name(),
            "reason": self.reason,
        })
    }
}

/// A finished run: one whose turn ended with a result or an ask.
struct Run {
    id: i64,
    step: String,
    result: Option<Value>,
}

/// Scores the latest finished run of each step of `task` with every scorer, or only `only`, then
/// appends their records to the trace.
pub fn score(
    db: &mut Connection,
    factory: &Factory,
    task: i64,
    only: Option<&str>,
) -> Result<Vec<Score>, String> {
    let scorers: Vec<(&String, &Loaded<Scorer>)> = match only {
        Some(name) => match factory.scorers.get_key_value(name) {
            Some(found) => vec![found],
            None => return Err(format!("no scorer {name} in scorers/")),
        },
        None => factory.scorers.iter().collect(),
    };
    if scorers.is_empty() {
        return Err("the factory has no scorers: add one to scorers/".to_owned());
    }
    let error = |error: rusqlite::Error| error.to_string();
    let found = db
        .query_row("SELECT 1 FROM tasks WHERE id = ?1", [task], |_| Ok(()))
        .optional()
        .map_err(error)?;
    if found.is_none() {
        return Err(format!("no task {task}"));
    }
    let runs = finished(db, task).map_err(error)?;
    if runs.is_empty() {
        return Err(format!("task {task} has no finished run"));
    }
    let mut scores = Vec::new();
    for run in &runs {
        for (name, scorer) in &scorers {
            let (verdict, reason) = match scorer.value.kind {
                ScoreKind::Outcome => outcome(&scorer.value, run),
                ScoreKind::AskShape => ask_shape(db, &scorer.value, run).map_err(error)?,
                ScoreKind::Command => (Verdict::Error, "command scorers do not run yet".to_owned()),
            };
            scores.push(Score {
                run: run.id,
                step: run.step.clone(),
                scorer: (*name).clone(),
                kind: scorer.value.kind,
                verdict,
                reason,
            });
        }
    }
    let machine = room::hostname();
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        runner: None,
        machine,
        usage: transcript::usage,
    };
    let tx = db.transaction().map_err(error)?;
    for score in &scores {
        trace::scored(&tx, &trace, task, score).map_err(error)?;
    }
    tx.commit().map_err(error)?;
    Ok(scores)
}

/// The latest finished run of each step of `task`, oldest first.
fn finished(db: &Connection, task: i64) -> rusqlite::Result<Vec<Run>> {
    let mut statement = db.prepare(
        "WITH done AS (
           SELECT id, step FROM runs WHERE task = ?1
             AND (EXISTS (SELECT 1 FROM results WHERE results.run = runs.id)
               OR EXISTS (SELECT 1 FROM asks WHERE asks.run = runs.id)))
         SELECT done.id, done.step,
           (SELECT json_object('run', run, 'step', step, 'outcome', outcome, 'summary', summary,
              'evidence', evidence, 'commit', commit_sha, 'reason', reason)
            FROM results WHERE results.run = done.id)
         FROM done WHERE done.id = (SELECT max(id) FROM done AS later WHERE later.step = done.step)
         ORDER BY done.id",
    )?;
    statement
        .query_map([task], |row| {
            let result: Option<String> = row.get(2)?;
            Ok(Run {
                id: row.get(0)?,
                step: row.get(1)?,
                result: result.and_then(|text| serde_json::from_str(&text).ok()),
            })
        })?
        .collect()
}

const NO_RESULT: &str = "the run asked and sent no result";

fn outcome(scorer: &Scorer, run: &Run) -> (Verdict, String) {
    let Some(outcome) = run
        .result
        .as_ref()
        .and_then(|result| result["outcome"].as_str())
    else {
        return (Verdict::Fail, NO_RESULT.to_owned());
    };
    if scorer.expect.iter().any(|expected| expected == outcome) {
        (Verdict::Pass, format!("outcome {outcome}"))
    } else {
        let expected = scorer.expect.join(" or ");
        (Verdict::Fail, format!("outcome {outcome}, not {expected}"))
    }
}

fn ask_shape(db: &Connection, scorer: &Scorer, run: &Run) -> rusqlite::Result<(Verdict, String)> {
    let mut statement = db.prepare(
        "SELECT id, question, what_waits, next_action FROM asks WHERE run = ?1 ORDER BY id",
    )?;
    let asks = statement
        .query_map([run.id], |row| {
            let ask = json!({
                "question": row.get::<_, Option<String>>(1)?,
                "whatWaits": row.get::<_, Option<String>>(2)?,
                "nextAction": row.get::<_, Option<String>>(3)?,
            });
            Ok((row.get::<_, i64>(0)?, eval::shape_of(&ask)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if asks.is_empty() {
        return Ok((Verdict::NotApplicable, "the run asked nothing".to_owned()));
    }
    let lacking: Vec<String> = asks
        .iter()
        .filter_map(|(id, shape)| {
            let shape: Vec<&str> = shape.split_whitespace().collect();
            let missing: Vec<&str> = scorer
                .expect
                .iter()
                .map(String::as_str)
                .filter(|part| !shape.contains(part))
                .collect();
            (!missing.is_empty()).then(|| format!("ask {id} lacks {}", missing.join(", ")))
        })
        .collect();
    Ok(match lacking.is_empty() {
        true => (
            Verdict::Pass,
            format!("{} ask(s) show {}", asks.len(), scorer.expect.join(", ")),
        ),
        false => (Verdict::Fail, lacking.join("; ")),
    })
}
