use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::servers::Servers;

/// Priory's own directory: `PRIORY_HOME`, else `$XDG_STATE_HOME/priory-oss`, else `~/.local/state/priory-oss`.
pub fn home() -> PathBuf {
    if let Some(home) = std::env::var_os("PRIORY_HOME").filter(|home| !home.is_empty()) {
        return PathBuf::from(home);
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|state| !state.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    state.join("priory-oss")
}

/// The path hooks and MCP servers run Priory by: `PRIORY_EXE`, else the absolute path Priory was
/// invoked by, else its resolved binary. Codex keys hook trust on the command, so a path that
/// survives upgrades, such as a profile link rather than a Nix store path, keeps that trust.
pub fn exe() -> Result<PathBuf, String> {
    if let Some(exe) = std::env::var_os("PRIORY_EXE").filter(|exe| !exe.is_empty()) {
        return Ok(PathBuf::from(exe));
    }
    if let Some(invoked) = std::env::args_os().next().map(PathBuf::from)
        && invoked.is_absolute()
    {
        return Ok(invoked);
    }
    std::env::current_exe().map_err(|error| error.to_string())
}

static COMMAND_TIMEOUT: OnceLock<Duration> = OnceLock::new();

/// Sets how long one git, gh or tmux command may run, once per process; 5 minutes when unset.
pub fn bound_commands(timeout: Duration) {
    let _ = COMMAND_TIMEOUT.set(timeout);
}

pub fn command_timeout() -> Duration {
    COMMAND_TIMEOUT
        .get()
        .copied()
        .unwrap_or(Duration::from_secs(300))
}

/// How long the loop waits for a network call, or for git on a worktree it sweeps.
pub const LOOP_TIMEOUT: Duration = Duration::from_secs(30);

/// `LOOP_TIMEOUT`, or the command timeout when that is shorter.
pub fn loop_timeout() -> Duration {
    LOOP_TIMEOUT.min(command_timeout())
}

thread_local! {
    /// When the work in `as_one` must end, on the thread that runs it.
    static ENDS: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Runs `work`, whose `soon` commands share one `loop_timeout`.
pub(crate) fn as_one<T>(work: impl FnOnce() -> T) -> T {
    as_one_within(loop_timeout(), work)
}

/// As `as_one`, with `limit` to share.
pub(crate) fn as_one_within<T>(limit: Duration, work: impl FnOnce() -> T) -> T {
    let before = ENDS.replace(Some(Instant::now() + limit));
    let done = work();
    ENDS.set(before);
    done
}

/// What is left of `loop_timeout`, or of the `as_one` around.
pub(crate) fn loop_left() -> Duration {
    ENDS.get()
        .map_or_else(loop_timeout, left)
        .min(loop_timeout())
}

/// As `run`, giving up after `loop_timeout`, or at the end of the `as_one` around.
pub(crate) fn soon(command: &mut Command) -> Result<String, String> {
    within(command, loop_left())
}

/// As `soon`, with `input` on the command's stdin.
pub(crate) fn fed_soon(command: &mut Command, input: Vec<u8>) -> Result<String, String> {
    fed_raw_within(command, Some(input), loop_left())
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

pub(crate) fn within(command: &mut Command, timeout: Duration) -> Result<String, String> {
    fed_raw_within(command, None, timeout).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Whether `error` is of a command that ran past its deadline.
pub fn timed_out(error: &str) -> bool {
    error.contains("timed out after ")
}

/// Runs `command` in its own process group and returns its output, or `None` when it ran past the
/// command timeout. Then it kills the whole group, so a hung fetch cannot stop the daemon's one
/// thread, and no child that ignores TERM keeps the output pipe open.
pub fn output_within(command: &mut Command) -> std::io::Result<Option<Output>> {
    fed_within(command, None, command_timeout())
}

/// How long a killed command may take to end before its caller goes on.
const REAP: Duration = Duration::from_secs(1);

/// As `output_within`, with `input` written to the command's stdin, and `timeout` as its deadline.
pub fn fed_within(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    let stdin = if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    };
    let mut child = command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let _running = crate::ticks::running(command);
    let group = child.id();
    let stdin = child.stdin.take();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        // Its output is read while it takes its input: a command that fills the output pipe first
        // would wait for its reader, and the writer for it.
        let writer = std::thread::spawn(move || match (stdin, input) {
            (Some(mut stdin), Some(input)) => stdin.write_all(&input),
            _ => Ok(()),
        });
        let output = child.wait_with_output();
        let written = writer.join().expect("the writer does not panic");
        // A command that failed before reading its input explains itself better than the pipe.
        sender.send(output.and_then(|output| match written {
            Err(error)
                if error.kind() == std::io::ErrorKind::BrokenPipe && !output.status.success() =>
            {
                Ok(output)
            }
            written => written.map(|()| output),
        }))
    });
    if let Ok(output) = receiver.recv_timeout(timeout) {
        return output.map(Some);
    }
    crate::script::kill(group);
    // A process blocked on a hung mount outlives SIGKILL, so its end is not waited for without bound.
    let _ = receiver.recv_timeout(REAP);
    Ok(None)
}

/// Starts `curl` POSTing the JSON in `body` to `url`, without waiting for it. It prints the HTTP
/// status on stdout. `curl` gives up after the command timeout, and `--disable` skips a `.curlrc`.
/// Any header lines in the file `headers` go too, so no credential shows in the process list.
pub fn post_json(
    url: &str,
    headers: Option<&Path>,
    body: &Path,
) -> std::io::Result<std::process::Child> {
    let mut command = post(url, body);
    command.args(["--header", "Content-Type: application/json"]);
    if let Some(headers) = headers {
        command
            .arg("--header")
            .arg(format!("@{}", headers.display()));
    }
    command.spawn()
}

/// As `post_json`, for a Web Push request: HTTPS only, with the header lines in the file `headers`,
/// so no credential shows in the process list.
pub fn post_push(url: &str, headers: &Path, body: &Path) -> std::io::Result<std::process::Child> {
    post(url, body)
        .args(["--proto", "=https", "--header"])
        .arg(format!("@{}", headers.display()))
        .spawn()
}

fn post(url: &str, body: &Path) -> Command {
    let mut command = Command::new("curl");
    command
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--output",
            "/dev/null",
        ])
        .args([
            "--write-out",
            "%{http_code}",
            "--connect-timeout",
            "10",
            "--max-time",
        ])
        .arg(command_timeout().as_secs().max(1).to_string())
        .arg("--data-binary")
        .arg(format!("@{}", body.display()))
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Runs `command` and returns its stdout, or an error naming it with its stderr.
pub fn run(command: &mut Command) -> Result<String, String> {
    fed(command, None)
}

/// As `run`, with `input` on the command's stdin.
fn fed(command: &mut Command, input: Option<Vec<u8>>) -> Result<String, String> {
    fed_raw(command, input).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// As `fed`, returning the command's stdout as it wrote it.
pub(crate) fn fed_raw(command: &mut Command, input: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    fed_raw_within(command, input, command_timeout())
}

/// As `fed_raw`, giving up after `timeout`.
pub(crate) fn fed_raw_within(
    command: &mut Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let output = fed_within(command, input, timeout)
        .map_err(|error| format!("{command:?}: {error}"))?
        .ok_or_else(|| {
            // Rounded up, since what is left of a deadline is just under it.
            format!(
                "{command:?}: timed out after {}s",
                timeout.as_secs_f64().ceil()
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

/// The variable each git command Priory runs carries, set to the pid of the Priory process that
/// runs it, so the sweep tells Priory's own short checks in a worktree, such as the desk's, from a
/// process that keeps the worktree.
pub(crate) const OWN_GIT: &str = "PRIORY_GIT";

/// Git in `dir`, as Priory runs it on a clone or a worktree. It runs no hook and no file system
/// monitor, since a jailed agent can write both into the clone it shares with the host.
pub fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .env(OWN_GIT, std::process::id().to_string())
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ]);
    command
}

/// Refuses `repo` unless it is a GitHub `owner/name`: an owner of letters, digits and `-`, a name of
/// letters, digits, `-`, `_` and `.`, neither starting with `-`, and a name other than `.` or `..`.
/// Priory joins it onto folders under its home, so anything else could leave that home.
pub fn check_repo(repo: &str) -> Result<(), String> {
    let part = |part: &str, allowed: fn(char) -> bool| {
        !part.is_empty() && !part.starts_with('-') && part.chars().all(allowed)
    };
    let valid = repo.split_once('/').is_some_and(|(owner, name)| {
        part(owner, |c| c.is_ascii_alphanumeric() || c == '-')
            && part(name, |c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && name != "."
            && name != ".."
    });
    if !valid {
        return Err(format!("repo {repo:?} is not a GitHub owner/name"));
    }
    Ok(())
}

/// How `worktree` found or made a task's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// The folder was there, on its branch, or put back on it as the recovery says.
    Kept(Option<Recovery>),
    /// The branch was there and its folder was not, as after a retirement or a sweep.
    Restored,
    /// A new branch, from this commit.
    New(String),
}

impl Added {
    /// The commit a new branch starts from; `None` when the branch already existed.
    pub fn base(&self) -> Option<&str> {
        match self {
            Added::New(base) => Some(base),
            Added::Kept(_) | Added::Restored => None,
        }
    }
}

/// How `worktree` put a task's worktree, found off its branch, back on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    /// The commit the worktree had checked out.
    pub head: String,
    /// The branch it was on; `None` when its HEAD was detached.
    pub from_branch: Option<String>,
    /// The task's branch it went back to.
    pub branch: String,
    /// The branch made to keep the commits at risk of a detached HEAD.
    pub kept_branch: Option<String>,
    /// How many commits of the old HEAD no remote holds, as is or rebased. `kept_branch` keeps
    /// them, or `from_branch` when the HEAD was on one.
    pub kept_commits: usize,
}

/// The files whose presence in a worktree's git folder says an operation is in progress, with the
/// operation's name and the commands that finish it and drop it.
const OPERATIONS: [(&str, &str, &str, &str); 7] = [
    (
        "rebase-merge",
        "a rebase",
        "rebase --continue",
        "rebase --abort",
    ),
    (
        "rebase-apply",
        "a rebase",
        "rebase --continue",
        "rebase --abort",
    ),
    ("MERGE_HEAD", "a merge", "merge --continue", "merge --abort"),
    (
        "CHERRY_PICK_HEAD",
        "a cherry-pick",
        "cherry-pick --continue",
        "cherry-pick --abort",
    ),
    (
        "REVERT_HEAD",
        "a revert",
        "revert --continue",
        "revert --abort",
    ),
    ("BISECT_LOG", "a bisect", "bisect reset", "bisect reset"),
    // A sequence of picks or reverts leaves this folder between its commits, and after a crash.
    (
        "sequencer",
        "a cherry-pick or revert sequence",
        "cherry-pick --continue",
        "cherry-pick --quit",
    ),
];

/// Puts the worktree at `path`, on `from` or detached, back on `branch` when no work can be lost:
/// no uncommitted change and no operation in progress. Commits of a detached HEAD that no remote
/// holds, as is or rebased, stay on a new `<branch>-kept-<short sha>` first. Refuses otherwise,
/// with the commands a person runs.
fn recover(path: &Path, branch: &str, from: Option<String>) -> Result<Recovery, String> {
    let head = run(git(path).args(["rev-parse", "HEAD"]))?
        .trim()
        .to_owned();
    let short = run(git(path).args(["rev-parse", "--short", "HEAD"]))?
        .trim()
        .to_owned();
    let place = match &from {
        Some(from) => format!("is on branch {from}"),
        None => format!("is detached at {short}"),
    };
    let shown = path.display();
    let dir = quote(path);
    for (file, operation, finish, drop) in OPERATIONS {
        let marker = run(git(path).args(["rev-parse", "--git-path", file]))?;
        if path.join(marker.trim()).exists() {
            let how = if finish == drop {
                format!("End it with git -C {dir} {finish}")
            } else {
                format!("Finish it with git -C {dir} {finish}, or drop it with git -C {dir} {drop}")
            };
            return Err(format!(
                "{shown} has {operation} in progress at {short}, not on {branch}. {how}, then \
                 resume the task."
            ));
        }
    }
    let status = run(git(path).args(["status", "--porcelain=v1", "--untracked-files=all"]))?;
    let changed = changes(&status);
    if !changed.is_empty() {
        let kept = format!("{branch}-kept-{short}");
        return Err(format!(
            "{shown} {place}, not on {branch}, and has {changed}. Keep them on a branch: \
             git -C {dir} switch -c {kept} && git -C {dir} add -A && git -C {dir} commit -m \
             \"Kept by hand\". Or drop them: git -C {dir} reset --hard && git -C {dir} clean -fd. \
             Then resume the task, and Priory puts the worktree back on {branch}."
        ));
    }
    if self::head(path, branch).is_none() {
        return Err(format!(
            "{shown} {place}, not on {branch}, and {branch} is gone"
        ));
    }
    let at_risk = at_risk(path, Some(branch), SCAN, PATCHES)?;
    let kept_branch = match (&from, at_risk) {
        (None, 1..) => {
            let kept = format!("{branch}-kept-{short}");
            match self::head(path, &kept) {
                Some(commit) if commit == head => {}
                Some(_) => {
                    return Err(format!(
                        "{shown} {place}, not on {branch}, with commits no remote holds, and \
                         {kept} to keep them is on another commit"
                    ));
                }
                None => {
                    run(git(path).args(["branch", &kept, &head]))?;
                }
            }
            Some(kept)
        }
        _ => None,
    };
    run(git(path).args(["switch", "--quiet", branch]))?;
    Ok(Recovery {
        head,
        from_branch: from,
        branch: branch.to_owned(),
        kept_branch,
        kept_commits: at_risk,
    })
}

/// The staged, unstaged and untracked files of `git status --porcelain=v1`, in words, or an empty
/// string when there are none.
pub(crate) fn changes(status: &str) -> String {
    let (mut staged, mut unstaged, mut untracked) = (0, 0, 0);
    for line in status.lines() {
        let mut codes = line.chars();
        match (codes.next(), codes.next()) {
            (Some('?'), Some('?')) => untracked += 1,
            (Some(index), Some(tree)) => {
                staged += usize::from(index != ' ');
                unstaged += usize::from(tree != ' ');
            }
            _ => {}
        }
    }
    let parts: Vec<String> = [
        (staged, "staged"),
        (unstaged, "unstaged"),
        (untracked, "untracked"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, kind)| {
        let files = if count == 1 { "file" } else { "files" };
        format!("{count} {kind} {files}")
    })
    .collect();
    match parts.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// The most remote commits a recovery compares by patch; past it, a detached HEAD's commits are
/// kept on a branch unread.
pub(crate) const SCAN: usize = 2000;

/// How many commits on the HEAD of `path` are neither on `branch`, when given, nor on a
/// remote-tracking ref, nor patch-equivalent to a commit there. A merge commit has no patch id, so
/// it always counts. When more than `scan` remote commits could match, or
/// their patches take more than `cap` bytes, none is compared and every commit counts. The whole scan
/// gives up after `loop_timeout`.
pub(crate) fn at_risk(
    path: &Path,
    branch: Option<&str>,
    scan: usize,
    cap: usize,
) -> Result<usize, String> {
    let deadline = Instant::now() + loop_left();
    let local: Vec<String> = branch
        .map(|branch| format!("refs/heads/{branch}"))
        .into_iter()
        .collect();
    let listed = |merges: bool| -> Result<Vec<String>, String> {
        let mut command = git(path);
        command.arg("rev-list");
        if !merges {
            command.arg("--no-merges");
        }
        let out = by(
            command
                .args(["HEAD", "--not"])
                .args(&local)
                .arg("--remotes"),
            deadline,
        )?;
        Ok(out.lines().map(str::to_owned).collect())
    };
    let all = listed(true)?;
    if all.is_empty() {
        return Ok(0);
    }
    let plain = listed(false)?;
    let merges = all.len() - plain.len();
    if plain.is_empty() {
        return Ok(merges);
    }
    let dates = by(
        git(path)
            .args(["log", "--no-walk=unsorted", "--format=%at"])
            .args(&plain),
        deadline,
    )?;
    let oldest = dates
        .lines()
        .filter_map(|date| date.trim().parse::<i64>().ok())
        .min()
        .unwrap_or(0);
    // A rebase keeps the author date, so a rebased copy was committed after it.
    let since = format!("--max-age={oldest}");
    let mut window = vec!["--no-merges".to_owned(), since, "--remotes".to_owned()];
    window.extend(local);
    window.extend(["--not".to_owned(), "HEAD".to_owned()]);
    let count = by(
        git(path).args(["rev-list", "--count"]).args(&window),
        deadline,
    )?;
    if count
        .trim()
        .parse::<usize>()
        .map_or(true, |count| count > scan)
    {
        return Ok(all.len());
    }
    let Some(ours) = patch_ids(path, &[], &plain, deadline, cap)? else {
        return Ok(all.len());
    };
    let Some(theirs) = patch_ids(path, &window, &[], deadline, cap)? else {
        return Ok(all.len());
    };
    let held: std::collections::HashSet<_> = theirs.into_iter().map(|(id, _)| id).collect();
    let unmatched = plain
        .iter()
        .filter(|commit| {
            !ours
                .iter()
                .any(|(id, ours)| ours == *commit && held.contains(id))
        })
        .count();
    Ok(merges + unmatched)
}

/// The `(patch id, commit)` pairs of the commits that `walk` reaches, or of the `commits` alone,
/// as `git patch-id --verbatim` reads them from `git log -p`. Whitespace counts, so a commit amended
/// only in its indentation is a different patch. A commit with an empty diff has none.
/// `None` when the patches take more than `cap` bytes.
fn patch_ids(
    path: &Path,
    walk: &[String],
    commits: &[String],
    deadline: Instant,
    cap: usize,
) -> Result<Option<Vec<(String, String)>>, String> {
    let mut log = git(path);
    log.args([
        "log",
        "-p",
        "--no-color",
        "--no-ext-diff",
        "--format=commit %H",
    ]);
    if !commits.is_empty() {
        log.arg("--no-walk=unsorted").args(commits);
    }
    let Some(patches) = capped(log.args(walk), deadline, cap)? else {
        return Ok(None);
    };
    let ids = fed_raw_within(
        git(path).args(["patch-id", "--verbatim"]),
        Some(patches),
        left(deadline),
    )?;
    Ok(Some(
        String::from_utf8_lossy(&ids)
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(id, commit)| (id.to_owned(), commit.to_owned()))
            .collect(),
    ))
}

fn left(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// As `run`, giving up at `deadline`.
fn by(command: &mut Command, deadline: Instant) -> Result<String, String> {
    within(command, left(deadline))
}

/// The output of `command`, or `None` when it prints more than `cap` bytes. It ends at `deadline`.
fn capped(command: &mut Command, deadline: Instant, cap: usize) -> Result<Option<Vec<u8>>, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("{command:?}: {error}"))?;
    let _running = crate::ticks::running(command);
    let group = child.id();
    let stdout = child.stdout.take().expect("stdout is piped");
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut read = Vec::new();
        let done = stdout.take(cap as u64 + 1).read_to_end(&mut read);
        if read.len() > cap {
            crate::script::kill(group);
        }
        sender.send(done.and_then(|_| child.wait()).map(|status| (read, status)))
    });
    match receiver.recv_timeout(left(deadline)) {
        Ok(Ok((read, _))) if read.len() > cap => Ok(None),
        Ok(Ok((read, status))) if status.success() => Ok(Some(read)),
        Ok(Ok((_, status))) => Err(format!("{command:?}: {status}")),
        Ok(Err(error)) => Err(format!("{command:?}: {error}")),
        Err(_) => {
            crate::script::kill(group);
            let _ = receiver.recv_timeout(REAP);
            Err(format!(
                "{command:?}: timed out after {}s",
                loop_timeout().as_secs()
            ))
        }
    }
}

