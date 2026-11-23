use crate::recipe::{self, Teardown};
use priory_factory::factory::Factory;
use priory_runtime::worktrees::{self, Found, Verdict};
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// A task folder under `PRIORY_HOME/worktrees` and why it stays, or `None` when it can go.
/// `retire` keeps the branch when the folder goes.
struct Entry {
    found: Found,
    stays: Option<String>,
    retire: bool,
}

/// The task a worktree folder is named after: `<task>`, or `<runner>-<task>` with runners.
fn task_of(found: &Found) -> Option<i64> {
    found.name.rsplit('-').next().and_then(|id| id.parse().ok())
}

fn assess(db: &Connection, home: &Path) -> rusqlite::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for found in worktrees::found(home) {
        let mut retire = false;
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
                let verdict = worktrees::verdict(home, &found, head.as_deref(), state == "done");
                retire = verdict == Verdict::Retire;
                match verdict {
                    Verdict::Keep(stays) => Some(stays),
                    Verdict::Remove | Verdict::Retire => {
                        match task_of(&found).map(|id| recipe::teardown_of(db, id)) {
                            Some(found) => match found? {
                                Some(Teardown::Runs) => Some("teardown runs".to_owned()),
                                Some(Teardown::Failed(reason)) => Some(reason),
                                Some(Teardown::Done) | None => None,
                            },
                            None => None,
                        }
                    }
                }
            }
        };
        entries.push(Entry {
            found,
            stays,
            retire,
        });
    }
    Ok(entries)
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
    let entries = assess(db, home)?;
    measure(home, &entries);
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

fn sizes_file(home: &Path) -> PathBuf {
    home.join("sweep").join("sizes.json")
}

/// Records the size of each folder that stays, in bytes, for the desk to read.
fn measure(home: &Path, entries: &[Entry]) {
    let sizes: std::collections::BTreeMap<String, u64> = entries
        .iter()
        .filter(|entry| entry.stays.is_some())
        .map(|entry| {
            let path = entry.found.path.display().to_string();
            (path, worktrees::size(&entry.found.path))
        })
        .collect();
    let file = sizes_file(home);
    let written = std::fs::create_dir_all(file.parent().expect("a file has a folder"))
        .and_then(|()| std::fs::write(&file, serde_json::to_vec(&sizes).expect("serializes")));
    if let Err(written) = written {
        eprintln!("priory daemon: recording {}: {written}", file.display());
    }
}

/// `bytes` in the largest unit that keeps it at 1 or more, such as `7.4 GB`.
fn human(bytes: u64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} TB")
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
/// the size the last sweep measured.
pub fn stays(db: &Connection, home: &Path) -> rusqlite::Result<Vec<(String, Option<i64>, String)>> {
    let sizes: std::collections::HashMap<String, u64> = std::fs::read(sizes_file(home))
        .ok()
        .and_then(|read| serde_json::from_slice(&read).ok())
        .unwrap_or_default();
    Ok(assess(db, home)?
        .into_iter()
        .filter_map(|entry| {
            let reason = entry.stays?;
            let task = entry.found.name.parse().ok();
            let path = entry.found.path.display().to_string();
            let reason = match sizes.get(&path) {
                Some(&bytes) => format!("{reason}, {}", human(bytes)),
                None => reason,
            };
            Some((path, task, reason))
        })
        .collect())
}
