//! `priory score`: grades the latest finished run of each step of a task with the factory's
//! scorers, and appends one `factory.run.scored` trace record per score.

use crate::judge::{self, Calibration};
use crate::trace::{self, Trace};
use priory_factory::factory::{Factory, Loaded, ScoreKind, Scorer};
use priory_runtime::{eval, room, runtime, script, transcript};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::{Value, json};
use std::collections::BTreeMap;
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
    /// What a judge's call cost, in dollars; 0 for a scorer that calls no model.
    pub cost: f64,
    /// For a calibrated judge, its model and the calibration its verdict rests on.
    pub judged: Option<Judged>,
}

/// The model and calibration behind a judge's verdict.
pub struct Judged {
    pub model: String,
    pub calibration: Calibration,
}

impl Score {
    pub fn json(&self) -> Value {
        let mut value = json!({
            "run": self.run,
            "step": self.step,
            "scorer": self.scorer,
            "verdict": self.verdict.name(),
            "reason": self.reason,
        });
        if let Some(judged) = &self.judged {
            value["agreement"] = json!(judged.calibration.shown());
        }
        value
    }
}

/// What the judges of one scoring may use: each judge's calibration or why it does not count, the
/// budget in dollars, if one was given, and what they spent of it.
pub struct Judging {
    pub calibrations: BTreeMap<String, Result<Calibration, String>>,
    pub budget: Option<f64>,
    pub spent: f64,
}

impl Judging {
    /// Reads the calibration of each judge scorer of `factory` from `db`, this machine's state.
    pub fn new(db: &Connection, factory: &Factory, budget: Option<f64>) -> Judging {
        let calibrations = factory
            .scorers
            .iter()
            .filter(|(_, scorer)| scorer.value.kind == ScoreKind::Judge)
            .map(|(name, _)| (name.clone(), judge::calibrated(db, factory, name)))
            .collect();
        Judging {
            calibrations,
            budget,
            spent: 0.0,
        }
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
    budget: Option<f64>,
) -> Result<Vec<Score>, String> {
    let names: Vec<&str> = match only {
        Some(name) if factory.scorers.contains_key(name) => vec![name],
        Some(name) => return Err(format!("no scorer {name} in scorers/")),
        None => factory.scorers.keys().map(String::as_str).collect(),
    };
    if names.is_empty() {
        return Err("the factory has no scorers: add one to scorers/".to_owned());
    }
    crate::sweep::leftovers(home, &std::env::temp_dir());
    let mut judging = Judging::new(db, factory, budget);
    let scores = grade(db, factory, home, task, &names, &mut judging)?;
    let error = |error: rusqlite::Error| error.to_string();
    let machine = room::hostname();
    let trace = Trace {
        factory,
        revision: runtime::revision(&factory.dir),
        runner: None,
        machine,
        usage: transcript::usage,
    };
    // A deferred transaction that reads first cannot wait for the daemon's write: it fails at once.
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    for score in &scores {
        trace::scored(&tx, &trace, task, score).map_err(error)?;
    }
    tx.commit().map_err(error)?;
    Ok(scores)
}

/// Scores the latest finished run of each step of `task` with each scorer `names` holds, and
/// records nothing. Each name must be one of the factory's scorers.
pub fn grade(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    task: i64,
    names: &[&str],
    judging: &mut Judging,
) -> Result<Vec<Score>, String> {
    let error = |error: rusqlite::Error| error.to_string();
    let mut scorers: Vec<(&String, &Loaded<Scorer>)> = Vec::new();
    for name in names {
        match factory.scorers.get_key_value(*name) {
            Some(found) => scorers.push(found),
            None => return Err(format!("no scorer {name} in scorers/")),
        }
    }
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
            let (mut cost, mut judged) = (0.0, None);
            let (verdict, reason) = match scorer.value.kind {
                ScoreKind::Outcome => outcome(&scorer.value, run),
                ScoreKind::AskShape => ask_shape(db, &scorer.value, run).map_err(error)?,
                ScoreKind::Command => command(factory, home, &found, &scorer.value, run),
                ScoreKind::Judge => {
                    let graded = judge(db, factory, home, &found, name, run, task, judging)
                        .map_err(error)?;
                    cost = graded.cost;
                    judged = graded.judged;
                    (graded.verdict, graded.reason)
                }
            };
            scores.push(Score {
                run: run.id,
                step: run.step.clone(),
                scorer: (*name).clone(),
                kind: scorer.value.kind,
                verdict,
                reason,
                cost,
                judged,
            });
        }
    }
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

/// What a `diff` scorer reads of a run: its repository, start commit, result commit and result.
struct Target<'a> {
    repo: &'a str,
    base: &'a str,
    head: &'a str,
    result: &'a Value,
}

/// The change a `diff` scorer reads, or the verdict when there is none to read.
fn target<'a>(task: &'a Task, run: &'a Run) -> Result<Target<'a>, (Verdict, String)> {
    let error = |reason: String| (Verdict::Error, reason);
    let Some(result) = &run.result else {
        return Err((Verdict::Fail, NO_RESULT.to_owned()));
    };
    let Some(head) = result["commit"].as_str() else {
        return Err(error("the result names no commit".to_owned()));
    };
    // The agent wrote it, and git would read a value starting with `-` as an option.
    if !(7..=64).contains(&head.len()) || !head.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(error(format!(
            "the result's commit {head:?} is not a commit hash"
        )));
    }
    let Some(base) = &task.base else {
        return Err(error("no start commit".to_owned()));
    };
    let Some(repo) = &task.repo else {
        return Err(error("the task names no repository".to_owned()));
    };
    Ok(Target {
        repo,
        base,
        head,
        result,
    })
}

