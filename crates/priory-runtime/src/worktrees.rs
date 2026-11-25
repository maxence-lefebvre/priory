use crate::runtime::{
    OWN_GIT, PATCHES, SCAN, as_one, at_risk, fetching, git, is_ancestor, move_aside, read_regular,
    run, soon, soon_raw, timed_out, uid, unseal,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A folder under `PRIORY_HOME/worktrees/<owner>/<name>/`, named for a task.
#[derive(Clone)]
pub struct Found {
    pub path: PathBuf,
    pub repo: String,
    pub name: String,
    /// Its entry in the clone, when the folder moved and its `.git` no longer leads there.
    pub(crate) entry: Option<PathBuf>,
}

impl Found {
    fn git(&self) -> Command {
        let mut command = git(&self.path);
        if let Some(entry) = &self.entry {
            command
                .env("GIT_DIR", entry)
                .env("GIT_WORK_TREE", &self.path);
        }
        command
    }

    fn clone_in(&self, home: &Path) -> PathBuf {
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
                    entry: None,
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
    // Two fetches of one ref at a time would fail one of them.
    let fetching = fetching(&clone);
    let _fetching = fetching
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    as_one(|| {
        soon(git(&clone).args(["fetch", "--quiet", "origin"]))?;
        soon(git(&clone).args(["remote", "set-head", "origin", "--auto"])).map(drop)
    })
}

/// Why a worktree stays when the sweep ran all its patch scans.
pub const SCAN_WAITS: &str = "its patch check waits for the next sweep";

/// What the sweep does with the worktree of an ended task, whose HEAD is `tip`.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// The branch merged: the worktree and the branch go.
    Remove { tip: String },
    /// Every commit is on a remote: the worktree goes and the branch stays for a revive.
    Retire { tip: String },
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
        return Ok(Verdict::Remove {
            tip: tip.to_owned(),
        });
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
        (true, true) => Verdict::Retire {
            tip: tip.to_owned(),
        },
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

/// Forgets the scans and the walks of worktrees that are gone.
pub fn forget_scans(home: &Path, found: &[Found]) {
    let kept: HashSet<&Path> = found.iter().map(|found| found.path.as_path()).collect();
    let keeps = |path: &Path| !path.starts_with(home) || kept.contains(path);
    if let Some(scans) = scans().as_mut() {
        scans.retain(|path, _| keeps(path));
    }
    if let Some(walks) = walks().as_mut() {
        walks.retain(|path, _| keeps(path));
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
        unseal(path);
        // The walk takes a while, in which a mount can appear.
        unmounted(path, &mountinfo)?;
        let _ = fs::remove_dir_all(path);
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            _ => Err(format!("cannot delete {}", path.display())),
        }
    };
    for scratch in scratches(path) {
        gone(&scratch)?;
    }
    gone(path)
}

/// The `tmp` of each cargo target at or under `path`.
fn scratches(path: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let scratch = scratch(&folder);
        for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) && Some(&path) != scratch.as_ref()
            {
                folders.push(path);
            }
        }
        found.extend(scratch);
    }
    found
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

/// How long a run's start walks the files of a folder without its gitdir.
const WALK: Duration = Duration::from_secs(10);

fn slow_walk(path: &Path) -> String {
    format!(
        "{}: its files take over {}s to check",
        path.display(),
        WALK.as_secs()
    )
}

fn holds_repository(folder: &Path) -> String {
    format!("{} holds a git repository", folder.display())
}

/// Refuses a folder that holds a repository (see `repository_in`), or one it cannot walk by `deadline`.
fn no_repository(path: &Path, deadline: Option<Instant>) -> Result<(), String> {
    match repository_in(path, deadline)? {
        Some(folder) => Err(holds_repository(&folder)),
        None => Ok(()),
    }
}

/// Lets its owner read the folder, when that is the user `own` gives and the mode refuses it.
fn open_folder(folder: &Path, own: fn() -> u32) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(folder)?;
    let mode = meta.permissions().mode();
    if !meta.is_dir() || mode & 0o500 == 0o500 || meta.uid() != own() {
        return Ok(());
    }
    fs::set_permissions(folder, fs::Permissions::from_mode(mode | 0o700))
}

/// Opens every folder of the worktree, since git passes over the files of one it cannot read.
fn open_all(worktree: &Path, own: fn() -> u32) -> Result<(), String> {
    let mut folders = vec![worktree.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let unread = |error| format!("cannot read {}: {error}", folder.display());
        open_folder(&folder, own).map_err(unread)?;
        for entry in fs::read_dir(&folder).map_err(unread)? {
            let entry = entry.map_err(unread)?;
            if entry.file_type().map_err(unread)?.is_dir() {
                folders.push(entry.path());
            }
        }
    }
    Ok(())
}

/// The first folder at or under `path` with a `.git` entry or a bare repository in it, except in a
/// cargo target's `tmp`. It opens the folders the daemon owns and cannot read.
fn repository_in(path: &Path, deadline: Option<Instant>) -> Result<Option<PathBuf>, String> {
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let unread = |error| format!("cannot read {}: {error}", folder.display());
        open_folder(&folder, uid).map_err(unread)?;
        let scratch = scratch(&folder);
        for entry in fs::read_dir(&folder).map_err(unread)? {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(slow_walk(path));
            }
            let entry = entry.map_err(unread)?;
            if entry.file_name() == ".git" || entry.file_name() == "HEAD" && bare(&folder) {
                return Ok(Some(folder));
            }
            if entry.file_type().map_err(unread)?.is_dir() && Some(entry.path()) != scratch {
                folders.push(entry.path());
            }
        }
    }
    Ok(None)
}

/// Whether `folder` still is a repository.
fn repository(folder: &Path) -> bool {
    fs::symlink_metadata(folder.join(".git")).is_ok() || bare(folder)
}

/// Whether `folder` is laid out as a bare repository.
fn bare(folder: &Path) -> bool {
    ["HEAD", "objects", "refs"]
        .iter()
        .all(|name| fs::symlink_metadata(folder.join(name)).is_ok())
}

/// The files of a worktree as its index and its ignore rules list them, relative to it.
#[derive(Default)]
struct Files {
    tracked: Vec<PathBuf>,
    ignored: Vec<PathBuf>,
    /// The index names a submodule.
    submodule: bool,
}

fn files(found: &Found) -> Result<Files, String> {
    let path = |name: &[u8]| PathBuf::from(std::ffi::OsString::from_vec(name.to_vec()));
    let staged = soon_raw(found.git().args(["ls-files", "--stage", "-z"]))?;
    let staged: Vec<_> = staged
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let tab = entry.iter().position(|byte| *byte == b'\t')?;
            Some((&entry[..tab], &entry[tab + 1..]))
        })
        .collect();
    let ignored = soon_raw(found.git().args([
        "--no-optional-locks",
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "-z",
    ]))?;
    Ok(Files {
        submodule: staged.iter().any(|(mode, _)| mode.starts_with(b"160000 ")),
        tracked: staged.iter().map(|(_, file)| path(file)).collect(),
        ignored: ignored
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(path)
            .collect(),
    })
}

/// The folder of a repository the branch of the worktree does not hold: one in an ignored file, as
/// `repository_in` finds it, or a `.git` in a folder of tracked files. A tracked folder laid out as a
/// bare repository is on the branch. It walks every ignored file, so call it off the tick.
fn unsaved_repository(worktree: &Path, files: &Files) -> Result<Option<PathBuf>, String> {
    let folders: HashSet<&Path> = files
        .tracked
        .iter()
        .flat_map(|file| file.ancestors().skip(1))
        .filter(|folder| !folder.as_os_str().is_empty())
        .collect();
    for folder in folders {
        let folder = worktree.join(folder);
        if fs::symlink_metadata(folder.join(".git")).is_ok() {
            return Ok(Some(folder));
        }
    }
    for entry in &files.ignored {
        let path = worktree.join(entry);
        let meta = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if meta.is_dir()
            && let Some(folder) = repository_in(&path, None)?
        {
            return Ok(Some(folder));
        }
        let part = ["HEAD", "objects", "refs"]
            .iter()
            .any(|name| path.file_name().is_some_and(|file| file == *name));
        if let Some(parent) = path.parent().filter(|parent| part && bare(parent)) {
            return Ok(Some(parent.to_path_buf()));
        }
    }
    Ok(None)
}

