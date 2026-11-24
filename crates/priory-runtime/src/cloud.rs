//! The host side of a cloud run: what reaches GitHub before `claude --cloud`, the isolated clone it
//! runs in, and the commit its session pushes. Nothing here moves a commit it did not check.

use crate::runtime::{
    ClaudeCode, Harness, NESTED, changes, command_timeout, fed_within, git, head, is_ancestor,
    kill, one_line, run, shown, tmux, tmux_socket, tmux_socket_fits, write_private,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long one git command of a poll may take.
const POLL_TIMEOUT: Duration = Duration::from_secs(15);

/// Why a start gave no session id.
pub enum NotStarted {
    /// No session exists.
    Refused(String),
    /// A session may exist, so a second start could double it.
    Uncertain(String),
}

impl From<String> for NotStarted {
    fn from(reason: String) -> Self {
        NotStarted::Refused(reason)
    }
}

/// A cloud session that started.
pub struct Started {
    pub id: String,
    /// The line that says the CLI uploaded the clone, where GitHub gave the session no access.
    pub upload: Option<String>,
}

/// What a start needs: the task's branch at `head`, which the session starts from.
pub struct Start {
    pub tmux: String,
    pub home: PathBuf,
    pub repo: String,
    pub worktree: PathBuf,
    pub branch: String,
    pub head: String,
    pub prompt: String,
}

/// The folder `claude --cloud` runs in for `repo`: a clone of one branch, made anew per start.
pub fn folder(home: &Path, repo: &str) -> PathBuf {
    home.join("cloud").join(repo)
}

/// The commit `branch` is at, or why `worktree` is not on it with nothing uncommitted.
pub fn clean_head(worktree: &Path, branch: &str) -> Result<String, String> {
    let on = run(git(worktree).args(["branch", "--show-current"]))?;
    let on = on.trim();
    if on != branch {
        return Err(format!(
            "{} is on {}, not on {branch}",
            worktree.display(),
            if on.is_empty() { "no branch" } else { on }
        ));
    }
    let changed = changes(&run(git(worktree).args([
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
    ]))?);
    if !changed.is_empty() {
        return Err(format!(
            "{} has {changed}, which a cloud session would not see. Commit, ignore or remove them",
            worktree.display()
        ));
    }
    head(worktree, branch).ok_or_else(|| format!("branch {branch} is gone"))
}

/// Whether `head` holds a commit of the local branch `parent` that no branch of `origin` has, as
/// Priory's clone last saw `origin`.
pub fn holds_unpushed(worktree: &Path, head: &str, parent: &str) -> Result<bool, String> {
    let parent = format!("refs/heads/{parent}");
    let shared = run(git(worktree).args(["merge-base", head, &parent]))?;
    let listed = ["rev-list", "--max-count=1", shared.trim()];
    let unpushed = run(git(worktree)
        .args(listed)
        .args(["--not", "--remotes=origin"]))?;
    Ok(!unpushed.trim().is_empty())
}

/// Pushes the task's branch, then runs `claude --cloud` in an isolated clone of that branch alone.
/// The CLI uploads the whole clone it runs in when the Claude GitHub App is not on the repository,
/// and a Priory worktree shares its clone with every other task.
pub fn start(start: &Start) -> Result<Started, NotStarted> {
    let target = format!("{}:refs/heads/{}", start.head, start.branch);
    let pushed = ["push", "--quiet", "origin", &target];
    remote(&start.worktree, &pushed, command_timeout())?;
    let found = remote_head(&start.worktree, &start.branch, command_timeout())?;
    if found.as_deref() != Some(&start.head) {
        return Err(NotStarted::Refused(format!(
            "origin has {} at {}, not at {}",
            start.branch,
            found.as_deref().unwrap_or("no commit"),
            start.head
        )));
    }
    let dir = folder(&start.home, &start.repo);
    let started = isolate(start, &dir)
        .map_err(NotStarted::Refused)
        .and_then(|()| create(start, &dir));
    let _ = fs::remove_dir_all(&dir);
    started
}

fn isolate(start: &Start, dir: &Path) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("{}: {error}", dir.display());
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(failed)?;
    }
    fs::create_dir_all(dir.parent().expect("the folder has a parent")).map_err(failed)?;
    let clone = start.home.join("repos").join(&start.repo);
    // `--no-local` copies only what the branch reaches, never the clone's other objects.
    run(git(&start.home)
        .args(["clone", "--quiet", "--no-checkout", "--no-local"])
        .args(["--no-tags", "--single-branch", "--template=", "--branch"])
        .arg(&start.branch)
        .arg(&clone)
        .arg(dir))?;
    let url = format!("https://github.com/{}.git", start.repo);
    run(git(dir).args(["remote", "set-url", "origin", &url]))?;
    let tracking = format!("refs/remotes/origin/{}", start.branch);
    run(git(dir).args(["update-ref", &tracking, &start.head]))?;
    run(git(dir).args(["reset", "--quiet", "--hard", &start.head]))?;
    Ok(())
}

