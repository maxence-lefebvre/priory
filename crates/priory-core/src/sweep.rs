use crate::recipe::{self, Teardown};
use crate::trace::{self, Trace};
use priory_factory::factory::Factory;
use priory_runtime::runtime::timed_out;
use priory_runtime::slice;
use priory_runtime::worktrees::{self, Found, RECLAIM, Verdict, reclaim_folder};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A task folder under `PRIORY_HOME/worktrees` and why it stays, or `None` when it can go.
/// `retire` keeps the branch when the folder goes. `judged` is the reason the checks gave, which the
/// sweep records for the desk.
struct Entry {
    found: Found,
    stays: Option<String>,
    retire: bool,
    judged: Option<String>,
    /// The task's state and the worktree's HEAD when the sweep judged it.
    state: String,
    tip: Option<String>,
    /// The walk of its files allows the removal.
    walked: bool,
}

/// How `assess` learns why an ended task's worktree stays.
enum Judge<'a> {
    /// It reads what the sweep's thread found.
    Answers(&'a Answers),
    /// It reads what the last sweep recorded, as the desk does.
    Recorded(&'a Record),
}

/// The worktree of an ended task, for the sweep's thread.
struct Job {
    found: Found,
    /// The head commit of the task's merged pull request.
    merged: Option<String>,
    done: bool,
    /// No teardown is left to run, so the walk of its files is the last one before the removal.
    torn: bool,
}

/// What the thread found of one worktree. `walked` is `None` when nothing called for the walk.
struct Answer {
    verdict: Verdict,
    walked: Option<Result<(), String>>,
}

type Answers = BTreeMap<PathBuf, Answer>;

/// What the sweep's thread found: the repositories it did not fetch, and each worktree it judged.
struct Judged {
    unfetched: Vec<String>,
    answers: Answers,
    /// The worktree that went whose branch's delete ran past its limit again.
    cut: Option<Found>,
}

/// The task a worktree folder is named after.
fn task_of(found: &Found) -> Option<i64> {
    worktrees::task_of(&found.name)
}

/// The patch scans one sweep runs, each a few seconds.
const SCANS: usize = 2;

/// When each clone was last fetched and each worktree last checked, so a sweep takes the oldest
/// first and none starves.
static TRIED: Mutex<BTreeMap<PathBuf, Instant>> = Mutex::new(BTreeMap::new());

fn tried() -> std::sync::MutexGuard<'static, BTreeMap<PathBuf, Instant>> {
    TRIED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Why the folder stays whatever git says of it, or the ended state of its task.
fn ended(db: &Connection, found: &Found) -> rusqlite::Result<Result<String, String>> {
    let task = match task_of(found) {
        Some(id) => db
            .query_row(
                "SELECT state, worktree FROM tasks WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?,
        None => None,
    };
    Ok(match task {
        None => Err(format!("no task {}", found.name)),
        Some((_, tree)) if tree.as_deref() != found.path.to_str() => Err(format!(
            "not recorded as the worktree of task {}",
            found.name
        )),
        Some((state, _))
            if matches!(
                state.as_str(),
                "queued" | "held" | "ready" | "running" | "asleep" | "blocked"
            ) =>
        {
            Err(format!("kept while task {} is {state}", found.name))
        }
        Some((state, _)) => Ok(state),
    })
}

fn assess(db: &Connection, home: &Path, judge: Judge) -> rusqlite::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for found in worktrees::found(home) {
        let mut entry = Entry {
            found,
            stays: None,
            retire: false,
            judged: None,
            state: String::new(),
            tip: None,
            walked: false,
        };
        match ended(db, &entry.found)? {
            Err(stays) => entry.stays = Some(stays),
            Ok(state) => {
                entry.state = state;
                entry.judged = match judge {
                    Judge::Answers(answers) => match answers.get(&entry.found.path) {
                        None => Some("its checks wait for the next sweep".to_owned()),
                        Some(answer) => judged(answer, &mut entry),
                    },
                    Judge::Recorded(record) => record
                        .get(&entry.found.path.display().to_string())
                        .and_then(|kept| kept.reason.clone()),
                };
                entry.stays = match (&entry.judged, task_of(&entry.found)) {
                    (Some(reason), _) => Some(reason.clone()),
                    (None, Some(task)) => match recipe::teardown_of(db, task)? {
                        Some(Teardown::Runs) => Some("teardown runs".to_owned()),
                        Some(Teardown::Failed(reason)) => Some(reason),
                        Some(Teardown::Done) | None => None,
                    },
                    (None, None) => None,
                };
            }
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// Why the thread's `answer` keeps the worktree of `entry`, which takes what else it says.
fn judged(answer: &Answer, entry: &mut Entry) -> Option<String> {
    let tip = match &answer.verdict {
        Verdict::Keep(reason) => return Some(reason.clone()),
        Verdict::Remove { tip } => tip,
        // A task revived and dropped since the check keeps its worktree.
        Verdict::Retire { .. } if entry.state != "done" => {
            return Some("its checks wait for the next sweep".to_owned());
        }
        Verdict::Retire { tip } => {
            entry.retire = true;
            tip
        }
    };
    entry.tip = Some(tip.clone());
    match &answer.walked {
        Some(Err(reason)) => return Some(reason.clone()),
        Some(Ok(())) => entry.walked = true,
        None => {}
    }
    None
}

/// The worktrees the sweep's thread judges. A squash merge leaves the head of the task's merged pull
/// request out of the default branch, so the inbox names it.
fn jobs(db: &Connection, factory: &Factory, all: Vec<Found>) -> rusqlite::Result<Vec<Job>> {
    let mut jobs = Vec::new();
    for found in all {
        let (Ok(state), Some(task)) = (ended(db, &found)?, task_of(&found)) else {
            continue;
        };
        let merged = db
            .query_row(
                "SELECT json_extract(data, '$.head_sha') FROM inbox
                 WHERE type = 'github.pull_request.closed' AND json_extract(data, '$.merged') = 1
                 AND json_extract(data, '$.repo') = ?1 AND json_extract(data, '$.head_branch') = 'priory/' || ?2
                 ORDER BY seq DESC LIMIT 1",
                [&found.repo, &found.name],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        jobs.push(Job {
            merged,
            done: state == "done",
            torn: recipe::torn(db, factory, task, &found.repo)?,
            found,
        });
    }
    Ok(jobs)
}

/// Fetches the clone of each of `repos`, then judges the worktree of each job, the one checked
/// longest ago first, and walks the files of one that may go. It takes as long as the disk and the
/// network do, so call it off the tick.
fn judge(home: &Path, repos: &[String], mut jobs: Vec<Job>) -> Judged {
    let unfetched = fetch(home, repos, jiff::Timestamp::now());
    {
        let tried = tried();
        jobs.sort_by_key(|job| tried.get(&job.found.path).copied());
    }
    let mut answers = Answers::new();
    let mut scans = SCANS;
    for job in jobs {
        let verdict = worktrees::verdict(
            home,
            &job.found,
            job.merged.as_deref(),
            job.done,
            &mut scans,
        );
        // A check that timed out says the disk hangs, so the sweep asks git nothing more.
        let mut hung = matches!(&verdict, Verdict::Keep(why) if timed_out(why));
        if verdict != Verdict::Keep(worktrees::SCAN_WAITS.to_owned()) {
            tried().insert(job.found.path.clone(), Instant::now());
        }
        let goes = job.torn
            && !matches!(verdict, Verdict::Keep(_))
            && !unfetched.contains(&job.found.repo);
        // shortcut: one walk at a time, so a huge worktree delays the others; walk several at once when it shows.
        let walked = goes.then(|| worktrees::removable(home, &job.found));
        hung |= matches!(&walked, Some(Err(why)) if timed_out(why));
        answers.insert(job.found.path, Answer { verdict, walked });
        if hung {
            break;
        }
    }
    Judged {
        unfetched,
        answers,
        cut: None,
    }
}

/// A thread the loop waits for, tick by tick.
struct Off<T> {
    thread: JoinHandle<T>,
    started: Instant,
    late: bool,
    what: String,
}

impl<T: Send + 'static> Off<T> {
    fn new(what: String, work: impl FnOnce() -> T + Send + 'static) -> Off<T> {
        Off {
            thread: std::thread::spawn(work),
            started: Instant::now(),
            late: false,
            what,
        }
    }

    /// Whether it just ran past `OVERDUE`, which is logged.
    fn turns_late(&mut self) -> bool {
        let turns = !self.late && self.started.elapsed() >= OVERDUE;
        if turns {
            self.late = true;
            eprintln!("priory daemon: {} takes over 10 minutes", self.what);
        }
        turns
    }

    /// Its answer, `None` when it panicked.
    fn join(self) -> Option<T> {
        let answer = self.thread.join().ok();
        if answer.is_none() {
            eprintln!("priory daemon: {} panicked", self.what);
        }
        answer
    }
}

/// Retires one asleep task: the walk of its files.
struct Retiring {
    task: i64,
    found: Found,
    /// No teardown was left to run when the walk started.
    torn: bool,
    walk: Off<Result<(), String>>,
}

/// Removes a worktree moved aside.
struct Deleting {
    found: Found,
    /// The asleep task it retires.
    retires: Option<i64>,
    thread: Off<Removed>,
}

/// What became of a worktree moved aside.
struct Removed {
    /// Why it stays.
    stays: Option<String>,
    /// The delete of its branch ran past its limit.
    cut: bool,
}

/// Removes the worktree and branch of every ended task that nothing keeps, after fetching each clone
/// (see `fetch`), and after the repository's teardown succeeded in it. A teardown owed here starts
/// at the daemon's next tick. A thread fetches, judges and walks. The tick
/// moves one worktree aside, and another thread checks it again and deletes it.
#[derive(Default)]
pub(crate) struct Sweeping {
    /// The sweeps started, and the last one that ended or was late.
    started: u64,
    ended: u64,
    judging: Option<Off<Judged>>,
    entries: Vec<Entry>,
    /// What may go, as indexes of `entries`.
    next: VecDeque<usize>,
    open: bool,
    deleting: Option<Deleting>,
    /// The deletes the loop no longer waits for.
    overdue: Vec<Deleting>,
    /// The worktrees that went whose branch's delete ran past its limit.
    branches: VecDeque<Found>,
    /// The asleep tasks that may retire, longest asleep first.
    asleep: VecDeque<(i64, String)>,
    retiring: Option<Retiring>,
    /// Each run that exited on a short disk, the sweep its repair waits for, and the cause.
    repairs: HashMap<i64, (u64, Instant, String)>,
}

impl Sweeping {
    /// Whether a sweep or a retirement is under way.
    fn busy(&self) -> bool {
        self.open || self.deleting.is_some() || self.retiring.is_some() || !self.asleep.is_empty()
    }

    /// Whether the repair of `run` waits for a sweep. `short` says why the disk keeps the run from
    /// starting again, which asks for one.
    pub(crate) fn repairs(
        &mut self,
        run: i64,
        short: impl FnOnce() -> rusqlite::Result<Option<String>>,
    ) -> rusqlite::Result<bool> {
        if let Some((sweep, asked, _)) = self.repairs.get(&run) {
            // A sweep that hangs lets none start after it.
            return Ok(self.ended < *sweep && asked.elapsed() < OVERDUE);
        }
        let cause = short()?;
        let waits = cause.is_some();
        if let Some(cause) = cause {
            self.repairs
                .insert(run, (self.started + 1, Instant::now(), cause));
        }
        Ok(waits)
    }

    /// Forgets the repairs of the runs that are not `live`.
    pub(crate) fn forget_repairs(&mut self, live: impl Fn(i64) -> bool) {
        self.repairs.retain(|run, _| live(*run));
    }

    /// Why `run` exited, when a sweep ran to repair it.
    pub(crate) fn repaired(&mut self, run: i64) -> Option<String> {
        self.repairs.remove(&run).map(|(_, _, cause)| cause)
    }

    /// Whether a repair waits for a sweep that did not start.
    pub(crate) fn asked(&self) -> bool {
        self.repairs
            .values()
            .any(|(sweep, _, _)| *sweep > self.started)
    }

    /// Starts a sweep, unless one or a retirement is under way.
    pub(crate) fn start(
        &mut self,
        db: &Connection,
        factory: &Factory,
        home: &Path,
    ) -> rusqlite::Result<bool> {
        if self.busy() {
            return Ok(false);
        }
        crate::evidence::sweep(db, home)?;
        let all = worktrees::found(home);
        worktrees::forget_scans(home, &all);
        let mut repos: Vec<_> = all.iter().map(|found| found.repo.clone()).collect();
        repos.dedup();
        let jobs = jobs(db, factory, all)?;
        // One per sweep: those behind it would wait for its git, in which their tasks can wake.
        let mut branch = None;
        while branch.is_none()
            && let Some(found) = self.branches.pop_front()
        {
            if ended(db, &found)?.is_ok() {
                branch = worktrees::Branch::of(home, &found);
            }
        }
        let home = home.to_path_buf();
        self.judging = Some(Off::new("the sweep's check of the worktrees".to_owned(), {
            move || {
                let cut = branch.and_then(|branch| branch.delete(&home));
                let judged = judge(&home, &repos, jobs);
                Judged { cut, ..judged }
            }
        }));
        self.started += 1;
        self.open = true;
        Ok(true)
    }

    /// Takes the sweep, or the retirement after it, one step on: at most one worktree goes. Returns
    /// whether the sweep just ended, or is late. `short` says the disk is short, which a retirement
    /// ends with.
    pub(crate) fn tick(
        &mut self,
        db: &Connection,
        factory: &Factory,
        home: &Path,
        short: impl FnOnce() -> bool,
    ) -> rusqlite::Result<bool> {
        let ended = |deleting: &mut Deleting| deleting.thread.thread.is_finished();
        for deleting in self.overdue.extract_if(.., ended).collect::<Vec<_>>() {
            self.removed(db, home, deleting)?;
        }
        if let Some(mut deleting) = self.deleting.take() {
            if ended(&mut deleting) {
                self.removed(db, home, deleting)?;
            } else if deleting.thread.turns_late() {
                self.overdue.push(deleting);
            } else {
                self.deleting = Some(deleting);
                return Ok(false);
            }
        }
        if let Some(mut judging) = self.judging.take() {
            if !judging.thread.is_finished() {
                let late = judging.turns_late();
                if late {
                    self.ended = self.started;
                }
                self.judging = Some(judging);
                return Ok(late);
            }
            if let Some(judged) = judging.join() {
                self.judged(db, factory, home, judged)?;
            }
            return Ok(false);
        }
        if let Some(entry) = self.next.pop_front() {
            self.remove(db, factory, home, entry)?;
            return Ok(false);
        }
        if self.open {
            self.open = false;
            self.ended = self.started;
            return Ok(true);
        }
        self.retire_next(db, factory, home, short)?;
        Ok(false)
    }

    /// Records what the thread found, and lists what may go.
    fn judged(
        &mut self,
        db: &Connection,
        factory: &Factory,
        home: &Path,
        judged: Judged,
    ) -> rusqlite::Result<()> {
        self.branches.extend(judged.cut);
        self.entries = assess(db, home, Judge::Answers(&judged.answers))?;
        record(home, &self.entries);
        let mut next: Vec<usize> = Vec::new();
        for (at, entry) in self.entries.iter().enumerate() {
            let Some(task) = task_of(&entry.found) else {
                continue;
            };
            let kept = entry.judged.is_some() || entry.state.is_empty();
            if judged.unfetched.contains(&entry.found.repo) {
                continue;
            }
            // A teardown that failed stays failed until the factory's teardown changes.
            if matches!(recipe::teardown_of(db, task)?, Some(Teardown::Failed(_)))
                && !entry.state.is_empty()
            {
                recipe::teardown(db, factory, task, &entry.found.repo, &entry.found.path)?;
            } else if !kept && entry.stays.is_none() {
                next.push(at);
            }
        }
        let tried = tried();
        next.sort_by_key(|at| tried.get(&removing(&self.entries[*at].found.path)).copied());
        self.next = next.into();
        Ok(())
    }

    /// Moves aside the worktree of `entries[at]`, or owes its teardown.
    fn remove(
        &mut self,
        db: &Connection,
        factory: &Factory,
        home: &Path,
        at: usize,
    ) -> rusqlite::Result<()> {
        let entry = &self.entries[at];
        let Some(task) = task_of(&entry.found) else {
            return Ok(());
        };
        let path = entry.found.path.clone();
        let taken = match busy(db, task, &format!("'{}'", entry.state))? {
            Some(reason) => Err(reason),
            None => {
                let torn = recipe::teardown(db, factory, task, &entry.found.repo, &path)?;
                // The walk of one that was owed a teardown comes at the next sweep.
                if !matches!(torn, Teardown::Done) || !entry.walked {
                    return Ok(());
                }
                // Only this thread starts runs, so none starts between these checks and the move.
                worktrees::take(home, &entry.found)
            }
        };
        match taken {
            Ok(taken) => {
                let (found, tip) = (entry.found.clone(), entry.tip.clone());
                let branch = !entry.retire;
                self.delete(home, taken, found, tip, branch, None);
            }
            Err(error) => self.stayed(home, &path, error),
        }
        Ok(())
    }

    /// Records why a worktree that was to go stays.
    fn stayed(&mut self, home: &Path, path: &Path, error: String) {
        tried().insert(removing(path), Instant::now());
        eprintln!("priory daemon: removing {}: {error}", path.display());
        if !path.exists() {
            return;
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.found.path == path);
        if let Some(entry) = entry {
            entry.stays = Some(error.clone());
            entry.judged = Some(error);
            record(home, &self.entries);
        }
    }

    /// Checks what a tick moved aside and deletes it, on a thread.
    fn delete(
        &mut self,
        home: &Path,
        mut taken: worktrees::Taken,
        found: Found,
        tip: Option<String>,
        branch: bool,
        retires: Option<i64>,
    ) {
        let home = home.to_path_buf();
        let path = found.path.clone();
        let remove = move || {
            if let Err(stays) = worktrees::detach(&home, &mut taken, tip.as_deref(), branch) {
                if taken.displaced {
                    let what = format!("removing {}: {stays}", path.display());
                    note(&home, MOVED, what, false);
                }
                return Removed {
                    stays: Some(stays),
                    cut: false,
                };
            }
            if let Some(error) = &taken.branch_stays {
                eprintln!("priory daemon: removing {}: {error}", path.display());
            }
            if let Err(error) = worktrees::dispose(&home, &taken) {
                eprintln!("priory daemon: removing {error}");
            }
            Removed {
                stays: None,
                cut: taken.branch_stays.as_deref().is_some_and(timed_out),
            }
        };
        self.deleting = Some(Deleting {
            found,
            retires,
            thread: Off::new("deleting a removed worktree".to_owned(), remove),
        });
    }

    /// Takes the answer of a thread that removed a worktree.
    fn removed(
        &mut self,
        db: &Connection,
        home: &Path,
        deleting: Deleting,
    ) -> rusqlite::Result<()> {
        let Deleting {
            found,
            retires,
            thread,
        } = deleting;
        let Some(Removed { stays, cut }) = thread.join() else {
            return Ok(());
        };
        if cut {
            self.branches.push_back(found);
            return Ok(());
        }
        match (stays, retires) {
            (Some(stays), Some(task)) => unretired(db, task, &found, &stays),
            (Some(stays), None) => {
                self.stayed(home, &found.path, stays);
                Ok(())
            }
            (None, _) => Ok(()),
        }
    }

    /// Retires asleep tasks, longest asleep first, while the disk is short: removes each clean
    /// worktree and keeps its branch and sessions. A task that cannot retire gets the reason once.
    pub(crate) fn retire(&mut self, db: &Connection) -> rusqlite::Result<()> {
        if self.retiring.is_some() {
            return Ok(());
        }
        let mut asleep = db
            .prepare(
                "SELECT id, worktree FROM tasks
                 WHERE state = 'asleep' AND worktree IS NOT NULL ORDER BY changed, id",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Those that failed go last, or they would starve those behind them.
        let tried = tried();
        asleep.sort_by_key(|(_, worktree)| tried.get(&removing(Path::new(worktree))).copied());
        self.asleep = asleep.into();
        Ok(())
    }

    fn retire_next(
        &mut self,
        db: &Connection,
        factory: &Factory,
        home: &Path,
        short: impl FnOnce() -> bool,
    ) -> rusqlite::Result<()> {
        if let Some(mut retiring) = self.retiring.take() {
            if retiring.walk.thread.is_finished() {
                return self.retired(db, factory, home, retiring);
            }
            retiring.walk.turns_late();
            self.retiring = Some(retiring);
            return Ok(());
        }
        if self.asleep.is_empty() {
            return Ok(());
        }
        if !short() {
            self.asleep.clear();
            return Ok(());
        }
        let mut found = worktrees::found(home);
        while let Some((task, worktree)) = self.asleep.pop_front() {
            let at = found.iter().position(|found| {
                found.path.to_str() == Some(&worktree) && found.name == task.to_string()
            });
            let Some(at) = at else {
                continue;
            };
            let found = found.swap_remove(at);
            let (walked, home) = (found.clone(), home.to_path_buf());
            self.retiring = Some(Retiring {
                task,
                torn: recipe::torn(db, factory, task, &found.repo)?,
                walk: Off::new(format!("the walk of {worktree}"), move || {
                    worktrees::removable(&home, &walked)
                }),
                found,
            });
            return Ok(());
        }
        Ok(())
    }

    /// Retires the task whose walk ended, when nothing keeps its worktree.
    fn retired(
        &mut self,
        db: &Connection,
        factory: &Factory,
        home: &Path,
        retiring: Retiring,
    ) -> rusqlite::Result<()> {
        let Retiring {
            task,
            found,
            torn,
            walk,
        } = retiring;
        let walked = walk
            .join()
            .unwrap_or_else(|| Err("its check panicked".to_owned()));
        // Checked before the mark too, which clears the reason.
        let mut stays = match walked {
            Err(reason) => Some(reason),
            Ok(()) => busy(db, task, "'asleep'")?,
        };
        if stays.is_none() {
            // One teardown at a time: the next task's waits until this one retired and the disk
            // is measured again.
            match recipe::teardown(db, factory, task, &found.repo, &found.path)? {
                // A teardown that ended after the walk started asks for another walk.
                Teardown::Done if torn => {}
                Teardown::Done | Teardown::Runs => {
                    self.asleep.clear();
                    return Ok(());
                }
                Teardown::Failed(reason) => stays = Some(reason),
            }
        }
        if stays.is_none() {
            // Marked first, so a crash before the removal leaves a mark on a worktree, not the reverse.
            db.execute(
                "UPDATE tasks SET retired = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), reason = NULL
                 WHERE id = ?1",
                [task],
            )?;
            match worktrees::take(home, &found) {
                Ok(taken) => {
                    self.delete(home, taken, found, None, false, Some(task));
                    return Ok(());
                }
                Err(reason) => stays = Some(reason),
            }
        }
        let stays = stays.expect("a task that did not retire has a reason");
        unretired(db, task, &found, &stays)
    }
}

/// Says why `task` keeps its worktree, after a retirement was to take it.
fn unretired(db: &Connection, task: i64, found: &Found, stays: &str) -> rusqlite::Result<()> {
    tried().insert(removing(&found.path), Instant::now());
    if found.path.exists() {
        db.execute("UPDATE tasks SET retired = NULL WHERE id = ?1", [task])?;
    }
    let reason = format!("cannot retire: {stays}");
    // A task that woke during the walk keeps its own reason.
    let reported = db.execute(
        "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND state = 'asleep' AND reason IS NOT ?2",
        rusqlite::params![task, reason],
    )?;
    if reported > 0 {
        eprintln!("priory daemon: task {task} {reason}");
    }
    Ok(())
}

/// What the sweep recorded of a folder that stays: why git keeps it, and its size in bytes.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Kept {
    reason: Option<String>,
    bytes: Option<u64>,
}

/// Each folder that stays, by path.
type Record = BTreeMap<String, Kept>;

/// Serializes the writers of `sizes.json`: the sweep and the walk that measures.
static RECORDING: Mutex<()> = Mutex::new(());

/// Whether a walk that measures the folders runs.
static MEASURING: AtomicBool = AtomicBool::new(false);

fn record_file(home: &Path) -> PathBuf {
    home.join("sweep").join("sizes.json")
}

fn recorded(home: &Path) -> Record {
    std::fs::read(record_file(home))
        .ok()
        .and_then(|read| serde_json::from_slice(&read).ok())
        .unwrap_or_default()
}

fn write_record(home: &Path, record: &Record) {
    let file = record_file(home);
    let staging = file.with_extension("json.partial");
    let written = std::fs::create_dir_all(file.parent().expect("a file has a folder"))
        .and_then(|()| std::fs::write(&staging, serde_json::to_vec(record).expect("serializes")))
        .and_then(|()| std::fs::rename(&staging, &file));
    if let Err(written) = written {
        eprintln!("priory daemon: recording {}: {written}", file.display());
    }
}

/// Records each folder that stays and why git keeps it, with the size the last walk measured.
fn record(home: &Path, entries: &[Entry]) {
    let _recording = RECORDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut before = recorded(home);
    let record = entries
        .iter()
        .filter(|entry| entry.stays.is_some())
        .map(|entry| {
            let path = entry.found.path.display().to_string();
            let bytes = before.remove(&path).and_then(|kept| kept.bytes);
            let reason = entry.judged.clone();
            (path, Kept { reason, bytes })
        })
        .collect();
    write_record(home, &record);
}

/// Measures each folder the last sweep recorded, on a thread of its own, since walking every file
/// of every kept worktree takes seconds. It starts no second walk while one runs.
pub fn measure(home: &Path) {
    if MEASURING.swap(true, Ordering::SeqCst) {
        return;
    }
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        let sizes: Vec<(String, u64)> = recorded(&home)
            .into_keys()
            .map(|path| {
                let bytes = worktrees::size(Path::new(&path));
                (path, bytes)
            })
            .collect();
        {
            let _recording = RECORDING
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut record = recorded(&home);
            for (path, bytes) in sizes {
                if let Some(kept) = record.get_mut(&path) {
                    kept.bytes = Some(bytes);
                }
            }
            write_record(&home, &record);
        }
        MEASURING.store(false, Ordering::SeqCst);
    });
}

/// `bytes` in the largest binary unit that keeps it at 1 or more, such as `7.4 GiB`.
fn human(bytes: u64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KiB", "MiB", "GiB"] {
        if size < 1024.0 {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} TiB")
}

/// In `<home>`: why the daemon leaves the agents slice's limits alone, or the last time the sweep
/// restored them.
const SLICE: &str = "slice.json";

/// In `<home>`: the last removal that left files elsewhere than it found them.
const MOVED: &str = "moved.json";

/// What `SLICE` or `MOVED` keeps for the desk.
#[derive(serde::Serialize, serde::Deserialize)]
struct Noted {
    at: jiff::Timestamp,
    what: String,
    #[serde(default)]
    refused: bool,
}

fn note(home: &Path, name: &str, what: String, refused: bool) {
    eprintln!("priory daemon: {what}");
    let file = home.join(name);
    let at = jiff::Timestamp::now();
    let json = serde_json::to_vec(&Noted { at, what, refused }).expect("serializes");
    if let Err(error) = std::fs::write(&file, json) {
        eprintln!("priory daemon: recording {}: {error}", file.display());
    }
}

fn noted(home: &Path, name: &str) -> Option<Noted> {
    serde_json::from_slice(&std::fs::read(home.join(name)).ok()?).ok()
}

/// Takes a refusal that no longer holds off the desk.
pub(crate) fn clear_refusal(home: &Path) {
    let file = home.join(SLICE);
    if noted(home, SLICE).is_some_and(|noted| noted.refused)
        && let Err(error) = std::fs::remove_file(&file)
    {
        eprintln!("priory daemon: removing {}: {error}", file.display());
    }
}

/// The sweep's check of the agents slice's limits.
pub(crate) struct Limits {
    refused: Option<String>,
    /// It took the slice and has not set its limits yet.
    uncapped: bool,
    failed: bool,
}

impl Limits {
    /// The check of a daemon whose claim of `slice` was `refused`, if it was.
    pub(crate) fn new(slice: &str, home: &Path, refused: Option<String>) -> Self {
        let mut limits = Limits {
            refused: None,
            uncapped: false,
            failed: false,
        };
        if let Some(why) = refused {
            limits.refuse(slice, home, why);
        }
        limits
    }

    /// Says on the desk and in the log, once per reason, why the limits are left alone.
    fn refuse(&mut self, slice: &str, home: &Path, why: String) {
        if self.refused.as_ref() != Some(&why) {
            note(
                home,
                SLICE,
                format!("left the limits of {slice} alone: {why}"),
                true,
            );
            self.refused = Some(why);
        }
    }

    /// Sets the limits of a `slice` this daemon just took, or restores those another process
    /// changed, and says so in the log and on the desk. A check that keeps failing is logged once.
    pub(crate) fn check(&mut self, slice: &str, share: &crate::room::Share, home: &Path) {
        if let Err(why) = slice::claim(slice, home) {
            self.refuse(slice, home, why);
            return;
        }
        if self.refused.take().is_some() {
            eprintln!("priory daemon: took {slice}");
            clear_refusal(home);
            self.uncapped = true;
        }
        let checked = if self.uncapped {
            slice::cap(slice, share.cpu, share.memory).map(|()| None)
        } else {
            slice::restore(slice, share.cpu, share.memory)
        };
        let found = match checked {
            Ok(found) => found,
            Err(error) => {
                if !std::mem::replace(&mut self.failed, true) {
                    eprintln!("priory daemon: checking the limits of {slice}: {error}");
                }
                return;
            }
        };
        self.failed = false;
        self.uncapped = false;
        if let Some(found) = found {
            let what =
                format!("another process set {slice} to {found}; the sweep restored its limits");
            note(home, SLICE, what, false);
        }
    }
}

/// Why the daemon leaves the agents slice's limits alone, or their last restore, for a day.
pub fn slice_banner(home: &Path) -> Option<String> {
    banner(home, SLICE)
}

/// The last removal that left files elsewhere than it found them, for a day.
pub fn moved_banner(home: &Path) -> Option<String> {
    banner(home, MOVED)
}

fn banner(home: &Path, name: &str) -> Option<String> {
    let noted = noted(home, name)?;
    if noted.refused {
        return Some(noted.what);
    }
    let day = jiff::SignedDuration::from_hours(24);
    (noted.at + day > jiff::Timestamp::now()).then(|| {
        let at = noted.at.round(jiff::Unit::Second).unwrap_or(noted.at);
        format!("{} at {at}", noted.what)
    })
}

/// How long the sweep waits before it fetches again a clone whose fetch failed.
const RETRY: jiff::SignedDuration = jiff::SignedDuration::from_hours(1);

/// A clone whose last fetch failed, kept in `<home>/fetches/<owner>/<name>.json`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Failing {
    since: jiff::Timestamp,
    tried: jiff::Timestamp,
    error: String,
}

fn failing_file(home: &Path, repo: &str) -> PathBuf {
    home.join("fetches").join(format!("{repo}.json"))
}

fn failing(home: &Path, repo: &str) -> Option<Failing> {
    serde_json::from_slice(&std::fs::read(failing_file(home, repo)).ok()?).ok()
}

/// The key of `TRIED` for the last good fetch of `repo`.
fn fetched(home: &Path, repo: &str) -> PathBuf {
    home.join("fetches").join(repo)
}

/// Fetches the clone of each of `repos` and returns those it did not fetch. A clone whose fetch
/// failed is logged once, kept as failing, and fetched again only an hour after its last try. A
/// failure of a repository with no worktree left is forgotten.
fn fetch(home: &Path, repos: &[String], now: jiff::Timestamp) -> Vec<String> {
    let now = jiff::Timestamp::from_second(now.as_second()).unwrap_or(now);
    let mut unfetched = Vec::new();
    let mut hung = false;
    let mut oldest: Vec<&String> = repos.iter().collect();
    {
        let tried = tried();
        oldest.sort_by_key(|repo| tried.get(&home.join("repos").join(repo)).copied());
    }
    for repo in oldest {
        // One timeout says the network hangs: the other clones wait for the next sweep.
        if hung {
            // Its refs from an earlier sweep still say what merged.
            let fresh = tried().contains_key(&fetched(home, repo));
            if !fresh {
                unfetched.push(repo.clone());
            }
            continue;
        }
        let failed = failing(home, repo);
        if failed
            .as_ref()
            .is_some_and(|failed| failed.tried + RETRY > now)
        {
            unfetched.push(repo.clone());
            continue;
        }
        let file = failing_file(home, repo);
        tried().insert(home.join("repos").join(repo), Instant::now());
        let outcome = worktrees::fetch(home, repo);
        if outcome.is_ok() {
            tried().insert(fetched(home, repo), Instant::now());
        }
        match outcome {
            Ok(()) if failed.is_some() => {
                let _ = std::fs::remove_file(&file);
                eprintln!("priory daemon: fetching {repo} works again");
            }
            Ok(()) => {}
            Err(error) => {
                hung = timed_out(&error);
                if failed.is_none() {
                    eprintln!(
                        "priory daemon: fetching {repo}: {error}; the sweep tries it again every hour"
                    );
                }
                let failing = Failing {
                    since: failed.map_or(now, |failed| failed.since),
                    tried: now,
                    error: git_error(&error),
                };
                let written = std::fs::create_dir_all(file.parent().expect("a repo has an owner"))
                    .and_then(|()| {
                        std::fs::write(&file, serde_json::to_vec(&failing).expect("serializes"))
                    });
                if let Err(written) = written {
                    eprintln!("priory daemon: recording {}: {written}", file.display());
                }
                unfetched.push(repo.clone());
            }
        }
    }
    for (repo, file) in failing_files(home) {
        if !repos.contains(&repo) {
            let _ = std::fs::remove_file(file);
        }
    }
    unfetched
}

/// The first line git printed, without the command `run` puts first.
fn git_error(error: &str) -> String {
    let first = error.lines().next().unwrap_or_default();
    first
        .split_once("\": ")
        .map_or(first, |(_, said)| said)
        .to_owned()
}

/// Each `owner/name` with a failing fetch on record, and its file.
fn failing_files(home: &Path) -> Vec<(String, PathBuf)> {
    let read = |dir: &Path| {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .collect::<Vec<_>>()
    };
    let mut found = Vec::new();
    for (owner, dir) in read(&home.join("fetches")) {
        for (file, path) in read(&dir) {
            if let Some(name) = file.strip_suffix(".json") {
                found.push((format!("{owner}/{name}"), path));
            }
        }
    }
    found.sort();
    found
}

/// One line per repository whose clone fails to fetch while it still has worktrees: since when,
/// git's error, and the folders to remove when its remote is gone. Joined by `; `.
pub fn fetch_banner(home: &Path) -> Option<String> {
    let lines: Vec<String> = failing_files(home)
        .into_iter()
        .filter(|(repo, _)| home.join("worktrees").join(repo).is_dir())
        .filter_map(|(repo, _)| {
            let failed = failing(home, &repo)?;
            Some(format!(
                "fetching {repo} fails since {}: {}; the sweep tries it again every hour. \
                 If its remote is gone, remove {} and {}",
                failed.since,
                failed.error,
                home.join("worktrees").join(&repo).display(),
                home.join("repos").join(&repo).display()
            ))
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("; "))
}

/// A build folder of an idle task's worktree, and the disk it takes.
struct Output {
    task: i64,
    found: Arc<Found>,
    build: Vec<String>,
    path: PathBuf,
    bytes: u64,
}

/// Deletes the build output of idle tasks' worktrees while free disk is under the reclaim
/// threshold, one folder at a time, largest first and longest idle on a tie. Idle is done, dropped,
/// held or asleep, with no live run and no memory grant. One thread lists and sizes the folders and
/// another deletes, so the tick never walks one.
#[derive(Default)]
pub(crate) struct Reclaim {
    walk: Option<JoinHandle<Vec<Output>>>,
    pending: VecDeque<Output>,
    delete: Option<Delete>,
    /// The paths the daemon logged a problem with, so it logs each once.
    reported: HashSet<PathBuf>,
    /// The build folders on another filesystem than `home`, which cannot move.
    elsewhere: HashSet<PathBuf>,
    /// A sweep came due and its retirement has not run yet.
    retires: bool,
}

/// How long the loop waits for one thread of a sweep or a delete.
const OVERDUE: Duration = Duration::from_secs(10 * 60);

/// A thread that deletes `output`, moved to `home/reclaim`, or what a stopped daemon left there. It
/// returns each folder it could not delete, and why.
struct Delete {
    output: Option<Output>,
    thread: JoinHandle<Vec<(PathBuf, String)>>,
    started: Instant,
    /// It ran past `OVERDUE`, so retirement no longer waits for it.
    overdue: bool,
}

impl Delete {
    fn new(output: Option<Output>, thread: JoinHandle<Vec<(PathBuf, String)>>) -> Delete {
        Delete {
            output,
            thread,
            started: Instant::now(),
            overdue: false,
        }
    }
}

impl Reclaim {
    /// Whether no walk and no delete runs and no folder waits.
    fn settled(&self) -> bool {
        self.walk.is_none() && self.delete.is_none() && self.pending.is_empty()
    }

    /// Whether to retire now: once per due sweep, when all that sweep can reclaim is gone or its
    /// delete is overdue.
    pub(crate) fn retires(&mut self) -> bool {
        let overdue = self.delete.as_ref().is_some_and(|delete| delete.overdue);
        let now = self.retires && (overdue || self.settled());
        self.retires &= !now;
        now
    }

    fn report(&mut self, path: PathBuf, reason: &str) {
        if self.reported.insert(path) {
            eprintln!("priory daemon: reclaiming build output: {reason}");
        }
    }

    /// With `due`, starts a walk when none runs, nothing waits and no retirement is pending. `under`
    /// says free disk is under the threshold. Each deletion is traced once it ends.
    pub(crate) fn tick(
        &mut self,
        db: &mut Connection,
        factory: &Factory,
        home: &Path,
        trace: &Trace,
        due: bool,
        mut under: impl FnMut() -> bool,
    ) -> rusqlite::Result<()> {
        // A walk per due tick would postpone a pending retirement for ever.
        let pending = self.retires;
        self.retires |= due;
        let due = due && !pending;
        if let Some(delete) = self.delete.take_if(|delete| delete.thread.is_finished()) {
            let stuck = delete.thread.join().unwrap_or_else(|_| {
                let reason = "a delete thread panicked".to_owned();
                vec![(home.join(RECLAIM), reason)]
            });
            if let (Some(output), true) = (delete.output, stuck.is_empty()) {
                let tx = db.transaction()?;
                trace::worktree_reclaimed(&tx, trace, output.task, &output.path, output.bytes)?;
                tx.commit()?;
            }
            for (path, reason) in stuck {
                self.report(path, &reason);
            }
        }
        if let Some(walk) = self.walk.take_if(|walk| walk.is_finished()) {
            self.pending = walk.join().unwrap_or_default().into();
        }
        if let Some(delete) = &mut self.delete
            && !delete.overdue
            && delete.started.elapsed() >= OVERDUE
        {
            delete.overdue = true;
            let path = match &delete.output {
                Some(output) => output.path.clone(),
                None => home.join(RECLAIM),
            };
            eprintln!(
                "priory daemon: reclaiming build output: deleting {} takes over 10 minutes; \
                 retirement no longer waits for it",
                path.display()
            );
        }
        if self.walk.is_some() || self.delete.is_some() {
            return Ok(());
        }
        let Some(output) = self.pending.pop_front() else {
            if !due {
                return Ok(());
            }
            if let Err(reason) = reclaim_folder(home) {
                self.report(home.join(RECLAIM), &reason);
                return Ok(());
            }
            let mut left = leftovers(home);
            left.retain(|path| !worktrees::moved_back(home, path));
            worktrees::drop_links(home);
            if !left.is_empty() {
                self.delete = Some(Delete::new(None, delete(home, left, None)));
            }
            if under() {
                let worktrees = idle_build(db, factory, home)?;
                self.walk = Some(walk(home, worktrees, self.elsewhere.clone()));
            }
            return Ok(());
        };
        if !under() {
            self.pending.clear();
            return Ok(());
        }
        if !idle(db)?.iter().any(|(task, _)| *task == output.task) {
            return Ok(());
        }
        // A reload may have changed the list since the walk.
        if build_of(factory, &output.found).as_ref() != Some(&output.build) {
            return Ok(());
        }
        // Only this thread starts runs, so none starts between these checks and the move.
        let name = format!("{}-{}", output.task, jiff::Timestamp::now().as_nanosecond());
        let aside = match reclaim_folder(home) {
            Ok(folder) => folder.join(name),
            Err(reason) => {
                self.pending.clear();
                self.report(home.join(RECLAIM), &reason);
                return Ok(());
            }
        };
        let Output {
            found, build, path, ..
        } = &output;
        if let Err(error) = worktrees::set_aside(home, found, build, path, &aside) {
            eprintln!("priory daemon: task {}: {}", output.task, error.reason);
            if error.elsewhere {
                self.elsewhere.insert(output.path);
            }
            return Ok(());
        }
        let thread = delete(home, vec![aside], Some(output.path.clone()));
        self.delete = Some(Delete::new(Some(output), thread));
        Ok(())
    }
}

/// The folders a stopped daemon left under `home/reclaim`: real ones the daemon named, a task and
/// nanoseconds, which have 18 digits or more.
fn leftovers(home: &Path) -> Vec<PathBuf> {
    let Ok(folder) = reclaim_folder(home) else {
        return Vec::new();
    };
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(worktrees::reclaim_name)
        })
        .map(|entry| entry.path())
        .filter(|path| !worktrees::in_hand(path))
        .collect()
}

