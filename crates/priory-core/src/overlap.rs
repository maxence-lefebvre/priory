use crate::hold;
use priory_factory::factory::{After, Factory, Hold};
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

/// Records the files each task's branch changed at the end of its latest run, then holds each
/// queued or ready task that changes files an earlier, unfinished task changes, until that task's
/// pull request merges or, with none, the task ends. A task that already has work on an open pull
/// request is never held. A person's release of the hold stays final for that earlier task. Tasks in
/// other repositories never overlap, a task never waits on one that waits on it, and a stacked batch
/// entry never waits on a task it is stacked on.
pub(crate) fn tick(db: &mut Connection, factory: &Factory, home: &Path) -> rusqlite::Result<()> {
    if !factory.overlap() {
        return Ok(());
    }
    record(db, home)?;
    let mut fresh = BTreeSet::new();
    for (task, _) in overlaps(db)? {
        if !works_on_open(db, home, task)? {
            fresh.insert(task);
        }
    }
    if fresh.is_empty() {
        return Ok(());
    }
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Read again: the state may have moved while git ran.
    for (task, held) in overlaps(&tx)? {
        if fresh.contains(&task) {
            hold::hold(&tx, task, &held, factory)?
                .map(drop)
                .unwrap_or_default();
        }
    }
    tx.commit()
}

