//! Cloud runs: a step whose agent has `harness = "claude-cloud"` runs in a Claude cloud session.
//! The session reports by pushing a branch that Priory names, which the daemon polls on `origin`.

use crate::trace::Trace;
use priory_factory::factory::{Agent, Factory, Harness, Step};
use priory_runtime::cloud::{NotStarted, NotTaken, Pushed, Start, Started};
use priory_runtime::{cloud, runtime};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

type Error = Box<dyn std::error::Error>;

/// The harness name a cloud run records.
pub(crate) const HARNESS: &str = "claude-cloud";
/// How long a cloud step waits for its branch when it sets no `timeout`.
const DEADLINE: Duration = Duration::from_secs(2 * 60 * 60);
/// How often the daemon asks `origin` for a live cloud run's branch.
const POLL: Duration = Duration::from_secs(60);
/// How soon it asks again: for a push it saw once, and past the deadline after an error.
const SOON: Duration = Duration::from_secs(15);
/// How long a task waits after its first failing start; each further one in a row doubles it.
const BACKOFF: i64 = 60;
/// The longest such wait.
const BACKOFF_MAX: i64 = 30 * 60;

/// The run whose session a thread starts. One at a time: a repository has one folder for them.
static STARTING: Mutex<Option<(i64, Starting)>> = Mutex::new(None);

type Starting = Receiver<Result<Started, NotStarted>>;

/// When each task's checks before a cloud start last ran out of time.
static SLOW: Mutex<BTreeMap<i64, Instant>> = Mutex::new(BTreeMap::new());

/// How many polls' threads run.
static POLLS: AtomicUsize = AtomicUsize::new(0);

/// What the daemon knows of each live cloud run past what the state holds.
static WATCHED: Mutex<BTreeMap<i64, Watch>> = Mutex::new(BTreeMap::new());

// shortcut: polls live in memory, so a restart polls every live cloud run at once; fine at a
// handful of runs, move to the state file if a factory runs many.
#[derive(Default)]
struct Watch {
    polled: Option<Instant>,
    polling: Option<Receiver<Result<Found, NotTaken>>>,
    /// When its first poll past the deadline started.
    late: Option<Instant>,
    /// The last poll, past the deadline, found no push.
    absent: bool,
    /// The commit the last poll that found a push saw, taken once a second poll sees it too.
    seen: Option<String>,
    /// Why the last poll took no push.
    trouble: Option<String>,
    /// What the CLI said when it uploaded the clone.
    upload: Option<String>,
}

/// What a poll found on `origin`.
enum Found {
    /// No push, or the commit the session started at.
    Nothing,
    /// A push that no earlier poll saw.
    Seen(String),
    /// A push that the poll took.
    Taken(Pushed),
}

/// Whether `agent` starts its steps in a cloud session. A run says itself whether it is one.
pub(crate) fn is_cloud(agent: &Agent) -> bool {
    matches!(agent.harness, Harness::ClaudeCloud)
}

/// Whether run `run` is a cloud run, whatever its agent's harness is now.
pub(crate) fn is_cloud_run(tx: &Transaction, run: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT harness IS ?2 FROM runs WHERE id = ?1",
        params![run, HARNESS],
        |row| row.get(0),
    )
}

/// What cancelling `task` leaves in the cloud, if it has a live cloud run.
pub(crate) fn left(tx: &Transaction, task: i64) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT thread FROM runs WHERE task = ?1 AND ended IS NULL AND harness = ?2",
        params![task, HARNESS],
        |row| {
            Ok(match row.get::<_, Option<String>>(0)? {
                Some(cloud) => format!("its cloud session still runs: {}", link(&cloud)),
                None => "its cloud session may still start: look for it at https://claude.ai/code"
                    .to_owned(),
            })
        },
    )
    .optional()
}

fn link(cloud: &str) -> String {
    format!("https://claude.ai/code/{cloud}")
}

/// A cloud run whose start made no session: its step has a result and it has no session id.
const REFUSED: &str = "harness = 'claude-cloud' AND thread IS NULL AND ended IS NOT NULL
  AND EXISTS (SELECT 1 FROM results WHERE results.run = runs.id)";