/// Moves back the worktrees a stopped daemon left under `home/reclaim`, before a start can prune
/// their entries.
pub(crate) fn move_back(home: &Path) {
    for path in leftovers(home) {
        worktrees::moved_back(home, &path);
    }
}

/// Deletes each of `paths`, folders under `home/reclaim`, on a thread. One that must stay after all
/// goes back to `from`, when nothing took its place.
fn delete(
    home: &Path,
    paths: Vec<PathBuf>,
    from: Option<PathBuf>,
) -> JoinHandle<Vec<(PathBuf, String)>> {
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        let mut stuck = Vec::new();
        for path in paths {
            let deleted = reclaim_folder(&home).and_then(|_| {
                worktrees::expendable(&path).map_err(|reason| {
                    let back = from.as_ref().is_some_and(|from| {
                        std::fs::symlink_metadata(from).is_err()
                            && std::fs::rename(&path, from).is_ok()
                    });
                    let end = if back {
                        "it moved back"
                    } else {
                        "it stays there"
                    };
                    format!("{reason}; {end}")
                })?;
                worktrees::discard(&path)
            });
            if let Err(reason) = deleted {
                stuck.push((path, reason));
            }
        }
        stuck
    })
}

/// Whether nothing of a task can be at work in its worktree: no live run and no memory grant.
const UNUSED: &str =
    "NOT EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id AND runs.ended IS NULL)
     AND NOT EXISTS (SELECT 1 FROM grants WHERE grants.task = tasks.id)";

