//! Gates: a step's scorers, and the factory's critic for a plan no person reads, grade a result
//! before the daemon stores it. A failing scorer refuses the result and its reason reaches the agent
//! once; none can approve anything. The factory's pre-review reads an approval step's lessons
//! before that step asks a person, and sends a result back once. docs/daemon/score.md describes
//! them.

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
/// model or code step and no person approves the plan, as `intervention = "plan"` makes one, unless
/// autopilot may approve it.
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
    let unread = intervention.as_deref() != Some(Intervention::Plan.as_str())
        || crate::autopilot::approves_plans(tx, factory, task, workflow)?;
    if unread && !names.iter().any(|name| name == critic) {
        names.push(critic.to_owned());
    }
    Ok(names)
}

/// The factory's pre-review of `outcome` at `step` of `run`, with the lessons it reads: when the
/// outcome leads to a human step with `approves`, the task sees at least one lesson for that step,
/// and no pre-review refused a result of this run before, since it sends a result back once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pre_review(
    tx: &Transaction,
    run: i64,
    task: i64,
    workflow: &str,
    step: &Step,
    outcome: &str,
    factory: &Factory,
    kit: crate::kit::Current<'_>,
) -> rusqlite::Result<Option<(String, Vec<String>)>> {
    let Some(name) = factory.pre_review() else {
        return Ok(None);
    };
    let Some(target) = step.next.get(outcome).filter(|target| {
        factory
            .workflows
            .get(workflow)
            .and_then(|flow| flow.value.step(target))
            .is_some_and(|target| target.decides == Decider::Human && !target.approves.is_empty())
    }) else {
        return Ok(None);
    };
    let refused: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM gates WHERE run = ?1 AND state = 'refused'
           AND lessons IS NOT NULL)",
        [run],
        |row| row.get(0),
    )?;
    if refused {
        return Ok(None);
    }
    // The task's own lessons count, but never its forgets nor rewrites: its agent writes those.
    let mut layer = crate::memories::layer(tx, None)?;
    layer.extend(
        crate::memories::layer(tx, Some(task))?
            .into_iter()
            .filter(|(_, memory)| memory.as_ref().is_some_and(|memory| memory.step.is_some())),
    );
    let lessons =
        crate::kit::sources(factory, kit, &layer, None).lessons(&format!("{workflow}/{target}"));
    Ok((!lessons.is_empty()).then(|| (name.to_owned(), lessons)))
}

/// Holds the result `arguments` of `run` for its gate's `scorers`, with `read` as they read it, and
/// the `lessons` its pre-review reads.
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
    lessons: Option<&[String]>,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO gates (run, task, step, outcome, arguments, result, scorers, lessons)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            run,
            task,
            step,
            outcome,
            arguments.to_string(),
            read.to_string(),
            serde_json::to_string(scorers).expect("names serialize"),
            lessons.map(|lessons| serde_json::to_string(lessons).expect("lessons serialize"))
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
    drop_ended(db)?;
    let mut kills = Vec::new();
    let waiting: Vec<(i64, i64, String, String, Option<String>)> = {
        let mut statement = db.prepare(
            "SELECT id, task, result, scorers, lessons FROM gates WHERE state = 'grading'
             ORDER BY id",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
    };
    for (id, task, read, names, lessons) in waiting {
        if grading.running.contains_key(&id) {
            continue;
        }
        match asked(db, id)? {
            Asked::Open => {
                let awake: bool = db.query_row(
                    "SELECT EXISTS (SELECT 1 FROM gates JOIN runs ON runs.id = gates.run
                       WHERE gates.id = ?1 AND gates.stopped AND runs.ended IS NULL)",
                    [id],
                    |row| row.get(0),
                )?;
                if !awake {
                    continue;
                }
                let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
                kills.extend(park(&tx, id)?);
                tx.commit()?;
                continue;
            }
            Asked::Through => {
                kills.extend(apply(db, trace, id, &Vec::new(), true, head)?);
                continue;
            }
            Asked::None | Asked::Again => {}
        }
        let mut job = prepare(db, trace.factory, task, &read, &names)?;
        job.lessons = lessons
            .and_then(|lessons| serde_json::from_str(&lessons).ok())
            .unwrap_or_default();
        job.pre_review = trace.factory.pre_review().map(str::to_owned);
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
    for id in finished {
        let thread = grading.running.remove(&id).expect("listed above");
        let (run, step): (i64, String) =
            db.query_row("SELECT run, step FROM gates WHERE id = ?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;
        let graded = joined(thread.join(), run, &step);
        kills.extend(apply(db, trace, id, &graded, false, head)?);
    }
    Ok(kills)
}

/// A gate that waits on a person's ask keeps its result while its run sleeps and its task stays
/// asleep at the gate's step with no later run. Any other gate whose run ended drops its result.
const PARKED: &str = "gates.ask IS NOT NULL AND runs.ended_as = 'asleep' AND EXISTS (
  SELECT 1 FROM tasks WHERE tasks.id = gates.task AND tasks.state = 'asleep'
  AND tasks.step = gates.step)
  AND NOT EXISTS (SELECT 1 FROM runs AS later WHERE later.task = gates.task AND later.id > gates.run)";