/// The most bytes of patches a comparison reads, 5 times the largest seen.
pub(crate) const PATCHES: usize = 64 << 20;

/// Whether `path` is a worktree of `clone` whose gitdir is gone, as a `git worktree remove` that
/// failed partway leaves it: git deleted the tracked files and the gitdir, not the rest.
fn orphaned(path: &Path, clone: &Path) -> bool {
    let Ok(link) = fs::read_to_string(path.join(".git")) else {
        return false;
    };
    let Some(gitdir) = link.trim().strip_prefix("gitdir: ").map(Path::new) else {
        return false;
    };
    let Ok(registry) = fs::canonicalize(clone.join(".git").join("worktrees")) else {
        return false;
    };
    gitdir.parent() == Some(registry.as_path()) && !gitdir.exists()
}

/// Why the folder `worktree` would replace must stay for now: it has no gitdir, and a mount is at or
/// under it.
pub fn mounted_orphan(home: &Path, repo: &str, name: &str) -> Option<String> {
    let path = home.join("worktrees").join(repo).join(name);
    if !orphaned(&path, &home.join("repos").join(repo)) {
        return None;
    }
    crate::worktrees::unmounted(&path, &crate::worktrees::mountinfo).err()
}

/// The task's worktree folder `name` on `priory/<name>`, from the local branch `from` of Priory's
/// own clone of `repo` (`owner/name`), else from its default branch. The clone has no checkout: a person trusts it and the worktrees
/// folder once in Claude, which covers its worktrees. A worktree of that clone left on that branch by an interrupted
/// start is reused. `vouch` reads the default branch's head from GitHub outside the clone, for a base
/// that no agent chose. `keep_branches` keeps each branch that `origin`
/// deletes in the clone. The third value says whether the folder was kept, added back, or new.
pub fn worktree(
    home: &Path,
    repo: &str,
    name: &str,
    from: Option<&str>,
    vouch: bool,
    keep_branches: bool,
) -> Result<(PathBuf, String, Added), String> {
    check_repo(repo)?;
    let clone = home.join("repos").join(repo);
    let branch = format!("priory/{name}");
    let path = home.join("worktrees").join(repo).join(name);
    if !clone.exists() {
        // A clone interrupted before the rename leaves only the partial directory, which is replaced.
        // It lives apart from `repos`, where `<name>.partial` can be another repository's clone.
        let partial = home.join("partial").join(repo);
        let _ = fs::remove_dir_all(&partial);
        for dir in [&partial, &clone] {
            fs::create_dir_all(dir.parent().expect("a clone has a parent"))
                .map_err(|error| error.to_string())?;
        }
        phased("clone", || {
            run(Command::new("git")
                .args(["clone", "--quiet", "--no-checkout"])
                .arg(format!("https://github.com/{repo}.git"))
                .arg(&partial))
        })?;
        fs::rename(&partial, &clone).map_err(|error| error.to_string())?;
    }
    if keep_branches {
        never_prune(&clone);
    }
    if let Some(reason) = mounted_orphan(home, repo, name) {
        return Err(reason);
    }
    // What is there now is not the worktree, which may come back.
    if crate::worktrees::removes_at(&path) {
        return Err(sweep_removes(repo));
    }
    if orphaned(&path, &clone) {
        if crate::worktrees::orphan_without_repository(&path).is_ok() {
            crate::worktrees::discard_aside(home, &path);
        }
        if path.exists() {
            move_aside(home, &path)?;
        }
    }
    if path.exists() {
        let common =
            run(git(&path).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
        let current = run(git(&path).args(["branch", "--show-current"]))?;
        let ours =
            fs::canonicalize(common.trim()).ok() == fs::canonicalize(clone.join(".git")).ok();
        if !ours {
            return Err(format!(
                "{} exists but is not {branch} of {}",
                path.display(),
                clone.display()
            ));
        }
        let current = current.trim();
        if current == branch {
            return Ok((path, branch, Added::Kept(None)));
        }
        let from = (!current.is_empty()).then(|| current.to_owned());
        let recovery = recover(&path, &branch, from)?;
        return Ok((path, branch, Added::Kept(Some(recovery))));
    }
    // A revived task whose worktree the sweep removed carries on from the branch it left.
    if head(&clone, &branch).is_some() {
        // The prune would drop the entry of a worktree the sweep moved aside.
        if crate::worktrees::removes_in(&clone) {
            return Err(sweep_removes(repo));
        }
        phased("checkout", || {
            // A worktree folder a person deleted stays registered, and git refuses to add it again.
            run(git(&clone).args(["worktree", "prune"]))?;
            run(git(&clone)
                .args(["worktree", "add", "--quiet"])
                .arg(&path)
                .arg(&branch))
        })?;
        return Ok((path, branch, Added::Restored));
    }
    let fetching = fetching(&clone);
    let _fetching = match fetching.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return Err(sweep_fetches(repo)),
    };
    let trunk = phased("fetch", || {
        // Before the fetch, which then brings the commit.
        let trunk = (vouch && from.is_none())
            .then(|| trunk(repo, loop_timeout()))
            .transpose()?;
        soon(git(&clone).args(["fetch", "--quiet", "origin"]))?;
        soon(git(&clone).args(["remote", "set-head", "origin", "--auto"]))?;
        if let Some(trunk) = &trunk
            && run(git(&clone).args(["cat-file", "-e", trunk])).is_err()
        {
            soon(git(&clone).args(["fetch", "--quiet", "origin", trunk]))?;
        }
        Ok(trunk)
    })
    .inspect(|_| fetched(repo))
    .map_err(|error| {
        if timed_out(&error) {
            put_off(repo)
        } else {
            error
        }
    })?;
    let start = match (from, trunk) {
        (Some(from), _) => {
            head(&clone, from).ok_or_else(|| format!("branch {from} to start from is gone"))?
        }
        (None, Some(trunk)) => trunk,
        (None, None) => run(git(&clone).args(["rev-parse", "--verify", "origin/HEAD"]))?
            .trim()
            .to_owned(),
    };
    phased("checkout", || {
        run(git(&clone)
            // An agent can write the clone's replace refs, which would swap the commit's content.
            .arg("--no-replace-objects")
            // Tracking the trunk would let a bare `git push` with `push.default=upstream` land on it.
            .args(["worktree", "add", "--quiet", "--no-track", "-b", &branch])
            .arg(&path)
            .arg(&start))
    })?;
    // Not the branch's head: every worktree shares the clone's refs, so an agent can have moved it.
    Ok((path, branch, Added::New(start)))
}

/// Turns off a person's `fetch.prune` in `clone`, which would drop a merged branch, whose commits
/// then look never pushed. Its git commands share one `loop_timeout`. Best effort.
fn never_prune(clone: &Path) {
    let config = || {
        let mut command = git(clone);
        command.args(["config", "--local"]);
        command
    };
    as_one(|| {
        match soon(config().args(["--get", "remote.origin.prune"])) {
            Ok(set) if set.trim() == "false" => {}
            // A write killed midway leaves a lock file, so a slow disk gets none.
            Err(error) if timed_out(&error) => {}
            _ => {
                let _ = soon(config().args(["--replace-all", "remote.origin.prune", "false"]));
            }
        }
    });
}

/// How a task's reason starts while its worktree waits for a fetch that timed out.
pub const FETCH_WAITS: &str = "waits to fetch ";

/// Each repository whose fetch timed out: when it may be tried again, and how long that wait is.
static SLOW: std::sync::Mutex<std::collections::BTreeMap<String, (Instant, Duration)>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

fn slow() -> std::sync::MutexGuard<'static, std::collections::BTreeMap<String, (Instant, Duration)>>
{
    SLOW.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

static FETCHING: Mutex<std::collections::BTreeMap<PathBuf, Arc<Mutex<()>>>> =
    Mutex::new(std::collections::BTreeMap::new());

/// What a fetch of `clone` holds, since the sweep fetches on a thread.
pub(crate) fn fetching(clone: &Path) -> Arc<Mutex<()>> {
    FETCHING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(clone.to_owned())
        .or_default()
        .clone()
}

fn fetched(repo: &str) {
    slow().remove(repo);
}

/// Waits 1 minute before `repo` is fetched again, twice as long after each next timeout, 10 minutes
/// at most. Returns why its tasks wait.
fn put_off(repo: &str) -> String {
    let mut slow = slow();
    let wait = slow.get(repo).map_or(Duration::from_secs(60), |(_, wait)| {
        (*wait * 2).min(Duration::from_secs(600))
    });
    slow.insert(repo.to_owned(), (Instant::now() + wait, wait));
    fetch_reason(repo)
}

fn fetch_reason(repo: &str) -> String {
    format!(
        "{FETCH_WAITS}{repo}: timed out after {}s",
        loop_timeout().as_secs()
    )
}

fn sweep_fetches(repo: &str) -> String {
    format!("{FETCH_WAITS}{repo}: the sweep fetches it")
}

fn sweep_removes(repo: &str) -> String {
    format!("{FETCH_WAITS}{repo}: the sweep removes a worktree of it")
}

/// Why worktree `name` of `repo` waits. The last fetch timed out a short while ago or the sweep runs
/// one, when a fetch would make it. Else the sweep removes a worktree, when it would be added back.
pub fn fetch_waits(home: &Path, repo: &str, name: &str) -> Option<String> {
    let waits = slow()
        .get(repo)
        .is_some_and(|(again, _)| *again > Instant::now());
    let clone = home.join("repos").join(repo);
    let path = home.join("worktrees").join(repo).join(name);
    if crate::worktrees::removes_at(&path) {
        return Some(sweep_removes(repo));
    }
    let exists = path.exists();
    let branch = || {
        soon(git(&clone).args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/priory/{name}^{{commit}}"),
        ]))
        .is_ok()
    };
    if !exists && crate::worktrees::removes_in(&clone) && branch() {
        return Some(sweep_removes(repo));
    }
    let fetching = fetching(&clone);
    let swept = matches!(fetching.try_lock(), Err(TryLockError::WouldBlock));
    let reason = if waits {
        fetch_reason(repo)
    } else if swept {
        sweep_fetches(repo)
    } else {
        return None;
    };
    (!exists && !branch()).then_some(reason)
}

/// The head of `repo`'s default branch as GitHub has it. It runs outside the clone, whose refs and
/// config an agent can write.
pub(crate) fn trunk(repo: &str, within: Duration) -> Result<String, String> {
    let url = format!("https://github.com/{repo}.git");
    let refs = fed_raw_within(
        git(Path::new("/")).args(["ls-remote", &url, "HEAD"]),
        None,
        within,
    )?;
    head_of(&String::from_utf8_lossy(&refs))
        .map(str::to_owned)
        .ok_or_else(|| format!("{url} names no HEAD commit"))
}

