//! `priory score`: grades the latest finished run of each step of a task with the factory's
//! scorers, and appends one `factory.run.scored` trace record per score.

use crate::trace::{self, Trace};
use priory_factory::factory::{Factory, Loaded, ScoreKind, Scorer};
use priory_runtime::{eval, room, runtime, script, transcript};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

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

/// The task as a scorer reads it.
struct Task {
    repo: Option<String>,
    base: Option<String>,
}

/// A finished run: one whose turn ended with a result or an ask.
struct Run {
    id: i64,
    step: String,
    result: Option<Value>,
}

/// Scores the latest finished run of each step of `task` with every scorer, or only `only`, then
/// appends their records to the trace. `home` holds Priory's clones.
pub fn score(
    db: &mut Connection,
    factory: &Factory,
    home: &Path,
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
        .query_row(
            "SELECT repo, base_sha FROM tasks WHERE id = ?1",
            [task],
            |row| {
                Ok(Task {
                    repo: row.get(0)?,
                    base: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(error)?;
    let Some(found) = found else {
        return Err(format!("no task {task}"));
    };
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
                ScoreKind::Command => command(factory, home, &found, &scorer.value, run),
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

fn command(
    factory: &Factory,
    home: &Path,
    task: &Task,
    scorer: &Scorer,
    run: &Run,
) -> (Verdict, String) {
    let error = |reason: String| (Verdict::Error, reason);
    let Some(result) = &run.result else {
        return (Verdict::Fail, NO_RESULT.to_owned());
    };
    let Some(head) = result["commit"].as_str() else {
        return error("the result names no commit".to_owned());
    };
    let Some(base) = &task.base else {
        return error("no start commit".to_owned());
    };
    let Some(program) = &scorer.run else {
        return error("the scorer names no `run`".to_owned());
    };
    let clone = match &task.repo {
        Some(repo) => home.join("repos").join(repo),
        None => return error("the task names no repository".to_owned()),
    };
    if !clone.is_dir() {
        return error(format!("no clone at {}", clone.display()));
    }
    let scratch = home.join("scores").join(uuid::Uuid::new_v4().to_string());
    let added = git(&clone)
        .args(["worktree", "add", "--quiet", "--detach"])
        .arg(&scratch)
        .arg(head)
        .output();
    match added {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let why = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return error(format!("cannot check out {head}: {why}"));
        }
        Err(why) => return error(format!("cannot run git: {why}")),
    }
    let answer = run_command(
        &factory.dir.join(program),
        &scratch,
        base,
        head,
        result,
        scorer.timeout(),
    );
    let _ = git(&clone)
        .args(["worktree", "remove", "--force"])
        .arg(&scratch)
        .output();
    answer
}

/// Runs `program` in `dir` with a cleared environment, and reads its exit code as the verdict.
fn run_command(
    program: &Path,
    dir: &Path,
    base: &str,
    head: &str,
    result: &Value,
    timeout: Duration,
) -> (Verdict, String) {
    let error = |reason: String| (Verdict::Error, reason);
    let file = dir.with_extension("result.json");
    let log = dir.with_extension("log");
    if let Err(why) = std::fs::write(&file, result.to_string()) {
        return error(format!("{}: {why}", file.display()));
    }
    let mut command = Command::new(program);
    command.env_clear();
    // Without PATH and HOME, a scorer could not find the toolchain the gate needs.
    for name in ["PATH", "HOME"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("PRIORY_SCORE_BASE", base)
        .env("PRIORY_SCORE_HEAD", head)
        .env("PRIORY_SCORE_RESULT", &file);
    let answer = match script::spawn_command(command, dir, &log, timeout) {
        Err(why) => error(why),
        Ok(mut running) => loop {
            if let Some((status, output)) = running.poll() {
                let last = output.lines().last().unwrap_or_default().trim().to_owned();
                let reason = |fallback: &str| match last.is_empty() {
                    true => fallback.to_owned(),
                    false => last.clone(),
                };
                break match status.as_str() {
                    "exit 0" => (Verdict::Pass, reason("exit 0")),
                    "exit 1" => (Verdict::Fail, reason("exit 1")),
                    _ if last.is_empty() => error(status),
                    _ => error(format!("{status}: {last}")),
                };
            }
            std::thread::sleep(Duration::from_millis(50));
        },
    };
    let _ = std::fs::remove_file(&file);
    answer
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}
