use crate::runtime::{OWN_GIT, SCAN, at_risk, clone_of, git, remove_sealed, run, unseal};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
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
    if let Some(reason) = unsettled(home, found, &[])? {
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
    unsettled(home, found, &[]).unwrap_or_else(|error| Some(format!("cannot check: {error}")))
}

/// The folders of `build` that exist in the worktree, each with the disk it takes in bytes, or why
/// none may go: the same checks as a retirement, where untracked files inside `build` do not count.
/// A folder in `skip`, or one that must stay, is left out. It walks every file, so call it off the
/// tick.
pub fn build_output(
    home: &Path,
    found: &Found,
    build: &[String],
    skip: &HashSet<PathBuf>,
) -> Result<Vec<(PathBuf, u64)>, String> {
    if let Some(reason) = unsettled(home, found, build)? {
        return Err(reason);
    }
    Ok(build
        .iter()
        .map(|folder| found.path.join(folder))
        .filter(|path| !skip.contains(path))
        .filter(|path| deletable(found, path).is_ok() && no_repository(path).is_ok())
        .map(|path| {
            let bytes = size(&path);
            (path, bytes)
        })
        .collect())
}

/// Why the build folder `path` must stay: a symlink on the way leads out, a mount is at or under it,
/// it is in a nested repository or a submodule, or git tracks a file in it.
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
    unmounted(path)?;
    let nested = || Err(format!("{} is in a nested repository", path.display()));
    let within = || rest.ancestors().filter(|part| !part.as_os_str().is_empty());
    if within().any(|part| fs::symlink_metadata(worktree.join(part).join(".git")).is_ok()) {
        return nested();
    }
    let staged = run(git(worktree)
        .args(["ls-files", "--stage", "-z", "--"])
        .args(within()))?;
    let gitlink = |entry: &str| {
        entry
            .strip_prefix("160000 ")
            .and_then(|entry| entry.split_once('\t'))
            .is_some_and(|(_, at)| within().any(|part| part == Path::new(at)))
    };
    if staged.split('\0').any(gitlink) {
        return nested();
    }
    let tracked = run(git(worktree).args(["ls-files", "--"]).arg(rest))?;
    if !tracked.is_empty() {
        return Err(format!("{} holds tracked files", path.display()));
    }
    Ok(())
}

/// Why the folder `path`, already moved aside, must not be deleted: a mount is at or under it, or it
/// holds a git repository. It walks every file, so call it off the tick.
pub fn expendable(path: &Path) -> Result<(), String> {
    unmounted(path)?;
    no_repository(path)
}

/// Deletes the folder `path`, which `expendable` allows. The `tmp` of a cargo target goes first, so
/// what a stopped delete leaves still has the tag that exempts the repositories in it.
pub fn discard(path: &Path) -> Result<(), String> {
    let gone = |path: &Path| {
        remove_sealed(path);
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            _ => Err(format!("cannot delete {}", path.display())),
        }
    };
    if let Some(scratch) = scratch(path) {
        gone(&scratch)?;
    }
    gone(path)
}

/// The `tmp` folder of a cargo target, where tests keep scratch files.
fn scratch(path: &Path) -> Option<PathBuf> {
    let tag = path.join("CACHEDIR.TAG");
    let tmp = path.join("tmp");
    let cargo = fs::symlink_metadata(&tag).is_ok_and(|meta| meta.is_file())
        && fs::read_to_string(&tag).is_ok_and(|tag| {
            tag.starts_with("Signature: 8a477f597d28d172789f06886806bc55")
                && tag.contains("created by cargo")
        })
        && fs::symlink_metadata(&tmp).is_ok_and(|meta| meta.is_dir());
    cargo.then_some(tmp)
}

/// Refuses a folder with a `.git` entry or a bare repository in it, except in a cargo target's `tmp`.
fn no_repository(path: &Path) -> Result<(), String> {
    let scratch = scratch(path);
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let unread = |error| format!("cannot read {}: {error}", folder.display());
        for entry in fs::read_dir(&folder).map_err(unread)? {
            let entry = entry.map_err(unread)?;
            let bare = || {
                ["objects", "refs"]
                    .iter()
                    .all(|name| fs::symlink_metadata(folder.join(name)).is_ok())
            };
            if entry.file_name() == ".git" || entry.file_name() == "HEAD" && bare() {
                return Err(format!("{} holds a git repository", folder.display()));
            }
            if entry.file_type().map_err(unread)?.is_dir() && Some(entry.path()) != scratch {
                folders.push(entry.path());
            }
        }
    }
    Ok(())
}

