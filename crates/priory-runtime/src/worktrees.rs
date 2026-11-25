use crate::runtime::{
    OWN_GIT, PATCHES, SCAN, as_one, at_risk, clone_of, git, is_ancestor, run, soon, unseal,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    as_one(|| {
        soon(git(&clone).args(["fetch", "--quiet", "origin"]))?;
        soon(git(&clone).args(["remote", "set-head", "origin", "--auto"])).map(drop)
    })
}

/// Why a worktree stays when the sweep ran all its patch scans.
pub const SCAN_WAITS: &str = "its patch check waits for the next sweep";

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
/// `scans` is how many patch scans this sweep may still run.
pub fn verdict(
    home: &Path,
    found: &Found,
    merged_head: Option<&str>,
    done: bool,
    scans: &mut usize,
) -> Verdict {
    as_one(|| judged(home, found, merged_head, done, scans))
        .unwrap_or_else(|error| Verdict::Keep(format!("cannot check: {error}")))
}

fn judged(
    home: &Path,
    found: &Found,
    merged_head: Option<&str>,
    done: bool,
    scans: &mut usize,
) -> Result<Verdict, String> {
    if let Some(reason) = unsettled(home, found, &[])? {
        return Ok(Verdict::Keep(reason));
    }
    let path = &found.path;
    let tip = soon(git(path).args(["rev-parse", "HEAD"]))?;
    let tip = tip.trim();
    let merged = is_ancestor(path, tip, "origin/HEAD")?;
    if merged || merged_head == Some(tip) {
        return Ok(Verdict::Remove);
    }
    // A patch-equivalent commit counts as pushed only for a done task, which loses its worktree.
    let mut pushed =
        soon(git(path).args(["rev-list", "-n1", tip, "--not", "--remotes"]))?.is_empty();
    if done && !pushed {
        match scanned(path, tip, scans)? {
            Some(at_risk) => pushed = at_risk == 0,
            None => {
                return Ok(Verdict::Keep(SCAN_WAITS.to_owned()));
            }
        }
    }
    Ok(match (pushed, done) {
        (true, true) => Verdict::Retire,
        (true, false) => Verdict::Keep("not merged".to_owned()),
        (false, _) => Verdict::Keep("unpushed commits".to_owned()),
    })
}

/// A worktree's last patch scan.
struct Scan {
    /// HEAD and the remote-tracking refs it compared, hashed.
    of: u64,
    at_risk: usize,
}

static SCANS: Mutex<Option<HashMap<PathBuf, Scan>>> = Mutex::new(None);

fn scans() -> MutexGuard<'static, Option<HashMap<PathBuf, Scan>>> {
    SCANS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Forgets the scans of worktrees that are gone.
pub fn forget_scans(home: &Path, found: &[Found]) {
    if let Some(scans) = scans().as_mut() {
        let kept: HashSet<&Path> = found.iter().map(|found| found.path.as_path()).collect();
        scans.retain(|path, _| !path.starts_with(home) || kept.contains(path.as_path()));
    }
}

/// What `at_risk` says of the worktree whose HEAD is `tip`, again only when HEAD or a remote ref
/// changed. `None` when `left`, the scans this sweep may still run, is 0.
fn scanned(path: &Path, tip: &str, left: &mut usize) -> Result<Option<usize>, String> {
    let refs = soon(git(path).args([
        "for-each-ref",
        "--format=%(objectname) %(refname)",
        "refs/remotes",
    ]))?;
    let mut hasher = DefaultHasher::new();
    (tip, refs).hash(&mut hasher);
    let of = hasher.finish();
    if let Some(scan) = scans().as_ref().and_then(|scans| scans.get(path))
        && scan.of == of
    {
        return Ok(Some(scan.at_risk));
    }
    if *left == 0 {
        return Ok(None);
    }
    *left -= 1;
    let at_risk = at_risk(path, None, SCAN, PATCHES)?;
    scans()
        .get_or_insert_default()
        .insert(path.to_owned(), Scan { of, at_risk });
    Ok(Some(at_risk))
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
        .filter(|path| deletable(found, path).is_ok() && no_repository(path, None).is_ok())
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
    unmounted(path, &mountinfo)?;
    let nested = || Err(format!("{} is in a nested repository", path.display()));
    let within = || rest.ancestors().filter(|part| !part.as_os_str().is_empty());
    if within().any(|part| fs::symlink_metadata(worktree.join(part).join(".git")).is_ok()) {
        return nested();
    }
    let staged = soon(
        git(worktree)
            .args(["ls-files", "--stage", "-z", "--"])
            .args(within()),
    )?;
    let gitlink = |entry: &str| {
        entry
            .strip_prefix("160000 ")
            .and_then(|entry| entry.split_once('\t'))
            .is_some_and(|(_, at)| within().any(|part| part == Path::new(at)))
    };
    if staged.split('\0').any(gitlink) {
        return nested();
    }
    let tracked = soon(git(worktree).args(["ls-files", "--"]).arg(rest))?;
    if !tracked.is_empty() {
        return Err(format!("{} holds tracked files", path.display()));
    }
    Ok(())
}