/// Drops the gates whose run ended, except those that sleep on an ask. An ask no gate waits on any
/// more is withdrawn, and one whose task dropped its gate moves to a task that still waits on it.
fn drop_ended(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let dropped: Vec<(i64, Option<i64>)> = {
        let mut statement = tx.prepare(&format!(
            "SELECT gates.id, gates.ask FROM gates JOIN runs ON runs.id = gates.run
             WHERE gates.state = 'grading' AND runs.ended IS NOT NULL AND NOT ({PARKED})"
        ))?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (id, ask) in &dropped {
        end(
            &tx,
            *id,
            "dropped",
            Some("its run ended before the gate graded it"),
        )?;
        let Some(ask) = ask else {
            continue;
        };
        tx.execute(
            "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'moved-on'
             WHERE id = ?1 AND answer IS NULL AND withdrawn IS NULL
             AND NOT EXISTS (SELECT 1 FROM gates WHERE ask = ?1 AND state = 'grading')",
            [ask],
        )?;
        tx.execute(
            "UPDATE asks SET task = (SELECT task FROM gates WHERE ask = ?1 AND state = 'grading'
               ORDER BY id LIMIT 1),
               step = (SELECT step FROM gates WHERE ask = ?1 AND state = 'grading' ORDER BY id LIMIT 1)
             WHERE id = ?1 AND answer IS NULL AND withdrawn IS NULL
             AND task NOT IN (SELECT task FROM gates WHERE ask = ?1 AND state = 'grading')",
            [ask],
        )?;
        waiting(&tx, *ask)?;
    }
    tx.commit()
}

/// Ends the live run of gate `id` as asleep once its turn ended, since the result waits on a
/// person's ask and a waiting agent holds no process. Returns the tmux session to kill.
fn park(tx: &Transaction, id: i64) -> rusqlite::Result<Option<String>> {
    let (run, stopped): (i64, bool) = tx.query_row(
        "SELECT run, stopped FROM gates WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !stopped {
        return Ok(None);
    }
    crate::tools::sleep(tx, run)
}

/// The scores of a finished grading thread. A thread that panicked graded nothing, so one error
/// that counts refuses the result: a gate that cannot grade cannot let it through.
fn joined(thread: std::thread::Result<Graded>, run: i64, step: &str) -> Graded {
    thread.unwrap_or_else(|_| {
        let score = Score {
            run,
            step: step.to_owned(),
            scorer: "gate".to_owned(),
            kind: ScoreKind::Outcome,
            verdict: Verdict::Error,
            reason: "the gate's grading crashed".to_owned(),
            cost: 0.0,
            judged: None,
            intent: None,
        };
        vec![(score, true)]
    })
}

/// What one grading reads, owned, to run on a thread of its own.
struct Job {
    dir: PathBuf,
    task: score::Task,
    run: Run,
    gauges: Vec<Gauge>,
    /// The scorers the gate names that the factory no longer has: each refuses.
    lost: Vec<String>,
    /// The lessons the pre-review reads, and its name: only it reads them.
    lessons: Vec<String>,
    pre_review: Option<String>,
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
        intent: None,
        noop: Vec::new(),
        asked: false,
        wrote: false,
    });
    Ok(Job {
        dir: factory.dir.clone(),
        task,
        run: Run {
            id: result["run"].as_i64().unwrap_or_default(),
            step: result["step"].as_str().unwrap_or_default().to_owned(),
            result: Some(result),
            asks: Some(Vec::new()),
            lessons: Vec::new(),
            latest: true,
        },
        gauges,
        lost,
        lessons: Vec::new(),
        pre_review: None,
    })
}