/// Refuses a folder with a mount at or under it, which a rename carries along and a delete empties.
fn unmounted(path: &Path) -> Result<(), String> {
    let unread = |error| format!("cannot check {} for mounts: {error}", path.display());
    let real = fs::canonicalize(path).map_err(unread)?;
    let table = fs::read("/proc/self/mountinfo").map_err(unread)?;
    match mount_under(&table, &real) {
        Some(mount) => Err(format!("{} is a mount", mount.display())),
        None => Ok(()),
    }
}

/// The first mount point at or under `path` in a `mountinfo` table.
fn mount_under(table: &[u8], path: &Path) -> Option<PathBuf> {
    table
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.split(|byte| *byte == b' ').nth(4))
        .map(|point| PathBuf::from(std::ffi::OsString::from_vec(unescaped(point))))
        .find(|point| point.starts_with(path))
}

/// `field` with each `\` and 3 octal digits, as the kernel writes a space, back to its byte.
fn unescaped(field: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut rest = field;
    while let Some((first, after)) = rest.split_first() {
        let octal = (*first == b'\\')
            .then(|| after.get(..3))
            .flatten()
            .filter(|digits| digits.iter().all(|digit| (b'0'..=b'7').contains(digit)))
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok());
        bytes.push(octal.unwrap_or(*first));
        rest = if octal.is_some() { &after[3..] } else { after };
    }
    bytes
}

/// Why a build folder did not move.
#[derive(Debug)]
pub struct Unmoved {
    pub reason: String,
    /// The worktree is on another filesystem, so no later try can work either.
    pub elsewhere: bool,
}

impl From<String> for Unmoved {
    fn from(reason: String) -> Unmoved {
        Unmoved {
            reason,
            elsewhere: false,
        }
    }
}

/// Moves the build folder `path` of `found` to `aside`, on the same disk, after the checks of
/// `build_output` pass again, since the sizing ran a while ago. A rename takes no longer for a large
/// folder, so the caller can delete `aside` off the tick.
pub fn set_aside(
    home: &Path,
    found: &Found,
    build: &[String],
    path: &Path,
    aside: &Path,
) -> Result<(), Unmoved> {
    if let Some(reason) = unsettled(home, found, build)? {
        return Err(reason.into());
    }
    deletable(found, path)?;
    // Moving a folder to another parent rewrites its `..`, which a read-only folder refuses.
    let failed = |error: std::io::Error| Unmoved {
        reason: format!("cannot move {}: {error}", path.display()),
        elsewhere: error.kind() == ErrorKind::CrossesDevices,
    };
    let before = fs::symlink_metadata(path).map_err(failed)?.permissions();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(failed)?;
    fs::rename(path, aside).map_err(|error| {
        let _ = fs::set_permissions(path, before);
        failed(error)
    })
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
    // Without it, the walk's status could hold `index.lock` while a run starts.
    let status = run(git(path).args([
        "--no-optional-locks",
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TABLE: &[u8] = b"22 1 8:1 / / rw - ext4 /dev/sda1 rw\n\
        23 22 0:5 / /w/a\\040b/target/mnt rw shared:1 - tmpfs tmpfs rw\n";

    #[test]
    fn a_mount_at_or_under_a_folder_is_found_and_one_beside_or_above_it_is_not() {
        let under = |path: &str| mount_under(TABLE, Path::new(path));
        let mount = Some(PathBuf::from("/w/a b/target/mnt"));
        assert_eq!(under("/w/a b/target"), mount);
        assert_eq!(under("/w/a b/target/mnt"), mount);
        assert_eq!(under("/w/a b/target/mn"), None);
        assert_eq!(under("/w/a b/target/mnt/deep"), None);
        assert_eq!(under("/w/other"), None);
    }

    #[test]
    fn a_mounted_folder_is_neither_taken_nor_deleted() {
        let found = Found {
            path: PathBuf::from("/"),
            repo: "o/r".to_owned(),
            name: "1".to_owned(),
        };
        let proc = Path::new("/proc");
        let refused = deletable(&found, proc).unwrap_err();
        assert!(refused.ends_with("is a mount"), "{refused}");
        let refused = expendable(proc).unwrap_err();
        assert!(refused.ends_with("is a mount"), "{refused}");
    }

    proptest! {
        #[test]
        fn mount_under_never_panics(table in proptest::collection::vec(any::<u8>(), 0..200)) {
            let _ = mount_under(&table, Path::new("/w"));
        }

        #[test]
        fn mount_under_reads_back_any_mount_point(name in "[a-z \\\\\t]{1,20}") {
            let escaped: String = name
                .bytes()
                .map(|byte| match byte {
                    b'a'..=b'z' => char::from(byte).to_string(),
                    byte => format!("\\{byte:03o}"),
                })
                .collect();
            let table = format!("23 22 0:5 / /w/{escaped} rw - tmpfs tmpfs rw\n");
            let point = Path::new("/w").join(&name);
            prop_assert_eq!(mount_under(table.as_bytes(), Path::new("/w")), Some(point));
        }
    }
}