/// Why the worktree of `task` cannot go now: its state left `states`, a quoted list, or it is in use.
fn busy(db: &Connection, task: i64, states: &str) -> rusqlite::Result<Option<String>> {
    let free = db
        .prepare(&format!(
            "SELECT 1 FROM tasks WHERE id = ?1 AND state IN ({states}) AND {UNUSED}"
        ))?
        .exists([task])?;
    Ok((!free).then(|| format!("task {task} has a live run or a memory grant, or left its state")))
}

/// The idle tasks and their worktree paths, longest idle first.
fn idle(db: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    db.prepare(&format!(
        "SELECT id, worktree FROM tasks
         WHERE state IN ('done', 'dropped', 'held', 'asleep') AND worktree IS NOT NULL
           AND {UNUSED} ORDER BY changed, id"
    ))?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// Each idle task's worktree with the build folders its repository names, longest idle first. A
/// worktree with a `Cargo.toml` defaults to `target`.
fn idle_build(
    db: &Connection,
    factory: &Factory,
    home: &Path,
) -> rusqlite::Result<Vec<(i64, Found, Vec<String>)>> {
    let mut found = worktrees::found(home);
    let mut worktrees = Vec::new();
    for (task, worktree) in idle(db)? {
        let Some(at) = found.iter().position(|found| {
            found.path.to_str() == Some(&worktree) && task_of(found) == Some(task)
        }) else {
            continue;
        };
        let found = found.swap_remove(at);
        if let Some(build) = build_of(factory, &found) {
            worktrees.push((task, found, build));
        }
    }
    Ok(worktrees)
}

/// The build folders of the worktree, `target` by default when it has a `Cargo.toml`.
fn build_of(factory: &Factory, found: &Found) -> Option<Vec<String>> {
    match factory.build(&found.repo) {
        Some(build) => Some(build.to_vec()),
        None => found
            .path
            .join("Cargo.toml")
            .is_file()
            .then(|| vec![String::from("target")]),
    }
}

/// Lists and sizes the build folders of `worktrees` on a thread, largest first.
/// It skips the folders `elsewhere`.
fn walk(
    home: &Path,
    worktrees: Vec<(i64, Found, Vec<String>)>,
    elsewhere: HashSet<PathBuf>,
) -> JoinHandle<Vec<Output>> {
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        let mut outputs = Vec::new();
        for (task, found, build) in worktrees {
            let Ok(folders) = worktrees::build_output(&home, &found, &build, &elsewhere) else {
                continue;
            };
            let found = Arc::new(found);
            outputs.extend(folders.into_iter().map(|(path, bytes)| Output {
                task,
                found: found.clone(),
                build: build.clone(),
                path,
                bytes,
            }));
        }
        // Stable, so equal sizes keep the longest idle first.
        outputs.sort_by_key(|output| std::cmp::Reverse(output.bytes));
        outputs
    })
}