/// Grades the held result with each scorer of `job`, in order. A judge whose calibration does not
/// pass still answers, with why, and does not count.
fn grade(home: &Path, job: Job) -> Graded {
    let reviewed = Run {
        id: job.run.id,
        step: job.run.step.clone(),
        result: job.run.result.clone(),
        asks: Some(Vec::new()),
        lessons: job.lessons.clone(),
        latest: job.run.latest,
    };
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
            let run = match job.pre_review.as_deref() == Some(gauge.name.as_str()) {
                true => &reviewed,
                false => &job.run,
            };
            let score = score::measure(&job.dir, home, &job.task, gauge, run, &mut judging);
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
            intent: None,
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
    through: bool,
    head: &dyn Fn(&Path, &str) -> Option<String>,
) -> rusqlite::Result<Option<String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    #[allow(clippy::type_complexity)]
    let (run, task, step, outcome, arguments, state, stopped, lessons): (
        i64,
        i64,
        String,
        String,
        String,
        String,
        bool,
        Option<String>,
    ) = tx.query_row(
        "SELECT run, task, step, outcome, arguments, state, stopped, lessons FROM gates
         WHERE id = ?1",
        [id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
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
    // A person reads the approval ask after a pre-review anyway, so an uncalibrated one asks nothing.
    // Only a gate that holds lessons is a pre-review: the same judge in another gate still holds.
    let reviewer = lessons.as_ref().and(trace.factory.pre_review());
    let uncalibrated: Vec<String> = graded
        .iter()
        .filter(|(score, counts)| !counts && reviewer != Some(score.scorer.as_str()))
        .map(|(score, _)| {
            let why = score.reason.strip_prefix("not calibrated: ");
            format!(
                "{} is not calibrated: {}",
                score.scorer,
                why.unwrap_or(&score.reason)
            )
        })
        .collect();
    if refusals.is_empty() && !uncalibrated.is_empty() && !through {
        let mut judges: Vec<&str> = graded
            .iter()
            .filter(|(score, counts)| !counts && reviewer != Some(score.scorer.as_str()))
            .map(|(score, _)| score.scorer.as_str())
            .collect();
        judges.sort_unstable();
        judges.dedup();
        ask(&tx, id, task, &judges, &uncalibrated)?;
        let kill = park(&tx, id)?;
        tx.commit()?;
        return Ok(kill);
    }
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
    let lessons: Option<Vec<String>> = lessons.and_then(|text| serde_json::from_str(&text).ok());
    // A refusal from the pre-review alone sends the result back once, and never blocks the task.
    let mut refusers = graded.iter().filter(|(score, counts)| {
        *counts && matches!(score.verdict, Verdict::Fail | Verdict::Error)
    });
    let reviewed = refusers.next().is_some_and(|(first, _)| {
        reviewer == Some(first.scorer.as_str())
            && refusers.all(|(score, _)| reviewer == Some(score.scorer.as_str()))
    });
    if let (true, Some(lessons)) = (reviewed, &lessons) {
        let workflow: String =
            tx.query_row("SELECT workflow FROM tasks WHERE id = ?1", [task], |row| {
                row.get(0)
            })?;
        let target = trace
            .factory
            .workflows
            .get(&workflow)
            .and_then(|flow| flow.value.step(&step)?.next.get(&outcome).cloned())
            .unwrap_or_default();
        let kept: String = lessons
            .iter()
            .map(|lesson| format!("\n- {lesson}"))
            .collect();
        end(&tx, id, "refused", Some(&reason))?;
        wake(
            &tx,
            task,
            &format!(
                "Before step {target} asks a person, the pre-review found outcome {outcome} repeats a lesson. {reason}. The lessons kept for that step:{kept}\nFix it, then send the result again: it then goes to the person."
            ),
        )?;
        tx.commit()?;
        return Ok(None);
    }
    let earlier: i64 = tx.query_row(
        "SELECT count(*) FROM gates WHERE run = ?1 AND state = 'refused'",
        [run],
        |row| row.get(0),
    )?;
    end(&tx, id, "refused", Some(&reason))?;
    if earlier == 0 {
        wake(
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

/// The options of the ask a gate raises when a judge it names is not calibrated.
pub(crate) const GRADE_AGAIN: &str = "grade again";
pub(crate) const LET_THROUGH: &str = "let it through";

/// Where a person's ask on gate `id` stands.
enum Asked {
    /// No ask, so the gate grades.
    None,
    /// An ask waits for a person, so the gate neither grades nor stores.
    Open,
    /// The latest ask was answered `grade again`.
    Again,
    /// The latest ask was answered `let it through`: the result is stored this once.
    Through,
}

fn asked(db: &Connection, id: i64) -> rusqlite::Result<Asked> {
    let latest: Option<(Option<String>, bool)> = db
        .query_row(
            "SELECT asks.answer, asks.withdrawn IS NOT NULL FROM gates
             JOIN asks ON asks.id = gates.ask WHERE gates.id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match latest {
        None => Asked::None,
        Some((None, false)) => Asked::Open,
        Some((Some(answer), _)) if answer == LET_THROUGH => Asked::Through,
        Some(_) => Asked::Again,
    })
}

/// Puts gate `id`, whose only objection is a judge that is not calibrated, on the open ask of the
/// gates that wait on the same `judges`, or asks a person anew, since such a judge can neither pass
/// nor refuse a plan no person reads. The result stays held, and one answer settles each gate.
fn ask(
    tx: &Transaction,
    id: i64,
    task: i64,
    judges: &[&str],
    uncalibrated: &[String],
) -> rusqlite::Result<()> {
    let judges = serde_json::to_string(judges).expect("names serialize");
    let open: Option<i64> = tx
        .query_row(
            "SELECT asks.id FROM gates JOIN asks ON asks.id = gates.ask
             WHERE gates.judges = ?1 AND gates.state = 'grading' AND gates.id != ?2
             AND asks.answer IS NULL AND asks.withdrawn IS NULL
             ORDER BY asks.id DESC LIMIT 1",
            params![judges, id],
            |row| row.get(0),
        )
        .optional()?;
    let ask = match open {
        Some(ask) => ask,
        None => {
            let step: String =
                tx.query_row("SELECT step FROM gates WHERE id = ?1", [id], |row| {
                    row.get(0)
                })?;
            let asked: i64 = tx.query_row(
                "SELECT count(*) FROM asks WHERE name LIKE 'gate-' || ?1 || '-%'",
                [id],
                |row| row.get(0),
            )?;
            let question = format!(
                "Gates hold results while {}. Calibrate it with priory score --calibrate, then answer {GRADE_AGAIN} to grade each result that waits on this ask again, or answer {LET_THROUGH} to store each of them this once.",
                uncalibrated.join("; ")
            );
            let title = format!("gate held: {}", uncalibrated.join("; "));
            tx.execute(
                "INSERT INTO asks (task, step, kind, name, question, options, title)
                 VALUES (?1, ?2, 'agent', ?3, ?4, ?5, ?6)",
                params![
                    task,
                    step,
                    format!("gate-{id}-{}", asked + 1),
                    question,
                    serde_json::json!([GRADE_AGAIN, LET_THROUGH]).to_string(),
                    title
                ],
            )?;
            tx.last_insert_rowid()
        }
    };
    tx.execute(
        "UPDATE gates SET ask = ?2, judges = ?3 WHERE id = ?1",
        params![id, ask, judges],
    )?;
    waiting(tx, ask)
}

/// Lists, on ask `ask`, the tasks whose held results wait on it.
fn waiting(tx: &Transaction, ask: i64) -> rusqlite::Result<()> {
    let tasks: Vec<String> = {
        let mut statement = tx.prepare(
            "SELECT DISTINCT task FROM gates WHERE ask = ?1 AND state = 'grading' ORDER BY task",
        )?;
        statement
            .query_map([ask], |row| row.get::<_, i64>(0))?
            .map(|task| task.map(|task| task.to_string()))
            .collect::<rusqlite::Result<_>>()?
    };
    if tasks.is_empty() {
        return Ok(());
    }
    let label = if tasks.len() == 1 { "task" } else { "tasks" };
    tx.execute(
        "UPDATE asks SET what_waits = ?2 WHERE id = ?1",
        params![
            ask,
            format!("the held results of {label} {}", tasks.join(", "))
        ],
    )
    .map(drop)
}

/// Queues a refusal as the task's next prompt, and wakes the task when its run slept on an ask.
fn wake(tx: &Transaction, task: i64, text: &str) -> rusqlite::Result<()> {
    crate::tools::refused(tx, task, text)?;
    crate::tools::set_ready(tx, task)
}

/// Records a person's answer to `ask`, when it is a gate's: the gate acts on it at its next tick.
/// None for another ask.
pub(crate) fn answered(
    tx: &Transaction,
    ask: i64,
    answer: &str,
    by: &str,
) -> rusqlite::Result<Option<Result<String, String>>> {
    let name: Option<String> = tx
        .query_row(
            "SELECT name FROM asks WHERE id = ?1 AND name LIKE 'gate-%' AND kind = 'agent'",
            [ask],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(name) = name else {
        return Ok(None);
    };
    if answer != GRADE_AGAIN && answer != LET_THROUGH {
        return Ok(Some(Err(format!(
            "ask {name} takes one of: {GRADE_AGAIN}, {LET_THROUGH}"
        ))));
    }
    tx.execute(
        "UPDATE asks SET answer = ?2, answered = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
           answered_by = ?3 WHERE id = ?1",
        params![ask, answer, by],
    )?;
    Ok(Some(Ok(format!("ask {name} answered"))))
}

fn end(tx: &Transaction, id: i64, state: &str, reason: Option<&str>) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE gates SET state = ?2, reason = ?3, ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         WHERE id = ?1",
        params![id, state, reason],
    )
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grading_thread_that_panicked_refuses_the_result() {
        let thread = std::thread::spawn(|| -> Graded { panic!("a scorer panicked") });

        let graded = joined(thread.join(), 4, "implement");

        assert_eq!(graded.len(), 1);
        let (score, counts) = &graded[0];
        assert!(counts);
        assert!(matches!(score.verdict, Verdict::Error));
        assert_eq!(score.reason, "the gate's grading crashed");
        assert_eq!((score.run, score.step.as_str()), (4, "implement"));
    }
}