/// As `unsaved_repository`, in every folder, so it also finds what appeared after `files` was read.
fn repository_left(root: &Path, files: &Files) -> Result<Option<PathBuf>, String> {
    let ignored: HashSet<&Path> = files.ignored.iter().map(PathBuf::as_path).collect();
    let tracked: HashSet<&Path> = files
        .tracked
        .iter()
        .flat_map(|file| file.ancestors())
        .collect();
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let unread = |error| format!("cannot read {}: {error}", folder.display());
        let at = folder.strip_prefix(root).unwrap_or(&folder);
        let on_branch = || {
            ["HEAD", "objects", "refs"].iter().all(|name| {
                let part = at.join(name);
                tracked.contains(part.as_path()) && !ignored.contains(part.as_path())
            })
        };
        open_folder(&folder, uid).map_err(unread)?;
        let scratch = at
            .ancestors()
            .any(|part| ignored.contains(part))
            .then(|| scratch(&folder))
            .flatten();
        for entry in fs::read_dir(&folder).map_err(unread)? {
            let entry = entry.map_err(unread)?;
            let name = entry.file_name();
            if name == ".git" || name == "HEAD" && bare(&folder) && !on_branch() {
                return Ok(Some(folder));
            }
            if entry.file_type().map_err(unread)?.is_dir() && Some(entry.path()) != scratch {
                folders.push(entry.path());
            }
        }
    }
    Ok(None)
}

/// A walk that found a repository.
struct Walk {
    /// The worktree's HEAD, index and the folders down to the repository, hashed.
    of: u64,
    folder: PathBuf,
}

static WALKS: Mutex<Option<HashMap<PathBuf, Walk>>> = Mutex::new(None);

fn walks() -> MutexGuard<'static, Option<HashMap<PathBuf, Walk>>> {
    WALKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a kept answer depends on: HEAD, the index, and each folder from the worktree to `folder`.
fn mark(worktree: &Path, folder: &Path) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;
    let head = soon(git(worktree).args(["rev-parse", "HEAD"]))?;
    let index =
        soon(git(worktree).args(["rev-parse", "--path-format=absolute", "--git-path", "index"]))?;
    let mut hasher = DefaultHasher::new();
    head.hash(&mut hasher);
    let folders = folder
        .ancestors()
        .take_while(|part| part.starts_with(worktree));
    for path in std::iter::once(Path::new(index.trim())).chain(folders) {
        let meta = fs::symlink_metadata(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        (meta.ino(), meta.mtime(), meta.mtime_nsec(), meta.size()).hash(&mut hasher);
    }
    Ok(hasher.finish())
}

/// The repository the last walk found, while what kept the worktree did not change.
fn known(worktree: &Path) -> Option<PathBuf> {
    let known = walks()
        .as_ref()
        .and_then(|walks| walks.get(worktree))
        .map(|walk| (walk.of, walk.folder.clone()));
    let (of, folder) = known?;
    (repository(&folder) && mark(worktree, &folder) == Ok(of)).then_some(folder)
}

/// What `unsaved_repository` says, kept for `known`.
fn walked(worktree: &Path, files: &Files) -> Result<Option<PathBuf>, String> {
    let found = unsaved_repository(worktree, files)?;
    let mut walk = None;
    if let Some(folder) = &found {
        // One that went since the walk would otherwise keep the worktree for good.
        walk = mark(worktree, folder)
            .ok()
            .filter(|_| repository(folder))
            .map(|of| Walk {
                of,
                folder: folder.clone(),
            });
    }
    let mut walks = walks();
    let walks = walks.get_or_insert_default();
    match walk {
        Some(walk) => walks.insert(worktree.to_owned(), walk),
        None => walks.remove(worktree),
    };
    Ok(found)
}

