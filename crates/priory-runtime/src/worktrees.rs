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

/// Fetches `origin` into Priory's clone of `repo` and moves the clone's branch up to
/// `origin/HEAD`, so that `git branch -d` sees what merged.
pub fn fetch(home: &Path, repo: &str) -> Result<(), String> {
    let clone = home.join("repos").join(repo);
    run(git(&clone).args(["fetch", "--quiet", "origin"]))?;
    if run(git(&clone).args(["merge-base", "--is-ancestor", "HEAD", "origin/HEAD"])).is_ok() {
        run(git(&clone).args(["update-ref", "HEAD", "origin/HEAD"]))?;
    }
    Ok(())
}

/// Why the worktree must stay, or `None` when it can go. `merged_head` is the head commit of the
/// task's merged pull request, which a squash merge leaves out of the default branch.
pub fn blocker(home: &Path, found: &Found, merged_head: Option<&str>) -> Option<String> {
    let clone = found.clone(home);
    let path = &found.path;
    let ours =
        clone_of(path).and_then(|of| fs::canonicalize(of).ok()) == fs::canonicalize(&clone).ok();
    if !ours {
        return Some(format!("not a worktree of {}", clone.display()));
    }
    let branch = found.branch();
    let on = run(git(path).args(["branch", "--show-current"])).ok()?;
    if on.trim() != branch {
        return Some(format!("not on branch {branch}"));
    }
    let status = run(git(path).args(["status", "--porcelain"])).ok()?;
    if status.lines().any(|line| !line.starts_with("??")) {
        return Some("uncommitted changes".to_owned());
    }
    if !status.is_empty() {
        return Some("untracked files".to_owned());
    }
    if occupied(path) {
        return Some("a process runs in it".to_owned());
    }
    let tip = run(git(path).args(["rev-parse", "HEAD"])).ok()?;
    let tip = tip.trim();
    let merged = run(git(path).args(["merge-base", "--is-ancestor", tip, "origin/HEAD"])).is_ok();
    if merged || merged_head == Some(tip) {
        return None;
    }
    let unpushed = run(git(path).args(["rev-list", "-n1", tip, "--not", "--remotes"])).ok()?;
    Some(
        if unpushed.is_empty() {
            "not merged"
        } else {
            "unpushed commits"
        }
        .to_owned(),
    )
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
    let clone = found.clone(home);
    run(git(&clone).args(["worktree", "remove"]).arg(&found.path))?;
    run(git(&clone).args(["branch", "-d", &found.branch()])).map(drop)
}
