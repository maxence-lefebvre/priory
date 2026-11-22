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
        // With runners the folder is `<runner>-<task>`.
        let task = found
            .name
            .rsplit('-')
            .next()
            .and_then(|id| id.parse::<i64>().ok());
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

/// What starts the name of a replay's scratch folder in the temporary folder, before its pid.
pub(crate) const REPLAY: &str = "priory-replay-";

/// The name of a scratch folder that dies with this process: its pid, then a random part, short
/// so a tmux socket inside it stays under the 108-byte limit.
pub(crate) fn scratch_name() -> String {
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-{}", std::process::id(), &random[..12])
}

/// Removes what an interrupted `priory bench`, `priory score` or judge left: each folder under
/// `PRIORY_HOME/scores` and each replay folder under `tmp` whose process is gone, after it kills
/// the replay's own tmux server through that folder's socket. A process killed with Ctrl-C runs no
/// cleanup, so the next one does it.
pub fn leftovers(home: &Path, tmp: &Path) {
    let mut scored = false;
    for dir in dead(&home.join("scores"), "") {
        priory_runtime::runtime::remove_sealed(&dir);
        scored = true;
    }
    if scored {
        prune(&home.join("repos"));
    }
    for dir in dead(tmp, REPLAY) {
        let sockets = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("tmux-"))
            .flat_map(|entry| {
                std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            });
        for socket in sockets {
            let _ = std::process::Command::new("tmux")
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .arg("-S")
                .arg(socket.path())
                .arg("kill-server")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        priory_runtime::runtime::remove_sealed(&dir);
    }
}

/// The folders in `parent` named `<prefix><pid>-...` whose process `pid` no longer runs.
fn dead(parent: &Path, prefix: &str) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let pid = name
                .strip_prefix(prefix)
                .and_then(|rest| rest.split('-').next())
                .and_then(|pid| pid.parse::<u32>().ok());
            pid.is_some_and(|pid| pid != std::process::id() && !runs(pid))
        })
        .map(|entry| entry.path())
        .collect()
}

/// Whether process `pid` runs, as `kill -0` tells.
fn runs(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Forgets, in each clone under `repos`, the worktrees whose folder is gone.
fn prune(repos: &Path) {
    let clones = std::fs::read_dir(repos)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|owner| {
            std::fs::read_dir(owner.path())
                .into_iter()
                .flatten()
                .flatten()
        });
    for clone in clones {
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(clone.path())
            .args(["worktree", "prune"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Retires asleep tasks, longest asleep first, while `short` says the disk is short: removes each
/// clean worktree and keeps its branch and sessions. A task that cannot retire gets the reason once.
pub(crate) fn retire(
    db: &Connection,
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

/// Every worktree under `home` that stays: its path, its task when its name is one, and why.
pub fn stays(db: &Connection, home: &Path) -> rusqlite::Result<Vec<(String, Option<i64>, String)>> {
    Ok(assess(db, home)?
        .into_iter()
        .filter_map(|entry| {
            let reason = entry.stays?;
            let task = entry.found.name.parse().ok();
            Some((entry.found.path.display().to_string(), task, reason))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// A pid that no process has: a child that already ended.
    fn ended() -> u32 {
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn leftovers_of_an_ended_process_go_with_their_tmux_server_and_a_live_ones_stay() {
        let root = std::env::temp_dir().join(format!("priory-leftovers-{}", std::process::id()));
        priory_runtime::runtime::remove_sealed(&root);
        let (home, tmp) = (root.join("home"), root.join("tmp"));
        let gone = ended();
        let live = std::process::id();
        let scores = [
            home.join(format!("scores/{gone}-a")),
            home.join(format!("scores/{live}-b")),
        ];
        let replays = [
            tmp.join(format!("{REPLAY}{gone}-c")),
            tmp.join(format!("{REPLAY}{live}-d")),
        ];
        for dir in scores.iter().chain(&replays) {
            std::fs::create_dir_all(dir.join("x")).unwrap();
        }
        let socket = replays[0].join("tmux-0/default");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let tmux = |args: &[&str]| {
            Command::new("tmux")
                .arg("-S")
                .arg(&socket)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        };
        assert!(tmux(&["new-session", "-d", "sleep", "600"]));

        leftovers(&home, &tmp);

        let alive = tmux(&["has-session"]);
        if alive {
            tmux(&["kill-server"]);
        }
        assert!(!alive, "the ended replay's tmux server still runs");
        assert!(!scores[0].exists() && !replays[0].exists());
        assert!(scores[1].exists() && replays[1].exists());
        priory_runtime::runtime::remove_sealed(&root);
    }
}