/// As `no_repository` for a worktree without its gitdir, whose own `.git` does not count.
fn orphan_without_repository(path: &Path) -> Result<(), String> {
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

/// Clears the place of the worktree `path` without its gitdir: deletes it, or moves it to the trash
/// and logs why when it holds a repository or cannot go.
pub(crate) fn clear_orphan(home: &Path, path: &Path, mounts: Mounts) -> Result<(), String> {
    let checked = orphan_without_repository(path);
    // The walk takes a while, in which a mount can appear.
    unmounted(path, mounts)?;
    if checked.is_ok() {
        discard_aside(home, path);
    }
    if path.exists() {
        let to = move_aside(home, path)?;
        let why = checked
            .err()
            .unwrap_or_else(|| "it cannot move to the reclaim folder".to_owned());
        eprintln!(
            "priory daemon: moved {}, which has no entry in its clone, to {}: {why}",
            path.display(),
            to.display()
        );
    }
    Ok(())
}

/// Whether a mount is at or under the worktree, or the table of mounts cannot be read.
pub fn mounted(found: &Found) -> bool {
    unmounted(&found.path, &mountinfo).is_err()
}

/// Reads a `mountinfo` table.
type Mounts<'a> = &'a dyn Fn() -> std::io::Result<Vec<u8>>;

pub(crate) fn mountinfo() -> std::io::Result<Vec<u8>> {
    fs::read("/proc/self/mountinfo")
}

/// Removes the checkout `path` of `clone`, whatever it holds but a mount. One whose folder is gone
/// loses its entry.
pub fn remove_checkout(clone: &Path, path: &Path) -> Result<(), String> {
    let gone = fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound);
    if !gone {
        unmounted(path, &mountinfo)?;
    }
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

/// The task a worktree folder is named after: `<task>`, or `<runner>-<task>` with runners.
pub fn task_of(name: &str) -> Option<i64> {
    name.rsplit('-').next()?.parse().ok()
}

/// Renames the folder `path` of `task` into `home/reclaim`, or beside it on another filesystem, and
/// returns where.
fn aside(home: &Path, path: &Path, task: i64) -> Result<PathBuf, String> {
    let failed = |error| format!("cannot move {}: {error}", path.display());
    let aside = reclaim_path(home, task)?;
    match fs::rename(path, &aside) {
        Ok(()) => Ok(aside),
        Err(error) if error.kind() == ErrorKind::CrossesDevices => {
            let beside = path.with_file_name(aside.file_name().unwrap_or_default());
            fs::rename(path, &beside).map_err(failed)?;
            Ok(beside)
        }
        Err(error) => Err(failed(error)),
    }
}

/// Deletes the task folder `path`, a worktree without its gitdir that holds no repository: a rename,
/// into `home/reclaim` or beside it on another filesystem, then a thread. It stays when neither
/// works, and the caller moves it.
fn discard_aside(home: &Path, path: &Path) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let Some(aside) = task_of(&name).and_then(|task| aside(home, path, task).ok()) else {
        return;
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

/// Why the worktree must stay although its task is done with it: a mount under it, what git would
/// refuse to remove, a repository its branch does not hold, or what `unsettled` says. It walks every
/// ignored file, so call it off the tick.
pub fn removable(home: &Path, found: &Found) -> Result<(), String> {
    removable_under(home, found, &mountinfo)
}

fn removable_under(home: &Path, found: &Found, mounts: Mounts) -> Result<(), String> {
    // So the walk enters no mount.
    unmounted(&found.path, mounts)?;
    if let Some(folder) = as_one(|| known(&found.path)) {
        return Err(holds_repository(&folder));
    }
    if occupied(&found.path) {
        return Err("a process runs in it".to_owned());
    }
    open_all(&found.path, uid)?;
    let files = as_one(|| {
        let files = files(found)?;
        refused_by_git(found, &files).map(|()| files)
    })?;
    if let Some(folder) = walked(&found.path, &files)? {
        return Err(holds_repository(&folder));
    }
    as_one(|| unsettled(home, found, &[]))?.map_or(Ok(()), Err)
}

/// Refuses what `git worktree remove` refuses and `unsettled` does not: a lock or a submodule.
fn refused_by_git(found: &Found, files: &Files) -> Result<(), String> {
    let gitdir = soon(found.git().args(["rev-parse", "--absolute-git-dir"]))?;
    let gitdir = Path::new(gitdir.trim());
    if fs::symlink_metadata(gitdir.join("locked")).is_ok() {
        return Err("its worktree is locked".to_owned());
    }
    let modules = fs::read_dir(gitdir.join("modules")).is_ok_and(|mut dir| dir.next().is_some());
    if files.submodule || modules {
        return Err("it holds a submodule".to_owned());
    }
    Ok(())
}

/// A worktree moved aside, for `detach` and then `dispose`.
pub struct Taken {
    aside: PathBuf,
    found: Found,
    /// The mode of its folder.
    before: fs::Permissions,
    files: Files,
    held: Held,
    /// Why its branch, which was to go too, stays.
    pub branch_stays: Option<String>,
    /// `detach` left files elsewhere than it found them.
    pub displaced: bool,
}

/// What this process still checks or deletes: each folder moved aside, or branch.
static HELD: Mutex<Vec<Hold>> = Mutex::new(Vec::new());

struct Hold {
    path: PathBuf,
    /// The clone and the worktree's place in it, while a `git worktree prune` there or a new
    /// worktree for it would break the removal.
    needs: Option<(PathBuf, PathBuf)>,
}

fn all_held() -> MutexGuard<'static, Vec<Hold>> {
    HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Keeps its place in `HELD` until it drops, even when the delete panics.
struct Held(PathBuf);

impl Held {
    fn new(path: PathBuf, clone: PathBuf, worktree: PathBuf) -> Held {
        all_held().push(Hold {
            path: path.clone(),
            needs: Some((clone, worktree)),
        });
        Held(path)
    }

    /// Lets the starts of its clone go on.
    fn release(&self) {
        for hold in all_held().iter_mut().filter(|hold| hold.path == self.0) {
            hold.needs = None;
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        all_held().retain(|hold| hold.path != self.0);
    }
}

/// Whether this process still checks or deletes the folder `aside`.
pub fn in_hand(aside: &Path) -> bool {
    all_held().iter().any(|hold| hold.path == aside)
}

/// Whether a removal in `clone` still needs its worktree's entry or its branch.
pub(crate) fn removes_in(clone: &Path) -> bool {
    let held = all_held();
    let mut needs = held.iter().filter_map(|hold| hold.needs.as_ref());
    needs.any(|(held, _)| held == clone)
}

/// Whether the worktree whose place is `path` is away for a removal that may move it back.
pub(crate) fn removes_at(path: &Path) -> bool {
    let held = all_held();
    let mut needs = held.iter().filter_map(|hold| hold.needs.as_ref());
    needs.any(|(_, held)| held == path)
}

/// Moves the worktree that `removable` allowed aside. It runs no git, so it fits a tick: `detach`
/// does the rest.
pub fn take(home: &Path, found: &Found) -> Result<Taken, String> {
    take_under(home, found, &mountinfo)
}

fn take_under(home: &Path, found: &Found, mounts: Mounts) -> Result<Taken, String> {
    let path = &found.path;
    let task = task_of(&found.name).ok_or_else(|| format!("no task {}", found.name))?;
    unmounted(path, mounts)?;
    // The move would break what a process does there by its path.
    if occupied(path) {
        return Err("a process runs in it".to_owned());
    }
    let failed = |error| format!("cannot move {}: {error}", path.display());
    // Moving a folder to another parent rewrites its `..`, which a read-only folder refuses.
    let before = fs::symlink_metadata(path).map_err(failed)?.permissions();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(failed)?;
    let aside = match aside(home, path, task) {
        Ok(aside) => aside,
        Err(reason) => {
            let _ = fs::set_permissions(path, before);
            return Err(reason);
        }
    };
    Ok(Taken {
        held: Held::new(aside.clone(), found.clone_in(home), found.path.clone()),
        aside,
        found: found.clone(),
        before,
        files: Files::default(),
        branch_stays: None,
        displaced: false,
    })
}

/// Drops the entry of a worktree moved aside, once every check but the walk passes on the moved
/// folder. Otherwise it moves back. `tip` is the HEAD that was judged. With `branch`, its branch goes
/// too, unless git refuses to lose a commit. Git can hang, so call it off the tick.
pub fn detach(
    home: &Path,
    taken: &mut Taken,
    tip: Option<&str>,
    branch: bool,
) -> Result<(), String> {
    if let Err(reason) = drop_entry(home, taken, tip) {
        return Err(put_back(home, taken, reason));
    }
    if branch {
        taken.branch_stays = as_one(|| remove_branch(home, &taken.found)).err();
    }
    taken.held.release();
    Ok(())
}

/// Moves the worktree back that stays for `reason`. Returns `reason` and what else moved.
fn put_back(home: &Path, taken: &mut Taken, reason: String) -> String {
    let (path, aside) = (taken.found.path.clone(), taken.aside.clone());
    let stays = |error| format!("it stays in {}: {error}", aside.display());
    // The entry is now that worktree's, so the moved one has none.
    if entry_of(home, &taken.found, &path.join(".git")).is_some() {
        taken.displaced = true;
        let end = match move_aside(home, &aside) {
            Ok(_) => "the removed one moved to the trash folder".to_owned(),
            Err(error) => stays(error),
        };
        return format!("{reason}; another worktree took its place, so {end}");
    }
    let mut reason = reason;
    let mut moved = fs::rename(&aside, &path);
    if moved.is_err() && fs::symlink_metadata(&path).is_ok() {
        match clear(home, &path) {
            Ok(to) => {
                taken.displaced = true;
                reason = format!("{reason}; what took its place moved to {to}");
                moved = fs::rename(&aside, &path);
            }
            Err(error) => reason = format!("{reason}; {error}"),
        }
    }
    if let Err(error) = moved {
        taken.displaced = true;
        return format!("{reason}; {}", stays(error.to_string()));
    }
    let _ = fs::set_permissions(&path, taken.before.clone());
    if !lost_entry(&path) {
        return reason;
    }
    taken.displaced = true;
    trashed(home, &path, &aside, reason)
}

/// Moves what took a worktree's place `path` to the trash, or beside it on another filesystem.
/// Returns where.
fn clear(home: &Path, path: &Path) -> Result<String, String> {
    let Err(error) = move_aside(home, path) else {
        return Ok("the trash folder".to_owned());
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let beside = path.with_file_name(format!("{name}-{stamp}"));
    match fs::rename(path, &beside) {
        Ok(()) => Ok(beside.display().to_string()),
        Err(_) => Err(error),
    }
}

/// Whether the entry that the worktree `path` names is gone, as a `git worktree prune` leaves one
/// whose folder is elsewhere.
fn lost_entry(path: &Path) -> bool {
    let entry = real(path).and_then(|at| linked(&path.join(".git"), &at));
    entry.is_some_and(|entry| fs::symlink_metadata(entry).is_err())
}

/// Moves `path` to the trash, or to `aside` on another filesystem, since a run's start deletes a
/// worktree without its entry. Returns `reason` and where it is.
fn trashed(home: &Path, path: &Path, aside: &Path, reason: String) -> String {
    match move_aside(home, path) {
        Ok(_) => {
            format!("{reason}; its entry in the clone is gone, so it moved to the trash folder")
        }
        Err(error) => match fs::rename(path, aside) {
            Ok(()) => format!("{reason}; {error}; it stays in {}", aside.display()),
            Err(_) => format!("{reason}; {error}"),
        },
    }
}

/// Checks the moved folder, which no later change reaches by the old path, and drops its entry in
/// the clone.
fn drop_entry(home: &Path, taken: &mut Taken, tip: Option<&str>) -> Result<(), String> {
    let failed = |error| format!("cannot move {}: {error}", taken.aside.display());
    unmounted(&taken.aside, &mountinfo)?;
    let link = taken.aside.join(".git");
    let entry = entry_of(home, &taken.found, &link)
        .ok_or_else(|| format!("{} has no entry in its clone", taken.found.path.display()))?;
    // Held open, so no entry that git makes in its place gets its inode.
    let handle = fs::File::open(&entry)
        .and_then(|file| Ok((file.metadata()?, file)))
        .map_err(|error| format!("cannot open {}: {error}", entry.display()))?;
    let moved = Found {
        path: fs::canonicalize(&taken.aside).map_err(failed)?,
        entry: Some(entry.clone()),
        ..taken.found.clone()
    };
    taken.files = as_one(|| checked(home, &moved, tip))?;
    if !whole(&entry) {
        return Err(format!("{} is not whole", entry.display()));
    }
    free(&taken.found.path)?;
    // Before the entry goes, so a daemon that stops in between leaves a folder the sweep can delete.
    let kept = kept_link(&taken.aside);
    fs::rename(&link, &kept).map_err(failed)?;
    match take_entry(taken, &entry, &handle.0) {
        Ok(gone) => {
            // A start's `git worktree prune` takes what stays of it.
            let _ = fs::remove_dir_all(gone);
            let _ = fs::remove_file(kept);
            Ok(())
        }
        Err(reason) => {
            fs::rename(&kept, &link)
                .map_err(|error| format!("cannot move {}: {error}", kept.display()))?;
            Err(reason)
        }
    }
}

/// Refuses a worktree's place that something took.
fn free(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("cannot check {}: {error}", path.display())),
        Ok(_) => Err(format!("something took its place {}", path.display())),
    }
}

/// Renames the checked `entry`, whose folder is `checked`, out of git's way, and returns where. What
/// a git changed since its check puts it back.
fn take_entry(taken: &Taken, entry: &Path, checked: &fs::Metadata) -> Result<PathBuf, String> {
    let failed = |error| format!("cannot move {}: {error}", entry.display());
    let name = taken
        .aside
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let gone = entry.with_file_name(format!("removed-{name}"));
    fs::rename(entry, &gone).map_err(failed)?;
    let same = fs::symlink_metadata(&gone)
        .is_ok_and(|meta| (meta.dev(), meta.ino()) == (checked.dev(), checked.ino()));
    let unlocked = fs::symlink_metadata(gone.join("locked")).is_err();
    let place = real(&taken.found.path);
    let names = named(&gone).and_then(|path| real(&path));
    let reason = match free(&taken.found.path) {
        Err(reason) => reason,
        Ok(()) if same && unlocked && place.is_some() && names == place => return Ok(gone),
        Ok(()) => format!("{} changed during the sweep", entry.display()),
    };
    // Not over the entry of a worktree git just added.
    if fs::symlink_metadata(entry).is_err() {
        fs::rename(&gone, entry).map_err(failed)?;
    }
    Err(reason)
}

/// Where the `.git` of the folder `aside` waits until the folder is gone.
fn kept_link(aside: &Path) -> PathBuf {
    aside.with_extension("link")
}

/// Whether `entry`, of a worktree in its clone, has all git needs of it.
fn whole(entry: &Path) -> bool {
    ["HEAD", "commondir", "gitdir", "index"]
        .iter()
        .all(|file| fs::symlink_metadata(entry.join(file)).is_ok())
}

/// `path` without its `.` and `..`, for one whose folders are gone.
fn normal(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            part => normal.push(part),
        }
    }
    normal
}