/// The commit of HEAD in `refs`, the output of `git ls-remote`.
fn head_of(refs: &str) -> Option<&str> {
    refs.lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(_, name)| *name == "HEAD")
        .map(|(commit, _)| commit)
        .filter(|commit| {
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

/// Runs `phase` of a worktree's start, and times it for the metrics by how it ended.
fn phased<T>(phase: &'static str, step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let started = std::time::Instant::now();
    let done = step();
    let outcome = match &done {
        Ok(_) => "ok",
        Err(error) if timed_out(error) => "timeout",
        Err(_) => "error",
    };
    crate::ticks::timed_as(
        PHASE,
        &[("priory.phase", phase), ("outcome", outcome)],
        started.elapsed(),
    );
    done
}

/// The metric that times each phase of a worktree's start.
pub const PHASE: &str = "priory.worktree.phase.duration";

/// The commit `branch` points to, read from Priory's clone under `home` when `worktree` is gone, as
/// after a retirement or a sweep.
pub fn kept_head(home: &Path, worktree: &Path, branch: &str) -> Option<String> {
    if worktree.exists() {
        return head(worktree, branch);
    }
    let repo = worktree
        .parent()?
        .strip_prefix(home.join("worktrees"))
        .ok()?;
    head(&home.join("repos").join(repo), branch)
}

/// The commit Priory's clone under `home` last pushed `branch` to, or fetched it at, on origin:
/// its remote-tracking branch, as the worktree `worktree` of that clone shares it. A git that runs
/// past `loop_timeout` is an error.
pub fn pushed_head(home: &Path, worktree: &Path, branch: &str) -> Result<Option<String>, String> {
    let Some(repo) = worktree
        .parent()
        .and_then(|parent| parent.strip_prefix(home.join("worktrees")).ok())
    else {
        return Ok(None);
    };
    let commit = format!("refs/remotes/origin/{branch}^{{commit}}");
    match soon(git(&home.join("repos").join(repo)).args([
        "rev-parse",
        "--verify",
        "--quiet",
        &commit,
    ])) {
        Ok(head) => Ok(Some(head.trim().to_owned())),
        Err(error) if timed_out(&error) => Err(error),
        Err(_) => Ok(None),
    }
}

/// `worktree`, or Priory's clone of its repository under `home` when it is gone.
fn repository(home: &Path, worktree: &Path) -> Result<PathBuf, String> {
    if worktree.exists() {
        return Ok(worktree.to_path_buf());
    }
    worktree
        .parent()
        .and_then(|parent| parent.strip_prefix(home.join("worktrees")).ok())
        .map(|repo| home.join("repos").join(repo))
        .ok_or_else(|| format!("{} is gone", worktree.display()))
}

/// The branches a task stacks on its own, `own`, named after it: `priory/7-api` for `priory/7`.
pub fn stacked(home: &Path, worktree: &Path, own: &str) -> Result<Vec<String>, String> {
    let dir = repository(home, worktree)?;
    let named = format!("refs/heads/{own}-*");
    let found = run(git(&dir).args(["for-each-ref", "--format=%(refname:strip=2)", &named]))?;
    Ok(found.lines().map(str::to_owned).collect())
}

/// The ones of `branches` that origin has, or why another task could not take over all the work of
/// `worktree` from origin: a changed or untracked file, a commit of the checkout or of a branch that
/// no branch of origin reaches, or a branch that is gone. Read from Priory's clone under `home` when
/// the worktree is gone.
pub fn unpushed(home: &Path, worktree: &Path, branches: &[String]) -> Result<Vec<String>, String> {
    let live = worktree.exists();
    let dir = repository(home, worktree)?;
    let mut heads = Vec::new();
    if live {
        let status = run(git(&dir).args(["status", "--porcelain=v1", "--untracked-files=all"]))?;
        let changed = changes(&status);
        if !changed.is_empty() {
            return Err(format!(
                "the worktree has {changed}: commit and push them first, since the other task takes over only what is pushed"
            ));
        }
    }
    let mut pushed = Vec::new();
    for branch in branches {
        let reference = format!("refs/remotes/origin/{branch}^{{commit}}");
        let on_origin =
            run(git(&dir).args(["rev-parse", "--verify", "--quiet", &reference])).is_ok();
        if on_origin {
            pushed.push(branch.clone());
        }
        match head(&dir, branch) {
            Some(head) => heads.push((format!("branch {branch}"), head)),
            None if !on_origin => {
                return Err(format!(
                    "branch {branch} is gone, here and on origin: push its work first, since the other task takes over only what is pushed"
                ));
            }
            None => {}
        }
    }
    if live {
        heads.extend(checked_out(&dir).map(|head| ("the checkout".to_owned(), head)));
    }
    for (name, head) in heads {
        let missing = run(git(&dir).args([
            "rev-list",
            "--max-count=1",
            &head,
            "--not",
            "--remotes=origin",
        ]))?;
        if !missing.trim().is_empty() {
            return Err(format!(
                "{name} has commits origin lacks: push it first, or delete what you abandoned, since the other task takes over only what is pushed"
            ));
        }
    }
    Ok(pushed)
}

/// The files `branch` changes against the default branch, read as `kept_head` reads, or `None`
/// when git cannot say.
pub fn changed_files(home: &Path, worktree: &Path, branch: &str) -> Option<Vec<String>> {
    let dir = if worktree.exists() {
        worktree.to_path_buf()
    } else {
        let repo = worktree
            .parent()?
            .strip_prefix(home.join("worktrees"))
            .ok()?;
        home.join("repos").join(repo)
    };
    let range = format!("origin/HEAD...refs/heads/{branch}");
    run(git(&dir).args(["diff", "--name-only", "-z", &range]))
        .ok()
        .map(|names| {
            names
                .split('\0')
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()
        })
}

/// The commit `branch` points to in the repository of `worktree`.
pub fn head(worktree: &Path, branch: &str) -> Option<String> {
    let commit = format!("refs/heads/{branch}^{{commit}}");
    run(git(worktree).args(["rev-parse", "--verify", "--quiet", &commit]))
        .ok()
        .map(|head| head.trim().to_owned())
}

/// The commit checked out in `dir`, or `None` outside git or before any commit.
pub fn checked_out(dir: &Path) -> Option<String> {
    run(git(dir).args(["rev-parse", "--verify", "--quiet", "HEAD"]))
        .ok()
        .map(|head| head.trim().to_owned())
}

/// The last commit that changed the files under `dir`, or `None` outside git or before any commit.
pub fn revision(dir: &Path) -> Option<String> {
    run(git(dir).args(["log", "-1", "--format=%H", "--", "."]))
        .ok()
        .map(|commit| commit.trim().to_owned())
        .filter(|commit| !commit.is_empty())
}

/// Pushes `commit` to `branch` on the origin of `worktree`, then returns the open pull request from
/// `branch` and its base: the one `gh pr list` finds, which must be on `base` when given, else one
/// it creates on `base`, or on the default branch with no `base`.
pub fn open_pull_request(
    worktree: &Path,
    repo: &str,
    branch: &str,
    commit: &str,
    base: Option<&str>,
    title: &str,
    body: &str,
) -> Result<(u64, String), String> {
    let target = format!("{commit}:refs/heads/{branch}");
    run(git(worktree).args(["push", "--quiet", "origin", &target]))?;
    let open = run(Command::new("gh").args([
        "pr",
        "list",
        "-R",
        repo,
        "--head",
        branch,
        "--state",
        "open",
        "--json",
        "number,baseRefName",
    ]))?;
    let open: Vec<serde_json::Value> = serde_json::from_str(&open)
        .map_err(|error| format!("gh pr list printed {open:?}: {error}"))?;
    if let Some(pull) = open.first()
        && let (Some(number), Some(found)) = (pull["number"].as_u64(), pull["baseRefName"].as_str())
    {
        if let Some(base) = base.filter(|base| *base != found) {
            return Err(format!(
                "pull request #{number} from {branch} is based on {found}, not {base}"
            ));
        }
        return Ok((number, found.to_owned()));
    }
    let base = match base {
        Some(base) => base.to_owned(),
        None => default_branch(worktree)?,
    };
    let url = run(Command::new("gh").args([
        "pr", "create", "-R", repo, "--base", &base, "--head", branch, "--title", title, "--body",
        body,
    ]))?;
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|number| number.parse().ok())
        .map(|number| (number, base))
        .ok_or_else(|| format!("gh pr create printed {url:?}, not a pull request URL"))
}

/// The default branch of `origin`, in the repository of `worktree`.
pub fn default_branch(worktree: &Path) -> Result<String, String> {
    Ok(
        run(git(worktree).args(["rev-parse", "--abbrev-ref", "origin/HEAD"]))?
            .trim()
            .trim_start_matches("origin/")
            .to_owned(),
    )
}

/// Whether commit `below` is `commit` or one of its ancestors, in the repository of `worktree`. A
/// git that runs past `loop_timeout` is an error, which is not a no.
pub fn is_ancestor(worktree: &Path, below: &str, commit: &str) -> Result<bool, String> {
    match soon(git(worktree).args(["merge-base", "--is-ancestor", below, commit])) {
        Ok(_) => Ok(true),
        Err(error) if timed_out(&error) => Err(error),
        Err(_) => Ok(false),
    }
}

/// As `is_ancestor`, with an error when git could not tell.
pub(crate) fn descends(worktree: &Path, below: &str, commit: &str) -> Result<bool, String> {
    let mut command = git(worktree);
    command.args(["merge-base", "--is-ancestor", below, commit]);
    let output = fed_within(&mut command, None, command_timeout())
        .map_err(|error| format!("{command:?}: {error}"))?
        .ok_or_else(|| format!("{command:?}: timed out"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// The origin branches, as last pushed or fetched, that hold every commit of the local branch
/// `branch` of `worktree`'s clone, or the other way round. A branch already in the trunk has none.
/// `None` when git cannot say.
pub fn in_line(home: &Path, worktree: &Path, branch: &str) -> Option<Vec<String>> {
    let repo = worktree
        .parent()?
        .strip_prefix(home.join("worktrees"))
        .ok()?;
    let clone = home.join("repos").join(repo);
    let head = kept_head(home, worktree, branch)?;
    if is_ancestor(&clone, &head, "origin/HEAD").ok()? {
        return Some(Vec::new());
    }
    let refs = |filter: &[&str]| {
        run(git(&clone)
            .args(["for-each-ref", "--format=%(refname:lstrip=3)"])
            .args(filter)
            .arg("refs/remotes/origin"))
        .ok()
    };
    let ahead = refs(&["--contains", &head])?;
    let behind = refs(&["--merged", &head, "--no-merged", "origin/HEAD"])?;
    Some(
        ahead
            .lines()
            .chain(behind.lines())
            .map(str::to_owned)
            .collect(),
    )
}

/// The open pull requests of `repo` that `author` opened, or every open one without an author,
/// each with its `number`, `title`, `url`, `headRefName` and `isCrossRepository`.
pub fn open_pull_requests(
    repo: &str,
    author: Option<&str>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut command = Command::new("gh");
    command.args([
        "pr",
        "list",
        "-R",
        repo,
        "--state",
        "open",
        "--limit",
        "1000",
        "--json",
        "number,title,url,headRefName,isCrossRepository",
    ]);
    if let Some(author) = author {
        command.args(["--author", author]);
    }
    let open = run(&mut command)?;
    serde_json::from_str(&open).map_err(|error| format!("gh pr list printed {open:?}: {error}"))
}

/// The state of pull request `number` on `repo`: OPEN, CLOSED or MERGED.
pub fn pull_request_state(repo: &str, number: u64) -> Result<String, String> {
    let number = number.to_string();
    run(Command::new("gh").args([
        "pr", "view", &number, "-R", repo, "--json", "state", "--jq", ".state",
    ]))
    .map(|state| state.trim().to_owned())
}

/// The line that marks a comment an agent wrote, since agents post as the person's `gh` login.
pub const AGENT_COMMENT: &str = "<!-- agent-comment -->";

/// Writes `body` as a new comment on pull request `number` of `repo`, marked as an agent's, and
/// returns its URL.
pub fn comment_on_pull_request(repo: &str, number: u64, body: &str) -> Result<String, String> {
    let number = number.to_string();
    let body = format!("{body}\n\n{AGENT_COMMENT}\n");
    fed(
        Command::new("gh").args(["pr", "comment", &number, "-R", repo, "--body-file", "-"]),
        Some(body.into_bytes()),
    )
    .map(|url| url.trim().to_owned())
}

/// Sets the commit status `context` of commit `sha` of `repo` to `state`, with `description`.
pub fn set_commit_status(
    repo: &str,
    sha: &str,
    state: &str,
    context: &str,
    description: &str,
) -> Result<(), String> {
    run(Command::new("gh").args([
        "api",
        &format!("repos/{repo}/statuses/{sha}"),
        "--method",
        "POST",
        "-f",
        &format!("state={state}"),
        "-f",
        &format!("context={context}"),
        "-f",
        &format!("description={description}"),
    ]))
    .map(drop)
}

/// Opens an issue on `repo` and returns its number. `mark`, a unique line the body ends with, finds
/// the issue that an earlier attempt opened before a crash, in any state, so none opens twice.
pub fn open_issue(repo: &str, title: &str, body: &str, mark: &str) -> Result<u64, String> {
    let found = run(Command::new("gh").args([
        "issue",
        "list",
        "-R",
        repo,
        "--state",
        "all",
        "--search",
        &format!("\"{mark}\" in:body"),
        "--json",
        "number,body",
    ]))?;
    let found: Vec<serde_json::Value> = serde_json::from_str(&found)
        .map_err(|error| format!("gh issue list printed {found:?}: {error}"))?;
    if let Some(number) = found
        .iter()
        .find(|issue| {
            issue["body"]
                .as_str()
                .is_some_and(|text| text.contains(mark))
        })
        .and_then(|issue| issue["number"].as_u64())
    {
        return Ok(number);
    }
    let body = format!("{body}\n\n{mark}");
    let url = run(Command::new("gh").args([
        "issue", "create", "-R", repo, "--title", title, "--body", &body,
    ]))?;
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| format!("gh issue create printed {url:?}, not an issue URL"))
}

pub(crate) fn tmux() -> Command {
    Command::new("tmux")
}

/// What a Claude Code session sets for its children. User settings such as `CLAUDE_CODE_USE_BEDROCK` stay.
pub(crate) const NESTED: [&str; 12] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_CHILD_NOTES_DIR",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// What a harness needs to start a run.
pub struct Launch<'a> {
    pub tmux: &'a str,
    pub dir: &'a Path,
    pub home: &'a Path,
    /// Priory's id for the run's conversation, which its pane, hooks and tools carry.
    pub session: &'a str,
    /// The harness's own id of the conversation to continue, which an earlier run of the task started.
    pub resume: Option<&'a str>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub body: &'a str,
    /// The factory's instructions, which go before `body`.
    pub instructions: &'a str,
    /// The plugin folder that holds the factory's skills, for a harness that loads plugins.
    pub plugin: Option<&'a Path>,
    /// The folder that holds the factory's memories, which the run reads outside its worktree.
    pub memories: Option<&'a Path>,
    /// The run's first prompt, passed as the harness's last argument after `--` rather than pasted.
    pub prompt: Option<&'a str>,
    /// The agent's Claude Code `permissions` settings, merged into Priory's own.
    pub permissions: serde_json::Value,
    /// The agent's `defaultMode`, or `manual`.
    pub permission_mode: &'a str,
    /// The command that starts the run in its own systemd scope, or none on a host without systemd.
    pub scope: &'a [String],
    /// Variables the run gets on top of the daemon's own, each `NAME=value`.
    pub env: &'a [String],
    /// The run takes a turn of a debate, so its MCP server serves `position`.
    pub debate: bool,
    /// A command that waits on the memory gate gives up at a limit, so the run's agent learns what
    /// to do then.
    pub gives_up: bool,
    /// The MCP servers of the person's config the run gets, or turns off, beside Priory's own.
    pub servers: &'a Servers,
    /// Which of Priory's own tools its MCP server serves the run.
    pub tools: crate::mcp::Tools,
    /// The sandbox the run's harness starts in, or none for the person's whole user.
    pub jail: Option<&'a crate::jail::Jail>,
    /// The factory turns on `trust_worktrees`, so the harness reads the trust Priory records.
    pub trust: bool,
    /// The direnv config folder that trusts the worktree, so the harness starts in its `.envrc`.
    pub direnv: Option<&'a Path>,
}

/// A coding agent CLI that Priory runs in tmux, with hooks and tools that report to the daemon.
pub trait Harness {
    /// The program's name, as a person types it.
    fn program(&self) -> &'static str;
    /// The program's arguments for `launch`, whose hooks and MCP server run `exe`.
    fn args(&self, exe: &Path, launch: &Launch) -> Result<Vec<String>, String>;
    /// What a person does once, in `clone` and the folder `worktrees` that holds Priory's
    /// worktrees, before the harness runs Priory's hooks there.
    fn trust(&self, exe: &Path, clone: &str, worktrees: &str) -> String;
    /// Whether `screen` shows the harness's dialog asking to trust its folder.
    fn asks_trust(&self, _screen: &str) -> bool {
        false
    }
    /// Whether a person must trust Priory's hooks too, which a folder grant does not cover.
    fn reviews_hooks(&self) -> bool {
        false
    }
    /// Whether `screen` shows the harness at its own prompt, which it reaches only past its dialogs.
    fn at_prompt(&self, _screen: &str) -> bool {
        false
    }
    /// Records the harness's trust in `worktree` and its `clone`, each a real path Priory owns.
    fn grant_trust(&self, worktree: &Path, clone: &Path) -> Vec<crate::trust::Grant>;
    /// Whether the harness's config still holds the trust `grant_trust` recorded.
    fn holds_trust(&self, worktree: &Path, clone: &Path) -> Result<bool, String>;
    /// A file the harness reads at start, and its text, which `launch` writes first.
    fn file(&self, _launch: &Launch) -> Option<(PathBuf, String)> {
        None
    }
    /// A file only the person may read, which `launch` writes first, since its text can hold tokens.
    fn private_file(&self, _exe: &Path, _launch: &Launch) -> Option<(PathBuf, String)> {
        None
    }
    /// The harness's own home under the person's `home`: its login, sessions, transcripts and
    /// trusted folders, which a jailed run keeps, each with how the jail binds it.
    fn own_home(&self, home: Option<&Path>) -> Vec<(PathBuf, Own)>;
    /// The variables that move the harness's own home, which a jailed run keeps.
    fn home_env(&self) -> &'static [&'static str];
}

/// How a jailed run gets a path of its harness's own home. Each folder is made when missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Own {
    /// A folder the run reads only.
    Read,
    /// A folder the run writes.
    Write,
    /// A folder the run writes in an empty copy of its own, gone when the run ends.
    Scratch,
    /// A file the run reads only, when it exists.
    File,
}

/// The folder a variable names, else `fallback` under `home`.
fn named_or(variable: &str, home: Option<&Path>, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(fallback)))
}

/// The factory's instructions, then the agent's own prompt.
fn system_prompt(launch: &Launch) -> String {
    [launch.instructions, launch.body]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The Claude Code tools an investigator's run never gets: each writes, or starts a subagent,
/// which can carry its own permission mode.
pub const INVESTIGATOR_DENIED: [&str; 7] = [
    "Edit",
    "MultiEdit",
    "Write",
    "NotebookEdit",
    "Bash",
    "Agent",
    "Task",
];

pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn program(&self) -> &'static str {
        "claude"
    }

    /// Claude Code runs hooks, plugins, MCP servers and scripts that its settings, plugins and
    /// `.claude.json` name, so those stay read-only. It writes its transcripts in `projects`, and
    /// the scripts its shell sources in `session-env` and `shell-snapshots`, which stay in the jail.
    fn own_home(&self, home: Option<&Path>) -> Vec<(PathBuf, Own)> {
        let named = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty());
        let Some(dir) = named
            .clone()
            .map(PathBuf::from)
            .or_else(|| Some(home?.join(".claude")))
        else {
            return Vec::new();
        };
        let mut own = vec![
            (dir.join("projects"), Own::Write),
            (dir.join("session-env"), Own::Scratch),
            (dir.join("shell-snapshots"), Own::Scratch),
            (dir.clone(), Own::Read),
        ];
        if named.is_none() {
            own.extend(home.map(|home| (home.join(".claude.json"), Own::File)));
        }
        own
    }

    fn home_env(&self) -> &'static [&'static str] {
        &["CLAUDE_CONFIG_DIR"]
    }

    fn args(&self, exe: &Path, launch: &Launch) -> Result<Vec<String>, String> {
        let mut args = match launch.resume {
            Some(id) => vec!["--resume".to_owned(), id.to_owned()],
            None => vec!["--session-id".to_owned(), launch.session.to_owned()],
        };
        if let Some(model) = launch.model {
            args.extend(["--model".to_owned(), model.to_owned()]);
        }
        if let Some(effort) = launch.effort {
            args.extend(["--effort".to_owned(), effort.to_owned()]);
        }
        if let Some(plugin) = launch.plugin {
            args.extend(["--plugin-dir".to_owned(), plugin.display().to_string()]);
        }
        if let Some(memories) = launch.memories {
            args.extend(["--add-dir".to_owned(), memories.display().to_string()]);
        }
        if let Some((file, _)) = self.file(launch) {
            args.extend([
                "--append-system-prompt-file".to_owned(),
                file.display().to_string(),
            ]);
        } else if !launch.body.is_empty() {
            args.extend(["--append-system-prompt".to_owned(), launch.body.to_owned()]);
        }
        // Without the flag, the user's own mode, such as auto, would decide the run's prompts (R7.2).
        args.extend([
            "--permission-mode".to_owned(),
            launch.permission_mode.to_owned(),
        ]);
        if launch.tools == crate::mcp::Tools::Investigator {
            // Claude merges `allow` across the person's, the repository's and the flag's settings,
            // so a person's `Bash(git:*)` would reach an investigator. An empty list loads none
            // but `--settings`, and the denied tools beat any `allow` (docs/design/consult.md).
            args.extend(["--setting-sources".to_owned(), String::new()]);
            args.extend([
                "--disallowedTools".to_owned(),
                INVESTIGATOR_DENIED.join(","),
            ]);
        }
        let mut permissions = launch.permissions.clone();
        if !launch.servers.denied.is_empty() {
            let deny = permissions["deny"]
                .as_array_mut()
                .expect("deny is an array");
            deny.extend(
                launch
                    .servers
                    .denied
                    .iter()
                    .map(|rule| rule.as_str().into()),
            );
        }
        args.extend(["--settings".to_owned(), settings(exe, &permissions)]);
        // Without the flag, every server of the person's own Claude config would reach the run too.
        // Under it Claude Code loads no claude.ai connector, so a run with connectors goes without
        // it and denies the other servers by rule and in the tool guard instead.
        if launch.servers.connectors.is_empty() {
            args.push("--strict-mcp-config".to_owned());
        }
        let (file, _) = self
            .private_file(exe, launch)
            .expect("Claude Code reads its MCP servers from a file");
        args.extend(["--mcp-config".to_owned(), file.display().to_string()]);
        Ok(args)
    }

    /// Claude 2.1.42 trusts a worktree by a trusted folder above it, and 2.1.310 by its trusted
    /// clone, so only both folders cover every version.
    fn trust(&self, _exe: &Path, clone: &str, worktrees: &str) -> String {
        format!(
            "trust the worktrees folder and the clone once: cd {worktrees} && claude, accept, exit, then cd {clone} && claude, accept, exit"
        )
    }

    /// Claude 2.1.330 reads the trust of the exact folder before it walks up, and keys a
    /// worktree's project by its clone, so the two keys cover the versions in the table.
    fn grant_trust(&self, worktree: &Path, clone: &Path) -> Vec<crate::trust::Grant> {
        crate::trust::claude(&crate::servers::claude_config(), &[worktree, clone])
    }

    fn holds_trust(&self, worktree: &Path, clone: &Path) -> Result<bool, String> {
        let config = crate::servers::claude_config();
        let config = fs::canonicalize(&config).unwrap_or(config);
        crate::trust::claude_holds(&config, &[worktree, clone])
    }

    fn asks_trust(&self, screen: &str) -> bool {
        screen.contains("Accessing workspace:")
            && (screen.contains("Yes, I trust this folder")
                || screen.contains("Quick safety check"))
    }

    /// Claude draws its input box between two rules; a dialog's `❯` has no rule right above it.
    fn at_prompt(&self, screen: &str) -> bool {
        let lines: Vec<_> = screen.lines().collect();
        lines.windows(2).enumerate().any(|(at, pair)| {
            pair[0].starts_with("───")
                && pair[1].starts_with('❯')
                && lines[at + 2..].iter().any(|line| line.starts_with("───"))
        })
    }

    /// The servers' definitions can hold tokens, which a command line would show to every process.
    fn private_file(&self, exe: &Path, launch: &Launch) -> Option<(PathBuf, String)> {
        let name = format!("{}.mcp.json", launch.session);
        Some((
            launch.home.join("prompts").join(name),
            mcp_config(exe, launch),
        ))
    }

    /// tmux refuses a command over about 16 KiB, so instructions reach Claude through a file.
    fn file(&self, launch: &Launch) -> Option<(PathBuf, String)> {
        (!launch.instructions.is_empty()).then(|| {
            let name = format!("{}.system.md", launch.session);
            (
                launch.home.join("prompts").join(name),
                system_prompt(launch),
            )
        })
    }
}

pub struct Codex;

impl Harness for Codex {
    fn own_home(&self, home: Option<&Path>) -> Vec<(PathBuf, Own)> {
        // Priory's trust profile stays read-only, so a run cannot turn it into a link the daemon
        // would write through.
        named_or("CODEX_HOME", home, ".codex")
            .map(|dir| {
                let profile = dir.join(format!("{}.config.toml", crate::trust::CODEX_PROFILE));
                vec![(dir, Own::Write), (profile, Own::File)]
            })
            .unwrap_or_default()
    }

