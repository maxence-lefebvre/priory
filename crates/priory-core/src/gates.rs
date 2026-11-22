//! Gates: a step's scorers, and the factory's critic for a plan no person reads, grade a result
//! before the daemon stores it. A failing scorer refuses the result and its reason reaches the agent
//! once; none can approve anything. docs/daemon/score.md describes them.

use crate::score::{self, Gauge, Judging, Run, Score, Verdict};
use crate::trace::{self, Trace};
use priory_factory::factory::{Decider, Factory, Intervention, ScoreKind, Step};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

/// What a gated judge may spend on one call, as a multiple of what one call cost on average when
/// it was calibrated: a real change can be longer than a labeled case.
const JUDGE_CALLS: f64 = 5.0;

/// The scorers that grade `outcome` at `step` of `task` before the daemon stores it: the step's
/// `gate` for the outcome, then the factory's critic when the outcome grades a plan that leads to a
/// model or code step and no person approves the plan, as `intervention = "plan"` makes one.
pub(crate) fn scorers(
    tx: &Transaction,
    task: i64,
    workflow: &str,
    step: &Step,
    outcome: &str,
    factory: &Factory,
) -> rusqlite::Result<Vec<String>> {
    let mut names = step.gate.get(outcome).cloned().unwrap_or_default();
    let Some(critic) = factory.critic() else {
        return Ok(names);
    };
    let unattended = step
        .next
        .get(outcome)
        .and_then(|target| factory.workflows.get(workflow)?.value.step(target))
        .is_some_and(|target| target.decides != Decider::Human);
    if !crate::tools::GRADES.contains(&outcome) || !unattended {
        return Ok(names);
    }
    let intervention: Option<String> = tx.query_row(
        "SELECT intervention FROM tasks WHERE id = ?1",
        [task],
        |row| row.get(0),
    )?;
    if intervention.as_deref() != Some(Intervention::Plan.as_str())
        && !names.iter().any(|name| name == critic)
    {
        names.push(critic.to_owned());
    }
    Ok(names)
}

/// Holds the result `arguments` of `run` for its gate's `scorers`, with `read` as they read it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hold(
    tx: &Transaction,
    run: i64,
    task: i64,
    step: &str,
    outcome: &str,
    arguments: &Value,
    read: &Value,
    scorers: &[String],
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO gates (run, task, step, outcome, arguments, result, scorers)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            run,
            task,
            step,
            outcome,
            arguments.to_string(),
            read.to_string(),
            serde_json::to_string(scorers).expect("names serialize")
        ],
    )
    .map(drop)
}

/// The outcome the run holds while its gate grades it, if any.
pub(crate) fn grading(tx: &Transaction, run: i64) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT outcome FROM gates WHERE run = ?1 AND state = 'grading'",
        [run],
        |row| row.get(0),
    )
    .optional()
}

/// Notes that the run's turn ended while its gate grades, and says whether one grades.
pub(crate) fn stopped(tx: &Transaction, run: i64) -> rusqlite::Result<bool> {
    let marked = tx.execute(
        "UPDATE gates SET stopped = 1 WHERE run = ?1 AND state = 'grading'",
        [run],
    )?;
    Ok(marked > 0)
}

/// A gate's scores, each with whether it counts: a judge counts only while its latest calibration
/// passes.
type Graded = Vec<(Score, bool)>;

/// The gradings in progress, each on a thread of its own, so a slow scorer never holds the
/// daemon's loop.
#[derive(Default)]
pub(crate) struct Grading {
    running: HashMap<i64, JoinHandle<Graded>>,
}

/// Drops the gates whose run ended, starts a grading for each gate that has none, and applies the
/// verdict of each grading that finished. Returns the tmux sessions to kill.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    grading: &mut Grading,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Vec<String>> {
    db.execute(
        "UPDATE gates SET state = 'dropped', reason = 'its run ended before the gate graded it',
         ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE state = 'grading' AND run IN (SELECT id FROM runs WHERE ended IS NOT NULL)",
        [],
    )?;
    let waiting: Vec<(i64, i64, String, String)> = {
        let mut statement = db.prepare(
            "SELECT id, task, result, scorers FROM gates WHERE state = 'grading' ORDER BY id",
        )?;
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?
    };
    for (id, task, read, names) in waiting {
        if grading.running.contains_key(&id) {
            continue;
        }
        let job = prepare(db, trace.factory, task, &read, &names)?;
        let home = home.to_path_buf();
        grading
            .running
            .insert(id, std::thread::spawn(move || grade(&home, job)));
    }
    let finished: Vec<i64> = grading
        .running
        .iter()
        .filter(|(_, thread)| thread.is_finished())
        .map(|(id, _)| *id)
        .collect();
    let mut kills = Vec::new();
    for id in finished {
        let thread = grading.running.remove(&id).expect("listed above");
        let graded = thread.join().unwrap_or_default();
        kills.extend(apply(db, trace, id, &graded, head)?);
    }
    Ok(kills)
}