/// Why `task` may start no further cloud session, once it started as many as a task may.
pub(crate) fn spent(
    db: &Connection,
    factory: &Factory,
    task: i64,
) -> rusqlite::Result<Option<String>> {
    let started: u32 = db.query_row(
        &format!(
            "SELECT count(*) FROM runs WHERE task = ?1 AND harness = ?2 AND NOT ({REFUSED})
             AND ended_as IS NOT 'denied'"
        ),
        params![task, HARNESS],
        |row| row.get(0),
    )?;
    let max = factory.cloud_sessions();
    Ok((started >= max).then(|| {
        format!(
            "the task started {started} of at most {max} cloud sessions: raise `[cloud] sessions` in factory.toml, then resume the task"
        )
    }))
}

/// Holds back `task`'s next try to start a cloud run: git ran out of time on its worktree.
pub(crate) fn slow(task: i64) {
    let mut slow = SLOW.lock().expect("no insert panics");
    slow.retain(|_, at| at.elapsed() < SOON);
    slow.insert(task, Instant::now());
}

/// Why a task cannot start a cloud run now.
pub(crate) fn waits(
    db: &Connection,
    factory: &Factory,
    task: i64,
) -> rusqlite::Result<Option<String>> {
    let (failed, since): (u32, Option<i64>) = db.query_row(
        &format!(
            "SELECT count(*), unixepoch('now') - unixepoch(max(ended)) FROM runs
             WHERE task = ?1 AND {REFUSED} AND id > coalesce(
               (SELECT max(id) FROM runs WHERE task = ?1 AND thread IS NOT NULL AND harness = '{HARNESS}'), 0)"
        ),
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let wait = match failed {
        0 => 0,
        _ => (BACKOFF << (failed - 1).min(10)).min(BACKOFF_MAX),
    };
    if let Some(left) = since.map(|since| wait - since).filter(|left| *left > 0) {
        return Ok(Some(format!(
            "waits to start a cloud session again: {failed} starts in a row failed, so the next one is in {left}s"
        )));
    }
    let slow = SLOW.lock().expect("no insert panics").get(&task).copied();
    if slow.is_some_and(|at| at.elapsed() < SOON) {
        return Ok(Some(
            "waits to check its worktree again: git was too slow".to_owned(),
        ));
    }
    let live: u32 = db.query_row(
        "SELECT count(*) FROM runs WHERE ended IS NULL AND harness = ?1",
        [HARNESS],
        |row| row.get(0),
    )?;
    let max = factory.cloud_max();
    if live >= max {
        return Ok(Some(format!(
            "waits for a cloud run to end: {live} of at most {max} run"
        )));
    }
    let starting = STARTING
        .lock()
        .expect("no start panics while it holds the lock");
    Ok(starting
        .is_some()
        .then(|| "waits for another cloud session to start".to_owned()))
}

/// The tmux session that runs `claude --cloud` for `task`, for the few seconds it takes.
pub(crate) fn tmux(task: i64) -> String {
    format!("priory-cloud-{task}")
}

/// The branch the cloud session of `task`'s run with harness session `session` pushes to. The
/// `claude/` prefix is the one cloud sessions use for their own branches.
pub(crate) fn branch(task: i64, session: &str) -> String {
    format!("claude/priory-{task}-{}", &session[..session.len().min(8)])
}

/// The rules that replace the `result` tool's line, since a cloud session cannot reach Priory.
fn rules(branch: &str) -> String {
    format!(
        "When you are done, commit your work and push it to the branch `{branch}`: `git push origin HEAD:{branch}`. \
         Start the last commit message with one line that says what changed. \
         Do not open a pull request: Priory opens it after a person reviews the branch. \
         Nobody answers questions during this session. If you need a decision, write it in the last \
         commit message under `Questions:` and push anyway."
    )
}

/// `text` without the first whole lines that are `part`: a person's later words may quote it.
fn without(text: &str, part: &str) -> String {
    let found = text.match_indices(part).find(|(at, _)| {
        let after = &text[at + part.len()..];
        (*at == 0 || text[..*at].ends_with('\n')) && (after.is_empty() || after.starts_with('\n'))
    });
    match found {
        Some((at, _)) if !part.is_empty() => {
            let after = &text[at + part.len()..];
            let after = after.strip_prefix('\n').unwrap_or(after);
            format!("{}{after}", &text[..at]).trim().to_owned()
        }
        _ => text.trim().to_owned(),
    }
}

/// `text` without each line that tells how to end `step`, which every queued prompt may carry.
fn unended(text: &str, step: &Step) -> String {
    let ending = crate::tools::ending(step);
    let kept: Vec<_> = text.lines().filter(|line| *line != ending).collect();
    kept.join("\n").trim().to_owned()
}

/// The step's entry prompt, without how to end a step with the `result` tool.
fn entry(tx: &Transaction, task: i64, step: &Step) -> rusqlite::Result<String> {
    let prompt = crate::tools::entry_prompt(tx, task, step)?;
    Ok(unended(&prompt, step))
}

/// Starts the cloud session of the recorded run `run` on a thread, with the agent's prompt, which
/// `start` holds, the step's, the task's event, what is queued for the task and the push rules.
/// `screen` says first, on that thread, why nothing may be pushed.
pub(crate) fn dispatch(
    db: &mut Connection,
    step: &Step,
    run: i64,
    task: i64,
    session: &str,
    mut start: Start,
    screen: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), Error> {
    let mut queued = Vec::new();
    while let Some(text) = crate::state::claim(db, run)? {
        queued.push(text);
    }
    let tx = db.transaction()?;
    tx.execute(
        "UPDATE runs SET cloud_head = ?2 WHERE id = ?1",
        params![run, start.head],
    )?;
    let entry = entry(&tx, task, step)?;
    let source = crate::state::source(&tx, task)?;
    tx.commit()?;
    let queued = unended(&queued.join("\n"), step);
    // A cloud session cannot send the classes the rule asks for.
    let queued: Vec<_> = queued
        .lines()
        .filter(|line| *line != crate::triage::RULE)
        .collect();
    let queued = queued.join("\n");
    let queued = without(&without(&queued, &entry), source.trim());
    start.prompt = [
        start.prompt.trim(),
        &entry,
        source.trim(),
        &queued,
        &rules(&branch(task, session)),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n\n");
    let (sender, receiver) = channel();
    *STARTING.lock().expect("no start panics") = Some((run, receiver));
    std::thread::spawn(move || {
        let started = screen()
            .map_err(NotStarted::Denied)
            .and_then(|()| cloud::start(&start));
        let _ = sender.send(started);
    });
    Ok(())
}

/// Stores what the start under way gave once it ended.
fn collect(db: &mut Connection, trace: &Trace, kit: crate::kit::Current<'_>) -> Result<(), Error> {
    let (run, started) = {
        let mut starting = STARTING.lock().expect("no start panics");
        let Some((run, receiver)) = starting.as_ref() else {
            return Ok(());
        };
        let started = match receiver.try_recv() {
            Ok(started) => started,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                Err(NotStarted::Uncertain("its thread stopped".into()))
            }
        };
        let run = *run;
        *starting = None;
        (run, started)
    };
    let live = db
        .query_row(
            "SELECT runs.task, runs.session, tasks.workflow, runs.step FROM runs
             JOIN tasks ON tasks.id = runs.task WHERE runs.id = ?1 AND runs.ended IS NULL",
            [run],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((task, session, workflow, step)) = live else {
        let left = match started {
            Ok(started) => format!("its cloud session still runs: {}", link(&started.id)),
            Err(NotStarted::Uncertain(reason)) => format!(
                "it may have started a cloud session: {reason}. Look for it at https://claude.ai/code"
            ),
            Err(NotStarted::Refused(_) | NotStarted::Denied(_)) => return Ok(()),
        };
        eprintln!("priory daemon: cloud run {run} ended during its start: {left}");
        db.execute(
            "UPDATE tasks SET reason = coalesce(reason || '; ', '') || ?2
             WHERE id = (SELECT task FROM runs WHERE id = ?1) AND state = 'dropped'",
            params![run, left],
        )?;
        return Ok(());
    };
    match started {
        Ok(started) => {
            db.execute(
                "UPDATE runs SET thread = ?2 WHERE id = ?1",
                params![run, started.id],
            )?;
            db.execute(
                "UPDATE outbox SET state = 'confirmed', confirmed = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE run = ?1 AND state = 'attempted'",
                [run],
            )?;
            if let Some(upload) = started.upload {
                eprintln!(
                    "priory daemon: cloud run {run} started from an uploaded copy of the task's branch: {upload}"
                );
                let mut watched = WATCHED.lock().expect("no poll panics");
                watched.entry(run).or_default().upload = Some(upload);
            }
        }
        Err(NotStarted::Uncertain(reason)) => {
            let reason = format!(
                "cloud run {run} may have started a cloud session: {reason}. Look for it at https://claude.ai/code before you resume the task"
            );
            crate::state::block(db, trace, task, "uncertain", &reason)?;
        }
        Err(NotStarted::Denied(reason)) => {
            give_back(db, run, task, step_of(trace, &workflow, &step))?;
            crate::state::block(db, trace, task, "denied", &reason)?;
        }
        Err(NotStarted::Refused(reason)) => {
            let step = step_of(trace, &workflow, &step);
            let summary = format!("the cloud session did not start: {reason}");
            fail(db, trace, kit, run, task, &session, step, &summary)?;
        }
    }
    Ok(())
}

fn step_of<'f>(trace: &Trace<'f>, workflow: &str, step: &str) -> Option<&'f Step> {
    let workflow = trace.factory.workflows.get(workflow)?;
    workflow.value.step(step)
}