    fn home_env(&self) -> &'static [&'static str] {
        &["CODEX_HOME"]
    }

    fn program(&self) -> &'static str {
        "codex"
    }

    fn args(&self, exe: &Path, launch: &Launch) -> Result<Vec<String>, String> {
        // Without these, the user's own config, such as an auto reviewer, would decide the prompts (R7.2).
        let (sandbox, approval, reviewer) = match launch.permission_mode {
            "acceptEdits" => ("workspace-write", "on-request", "user"),
            "auto" => ("workspace-write", "on-request", "auto_review"),
            "dontAsk" => ("read-only", "never", "user"),
            _ => ("read-only", "on-request", "user"),
        };
        let mut config = vec![
            ("sandbox_mode", json!(sandbox)),
            ("approval_policy", json!(approval)),
            ("approvals_reviewer", json!(reviewer)),
            (
                "mcp_servers.priory",
                json!({
                    "command": exe,
                    "args": mcp_args(launch),
                    "env": mcp_env(launch.home),
                    "default_tools_approval_mode": "approve",
                }),
            ),
        ];
        config.extend(codex_hooks(exe));
        if let Some(model) = launch.model {
            config.push(("model", json!(model)));
        }
        if let Some(effort) = launch.effort {
            config.push(("model_reasoning_effort", json!(effort)));
        }
        let instructions = system_prompt(launch);
        if !instructions.is_empty() {
            config.push(("developer_instructions", json!(instructions)));
        }
        let mut args = match launch.resume {
            Some(thread) => vec!["resume".to_owned(), thread.to_owned()],
            None => Vec::new(),
        };
        for (key, value) in config {
            args.extend(["-c".to_owned(), override_of(key, value)?]);
        }
        // Codex has no strict flag, so each server of the person's config the agent leaves out is turned off.
        for name in &launch.servers.off {
            args.extend(["-c".to_owned(), format!("mcp_servers.{name}.enabled=false")]);
        }
        // A shared server would lend the run another session's environment and hook trust.
        args.push("--no-daemon".to_owned());
        if launch.trust {
            // Codex 0.170 ignores a `-c projects.<path>.trust_level` override, but reads a profile.
            args.splice(
                0..0,
                ["-p".to_owned(), crate::trust::CODEX_PROFILE.to_owned()],
            );
        }
        Ok(args)
    }

    /// Codex keys trust on the repository root, which is the clone for every worktree.
    fn grant_trust(&self, _worktree: &Path, clone: &Path) -> Vec<crate::trust::Grant> {
        match named_or("CODEX_HOME", Some(&crate::servers::home()), ".codex") {
            Some(codex_home) => crate::trust::codex(&codex_home, clone),
            None => Vec::new(),
        }
    }

    fn holds_trust(&self, _worktree: &Path, clone: &Path) -> Result<bool, String> {
        let codex_home = named_or("CODEX_HOME", Some(&crate::servers::home()), ".codex")
            .ok_or("no CODEX_HOME and no HOME")?;
        crate::trust::codex_holds(&codex_home, clone)
    }

    fn asks_trust(&self, screen: &str) -> bool {
        screen.contains("Trust this folder?") && screen.contains("Trust and continue")
    }

    fn reviews_hooks(&self) -> bool {
        true
    }

    /// Codex runs only hooks a person trusted in its `/hooks` review, by their hash, so Priory's
    /// hooks never change between runs and this command shows the same ones.
    fn trust(&self, exe: &Path, clone: &str, _worktrees: &str) -> String {
        let overrides: String = codex_hooks(exe)
            .into_iter()
            .filter_map(|(key, value)| override_of(key, value).ok())
            .map(|over| format!(" -c '{}'", over.replace('\'', r"'\''")))
            .collect();
        format!(
            "trust the clone and Priory's hooks once: cd {clone} && codex{overrides}, accept the folder, type /hooks and trust Priory's hooks, then exit"
        )
    }
}

/// Codex's `-c key=value`, with the value as TOML.
fn override_of(key: &str, value: serde_json::Value) -> Result<String, String> {
    let value = toml::Value::try_from(value).map_err(|error| error.to_string())?;
    Ok(format!("{key}={value}"))
}

/// The same hooks for every run: the run's session reaches them as `PRIORY_SESSION`.
fn codex_hooks(exe: &Path) -> Vec<(&'static str, serde_json::Value)> {
    let quoted = quote(exe);
    let hook = |command: String, matcher: Option<&str>| {
        let mut group = json!({ "hooks": [{ "type": "command", "command": command }] });
        if let Some(matcher) = matcher {
            group["matcher"] = matcher.into();
        }
        json!([group])
    };
    let signal = |event: &str| hook(format!("{quoted} hook {event}"), None);
    let respond = |output: &str| format!("printf '%s' '{output}'");
    vec![
        ("hooks.SessionStart", signal("SessionStart")),
        ("hooks.UserPromptSubmit", signal("UserPromptSubmit")),
        ("hooks.Stop", signal("Stop")),
        // Codex 0.170 fires SessionStart with source compact only with the next turn.
        ("hooks.PostCompact", signal("PostCompact")),
        (
            "hooks.PreToolUse",
            json!([
                hook(respond(ASK_INSTEAD), Some("^request_user_input$"))[0],
                guard(&quoted, None),
            ]),
        ),
        ("hooks.PermissionRequest", hook(respond(DENY), None)),
    ]
}

/// The first line `program --version` prints, when it runs within the command timeout.
pub fn version(program: &str) -> Option<String> {
    let output = fed_within(
        Command::new(program).arg("--version"),
        None,
        command_timeout(),
    )
    .ok()??;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// Why a run did not start: one line, the cause first, and the tmux command when it ran one.
#[derive(Debug)]
pub struct NotStarted {
    pub reason: String,
    pub command: Option<String>,
}

impl From<String> for NotStarted {
    fn from(reason: String) -> Self {
        Self {
            reason,
            command: None,
        }
    }
}

/// The socket a tmux client of this process uses: the one `TMUX` names, else
/// `$TMUX_TMPDIR/tmux-<uid>/default`, with `/tmp` when `TMUX_TMPDIR` is unset.
pub(crate) fn tmux_socket() -> PathBuf {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    if let Some(socket) = set("TMUX").and_then(|tmux| {
        let tmux = tmux.to_string_lossy().into_owned();
        tmux.split(',').next().map(PathBuf::from)
    }) {
        return socket;
    }
    tmux_socket_in(&set("TMUX_TMPDIR").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from))
}

/// The default socket of a tmux server started with `TMUX_TMPDIR` set to `dir`.
pub fn tmux_socket_in(dir: &Path) -> PathBuf {
    dir.join(format!("tmux-{}", uid())).join("default")
}

/// Refuses a tmux socket path that cannot be bound, before tmux fails on it at the end of a long
/// error.
pub(crate) fn tmux_socket_fits(socket: &Path) -> Result<(), String> {
    let length = socket.as_os_str().len();
    if length < 108 {
        return Ok(());
    }
    Err(format!(
        "the tmux socket path {} is {length} bytes, over the 108-byte limit of a Unix socket; set \
         TMUX_TMPDIR to a shorter folder, such as /tmp",
        socket.display()
    ))
}

/// `stderr` as one line.
pub fn one_line(stderr: &str) -> String {
    let lines: Vec<_> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("; ")
}

/// Runs the rest under `direnv exec "$1"`, with direnv's stderr in `$0/direnv.error`. When direnv
/// failed before the harness started, which no `$0/direnv.started` tells, it marks
/// `$0/direnv.failed` and runs the rest without the `.envrc`.
/// direnv alone gets the run's private `XDG_DATA_HOME`. The harness gets the old one, or the one the
/// `.envrc` exports.
pub(crate) const DIRENV_EXEC: &str = r#"s=$0 d=$1; shift
PRIORY_DATA_HOME=${XDG_DATA_HOME-} XDG_DATA_HOME=$DIRENV_CONFIG/data direnv exec "$d" sh -c ': > "$0"; exec 2>&3 3>&-
if [ "${XDG_DATA_HOME-}" = "$1" ]; then
  if [ -n "${PRIORY_DATA_HOME-}" ]; then XDG_DATA_HOME=$PRIORY_DATA_HOME; else unset XDG_DATA_HOME; fi
fi
unset PRIORY_DATA_HOME; shift; exec "$@"' "$s/direnv.started" "$DIRENV_CONFIG/data" "$@" 3>&2 2>"$s/direnv.error"
c=$?; [ -e "$s/direnv.started" ] && exit $c; : > "$s/direnv.failed"; exec "$@""#;

/// The folder where a run under direnv leaves `direnv.started` and `direnv.error`: its spool, which
/// a jail binds writable.
pub fn direnv_spool(home: &Path, session: &str) -> PathBuf {
    let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    crate::jail::spool_dir(&home, session)
}

/// What direnv wrote to stderr when it kept the run `session` from starting in its `.envrc`, if it
/// did.
pub fn direnv_failed(home: &Path, session: &str) -> Option<String> {
    let spool = direnv_spool(home, session);
    if !spool.join("direnv.failed").exists() {
        return None;
    }
    let error = read_regular(&spool.join("direnv.error"), 64 << 10).unwrap_or_default();
    Some(String::from_utf8_lossy(&error).into_owned())
}

// Not libc's, which would be a new dependency for two numbers.
#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "powerpc64"
    )
))]
const O_NOFOLLOW: i32 = 0o100000;
#[cfg(all(
    target_os = "linux",
    not(any(
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "powerpc64"
    ))
))]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(target_os = "macos")]
const O_NOFOLLOW: i32 = 0x100;
#[cfg(target_os = "linux")]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(target_os = "macos")]
const O_NONBLOCK: i32 = 0x4;

/// The first `cap` bytes of the regular file at `path`, which an agent can write: no symlink is
/// followed, and no FIFO blocks the daemon.
pub(crate) fn read_regular(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// How long ago direnv handed the run `session` over to its harness, with or without the `.envrc`,
/// if it did.
pub fn direnv_handed(home: &Path, session: &str) -> Option<Duration> {
    let spool = direnv_spool(home, session);
    ["direnv.started", "direnv.failed"]
        .iter()
        .find_map(|name| fs::metadata(spool.join(name)).ok()?.modified().ok())
        .map(|at| at.elapsed().unwrap_or_default())
}

/// Whether the run `session` started under direnv is still loading its `.envrc`.
pub fn direnv_loading(home: &Path, session: &str) -> bool {
    let spool = direnv_spool(home, session);
    spool.join("direnv.error").exists()
        && !spool.join("direnv.started").exists()
        && !spool.join("direnv.failed").exists()
}

/// Starts the harness in a detached tmux session, with hooks that report to this daemon. The same
/// tmux command tags the new pane with the run's session id, so a crash cannot leave it untagged.
pub fn launch(harness: &dyn Harness, launch: &Launch) -> Result<(), NotStarted> {
    tmux_socket_fits(&tmux_socket())?;
    let exe = exe()?;
    if let Some((file, text)) = harness.file(launch) {
        fs::create_dir_all(file.parent().expect("the file has a parent"))
            .and_then(|()| fs::write(&file, text))
            .map_err(|error| format!("{}: {error}", file.display()))?;
    }
    if let Some((file, text)) = harness.private_file(&exe, launch) {
        write_private(&file, text.as_bytes())
            .map_err(|error| format!("{}: {error}", file.display()))?;
    }
    let mut command = tmux();
    command
        .args([
            "new-session",
            "-d",
            "-x",
            "200",
            "-y",
            "50",
            "-s",
            launch.tmux,
            "-c",
        ])
        .arg(launch.dir)
        .arg("-e")
        .arg(format!("PRIORY_HOME={}", launch.home.display()))
        .arg("-e")
        .arg(format!("PRIORY_SESSION={}", launch.session));
    // A tmux server started elsewhere has its own, and the hooks must find this daemon's socket.
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|run| !run.is_empty()) {
        command
            .arg("-e")
            .arg(format!("XDG_RUNTIME_DIR={}", run.to_string_lossy()));
    }
    command.args(launch.env.iter().flat_map(|variable| ["-e", variable]));
    if let Some(folder) = launch.direnv {
        command
            .arg("-e")
            .arg(format!("DIRENV_CONFIG={}", folder.display()));
    }
    command.arg("--");
    if let Some(prompt) = launch.prompt {
        // tmux refuses a command over about 16 KiB, so the prompt reaches the harness through a file.
        let file = launch.home.join("prompts").join(launch.session);
        fs::create_dir_all(file.parent().expect("the file has a parent"))
            .and_then(|()| fs::write(&file, prompt))
            .map_err(|error| format!("{}: {error}", file.display()))?;
        command
            .args([
                "sh",
                "-c",
                r#"p=$(cat "$0"; printf x); rm -f "$0"; exec "$@" "${p%x}""#,
            ])
            .arg(&file);
    }
    let jail = match launch.jail {
        Some(jail) => {
            let files: Vec<PathBuf> = harness
                .file(launch)
                .into_iter()
                .chain(harness.private_file(&exe, launch))
                .map(|(file, _)| file)
                .collect();
            jail.command(harness, &exe, launch, &files)?
        }
        None => Vec::new(),
    };
    // A scope execs the agent in place, so the pane's process is still the agent's, or the jail's.
    command.args(launch.scope).args(&jail);
    if launch.direnv.is_some() {
        let spool = direnv_spool(launch.home, launch.session);
        fs::create_dir_all(&spool).map_err(|error| format!("{}: {error}", spool.display()))?;
        // A resumed session keeps its spool, and the daemon must not read its last run's markers.
        for name in ["direnv.started", "direnv.failed", "direnv.error"] {
            let _ = fs::remove_file(spool.join(name));
        }
        // direnv's own failure exits 1 before the harness starts, as the harness's exit 1 might:
        // the marker tells them apart, and the harness then starts without the `.envrc`.
        command
            .args(["sh", "-c", DIRENV_EXEC])
            .arg(&spool)
            .arg(launch.dir);
    }
    command
        .arg("env")
        .args(NESTED.iter().flat_map(|name| ["-u", name]))
        .arg(harness.program())
        .args(harness.args(&exe, launch)?);
    if launch.prompt.is_some() {
        command.arg("--");
    }
    command.args([";", "set-option", "-p", "@priory", launch.session]);
    let shown = format!("{command:?}");
    let failed = |reason: String| NotStarted {
        reason,
        command: Some(shown.clone()),
    };
    let output = fed_within(&mut command, None, command_timeout())
        .map_err(|error| failed(format!("tmux new-session could not run: {error}")))?
        .ok_or_else(|| {
            failed(format!(
                "tmux new-session timed out after {}s",
                command_timeout().as_secs()
            ))
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = one_line(&String::from_utf8_lossy(&output.stderr));
    Err(failed(if stderr.is_empty() {
        format!("tmux new-session failed with {}", output.status)
    } else {
        format!("tmux new-session failed: {stderr}")
    }))
}

/// The bytes of `path` on the default branch of Priory's clone of `repo`.
pub fn repo_file(home: &Path, repo: &str, path: &str) -> Result<Vec<u8>, String> {
    let clone = home.join("repos").join(repo);
    let output = git(&clone)
        .args(["show", &format!("origin/HEAD:{path}")])
        .output()
        .map_err(|error| format!("git show in {}: {error}", clone.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git show origin/HEAD:{path} in {}: {}",
            clone.display(),
            one_line(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    Ok(output.stdout)
}

/// Writes `files`, by their paths inside the plugin, into `<dir>/plugin` unless that folder holds
/// them already, and returns it.
pub fn plugin(dir: &Path, files: &[(PathBuf, &[u8])]) -> Result<PathBuf, String> {
    write_once(dir, "plugin", files)
}

/// Writes `files`, by their paths inside `folder`, into `<dir>/<folder>` unless that folder holds
/// exactly them, read-only, and returns it. The files go to a staging folder renamed into place,
/// so a crash never leaves a half-written snapshot for a later run, and a snapshot a run edited is
/// replaced before another run reads it.
pub fn write_once(dir: &Path, folder: &str, files: &[(PathBuf, &[u8])]) -> Result<PathBuf, String> {
    let target = dir.join(folder);
    let wanted: Files = files
        .iter()
        .map(|(path, bytes)| (Path::new(folder).join(path), bytes.to_vec()))
        .collect();
    if sealed(dir).as_ref() == Some(&wanted) {
        return Ok(target);
    }
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let staging = dir.with_file_name(format!(
        ".{}.{}",
        name.unwrap_or_default(),
        std::process::id()
    ));
    remove_sealed(&staging);
    let written = files
        .iter()
        .try_for_each(|(path, bytes)| {
            let file = staging.join(folder).join(path);
            fs::create_dir_all(file.parent().expect("the file has a parent"))?;
            fs::write(file, bytes)
        })
        .and_then(|()| seal(&staging))
        .and_then(|()| replace(&staging, dir));
    remove_sealed(&staging);
    match written {
        Ok(()) => Ok(target),
        Err(_) if sealed(dir).as_ref() == Some(&wanted) => Ok(target),
        Err(error) => Err(format!("{}: {error}", target.display())),
    }
}

/// Every file under a folder, by its path inside it, with its bytes.
type Files = std::collections::BTreeMap<PathBuf, Vec<u8>>;

/// The files under `dir`, or none when `dir`, a folder or a file under it has a write bit, or
/// it holds anything but folders and files.
fn sealed(dir: &Path) -> Option<Files> {
    let mut files = Files::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let meta = fs::symlink_metadata(&folder).ok()?;
        if !meta.is_dir() || meta.permissions().mode() & 0o222 != 0 {
            return None;
        }
        for entry in fs::read_dir(&folder).ok()? {
            let path = entry.ok()?.path();
            let meta = fs::symlink_metadata(&path).ok()?;
            if meta.is_dir() {
                folders.push(path);
            } else if meta.is_file() && meta.permissions().mode() & 0o222 == 0 {
                let inside = path.strip_prefix(dir).ok()?.to_path_buf();
                files.insert(inside, fs::read(&path).ok()?);
            } else {
                return None;
            }
        }
    }
    Some(files)
}

/// Makes every file under `dir` 0444 and every folder 0555, `dir` included, so a run's tools
/// cannot write to it without changing its mode first.
fn seal(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.is_dir() {
            seal(&path)?;
        } else {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444))?;
        }
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o555))
}