fn read_line(path: &Path) -> Option<String> {
    let read = String::from_utf8(read_regular(path, 1 << 16).ok()?).ok()?;
    Some(read.trim_end_matches(['\n', '\r']).to_owned())
}

/// The entry that `link`, the `.git` of a worktree that was at `at`, names.
fn linked(link: &Path, at: &Path) -> Option<PathBuf> {
    let read = read_line(link)?;
    Some(normal(&at.join(read.strip_prefix("gitdir: ")?)))
}

/// The worktree that `entry` in its clone names.
fn named(entry: &Path) -> Option<PathBuf> {
    let link = normal(&entry.join(read_line(&entry.join("gitdir"))?));
    Some(link.parent()?.to_path_buf())
}

/// Where a path whose folder may be gone really is.
fn real(path: &Path) -> Option<PathBuf> {
    Some(
        fs::canonicalize(path.parent()?)
            .ok()?
            .join(path.file_name()?),
    )
}

/// The entry in the clone of the worktree `found`, whose `.git` moved to `link`. An agent can write
/// that file, so it counts only when the entry is the clone's and names the worktree.
fn entry_of(home: &Path, found: &Found, link: &Path) -> Option<PathBuf> {
    let from = real(&found.path)?;
    let entry = real(&linked(link, &from)?)?;
    let registry = fs::canonicalize(found.clone_in(home).join(".git/worktrees")).ok()?;
    let folder = fs::symlink_metadata(&entry).is_ok_and(|meta| meta.is_dir());
    let ours = entry.parent() == Some(&registry) && real(&named(&entry)?)? == from;
    (folder && ours).then_some(entry)
}

/// The files of the worktree, when nothing but its walk would keep it.
fn checked(home: &Path, found: &Found, tip: Option<&str>) -> Result<Files, String> {
    let files = files(found)?;
    refused_by_git(found, &files)?;
    let head = soon(found.git().args(["rev-parse", "HEAD"]))?;
    if tip.is_some_and(|tip| tip != head.trim()) {
        return Err("its HEAD moved during the sweep".to_owned());
    }
    unsettled(home, found, &[])?.map_or(Ok(files), Err)
}

/// Moves a worktree back that a stopped daemon left at `aside` before its entry in the clone went,
/// when nothing took its place. Returns whether it did.
pub fn moved_back(home: &Path, aside: &Path) -> bool {
    back(home, aside).is_some()
}

fn back(home: &Path, aside: &Path) -> Option<()> {
    let (link, kept) = (aside.join(".git"), kept_link(aside));
    let waits = fs::symlink_metadata(&link).is_err();
    // A relative link leads from a worktree's folder, which are all as deep.
    let at = fs::canonicalize(home).ok()?.join("worktrees/o/n/t");
    let entry = linked(if waits { &kept } else { &link }, &at)?;
    let from = named(&entry)?;
    let found = found_at(home, &from)?;
    // An agent can write both files, so they choose no place but the free one of this task.
    let task = task_of_aside(aside)?;
    let ours = task_of(&found.name) == Some(task)
        && entry_of(home, &found, if waits { &kept } else { &link }).as_ref()
            == Some(&real(&entry)?);
    if !ours || fs::symlink_metadata(&from).is_ok() {
        return None;
    }
    if waits {
        // Git began to delete an entry that is not whole, after every check passed.
        if !whole(&entry) {
            return None;
        }
        fs::rename(&kept, &link).ok()?;
    }
    fs::rename(aside, from).ok()
}

/// The task that `aside` in `home/reclaim` is named after.
fn task_of_aside(aside: &Path) -> Option<i64> {
    let name = aside.file_name()?.to_str()?;
    name.split_once('-')?.0.parse().ok()
}

