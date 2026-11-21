use crate::hold;
use priory_factory::factory::{After, Factory, Hold};
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

/// Records the files each task's branch changed at the end of its latest run, then holds each
/// queued or ready task that changes files an earlier, unfinished task changes, until that task's
/// pull request merges or, with none, the task ends. A person's release of the hold stays final for
/// that earlier task. Tasks in other repositories never overlap, and a task never waits on one that
/// waits on it.
pub(crate) fn tick(db: &mut Connection, factory: &Factory, home: &Path) -> rusqlite::Result<()> {
    if !factory.overlap() {
        return Ok(());
    }
    record(db, home)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let waiting: Vec<(i64, Vec<String>)> = tx
        .prepare(
            "SELECT id, files FROM tasks WHERE state IN ('queued', 'ready')
             AND json_array_length(files) > 0 ORDER BY id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?
        .map(|row| row.map(|(id, files)| (id, files_of(&files))))
        .collect::<rusqlite::Result<_>>()?;
    for (task, files) in waiting {
        let earlier: Vec<(i64, Vec<String>)> = tx
            .prepare(
                "SELECT id, files FROM tasks WHERE id < ?1 AND json_array_length(files) > 0
                 AND state NOT IN ('failed', 'dropped')
                 AND replace(worktree, '/' || id, '') = replace(
                   (SELECT worktree FROM tasks WHERE id = ?1), '/' || ?1, '')
                 AND NOT EXISTS (SELECT 1 FROM holds WHERE holds.task = tasks.id
                   AND holds.target = ?1 AND holds.released IS NULL)
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
            let said: bool = tx.query_row(
                "SELECT count(*) > 0 FROM holds WHERE task = ?1 AND reason LIKE ?2
                 AND (released IS NULL OR met_as = 'released by a person')",
                params![task, format!("overlaps task {other} on %")],
                |row| row.get(0),
            )?;
            if said {
                continue;
            }
            let Some(wait) = wait_for(&tx, other)? else {
                continue;
            };
            let held = Hold {
                reason: format!("overlaps task {other} on {}", shared.join(", ")),
                if_closed: wait.merge.as_ref().map(|_| "release".to_owned()),
                after: wait.after.into_iter().collect(),
                merge: wait.merge.into_iter().collect(),
                ..Hold::default()
            };
            hold::hold(&tx, task, &held, factory)?
                .map(drop)
                .unwrap_or_default();
            break;
        }
    }
    tx.commit()
}

struct Wait {
    after: Option<After>,
    merge: Option<String>,
}

/// What `other` ends or merges to get out of the way: its open pull request, else its end; `None`
/// when it is done with no pull request left open.
fn wait_for(tx: &rusqlite::Transaction, other: i64) -> rusqlite::Result<Option<Wait>> {
    let open: Option<String> = tx
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
    let ended: bool = tx.query_row(
        "SELECT state = 'done' FROM tasks WHERE id = ?1",
        [other],
        |row| row.get(0),
    )?;
    Ok((!ended).then_some(Wait {
        after: Some(After::Task(other)),
        merge: None,
    }))
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