/// Moves `path`, a worktree folder the daemon could not remove (a container can leave root-owned
/// files under an ignored folder), into `home/trash`, so the task's worktree can be added back. A
/// rename needs write access on the parent folder only.
pub(crate) fn move_aside(home: &Path, path: &Path) -> Result<(), String> {
    let trash = home.join("trash");
    fs::create_dir_all(&trash).map_err(|error| format!("{}: {error}", trash.display()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let aside = trash.join(format!("{name}-{stamp}"));
    fs::rename(path, &aside).map_err(|error| {
        format!(
            "{} could not be removed or moved aside ({error}): remove it by hand",
            path.display()
        )
    })
}

/// Removes `dir` and everything under it, a sealed folder included. Use it for any folder that
/// can hold a `PRIORY_HOME`: a user other than root cannot remove a sealed kit, plugin or memory
/// folder with `fs::remove_dir_all` alone.
pub fn remove_sealed(dir: &Path) {
    unseal(dir);
    let _ = fs::remove_dir_all(dir);
}

/// Makes `dir` and every folder under it writable by its owner, so their files can be removed.
pub(crate) fn unseal(dir: &Path) {
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        if !fs::symlink_metadata(&folder).is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let _ = fs::set_permissions(&folder, fs::Permissions::from_mode(0o755));
        for entry in fs::read_dir(&folder).into_iter().flatten().flatten() {
            folders.push(entry.path());
        }
    }
}

/// Renames `staging` to `target`, and moves aside and removes what `target` held before.
fn replace(staging: &Path, target: &Path) -> std::io::Result<()> {
    if fs::symlink_metadata(target).is_err() {
        return fs::rename(staging, target);
    }
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let old = target.with_file_name(format!(".{name}.{}.old", std::process::id()));
    remove_sealed(&old);
    fs::rename(target, &old)?;
    let renamed = fs::rename(staging, target);
    remove_sealed(&old);
    renamed
}

/// Runs git `args` and returns its stdout, or an error naming `what` with git's first line of
/// complaint, short enough for a banner.
fn kit_git(what: &str, command: &mut Command) -> Result<String, String> {
    kit_git_fed(what, command, None)
}

/// As `kit_git`, with `input` on git's stdin.
fn kit_git_fed(
    what: &str,
    command: &mut Command,
    input: Option<Vec<u8>>,
) -> Result<String, String> {
    let output = fed_within(
        command.env("GIT_TERMINAL_PROMPT", "0"),
        input,
        command_timeout(),
    )
    .map_err(|error| format!("{what}: {error}"))?
    .ok_or_else(|| format!("{what}: timed out after {}s", command_timeout().as_secs()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("failed");
        return Err(format!("{what}: {line}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Fetches `reference` of `repo` into the bare mirror `mirror`, made on first use, and returns the
/// commit it names. Git's own credentials fetch it, and a prompt for one fails instead of waiting.
pub fn kit_fetch(mirror: &Path, repo: &str, reference: &str) -> Result<String, String> {
    kit_mirror(mirror)?;
    kit_git(
        &format!("git fetch {repo} {reference}"),
        git(mirror).args(["fetch", "--quiet", "--no-tags", "--", repo, reference]),
    )?;
    kit_git(
        "git rev-parse",
        git(mirror).args(["rev-parse", "--verify", "--quiet", "FETCH_HEAD^{commit}"]),
    )
}

/// Makes the bare mirror `mirror` unless it exists.
fn kit_mirror(mirror: &Path) -> Result<(), String> {
    if !mirror.join("HEAD").is_file() {
        fs::create_dir_all(mirror).map_err(|error| format!("{}: {error}", mirror.display()))?;
        kit_git(
            "git init",
            Command::new("git")
                .args(["init", "--bare", "-q"])
                .arg(mirror),
        )?;
    }
    Ok(())
}

/// A commit that `kit_commit` made on a kit's ref, and the files it changed, each as git's status
/// letter (`A`, `M` or `D`) and path.
pub struct KitCommit {
    pub commit: String,
    pub changed: Vec<(char, String)>,
}

/// Fetches `reference` of `repo` into the bare mirror `mirror`, then commits `files` on top of it:
/// each path with its bytes, or none to remove it. The commit is dated `date` and authored by
/// Priory, so the same files on the same ref make the same commit. Returns none when the files
/// leave the ref's tree as it is.
pub fn kit_commit(
    mirror: &Path,
    repo: &str,
    reference: &str,
    files: &[(String, Option<Vec<u8>>)],
    message: &str,
    date: &str,
) -> Result<Option<KitCommit>, String> {
    kit_mirror(mirror)?;
    // A ref of its own, and no FETCH_HEAD, so the kit's own fetch on the daemon's tick never reads
    // this one's.
    let base_ref = "refs/priory/memories-base";
    kit_git(
        &format!("git fetch {repo} {reference}"),
        git(mirror).args([
            "fetch",
            "--quiet",
            "--no-tags",
            "--no-write-fetch-head",
            "--",
            repo,
            &format!("+{reference}:{base_ref}"),
        ]),
    )?;
    let base = kit_git(
        "git rev-parse",
        git(mirror).args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{base_ref}^{{commit}}"),
        ]),
    )?;
    let index = mirror.join(format!("priory-memories.{}.index", std::process::id()));
    let _ = fs::remove_file(&index);
    let indexed = |args: &[&str]| {
        let mut command = git(mirror);
        command.args(args).env("GIT_INDEX_FILE", &index);
        command
    };
    let made = (|| {
        kit_git("git read-tree", &mut indexed(&["read-tree", &base]))?;
        // A zero mode removes the path; `--index-info` needs no work tree, which a mirror lacks.
        let mut info = Vec::new();
        for (path, bytes) in files {
            let entry = match bytes {
                Some(bytes) => {
                    let blob = kit_git_fed(
                        "git hash-object",
                        git(mirror).args(["hash-object", "-w", "--stdin"]),
                        Some(bytes.clone()),
                    )?;
                    format!("100644 {blob}\t{path}\n")
                }
                None => format!("0 {}\t{path}\n", "0".repeat(40)),
            };
            info.extend(entry.into_bytes());
        }
        kit_git_fed(
            "git update-index",
            &mut indexed(&["update-index", "--index-info"]),
            Some(info),
        )?;
        let tree = kit_git("git write-tree", &mut indexed(&["write-tree"]))?;
        let changed = kit_git(
            "git diff-tree",
            git(mirror).args(["diff-tree", "-r", "--name-status", &base, &tree]),
        )?;
        let changed: Vec<(char, String)> = changed
            .lines()
            .filter_map(|line| {
                let (status, path) = line.split_once('\t')?;
                Some((status.chars().next()?, path.to_owned()))
            })
            .collect();
        if changed.is_empty() {
            return Ok(None);
        }
        let mut command = git(mirror);
        command.args(["commit-tree", &tree, "-p", &base, "-F", "-"]);
        for (name, value) in [
            ("GIT_AUTHOR_NAME", "Priory"),
            ("GIT_AUTHOR_EMAIL", "priory@localhost"),
            ("GIT_AUTHOR_DATE", date),
            ("GIT_COMMITTER_NAME", "Priory"),
            ("GIT_COMMITTER_EMAIL", "priory@localhost"),
            ("GIT_COMMITTER_DATE", date),
        ] {
            command.env(name, value);
        }
        let commit = kit_git_fed(
            "git commit-tree",
            &mut command,
            Some(message.as_bytes().to_vec()),
        )?;
        Ok(Some(KitCommit { commit, changed }))
    })();
    let _ = fs::remove_file(&index);
    made
}

/// Pushes `commit` of the bare mirror `mirror` to `branch` of `repo`, replacing what it held. Git's
/// own credentials push it, and a prompt for one fails instead of waiting.
pub fn kit_push(mirror: &Path, repo: &str, commit: &str, branch: &str) -> Result<(), String> {
    kit_git(
        &format!("git push {repo} {branch}"),
        git(mirror).args([
            "push",
            "--quiet",
            "--force",
            "--",
            repo,
            &format!("{commit}:refs/heads/{branch}"),
        ]),
    )
    .map(|_| ())
}

/// Opens a pull request from `branch` of the GitHub repository `repo` onto `base`, or its default
/// branch without one, titled `title` and described by `body`. An open one from `branch` is edited
/// to that title and body instead. Returns its number.
pub fn kit_pull_request(
    repo: &str,
    branch: &str,
    base: Option<&str>,
    title: &str,
    body: &str,
) -> Result<u64, String> {
    let open = run(Command::new("gh").args([
        "pr", "list", "-R", repo, "--head", branch, "--state", "open", "--json", "number",
    ]))?;
    let open: Vec<serde_json::Value> = serde_json::from_str(&open)
        .map_err(|error| format!("gh pr list printed {open:?}: {error}"))?;
    if let Some(number) = open.first().and_then(|pull| pull["number"].as_u64()) {
        let shown = number.to_string();
        run(Command::new("gh").args([
            "pr", "edit", &shown, "-R", repo, "--title", title, "--body", body,
        ]))?;
        return Ok(number);
    }
    let mut command = Command::new("gh");
    command.args(["pr", "create", "-R", repo]);
    if let Some(base) = base {
        command.args(["--base", base]);
    }
    command.args(["--head", branch, "--title", title, "--body", body]);
    let url = run(&mut command)?;
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| format!("gh pr create printed {url:?}, not a pull request URL"))
}

/// Checks `commit` of the bare mirror `mirror` out into `<dir>/<commit>`, read-only, unless that
/// folder holds the commit's files already, and returns it. It goes to a staging folder renamed
/// into place, so a crash never leaves half a checkout, and a symbolic link becomes a file that
/// holds its target. An edited checkout is replaced before its files are read again.
pub fn kit_checkout(mirror: &Path, dir: &Path, commit: &str) -> Result<PathBuf, String> {
    let target = dir.join(commit);
    let staging = dir.join(format!(".{commit}.{}", std::process::id()));
    let index = dir.join(format!(".{commit}.{}.index", std::process::id()));
    remove_sealed(&staging);
    fs::create_dir_all(&staging).map_err(|error| format!("{}: {error}", staging.display()))?;
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(mirror)
            .arg("--work-tree")
            .arg(&staging)
            .args(["-c", "core.symlinks=false"])
            .args(args)
            .env("GIT_INDEX_FILE", &index);
        command
    };
    let at = |error: std::io::Error| format!("{}: {error}", target.display());
    let written = kit_git("git read-tree", &mut git(&["read-tree", commit]))
        .and_then(|_| {
            kit_git(
                "git checkout-index",
                &mut git(&["checkout-index", "-a", "-f"]),
            )
        })
        .and_then(|_| seal(&staging).map_err(at))
        .and_then(|()| {
            let kept = sealed(&target);
            if kept.is_some() && kept == sealed(&staging) {
                return Ok(());
            }
            replace(&staging, &target).map_err(at)
        });
    let _ = fs::remove_file(&index);
    remove_sealed(&staging);
    match written {
        Ok(()) => Ok(target),
        Err(_) if sealed(&target).is_some() => Ok(target),
        Err(error) => Err(error),
    }
}

/// The MCP server that serves Priory's tools to the run, `priory mcp --session <id>`, and the
/// servers its agent names.
fn mcp_config(exe: &Path, launch: &Launch) -> String {
    let mut servers = launch.servers.defined.clone();
    servers.insert(
        "priory".to_owned(),
        json!({
            "command": exe,
            "args": mcp_args(launch),
            "env": mcp_env(launch.home),
        }),
    );
    json!({ "mcpServers": servers }).to_string()
}

/// Writes `text` to `file`, readable by its owner alone, even when an older file was not.
pub(crate) fn write_private(file: &Path, text: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(file.parent().expect("the file has a parent"))?;
    let mut written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(file)?;
    written.set_permissions(fs::Permissions::from_mode(0o600))?;
    written.write_all(text)
}

/// `priory mcp --session <id>`, with `--debate` for a debate turn, `--gives-up` when the memory gate has
/// a limit and the flag of its tool set, if any.
fn mcp_args<'a>(launch: &Launch<'a>) -> Vec<&'a str> {
    let mut args = vec!["mcp", "--session", launch.session];
    if launch.debate {
        args.push("--debate");
    }
    if launch.gives_up {
        args.push("--gives-up");
    }
    args.extend(launch.tools.flag());
    args
}

/// What `priory mcp` needs from the daemon: its home, and its state file as an absolute path, since
/// the bridge reads that file itself from the agent's worktree.
fn mcp_env(home: &Path) -> serde_json::Value {
    let mut env = json!({ "PRIORY_HOME": home });
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|run| !run.is_empty()) {
        env["XDG_RUNTIME_DIR"] = json!(run.to_string_lossy());
    }
    if let Some(db) = std::env::var_os("PRIORY_DB").and_then(|db| std::path::absolute(db).ok()) {
        env["PRIORY_DB"] = json!(db);
    }
    env
}

fn quote(exe: &Path) -> String {
    format!("'{}'", exe.display().to_string().replace('\'', r"'\''"))
}

/// The hook group that asks Priory's tool guard about each call `matcher` matches. Its timeout
/// stays well above the hook's own deadline, since a hook Claude kills lets the call through.
fn guard(quoted: &str, matcher: Option<&str>) -> serde_json::Value {
    let mut group = json!({ "hooks": [{
        "type": "command",
        "command": format!("{quoted} hook PreToolUse"),
        "timeout": 30,
    }] });
    if let Some(matcher) = matcher {
        group["matcher"] = matcher.into();
    }
    group
}

/// The Claude tools whose calls reach `priory hook PreToolUse`: the guard reads the commands and
/// MCP calls, and the file tools bring in the skills and memories whose `paths` match their file.
const CLAUDE_HOOKED: &str = "Bash|mcp__.*|Read|Edit|Write|MultiEdit|NotebookEdit";

const ASK_INSTEAD: &str = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Priory runs this session unattended. Call the ask tool of the priory MCP server instead."}}"#;

const DENY: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Priory runs this session unattended and never approves a permission prompt, so it denied this one. Go on without it. If the task cannot go on without it, call the ask or result tool of the priory MCP server."}}}"#;

/// Claude Code settings whose hooks call `priory hook <event>`, that allow Priory's tools, deny the
/// built-in question tool in favour of `ask` (R6.4), deny every permission request (R7.2), and
/// report a permission prompt that shows anyway.
fn settings(exe: &Path, permissions: &serde_json::Value) -> String {
    let quoted = quote(exe);
    let hooks: serde_json::Map<_, _> = ["SessionStart", "UserPromptSubmit", "Stop"]
        .into_iter()
        .map(|event| {
            let command = format!("{quoted} hook {event}");
            (
                event.to_owned(),
                json!([{ "hooks": [{ "type": "command", "command": command }] }]),
            )
        })
        .collect();
    let mut hooks = hooks;
    hooks.insert(
        "PreToolUse".to_owned(),
        json!([
            {
                "matcher": "AskUserQuestion",
                "hooks": [{ "type": "command", "command": format!("printf '%s' '{ASK_INSTEAD}'") }],
            },
            guard(&quoted, Some(CLAUDE_HOOKED)),
        ]),
    );
    hooks.insert(
        "PermissionRequest".to_owned(),
        json!([{
            "hooks": [{ "type": "command", "command": format!("printf '%s' '{DENY}'") }],
        }]),
    );
    hooks.insert(
        "Notification".to_owned(),
        json!([{
            "matcher": "permission_prompt",
            "hooks": [{ "type": "command", "command": format!("{quoted} hook PermissionPrompt") }],
        }]),
    );
    let mut permissions = permissions.clone();
    permissions["allow"]
        .as_array_mut()
        .expect("allow is an array")
        .insert(0, "mcp__priory".into());
    json!({ "hooks": hooks, "permissions": permissions }).to_string()
}

/// Starts a headless `claude -p` call of `model` on `prompt`, in `dir`, with no settings or MCP
/// servers of the user's, so it cannot be granted a tool that needs permission. Its JSON answer goes to `<id>.json` in `dir`, which [`answer`] reads.
pub fn ask_claude(
    dir: &Path,
    id: i64,
    model: &str,
    prompt: &str,
    timeout: Duration,
) -> Result<crate::script::Script, String> {
    let file = |extension: &str| dir.join(format!("{id}.{extension}"));
    fs::create_dir_all(dir)
        .and_then(|()| fs::write(file("prompt"), prompt))
        .map_err(|error| format!("{}: {error}", dir.display()))?;
    let unset: String = NESTED.iter().map(|name| format!("-u {name} ")).collect();
    let run = format!(
        "exec env {unset}claude -p --output-format json --model {} --no-session-persistence \
         --strict-mcp-config --setting-sources project < {} > {}",
        quote(Path::new(model)),
        quote(&file("prompt")),
        quote(&file("json"))
    );
    crate::script::spawn(&run, dir, &file("log"), timeout)
}

/// Whether the person's `claude`, run in `dir`, finds the MCP server `server`, as
/// `claude mcp get <server>` does: `Ok(false)` when it says it has no server of that name. Its
/// output is never kept, since for a server of the person's config it prints headers and env
/// values. Past `timeout`, it is killed with its children.
pub fn claude_finds(dir: &Path, server: &str, timeout: Duration) -> Result<bool, String> {
    let mut command = Command::new("claude");
    for name in NESTED {
        command.env_remove(name);
    }
    let child = command
        .current_dir(dir)
        .args(["mcp", "get", server])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("cannot start: {error}"))?;
    let pid = child.id();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || sender.send(child.wait_with_output()));
    match receiver.recv_timeout(timeout) {
        Ok(Ok(output)) if output.status.success() => Ok(true),
        Ok(Ok(output)) => {
            let said = |text: &[u8]| String::from_utf8_lossy(text).contains("No MCP server named");
            if said(&output.stderr) || said(&output.stdout) {
                Ok(false)
            } else {
                Err(format!("ended with {}", output.status))
            }
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => {
            crate::script::kill(pid);
            Err(format!("did not answer within {}s", timeout.as_secs()))
        }
    }
}

/// The JSON answer of handler call `id` in `dir`, if it wrote one.
pub fn answer(dir: &Path, id: i64) -> Option<String> {
    fs::read_to_string(dir.join(format!("{id}.json"))).ok()
}

/// Removes the files of handler call `id` in `dir`, once its answer is recorded.
pub fn forget(dir: &Path, id: i64) {
    for extension in ["prompt", "json", "log"] {
        let _ = fs::remove_file(dir.join(format!("{id}.{extension}")));
    }
}

/// The pane of the tmux session `name` that runs the harness session `session`, if its process
/// lives. A tmux with `remain-on-exit` keeps a dead pane, which counts as gone.
pub fn pane(name: &str, session: &str) -> Option<String> {
    let panes = run(tmux().args([
        "list-panes",
        "-s",
        "-t",
        &format!("={name}:"),
        "-F",
        "#{pane_id} #{pane_dead} #{@priory}",
    ]))
    .ok()?;
    panes.lines().find_map(|line| {
        let mut fields = line.splitn(3, ' ');
        let (pane, dead, tag) = (fields.next()?, fields.next()?, fields.next()?);
        (dead == "0" && tag == session).then(|| pane.to_owned())
    })
}

