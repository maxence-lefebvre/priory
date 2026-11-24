use crate::runtime::{OWN_GIT, clone_of, git, remove_sealed, run};
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
    if let Some(reason) = unsettled(home, found, &[])? {
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
    unsettled(home, found, &[]).unwrap_or_else(|error| Some(format!("cannot check: {error}")))
}

/// The folders of `build` that exist in the worktree, each with the disk it takes in bytes, or why
/// none may go: the same checks as a retirement, where untracked files inside `build` do not count.
/// A folder that holds a tracked file is left out. It walks every file, so call it off the tick.
pub fn build_output(
    home: &Path,
    found: &Found,
    build: &[String],
) -> Result<Vec<(PathBuf, u64)>, String> {
    if let Some(reason) = unsettled(home, found, build)? {
        return Err(reason);
    }
    Ok(build
        .iter()
        .map(|folder| found.path.join(folder))
        .filter(|path| deletable(found, path).is_ok())
        .map(|path| {
            let bytes = size(&path);
            (path, bytes)
        })
        .collect())
}

/// Why the build folder `path` must stay: a symlink on the way leads out, or git tracks a file in it.
fn deletable(found: &Found, path: &Path) -> Result<(), String> {
    let worktree = &found.path;
    let (Ok(root), Ok(rest)) = (fs::canonicalize(worktree), path.strip_prefix(worktree)) else {
        return Err(format!("{} is not inside the worktree", path.display()));
    };
    let real = fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        && fs::canonicalize(path).ok() == Some(root.join(rest));
    if !real {
        return Err(format!(
            "{} is not a folder inside the worktree",
            path.display()
        ));
    }
    let tracked = run(git(worktree).args(["ls-files", "--"]).arg(rest))?;
    if !tracked.is_empty() {
        return Err(format!("{} holds tracked files", path.display()));
    }
    Ok(())
}

/// Deletes the build folder `path` of `found` and all it holds, read-only folders included, after
/// the checks of `build_output` pass again, since the sizing ran a while ago.
pub fn reclaim(home: &Path, found: &Found, build: &[String], path: &Path) -> Result<(), String> {
    if let Some(reason) = unsettled(home, found, build)? {
        return Err(reason);
    }
    deletable(found, path)?;
    remove_sealed(path);
    if fs::symlink_metadata(path).is_ok() {
        return Err(format!("cannot delete {}", path.display()));
    }
    Ok(())
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

/// Removes the worktree and keeps its branch. Git refuses when anything is uncommitted or untracked.
pub fn retire(home: &Path, found: &Found) -> Result<(), String> {
    run(git(&found.clone(home))
        .args(["worktree", "remove"])
        .arg(&found.path))
    .map(drop)
}

/// Why removing the worktree would lose a file or a process's folder: everything else is on its
/// branch. Untracked files inside the `build` folders do not count.
fn unsettled(home: &Path, found: &Found, build: &[String]) -> Result<Option<String>, String> {
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
    let built = |line: &&str| {
        let Some(file) = line.strip_prefix("?? ") else {
            return false;
        };
        build.iter().any(|folder| {
            let folder = folder.trim_end_matches('/');
            file.strip_prefix(folder)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    };
    let status: Vec<&str> = status.lines().filter(|line| !built(line)).collect();
    if status.iter().any(|line| !line.starts_with("??")) {
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