/// What one grading reads, owned, to run on a thread of its own.
struct Job {
    dir: PathBuf,
    task: score::Task,
    run: Run,
    gauges: Vec<Gauge>,
    /// The scorers the gate names that the factory no longer has: each refuses.
    lost: Vec<String>,
}

fn prepare(
    db: &Connection,
    factory: &Factory,
    task: i64,
    read: &str,
    names: &str,
) -> rusqlite::Result<Job> {
    let result: Value = serde_json::from_str(read).unwrap_or_default();
    let names: Vec<String> = serde_json::from_str(names).unwrap_or_default();
    let (known, lost): (Vec<String>, Vec<String>) = names
        .into_iter()
        .partition(|name| factory.scorers.contains_key(name));
    let known: Vec<&str> = known.iter().map(String::as_str).collect();
    let calibrations = Judging::new(db, factory, None).calibrations;
    let gauges = score::gauges(factory, &known, &calibrations).unwrap_or_default();
    let task = score::task_of(db, task)?.unwrap_or(score::Task {
        repo: None,
        base: None,
        request: String::new(),
    });
    Ok(Job {
        dir: factory.dir.clone(),
        task,
        run: Run {
            id: result["run"].as_i64().unwrap_or_default(),
            step: result["step"].as_str().unwrap_or_default().to_owned(),
            result: Some(result),
            asks: Some(Vec::new()),
        },
        gauges,
        lost,
    })
}

/// Grades the held result with each scorer of `job`, in order. A judge whose calibration does not
/// pass still answers, with why, and does not count.
fn grade(home: &Path, job: Job) -> Graded {
    let mut graded: Graded = job
        .gauges
        .iter()
        .map(|gauge| {
            let (budget, counts) = match &gauge.judge {
                Some(Ok((_, calibration))) => {
                    (Some((calibration.per_call() * JUDGE_CALLS).max(0.01)), true)
                }
                Some(Err(_)) => (None, false),
                None => (None, true),
            };
            let mut judging = Judging {
                calibrations: BTreeMap::new(),
                budget,
                spent: 0.0,
            };
            let score = score::measure(&job.dir, home, &job.task, gauge, &job.run, &mut judging);
            (score, counts)
        })
        .collect();
    for name in job.lost {
        let score = Score {
            run: job.run.id,
            step: job.run.step.clone(),
            scorer: name.clone(),
            kind: ScoreKind::Outcome,
            verdict: Verdict::Error,
            reason: format!("no scorer {name} in scorers/"),
            cost: 0.0,
            judged: None,
        };
        graded.push((score, true));
    }
    graded
}

/// Records the scores of gate `id`, then stores its result and moves the task when none refused
/// it. A refusal reaches the agent once; a second one in the same run blocks the task for a
/// person. Returns the tmux session to kill.
fn apply(
    db: &mut Connection,
    trace: &Trace,
    id: i64,
    graded: &Graded,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Option<String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (run, task, outcome, arguments, state, stopped): (i64, i64, String, String, String, bool) =
        tx.query_row(
            "SELECT run, task, outcome, arguments, state, stopped FROM gates WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
    for (score, _) in graded {
        trace::scored(&tx, trace, task, score, Some(&outcome))?;
    }
    if state != "grading" {
        tx.commit()?;
        return Ok(None);
    }
    let mut refusals: Vec<String> = graded
        .iter()
        .filter(|(score, counts)| {
            *counts && matches!(score.verdict, Verdict::Fail | Verdict::Error)
        })
        .map(|(score, _)| format!("{}: {}", score.scorer, score.reason))
        .collect();
    if refusals.is_empty() {
        let arguments: Value = serde_json::from_str(&arguments).unwrap_or_default();
        match crate::tools::release(&tx, run, &arguments, stopped, trace, head)? {
            Ok(kill) => {
                end(&tx, id, "passed", None)?;
                tx.commit()?;
                return Ok(kill);
            }
            Err(why) => refusals.push(why),
        }
    }
    let reason = refusals.join("; ");
    let earlier: i64 = tx.query_row(
        "SELECT count(*) FROM gates WHERE run = ?1 AND state = 'refused'",
        [run],
        |row| row.get(0),
    )?;
    end(&tx, id, "refused", Some(&reason))?;
    if earlier == 0 {
        crate::tools::refused(
            &tx,
            task,
            &format!(
                "The gate refused outcome {outcome}. {reason}. Fix it, then send the result again. A second refusal in this run goes to a person."
            ),
        )?;
        tx.commit()?;
        return Ok(None);
    }
    let tmux: Option<String> = tx
        .query_row(
            "SELECT tmux FROM runs WHERE id = ?1 AND ended IS NULL",
            [run],
            |row| row.get(0),
        )
        .optional()?;
    tx.commit()?;
    let blocked = format!("the gate refused outcome {outcome} twice: {reason}");
    crate::state::block(db, trace, task, "refused", &blocked)?;
    Ok(tmux)
}

fn end(tx: &Transaction, id: i64, state: &str, reason: Option<&str>) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE gates SET state = ?2, reason = ?3, ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, state, reason],
    )
    .map(drop)
}