/// The process id of the live pane of the tmux session `name` that runs the harness session
/// `session`. A scope execs the agent in place, so it is the scope's process.
pub fn pane_pid(name: &str, session: &str) -> Option<u32> {
    let panes = run(tmux().args([
        "list-panes",
        "-s",
        "-t",
        &format!("={name}:"),
        "-F",
        "#{pane_pid} #{pane_dead} #{@priory}",
    ]))
    .ok()?;
    panes.lines().find_map(|line| {
        let mut fields = line.splitn(3, ' ');
        let (pid, dead, tag) = (fields.next()?, fields.next()?, fields.next()?);
        (dead == "0" && tag == session)
            .then(|| pid.parse().ok())
            .flatten()
    })
}

/// Every tmux session with a pane Priory tagged, and that pane's harness session id.
pub fn tagged() -> Vec<(String, String)> {
    let panes = run(tmux().args(["list-panes", "-a", "-F", "#{session_name} #{@priory}"]))
        .unwrap_or_default();
    panes
        .lines()
        .filter_map(|line| {
            let (name, tag) = line.rsplit_once(' ')?;
            (!tag.is_empty()).then(|| (name.to_owned(), tag.to_owned()))
        })
        .collect()
}

/// The last 30 non-empty lines on the active pane of the tmux session `name`, each wrapped row
/// joined to the row it continues so a secret that wraps reads whole, or none once it is gone.
pub fn screen(name: &str) -> Vec<String> {
    shown(&format!("={name}:"), true)
}

/// The last 30 non-empty lines on the pane of the tmux session `name` that runs the harness session
/// `session`, whichever pane is active, or none once it is gone.
pub fn agent_screen(name: &str, session: &str) -> Vec<String> {
    pane(name, session).map_or_else(Vec::new, |pane| shown(&pane, false))
}

fn shown(target: &str, joined: bool) -> Vec<String> {
    let mut capture = tmux();
    capture.args(["capture-pane", "-p", "-t", target]);
    if joined {
        capture.arg("-J");
    }
    let shown = run(&mut capture).unwrap_or_default();
    let lines: Vec<_> = shown
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    lines[lines.len().saturating_sub(30)..].to_vec()
}

/// What a pane shows: its size, its cursor unless hidden, how many rows of scrollback it keeps, and
/// each row with its SGR colour codes, trailing spaces trimmed: `earlier` rows of scrollback, then
/// the screen.
pub struct Snapshot {
    pub width: u16,
    pub height: u16,
    pub cursor: Option<(u16, u16)>,
    pub scrollback: usize,
    pub earlier: usize,
    pub lines: Vec<String>,
}

/// The screen of the live pane of the tmux session `name` that runs the harness session `session`,
/// after up to `history` rows of its scrollback, read by that pane's id, or none once it is gone or
/// when tmux takes longer than `deadline`.
pub fn snapshot(name: &str, session: &str, history: usize, deadline: Duration) -> Option<Snapshot> {
    let end = std::time::Instant::now() + deadline;
    let within = |command: &mut Command| {
        let left = end.saturating_duration_since(std::time::Instant::now());
        let output = fed_within(command, None, left).ok()??;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let panes = within(tmux().args([
        "list-panes",
        "-s",
        "-t",
        &format!("={name}:"),
        "-F",
        "#{pane_id} #{pane_dead} #{pane_width} #{pane_height} #{cursor_flag} #{cursor_x} #{cursor_y} #{history_size} #{@priory}",
    ]))?;
    let (pane, mut snapshot) = tagged_pane(&panes, session)?;
    snapshot.earlier = history.min(snapshot.scrollback);
    let start = format!("-{}", snapshot.earlier);
    let shown = within(tmux().args(["capture-pane", "-p", "-e", "-S", &start, "-t", pane]))?;
    snapshot.lines = shown
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect();
    snapshot.lines.resize(
        snapshot.earlier + usize::from(snapshot.height),
        String::new(),
    );
    Some(snapshot)
}

/// The live pane running the harness session `session` in a `list-panes` listing of id, dead flag,
/// width, height, cursor flag, cursor x and y, scrollback rows and tag: its id, its size, its cursor
/// unless hidden and its scrollback, with no rows yet.
fn tagged_pane<'a>(panes: &'a str, session: &str) -> Option<(&'a str, Snapshot)> {
    panes.lines().find_map(|line| {
        let fields: Vec<_> = line.splitn(9, ' ').collect();
        let [pane, "0", width, height, shown, x, y, scrollback, tag] = fields[..] else {
            return None;
        };
        if tag != session {
            return None;
        }
        let cursor = match shown {
            "1" => Some((x.parse().ok()?, y.parse().ok()?)),
            "0" => None,
            _ => return None,
        };
        let (width, height) = (width.parse().ok()?, height.parse().ok()?);
        let scrollback = scrollback.parse().ok()?;
        let lines = Vec::new();
        Some((
            pane,
            Snapshot {
                width,
                height,
                cursor,
                scrollback,
                earlier: 0,
                lines,
            },
        ))
    })
}

/// The command that attaches a tmux client to the session `name`, through the socket of the server
/// that runs it, or none unless a live pane there runs the harness session `session`. A
/// `read_only` client cannot type, and sizes the window only while no other client does.
pub fn attach(name: &str, session: &str, read_only: bool) -> Option<Vec<String>> {
    let panes = run(tmux().args([
        "list-panes",
        "-s",
        "-t",
        &format!("={name}:"),
        "-F",
        "#{pane_dead} #{@priory} #{socket_path}",
    ]))
    .ok()?;
    let socket = panes.lines().find_map(|line| {
        let mut fields = line.splitn(3, ' ');
        let (dead, tag, socket) = (fields.next()?, fields.next()?, fields.next()?);
        (dead == "0" && tag == session).then_some(socket)
    })?;
    let read_only: &[&str] = if read_only {
        &["-f", "read-only,ignore-size"]
    } else {
        &[]
    };
    Some(
        ["tmux", "-S", socket, "attach-session"]
            .into_iter()
            .chain(read_only.iter().copied())
            .chain(["-t", &format!("={name}")])
            .map(str::to_owned)
            .collect(),
    )
}

/// How many tmux clients that can type are attached to the session `name`, or none when tmux
/// cannot tell.
pub fn clients(name: &str) -> Option<usize> {
    run(tmux().args([
        "list-clients",
        "-t",
        &format!("={name}"),
        "-F",
        "#{client_readonly}",
    ]))
    .ok()
    .map(|clients| clients.lines().filter(|line| *line == "0").count())
}

/// Kills the tmux session `name`, if it still exists.
pub fn kill(name: &str) {
    let _ = run(tmux().args(["kill-session", "-t", &format!("={name}")]));
}

/// The clone a worktree belongs to: the folder holding its common `.git`.
pub fn clone_of(worktree: &Path) -> Option<PathBuf> {
    let common =
        run(git(worktree).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))
            .ok()?;
    Path::new(common.trim()).parent().map(Path::to_path_buf)
}

/// Pastes `text` into `pane` as one bracketed paste, then presses Enter. tmux reads the text from
/// stdin, so none of it is taken for key names.
pub fn type_text(pane: &str, text: &str) -> Result<(), String> {
    paste(pane, text)?;
    press(pane, "Enter")
}

/// `text` without its control characters, newline and tab kept: an escape sequence would end a
/// bracketed paste, and what follows would reach the terminal as keystrokes.
pub fn typable(text: &str) -> String {
    text.replace(|c: char| c.is_control() && !matches!(c, '\n' | '\t'), "")
}

/// Pastes the `typable` part of `text` into `pane` as one bracketed paste, read from stdin so no
/// part of it is an argument tmux could take for a key name or a command.
pub fn paste(pane: &str, text: &str) -> Result<(), String> {
    let buffer = format!("priory{pane}");
    fed(
        tmux().args(["load-buffer", "-b", &buffer, "-"]),
        Some(typable(text).into_bytes()),
    )?;
    run(tmux().args(["paste-buffer", "-p", "-d", "-b", &buffer, "-t", pane]))?;
    Ok(())
}

/// Presses the key tmux names `key`, such as `Enter` or `C-c`, in `pane`.
pub fn press(pane: &str, key: &str) -> Result<(), String> {
    run(tmux().args(["send-keys", "-t", pane, key]))?;
    Ok(())
}

/// The socket of the daemon of `home`: a short path every client derives the same way, because a
/// Unix socket path over 108 bytes cannot be bound.
pub fn socket_path(home: &Path) -> PathBuf {
    let run = std::env::var_os("XDG_RUNTIME_DIR").filter(|run| !run.is_empty());
    socket_in(run.as_deref().map(Path::new), home)
}

/// `socket_path` with the runtime directory `run` given, or `/tmp/priory-<uid>` when there is none.
pub fn socket_in(run: Option<&Path>, home: &Path) -> PathBuf {
    let home = fs::canonicalize(home)
        .or_else(|_| std::path::absolute(home))
        .unwrap_or_else(|_| home.to_owned());
    // FNV-1a, so a hook and a daemon of different builds agree.
    let hash = home
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    let name = format!("{hash:016x}.sock");
    let under_run = run.map(|run| run.join("priory").join(&name));
    // sun_path holds 108 bytes with its NUL.
    under_run
        .filter(|path| path.as_os_str().len() < 108)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/priory-{}", uid())).join(name))
}

pub(crate) fn uid() -> u32 {
    if let Ok(own) = fs::metadata("/proc/self") {
        return own.uid();
    }
    let id = run(Command::new("id").arg("-u")).unwrap_or_default();
    id.trim().parse().unwrap_or(u32::MAX)
}

/// Why the socket of `home` is not under the runtime directory `run`, if it is not.
fn fallback_reason(run: Option<&Path>, home: &Path) -> Option<String> {
    match run {
        None => Some("XDG_RUNTIME_DIR is not set".to_owned()),
        Some(run) if !socket_in(Some(run), home).starts_with(run) => Some(format!(
            "the socket path under XDG_RUNTIME_DIR {} is over the 108-byte limit of a Unix socket",
            run.display()
        )),
        Some(_) => None,
    }
}

/// Makes `dir` for the socket, readable by its owner only, and tightens one of the user `owner`
/// that others can enter. It refuses one that is not a folder or belongs to another user; `why`
/// says why the socket is there.
pub(crate) fn private_dir(dir: &Path, owner: u32, why: Option<&str>) -> Result<(), String> {
    let at = |error: std::io::Error| format!("{}: {error}", dir.display());
    let private = fs::Permissions::from_mode(0o700);
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        // The mode a folder gets on create can be widened by a default ACL on its parent.
        Ok(()) => return fs::set_permissions(dir, private).map_err(at),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(at(error)),
    }
    let found = fs::symlink_metadata(dir).map_err(at)?;
    if found.is_dir() && found.uid() == owner {
        if found.mode() & 0o077 != 0 {
            fs::set_permissions(dir, private).map_err(at)?;
        }
        return Ok(());
    }
    let todo = match why {
        Some(why) => format!(
            "the socket falls back here because {why}. Remove the folder, or set XDG_RUNTIME_DIR \
             to a short folder of your own"
        ),
        None => "remove the folder so the daemon can make its own".to_owned(),
    };
    Err(format!(
        "{}: not a private folder of this user (owner {}, mode {:o}); {todo}",
        dir.display(),
        found.uid(),
        found.mode() & 0o777
    ))
}

fn spool(home: &Path) -> PathBuf {
    home.join("signals.spool")
}

/// One line from the socket, with the sender that takes its reply once it is applied.
pub struct Line {
    pub text: String,
    pub applied: Sender<String>,
}

/// The daemon's end of the socket: its lock, and the lines that arrive.
pub struct Listening {
    _lock: File,
    pub lines: Receiver<Line>,
}