fn command(
    factory: &Factory,
    home: &Path,
    task: &Task,
    scorer: &Scorer,
    run: &Run,
) -> (Verdict, String) {
    match target(task, run) {
        Err(answer) => answer,
        Ok(at) => on_commit(factory, home, at.repo, at.base, at.head, scorer, at.result),
    }
}

/// A judge's verdict on one run, with what it cost.
struct Graded {
    verdict: Verdict,
    reason: String,
    cost: f64,
    judged: Option<Judged>,
}

/// Asks the judge scorer `name` about `run`'s change, once its calibration counts and within the
/// budget `judging` holds.
#[allow(clippy::too_many_arguments)]
fn judge(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    task: &Task,
    name: &str,
    run: &Run,
    id: i64,
    judging: &mut Judging,
) -> rusqlite::Result<Graded> {
    let answer = |verdict, reason: String| Graded {
        verdict,
        reason,
        cost: 0.0,
        judged: None,
    };
    let calibration = match judging.calibrations.get(name) {
        Some(Ok(calibration)) => calibration.clone(),
        Some(Err(why)) => return Ok(answer(Verdict::Error, why.clone())),
        None => return Ok(answer(Verdict::Error, "not calibrated".to_owned())),
    };
    let at = match target(task, run) {
        Ok(at) => at,
        Err((verdict, reason)) => return Ok(answer(verdict, reason)),
    };
    let Some(budget) = judging.budget else {
        return Ok(answer(
            Verdict::Error,
            "a judge costs money: give --budget USD".to_owned(),
        ));
    };
    if judging.spent >= budget {
        return Ok(answer(
            Verdict::Error,
            "over budget: the judges spent the budget".to_owned(),
        ));
    }
    let diff = match diff(home, at.repo, at.base, at.head) {
        Ok(diff) => diff,
        Err(why) => return Ok(answer(Verdict::Error, why)),
    };
    let request = request(db, id)?;
    let opened = match judge::open(factory, name) {
        Ok(opened) => opened,
        Err(why) => return Ok(answer(Verdict::Error, why)),
    };
    let read = judge::Read {
        request: &request,
        result: at.result,
        diff: &diff,
    };
    let asked = judge::ask(&opened, &home.join("scores"), &read, budget - judging.spent);
    judging.spent += asked.cost;
    let judged = Some(Judged {
        model: opened.model.to_owned(),
        calibration,
    });
    Ok(Graded {
        verdict: asked.verdict,
        reason: asked.reason,
        cost: asked.cost,
        judged,
    })
}

/// What `head` changed since `base`, from Priory's clone of `repo` under `home`.
fn diff(home: &Path, repo: &str, base: &str, head: &str) -> Result<String, String> {
    let clone = home.join("repos").join(repo);
    let output = git(&clone)
        .args(["diff", "--no-color", "--no-ext-diff", base, head, "--"])
        .output()
        .map_err(|why| format!("cannot run git: {why}"))?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!("cannot read the diff of {head}: {why}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The data of the message that asked for task `id`, as JSON, or a line that says it is gone.
fn request(db: &Connection, id: i64) -> rusqlite::Result<String> {
    let data: Option<String> = db
        .query_row(
            "SELECT data FROM inbox WHERE task = ?1 ORDER BY seq LIMIT 1",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(data
        .and_then(|data| serde_json::from_str::<Value>(&data).ok())
        .map(|data| serde_json::to_string_pretty(&data).unwrap_or_default())
        .unwrap_or_else(|| "(the request is not in the inbox)".to_owned()))
}

/// Runs the `command` scorer `scorer` on the commit `head` of `repo`, from Priory's clone under
/// `home`, against the start commit `base`, with `result` as the result it reads.
pub fn on_commit(
    factory: &Factory,
    home: &Path,
    repo: &str,
    base: &str,
    head: &str,
    scorer: &Scorer,
    result: &Value,
) -> (Verdict, String) {
    let error = |reason: String| (Verdict::Error, reason);
    let Some(program) = &scorer.run else {
        return error("the scorer names no `run`".to_owned());
    };
    let program = match inside(&factory.dir, program) {
        Ok(program) => program,
        Err(why) => return error(why),
    };
    let clone = home.join("repos").join(repo);
    if !clone.is_dir() {
        return error(format!("no clone at {}", clone.display()));
    }
    let scratch = home.join("scores").join(crate::sweep::scratch_name());
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
    let answer = run_command(&program, &scratch, base, head, result, scorer.timeout());
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

/// `run` resolved in the factory folder `dir`, refused when it leads outside it, as through `..`
/// or a symbolic link.
fn inside(dir: &Path, run: &Path) -> Result<std::path::PathBuf, String> {
    let shown = run.display();
    let folder = dir
        .canonicalize()
        .map_err(|why| format!("cannot read the factory folder: {why}"))?;
    let program = dir
        .join(run)
        .canonicalize()
        .map_err(|why| format!("`run` names {shown}, which cannot be read: {why}"))?;
    if !program.starts_with(&folder) {
        return Err(format!(
            "`run` names {shown}, which leads outside the factory folder"
        ));
    }
    Ok(program)
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}
