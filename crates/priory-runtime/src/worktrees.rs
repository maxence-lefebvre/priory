use crate::runtime::{OWN_GIT, SCAN, at_risk, clone_of, git, run, unseal};
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

/// What the sweep does with the worktree of an ended task.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// The branch merged: the worktree and the branch go.
    Remove,
    /// Every commit is on a remote: the worktree goes and the branch stays for a revive.
    Retire,
    /// The worktree stays, for this reason.
    Keep(String),
}

/// What the sweep does with the worktree. `merged_head` is the head commit of the task's merged
/// pull request, which a squash merge leaves out of the default branch. `done` retires a worktree
/// whose commits are all pushed, or patch-equivalent to pushed ones. A check that cannot run keeps
/// the worktree.
pub fn verdict(home: &Path, found: &Found, merged_head: Option<&str>, done: bool) -> Verdict {
    judged(home, found, merged_head, done)
        .unwrap_or_else(|error| Verdict::Keep(format!("cannot check: {error}")))
}

fn judged(
    home: &Path,
    found: &Found,
    merged_head: Option<&str>,
    done: bool,
) -> Result<Verdict, String> {
    if let Some(reason) = unsettled(home, found)? {
        return Ok(Verdict::Keep(reason));
    }
    let path = &found.path;
    let tip = run(git(path).args(["rev-parse", "HEAD"]))?;
    let tip = tip.trim();
    let merged = run(git(path).args(["merge-base", "--is-ancestor", tip, "origin/HEAD"])).is_ok();
    if merged || merged_head == Some(tip) {
        return Ok(Verdict::Remove);
    }
    // A patch-equivalent commit counts as pushed only for a done task, which loses its worktree.
    let pushed = if done {
        at_risk(path, None, SCAN)? == 0
    } else {
        run(git(path).args(["rev-list", "-n1", tip, "--not", "--remotes"]))?.is_empty()
    };
    Ok(match (pushed, done) {
        (true, true) => Verdict::Retire,
        (true, false) => Verdict::Keep("not merged".to_owned()),
        (false, _) => Verdict::Keep("unpushed commits".to_owned()),
    })
}

/// The disk the folder takes, in bytes, without following symbolic links.
pub fn size(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    let own = meta.blocks() * 512;
    if !meta.is_dir() {
        return own;
    }
    own + fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| size(&entry.path()))
        .sum::<u64>()
}

/// Why the worktree of an asleep task must stay, or `None` when it can retire. Retiring keeps the
/// branch in the clone, so its commits need not be pushed.
pub fn retire_blocker(home: &Path, found: &Found) -> Option<String> {
    unsettled(home, found).unwrap_or_else(|error| Some(format!("cannot check: {error}")))
}

/// Removes the worktree and keeps its branch. Git refuses when anything is uncommitted or untracked.
/// The branch stays even once pushed, since the sweep fetches remote refs without `--prune` and a
/// reopen adds the worktree back from it.
pub fn retire(home: &Path, found: &Found) -> Result<(), String> {
    // A read-only folder under an ignored `target/` would stop git halfway, with the gitdir gone.
    unseal(&found.path);
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

/// Whether a process other than Priory's own git commands has its working directory inside `path`.
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
                && !own(&process.path())
        })
}

/// Whether the process is a git command Priory runs, or a child of one, which ends on its own: its
/// `PRIORY_GIT` names a running Priory process, which is its ancestor, and the ancestor's child on
/// the way carries the same marker. Any process can set the variable, but only Priory's own git
/// commands are both its children and carry it.
fn own(process: &Path) -> bool {
    let Some(value) = marker(process) else {
        return false;
    };
    let Ok(priory) = value.parse::<u32>() else {
        return false;
    };
    if !is_priory(priory) {
        return false;
    }
    let mut below = process.to_owned();
    // A bound on the walk, since pids can be reused while it reads.
    for _ in 0..64 {
        let Some(parent) = parent(&below) else {
            return false;
        };
        if parent == priory {
            return marker(&below).as_deref() == Some(value.as_str());
        }
        if parent <= 1 {
            return false;
        }
        below = Path::new("/proc").join(parent.to_string());
    }
    false
}

/// The value of `PRIORY_GIT` in the process's environment.
fn marker(process: &Path) -> Option<String> {
    let environ = fs::read(process.join("environ")).ok()?;
    let prefix = format!("{OWN_GIT}=");
    environ
        .split(|byte| *byte == 0)
        .find_map(|var| var.strip_prefix(prefix.as_bytes()))
        .map(|value| String::from_utf8_lossy(value).into_owned())
}

/// The pid of the process's parent, read from its `stat`, whose command name may hold any byte.
fn parent(process: &Path) -> Option<u32> {
    let stat = fs::read(process.join("stat")).ok()?;
    let after = stat.iter().rposition(|byte| *byte == b')')?;
    let rest = String::from_utf8_lossy(&stat[after + 1..]).into_owned();
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Whether process `pid` runs the same program as this process, Priory, even one replaced on disk
/// since it started.
fn is_priory(pid: u32) -> bool {
    let program = |link: PathBuf| {
        fs::read_link(link).ok().map(|path| {
            let path = path.to_string_lossy().into_owned();
            path.strip_suffix(" (deleted)").unwrap_or(&path).to_owned()
        })
    };
    let own = program(PathBuf::from("/proc/self/exe"));
    own.is_some() && own == program(Path::new("/proc").join(pid.to_string()).join("exe"))
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
