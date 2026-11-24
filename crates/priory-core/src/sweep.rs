use crate::recipe::{self, Teardown};
use crate::trace::{self, Trace};
use priory_factory::factory::Factory;
use priory_runtime::runtime::remove_sealed;
use priory_runtime::worktrees::{self, Found, Verdict};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// A task folder under `PRIORY_HOME/worktrees` and why it stays, or `None` when it can go.
/// `retire` keeps the branch when the folder goes. `judged` is the reason git gave, which the
/// sweep records for the desk.
struct Entry {
    found: Found,
    stays: Option<String>,
    retire: bool,
    judged: Option<String>,
}

/// How `assess` learns why git keeps the worktree of an ended task.
enum Judge<'a> {
    /// It asks git, as the sweep does.
    Git,
    /// It reads what the last sweep recorded, as the desk does.
    Recorded(&'a Record),
}

/// The task a worktree folder is named after: `<task>`, or `<runner>-<task>` with runners.
fn task_of(found: &Found) -> Option<i64> {
    found.name.rsplit('-').next().and_then(|id| id.parse().ok())
}

fn assess(db: &Connection, home: &Path, judge: Judge) -> rusqlite::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for found in worktrees::found(home) {
        let mut retire = false;
        let mut judged = None;
        let task = task_of(&found);
        let task = task
            .map(|id| {
                db.query_row(
                    "SELECT state, worktree FROM tasks WHERE id = ?1",
                    [id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()
            })
            .transpose()?
            .flatten();
        let stays = match task.as_ref().map(|(state, _)| state.as_str()) {
            None => Some(format!("no task {}", found.name)),
            Some(_)
                if task.as_ref().and_then(|(_, tree)| tree.as_deref()) != found.path.to_str() =>
            {
                Some(format!(
                    "not recorded as the worktree of task {}",
                    found.name
                ))
            }
            Some(state @ ("queued" | "held" | "ready" | "running" | "asleep" | "blocked")) => {
                Some(format!("kept while task {} is {state}", found.name))
            }
            Some(state) => {
                let verdict = match judge {
                    Judge::Git => verdict(db, home, &found, state == "done")?,
                    Judge::Recorded(record) => record
                        .get(&found.path.display().to_string())
                        .and_then(|kept| kept.reason.clone())
                        .map_or(Verdict::Remove, Verdict::Keep),
                };
                let teardown = || match task_of(&found).map(|id| recipe::teardown_of(db, id)) {
                    Some(found) => Ok(match found? {
                        Some(Teardown::Runs) => Some("teardown runs".to_owned()),
                        Some(Teardown::Failed(reason)) => Some(reason),
                        Some(Teardown::Done) | None => None,
                    }),
                    None => Ok::<_, rusqlite::Error>(None),
                };
                match verdict {
                    Verdict::Keep(stays) => {
                        judged = Some(stays.clone());
                        Some(stays)
                    }
                    Verdict::Remove => teardown()?,
                    Verdict::Retire => {
                        retire = true;
                        teardown()?
                    }
                }
            }
        };
        entries.push(Entry {
            found,
            stays,
            retire,
            judged,
        });
    }
    Ok(entries)
}

/// What git says of the worktree of an ended task. A squash merge leaves the head of the task's
/// merged pull request out of the default branch, so the inbox names it.
fn verdict(db: &Connection, home: &Path, found: &Found, done: bool) -> rusqlite::Result<Verdict> {
    let head: Option<String> = db
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
    Ok(worktrees::verdict(home, found, head.as_deref(), done))
}

/// Removes the worktree and branch of every ended task that nothing keeps, after fetching each
/// clone (see `fetch`), and after the repository's teardown succeeded in it. A teardown owed here
/// starts at the daemon's next tick.
pub fn run(db: &Connection, factory: &Factory, home: &Path) -> rusqlite::Result<()> {
    crate::evidence::sweep(db, home)?;
    let mut repos: Vec<_> = worktrees::found(home)
        .into_iter()
        .map(|found| found.repo)
        .collect();
    repos.dedup();
    let unfetched = fetch(home, &repos, jiff::Timestamp::now());
    let entries = assess(db, home, Judge::Git)?;
    record(home, &entries);
    for entry in entries {
        if unfetched.contains(&entry.found.repo) {
            continue;
        }
        // A teardown that failed stays failed until the factory's teardown changes.
        let torn = match (&entry.stays, task_of(&entry.found)) {
            (None, Some(task)) => {
                recipe::teardown(db, factory, task, &entry.found.repo, &entry.found.path)?
            }
            (Some(_), Some(task))
                if matches!(recipe::teardown_of(db, task)?, Some(Teardown::Failed(_))) =>
            {
                recipe::teardown(db, factory, task, &entry.found.repo, &entry.found.path)?;
                continue;
            }
            _ => continue,
        };
        if !matches!(torn, Teardown::Done) {
            continue;
        }
        let removed = if entry.retire {
            worktrees::retire(home, &entry.found)
        } else {
            worktrees::remove(home, &entry.found)
        };
        if let Err(error) = removed {
            eprintln!(
                "priory daemon: removing {}: {error}",
                entry.found.path.display()
            );
        }
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

/// Fetches the clone of each of `repos` and returns those it did not fetch. A clone whose fetch
/// failed is logged once, kept as failing, and fetched again only an hour after its last try. A
/// failure of a repository with no worktree left is forgotten.
fn fetch(home: &Path, repos: &[String], now: jiff::Timestamp) -> Vec<String> {
    let now = jiff::Timestamp::from_second(now.as_second()).unwrap_or(now);
    let mut unfetched = Vec::new();
    for repo in repos {
        let failed = failing(home, repo);
        if failed
            .as_ref()
            .is_some_and(|failed| failed.tried + RETRY > now)
        {
            unfetched.push(repo.clone());
            continue;
        }
        let file = failing_file(home, repo);
        match worktrees::fetch(home, repo) {
            Ok(()) if failed.is_some() => {
                let _ = std::fs::remove_file(&file);
                eprintln!("priory daemon: fetching {repo} works again");
            }
            Ok(()) => {}
            Err(error) => {
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
}

/// A thread that deletes what `home/reclaim` holds, the `output` moved there included, if any. A
/// daemon that stopped during a delete leaves files there.
struct Delete {
    output: Option<Output>,
    thread: JoinHandle<Result<(), String>>,
}

impl Reclaim {
    /// With `due`, starts a walk when none runs and nothing waits. `under` says free disk is
    /// under the threshold. Each deletion is traced once it ends.
    pub(crate) fn tick(
        &mut self,
        db: &mut Connection,
        factory: &Factory,
        home: &Path,
        trace: &Trace,
        due: bool,
        mut under: impl FnMut() -> bool,
    ) -> rusqlite::Result<()> {
        if let Some(delete) = self.delete.take_if(|delete| delete.thread.is_finished()) {
            let deleted = delete
                .thread
                .join()
                .unwrap_or_else(|_| Err("a delete thread panicked".to_owned()));
            match (deleted, delete.output) {
                (Err(error), _) => eprintln!("priory daemon: reclaiming build output: {error}"),
                (Ok(()), Some(output)) => {
                    let tx = db.transaction()?;
                    trace::worktree_reclaimed(&tx, trace, output.task, &output.path, output.bytes)?;
                    tx.commit()?;
                }
                (Ok(()), None) => {}
            }
        }
        if let Some(walk) = self.walk.take_if(|walk| walk.is_finished()) {
            self.pending = walk.join().unwrap_or_default().into();
        }
        if self.walk.is_some() || self.delete.is_some() {
            return Ok(());
        }
        let Some(output) = self.pending.pop_front() else {
            if due && set_aside(home).next().is_some() {
                self.delete = Some(Delete {
                    output: None,
                    thread: delete(home),
                });
            }
            if due && under() {
                self.walk = Some(walk(home, idle_build(db, factory, home)?));
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
        let aside = home.join(RECLAIM).join(name);
        let Output {
            found, build, path, ..
        } = &output;
        if let Err(error) = worktrees::set_aside(home, found, build, path, &aside) {
            eprintln!("priory daemon: task {}: {error}", output.task);
            return Ok(());
        }
        self.delete = Some(Delete {
            output: Some(output),
            thread: delete(home),
        });
        Ok(())
    }
}

/// The folder of `home` a build folder moves to before its delete.
const RECLAIM: &str = "reclaim";

/// What waits under `home/reclaim` for its delete.
fn set_aside(home: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(home.join(RECLAIM))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
}

/// Deletes all of `home/reclaim` on a thread.
fn delete(home: &Path) -> JoinHandle<Result<(), String>> {
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        for path in set_aside(&home) {
            remove_sealed(&path);
        }
        match set_aside(&home).next() {
            Some(left) => Err(format!("cannot delete {}", left.display())),
            None => Ok(()),
        }
    })
}

/// The idle tasks and their worktree paths, longest idle first.
fn idle(db: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    db.prepare(
        "SELECT id, worktree FROM tasks
         WHERE state IN ('done', 'dropped', 'held', 'asleep') AND worktree IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id AND ended IS NULL)
           AND NOT EXISTS (SELECT 1 FROM grants WHERE grants.task = tasks.id)
         ORDER BY changed, id",
    )?
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
fn walk(home: &Path, worktrees: Vec<(i64, Found, Vec<String>)>) -> JoinHandle<Vec<Output>> {
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        let mut outputs = Vec::new();
        for (task, found, build) in worktrees {
            let Ok(folders) = worktrees::build_output(&home, &found, &build) else {
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

/// Retires asleep tasks, longest asleep first, while `short` says the disk is short: removes each
/// clean worktree and keeps its branch and sessions. A task that cannot retire gets the reason once.
pub(crate) fn retire(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    mut short: impl FnMut() -> bool,
) -> rusqlite::Result<()> {
    if !short() {
        return Ok(());
    }
    let asleep = db
        .prepare(
            "SELECT id, worktree FROM tasks
             WHERE state = 'asleep' AND worktree IS NOT NULL ORDER BY changed, id",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let found = worktrees::found(home);
    for (task, worktree) in asleep {
        let Some(found) = found
            .iter()
            .find(|found| found.path.to_str() == Some(&worktree) && found.name == task.to_string())
        else {
            continue;
        };
        // Marked first, so a crash before the removal leaves a mark on a worktree, not the reverse.
        let mut stays = worktrees::retire_blocker(home, found);
        if stays.is_none() {
            // One teardown at a time: the next task's waits until this one retired and the disk
            // is measured again.
            match recipe::teardown(db, factory, task, &found.repo, &found.path)? {
                Teardown::Done => {}
                Teardown::Runs => return Ok(()),
                Teardown::Failed(reason) => stays = Some(reason),
            }
        }
        if stays.is_none() {
            db.execute(
                "UPDATE tasks SET retired = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), reason = NULL
                 WHERE id = ?1",
                [task],
            )?;
            stays = worktrees::retire(home, found).err();
            if stays.is_some() {
                db.execute("UPDATE tasks SET retired = NULL WHERE id = ?1", [task])?;
            }
        }
        let Some(stays) = stays else {
            if short() {
                continue;
            }
            return Ok(());
        };
        let reason = format!("cannot retire: {stays}");
        let reported = db.execute(
            "UPDATE tasks SET reason = ?2 WHERE id = ?1 AND reason IS NOT ?2",
            rusqlite::params![task, reason],
        )?;
        if reported > 0 {
            eprintln!("priory daemon: task {task} {reason}");
        }
    }
    Ok(())
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
    use priory_runtime::runtime;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// A home with a clone of `o/r` and the worktree of each task it adds, removed on drop.
    struct Home {
        dir: PathBuf,
        db: Connection,
        factory: Factory,
        reclaim: Reclaim,
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

        fn tick(&mut self, due: bool, under: bool) {
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
        assert_eq!(set_aside(&home.dir).count(), 0);
        home.tick(false, true);
        assert_eq!(home.traced(), 1);
        assert!(tree.join("a.txt").exists());
    }

    #[test]
    fn reclaim_clears_what_a_stopped_daemon_left_even_with_room() {
        let mut home = Home::new("left");
        std::fs::create_dir_all(home.dir.join("reclaim/7-1/debug")).unwrap();
        home.tick(true, false);
        home.deleted();
        home.tick(false, false);
        assert_eq!(set_aside(&home.dir).count(), 0);
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
}