/// The desk's line on the build output reclaimed in the 24 hours before `now`, if any.
pub fn reclaim_line(db: &Connection, now: jiff::Timestamp) -> rusqlite::Result<Option<String>> {
    let since = (now - jiff::SignedDuration::from_hours(24)).to_string();
    let (count, bytes, last): (i64, Option<f64>, Option<String>) = db.query_row(
        "SELECT count(*), sum(json_extract(event, '$.data.bytes')), max(time) FROM trace
         WHERE type = 'factory.worktree.reclaimed' AND time >= ?1",
        [since],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let Some(last) = last else {
        return Ok(None);
    };
    let folders = if count == 1 { "folder" } else { "folders" };
    let last = last
        .get(..19)
        .map_or(last.clone(), |second| format!("{second}Z"));
    Ok(Some(format!(
        "reclaimed {:.1} GB of build output from {count} {folders} in the last 24 hours, last at {last}",
        bytes.unwrap_or(0.0) / (1u64 << 30) as f64
    )))
}

/// The key of `TRIED` for the removal of `worktree`.
fn removing(worktree: &Path) -> PathBuf {
    worktree.join("removal")
}

/// Every worktree under `home` that stays: its path, its task when its name is one, and why, with
/// the size the last walk measured. It reads why git keeps a folder from the last sweep's record,
/// so it runs no git.
pub fn stays(db: &Connection, home: &Path) -> rusqlite::Result<Vec<(String, Option<i64>, String)>> {
    let record = recorded(home);
    Ok(assess(db, home, Judge::Recorded(&record))?
        .into_iter()
        .filter_map(|entry| {
            let reason = entry.stays?;
            let task = entry.found.name.parse().ok();
            let path = entry.found.path.display().to_string();
            let reason = match record.get(&path).and_then(|kept| kept.bytes) {
                Some(bytes) => format!("{reason}, {}", human(bytes)),
                None => reason,
            };
            Some((path, task, reason))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use priory_runtime::runtime::{self, remove_sealed};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    /// A home with a clone of `o/r` and the worktree of each task it adds, removed on drop.
    struct Home {
        dir: PathBuf,
        db: Connection,
        factory: Factory,
        reclaim: Reclaim,
        sweeping: Sweeping,
    }

    impl Home {
        fn new(name: &str) -> Home {
            let dir =
                std::env::temp_dir().join(format!("priory-reclaim-{name}-{}", std::process::id()));
            remove_sealed(&dir);
            let clone = dir.join("repos/o/r");
            std::fs::create_dir_all(&clone).unwrap();
            std::fs::write(
                dir.join("factory.toml"),
                "name = \"demo\"\n[repos.\"o/r\"]\nbuild = [\"target\"]\n",
            )
            .unwrap();
            git(&clone, &["init", "-q"]);
            std::fs::write(clone.join("a.txt"), "a\n").unwrap();
            git(&clone, &["add", "."]);
            git(&clone, &["commit", "-q", "-m", "a"]);
            let factory = priory_factory::factory::load(&dir).0;
            Home {
                db: crate::state::memory(),
                factory,
                reclaim: Reclaim::default(),
                sweeping: Sweeping::default(),
                dir,
            }
        }

        /// Adds `task` in `state` with its worktree, which holds build output, and returns the worktree.
        fn task(&self, task: i64, state: &str) -> PathBuf {
            let tree = self.dir.join(format!("worktrees/o/r/{task}"));
            let branch = format!("priory/{task}");
            git(
                &self.dir.join("repos/o/r"),
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &branch,
                    tree.to_str().unwrap(),
                ],
            );
            std::fs::create_dir_all(tree.join("target/debug")).unwrap();
            std::fs::write(tree.join("target/debug/lib.rlib"), "built").unwrap();
            self.db
                .execute(
                    "INSERT INTO tasks (id, key, workflow, step, state, trigger, source, worktree)
                     VALUES (?1, 'cli:' || ?1, 'build', 'code', ?2, 'cli', 'a', ?3)",
                    rusqlite::params![task, state, tree.to_str()],
                )
                .unwrap();
            tree
        }

        /// Reloads the factory with `build = []`.
        fn opt_out(&mut self) {
            let text = "name = \"demo\"\n[repos.\"o/r\"]\nbuild = []\n";
            std::fs::write(self.dir.join("factory.toml"), text).unwrap();
            self.factory = priory_factory::factory::load(&self.dir).0;
        }

        /// Ticks as the daemon does and returns whether it retires.
        fn tick(&mut self, due: bool, under: bool) -> bool {
            let trace = Trace {
                factory: &self.factory,
                revision: None,
                machine: None,
                runner: None,
                usage: |_, _| None,
            };
            self.reclaim
                .tick(&mut self.db, &self.factory, &self.dir, &trace, due, || {
                    under
                })
                .unwrap();
            self.reclaim.retires()
        }

        /// Runs a due tick and waits for the walk it starts. The next tick takes its first folder.
        fn walk(&mut self) {
            self.tick(true, true);
            let walk = self.reclaim.walk.as_ref().expect("a walk runs");
            until(|| walk.is_finished());
        }

        /// Waits for the delete in flight.
        fn deleted(&self) {
            let delete = self.reclaim.delete.as_ref().expect("a delete runs");
            until(|| delete.thread.is_finished());
        }

        fn traced(&self) -> i64 {
            self.db
                .query_row(
                    "SELECT count(*) FROM trace WHERE type = 'factory.worktree.reclaimed'",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        }

        /// One tick of the sweep, on a disk that is short. Returns whether the sweep ended.
        fn sweep_tick(&mut self) -> bool {
            self.sweeping
                .tick(&self.db, &self.factory, &self.dir, || true)
                .unwrap()
        }

        fn start_sweep(&mut self) -> bool {
            self.sweeping
                .start(&self.db, &self.factory, &self.dir)
                .unwrap()
        }

        /// Runs a sweep to its end.
        fn sweep(&mut self) {
            assert!(self.start_sweep());
            let deadline = Instant::now() + Duration::from_secs(60);
            while !self.sweep_tick() {
                assert!(Instant::now() < deadline, "the sweep does not end");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// Runs a retirement to its end.
        fn retire(&mut self) {
            self.sweeping.retire(&self.db).unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while self.sweeping.busy() {
                assert!(Instant::now() < deadline, "the retirement does not end");
                self.sweep_tick();
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        /// Ticks once after a walk and asserts that nothing goes.
        fn keeps(&mut self, tree: &Path) {
            self.tick(false, true);
            assert!(self.reclaim.delete.is_none());
            assert!(tree.join("target/debug/lib.rlib").exists());
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            remove_sealed(&self.dir);
        }
    }

    fn until(done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = runtime::git(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
    }

    #[test]
    fn reclaim_moves_the_folder_on_the_tick_and_traces_it_after_the_delete() {
        let mut home = Home::new("moves");
        let tree = home.task(1, "asleep");
        home.walk();
        home.tick(false, true);
        assert!(!tree.join("target").exists(), "a later run finds none");
        assert_eq!(home.traced(), 0);

        home.deleted();
        assert_eq!(leftovers(&home.dir).len(), 0);
        home.tick(false, true);
        assert_eq!(home.traced(), 1);
        assert!(tree.join("a.txt").exists());
    }

    #[test]
    fn reclaim_clears_what_a_stopped_daemon_left_even_with_room() {
        let mut home = Home::new("left");
        std::fs::create_dir_all(home.dir.join("reclaim/7-111111111111111111/debug")).unwrap();
        home.tick(true, false);
        home.deleted();
        home.tick(false, false);
        assert_eq!(leftovers(&home.dir).len(), 0);
        assert_eq!(home.traced(), 0);
    }

    #[test]
    fn reclaim_keeps_a_worktree_changed_after_the_walk() {
        let mut home = Home::new("changed");
        let tree = home.task(1, "asleep");
        home.walk();
        std::fs::write(tree.join("a.txt"), "edited\n").unwrap();
        home.keeps(&tree);
    }

    #[test]
    fn reclaim_keeps_a_worktree_a_process_entered_after_the_walk() {
        let mut home = Home::new("process");
        let tree = home.task(1, "asleep");
        home.walk();
        let mut process = Command::new("sleep")
            .arg("60")
            .current_dir(&tree)
            .spawn()
            .unwrap();
        let cwd = PathBuf::from(format!("/proc/{}/cwd", process.id()));
        let entered = std::fs::canonicalize(&tree).unwrap();
        until(|| std::fs::read_link(&cwd).is_ok_and(|cwd| cwd == entered));
        home.keeps(&tree);
        process.kill().unwrap();
        process.wait().unwrap();
    }

    #[test]
    fn reclaim_keeps_a_worktree_whose_task_revived_after_the_walk() {
        for state in ["ready", "running"] {
            let mut home = Home::new(state);
            let tree = home.task(1, "done");
            home.walk();
            home.db
                .execute("UPDATE tasks SET state = ?1 WHERE id = 1", [state])
                .unwrap();
            home.keeps(&tree);
        }
    }

    #[test]
    fn reclaim_keeps_a_worktree_whose_task_has_a_live_run_after_the_walk() {
        let mut home = Home::new("run");
        let tree = home.task(1, "asleep");
        home.walk();
        home.db
            .execute(
                "INSERT INTO runs (task, step, tmux, session) VALUES (1, 'code', 't', 's')",
                [],
            )
            .unwrap();
        home.keeps(&tree);
    }

    #[test]
    fn reclaim_keeps_a_worktree_with_an_untracked_file_outside_the_build_folder() {
        let mut home = Home::new("untracked");
        let tree = home.task(1, "asleep");
        std::fs::write(tree.join("notes.txt"), "mine\n").unwrap();
        home.walk();
        home.keeps(&tree);

        std::fs::remove_file(tree.join("notes.txt")).unwrap();
        home.walk();
        std::fs::write(tree.join("notes.txt"), "mine\n").unwrap();
        home.keeps(&tree);
    }

    #[test]
    fn reclaim_skips_a_repository_with_an_empty_build_list() {
        let mut home = Home::new("opt-out");
        let tree = home.task(1, "asleep");
        // A committed Cargo.toml and an ignored target, so only the empty list keeps it.
        std::fs::write(tree.join("Cargo.toml"), "").unwrap();
        std::fs::write(tree.join(".gitignore"), "target\n").unwrap();
        git(&tree, &["add", "Cargo.toml", ".gitignore"]);
        git(&tree, &["commit", "-q", "-m", "cargo"]);
        home.opt_out();
        home.walk();
        home.keeps(&tree);
    }

    #[test]
    fn reclaim_stops_when_the_repository_opts_out_after_the_walk() {
        let mut home = Home::new("reload");
        let tree = home.task(1, "asleep");
        home.walk();
        home.opt_out();
        home.keeps(&tree);
    }

    #[test]
    fn reclaim_stops_once_free_disk_is_back() {
        let mut home = Home::new("stops");
        let trees = [home.task(1, "asleep"), home.task(2, "asleep")];
        home.walk();
        home.tick(false, true);
        home.deleted();
        home.tick(false, false);
        home.tick(false, true);
        assert!(home.reclaim.delete.is_none());
        let left = trees.iter().filter(|tree| tree.join("target").exists());
        assert_eq!(left.count(), 1);
        assert_eq!(home.traced(), 1);
    }

    #[test]
    fn reclaim_never_takes_a_blocked_or_failed_task() {
        for state in ["blocked", "failed"] {
            let mut home = Home::new(state);
            let tree = home.task(1, state);
            home.walk();
            home.keeps(&tree);

            // A task that turns blocked or failed after the walk keeps its output too.
            home.db
                .execute("UPDATE tasks SET state = 'asleep' WHERE id = 1", [])
                .unwrap();
            home.walk();
            home.db
                .execute("UPDATE tasks SET state = ?1 WHERE id = 1", [state])
                .unwrap();
            home.keeps(&tree);
        }
    }

    const CARGO: &str = "Signature: 8a477f597d28d172789f06886806bc55\n\
        # This file is a cache directory tag created by cargo.\n";

    /// Walks, then takes the first folder and waits for its delete and the tick after it.
    fn take(home: &mut Home) {
        home.walk();
        home.tick(false, true);
        home.deleted();
        home.tick(false, true);
    }

    #[test]
    fn reclaim_neither_empties_nor_fills_a_folder_a_symlink_leads_to() {
        let mut home = Home::new("symlink");
        let tree = home.task(1, "asleep");
        let outside = home.dir.join("outside");
        std::fs::create_dir_all(outside.join("7-111111111111111111")).unwrap();
        std::os::unix::fs::symlink(&outside, home.dir.join("reclaim")).unwrap();
        home.tick(true, true);
        assert!(home.reclaim.settled());

        // Also when the symlink takes the folder's place after the walk.
        std::fs::remove_file(home.dir.join("reclaim")).unwrap();
        home.walk();
        std::fs::remove_dir(home.dir.join("reclaim")).unwrap();
        std::os::unix::fs::symlink(&outside, home.dir.join("reclaim")).unwrap();
        home.keeps(&tree);
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    }

    #[test]
    fn reclaim_deletes_only_folders_it_named() {
        let mut home = Home::new("foreign");
        let reclaim = home.dir.join("reclaim");
        let file = format!("8-{}", "1".repeat(18));
        let kept = [
            "-111111111111111111",
            "7-",
            "7-11111111111111111",
            "7-111111111111111111-2",
            "7-x",
            "notes",
        ];
        for name in kept {
            std::fs::create_dir_all(reclaim.join(name)).unwrap();
        }
        for digits in [18, 19] {
            let named = format!("7-{}", "1".repeat(digits));
            std::fs::create_dir_all(reclaim.join(named)).unwrap();
        }
        std::fs::write(reclaim.join(&file), "a file").unwrap();
        home.tick(true, false);
        home.deleted();
        let mut left: Vec<_> = std::fs::read_dir(&reclaim)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        left.sort();
        let mut kept: Vec<_> = kept.map(std::ffi::OsString::from).into();
        kept.push(file.into());
        kept.sort();
        assert_eq!(left, kept);
    }

    #[test]
    fn a_stuck_leftover_is_reported_once_and_later_output_is_traced() {
        let mut home = Home::new("stuck");
        home.task(1, "asleep");
        let stuck = home.dir.join("reclaim/7-111111111111111111");
        std::fs::create_dir_all(stuck.join("nested/.git")).unwrap();
        for _ in 0..2 {
            home.tick(true, false);
            home.deleted();
            home.tick(false, false);
        }
        assert_eq!(home.reclaim.reported, HashSet::from([stuck.clone()]));

        home.walk();
        home.deleted();
        home.tick(false, true);
        home.deleted();
        home.tick(false, true);
        assert_eq!(home.traced(), 1);
        assert!(stuck.join("nested/.git").exists());
    }

    #[test]
    fn an_undeletable_leftover_is_reported_and_not_traced() {
        let mut home = Home::new("undeletable");
        home.task(1, "asleep");
        let reclaim = home.dir.join("reclaim");
        let stuck = reclaim.join("7-111111111111111111");
        std::fs::create_dir_all(&stuck).unwrap();
        std::fs::set_permissions(&reclaim, std::fs::Permissions::from_mode(0o555)).unwrap();
        home.tick(true, false);
        home.deleted();
        home.tick(false, false);
        std::fs::set_permissions(&reclaim, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(home.reclaim.reported.contains(&stuck));
        assert_eq!(home.traced(), 0);
        assert!(stuck.exists());
    }

    #[test]
    fn reclaim_keeps_a_folder_that_holds_a_repository() {
        let mut home = Home::new("nested");
        let tree = home.task(1, "asleep");
        std::fs::create_dir_all(tree.join("target/nested/.git")).unwrap();
        home.walk();
        home.keeps(&tree);

        std::fs::remove_dir(tree.join("target/nested/.git")).unwrap();
        home.walk();
        std::fs::create_dir(tree.join("target/nested/.git")).unwrap();
        home.tick(false, true);
        home.deleted();
        home.tick(false, true);
        assert!(tree.join("target/debug/lib.rlib").exists(), "it moved back");
        assert_eq!(home.traced(), 0);
    }

    #[test]
    fn reclaim_keeps_a_folder_that_holds_a_bare_repository() {
        let mut home = Home::new("bare");
        let tree = home.task(1, "asleep");
        git(&tree, &["init", "-q", "--bare", "target/debug/cache/o.git"]);
        home.walk();
        home.keeps(&tree);

        std::fs::rename(tree.join("target/debug/cache"), tree.join("target/tmp")).unwrap();
        std::fs::write(tree.join("target/CACHEDIR.TAG"), CARGO).unwrap();
        take(&mut home);
        assert!(!tree.join("target").exists());
        assert_eq!(home.traced(), 1);
    }

    #[test]
    fn reclaim_keeps_a_build_folder_that_is_a_bare_repository() {
        let mut home = Home::new("bare-root");
        let tree = home.task(1, "asleep");
        git(&tree, &["init", "-q", "--bare", "target"]);
        home.walk();
        home.keeps(&tree);
    }

    /// Ticks with every tick due until a retirement runs, and returns whether it ran.
    fn retires_while_always_due(home: &mut Home) -> bool {
        (0..20).any(|_| {
            let retires = home.tick(true, true);
            // What is in flight ends before the next tick, as when ticks are slower than it.
            if let Some(walk) = &home.reclaim.walk {
                until(|| walk.is_finished());
            }
            if let Some(delete) = &home.reclaim.delete {
                until(|| delete.thread.is_finished());
            }
            retires
        })
    }

    #[test]
    fn a_retirement_runs_though_every_tick_is_due() {
        let mut home = Home::new("always-due");
        assert!(retires_while_always_due(&mut home), "nothing to take");

        let tree = home.task(1, "asleep");
        assert!(retires_while_always_due(&mut home), "a folder to take");
        assert!(!tree.join("target").exists(), "the reclaim goes first");
        assert_eq!(home.traced(), 1);
        assert!(!home.tick(false, true), "once per due sweep");

        home.walk();
    }

    #[test]
    fn a_retirement_runs_though_a_stuck_leftover_is_retried_at_every_sweep() {
        let mut home = Home::new("due-stuck");
        let stuck = home.dir.join("reclaim/7-111111111111111111");
        std::fs::create_dir_all(stuck.join("nested/.git")).unwrap();
        assert!(retires_while_always_due(&mut home));
    }

    #[test]
    fn a_retirement_waits_10_minutes_for_a_delete_that_hangs() {
        let mut home = Home::new("hangs");
        let trees = [home.task(1, "asleep"), home.task(2, "asleep")];
        home.walk();
        let (release, hung) = std::sync::mpsc::channel::<()>();
        let output = home.reclaim.walk.take().unwrap().join().unwrap();
        home.reclaim.pending = output.into();
        let first = home.reclaim.pending.pop_front();
        home.reclaim.delete = Some(Delete::new(
            first,
            std::thread::spawn(move || {
                let _ = hung.recv();
                Vec::new()
            }),
        ));
        assert!(!home.tick(true, true));

        home.reclaim.delete.as_mut().unwrap().started = Instant::now() - OVERDUE;
        assert!(home.tick(true, true));
        assert!(!home.tick(false, true), "once per due sweep");
        assert!(home.tick(true, true), "the next sweep waits no longer");
        assert!(trees.iter().all(|tree| tree.join("target").exists()));

        release.send(()).unwrap();
        home.deleted();
        home.tick(false, true);
        assert_eq!(home.traced(), 1);
        home.deleted();
        home.tick(false, true);
        assert_eq!(home.traced(), 2);
        assert_eq!(
            trees
                .iter()
                .filter(|tree| tree.join("target").exists())
                .count(),
            1,
            "the next folder goes once the delete ends"
        );
    }

    #[test]
    fn reclaim_takes_a_cargo_target_with_a_repository_in_its_tmp() {
        let mut home = Home::new("cargo");
        let tree = home.task(1, "asleep");
        std::fs::create_dir_all(tree.join("target/tmp/a/.git")).unwrap();
        home.walk();
        home.keeps(&tree);

        std::fs::write(tree.join("target/CACHEDIR.TAG"), CARGO).unwrap();
        take(&mut home);
        assert!(!tree.join("target").exists());
        assert_eq!(home.traced(), 1);
    }

    #[test]
    fn reclaim_keeps_a_folder_inside_a_submodule() {
        for checked_out in [true, false] {
            let mut home = Home::new("submodule");
            let text = "name = \"demo\"\n[repos.\"o/r\"]\nbuild = [\"sub/target\"]\n";
            std::fs::write(home.dir.join("factory.toml"), text).unwrap();
            home.factory = priory_factory::factory::load(&home.dir).0;
            let tree = home.task(1, "asleep");
            std::fs::remove_dir_all(tree.join("target")).unwrap();
            let sub = tree.join("sub");
            std::fs::create_dir_all(sub.join("target")).unwrap();
            git(&sub, &["init", "-q"]);
            std::fs::write(sub.join("target/kept"), "tracked").unwrap();
            git(&sub, &["add", "."]);
            git(&sub, &["commit", "-q", "-m", "sub"]);
            git(&tree, &["-c", "advice.addEmbeddedRepo=false", "add", "sub"]);
            git(&tree, &["commit", "-q", "-m", "sub"]);
            if !checked_out {
                std::fs::remove_dir_all(sub.join(".git")).unwrap();
            }
            home.walk();
            home.tick(false, true);
            assert!(home.reclaim.settled());
            assert!(sub.join("target/kept").exists());
        }
    }

    #[test]
    fn reclaim_tries_a_worktree_on_another_filesystem_once_and_leaves_its_mode() {
        use std::os::unix::fs::MetadataExt;
        let mut home = Home::new("elsewhere");
        let shm = PathBuf::from(format!("/dev/shm/priory-elsewhere-{}", std::process::id()));
        std::fs::create_dir_all(&shm).unwrap();
        let device = |path: &Path| std::fs::metadata(path).unwrap().dev();
        assert_ne!(device(&shm), device(&home.dir));
        std::os::unix::fs::symlink(&shm, home.dir.join("worktrees")).unwrap();
        let target = home.task(1, "asleep").join("target");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o555)).unwrap();

        home.walk();
        home.tick(false, true);
        assert!(home.reclaim.delete.is_none());
        assert_eq!(home.reclaim.elsewhere, HashSet::from([target.clone()]));
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o555);

        home.walk();
        let walked = home.reclaim.walk.take().unwrap().join().unwrap();
        assert!(walked.is_empty());
        remove_sealed(&shm);
    }

    #[test]
    fn a_retirement_keeps_the_worktree_of_a_task_in_use() {
        let mut home = Home::new("in-use");
        let exclude = home.dir.join("repos/o/r/.git/info/exclude");
        std::fs::write(exclude, "target\n").unwrap();
        let tree = home.task(1, "asleep");
        home.db
            .execute(
                "INSERT INTO grants (bytes, pid, started, task, command, created)
                 VALUES (1, 1, 1, 1, 'build', 'now')",
                [],
            )
            .unwrap();
        home.retire();
        assert!(tree.exists());
        let reason: String = home
            .db
            .query_row("SELECT reason FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert!(reason.contains("memory grant"), "{reason}");

        home.db.execute("DELETE FROM grants", []).unwrap();
        home.retire();
        assert!(!tree.exists());
    }

    fn unpushed(home: &Home, task: i64) -> PathBuf {
        let tree = home.task(task, "done");
        std::fs::write(tree.join("a.txt"), format!("{task}\n")).unwrap();
        git(&tree, &["commit", "-q", "-am", "b"]);
        tree
    }

    fn reasons(home: &Home) -> Vec<String> {
        swept(home)
            .into_iter()
            .map(|entry| entry.stays.unwrap_or_default())
            .collect()
    }

    /// What one sweep's thread says of each worktree, the one it would have checked first first.
    fn swept(home: &Home) -> Vec<Entry> {
        let mut jobs = jobs(&home.db, &home.factory, worktrees::found(&home.dir)).unwrap();
        let order: Vec<PathBuf> = {
            let tried = tried();
            jobs.sort_by_key(|job| tried.get(&job.found.path).copied());
            jobs.iter().map(|job| job.found.path.clone()).collect()
        };
        let judged = judge(&home.dir, &[], jobs);
        let mut entries = assess(&home.db, &home.dir, Judge::Answers(&judged.answers)).unwrap();
        entries.sort_by_key(|entry| order.iter().position(|path| *path == entry.found.path));
        entries
    }

    fn ignoring_target(name: &str) -> Home {
        let home = Home::new(name);
        std::fs::write(home.dir.join("repos/o/r/.git/info/exclude"), "target\n").unwrap();
        git(
            &home.dir.join("repos/o/r"),
            &["update-ref", "refs/remotes/origin/main", "HEAD"],
        );
        // An origin that has the default branch alone.
        git(
            &home.dir,
            &["clone", "-q", "--bare", "repos/o/r", "origin.git"],
        );
        let origin = home.dir.join("origin.git");
        git(
            &home.dir.join("repos/o/r"),
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        home
    }

    #[test]
    fn a_task_revived_and_dropped_during_the_check_keeps_its_pushed_worktree() {
        let mut home = ignoring_target("revived");
        let tree = unpushed(&home, 1);
        git(&tree, &["update-ref", "refs/remotes/origin/x", "HEAD"]);
        assert!(home.start_sweep());
        home.db
            .execute("UPDATE tasks SET state = 'dropped'", [])
            .unwrap();
        while !home.sweep_tick() {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(tree.exists());
        home.db
            .execute("UPDATE tasks SET state = 'done'", [])
            .unwrap();
        home.sweep();
        assert!(!tree.exists());
    }

    #[test]
    fn no_tick_of_a_sweep_takes_2_seconds_and_each_removes_one_worktree_at_most() {
        let mut home = ignoring_target("ticks");
        let trees: Vec<PathBuf> = (1..=25).map(|task| home.task(task, "done")).collect();
        for folder in 0..200 {
            let folder = trees[0].join(format!("target/debug/{folder}"));
            std::fs::create_dir_all(&folder).unwrap();
            for file in 0..50 {
                std::fs::write(folder.join(file.to_string()), "").unwrap();
            }
        }
        let left = |trees: &[PathBuf]| trees.iter().filter(|tree| tree.exists()).count();
        let mut hung: Option<(PathBuf, Vec<u8>)> = None;
        let mut longest = Duration::ZERO;
        // The second sweep's thread meets the hung git, and the third one none.
        for sweep in 1..=3 {
            if sweep == 3 {
                let (index, read) = hung.as_ref().expect("a check hung");
                std::fs::remove_file(index).unwrap();
                std::fs::write(index, read).unwrap();
            }
            assert!(home.start_sweep());
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                let before = left(&trees);
                let tick = Instant::now();
                let ended = home.sweep_tick();
                longest = longest.max(tick.elapsed());
                assert!(before <= left(&trees) + 1, "one worktree per tick");
                if ended {
                    break;
                }
                if hung.is_none() && left(&trees) == 20 {
                    let next = home.sweeping.next[0];
                    let name = &home.sweeping.entries[next].found.name;
                    // Git blocks on a fifo in place of the index, in the check of the moved folder.
                    let index = home
                        .dir
                        .join(format!("repos/o/r/.git/worktrees/{name}/index"));
                    let read = std::fs::read(&index).unwrap();
                    std::fs::remove_file(&index).unwrap();
                    runtime::run(Command::new("mkfifo").arg(&index)).unwrap();
                    hung = Some((index, read));
                }
                assert!(Instant::now() < deadline, "the sweep does not end");
                std::thread::sleep(Duration::from_millis(5));
            }
            if sweep < 3 {
                assert_eq!(left(&trees), 1, "the hung check keeps its own worktree");
            }
        }

        assert!(longest < Duration::from_secs(2), "{longest:?}");
        assert_eq!(left(&trees), 0);
    }

    #[test]
    fn a_worktree_whose_task_woke_after_the_walk_stays() {
        let mut home = ignoring_target("woke");
        let tree = home.task(1, "asleep");
        home.sweeping.retire(&home.db).unwrap();
        home.sweep_tick();
        let walk = &home.sweeping.retiring.as_ref().unwrap().walk;
        until(|| walk.thread.is_finished());
        home.db
            .execute("UPDATE tasks SET state = 'ready', reason = 'woke'", [])
            .unwrap();
        home.sweep_tick();
        assert!(tree.exists());
        let (reason, retired): (String, Option<String>) = home
            .db
            .query_row("SELECT reason, retired FROM tasks", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((reason.as_str(), retired), ("woke", None));

        // As one that an ended task's sweep judged.
        home.db
            .execute("UPDATE tasks SET state = 'done'", [])
            .unwrap();
        assert!(home.start_sweep());
        let judging = home.sweeping.judging.as_ref().unwrap();
        until(|| judging.thread.is_finished());
        home.sweep_tick();
        home.db
            .execute(
                "INSERT INTO runs (task, step, tmux, session) VALUES (1, 'code', 't', 's')",
                [],
            )
            .unwrap();
        while !home.sweep_tick() {}
        assert!(tree.exists());
    }

    #[test]
    fn a_retirement_that_hangs_leaves_the_next_task_its_turn() {
        let mut home = ignoring_target("hung-retirement");
        let trees = [home.task(1, "asleep"), home.task(2, "asleep")];
        let index = home.dir.join("repos/o/r/.git/worktrees/1/index");
        std::fs::remove_file(&index).unwrap();
        runtime::run(Command::new("mkfifo").arg(&index)).unwrap();

        home.retire();

        assert!(trees[0].exists() && !trees[1].exists());
    }

    #[test]
    fn a_task_whose_worktree_changed_as_it_moved_is_not_retired() {
        let mut home = ignoring_target("moved-changed");
        let tree = home.task(1, "asleep");
        home.sweeping.retire(&home.db).unwrap();
        home.sweep_tick();
        let walk = &home.sweeping.retiring.as_ref().unwrap().walk;
        until(|| walk.thread.is_finished());
        std::fs::write(tree.join("late"), "x").unwrap();
        home.sweep_tick();
        assert!(home.sweeping.deleting.is_some(), "the tick runs no git");
        while home.sweeping.busy() {
            home.sweep_tick();
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(tree.join("late").exists());
        let (reason, retired): (String, Option<String>) = home
            .db
            .query_row("SELECT reason, retired FROM tasks", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(
            (reason.as_str(), retired),
            ("cannot retire: untracked files", None)
        );
    }

    #[test]
    fn a_folder_that_took_a_worktrees_place_is_named_on_the_desk() {
        let mut home = ignoring_target("taken-place");
        let tree = home.task(1, "asleep");
        let found = worktrees::found(&home.dir).remove(0);
        let taken = worktrees::take(&home.dir, &found).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("theirs"), "x").unwrap();
        let dir = home.dir.clone();
        home.sweeping
            .delete(&dir, taken, found, None, false, Some(1));
        while home.sweeping.busy() {
            home.sweep_tick();
            std::thread::sleep(Duration::from_millis(5));
        }

        assert!(tree.join("a.txt").exists());
        let moved = "what took its place moved to the trash folder";
        let banner = moved_banner(&home.dir).unwrap();
        assert!(banner.contains(moved), "{banner}");
        let reason: String = home
            .db
            .query_row("SELECT reason FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert!(reason.contains(moved), "{reason}");
    }

    #[test]
    fn a_worktree_a_stopped_daemon_moved_goes_back_and_one_in_hand_waits() {
        let mut home = ignoring_target("moved");
        let tree = home.task(1, "done");
        let found = worktrees::found(&home.dir).remove(0);
        let taken = worktrees::take(&home.dir, &found).unwrap();
        assert!(leftovers(&home.dir).is_empty(), "a thread has it");
        move_back(&home.dir);
        assert!(!tree.exists());

        drop(taken);
        home.tick(true, false);
        assert!(tree.join("a.txt").exists());

        drop(worktrees::take(&home.dir, &found).unwrap());
        move_back(&home.dir);
        assert!(tree.join("a.txt").exists());
    }

    #[test]
    fn a_sweep_deletes_the_branch_the_last_one_had_no_time_for() {
        let mut home = ignoring_target("cut-branch");
        let tree = home.task(1, "done");
        home.task(2, "done");
        let mut all = worktrees::found(&home.dir);
        let (other, found) = (all.remove(1), all.remove(0));
        let clone = home.dir.join("repos/o/r");
        git(
            &clone,
            &["worktree", "remove", "--force", tree.to_str().unwrap()],
        );
        home.db
            .execute("UPDATE tasks SET state = 'ready' WHERE id = 2", [])
            .unwrap();
        home.sweeping.branches = [other, found.clone()].into();
        let branch = |task| runtime::head(&clone, &format!("priory/{task}"));

        home.sweep();

        assert!(home.sweeping.branches.is_empty());
        assert_eq!(branch(1), None);
        assert!(branch(2).is_some(), "its task woke");

        // A delete that hangs is tried at the sweep after it.
        let config = clone.join(".git/config");
        let read = std::fs::read(&config).unwrap();
        std::fs::remove_file(&config).unwrap();
        runtime::run(Command::new("mkfifo").arg(&config)).unwrap();
        home.sweeping.branches = [found].into();
        assert!(home.start_sweep());
        let deadline = Instant::now() + Duration::from_secs(150);
        while !home.sweep_tick() {
            assert!(Instant::now() < deadline, "the sweep does not end");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::remove_file(&config).unwrap();
        std::fs::write(&config, read).unwrap();
        assert_eq!(home.sweeping.branches.len(), 1);
    }

    #[test]
    fn a_repair_goes_with_its_run() {
        let mut sweeping = Sweeping::default();
        let short = || Ok(Some("disk full".to_owned()));
        assert!(sweeping.repairs(1, short).unwrap());
        assert!(sweeping.repairs(2, short).unwrap());
        sweeping.forget_repairs(|run| run == 2);
        assert_eq!(sweeping.repaired(1), None);
        assert!(sweeping.repaired(2).is_some());
    }

    #[test]
    fn a_repair_waits_for_a_sweep_that_starts_after_it_was_asked() {
        let mut home = ignoring_target("repair");
        home.task(1, "done");
        let short = || Ok(Some("disk full".to_owned()));
        assert!(!home.sweeping.repairs(1, || Ok(None)).unwrap());
        assert!(!home.sweeping.asked());
        assert!(home.start_sweep());

        assert!(home.sweeping.repairs(1, short).unwrap());
        while !home.sweep_tick() {}
        assert!(home.sweeping.asked(), "that sweep started before");
        assert!(home.sweeping.repairs(1, short).unwrap());
        home.sweep();

        assert!(!home.sweeping.asked());
        assert!(!home.sweeping.repairs(1, short).unwrap());

        // A sweep that hangs starts no other, so the repair waits 10 minutes at most.
        assert!(home.sweeping.repairs(2, short).unwrap());
        home.sweeping.repairs.get_mut(&2).unwrap().1 = Instant::now() - OVERDUE;
        assert!(!home.sweeping.repairs(2, short).unwrap());
        assert_eq!(home.sweeping.repaired(1).as_deref(), Some("disk full"));
    }

    #[test]
    fn a_sweep_scans_two_worktrees_and_the_next_one_the_rest() {
        let home = ignoring_target("scans");
        for task in 1..=3 {
            unpushed(&home, task);
        }
        let waits = "its patch check waits for the next sweep";
        assert_eq!(
            reasons(&home),
            ["unpushed commits", "unpushed commits", waits]
        );
        assert_eq!(
            reasons(&home),
            ["unpushed commits"; 3],
            "all cached or scanned"
        );

        // A new remote ref asks for every scan again, and the one scanned longest ago goes first.
        git(
            &home.dir.join("repos/o/r"),
            &["update-ref", "refs/remotes/origin/x", "HEAD"],
        );
        let order = swept(&home);
        let last = order[2].found.name.clone();
        assert_eq!(order[2].stays.as_deref(), Some(waits));
        let order = swept(&home);
        assert_eq!(order[0].found.name, last, "not starved");
        assert_eq!(order[0].stays.as_deref(), Some("unpushed commits"));
    }

    #[test]
    fn a_check_that_hangs_keeps_its_worktree_and_the_next_sweep_takes_it_last() {
        let home = ignoring_target("hung-check");
        let first = home.task(1, "done");
        home.task(2, "done");
        // Git blocks on a fifo in place of the index.
        let index = home.dir.join("repos/o/r/.git/worktrees/1/index");
        std::fs::remove_file(&index).unwrap();
        runtime::run(Command::new("mkfifo").arg(&index)).unwrap();
        let started = Instant::now();

        let kept = reasons(&home);

        assert!(kept[0].starts_with("cannot check: "), "{kept:?}");
        assert!(timed_out(&kept[0]), "{kept:?}");
        assert_eq!(kept[1], "its checks wait for the next sweep");
        assert!(started.elapsed() < Duration::from_secs(45));
        assert!(first.exists());
        let next = swept(&home);
        assert_eq!(next[0].found.name, "2");
        assert_eq!(next[0].stays, None);
    }

    #[test]
    fn a_fetch_that_hangs_leaves_the_other_clones_for_the_next_sweep() {
        let home = Home::new("hung-fetch");
        let config = home.dir.join("repos/o/r/.git/config");
        std::fs::remove_file(&config).unwrap();
        runtime::run(Command::new("mkfifo").arg(&config)).unwrap();
        let repos = ["o/r".to_owned(), "o/other".to_owned()];
        let started = Instant::now();

        let unfetched = fetch(&home.dir, &repos, jiff::Timestamp::now());

        assert_eq!(unfetched, repos);
        assert!(started.elapsed() < Duration::from_secs(45));
        assert!(
            failing(&home.dir, "o/r")
                .unwrap()
                .error
                .contains("timed out after 30s")
        );
        assert!(failing(&home.dir, "o/other").is_none(), "it was not tried");
    }

    #[test]
    fn old_evidence_moves_aside_for_the_reclaims_thread() {
        let mut home = Home::new("old-evidence");
        home.task(1, "done");
        let folder = home.dir.join("evidence/1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.png"), "x").unwrap();
        home.db
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 UPDATE tasks SET changed = '2020-01-01T00:00:00Z';
                 INSERT INTO evidence (task, run, kind, media, caption, commit_sha, bytes, sha256)
                 VALUES (1, 1, 'image', 'image/png', 'a', 'c', 1, 'd');",
            )
            .unwrap();

        crate::evidence::sweep(&home.db, &home.dir).unwrap();

        assert!(!folder.exists());
        assert_eq!(leftovers(&home.dir).len(), 1);
        let left: i64 = home
            .db
            .query_row("SELECT count(*) FROM evidence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
        home.tick(true, false);
        home.deleted();
        assert!(leftovers(&home.dir).is_empty());
    }

    #[test]
    fn the_walk_leaves_the_index_of_a_worktree_alone() {
        use std::os::unix::fs::MetadataExt;
        let mut home = Home::new("index");
        let tree = home.task(1, "asleep");
        let index = home.dir.join("repos/o/r/.git/worktrees/1/index");
        let before = std::fs::metadata(&index).unwrap().ino();
        // A new timestamp with the same content, which a plain status writes into the index.
        std::fs::File::open(tree.join("a.txt"))
            .and_then(|file| file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)))
            .unwrap();
        home.walk();
        assert_eq!(std::fs::metadata(&index).unwrap().ino(), before);
    }
}