/// Deletes the links in `home/reclaim` whose folder is gone.
pub fn drop_links(home: &Path) {
    let Ok(folder) = reclaim_folder(home) else {
        return;
    };
    for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        let aside = path.with_extension("");
        let ours = path
            .extension()
            .is_some_and(|extension| extension == "link")
            && aside
                .file_name()
                .is_some_and(|name| reclaim_name(&name.to_string_lossy()));
        if ours && fs::symlink_metadata(&aside).is_err() && !in_hand(&aside) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Whether the daemon gave `name` in `home/reclaim`: a task and nanoseconds, which have 18 digits or
/// more.
pub fn reclaim_name(name: &str) -> bool {
    let digits =
        |part: &str, least| part.len() >= least && part.bytes().all(|byte| byte.is_ascii_digit());
    name.split_once('-')
        .is_some_and(|(task, nanoseconds)| digits(task, 1) && digits(nanoseconds, 18))
}

/// The branch of a removed worktree, whose delete ran past its limit. Until it drops, a start that
/// adds the worktree back waits.
pub struct Branch {
    found: Found,
    held: Held,
}

impl Branch {
    /// The branch of `found`, unless its worktree is back.
    pub fn of(home: &Path, found: &Found) -> Option<Branch> {
        fs::symlink_metadata(&found.path).is_err().then(|| Branch {
            held: Held::new(found.path.clone(), found.clone_in(home), found.path.clone()),
            found: found.clone(),
        })
    }

    /// Deletes it, and returns its worktree when that ran past its limit again.
    pub fn delete(self, home: &Path) -> Option<Found> {
        let cut = as_one(|| remove_branch(home, &self.found));
        self.held.release();
        cut.is_err_and(|error| timed_out(&error))
            .then_some(self.found)
    }
}

/// The worktree folder of `home` that goes at `path`.
fn found_at(home: &Path, path: &Path) -> Option<Found> {
    let worktrees = fs::canonicalize(home.join("worktrees")).ok()?;
    let folder = fs::canonicalize(path.parent()?).ok()?;
    let repo = folder.strip_prefix(worktrees).ok()?;
    let name = path.file_name()?.to_str()?;
    task_of(name)?;
    (repo.components().count() == 2).then_some(())?;
    Some(Found {
        path: home.join("worktrees").join(repo).join(name),
        repo: repo.to_str()?.to_owned(),
        name: name.to_owned(),
        entry: None,
    })
}

/// Deletes a worktree moved aside once no mount and no repository is in it. One that has either
/// moves to `home/trash`. It walks every file, so call it off the tick.
pub fn dispose(home: &Path, taken: &Taken) -> Result<(), String> {
    dispose_under(home, taken, &mountinfo)
}

fn dispose_under(home: &Path, taken: &Taken, mounts: Mounts) -> Result<(), String> {
    let aside = &taken.aside;
    let checked =
        unmounted(aside, mounts).and_then(|()| match repository_left(aside, &taken.files)? {
            Some(folder) => Err(holds_repository(&folder)),
            None => Ok(()),
        });
    let reason = match checked {
        Ok(()) => return discard(aside),
        Err(reason) => reason,
    };
    let end = match move_aside(home, aside) {
        Ok(_) => "it moved to the trash folder".to_owned(),
        Err(error) => error,
    };
    Err(format!("{}: {reason}; {end}", taken.found.path.display()))
}

/// Why removing the worktree would lose a file or a process's folder: everything else is on its
/// branch. Untracked files inside the `build` folders do not count.
fn unsettled(home: &Path, found: &Found, build: &[String]) -> Result<Option<String>, String> {
    let clone = found.clone_in(home);
    let path = &found.path;
    let common =
        soon(
            found
                .git()
                .args(["rev-parse", "--path-format=absolute", "--git-common-dir"]),
        );
    let of = match common {
        Err(error) if timed_out(&error) => return Err(error),
        common => common
            .ok()
            .and_then(|common| fs::canonicalize(Path::new(common.trim()).parent()?).ok()),
    };
    if of != fs::canonicalize(&clone).ok() {
        return Ok(Some(format!("not a worktree of {}", clone.display())));
    }
    let branch = found.branch();
    if soon(found.git().args(["branch", "--show-current"]))?.trim() != branch {
        return Ok(Some(format!("not on branch {branch}")));
    }
    // Without it, the walk's status could hold `index.lock` while a run starts.
    let status = soon(found.git().args([
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

/// Deletes the branch of a worktree that went. Git refuses when it would lose a commit.
fn remove_branch(home: &Path, found: &Found) -> Result<(), String> {
    let clone = found.clone_in(home);
    let branch = found.branch();
    // A task branch tracks nothing, so `-d` alone would judge the merge against the clone's HEAD.
    let merged = is_ancestor(&clone, &branch, "origin/HEAD")?;
    let delete = if merged { "-D" } else { "-d" };
    soon(git(&clone).args(["branch", delete, &branch])).map(drop)
}

#[cfg(test)]
pub(crate) fn retire(home: &Path, found: &Found) -> Result<(), String> {
    retire_under(home, found, &mountinfo)
}

#[cfg(test)]
fn retire_under(home: &Path, found: &Found, mounts: Mounts) -> Result<(), String> {
    removable_under(home, found, mounts)?;
    let mut taken = take_under(home, found, mounts)?;
    detach(home, &mut taken, None, false)?;
    dispose_under(home, &taken, mounts)
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
            entry: None,
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
                entry: None,
            };
            (Home(dir), found)
        }
    }

    fn detached(
        home: &Path,
        found: &Found,
        tip: Option<&str>,
        branch: bool,
    ) -> Result<Taken, String> {
        let mut taken = take(home, found)?;
        detach(home, &mut taken, tip, branch)?;
        Ok(taken)
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
        let read = || Ok(table.clone().into_bytes());

        let refused = take_under(&home.0, &found, &read).err().unwrap();

        assert_eq!(refused, format!("{} is a mount", point.display()));
        assert!(found.path.exists());

        // And one that appears once the worktree moved.
        let taken = detached(&home.0, &found, None, false).unwrap();
        let point = fs::canonicalize(&taken.aside).unwrap().join("target");
        let table = format!("23 22 0:5 / {} rw - tmpfs tmpfs rw\n", point.display());
        let read = || Ok(table.clone().into_bytes());
        let refused = dispose_under(&home.0, &taken, &read).unwrap_err();
        assert!(refused.contains("target is a mount"), "{refused}");
        let kept = fs::read_dir(home.0.join("trash")).unwrap();
        let kept: Vec<_> = kept.flatten().map(|entry| entry.path()).collect();
        assert!(kept.len() == 1 && kept[0].join("fixture/HEAD").exists());
    }

    /// The change `late` makes in the worktree after its walk keeps it whole, for `reason`.
    fn kept_by(name: &str, reason: &str, late: impl FnOnce(&Found)) {
        let (home, found) = Home::new(name);
        removable(&home.0, &found).unwrap();
        let tip = run(git(&found.path).args(["rev-parse", "HEAD"])).unwrap();
        late(&found);

        let refused = detached(&home.0, &found, Some(tip.trim()), true)
            .err()
            .unwrap();

        assert!(refused.contains(reason), "{refused}");
        assert!(found.path.join("fixture/HEAD").exists());
        assert!(home.0.join("repos/o/n/.git/worktrees/1").exists());
    }

    #[test]
    fn a_worktree_that_changed_after_its_walk_stays() {
        kept_by("late-file", "untracked files", |found| {
            fs::write(found.path.join("new"), "x").unwrap();
        });
        kept_by("late-edit", "uncommitted changes", |found| {
            fs::write(found.path.join("fixture/HEAD"), "y").unwrap();
        });
        kept_by("late-commit", "its HEAD moved during the sweep", |found| {
            sh(
                &found.path,
                &["commit", "-q", "--allow-empty", "-m", "late"],
            );
        });
        kept_by("late-branch", "not on branch priory/1", |found| {
            sh(&found.path, &["switch", "-q", "-c", "other"]);
        });
        kept_by("late-lock", "its worktree is locked", |found| {
            sh(&found.path, &["worktree", "lock", "."]);
        });
        let mut process = None;
        kept_by("late-process", "a process runs in it", |found| {
            let mut sleep = std::process::Command::new("sleep");
            process = Some(sleep.arg("60").current_dir(&found.path).spawn().unwrap());
            let cwd = PathBuf::from(format!("/proc/{}/cwd", process.as_ref().unwrap().id()));
            let entered = fs::canonicalize(&found.path).unwrap();
            while fs::read_link(&cwd).ok().as_ref() != Some(&entered) {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let mut process = process.unwrap();
        process.kill().unwrap();
        process.wait().unwrap();
    }

    #[test]
    fn a_worktree_with_a_submodule_stays() {
        let (home, found) = Home::new("submodule");
        let sub = found.path.join("sub");
        fs::create_dir_all(&sub).unwrap();
        sh(&sub, &["init", "-q"]);
        sh(&sub, &["commit", "-q", "--allow-empty", "-m", "sub"]);
        sh(
            &found.path,
            &["-c", "advice.addEmbeddedRepo=false", "add", "sub"],
        );
        sh(&found.path, &["commit", "-q", "-m", "sub"]);

        let refused = removable(&home.0, &found).unwrap_err();

        assert_eq!(refused, "it holds a submodule");
        assert!(detached(&home.0, &found, None, false).is_err());
    }

    #[test]
    fn a_repository_made_after_the_checks_moves_to_the_trash_whole() {
        let (home, found) = Home::new("late-repository");
        removable(&home.0, &found).unwrap();
        let taken = detached(&home.0, &found, None, false).unwrap();
        assert!(!found.path.exists());
        assert!(!home.0.join("repos/o/n/.git/worktrees/1").exists());
        sh(&taken.aside, &["init", "-q", "target/n"]);

        let refused = dispose(&home.0, &taken).unwrap_err();

        assert!(
            refused.ends_with("holds a git repository; it moved to the trash folder"),
            "{refused}"
        );
        assert!(!taken.aside.exists());
        let trash = fs::read_dir(home.0.join("trash")).unwrap().next().unwrap();
        assert!(trash.unwrap().path().join("target/n/.git/HEAD").exists());
    }

    #[test]
    fn a_change_between_the_last_check_and_the_move_puts_the_worktree_back() {
        let (home, found) = Home::new("window");
        removable(&home.0, &found).unwrap();
        let mut taken = take(&home.0, &found).unwrap();
        let aside = taken.aside.clone();
        fs::write(aside.join("new"), "x").unwrap();
        let refused = drop_entry(&home.0, &mut taken, None).unwrap_err();
        assert_eq!(refused, "untracked files");
        assert!(home.0.join("repos/o/n/.git/worktrees/1").exists());

        // A repository in an ignored folder that the first check did not list.
        fs::remove_file(aside.join("new")).unwrap();
        sh(&aside, &["init", "-q", "target/n"]);
        drop_entry(&home.0, &mut taken, None).unwrap();
        let refused = dispose(&home.0, &taken).unwrap_err();
        assert!(
            refused.contains("target/n holds a git repository"),
            "{refused}"
        );
    }

    #[test]
    fn a_late_bare_repository_around_a_tracked_head_moves_to_the_trash() {
        let (home, found) = Home::new("late-bare");
        fs::create_dir_all(found.path.join("half")).unwrap();
        fs::write(found.path.join("half/HEAD"), "x\n").unwrap();
        sh(&found.path, &["add", "."]);
        sh(&found.path, &["commit", "-q", "-m", "half"]);
        let taken = detached(&home.0, &found, None, false).unwrap();
        for folder in ["objects", "refs"] {
            fs::create_dir_all(taken.aside.join("half").join(folder)).unwrap();
        }

        let refused = dispose(&home.0, &taken).unwrap_err();

        assert!(refused.contains("half holds a git repository"), "{refused}");
    }

    #[test]
    fn a_folder_that_took_its_place_moves_to_the_trash_and_the_worktree_goes_back() {
        let (home, found) = Home::new("taken-place");
        let mut taken = take(&home.0, &found).unwrap();
        fs::create_dir_all(found.path.join("theirs")).unwrap();
        fs::write(found.path.join("theirs/file"), "x").unwrap();
        fs::write(taken.aside.join("ours"), "x").unwrap();
        // The task's start waits, although the folder is there.
        let waits = "waits to fetch o/n: the sweep removes a worktree of it";
        let start = || crate::runtime::worktree(&home.0, "o/n", "1", None, false);
        assert_eq!(start().unwrap_err(), waits);
        let fetch_waits = || crate::runtime::fetch_waits(&home.0, "o/n", "1");
        assert_eq!(fetch_waits().as_deref(), Some(waits));

        let refused = detach(&home.0, &mut taken, None, true).unwrap_err();

        assert_eq!(
            refused,
            "untracked files; what took its place moved to the trash folder"
        );
        assert!(taken.displaced);
        assert!(found.path.join("ours").exists() && !found.path.join("theirs").exists());
        assert!(in_trash(&home.0, "theirs/file"));
        drop(taken);
        start().unwrap();
    }

    #[test]
    fn a_folder_the_trash_cannot_take_moves_beside_the_worktree() {
        let (home, found) = Home::new("taken-beside");
        fs::write(home.0.join("trash"), "").unwrap();
        let mut taken = take(&home.0, &found).unwrap();
        fs::create_dir_all(&found.path).unwrap();
        fs::write(found.path.join("theirs"), "x").unwrap();

        let refused = detach(&home.0, &mut taken, None, true).unwrap_err();

        assert!(refused.starts_with("something took its place"), "{refused}");
        assert!(found.path.join("fixture/HEAD").exists());
        let beside = fs::read_dir(found.path.parent().unwrap()).unwrap();
        assert!(beside.flatten().any(|at| at.path().join("theirs").exists()));
    }

    /// Whether a folder in the trash of `home` holds `file`.
    fn in_trash(home: &Path, file: &str) -> bool {
        let trash = fs::read_dir(home.join("trash"));
        let mut kept = trash.into_iter().flatten().flatten();
        kept.any(|entry| entry.path().join(file).exists())
    }

    #[test]
    fn a_worktree_put_back_while_the_folder_is_away_survives_with_its_nested_repository() {
        for prune in [false, true] {
            let (home, found) = Home::new("put-back");
            let clone = home.0.join("repos/o/n");
            let mut taken = take(&home.0, &found).unwrap();
            let path = found.path.to_str().unwrap();
            if prune {
                sh(&clone, &["worktree", "prune"]);
                sh(&clone, &["worktree", "add", "-q", path, "priory/1"]);
            } else {
                sh(&clone, &["worktree", "add", "-q", "-f", path, "priory/1"]);
            }
            let nested = found.path.join("target/n");
            fs::create_dir_all(&nested).unwrap();
            sh(&nested, &["init", "-q"]);
            sh(&nested, &["commit", "-q", "--allow-empty", "-m", "nested"]);

            let refused = detach(&home.0, &mut taken, None, true).unwrap_err();

            assert!(refused.contains("another worktree took its"), "{refused}");
            assert!(taken.displaced);
            run(git(&nested).args(["rev-parse", "HEAD"])).unwrap();
            run(git(&found.path).args(["status", "--porcelain"])).unwrap();
            assert!(in_trash(&home.0, "fixture/HEAD") && !taken.aside.exists());
        }
    }

    #[test]
    fn an_entry_that_changed_after_its_check_goes_back() {
        let (home, found) = Home::new("changed-entry");
        let clone = home.0.join("repos/o/n");
        let entry = clone.join(".git/worktrees/1");
        let taken = take(&home.0, &found).unwrap();
        let handle = fs::File::open(&entry).unwrap();
        let checked = handle.metadata().unwrap();

        // A worktree that git put back and a person deleted, so the place is free again.
        let path = found.path.to_str().unwrap();
        sh(&clone, &["worktree", "add", "-q", "-f", path, "priory/1"]);
        let refused = take_entry(&taken, &entry, &checked).unwrap_err();
        assert!(refused.starts_with("something took its place"), "{refused}");
        fs::remove_dir_all(&found.path).unwrap();
        let refused = take_entry(&taken, &entry, &checked).unwrap_err();
        assert!(refused.ends_with("changed during the sweep"), "{refused}");

        let checked = fs::metadata(&entry).unwrap();
        fs::write(entry.join("locked"), "").unwrap();
        take_entry(&taken, &entry, &checked).unwrap_err();
        fs::remove_file(entry.join("locked")).unwrap();
        let gitdir = fs::read(entry.join("gitdir")).unwrap();
        fs::write(entry.join("gitdir"), "/elsewhere/.git\n").unwrap();
        take_entry(&taken, &entry, &checked).unwrap_err();
        fs::write(entry.join("gitdir"), gitdir).unwrap();

        let gone = take_entry(&taken, &entry, &checked).unwrap();
        assert!(gone.join("HEAD").exists() && !entry.exists());
        // As a daemon that stops here leaves it.
        sh(&clone, &["worktree", "prune"]);
        assert!(!gone.exists());
    }

    #[test]
    fn a_worktree_a_stopped_daemon_moved_goes_back_while_its_entry_is_there() {
        let (home, found) = Home::new("stopped");
        let moved = aside(&home.0, &found.path, 1).unwrap();
        fs::create_dir_all(&found.path).unwrap();
        assert!(!moved_back(&home.0, &moved), "its place is taken");
        fs::remove_dir(&found.path).unwrap();
        assert!(moved_back(&home.0, &moved));
        removable(&home.0, &found).unwrap();

        // Nor does one of another task, or one outside the clone's entries.
        let moved = aside(&home.0, &found.path, 2).unwrap();
        assert!(!moved_back(&home.0, &moved), "the folder is task 2's");
        fs::rename(&moved, &found.path).unwrap();
        let moved = aside(&home.0, &found.path, 1).unwrap();
        let registry = home.0.join("repos/o/n/.git/worktrees/1");
        let link = fs::read(moved.join(".git")).unwrap();
        let forged = home.0.join("forged");
        fs::create_dir_all(&forged).unwrap();
        fs::copy(registry.join("gitdir"), forged.join("gitdir")).unwrap();
        fs::write(
            moved.join(".git"),
            format!("gitdir: {}\n", forged.display()),
        )
        .unwrap();
        assert!(!moved_back(&home.0, &moved), "the entry is not the clone's");
        fs::write(moved.join(".git"), link).unwrap();
        assert!(moved_back(&home.0, &moved));

        // An entry that names another place moves nothing there.
        let moved = aside(&home.0, &found.path, 1).unwrap();
        let outside = home.0.join("outside");
        let entry = home.0.join("repos/o/n/.git/worktrees/1/gitdir");
        fs::write(entry, format!("{}/.git\n", outside.display())).unwrap();
        assert!(!moved_back(&home.0, &moved));
        assert!(!outside.exists() && moved.exists());
    }

    #[test]
    fn a_link_to_another_entry_of_the_clone_moves_the_worktree_back() {
        let (home, found) = Home::new("other-entry");
        let clone = home.0.join("repos/o/n");
        let other = home.0.join("worktrees/o/n/2");
        let other = other.to_str().unwrap();
        sh(&clone, &["worktree", "add", "-q", "-b", "priory/2", other]);
        let entry = clone.join(".git/worktrees/2");
        fs::write(
            found.path.join(".git"),
            format!("gitdir: {}\n", entry.display()),
        )
        .unwrap();
        let refused = detached(&home.0, &found, None, true).err().unwrap();
        assert!(refused.contains("has no entry"), "{refused}");
        assert!(entry.exists() && found.path.join(".git").exists());
    }

    #[test]
    fn a_worktree_with_relative_links_goes_or_moves_back_whole() {
        let (home, found) = Home::new("relative");
        // As `worktree.useRelativePaths` writes them, which git before 2.48 does not.
        let entry = home.0.join("repos/o/n/.git/worktrees/1");
        let link = "gitdir: ../../../../repos/o/n/.git/worktrees/1\n";
        fs::write(found.path.join(".git"), link).unwrap();
        let gitdir = "../../../../../../worktrees/o/n/1/.git\n";
        fs::write(entry.join("gitdir"), gitdir).unwrap();
        removable(&home.0, &found).unwrap();

        fs::write(found.path.join("new"), "x").unwrap();
        let refused = detached(&home.0, &found, None, true).err().unwrap();
        assert_eq!(refused, "untracked files");
        fs::remove_file(found.path.join("new")).unwrap();
        removable(&home.0, &found).unwrap();

        let moved = aside(&home.0, &found.path, 1).unwrap();
        assert!(moved_back(&home.0, &moved));
        removable(&home.0, &found).unwrap();

        retire(&home.0, &found).unwrap();
        assert!(!found.path.exists() && !entry.exists());
    }

    #[test]
    fn a_worktree_whose_entry_a_prune_dropped_as_it_moved_goes_to_the_trash_whole() {
        for relative in [false, true] {
            let (home, found) = Home::new("pruned");
            let clone = home.0.join("repos/o/n");
            fs::write(found.path.join("new"), "x").unwrap();
            if relative {
                let link = "gitdir: ../../../../repos/o/n/.git/worktrees/1\n";
                fs::write(found.path.join(".git"), link).unwrap();
            }
            let mut taken = take(&home.0, &found).unwrap();
            sh(&clone, &["worktree", "prune"]);

            let refused = detach(&home.0, &mut taken, None, true).unwrap_err();

            assert!(refused.contains("moved to the trash folder"), "{refused}");
            assert!(!found.path.exists());
            let trash = fs::read_dir(home.0.join("trash")).unwrap().next().unwrap();
            assert!(trash.unwrap().path().join("new").exists());
            drop(taken);
            crate::runtime::worktree(&home.0, "o/n", "1", None, false).unwrap();
        }
    }

    #[test]
    fn a_pruned_worktree_the_trash_cannot_take_stays_out_of_a_starts_way() {
        let (home, found) = Home::new("no-trash");
        fs::write(found.path.join("new"), "x").unwrap();
        // A rename into another filesystem fails as this does.
        fs::write(home.0.join("trash"), "").unwrap();
        let mut taken = take(&home.0, &found).unwrap();
        let clone = home.0.join("repos/o/n");
        sh(&clone, &["worktree", "prune"]);

        detach(&home.0, &mut taken, None, true).unwrap_err();

        assert!(!found.path.exists() && taken.aside.join("new").exists());
    }

    #[test]
    fn a_process_in_the_worktree_stops_the_move_itself() {
        let (home, found) = Home::new("enters");
        let mut sleep = std::process::Command::new("sleep")
            .arg("60")
            .current_dir(&found.path)
            .spawn()
            .unwrap();
        let refused = take(&home.0, &found).err();
        sleep.kill().unwrap();
        sleep.wait().unwrap();
        assert_eq!(refused.as_deref(), Some("a process runs in it"));
        assert!(found.path.exists() && !home.0.join(RECLAIM).exists());
    }

    #[test]
    fn a_daemon_stopped_around_the_entrys_delete_loses_nothing_it_did_not_delete() {
        let (home, found) = Home::new("stopped-late");
        let stop = || {
            let taken = take(&home.0, &found).unwrap();
            fs::rename(taken.aside.join(".git"), kept_link(&taken.aside)).unwrap();
            taken.aside.clone()
        };
        let aside = stop();
        drop_links(&home.0);
        assert!(moved_back(&home.0, &aside), "git had not begun");
        assert!(!kept_link(&aside).exists());
        removable(&home.0, &found).unwrap();

        let aside = stop();
        fs::remove_file(home.0.join("repos/o/n/.git/worktrees/1/HEAD")).unwrap();
        assert!(!moved_back(&home.0, &aside), "git had begun");
        drop_links(&home.0);
        assert!(kept_link(&aside).exists(), "its folder is there");
        discard(&aside).unwrap();
        let foreign = home.0.join(RECLAIM).join("notes.link");
        fs::write(&foreign, "").unwrap();
        drop_links(&home.0);
        assert!(!kept_link(&aside).exists() && foreign.exists());
    }

    #[test]
    fn a_start_waits_to_add_a_worktree_back_while_the_sweep_drops_its_entry() {
        let (home, found) = Home::new("start-waits");
        let mut taken = take(&home.0, &found).unwrap();
        let waits = "waits to fetch o/n: the sweep removes a worktree of it";
        let start = || crate::runtime::worktree(&home.0, "o/n", "1", None, false);
        let fetch_waits = || crate::runtime::fetch_waits(&home.0, "o/n", "1");
        assert_eq!(start().unwrap_err(), waits);
        assert_eq!(fetch_waits().as_deref(), Some(waits));
        assert!(home.0.join("repos/o/n/.git/worktrees/1").exists());

        detach(&home.0, &mut taken, None, false).unwrap();

        assert_eq!(
            fetch_waits(),
            None,
            "the delete of its files holds no start"
        );
        start().unwrap();
        assert!(found.path.join("fixture/HEAD").exists());
    }

    #[test]
    fn a_branch_whose_delete_was_cut_goes_later_unless_its_worktree_is_back() {
        let (home, found) = Home::new("branch");
        let clone = home.0.join("repos/o/n");
        assert!(Branch::of(&home.0, &found).is_none());
        sh(&clone, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        detached(&home.0, &found, None, false).unwrap();
        let branch = Branch::of(&home.0, &found).unwrap();
        assert!(removes_in(&clone));
        assert!(branch.delete(&home.0).is_none());
        assert!(!removes_in(&clone));
        assert_eq!(crate::runtime::head(&clone, "priory/1"), None);
    }

    #[test]
    fn a_moved_folder_is_in_hand_until_it_is_disposed_of() {
        let (home, found) = Home::new("in-hand");
        let taken = detached(&home.0, &found, None, true).unwrap();
        assert_eq!(taken.branch_stays, None);
        let aside = taken.aside.clone();
        assert!(in_hand(&aside));
        dispose(&home.0, &taken).unwrap();
        drop(taken);
        assert!(!in_hand(&aside) && !aside.exists());
    }

    #[test]
    fn a_home_behind_a_symlink_loses_the_worktree_and_its_entry() {
        let (home, _) = Home::new("linked");
        let link = home.0.with_extension("link");
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&home.0, &link).unwrap();
        let found = Found {
            path: link.join("worktrees/o/n/1"),
            repo: "o/n".to_owned(),
            name: "1".to_owned(),
            entry: None,
        };

        retire(&link, &found).unwrap();

        fs::remove_file(&link).unwrap();
        assert!(!home.0.join("worktrees/o/n/1").exists());
        assert!(!home.0.join("repos/o/n/.git/worktrees/1").exists());
    }

    #[test]
    fn a_kept_answer_is_walked_again_only_when_the_worktree_changed() {
        let (home, found) = Home::new("walks");
        let tree = &found.path;
        sh(tree, &["init", "-q", "target/a/n"]);
        let refused = format!(
            "{} holds a git repository",
            tree.join("target/a/n").display()
        );
        let walk = || removable(&home.0, &found).unwrap_err();
        assert_eq!(walk(), refused);
        let kept = || known(tree).is_some();
        assert!(kept());

        sh(tree, &["commit", "-q", "--allow-empty", "-m", "b"]);
        assert!(!kept(), "HEAD moved");
        assert_eq!(walk(), refused);
        assert!(kept());
        fs::write(tree.join("new"), "x").unwrap();
        sh(tree, &["add", "new"]);
        assert!(!kept(), "the index changed");
        sh(tree, &["reset", "-q", "--hard"]);
        assert_eq!(walk(), refused);
        assert!(kept());

        fs::create_dir_all(tree.join("target/a/b")).unwrap();
        assert!(!kept(), "a folder on the way changed");
        fs::remove_dir_all(tree.join("target/a/n")).unwrap();
        removable(&home.0, &found).unwrap();
    }

    fn sealed(folder: PathBuf) -> PathBuf {
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("file"), "x").unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o000)).unwrap();
        folder
    }

    #[test]
    fn a_removal_reads_the_folders_the_daemon_owns_and_cannot_read() {
        let (home, found) = Home::new("unreadable");
        let hidden = sealed(found.path.join("hidden"));
        assert_eq!(
            removable(&home.0, &found),
            Err("untracked files".to_owned())
        );
        assert!(hidden.join("file").exists(), "it stays in place");
        fs::remove_dir_all(&hidden).unwrap();

        let target = found.path.join("target/sealed");
        sh(&found.path, &["init", "-q", "target/sealed/n"]);
        sealed(target.clone());
        let refused = format!("{} holds a git repository", target.join("n").display());
        assert_eq!(removable(&home.0, &found), Err(refused));
        fs::remove_dir_all(target.join("n")).unwrap();

        sealed(target);
        retire(&home.0, &found).unwrap();
        assert!(!found.path.exists());
    }

    #[test]
    fn a_folder_of_another_user_keeps_the_worktree_and_its_mode() {
        let (_home, found) = Home::new("not-owned");
        let target = sealed(found.path.join("target"));
        if fs::read_dir(&target).is_ok() {
            return; // Root reads it whatever its mode.
        }

        let refused = open_all(&found.path, || u32::MAX).unwrap_err();

        let unread = format!("cannot read {}: ", target.display());
        assert!(refused.starts_with(&unread), "{refused}");
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0);
    }

    #[test]
    fn a_mount_that_appears_during_an_orphans_walk_moves_nothing() {
        let (home, found) = Home::new("orphan-mount");
        let point = fs::canonicalize(&found.path).unwrap().join("target");
        fs::create_dir_all(&point).unwrap();
        let table = format!("23 22 0:5 / {} rw - tmpfs tmpfs rw\n", point.display());
        let read = || Ok(table.clone().into_bytes());

        let refused = clear_orphan(&home.0, &found.path, &read).unwrap_err();

        assert_eq!(refused, format!("{} is a mount", point.display()));
        assert!(point.exists());
        assert!(!home.0.join("trash").exists() && !home.0.join("reclaim").exists());
    }

    #[test]
    fn the_tmp_of_a_nested_cargo_target_does_not_count() {
        let (home, found) = Home::new("nested-target");
        let nested = found.path.join("target/inner/target");
        fs::create_dir_all(&nested).unwrap();
        let tag = "Signature: 8a477f597d28d172789f06886806bc55\n# created by cargo\n";
        fs::write(nested.join("CACHEDIR.TAG"), tag).unwrap();
        sh(&nested, &["init", "-q", "tmp/n"]);
        assert_eq!(scratches(&found.path), [nested.join("tmp")]);
        sh(&nested, &["init", "-q", "beside"]);
        let refused = removable(&home.0, &found).unwrap_err();
        assert!(
            refused.ends_with("beside holds a git repository"),
            "{refused}"
        );
        fs::remove_dir_all(nested.join("beside")).unwrap();

        retire(&home.0, &found).unwrap();

        assert!(!found.path.exists() && !home.0.join("trash").exists());
    }

    #[test]
    fn an_ignored_name_that_is_not_utf8_does_not_keep_its_worktree() {
        let (home, found) = Home::new("bytes");
        let exclude = home.0.join("repos/o/n/.git/info/exclude");
        fs::write(exclude, "*.ign\n").unwrap();
        let name = std::ffi::OsString::from_vec(b"a\xff.ign".to_vec());
        fs::write(found.path.join(name), "x").unwrap();

        retire(&home.0, &found).unwrap();

        assert!(!found.path.exists());
    }

    #[test]
    fn a_checkout_whose_folder_is_gone_loses_its_entry() {
        let (home, found) = Home::new("gone-checkout");
        let clone = home.0.join("repos/o/n");
        fs::remove_dir_all(&found.path).unwrap();

        remove_checkout(&clone, &found.path).unwrap();

        assert!(!clone.join(".git/worktrees/1").exists());
    }

    #[test]
    fn a_run_start_gives_up_on_a_walk_past_its_deadline() {
        let (_home, found) = Home::new("slow-walk");
        fs::create_dir_all(found.path.join("target/deep")).unwrap();
        let now = Some(Instant::now());

        let refused = no_repository(&found.path.join("target"), now).unwrap_err();

        assert!(
            refused.ends_with("its files take over 10s to check"),
            "{refused}"
        );
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
}