/// The hold each queued or ready task with no pull request of its own would get, by task. A hold
/// only targets a lower id, so placing one changes no other.
fn overlaps(db: &Connection) -> rusqlite::Result<Vec<(i64, Hold)>> {
    let waiting: Vec<(i64, Vec<String>)> = db
        .prepare(
            "SELECT id, files FROM tasks WHERE state IN ('queued', 'ready')
             AND json_array_length(files) > 0
             AND NOT EXISTS (SELECT 1 FROM pull_requests WHERE state = 'open'
               AND (task = tasks.id OR tasks.key IN (resource, 'cli:' || resource)))
             ORDER BY id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?
        .map(|row| row.map(|(id, files)| (id, files_of(&files))))
        .collect::<rusqlite::Result<_>>()?;
    let mut holds = Vec::new();
    for (task, files) in waiting {
        let earlier: Vec<(i64, Vec<String>)> = db
            .prepare(
                "SELECT id, files FROM tasks WHERE id < ?1 AND json_array_length(files) > 0
                 AND state NOT IN ('failed', 'dropped')
                 AND replace(worktree, '/' || id, '') = replace(
                   (SELECT worktree FROM tasks WHERE id = ?1), '/' || ?1, '')
                 AND NOT EXISTS (SELECT 1 FROM holds WHERE holds.task = tasks.id
                   AND holds.target = ?1 AND holds.released IS NULL)
                 -- A stacked entry's branch starts with the files of every task it is stacked on.
                 AND id NOT IN (WITH RECURSIVE below (id) AS (
                     SELECT base_task FROM tasks WHERE id = ?1
                     UNION SELECT tasks.base_task FROM tasks JOIN below ON tasks.id = below.id
                   ) SELECT id FROM below WHERE id IS NOT NULL)
                 ORDER BY id",
            )?
            .query_map([task], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?
            .map(|row| row.map(|(id, files)| (id, files_of(&files))))
            .collect::<rusqlite::Result<_>>()?;
        for (other, theirs) in earlier {
            let shared: Vec<&str> = files
                .iter()
                .filter(|file| theirs.contains(file))
                .map(String::as_str)
                .collect();
            if shared.is_empty() {
                continue;
            }
            let said: bool = db.query_row(
                "SELECT count(*) > 0 FROM holds WHERE task = ?1 AND reason LIKE ?2
                 AND (released IS NULL OR met_as = 'released by a person')",
                params![task, format!("overlaps task {other} on %")],
                |row| row.get(0),
            )?;
            if said {
                continue;
            }
            let Some(wait) = wait_for(db, other)? else {
                continue;
            };
            let held = Hold {
                reason: format!("overlaps task {other} on {}", shared.join(", ")),
                if_closed: wait.merge.as_ref().map(|_| "release".to_owned()),
                after: wait.after.into_iter().collect(),
                merge: wait.merge.into_iter().collect(),
                ..Hold::default()
            };
            holds.push((task, held));
            break;
        }
    }
    Ok(holds)
}

struct Wait {
    after: Option<After>,
    merge: Option<String>,
}

/// What `other` ends or merges to get out of the way: its open pull request, else its end; `None`
/// when it is done with no pull request left open.
fn wait_for(db: &Connection, other: i64) -> rusqlite::Result<Option<Wait>> {
    let open: Option<String> = db
        .query_row(
            "SELECT resource FROM pull_requests WHERE task = ?1 AND state = 'open' ORDER BY created LIMIT 1",
            [other],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(resource) = open {
        return Ok(Some(Wait {
            after: None,
            merge: Some(resource.trim_start_matches("github:").to_owned()),
        }));
    }
    let ended: bool = db.query_row(
        "SELECT state = 'done' FROM tasks WHERE id = ?1",
        [other],
        |row| row.get(0),
    )?;
    Ok((!ended).then_some(Wait {
        after: Some(After::Task(other)),
        merge: None,
    }))
}

/// A task, the run whose end recorded its head, and the pushed branch that head is in line with.
type InLine = (i64, Option<i64>, String);

/// Only a yes is kept: a no places a hold, and a held task is not asked about.
static IN_LINE: Mutex<BTreeSet<InLine>> = Mutex::new(BTreeSet::new());

fn known() -> std::sync::MutexGuard<'static, BTreeSet<InLine>> {
    IN_LINE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether `task`'s branch is in line with the branch of an open pull request of its repository,
/// other than one a stacked entry is stacked on: that entry is new work on top.
fn works_on_open(db: &Connection, home: &Path, task: i64) -> rusqlite::Result<bool> {
    let (worktree, branch, run): (Option<String>, Option<String>, Option<i64>) = db.query_row(
        "SELECT worktree, branch, files_run FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let (Some(worktree), Some(branch)) = (worktree, branch) else {
        return Ok(false);
    };
    let worktree = Path::new(&worktree);
    let Some(repo) = worktree
        .parent()
        .and_then(|repo| repo.strip_prefix(home.join("worktrees")).ok())
    else {
        return Ok(false);
    };
    let open: Vec<String> = db
        .prepare(
            "SELECT DISTINCT branch FROM pull_requests WHERE state = 'open'
             AND substr(resource, 1, instr(resource, '#') - 1) = ?2
             AND task NOT IN (WITH RECURSIVE below (id) AS (
                 SELECT base_task FROM tasks WHERE id = ?1
                 UNION SELECT tasks.base_task FROM tasks JOIN below ON tasks.id = below.id
               ) SELECT id FROM below WHERE id IS NOT NULL)",
        )?
        .query_map(params![task, format!("github:{}", repo.display())], |row| {
            row.get(0)
        })?
        .collect::<rusqlite::Result<_>>()?;
    let entry = |pushed: &str| (task, run, pushed.to_owned());
    if open.iter().any(|pushed| known().contains(&entry(pushed))) {
        return Ok(true);
    }
    if open.is_empty() {
        return Ok(false);
    }
    let found: Vec<InLine> = runtime::in_line(home, worktree, &branch)
        .iter()
        .filter(|pushed| open.contains(pushed))
        .map(|pushed| entry(pushed))
        .collect();
    let works = !found.is_empty();
    let mut known = known();
    known.retain(|(other, at, _)| *other != task || *at == run);
    known.extend(found);
    Ok(works)
}

fn files_of(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Stores the changed files of each task whose latest ended run is newer than its stored ones.
fn record(db: &Connection, home: &Path) -> rusqlite::Result<()> {
    let stale: Vec<(i64, String, String, i64)> = db
        .prepare(
            "SELECT tasks.id, tasks.worktree, tasks.branch, max(runs.id) FROM tasks
             JOIN runs ON runs.task = tasks.id AND runs.ended IS NOT NULL
             WHERE tasks.worktree IS NOT NULL AND tasks.branch IS NOT NULL
             GROUP BY tasks.id HAVING max(runs.id) > coalesce(tasks.files_run, 0)",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (task, worktree, branch, run) in stale {
        let Some(files) = runtime::changed_files(home, Path::new(&worktree), &branch) else {
            continue;
        };
        db.execute(
            "UPDATE tasks SET files = ?2, files_run = ?3 WHERE id = ?1",
            params![
                task,
                serde_json::to_string(&files).expect("names serialize"),
                run
            ],
        )?;
    }
    Ok(())
}