/// Runs `claude --cloud PROMPT` in `dir` inside a detached tmux session, since the CLI creates a
/// cloud session only in a terminal, and waits up to the command timeout for it to exit.
fn create(start: &Start, dir: &Path) -> Result<Started, NotStarted> {
    tmux_socket_fits(&tmux_socket())?;
    let name = start.tmux.as_str();
    // tmux refuses a command over about 16 KiB, so the prompt reaches the CLI through a file.
    let file = start.home.join("prompts").join(name);
    fs::create_dir_all(file.parent().expect("the file has a parent"))
        .and_then(|()| write_private(&file, &start.prompt))
        .map_err(|error| format!("{}: {error}", file.display()))?;
    let upload = ["CCR_FORCE_BUNDLE", "CCR_ENABLE_BUNDLE"];
    run(tmux()
        .args(["new-session", "-d", "-x", "200", "-y", "50", "-s", name])
        .arg("-c")
        .arg(dir)
        .args(["--", "sh", "-c", SCRIPT])
        .arg(&file)
        .arg("env")
        .args(
            NESTED
                .iter()
                .chain(&upload)
                .flat_map(|variable| ["-u", variable]),
        )
        .args(["claude", "--cloud"]))?;
    let deadline = Instant::now() + command_timeout();
    loop {
        let screen = shown(&format!("={name}:"), true);
        if let Some(code) = screen
            .iter()
            .find_map(|line| line.trim().strip_prefix(EXIT))
        {
            kill(name);
            let printed: Vec<_> = screen
                .iter()
                .filter(|line| !line.trim().starts_with(EXIT))
                .map(String::as_str)
                .collect();
            return read(code, &printed.join("\n"));
        }
        if ClaudeCode.asks_trust(&screen.join("\n")) {
            kill(name);
            return Err(NotStarted::Refused(format!(
                "claude --cloud asked whether to trust {}; Priory never answers it. Set trust_worktrees = true in factory.toml, or trust the folder by hand",
                dir.display()
            )));
        }
        if run(tmux().args(["has-session", "-t", &format!("={name}")])).is_err() {
            return Err(NotStarted::Uncertain(
                "claude --cloud was killed before it exited".into(),
            ));
        }
        if Instant::now() >= deadline {
            kill(name);
            return Err(NotStarted::Uncertain(format!(
                "claude --cloud did not exit within {}s",
                command_timeout().as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

const EXIT: &str = "priory-cloud-exit=";

/// Runs the CLI with the prompt file `$0` as its last argument, then prints its exit code and stays,
/// so the pane still shows what it printed.
const SCRIPT: &str = r#"p=$(cat "$0"; printf x); rm -f "$0"; "$@" "${p%x}"; echo "priory-cloud-exit=$?"; exec sleep 600"#;

/// What a `claude --cloud` that exited with `code` and `printed` started.
fn read(code: &str, printed: &str) -> Result<Started, NotStarted> {
    let id = session(printed);
    let said = one_line(printed);
    if code != "0" {
        let reason = format!("claude --cloud exited {code}: {said}");
        // 128 and up is a signal, which may land after the request went out.
        let refused = id.is_none()
            && !printed.contains("may still exist")
            && code.parse::<u32>().is_ok_and(|code| code < 128);
        return Err(if refused {
            NotStarted::Refused(reason)
        } else {
            NotStarted::Uncertain(reason)
        });
    }
    let Some(id) = id else {
        return Err(NotStarted::Uncertain(format!(
            "claude --cloud printed no session id: {said}"
        )));
    };
    let line = |marks: &[&str]| {
        own_lines(printed)
            .find(|line| marks.iter().any(|mark| line.contains(mark)))
            .map(str::to_owned)
    };
    if let Some(elsewhere) = line(&["is not on GitHub", "starts empty", "session starts from"]) {
        return Err(NotStarted::Uncertain(format!(
            "session https://claude.ai/code/{id} did not start from the task's branch: {elsewhere}"
        )));
    }
    let upload = line(&[
        "Uploading your working tree",
        "Uploaded your working tree",
        "Uploading this repository",
        "Uploaded this repository",
        "Packaging this repository",
        "Packaged this repository",
        "Left on this machine",
        "Left out of this upload",
    ]);
    Ok(Started { id, upload })
}

/// The lines of `printed` that hold no text of the session's title.
fn own_lines(printed: &str) -> impl Iterator<Item = &str> {
    printed
        .lines()
        .map(str::trim)
        .filter(|line| !line.contains("Created cloud session:"))
}

/// The `session_<id>` in the `View:` URL or the `Resume with:` line of `printed`.
pub fn session(printed: &str) -> Option<String> {
    own_lines(printed).find_map(|line| {
        let rest = match line.split_once("https://claude.ai/code/session_") {
            Some((_, rest)) => rest,
            None => line.strip_prefix("Resume with:")?.split_once("session_")?.1,
        };
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!id.is_empty()).then(|| format!("session_{id}"))
    })
}

/// Runs git against `origin` in `dir`, where nothing may prompt: the daemon has no terminal.
fn remote(dir: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let ssh = std::env::var("GIT_SSH_COMMAND")
        .ok()
        .or_else(|| {
            run(Command::new("git").args(["config", "--global", "core.sshCommand"]))
                .ok()
                .map(|set| set.trim().to_owned())
        })
        .filter(|set| !set.is_empty())
        .unwrap_or_else(|| "ssh".into());
    let mut command = git(dir);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", format!("{ssh} -oBatchMode=yes"))
        .args(args);
    let said = format!("git {}", args.join(" "));
    let output = fed_within(&mut command, None, timeout)
        .map_err(|error| format!("{said}: {error}"))?
        .ok_or_else(|| format!("{said}: timed out after {}s", timeout.as_secs()))?;
    if !output.status.success() {
        return Err(format!(
            "{said}: {}",
            one_line(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The commit `branch` points at on the worktree's `origin`, or none while it is not there.
fn remote_head(worktree: &Path, branch: &str, timeout: Duration) -> Result<Option<String>, String> {
    let name = format!("refs/heads/{branch}");
    let listed = remote(worktree, &["ls-remote", "origin", &name], timeout)?;
    Ok(listed.split_whitespace().next().map(str::to_owned))
}

/// The commit `branch` has on `origin`, fetched by that SHA, or none while it is not there.
pub fn poll(worktree: &Path, branch: &str) -> Result<Option<String>, String> {
    let Some(commit) = remote_head(worktree, branch, POLL_TIMEOUT)? else {
        return Ok(None);
    };
    let fetch = [
        "fetch",
        "--quiet",
        "--no-tags",
        "--no-write-fetch-head",
        "--no-recurse-submodules",
        "origin",
        &commit,
    ];
    remote(worktree, &fetch, POLL_TIMEOUT)?;
    Ok(Some(commit))
}

/// A commit a cloud session pushed: its full SHA, subject and body.
pub struct Pushed {
    pub commit: String,
    pub subject: String,
    pub body: String,
}

/// Why a pushed commit was not taken. Either way nothing moved.
pub enum NotTaken {
    /// The worktree cannot take it now; a person can change that.
    Later(String),
    Never(String),
}

/// Moves the worktree's own `branch` from `base`, where the session started, forward to the
/// fetched `commit`. A push of `base` itself is no work yet.
pub fn adopt(
    worktree: &Path,
    branch: &str,
    base: &str,
    commit: &str,
) -> Result<Option<Pushed>, NotTaken> {
    if commit == base {
        return Ok(None);
    }
    if !is_ancestor(worktree, base, commit) {
        return Err(NotTaken::Never(format!(
            "{commit} does not descend from {base}, where the session started"
        )));
    }
    let at = clean_head(worktree, branch).map_err(NotTaken::Later)?;
    // A daemon that stopped after the merge finds the branch at `commit` already.
    if at != commit {
        if at != base {
            return Err(NotTaken::Later(format!(
                "{branch} moved to {at} since the session started at {base}"
            )));
        }
        // Without `--no-overwrite-ignore`, git replaces an ignored local file that the push tracks.
        let merge = ["merge", "--quiet", "--ff-only", "--no-overwrite-ignore"];
        run(git(worktree).args(merge).arg(commit)).map_err(NotTaken::Later)?;
    }
    let field = |format: &str| {
        run(git(worktree).args(["log", "-1", &format!("--format={format}"), commit]))
            .map(|text| text.trim().to_owned())
            .map_err(NotTaken::Later)
    };
    Ok(Some(Pushed {
        commit: commit.to_owned(),
        subject: field("%s")?,
        body: field("%b")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn a_cloud_session_id_is_read_from_the_view_url_or_the_resume_line() {
        let title =
            "Created cloud session: Fix session_expired at https://claude.ai/code/session_title\n";
        let view = format!("{title}View: https://claude.ai/code/session_01V4gk?from=cli&m=0\n");
        let resume = format!("{title}Resume with: claude --teleport session_02Ab\n");
        assert_eq!(session(&view).as_deref(), Some("session_01V4gk"));
        assert_eq!(session(&resume).as_deref(), Some("session_02Ab"));
        assert_eq!(session(title), None);
        assert_eq!(session("Error: session_expired"), None);
    }

    #[test]
    fn only_an_exit_that_cannot_have_made_a_session_is_a_refused_start() {
        let refused = |code, printed| matches!(read(code, printed), Err(NotStarted::Refused(_)));
        assert!(refused("1", "Error: Not logged in."));
        assert!(!refused(
            "1",
            "Cancelled, so a cloud session may still exist"
        ));
        assert!(!refused("1", "View: https://claude.ai/code/session_01"));
        assert!(!refused("143", ""));
        assert!(!refused("0", ""));
        let uploaded = "Uploaded this repository\nView: https://claude.ai/code/session_01";
        assert!(read("0", uploaded).is_ok_and(|started| started.upload.is_some()));
    }

    proptest! {
        #[test]
        fn cloud_output_gives_a_session_id_or_none_without_panicking(
            code in "[0-9a-z]{0,4}",
            printed in "(.{0,20}(https://claude.ai/code/|Resume with: |)session_[A-Za-z0-9]{0,12}.{0,20}\n){0,3}|.{0,80}"
        ) {
            let _ = read(&code, &printed);
            if let Some(id) = session(&printed) {
                prop_assert!(printed.contains(&id));
                prop_assert!(id.len() > "session_".len());
            }
        }
    }
}