/// Ends run `run` as `failed`, with what it claimed queued again for the task's next step.
#[allow(clippy::too_many_arguments)]
fn fail(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
    run: i64,
    task: i64,
    session: &str,
    step: Option<&Step>,
    summary: &str,
) -> Result<(), Error> {
    give_back(db, run, task, step)?;
    end(db, trace, kit, task, session, "failed", summary, None)
}

/// Queues again what run `run` claimed, without the step's own prompt. The task's event goes back
/// to unread, so the next start adds it.
fn give_back(
    db: &mut Connection,
    run: i64,
    task: i64,
    step: Option<&Step>,
) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    let entry = match step {
        Some(step) => entry(&tx, task, step)?,
        None => String::new(),
    };
    let source = crate::state::source(&tx, task)?;
    tx.execute(
        "UPDATE inbox SET outbox = NULL WHERE id = (SELECT source FROM tasks WHERE id = ?1)
         AND outbox IN (SELECT id FROM outbox WHERE run = ?2)",
        params![task, run],
    )?;
    let claimed: Vec<(i64, String)> = tx
        .prepare(
            "SELECT id, text FROM outbox WHERE run = ?1
             AND state IN ('attempted', 'uncertain', 'confirmed') ORDER BY id",
        )?
        .query_map([run], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, text) in claimed {
        let text = match step {
            Some(step) => unended(&text, step),
            None => text,
        };
        let text = without(&without(&text, &entry), source.trim());
        if text.is_empty() {
            tx.execute("UPDATE inbox SET outbox = NULL WHERE outbox = ?1", [id])?;
            tx.execute(
                "UPDATE subscription_deliveries SET outbox = NULL WHERE outbox = ?1",
                [id],
            )?;
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        } else {
            tx.execute(
                "UPDATE outbox SET state = 'queued', run = NULL, attempted = NULL, confirmed = NULL,
                 text = ?2
                 WHERE id = ?1",
                params![id, text],
            )?;
        }
    }
    tx.commit()
}