/// Why the folder `path`, already moved aside, must not be deleted: a mount is at or under it, or it
/// holds a git repository. It walks every file, so call it off the tick.
pub fn expendable(path: &Path) -> Result<(), String> {
    unmounted(path, &mountinfo)?;
    no_repository(path, None)
}

/// Deletes the folder `path`, which `expendable` allows. The `tmp` of a cargo target goes first, so
/// what a stopped delete leaves still has the tag that exempts the repositories in it.
pub fn discard(path: &Path) -> Result<(), String> {
    let gone = |path: &Path| {
        unseal(path, None);
        // The walk takes a while, in which a mount can appear.
        unmounted(path, &mountinfo)?;
        let _ = fs::remove_dir_all(path);
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

/// How long the loop walks the files of a worktree.
const WALK: Duration = Duration::from_secs(10);

fn slow_walk(path: &Path) -> String {
    format!(
        "{}: its files take over {}s to check",
        path.display(),
        WALK.as_secs()
    )
}

/// Refuses a folder with a `.git` entry or a bare repository in it, except in a cargo target's `tmp`.
/// It also refuses one it cannot walk by `deadline`.
fn no_repository(path: &Path, deadline: Option<Instant>) -> Result<(), String> {
    let scratch = scratch(path);
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let unread = |error| format!("cannot read {}: {error}", folder.display());
        for entry in fs::read_dir(&folder).map_err(unread)? {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(slow_walk(path));
            }
            let entry = entry.map_err(unread)?;
            if entry.file_name() == ".git" || entry.file_name() == "HEAD" && bare(&folder) {
                return Err(format!("{} holds a git repository", folder.display()));
            }
            if entry.file_type().map_err(unread)?.is_dir() && Some(entry.path()) != scratch {
                folders.push(entry.path());
            }
        }
    }
    Ok(())
}

/// Whether `folder` is laid out as a bare repository.
fn bare(folder: &Path) -> bool {
    ["HEAD", "objects", "refs"]
        .iter()
        .all(|name| fs::symlink_metadata(folder.join(name)).is_ok())
}

/// Refuses a worktree whose ignored files hold a repository, as `no_repository` does, or with a
/// `.git` in a folder of tracked files. A tracked folder laid out as a bare repository is on the
/// branch.
fn no_unsaved_repository(worktree: &Path, deadline: Instant) -> Result<(), String> {
    let tracked = soon(git(worktree).args(["ls-files", "-z"]))?;
    let folders: HashSet<&Path> = tracked
        .split('\0')
        .flat_map(|file| Path::new(file).ancestors().skip(1))
        .filter(|folder| !folder.as_os_str().is_empty())
        .collect();
    for folder in folders {
        if Instant::now() >= deadline {
            return Err(slow_walk(worktree));
        }
        let folder = worktree.join(folder);
        if fs::symlink_metadata(folder.join(".git")).is_ok() {
            return Err(format!("{} holds a git repository", folder.display()));
        }
    }
    let ignored = soon(git(worktree).args([
        "--no-optional-locks",
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "-z",
    ]))?;
    for entry in ignored.split('\0').filter(|entry| !entry.is_empty()) {
        if Instant::now() >= deadline {
            return Err(slow_walk(worktree));
        }
        let path = worktree.join(entry);
        let meta = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if meta.is_dir() {
            no_repository(&path, Some(deadline))?;
        }
        let part = ["HEAD", "objects", "refs"]
            .iter()
            .any(|name| path.file_name().is_some_and(|file| file == *name));
        if let Some(parent) = path.parent().filter(|parent| part && bare(parent)) {
            return Err(format!("{} holds a git repository", parent.display()));
        }
    }
    Ok(())
}

