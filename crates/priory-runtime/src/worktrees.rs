use crate::runtime::{clone_of, git, run};
use std::fs;
use std::path::{Path, PathBuf};

/// A folder under `PRIORY_HOME/worktrees/<owner>/<name>/`, named for a task.
pub struct Found {
    pub path: PathBuf,
    pub repo: String,
    pub name: String,
}

impl Found {
    fn clone(&self, home: &Path) -> PathBuf {
        home.join("repos").join(&self.repo)
    }

    fn branch(&self) -> String {
        format!("priory/{}", self.name)
    }
}

/// Every task folder under `home/worktrees`.
pub fn found(home: &Path) -> Vec<Found> {
    let dirs = |path: &Path| -> Vec<(String, PathBuf)> {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .collect()
    };
    let mut all = Vec::new();
    for (owner, owner_path) in dirs(&home.join("worktrees")) {
        for (name, repo_path) in dirs(&owner_path) {
            for (task, path) in dirs(&repo_path) {
                all.push(Found {
                    path,
                    repo: format!("{owner}/{name}"),
                    name: task,
                });
            }
        }
    }
    all.sort_by(|a, b| a.path.cmp(&b.path));
    all
}

/// Fetches `origin` into Priory's clone of `repo` and refreshes `origin/HEAD`.
pub fn fetch(home: &Path, repo: &str) -> Result<(), String> {
    let clone = home.join("repos").join(repo);
    run(git(&clone).args(["fetch", "--quiet", "origin"]))?;
    run(git(&clone).args(["remote", "set-head", "origin", "--auto"])).map(drop)
}

/// Why the worktree must stay, or `None` when it can go. `merged_head` is the head commit of the
/// task's merged pull request, which a squash merge leaves out of the default branch. A check that
/// cannot run keeps the worktree.
pub fn blocker(home: &Path, found: &Found, merged_head: Option<&str>) -> Option<String> {
    blocked(home, found, merged_head).unwrap_or_else(|error| Some(format!("cannot check: {error}")))
}

fn blocked(
    home: &Path,
    found: &Found,
    merged_head: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(reason) = unsettled(home, found)? {
        return Ok(Some(reason));
    }
    let path = &found.path;
    let tip = run(git(path).args(["rev-parse", "HEAD"]))?;
    let tip = tip.trim();
    let merged = run(git(path).args(["merge-base", "--is-ancestor", tip, "origin/HEAD"])).is_ok();
    if merged || merged_head == Some(tip) {
        return Ok(None);
    }
    let unpushed = run(git(path).args(["rev-list", "-n1", tip, "--not", "--remotes"]))?;
    Ok(Some(
        if unpushed.is_empty() {
            "not merged"
        } else {
            "unpushed commits"
        }
        .to_owned(),
    ))
}

/// Why the worktree of an asleep task must stay, or `None` when it can retire. Retiring keeps the
/// branch in the clone, so its commits need not be pushed.
pub fn retire_blocker(home: &Path, found: &Found) -> Option<String> {
    unsettled(home, found).unwrap_or_else(|error| Some(format!("cannot check: {error}")))
}

/// Removes the worktree and keeps its branch. Git refuses when anything is uncommitted or untracked.
pub fn retire(home: &Path, found: &Found) -> Result<(), String> {
    run(git(&found.clone(home))
        .args(["worktree", "remove"])
        .arg(&found.path))
    .map(drop)
}

/// Why removing the worktree would lose a file or a process's folder: everything else is on its branch.
fn unsettled(home: &Path, found: &Found) -> Result<Option<String>, String> {
    let clone = found.clone(home);
    let path = &found.path;
    let ours =
        clone_of(path).and_then(|of| fs::canonicalize(of).ok()) == fs::canonicalize(&clone).ok();
    if !ours {
        return Ok(Some(format!("not a worktree of {}", clone.display())));
    }
    let branch = found.branch();
    if run(git(path).args(["branch", "--show-current"]))?.trim() != branch {
        return Ok(Some(format!("not on branch {branch}")));
    }
    let status = run(git(path).args([
        "status",
        "--porcelain",
        "--untracked-files=all",
        "--ignore-submodules=none",
    ]))?;
    if status.lines().any(|line| !line.starts_with("??")) {
        return Ok(Some("uncommitted changes".to_owned()));
    }
    if !status.is_empty() {
        return Ok(Some("untracked files".to_owned()));
    }
    if occupied(path) {
        return Ok(Some("a process runs in it".to_owned()));
    }
    Ok(None)
}

/// Whether a process has its working directory inside `path`.
fn occupied(path: &Path) -> bool {
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .any(|process| {
            fs::read_link(process.path().join("cwd")).is_ok_and(|cwd| cwd.starts_with(&path))
        })
}

/// Removes the worktree and its branch. Git refuses both when anything is left to lose.
pub fn remove(home: &Path, found: &Found) -> Result<(), String> {
    retire(home, found)?;
    let clone = found.clone(home);
    let branch = found.branch();
    // A task branch tracks nothing, so `-d` alone would judge the merge against the clone's HEAD.
    let merged =
        run(git(&clone).args(["merge-base", "--is-ancestor", &branch, "origin/HEAD"])).is_ok();
    let delete = if merged { "-D" } else { "-d" };
    run(git(&clone).args(["branch", delete, &branch])).map(drop)
}
