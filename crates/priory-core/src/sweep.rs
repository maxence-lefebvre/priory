use priory_runtime::worktrees::{self, Found};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

/// A task folder under `PRIORY_HOME/worktrees` and why it stays, or `None` when it can go.
struct Entry {
    found: Found,
    stays: Option<String>,
}

fn assess(db: &Connection, home: &Path) -> rusqlite::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for found in worktrees::found(home) {
        let task = found.name.parse::<i64>().ok();
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
                Some(format!("task {} still {state}", found.name))
            }
            Some(_) => {
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
                worktrees::blocker(home, &found, head.as_deref())
            }
        };
        entries.push(Entry { found, stays });
    }
    Ok(entries)
}

/// Removes the worktree and branch of every ended task that nothing keeps, after fetching each clone.
pub fn run(db: &Connection, home: &Path) -> rusqlite::Result<()> {
    let mut repos: Vec<_> = worktrees::found(home)
        .into_iter()
        .map(|found| found.repo)
        .collect();
    repos.dedup();
    let mut unfetched = Vec::new();
    for repo in repos {
        if let Err(error) = worktrees::fetch(home, &repo) {
            eprintln!("priory daemon: fetching {repo}: {error}");
            unfetched.push(repo);
        }
    }
    for entry in assess(db, home)?
        .into_iter()
        .filter(|entry| entry.stays.is_none() && !unfetched.contains(&entry.found.repo))
    {
        if let Err(error) = worktrees::remove(home, &entry.found) {
            eprintln!(
                "priory daemon: removing {}: {error}",
                entry.found.path.display()
            );
        }
    }
    Ok(())
}

/// Prints a `worktree` row of the desk for every worktree that stays, with the reason.
pub fn print_stays(db: &Connection, home: &Path) -> rusqlite::Result<()> {
    for entry in assess(db, home)? {
        if let Some(reason) = entry.stays {
            let task = entry
                .found
                .name
                .parse::<i64>()
                .map_or("-".to_owned(), |id| id.to_string());
            crate::state::emit(&format!(
                "-\tworktree\t{}\t{task}\t-\t{reason}",
                entry.found.path.display()
            ));
        }
    }
    Ok(())
}