/// As `no_repository` for a worktree without its gitdir, whose own `.git` does not count.
pub(crate) fn orphan_without_repository(path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + WALK;
    let unread = |error| format!("cannot read {}: {error}", path.display());
    for entry in fs::read_dir(path).map_err(unread)? {
        if Instant::now() >= deadline {
            return Err(slow_walk(path));
        }
        let entry = entry.map_err(unread)?;
        if entry.file_name() != ".git" && entry.file_type().map_err(unread)?.is_dir() {
            no_repository(&entry.path(), Some(deadline))?;
        }
    }
    Ok(())
}

/// Reads a `mountinfo` table.
type Mounts<'a> = &'a dyn Fn() -> std::io::Result<Vec<u8>>;

pub(crate) fn mountinfo() -> std::io::Result<Vec<u8>> {
    fs::read("/proc/self/mountinfo")
}

/// Removes the checkout `path` of `clone`, whatever it holds but a mount.
pub fn remove_checkout(clone: &Path, path: &Path) -> Result<(), String> {
    unmounted(path, &mountinfo)?;
    run(git(clone).args(["worktree", "remove", "--force"]).arg(path)).map(drop)
}

/// Deletes the folder `path` unless a mount is at or under it.
pub fn remove_unmounted(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    unmounted(path, &mountinfo)?;
    fs::remove_dir_all(path).map_err(|error| format!("cannot delete {}: {error}", path.display()))
}

/// The folder of `home` a folder moves to before its delete on a thread.
pub const RECLAIM: &str = "reclaim";

/// `home/reclaim`, made when absent. Anything but a real folder there is refused, since a symlink
/// would lead the delete out of `home`.
pub fn reclaim_folder(home: &Path) -> Result<PathBuf, String> {
    let folder = home.join(RECLAIM);
    match fs::symlink_metadata(&folder) {
        Ok(meta) if meta.is_dir() => Ok(folder),
        Ok(_) => Err(format!("{} is not a folder", folder.display())),
        Err(_) => match fs::create_dir(&folder) {
            Ok(()) => Ok(folder),
            Err(error) => Err(format!("cannot make {}: {error}", folder.display())),
        },
    }
}

/// A path in `home/reclaim` for a folder of `task`, as the sweep names what it deletes there.
pub fn reclaim_path(home: &Path, task: i64) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    Ok(reclaim_folder(home)?.join(format!("{task}-{stamp:018}")))
}

/// Deletes the task folder `path`, a worktree without its gitdir that holds no repository: a rename,
/// into `home/reclaim` or beside it on another filesystem, then a thread. It stays when neither
/// works, and the caller moves it.
pub(crate) fn discard_aside(home: &Path, path: &Path) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let Some(task) = name.rsplit('-').next().and_then(|id| id.parse().ok()) else {
        return;
    };
    let Ok(aside) = reclaim_path(home, task) else {
        return;
    };
    let aside = match fs::rename(path, &aside) {
        Ok(()) => aside,
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            let beside = path.with_file_name(aside.file_name().unwrap_or_default());
            if fs::rename(path, &beside).is_err() {
                return;
            }
            beside
        }
        Err(_) => return,
    };
    // Its own link would keep the sweep from deleting what a stopped daemon leaves.
    let _ = fs::remove_file(aside.join(".git"));
    std::thread::spawn(move || {
        if unmounted(&aside, &mountinfo).is_ok() {
            let _ = discard(&aside);
        }
    });
}

/// Refuses a folder with a mount at or under it, which a rename carries along and a delete empties.
pub(crate) fn unmounted(path: &Path, mounts: Mounts) -> Result<(), String> {
    let unread = |error| format!("cannot check {} for mounts: {error}", path.display());
    let real = fs::canonicalize(path).map_err(unread)?;
    let table = mounts().map_err(unread)?;
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
    retire_under(home, found, &mountinfo)
}

/// Why the worktree must stay although its task is done with it: what `unsettled` says, a mount
/// under it, or a repository its branch does not hold.
pub fn removable(home: &Path, found: &Found) -> Result<(), String> {
    as_one(|| removable_under(home, found, &mountinfo))
}