/// Takes the daemon lock under `home` and binds the signal socket. Each connection's line gets
/// the daemon's reply once the daemon applied it.
pub fn listen(home: &Path) -> Result<Listening, String> {
    let at = |error: std::io::Error| format!("{}: {error}", home.display());
    fs::create_dir_all(home).map_err(at)?;
    let lock = File::create(home.join("daemon.lock")).map_err(at)?;
    // A killed daemon's lock lingers while a child it forked has not yet exec'd.
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock.try_lock().is_err() {
        if Instant::now() >= deadline {
            return Err(format!("{}: another daemon holds its lock", home.display()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let path = socket_path(home);
    let run = std::env::var_os("XDG_RUNTIME_DIR").filter(|run| !run.is_empty());
    let why = fallback_reason(run.as_deref().map(Path::new), home);
    private_dir(
        path.parent().unwrap_or(Path::new("/")),
        uid(),
        why.as_deref(),
    )?;
    // bind creates the file before listen: renamed after, it never refuses a client that saw it.
    let bound = path.with_extension("new");
    let _ = fs::remove_file(&bound);
    let listener = UnixListener::bind(&bound).map_err(at)?;
    fs::rename(&bound, &path).map_err(at)?;
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            // A thread each, so a line the daemon answers late holds no other connection.
            let sender = sender.clone();
            std::thread::spawn(move || {
                let Some(text) = line_within(&stream, Duration::from_secs(2)) else {
                    return;
                };
                let (applied, done) = mpsc::channel();
                if sender.send(Line { text, applied }).is_err() {
                    return;
                }
                if let Ok(reply) = done.recv() {
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                    let _ = stream.write_all(format!("{reply}\n").as_bytes());
                }
            });
        }
    });
    Ok(Listening { _lock: lock, lines })
}

/// The first line `stream` sends, newline included, if it ends or the stream does within `limit`.
fn line_within(mut stream: &UnixStream, limit: Duration) -> Option<String> {
    let deadline = Instant::now() + limit;
    let mut line = Vec::new();
    let mut chunk = [0; 4096];
    while !line.contains(&b'\n') {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        stream.set_read_timeout(Some(left)).ok()?;
        match stream.read(&mut chunk).ok()? {
            0 => break,
            read => line.extend_from_slice(&chunk[..read]),
        }
    }
    let end = line.iter().position(|&byte| byte == b'\n');
    line.truncate(end.map_or(line.len(), |end| end + 1));
    String::from_utf8(line).ok()
}

/// The signals spooled while no daemon listened. `apply` runs on each before the spool is removed.
pub fn replay(
    home: &Path,
    mut apply: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    drain(home, &mut apply)?;
    // Each jailed run's own spool, by its session.
    let mut runs: Vec<PathBuf> = fs::read_dir(home.join("spools"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    runs.sort();
    for dir in runs {
        drain(&dir, &mut apply)?;
    }
    Ok(())
}

/// Applies and removes the spool in `dir`.
fn drain(dir: &Path, apply: &mut impl FnMut(&str) -> Result<(), String>) -> Result<(), String> {
    let taken = dir.join("signals.spool.taken");
    let at = |error: std::io::Error| format!("{}: {error}", taken.display());
    loop {
        // shortcut: a hook that opened the spool before this rename and writes after the read is lost; move the spool into SQLite if that shows up.
        if !taken.exists() && fs::rename(spool(dir), &taken).is_err() {
            return Ok(());
        }
        for line in fs::read_to_string(&taken).map_err(at)?.lines() {
            apply(line)?;
        }
        fs::remove_file(&taken).map_err(at)?;
    }
}

/// How long a verb waits for the daemon's reply to its line.
pub const REPLY: Duration = Duration::from_secs(10);

/// How `call`'s error starts when no daemon took the line, so it was never sent.
pub const NO_DAEMON: &str = "no priory daemon listens on";

/// How long `call_through_restart` tries again while the daemon's socket refuses it.
pub const RESTART: Duration = Duration::from_secs(30);

/// Sends one line to the daemon and returns its reply within about `wait`. Nothing is spooled:
/// with no daemon, it fails at once.
pub fn call(home: &Path, line: &str, wait: Duration) -> Result<String, String> {
    call_at(&socket_path(home), line, wait, Duration::ZERO)
}

/// `call` for an MCP server, which a run keeps across a daemon restart: while the socket refuses
/// it, as a restarting daemon's does, it tries again for up to `RESTART`, then waits `wait`.
pub fn call_through_restart(home: &Path, line: &str, wait: Duration) -> Result<String, String> {
    call_at(&socket_path(home), line, wait, RESTART)
}

/// Sends `line` on the socket at `path`, trying a refused connection again for up to `retry`. Once
/// connected, the reply has the whole of `wait`, so a late connect never cuts a call short.
fn call_at(path: &Path, line: &str, wait: Duration, retry: Duration) -> Result<String, String> {
    let deadline = Instant::now() + retry;
    let mut stream = loop {
        match UnixStream::connect(path) {
            Ok(stream) => break stream,
            Err(error)
                if error.kind() == std::io::ErrorKind::ConnectionRefused
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(error) => return Err(format!("{NO_DAEMON} {}: {error}", path.display())),
        }
    };
    let wait = Some(wait);
    let mut reply = String::new();
    stream
        .set_write_timeout(wait)
        .and_then(|()| stream.set_read_timeout(wait))
        .and_then(|()| stream.write_all(format!("{line}\n").as_bytes()))
        .and_then(|()| BufReader::new(&stream).read_line(&mut reply))
        .map_err(|error| format!("the daemon did not reply: {error}"))?;
    if reply.is_empty() {
        return Err("the daemon closed the connection without a reply".to_owned());
    }
    Ok(reply.trim_end().to_owned())
}

/// Sends one line to the daemon and waits up to 10 seconds for it to be applied. Only when no
/// daemon takes the line is it appended to the spool, so a slow daemon never gets it twice.
pub fn send(home: &Path, line: &str) -> Result<(), String> {
    let line = format!("{line}\n");
    let wait = Some(Duration::from_secs(10));
    let sent = UnixStream::connect(socket_path(home)).and_then(|mut stream| {
        stream.set_write_timeout(wait)?;
        stream.set_read_timeout(wait)?;
        stream.write_all(line.as_bytes())?;
        Ok(stream)
    });
    if let Ok(stream) = sent {
        let mut answer = String::new();
        // A daemon that closes or resets without replying died holding the line: spool it.
        match BufReader::new(stream).read_line(&mut answer) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
            Ok(_) if answer == "ok\n" => return Ok(()),
            Ok(_) | Err(_) => {
                eprintln!("priory hook: the daemon took the signal but has not applied it yet");
                return Ok(());
            }
        }
    }
    // A jailed run sees only its own spool, which the daemon names.
    let spool = std::env::var_os("PRIORY_SPOOL")
        .filter(|spool| !spool.is_empty())
        .map_or_else(|| spool(home), PathBuf::from);
    fs::create_dir_all(spool.parent().unwrap_or(home)).map_err(|error| error.to_string())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spool)
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|error| format!("{}: {error}", spool.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_command_that_answers_more_than_a_pipe_holds_while_it_reads_still_ends() {
        let input = vec![b'x'; 4 << 20];

        let output = fed_within(
            &mut Command::new("cat"),
            Some(input.clone()),
            Duration::from_secs(20),
        )
        .unwrap()
        .expect("cat ended in time");

        assert!(output.status.success());
        assert!(output.stdout == input);
    }

    /// A socket file that refuses every connection, as a stopped daemon's does, in a fresh folder.
    fn refusing(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("priory-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("d.sock");
        drop(UnixListener::bind(&path).unwrap());
        path
    }

    /// Runs `DIRENV_EXEC` for session `s` under a stub `direnv` whose body is `direnv`, around a
    /// harness that logs its start and exits 1. Returns the exit code, the starts and Priory home.
    fn direnv_exec(name: &str, direnv: &str) -> (Option<i32>, usize, PathBuf) {
        let dir = std::env::temp_dir().join(format!("priory-direnv-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        let stub = dir.join("bin/direnv");
        fs::write(&stub, format!("#!/bin/sh\n{direnv}\n")).unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        let spool = direnv_spool(&dir, "s");
        fs::create_dir_all(&spool).unwrap();
        let path = std::env::join_paths(
            std::iter::once(dir.join("bin"))
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let log = dir.join("starts");
        let status = Command::new("sh")
            .args(["-c", DIRENV_EXEC])
            .arg(&spool)
            .arg(&dir)
            .args(["sh", "-c", r#"echo started >> "$0"; exit 1"#])
            .arg(&log)
            .env("PATH", path)
            .stderr(Stdio::null())
            .status()
            .unwrap();
        let starts = fs::read_to_string(&log).unwrap_or_default().lines().count();
        (status.code(), starts, dir)
    }

    #[test]
    fn a_failing_direnv_starts_the_harness_once_without_it_and_keeps_its_error() {
        let (code, starts, home) =
            direnv_exec("failed", "echo 'direnv: error exit status 1' >&2; exit 1");

        assert_eq!((code, starts), (Some(1), 1));
        assert_eq!(
            direnv_failed(&home, "s").as_deref(),
            Some("direnv: error exit status 1\n")
        );
        assert!(!direnv_loading(&home, "s"));
    }

    #[test]
    fn direnvs_error_is_read_from_a_regular_file_alone_and_up_to_a_cap() {
        let home = std::env::temp_dir().join(format!("priory-error-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let spool = direnv_spool(&home, "s");
        fs::create_dir_all(&spool).unwrap();
        fs::write(spool.join("direnv.failed"), "").unwrap();
        let error = spool.join("direnv.error");
        let secret = home.join("secret");
        fs::write(&secret, "secret").unwrap();

        std::os::unix::fs::symlink(&secret, &error).unwrap();
        assert_eq!(direnv_failed(&home, "s").as_deref(), Some(""));
        fs::remove_file(&error).unwrap();

        assert!(
            Command::new("mkfifo")
                .arg(&error)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(direnv_failed(&home, "s").as_deref(), Some(""));
        fs::remove_file(&error).unwrap();

        fs::write(&error, vec![b'x'; 1 << 20]).unwrap();
        assert_eq!(direnv_failed(&home, "s").unwrap().len(), 64 << 10);
    }

    proptest! {
        #[test]
        fn a_remotes_refs_name_a_full_commit_or_none(
            refs in ".{0,200}", commit in "[0-9a-f]{0,64}", name in "HEAD|refs/heads/main",
        ) {
            prop_assert!(head_of(&refs).is_none_or(|commit| commit.len() >= 40));
            let line = format!("{commit}\t{name}\n");
            let full = matches!(commit.len(), 40 | 64) && name == "HEAD";
            prop_assert_eq!(head_of(&line), full.then_some(commit.as_str()));
        }
    }

    #[test]
    fn a_remotes_head_is_found_among_its_refs() {
        let commit = "0123456789".repeat(4);
        let refs = format!("{commit}\tHEAD\nffff\trefs/remotes/origin/HEAD\n");
        assert_eq!(head_of(&refs), Some(commit.as_str()));
    }

    #[test]
    fn a_harness_that_exits_1_under_direnv_starts_once() {
        let (code, starts, home) = direnv_exec("exits", r#"shift 2; exec "$@""#);

        assert_eq!((code, starts), (Some(1), 1));
        assert_eq!(direnv_failed(&home, "s"), None);
        assert!(!direnv_loading(&home, "s"));
    }

    #[test]
    fn a_run_whose_direnv_has_not_started_the_harness_is_loading() {
        let home =
            std::env::temp_dir().join(format!("priory-direnv-loading-{}", std::process::id()));
        let spool = direnv_spool(&home, "s");
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&spool).unwrap();
        fs::write(spool.join("direnv.error"), "direnv: loading .envrc\n").unwrap();

        assert!(direnv_loading(&home, "s"));
        assert_eq!(direnv_failed(&home, "s"), None);
    }

    #[test]
    fn a_call_through_a_restart_has_its_whole_wait_after_a_late_connect() {
        let path = refusing("late");
        let daemon = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1500));
                // Bound beside, then renamed over, as `listen` does, so every connect finds the file.
                let bound = path.with_extension("new");
                let listener = UnixListener::bind(&bound).unwrap();
                fs::rename(&bound, &path).unwrap();
                let (mut stream, _) = listener.accept().unwrap();
                let line = line_within(&stream, Duration::from_secs(5));
                // Longer than what a 2 second budget would leave after the late connect.
                std::thread::sleep(Duration::from_millis(1500));
                stream.write_all(b"done\n").unwrap();
                line
            })
        };

        let reply = call_at(&path, "hello", Duration::from_secs(2), RESTART);

        assert_eq!(reply, Ok("done".to_owned()));
        assert_eq!(daemon.join().unwrap().as_deref(), Some("hello\n"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_call_without_a_retry_fails_at_once_on_a_refusing_socket() {
        let path = refusing("refused");
        let started = Instant::now();

        let reply = call_at(&path, "hello", Duration::from_secs(2), Duration::ZERO);

        assert!(
            reply
                .as_ref()
                .is_err_and(|error| error.starts_with(NO_DAEMON)),
            "{reply:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn claudes_trust_dialog_is_seen_in_each_version_tested_and_nothing_else_is() {
        let dialog = |options: &str| {
            format!(
                " Accessing workspace:\n /home/a/worktrees/o/r/1\n Quick safety check: Is this a project you created or one you trust?\n{options}\n Enter to confirm · Esc to cancel"
            )
        };
        // 2.1.42 and 2.1.198 preselect yes, 2.1.330 no.
        assert!(ClaudeCode.asks_trust(&dialog(" ❯ 1. Yes, I trust this folder\n   2. No, exit")));
        assert!(ClaudeCode.asks_trust(&dialog(" ❯ No, exit\n   Yes, I trust this folder")));
        assert!(!ClaudeCode.asks_trust("  Trust this folder?"));
        assert!(!ClaudeCode.asks_trust(" Do you want to use this API key?\n ❯ No (recommended)"));
        assert!(!Codex.asks_trust(&dialog(" ❯ No, exit")));
    }

    /// The launch of an investigator, or of a worker when `tools` says so, in a throwaway home.
    fn launch_of<'a>(home: &'a Path, servers: &'a Servers, tools: crate::mcp::Tools) -> Launch<'a> {
        Launch {
            tmux: "priory-7",
            dir: Path::new("/w/7"),
            home,
            session: "s-7",
            resume: None,
            model: Some("claude-haiku-5-5"),
            effort: None,
            body: "You read Sentry.",
            instructions: "",
            plugin: None,
            memories: None,
            prompt: Some("One question."),
            permissions: json!({ "defaultMode": "dontAsk", "allow": ["Read"] }),
            permission_mode: "dontAsk",
            scope: &[],
            env: &[],
            debate: false,
            gives_up: false,
            servers,
            tools,
            jail: None,
            trust: false,
            direnv: None,
        }
    }

    #[test]
    fn an_investigators_claude_loads_no_user_or_project_settings_and_has_no_tool_that_writes() {
        let home = Path::new("/h");
        let exe = Path::new("/bin/priory");
        let servers = Servers::default();
        let launch = launch_of(home, &servers, crate::mcp::Tools::Investigator);

        let args = ClaudeCode.args(exe, &launch).unwrap();

        assert_eq!(
            args,
            [
                "--session-id",
                "s-7",
                "--model",
                "claude-haiku-5-5",
                "--append-system-prompt",
                "You read Sentry.",
                "--permission-mode",
                "dontAsk",
                "--setting-sources",
                "",
                "--disallowedTools",
                "Edit,MultiEdit,Write,NotebookEdit,Bash,Agent,Task",
                "--settings",
                &settings(exe, &launch.permissions),
                "--strict-mcp-config",
                "--mcp-config",
                "/h/prompts/s-7.mcp.json",
            ]
        );

        let worker = launch_of(home, &servers, crate::mcp::Tools::Worker);
        let args = ClaudeCode.args(exe, &worker).unwrap();
        assert!(!args.contains(&"--setting-sources".to_owned()), "{args:?}");
        assert!(!args.contains(&"--disallowedTools".to_owned()), "{args:?}");
    }

    #[test]
    fn remove_sealed_removes_a_home_holding_a_sealed_snapshot() {
        let home = std::env::temp_dir().join(format!("priory-sealed-{}", std::process::id()));
        remove_sealed(&home);
        let files = [(PathBuf::from("skills/a/SKILL.md"), b"text".as_slice())];
        let plugin = write_once(&home.join("kit/abc"), "plugin", &files).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&plugin.join("skills/a")), 0o555);
        assert_eq!(mode(&plugin.join("skills/a/SKILL.md")), 0o444);

        remove_sealed(&home);

        assert!(
            fs::symlink_metadata(&home).is_err(),
            "{} is left",
            home.display()
        );
    }

    #[test]
    fn the_tagged_pane_is_the_live_one_with_the_session_and_its_cursor_shows_only_when_on() {
        let panes = "%1 0 80 24 1 3 4 0 other\n%2 1 80 24 1 3 4 0 s1\n%3 0 120 30 0 9 9 0 s1\n%4 0 120 30 1 5 6 0 s1";
        let found = |panes, session| {
            tagged_pane(panes, session)
                .map(|(pane, shown)| (pane, shown.width, shown.height, shown.cursor))
        };

        assert_eq!(found(panes, "s1"), Some(("%3", 120, 30, None)));
        assert_eq!(
            found("%4 0 120 30 1 5 6 7 s1", "s1"),
            Some(("%4", 120, 30, Some((5, 6))))
        );
        assert_eq!(
            tagged_pane("%4 0 120 30 1 5 6 1500 s1", "s1").map(|(_, shown)| shown.scrollback),
            Some(1500)
        );
        assert_eq!(found("%4 0 120 30 2 5 6 0 s1", "s1"), None);
        assert_eq!(found("%4 0 120 30 1 5 6 many s1", "s1"), None);
        assert_eq!(found(panes, "s9"), None);
    }

    proptest! {
        #[test]
        fn a_pane_listing_parses_or_is_refused_without_panicking(
            listing in "(%[0-9]{1,3} [01] [0-9]{1,6} [0-9]{1,6} [0-9] [0-9]{1,6} [0-9]{1,6} [0-9]{1,6} s1\n){0,3}|.{0,80}"
        ) {
            if let Some((pane, ..)) = tagged_pane(&listing, "s1") {
                prop_assert!(listing.contains(pane));
            }
        }
    }

    #[test]
    fn a_runtime_directory_too_long_for_a_socket_path_falls_back_to_tmp() {
        let home = std::env::temp_dir();
        let long = PathBuf::from("/run/").join("u".repeat(90));

        let socket = socket_in(Some(&long), &home);

        assert!(
            socket.starts_with(format!("/tmp/priory-{}", uid())),
            "{socket:?}"
        );
        assert!(
            socket_in(Some(Path::new("/run/user/1000")), &home)
                .starts_with("/run/user/1000/priory")
        );
    }

    #[test]
    fn a_socket_directory_of_another_user_is_refused() {
        let dir = std::env::temp_dir().join(format!("priory-private-{}", std::process::id()));

        assert_eq!(private_dir(&dir, uid(), None), Ok(()));
        let refused = private_dir(&dir, uid() + 1, Some("XDG_RUNTIME_DIR is not set")).unwrap_err();
        fs::remove_dir(&dir).unwrap();

        assert!(
            refused.contains("not a private folder of this user"),
            "{refused}"
        );
        assert!(
            refused.contains("falls back here because XDG_RUNTIME_DIR is not set"),
            "{refused}"
        );
    }

    #[test]
    fn a_socket_directory_of_this_user_that_others_can_enter_is_tightened() {
        let dir = std::env::temp_dir().join(format!("priory-loose-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o775)).unwrap();

        let made = private_dir(&dir, uid(), None);
        let mode = fs::metadata(&dir).unwrap().mode() & 0o777;
        fs::remove_dir(&dir).unwrap();

        assert_eq!(made, Ok(()));
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn a_tmux_error_becomes_one_line() {
        let stderr = "error connecting to /tmp/x/default (File name too long)\n\n  more\n";

        assert_eq!(
            one_line(stderr),
            "error connecting to /tmp/x/default (File name too long); more"
        );
        assert!(tmux_socket_fits(Path::new("/tmp/tmux-1000/default")).is_ok());
        let long = PathBuf::from("/tmp").join("t".repeat(100)).join("default");
        assert!(
            tmux_socket_fits(&long)
                .unwrap_err()
                .starts_with("the tmux socket path ")
        );
    }

    #[test]
    fn the_fallback_reason_names_a_runtime_directory_over_the_socket_limit() {
        let home = std::env::temp_dir();
        let long = PathBuf::from("/run/").join("u".repeat(90));

        let why = fallback_reason(Some(&long), &home).unwrap();

        assert!(why.contains("over the 108-byte limit"), "{why}");
        assert!(why.contains(&*long.to_string_lossy()), "{why}");
        assert_eq!(
            fallback_reason(Some(Path::new("/run/user/1000")), &home),
            None
        );
    }

    /// A Priory home with a clone of `o/n` from a local bare origin, and task 1's worktree on
    /// `priory/1`, made by `worktree`.
    struct Home {
        dir: PathBuf,
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// Git in `dir` with a fixed identity, failing the test when it fails.
    fn sh(dir: &Path, args: &[&str]) -> String {
        run(git(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args))
        .unwrap()
        .trim()
        .to_owned()
    }

    fn commit(dir: &Path, file: &str, text: &str) -> String {
        fs::write(dir.join(file), text).unwrap();
        sh(dir, &["add", file]);
        sh(dir, &["commit", "--quiet", "-m", file]);
        sh(dir, &["rev-parse", "HEAD"])
    }

    impl Home {
        fn new(name: &str) -> Home {
            let dir =
                std::env::temp_dir().join(format!("priory-recover-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            let origin = dir.join("origin.git");
            let seed = dir.join("seed");
            fs::create_dir_all(&seed).unwrap();
            sh(
                &dir,
                &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
            );
            sh(&seed, &["init", "--quiet", "-b", "main"]);
            commit(&seed, "README.md", "one\n");
            sh(
                &seed,
                &["push", "--quiet", origin.to_str().unwrap(), "main"],
            );
            let clone = dir.join("home/repos/o/n");
            fs::create_dir_all(clone.parent().unwrap()).unwrap();
            sh(
                &dir,
                &[
                    "clone",
                    "--quiet",
                    "--no-checkout",
                    origin.to_str().unwrap(),
                    clone.to_str().unwrap(),
                ],
            );
            let home = Home { dir };
            // By hand: `worktree` would read GitHub's `o/n` for a new one.
            let tree = home.tree();
            let start = ["worktree", "add", "--quiet", "--no-track", "-b", "priory/1"];
            let path = [tree.to_str().unwrap(), "origin/HEAD"];
            sh(&clone, &[&start[..], &path[..]].concat());
            home
        }

        fn home(&self) -> PathBuf {
            self.dir.join("home")
        }

        fn tree(&self) -> PathBuf {
            self.home().join("worktrees/o/n/1")
        }

        fn again(&self) -> Result<Added, String> {
            worktree(&self.home(), "o/n", "1", None, true, false).map(|(_, _, added)| added)
        }
    }

    #[test]
    fn a_clean_detached_worktree_whose_commit_is_pushed_goes_back_on_its_branch() {
        let home = Home::new("pushed");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        let head = commit(&tree, "a.txt", "a\n");
        sh(
            &tree,
            &["push", "--quiet", "origin", "HEAD:refs/heads/feature"],
        );

        let added = home.again().unwrap();

        let Added::Kept(Some(recovery)) = added else {
            panic!("{added:?}")
        };
        assert_eq!(
            recovery,
            Recovery {
                head,
                from_branch: None,
                branch: "priory/1".to_owned(),
                kept_branch: None,
                kept_commits: 0,
            }
        );
        assert_eq!(sh(&tree, &["branch", "--show-current"]), "priory/1");
        assert_eq!(home.again(), Ok(Added::Kept(None)));
    }

    #[test]
    fn a_clean_detached_worktree_whose_commit_was_pushed_rebased_goes_back_on_its_branch() {
        let home = Home::new("rebased");
        let tree = home.tree();
        sh(&tree, &["switch", "--quiet", "-c", "other"]);
        commit(&tree, "b.txt", "b\n");
        sh(&tree, &["push", "--quiet", "origin", "other"]);
        sh(&tree, &["checkout", "--quiet", "--detach", "priory/1"]);
        sh(&tree, &["branch", "--quiet", "-D", "other"]);
        fs::write(tree.join("a.txt"), "a\n").unwrap();
        sh(&tree, &["add", "a.txt"]);
        run(git(&tree)
            .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(["commit", "--quiet", "-m", "a"]))
        .unwrap();
        let head = sh(&tree, &["rev-parse", "HEAD"]);
        // The pushed copy sits on another base, so only its patch matches the detached commit.
        sh(&tree, &["checkout", "--quiet", "--detach", "origin/other"]);
        sh(&tree, &["cherry-pick", &head]);
        sh(
            &tree,
            &["push", "--quiet", "origin", "HEAD:refs/heads/other"],
        );
        sh(&tree, &["checkout", "--quiet", "--detach", &head]);

        let added = home.again().unwrap();

        let Added::Kept(Some(recovery)) = added else {
            panic!("{added:?}")
        };
        assert_eq!(recovery.head, head);
        assert_eq!(recovery.kept_branch, None);
        assert_eq!(recovery.kept_commits, 0);
        assert_eq!(sh(&tree, &["branch", "--show-current"]), "priory/1");
    }

    #[test]
    fn a_commit_amended_only_in_whitespace_after_its_push_is_kept() {
        let home = Home::new("whitespace");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        commit(&tree, "a.txt", "a\n");
        sh(
            &tree,
            &["push", "--quiet", "origin", "HEAD:refs/heads/feature"],
        );
        fs::write(tree.join("a.txt"), "    a\n").unwrap();
        sh(
            &tree,
            &["commit", "--quiet", "--all", "--amend", "--no-edit"],
        );
        let head = sh(&tree, &["rev-parse", "HEAD"]);
        let short = sh(&tree, &["rev-parse", "--short", "HEAD"]);

        let added = home.again().unwrap();

        let kept = format!("priory/1-kept-{short}");
        let Added::Kept(Some(recovery)) = added else {
            panic!("{added:?}")
        };
        assert_eq!(recovery.kept_branch.as_deref(), Some(kept.as_str()));
        assert_eq!(recovery.kept_commits, 1);
        assert_eq!(sh(&tree, &["rev-parse", &kept]), head);
    }

    #[test]
    fn patches_over_the_byte_cap_count_every_commit_at_risk() {
        let home = Home::new("cap");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        let head = commit(&tree, "a.txt", "a\n");
        sh(&tree, &["checkout", "--quiet", "--detach", "origin/main"]);
        sh(&tree, &["cherry-pick", "-x", &head]);
        sh(
            &tree,
            &["push", "--quiet", "origin", "HEAD:refs/heads/copy"],
        );
        sh(&tree, &["checkout", "--quiet", "--detach", &head]);

        assert_eq!(at_risk(&tree, Some("priory/1"), SCAN, PATCHES), Ok(0));
        assert_eq!(at_risk(&tree, Some("priory/1"), SCAN, 10), Ok(1));
    }

    #[test]
    fn a_capped_command_ends_at_its_deadline_and_keeps_no_more_than_its_cap() {
        let soon = Instant::now() + Duration::from_millis(200);
        let stuck = capped(Command::new("sleep").arg("60"), soon, 10).unwrap_err();
        assert!(timed_out(&stuck), "{stuck}");

        let later = Instant::now() + Duration::from_secs(20);
        assert_eq!(capped(&mut Command::new("yes"), later, 1000), Ok(None));
        let echo = capped(Command::new("printf").arg("ab"), later, 2);
        assert_eq!(echo, Ok(Some(b"ab".to_vec())));
    }

    /// Puts a fifo in place of git's config `file`, on which git blocks until this drops.
    struct Hung {
        file: PathBuf,
        plain: PathBuf,
        /// Its close ends the read of each git that waits.
        _fifo: fs::File,
    }

    impl Hung {
        fn at(file: PathBuf) -> Self {
            let plain = file.with_file_name("config-plain");
            fs::rename(&file, &plain).unwrap();
            run(Command::new("mkfifo").arg(&file)).unwrap();
            let fifo = fs::File::options()
                .read(true)
                .write(true)
                .open(&file)
                .unwrap();
            Hung {
                file,
                plain,
                _fifo: fifo,
            }
        }
    }

    impl Drop for Hung {
        fn drop(&mut self) {
            let _ = fs::rename(&self.plain, &self.file);
        }
    }

    #[test]
    fn a_hung_prune_setting_ends_within_the_loops_timeout() {
        let home = Home::new("prune");
        home.tree();
        let clone = home.home().join("repos/o/n");
        let hung = Hung::at(clone.join(".git/config"));
        let (sender, ended) = mpsc::channel();

        let setting = std::thread::spawn(move || {
            never_prune(&clone);
            let _ = sender.send(());
        });

        let ended = ended.recv_timeout(Duration::from_secs(45));
        drop(hung);
        setting.join().unwrap();
        assert_eq!(ended, Ok(()));
    }

    #[test]
    fn a_clone_without_cloud_gets_back_the_prune_setting_it_had() {
        for had in [None, Some("true"), Some("false")] {
            let home = Home::new("prune-back");
            home.tree();
            let clone = home.home().join("repos/o/n");
            if let Some(had) = had {
                sh(&clone, &["config", "remote.origin.prune", had]);
            }
            let set = |cloud| {
                worktree(&home.home(), "o/n", "1", None, false, cloud).unwrap();
                run(git(&clone).args(["config", "--local", "--get", "remote.origin.prune"])).ok()
            };

            assert_eq!(set(true).as_deref(), Some("false\n"));
            assert_eq!(set(true).as_deref(), Some("false\n"));
            assert_eq!(set(false), had.map(|had| format!("{had}\n")));

            // What a person set since is theirs.
            set(true);
            sh(&clone, &["config", "remote.origin.prune", "true"]);
            assert_eq!(set(false).as_deref(), Some("true\n"));
        }
    }

    #[test]
    fn a_hung_ancestor_check_is_an_error_and_not_a_no() {
        let home = Home::new("ancestor");
        let tree = home.tree();
        let head = sh(&tree, &["rev-parse", "HEAD"]);
        assert_eq!(is_ancestor(&tree, &head, "HEAD"), Ok(true));
        assert_eq!(is_ancestor(&tree, "HEAD", "gone"), Ok(false));
        // Git blocks on a fifo in place of its config.
        let config = home.home().join("repos/o/n/.git/config");
        fs::remove_file(&config).unwrap();
        run(Command::new("mkfifo").arg(&config)).unwrap();

        let started = Instant::now();
        let hung = is_ancestor(&tree, &head, "HEAD").unwrap_err();

        assert!(timed_out(&hung), "{hung}");
        assert!(started.elapsed() < Duration::from_secs(45));
    }

    #[test]
    fn a_fetch_that_timed_out_waits_longer_each_time_and_a_good_one_clears_it() {
        let home = Path::new("/nowhere");
        let wait = || slow().get("o/slow").map(|(_, wait)| wait.as_secs());
        assert_eq!(fetch_waits(home, "o/slow", "1"), None);

        let reason = put_off("o/slow");
        assert_eq!(reason, "waits to fetch o/slow: timed out after 30s");
        assert_eq!(fetch_waits(home, "o/slow", "1"), Some(reason));
        assert_eq!(wait(), Some(60));
        put_off("o/slow");
        assert_eq!(wait(), Some(120));
        for _ in 0..5 {
            put_off("o/slow");
        }
        assert_eq!(wait(), Some(600));

        fetched("o/slow");
        assert_eq!(fetch_waits(home, "o/slow", "1"), None);
    }

    #[test]
    fn a_patch_scan_past_its_limit_counts_every_commit_at_risk() {
        let home = Home::new("scan");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        let head = commit(&tree, "a.txt", "a\n");
        sh(&tree, &["checkout", "--quiet", "--detach", "origin/main"]);
        // `-x` changes the message, so the copy is another commit even within the same second.
        sh(&tree, &["cherry-pick", "-x", &head]);
        sh(
            &tree,
            &["push", "--quiet", "origin", "HEAD:refs/heads/copy"],
        );
        sh(&tree, &["checkout", "--quiet", "--detach", &head]);

        assert_eq!(at_risk(&tree, Some("priory/1"), 1, PATCHES), Ok(0));
        assert_eq!(at_risk(&tree, Some("priory/1"), 0, PATCHES), Ok(1));
    }

    #[test]
    fn work_that_origin_lacks_is_unpushed_wherever_it_is() {
        let home = Home::new("unpushed");
        let tree = home.tree();
        let of = |branches: &[&str]| {
            let branches: Vec<String> = branches.iter().map(|name| (*name).to_owned()).collect();
            unpushed(&home.dir, &tree, &branches)
        };
        assert_eq!(of(&["priory/1"]), Ok(vec![]), "no commits");

        fs::write(tree.join("new.txt"), "n\n").unwrap();
        assert!(of(&["priory/1"]).unwrap_err().contains("1 untracked file"));
        sh(&tree, &["add", "new.txt"]);
        assert!(of(&["priory/1"]).unwrap_err().contains("1 staged file"));
        commit(&tree, "new.txt", "n\n");
        assert!(
            of(&["priory/1"])
                .unwrap_err()
                .contains("has commits origin lacks")
        );
        sh(&tree, &["push", "--quiet", "origin", "priory/1"]);
        assert_eq!(of(&["priory/1"]), Ok(vec!["priory/1".to_owned()]));

        sh(&tree, &["branch", "--quiet", "priory/10-api"]);
        sh(&tree, &["checkout", "--quiet", "-b", "priory/1-api"]);
        assert_eq!(
            stacked(&home.dir, &tree, "priory/1"),
            Ok(vec!["priory/1-api".to_owned()])
        );
        commit(&tree, "api.txt", "a\n");
        sh(&tree, &["checkout", "--quiet", "priory/1"]);
        assert_eq!(
            of(&["priory/1", "priory/1-api"])
                .unwrap_err()
                .split(':')
                .next(),
            Some("branch priory/1-api has commits origin lacks")
        );
        sh(&tree, &["push", "--quiet", "origin", "priory/1-api"]);
        assert_eq!(of(&["priory/1", "priory/1-api"]).unwrap().len(), 2);

        sh(&tree, &["checkout", "--quiet", "--detach"]);
        commit(&tree, "lost.txt", "l\n");
        assert!(
            of(&["priory/1"])
                .unwrap_err()
                .starts_with("the checkout has commits")
        );
        sh(&tree, &["checkout", "--quiet", "--detach", "priory/1"]);
        assert!(of(&["priory/1"]).is_ok());

        sh(&tree, &["branch", "--quiet", "-D", "priory/1-api"]);
        assert!(of(&["priory/1-api"]).is_ok(), "origin still has it");
        assert!(of(&["priory/gone"]).unwrap_err().contains("is gone"));
    }

    #[test]
    fn a_leftover_sequencer_folder_blocks_as_an_operation_in_progress() {
        let home = Home::new("sequencer");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        let folder = tree.join(sh(&tree, &["rev-parse", "--git-path", "sequencer"]));
        fs::create_dir_all(&folder).unwrap();

        let reason = home.again().unwrap_err();

        let dir = quote(&tree);
        assert!(
            reason.contains(&format!(
                "has a cherry-pick or revert sequence in progress at {}, not on priory/1. Finish \
                 it with git -C {dir} cherry-pick --continue, or drop it with git -C {dir} \
                 cherry-pick --quit",
                sh(&tree, &["rev-parse", "--short", "HEAD"])
            )),
            "{reason}"
        );
    }

    #[test]
    fn a_detached_head_ahead_of_its_branch_with_a_cherry_pick_in_progress_blocks() {
        let home = Home::new("picking");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        commit(&tree, "a.txt", "a\n");
        let marker = tree.join(sh(&tree, &["rev-parse", "--git-path", "CHERRY_PICK_HEAD"]));
        fs::write(&marker, sh(&tree, &["rev-parse", "HEAD"])).unwrap();

        let reason = home.again().unwrap_err();

        assert!(reason.contains("has a cherry-pick in progress"), "{reason}");
        assert_eq!(sh(&tree, &["branch", "--show-current"]), "");
    }

    #[test]
    fn a_clean_worktree_on_another_branch_goes_back_and_that_branch_keeps_its_commits() {
        let home = Home::new("other");
        let tree = home.tree();
        sh(&tree, &["switch", "--quiet", "-c", "side"]);
        let head = commit(&tree, "a.txt", "a\n");

        let added = home.again().unwrap();

        let Added::Kept(Some(recovery)) = added else {
            panic!("{added:?}")
        };
        assert_eq!(recovery.from_branch.as_deref(), Some("side"));
        assert_eq!(recovery.kept_branch, None);
        assert_eq!(recovery.kept_commits, 1);
        assert_eq!(sh(&tree, &["branch", "--show-current"]), "priory/1");
        assert_eq!(sh(&tree, &["rev-parse", "side"]), head);
    }

    #[test]
    fn a_detached_worktree_with_unpushed_commits_keeps_them_on_a_kept_branch() {
        let home = Home::new("unpushed");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        commit(&tree, "a.txt", "a\n");
        let head = commit(&tree, "b.txt", "b\n");
        let short = sh(&tree, &["rev-parse", "--short", "HEAD"]);

        let added = home.again().unwrap();

        let kept = format!("priory/1-kept-{short}");
        let Added::Kept(Some(recovery)) = added else {
            panic!("{added:?}")
        };
        assert_eq!(recovery.kept_branch.as_deref(), Some(kept.as_str()));
        assert_eq!(recovery.kept_commits, 2);
        assert_eq!(sh(&tree, &["rev-parse", &kept]), head);
        assert_eq!(sh(&tree, &["branch", "--show-current"]), "priory/1");
    }

    #[test]
    fn a_worktree_off_its_branch_with_uncommitted_changes_blocks_with_the_commands() {
        let home = Home::new("dirty");
        let tree = home.tree();
        sh(&tree, &["checkout", "--quiet", "--detach"]);
        let head = sh(&tree, &["rev-parse", "HEAD"]);
        fs::write(tree.join("staged.txt"), "s\n").unwrap();
        sh(&tree, &["add", "staged.txt"]);
        fs::write(tree.join("README.md"), "two\n").unwrap();
        fs::write(tree.join("loose.txt"), "l\n").unwrap();

        let reason = home.again().unwrap_err();

        let short = sh(&tree, &["rev-parse", "--short", "HEAD"]);
        let dir = quote(&tree);
        assert_eq!(
            reason,
            format!(
                "{} is detached at {short}, not on priory/1, and has 1 staged file, 1 unstaged \
                 file and 1 untracked file. Keep them on a branch: git -C {dir} switch -c \
                 priory/1-kept-{short} && git -C {dir} add -A && git -C {dir} commit -m \"Kept \
                 by hand\". Or drop them: git -C {dir} reset --hard && git -C {dir} clean -fd. \
                 Then resume the task, and Priory puts the worktree back on priory/1.",
                tree.display()
            )
        );
        assert_eq!(sh(&tree, &["rev-parse", "HEAD"]), head);
        assert_eq!(sh(&tree, &["branch", "--show-current"]), "");
    }

    #[test]
    fn a_worktree_with_a_rebase_in_progress_blocks_with_the_rebase_commands() {
        let home = Home::new("rebasing");
        let tree = home.tree();
        sh(&tree, &["switch", "--quiet", "-c", "onto"]);
        commit(&tree, "README.md", "onto\n");
        sh(&tree, &["checkout", "--quiet", "--detach", "priory/1"]);
        commit(&tree, "README.md", "mine\n");
        let rebase = run(git(&tree)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["rebase", "--quiet", "onto"]));
        assert!(rebase.is_err(), "the rebase conflicts");

        let reason = home.again().unwrap_err();

        let dir = quote(&tree);
        assert!(reason.contains("has a rebase in progress at "), "{reason}");
        assert!(
            reason.contains(&format!(
                "Finish it with git -C {dir} rebase --continue, or drop it with git -C {dir} rebase --abort"
            )),
            "{reason}"
        );
        assert!(
            tree.join(sh(&tree, &["rev-parse", "--git-path", "rebase-merge"]))
                .exists()
        );
    }

    #[test]
    fn a_folder_that_is_a_worktree_of_another_clone_still_blocks() {
        let home = Home::new("foreign");
        let other = home.dir.join("other");
        sh(
            &home.dir,
            &[
                "clone",
                "--quiet",
                home.dir.join("origin.git").to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        let path = home.home().join("worktrees/o/n/2");
        sh(
            &other,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "priory/2",
                path.to_str().unwrap(),
            ],
        );

        let reason = worktree(&home.home(), "o/n", "2", None, true, false).unwrap_err();

        assert_eq!(
            reason,
            format!(
                "{} exists but is not priory/2 of {}",
                path.display(),
                home.home().join("repos/o/n").display()
            )
        );
    }

    /// Task 1's worktree holds `target/sealed/out` under a 0555 folder that git ignores, as a
    /// test run that seals a kit leaves.
    fn sealed(home: &Home) -> PathBuf {
        let exclude = home.home().join("repos/o/n/.git/info/exclude");
        fs::write(&exclude, "target/\n").unwrap();
        let sealed = home.tree().join("target/sealed");
        fs::create_dir_all(&sealed).unwrap();
        fs::write(sealed.join("out"), "x\n").unwrap();
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o555)).unwrap();
        sealed
    }

    #[test]
    fn retiring_a_worktree_with_a_sealed_folder_under_an_ignored_target_removes_it_whole() {
        let home = Home::new("retire-sealed");
        sealed(&home);
        let found = crate::worktrees::Found {
            path: home.tree(),
            repo: "o/n".to_owned(),
            name: "1".to_owned(),
            entry: None,
        };

        crate::worktrees::retire(&home.home(), &found).unwrap();

        assert!(!home.tree().exists());
        assert!(!home.home().join("repos/o/n/.git/worktrees/1").exists());
        assert!(head(&home.home().join("repos/o/n"), "priory/1").is_some());
        assert_eq!(home.again(), Ok(Added::Restored));
    }

    #[test]
    fn a_folder_whose_gitdir_is_gone_is_added_back_from_its_branch() {
        let home = Home::new("orphaned");
        let sealed = sealed(&home);
        // What a `git worktree remove` that failed on the sealed folder leaves behind.
        fs::remove_file(home.tree().join("README.md")).unwrap();
        fs::remove_dir_all(home.home().join("repos/o/n/.git/worktrees/1")).unwrap();

        assert_eq!(home.again(), Ok(Added::Restored));

        assert!(!sealed.exists());
        let reclaim = home.home().join("reclaim");
        let deadline = Instant::now() + Duration::from_secs(20);
        while fs::read_dir(&reclaim).unwrap().next().is_some() {
            assert!(Instant::now() < deadline, "the old folder goes on a thread");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(home.tree().join("README.md").is_file());
        assert_eq!(sh(&home.tree(), &["branch", "--show-current"]), "priory/1");
    }

    #[test]
    fn a_folder_whose_gitdir_is_gone_and_that_holds_a_repository_moves_into_the_trash() {
        let home = Home::new("orphaned-repository");
        sealed(&home);
        sh(&home.tree(), &["init", "-q", "--bare", "target/o.git"]);
        fs::remove_dir_all(home.home().join("repos/o/n/.git/worktrees/1")).unwrap();

        assert_eq!(home.again(), Ok(Added::Restored));

        let trash = home.home().join("trash");
        let moved = fs::read_dir(trash).unwrap().next().unwrap().unwrap().path();
        assert!(moved.join("target/o.git/HEAD").is_file());
        assert!(!home.tree().join("target").exists());
        remove_sealed(&moved);
    }

    #[test]
    fn a_worktree_folder_that_cannot_be_removed_moves_into_the_trash() {
        let home = Home::new("aside");
        let leftover = home.home().join("worktrees/o/n/1");
        fs::create_dir_all(leftover.join("target")).unwrap();
        fs::write(leftover.join("target/owned-by-root"), "").unwrap();

        move_aside(&home.home(), &leftover).unwrap();
        fs::create_dir_all(&leftover).unwrap();
        move_aside(&home.home(), &leftover).unwrap();

        assert!(!leftover.exists());
        let moved = fs::read_dir(home.home().join("trash")).unwrap().count();
        assert_eq!(moved, 2, "each move keeps its own name");
    }

    #[test]
    fn changes_counts_staged_unstaged_and_untracked_files() {
        assert_eq!(changes(""), "");
        assert_eq!(changes("?? a\n"), "1 untracked file");
        assert_eq!(
            changes("MM a\nA  b\n?? c\n?? d\n"),
            "2 staged files, 1 unstaged file and 2 untracked files"
        );
        assert_eq!(changes(" D a\n"), "1 unstaged file");
    }
}