/// A live cloud run, as the tick reads it.
struct Live {
    id: i64,
    task: i64,
    session: String,
    cloud: Option<String>,
    worktree: String,
    branch: String,
    head: String,
    workflow: String,
    step: String,
    age: i64,
}

/// Ends each live cloud run whose branch is on `origin` as `pushed`, and each past its step's
/// `timeout` as `failed`. A run with no cloud session id and no start under way may or may not have
/// started one, so its task blocks for a person.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
) -> Result<(), Error> {
    collect(db, trace, kit)?;
    let starting = STARTING
        .lock()
        .expect("no start panics")
        .as_ref()
        .map(|(run, _)| *run);
    let live: Vec<Live> = db
        .prepare(
            "SELECT runs.id, runs.task, runs.session, runs.thread, coalesce(tasks.worktree, ''),
             coalesce(tasks.branch, ''), coalesce(runs.cloud_head, ''), tasks.workflow, runs.step,
             unixepoch('now') - unixepoch(runs.started)
             FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.ended IS NULL AND runs.harness = 'claude-cloud' ORDER BY runs.id",
        )?
        .query_map([], |row| {
            Ok(Live {
                id: row.get(0)?,
                task: row.get(1)?,
                session: row.get(2)?,
                cloud: row.get(3)?,
                worktree: row.get(4)?,
                branch: row.get(5)?,
                head: row.get(6)?,
                workflow: row.get(7)?,
                step: row.get(8)?,
                age: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    WATCHED
        .lock()
        .expect("no poll panics")
        .retain(|id, _| live.iter().any(|run| run.id == *id));
    for run in &live {
        if starting == Some(run.id) {
            continue;
        }
        let Some(cloud) = &run.cloud else {
            runtime::kill(&tmux(run.task));
            let reason = format!(
                "cloud run {} may or may not have started a cloud session: the daemon stopped while it \
                 started one. Look for it at https://claude.ai/code before you resume the task",
                run.id
            );
            crate::state::block(db, trace, run.task, "uncertain", &reason)?;
            continue;
        };
        let mut watch = WATCHED
            .lock()
            .expect("no poll panics")
            .remove(&run.id)
            .unwrap_or_default();
        if follow(db, trace, kit, run, cloud, &mut watch)? {
            WATCHED
                .lock()
                .expect("no poll panics")
                .insert(run.id, watch);
        }
    }
    Ok(())
}

/// Takes the poll of `run` that ended, starts the next one when it is due, and ends the run on a
/// push or its deadline. Says whether the run goes on.
fn follow(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
    run: &Live,
    cloud: &str,
    watch: &mut Watch,
) -> Result<bool, Error> {
    let pushes = branch(run.task, &run.session);
    let step = step_of(trace, &run.workflow, &run.step);
    if let Some(polling) = &watch.polling {
        let found = match polling.try_recv() {
            Ok(found) => found,
            Err(TryRecvError::Empty) => return Ok(true),
            Err(TryRecvError::Disconnected) => Err(NotTaken::Later("its thread stopped".into())),
        };
        watch.polling = None;
        watch.absent = false;
        watch.trouble = None;
        match found {
            Ok(Found::Nothing) => {
                watch.seen = None;
                watch.absent = watch.late.is_some();
            }
            // A push may be a first part of the work, or be replaced: the next poll confirms it.
            Ok(Found::Seen(commit)) => watch.seen = Some(commit),
            Ok(Found::Taken(pushed)) => {
                end(
                    db,
                    trace,
                    kit,
                    run.task,
                    &run.session,
                    "pushed",
                    &pushed.subject,
                    Some((&pushed.commit, &pushed.body)),
                )?;
                return Ok(false);
            }
            Err(NotTaken::Never(reason)) => {
                let summary = format!(
                    "cloud session {} pushed {pushes}, and Priory did not take it: {reason}",
                    link(cloud)
                );
                fail(
                    db,
                    trace,
                    kit,
                    run.id,
                    run.task,
                    &run.session,
                    step,
                    &summary,
                )?;
                return Ok(false);
            }
            Err(NotTaken::Later(reason)) => {
                eprintln!("priory daemon: cloud run {}: {reason}", run.id);
                watch.trouble = Some(reason);
            }
        }
    }
    let timeout = step.and_then(|step| step.timeout).unwrap_or(DEADLINE);
    let late = run.age >= 0 && run.age as u64 >= timeout.as_secs();
    // The grace counts from the first late poll, so a runner that slept past the deadline has it whole.
    let graced = watch.late.is_some_and(|since| since.elapsed() < timeout);
    if late && (watch.absent || (watch.trouble.is_some() && watch.late.is_some() && !graced)) {
        let seconds = timeout.as_secs();
        let mut summary = match &watch.trouble {
            None => format!("cloud session {cloud} pushed no branch {pushes} within {seconds}s."),
            Some(trouble) => format!(
                "Priory took no push of cloud session {cloud} to {pushes} within {seconds}s: {trouble}."
            ),
        };
        if let Some(upload) = &watch.upload {
            summary.push_str(&format!(
                " It started from an upload, so the Claude GitHub App may not be on the repository: {upload}."
            ));
        }
        summary.push_str(&format!(
            " A cloud run cannot ask, so open the session to see why: {}",
            link(cloud)
        ));
        fail(
            db,
            trace,
            kit,
            run.id,
            run.task,
            &run.session,
            step,
            &summary,
        )?;
        return Ok(false);
    }
    let confirms = watch.seen.is_some() && watch.trouble.is_none();
    let every = if late || confirms { SOON } else { POLL };
    let first_late = late && watch.late.is_none();
    if first_late || watch.polled.is_none_or(|polled| polled.elapsed() >= every) {
        if late {
            watch.late.get_or_insert_with(Instant::now);
        }
        watch.polled = Some(Instant::now());
        let (sender, receiver) = channel();
        watch.polling = Some(receiver);
        let worktree = Path::new(&run.worktree).to_path_buf();
        let (branch, head) = (run.branch.clone(), run.head.clone());
        let (seen, id) = (watch.seen.clone(), run.id);
        let poll = Poll::begin();
        std::thread::spawn(move || {
            let _poll = poll;
            let found = look(&worktree, &pushes, &branch, &head, seen, || still_live(id));
            let _ = sender.send(found);
        });
    }
    Ok(true)
}

/// Whether run `run` has not ended, as another thread reads it.
fn still_live(run: i64) -> Result<bool, String> {
    crate::state::read_only()?
        .query_row(
            "SELECT ended IS NULL FROM runs WHERE id = ?1",
            [run],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// Whether a poll works on a worktree, which an upgrade must not cut short.
pub(crate) fn polling() -> bool {
    POLLS.load(Ordering::SeqCst) > 0
}

/// Counts a poll's thread until it exits, which may be after its run ended.
struct Poll;

impl Poll {
    fn begin() -> Self {
        POLLS.fetch_add(1, Ordering::SeqCst);
        Poll
    }
}

impl Drop for Poll {
    fn drop(&mut self) {
        POLLS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Polls `pushes`, and takes the commit a poll before found there as `seen`, while `live` says so.
fn look(
    worktree: &Path,
    pushes: &str,
    branch: &str,
    head: &str,
    seen: Option<String>,
    live: impl Fn() -> Result<bool, String>,
) -> Result<Found, NotTaken> {
    let Some(commit) = cloud::poll(worktree, pushes).map_err(NotTaken::Later)? else {
        return Ok(Found::Nothing);
    };
    if commit == head {
        return Ok(Found::Nothing);
    }
    if seen.as_ref() != Some(&commit) {
        return Ok(Found::Seen(commit));
    }
    let taken = cloud::adopt(worktree, branch, head, &commit, live)?;
    Ok(taken.map_or(Found::Nothing, Found::Taken))
}

/// Stores the result the session's harness session `session` would have sent, then ends its turn as
/// a `Stop` does, so the task moves by `next` like a local run's. A step with no such outcome
/// blocks the task with the summary.
#[allow(clippy::too_many_arguments)]
fn end(
    db: &mut Connection,
    trace: &Trace,
    kit: crate::kit::Current<'_>,
    task: i64,
    session: &str,
    outcome: &str,
    summary: &str,
    pushed: Option<(&str, &str)>,
) -> Result<(), Error> {
    let mut arguments = json!({ "outcome": outcome, "summary": summary });
    if let Some((commit, body)) = pushed {
        arguments["commit"] = json!(commit);
        if !body.is_empty() {
            arguments["evidence"] = json!(body);
        }
    }
    let call = crate::tools::Call {
        tool: "result".into(),
        session: session.into(),
        arguments,
    };
    let skipped = match outcome {
        "pushed" => {
            let tx = db.transaction()?;
            crate::tools::graders(&tx, session, outcome, trace.factory, kit)?
        }
        _ => Vec::new(),
    };
    let (stored, _) = crate::tools::call(db, &call, trace, kit, &|worktree, branch| {
        runtime::head(worktree, branch)
    })?;
    if let Err(refused) = stored {
        let reason = format!("{summary} ({refused})");
        crate::state::block(db, trace, task, outcome, &reason)?;
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute(
        "UPDATE runs SET idle = 1, turn = NULL WHERE session = ?1 AND ended IS NULL",
        [session],
    )?;
    crate::tools::stop(&tx, session, trace)?;
    if !skipped.is_empty() {
        let line = format!(
            "A cloud session cannot fix what a gate refuses, so no pre-review or gate read this push: {}.",
            skipped.join(", ")
        );
        tx.execute(
            "UPDATE asks SET text = ltrim(coalesce(text, '') || char(10, 10) || ?2, char(10))
             WHERE id = (SELECT max(id) FROM asks WHERE task = ?1 AND kind = 'step'
               AND answer IS NULL AND withdrawn IS NULL)",
            params![task, line],
        )?;
    }
    tell(&tx, task)?;
    tx.commit()?;
    Ok(())
}

/// Shows what came during the run on the ask it led to, since no session read it.
fn tell(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    crate::news::expire(tx, task)?;
    let kept: Vec<String> = tx
        .prepare(
            "SELECT text FROM outbox WHERE task = ?1 AND state IN ('queued', 'held') AND compact = 0 ORDER BY id",
        )?
        .query_map([task], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for text in kept {
        let said: Vec<_> = text
            .lines()
            .filter(|line| !line.starts_with("End this step by calling the result tool"))
            .collect();
        crate::tools::tell_ask(tx, task, &said.join("\n"))?;
    }
    Ok(())
}

/// The most a cloud environment's setup script may hold, as its dialog's file picker takes it.
const SETUP_LIMIT: usize = 64 * 1024;

/// `priory cloud setup-script`: a bash script for a cloud environment's setup script. It writes the
/// instructions and skills a run of `agent` gets, the kit's first, under the session user's
/// `~/.claude`, then runs each repository's `cloud_setup` script, copied from Priory's clone.
pub fn setup_script(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    agent: Option<&str>,
) -> Result<String, String> {
    let agent = match agent {
        Some(name) => Some(
            &factory
                .agents
                .get(name)
                .ok_or_else(|| format!("agent {name} does not exist"))?
                .value,
        ),
        None => None,
    };
    let fetchers =
        crate::kit::Fetchers::new(db, factory, home).map_err(|error| error.to_string())?;
    let current = fetchers.current();
    let sources = crate::kit::sources(factory, current, &[], agent);
    let mut setups = Vec::new();
    let repos = factory
        .settings
        .as_ref()
        .map(|settings| &settings.value.repos);
    for (repo, settings) in repos.into_iter().flatten() {
        if let Some(path) = &settings.cloud_setup {
            setups.push((
                format!("{repo}:{path}"),
                runtime::repo_file(home, repo, path)?,
            ));
        }
    }
    let script = script(
        &factory.name,
        sources.commit,
        &sources.user_files(),
        &setups,
    );
    if script.len() > SETUP_LIMIT {
        return Err(format!(
            "the setup script is {} bytes, over the cloud environment's limit of {SETUP_LIMIT}: name the agent whose skills it needs with --agent, or give that agent fewer skills",
            script.len()
        ));
    }
    Ok(script)
}

/// The script that writes `files` under `~/.claude`, then runs each of `setups`, named by where it
/// came from.
fn script(
    factory: &str,
    commit: Option<&str>,
    files: &[(std::path::PathBuf, &[u8])],
    setups: &[(String, Vec<u8>)],
) -> String {
    let kit = commit.map_or_else(String::new, |commit| format!(", kit commit {commit}"));
    let mut script = format!(
        "#!/bin/bash\n# Written by `priory cloud setup-script` for the factory {factory}{kit}.\n\
         # Paste it into a cloud environment's setup script. It holds no secret.\nset -euo pipefail\n"
    );
    for (path, bytes) in files {
        let target = format!("\"$HOME/.claude/{}\"", path.display());
        script.push_str(&format!(
            "mkdir -p \"$(dirname {target})\"\n{}",
            write(&format!("> {target}"), bytes)
        ));
    }
    for (from, bytes) in setups {
        script.push_str(&format!("# {from}\n{}", write("| bash", bytes)));
    }
    script
}

/// A command that feeds `bytes` to `into`: a quoted heredoc for text, base64 for anything else.
fn write(into: &str, bytes: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(bytes) else {
        let encoded = <base64ct::Base64 as base64ct::Encoding>::encode_string(bytes);
        return format!("base64 -d <<'PRIORY_BASE64' {into}\n{encoded}\nPRIORY_BASE64\n");
    };
    let mut end = "PRIORY_END".to_owned();
    while text.lines().any(|line| line == end) {
        end.push('_');
    }
    let newline = if text.ends_with('\n') || text.is_empty() {
        ""
    } else {
        "\n"
    };
    // shortcut: a file whose last line has no newline gains one; fine for Markdown and shell.
    format!("cat <<'{end}' {into}\n{text}{newline}{end}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_poll_counts_until_its_thread_exits_by_a_panic_too() {
        let poll = Poll::begin();
        assert!(polling());
        let thread = std::thread::spawn(move || {
            let _poll = poll;
            panic!("the poll fails");
        });
        assert!(thread.join().is_err());
        assert!(!polling());
    }

    #[test]
    fn a_step_prompt_leaves_the_queue_once_and_a_persons_quote_of_it_stays() {
        let queued = "Fix it.\nA person says: Fix it. Then the docs.\nFix it.";
        assert_eq!(
            without(queued, "Fix it."),
            "A person says: Fix it. Then the docs.\nFix it."
        );
        assert_eq!(without("Fix it. Now.", "Fix it."), "Fix it. Now.");
        assert_eq!(without("Intro\nFix it.", "Fix it."), "Intro");
        assert_eq!(without("Fix it.", ""), "Fix it.");
    }

    #[test]
    fn the_setup_script_writes_files_through_heredocs_that_their_text_cannot_end() {
        let files = [
            (PathBuf::from("CLAUDE.md"), &b"Be brief.\nPRIORY_END\n"[..]),
            (PathBuf::from("skills/a/icon.bin"), &[0xff, 0x00][..]),
        ];
        let setups = [(
            "owner/repo:scripts/cloud-setup.sh".to_owned(),
            b"apt-get install -y jq".to_vec(),
        )];
        let script = script("demo", Some("abc"), &files, &setups);
        let ran = std::process::Command::new("bash")
            .arg("-c")
            .arg(script.replace("| bash", "> \"$HOME/setup.sh\""))
            .env(
                "HOME",
                std::env::temp_dir().join(format!("cloud-setup-{}", std::process::id())),
            )
            .output()
            .unwrap();
        assert!(ran.status.success(), "{ran:?}");
        let home = std::env::temp_dir().join(format!("cloud-setup-{}", std::process::id()));
        assert_eq!(
            std::fs::read(home.join(".claude/CLAUDE.md")).unwrap(),
            b"Be brief.\nPRIORY_END\n"
        );
        assert_eq!(
            std::fs::read(home.join(".claude/skills/a/icon.bin")).unwrap(),
            [0xff, 0x00]
        );
        assert_eq!(
            std::fs::read(home.join("setup.sh")).unwrap(),
            b"apt-get install -y jq\n"
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}