fn removable_under(home: &Path, found: &Found, mounts: Mounts) -> Result<(), String> {
    // So the walk enters no mount.
    unmounted(&found.path, mounts)?;
    no_unsaved_repository(&found.path, Instant::now() + WALK)?;
    unsettled(home, found, &[])?.map_or(Ok(()), Err)
}

/// Removes the worktree that `removable` just allowed.
pub fn remove_allowed(home: &Path, found: &Found) -> Result<(), String> {
    remove_allowed_under(home, found, &mountinfo)
}

fn retire_under(home: &Path, found: &Found, mounts: Mounts) -> Result<(), String> {
    as_one(|| removable_under(home, found, mounts))?;
    remove_allowed_under(home, found, mounts)
}

fn remove_allowed_under(home: &Path, found: &Found, mounts: Mounts) -> Result<(), String> {
    // The walk and git take a while, in which a mount can appear.
    unmounted(&found.path, mounts)?;
    // A read-only folder under an ignored `target/` would stop git halfway, with the gitdir gone.
    if !unseal(&found.path, Some(Instant::now() + WALK)) {
        return Err(slow_walk(&found.path));
    }
    // The walk above takes a while too.
    unmounted(&found.path, mounts)?;
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
    if soon(git(path).args(["branch", "--show-current"]))?.trim() != branch {
        return Ok(Some(format!("not on branch {branch}")));
    }
    // Without it, the walk's status could hold `index.lock` while a run starts.
    let status = soon(git(path).args([
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
    if occupied(path)? {
        return Ok(Some("a process runs in it".to_owned()));
    }
    Ok(None)
}

/// Whether a process other than Priory's own git commands has its working directory inside `path`.
fn occupied(path: &Path) -> Result<bool, String> {
    let Ok(path) = fs::canonicalize(path) else {
        return Ok(false);
    };
    if cfg!(target_os = "macos") {
        return occupied_of_lsof(&path, &|| {
            soon(Command::new("lsof").args(["-w", "-a", "-d", "cwd", "-Fn"]))
        });
    }
    Ok(fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .any(|process| {
            fs::read_link(process.path().join("cwd")).is_ok_and(|cwd| cwd.starts_with(&path))
                && !own(&process.path())
        }))
}

/// Whether any process has its working directory inside `path`, from the fields `lsof -Fn` prints.
/// Priory's own git counts too: macOS shows no environment, so nothing proves a git is Priory's.
fn occupied_of_lsof(
    path: &Path,
    lsof: &dyn Fn() -> Result<String, String>,
) -> Result<bool, String> {
    let unsure = |why: &str| format!("cannot check {} for processes: {why}", path.display());
    // lsof escapes the other bytes and the backslash, so such a path would match no process.
    let plain = |byte: u8| (b' '..=b'~').contains(&byte) && byte != b'\\';
    if !path.as_os_str().as_bytes().iter().copied().all(plain) {
        return Err(unsure("lsof escapes its path"));
    }
    let listed = lsof().map_err(|error| unsure(&error))?;
    let mut cwds = listed.lines().filter_map(|line| line.strip_prefix('n'));
    // Every process has one, this one included.
    let first = cwds.next().ok_or_else(|| unsure("lsof listed nothing"))?;
    Ok(std::iter::once(first)
        .chain(cwds)
        .any(|cwd| Path::new(cwd).starts_with(path)))
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
    let merged = is_ancestor(&clone, &branch, "origin/HEAD") == Ok(true);
    let delete = if merged { "-D" } else { "-d" };
    soon(git(&clone).args(["branch", delete, &branch])).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::remove_sealed;
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
        let refused = remove_unmounted(proc).unwrap_err();
        assert!(refused.ends_with("is a mount"), "{refused}");
        let refused = remove_checkout(Path::new("/"), proc).unwrap_err();
        assert!(refused.ends_with("is a mount"), "{refused}");
    }

    /// A home with a clone of `o/n` and task 1's worktree, which ignores `target` and `HEAD`.
    struct Home(PathBuf);

    impl Drop for Home {
        fn drop(&mut self) {
            remove_sealed(&self.0);
        }
    }

    impl Home {
        fn new(name: &str) -> (Home, Found) {
            let dir =
                std::env::temp_dir().join(format!("priory-removal-{name}-{}", std::process::id()));
            remove_sealed(&dir);
            let clone = dir.join("repos/o/n");
            fs::create_dir_all(clone.join("fixture/refs")).unwrap();
            fs::create_dir_all(clone.join("fixture/objects")).unwrap();
            for file in ["HEAD", "refs/a", "objects/a"] {
                fs::write(clone.join("fixture").join(file), "x\n").unwrap();
            }
            fs::write(clone.join(".gitignore"), "target\n/split/HEAD\n").unwrap();
            let path = dir.join("worktrees/o/n/1");
            sh(&clone, &["init", "-q"]);
            sh(&clone, &["add", "."]);
            sh(&clone, &["commit", "-q", "-m", "a"]);
            let tree = path.to_str().unwrap();
            sh(&clone, &["worktree", "add", "-q", "-b", "priory/1", tree]);
            let found = Found {
                path,
                repo: "o/n".to_owned(),
                name: "1".to_owned(),
            };
            (Home(dir), found)
        }
    }

    fn sh(dir: &Path, args: &[&str]) {
        run(git(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args))
        .unwrap();
    }

    #[test]
    fn a_removal_with_a_mount_under_the_worktree_touches_nothing() {
        let (home, found) = Home::new("mount");
        let mount = found.path.join("target/mnt");
        fs::create_dir_all(&mount).unwrap();
        fs::write(mount.join("keep"), "x\n").unwrap();
        fs::set_permissions(&mount, fs::Permissions::from_mode(0o555)).unwrap();
        let point = fs::canonicalize(&mount).unwrap();
        let table = format!("23 22 0:5 / {} rw - tmpfs tmpfs rw\n", point.display());

        let read = || Ok(table.clone().into_bytes());
        let refused = retire_under(&home.0, &found, &read).unwrap_err();

        assert_eq!(refused, format!("{} is a mount", point.display()));
        let failed = || Err(ErrorKind::PermissionDenied.into());
        let refused = retire_under(&home.0, &found, &failed).unwrap_err();
        assert!(refused.contains("for mounts"), "{refused}");
        assert!(mount.join("keep").exists());
        let mode = fs::metadata(&mount).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o555,
            "nothing is made writable through a mount"
        );
        assert!(home.0.join("repos/o/n/.git/worktrees/1").exists());

        let beside = || Ok(b"23 22 0:5 / /elsewhere rw - tmpfs tmpfs rw\n".to_vec());
        retire_under(&home.0, &found, &beside).unwrap();
        assert!(!found.path.exists());
    }

    #[test]
    fn a_removal_keeps_a_worktree_whose_ignored_files_hold_a_repository() {
        let (home, found) = Home::new("repository");
        for made in [&["-q", "--bare", "target/o.git"][..], &["-q", "target/n"]] {
            sh(&found.path, &[&["init"], made].concat());
            let folder = found.path.join(made[made.len() - 1]);
            let refused = retire(&home.0, &found).unwrap_err();
            assert_eq!(
                refused,
                format!("{} holds a git repository", folder.display())
            );
            fs::remove_dir_all(folder).unwrap();
        }
        let split = found.path.join("split");
        for folder in ["objects", "refs"] {
            fs::create_dir_all(split.join(folder)).unwrap();
            fs::write(split.join(folder).join("a"), "x\n").unwrap();
        }
        sh(&found.path, &["add", "."]);
        sh(&found.path, &["commit", "-q", "-m", "split"]);
        fs::write(split.join("HEAD"), "x\n").unwrap();
        let refused = retire(&home.0, &found).unwrap_err();
        assert!(
            refused.ends_with("split holds a git repository"),
            "{refused}"
        );
        fs::remove_file(split.join("HEAD")).unwrap();

        sh(&found.path, &["init", "-q", "fixture"]);
        let refused = retire(&home.0, &found).unwrap_err();
        assert!(
            refused.ends_with("fixture holds a git repository"),
            "{refused}"
        );
        fs::remove_dir_all(found.path.join("fixture/.git")).unwrap();

        retire(&home.0, &found).unwrap();
        assert!(!found.path.exists(), "the tracked fixture does not count");
    }

    #[test]
    fn a_mount_that_appears_after_the_checks_still_stops_the_removal() {
        let (home, found) = Home::new("late-mount");
        removable(&home.0, &found).unwrap();
        let point = fs::canonicalize(&found.path).unwrap().join("target");
        fs::create_dir_all(&point).unwrap();
        let table = format!("23 22 0:5 / {} rw - tmpfs tmpfs rw\n", point.display());

        let refused =
            remove_allowed_under(&home.0, &found, &|| Ok(table.clone().into_bytes())).unwrap_err();

        assert_eq!(refused, format!("{} is a mount", point.display()));
        assert!(found.path.exists());
        remove_allowed(&home.0, &found).unwrap();
        assert!(!found.path.exists());
    }

    #[test]
    fn a_walk_past_its_deadline_keeps_the_worktree() {
        let (_home, found) = Home::new("slow-walk");
        fs::create_dir_all(found.path.join("target/deep")).unwrap();
        let now = Some(Instant::now());

        let refused = no_repository(&found.path.join("target"), now).unwrap_err();

        assert!(
            refused.ends_with("its files take over 10s to check"),
            "{refused}"
        );
        assert!(no_unsaved_repository(&found.path, Instant::now()).is_err());
        assert!(!unseal(&found.path, now), "the delete does not start");
        no_unsaved_repository(&found.path, Instant::now() + WALK).unwrap();
    }

    #[test]
    fn a_patch_scan_runs_again_only_when_head_or_a_remote_ref_changed() {
        let (_home, found) = Home::new("scans");
        let tree = &found.path;
        let commit = |text: &str| {
            fs::write(tree.join("a"), text).unwrap();
            sh(tree, &["add", "a"]);
            sh(tree, &["commit", "-q", "-m", text]);
            run(git(tree).args(["rev-parse", "HEAD"]))
                .unwrap()
                .trim()
                .to_owned()
        };
        sh(tree, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        let tip = commit("one");
        let mut left = 0;
        assert_eq!(scanned(tree, &tip, &mut left), Ok(None), "none left");
        left = 5;
        assert_eq!(scanned(tree, &tip, &mut left), Ok(Some(1)));
        assert_eq!(scanned(tree, &tip, &mut left), Ok(Some(1)));
        assert_eq!(left, 4, "the second answer is the first one's");

        let tip = commit("two");
        assert_eq!(scanned(tree, &tip, &mut left), Ok(Some(2)));
        assert_eq!(left, 3, "HEAD moved");
        sh(tree, &["update-ref", "refs/remotes/origin/x", "HEAD"]);
        assert_eq!(scanned(tree, &tip, &mut left), Ok(Some(0)));
        assert_eq!(left, 2, "a remote ref changed");
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

    #[test]
    fn a_process_lsof_lists_inside_a_worktree_occupies_it_whatever_it_runs() {
        let in_w = |listed: &str| occupied_of_lsof(Path::new("/w/1"), &|| Ok(listed.to_owned()));
        assert_eq!(in_w("p12\nfcwd\nn/\np13\nfcwd\nn/w/1/a b\n"), Ok(true));
        assert_eq!(in_w("p12\nfcwd\nn/w/1\n"), Ok(true));
        assert_eq!(in_w("p12\nfcwd\nn/w/10\np13\nfcwd\nn/w\n"), Ok(false));
    }

    #[test]
    fn a_worktree_lsof_cannot_answer_for_is_not_called_free() {
        let of = |path: &str, listed: Result<String, String>| {
            occupied_of_lsof(Path::new(path), &|| listed.clone()).unwrap_err()
        };
        let refused = of("/w/1", Err("lsof: not found".to_owned()));
        assert!(refused.ends_with("lsof: not found"), "{refused}");
        let refused = of("/w/1", Ok(String::new()));
        assert!(refused.ends_with("lsof listed nothing"), "{refused}");
        for path in ["/w/\u{e9}/1", "/w/a\\b", "/w/a\tb"] {
            let refused = of(path, Ok("n/\n".to_owned()));
            assert!(refused.ends_with("lsof escapes its path"), "{refused}");
        }
    }

    proptest! {
        #[test]
        fn occupied_of_lsof_reads_any_text_without_panicking(listed in "[pn0-9/a\\n]{0,200}") {
            let _ = occupied_of_lsof(Path::new("/a"), &|| Ok(listed.clone()));
        }
    }
}
