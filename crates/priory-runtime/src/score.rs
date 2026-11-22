//! A `command` scorer's run: a scratch checkout of one commit under Priory's home, the scorer's
//! program exec'd there with a cleared environment, then the scratch folder removed by its path.

use crate::runtime::{git, run};
use crate::script;
use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// What a scorer's program needs: where it lives, what it grades and how long it may take.
pub struct Job<'a> {
    /// Priory's home, which holds the clones and the scratch folders.
    pub home: &'a Path,
    /// The task's worktree path, which names the clone holding its commits.
    pub worktree: &'a Path,
    /// The program, an absolute path, exec'd with no argument and no shell.
    pub program: &'a Path,
    /// The commit the task started from.
    pub base: &'a str,
    /// The result's commit, checked out for the program.
    pub head: &'a str,
    /// The result, as JSON, written to a file the program reads.
    pub result: &'a [u8],
    pub timeout: Duration,
}

/// How the program ended: its exit code when it exited, and a line saying how.
pub struct Ended {
    pub code: Option<i32>,
    pub reason: String,
}

/// Checks `job.head` out into a new folder under `home/scores`, runs the program there, and
/// removes that folder, whatever happened.
pub fn run_in_checkout(job: &Job) -> Result<Ended, String> {
    let scores = job.home.join("scores");
    fs::create_dir_all(&scores).map_err(|error| format!("{}: {error}", scores.display()))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let scratch = scores.join(format!("{}-{nanos}", std::process::id()));
    fs::create_dir(&scratch).map_err(|error| format!("{}: {error}", scratch.display()))?;
    let ended = checked_out(job, &scratch);
    if let Err(error) = fs::remove_dir_all(&scratch) {
        eprintln!("priory score: cannot remove {}: {error}", scratch.display());
    }
    ended
}

/// The clone under `home/repos` that the worktree `worktree` belongs to, whether or not the
/// worktree still exists.
fn clone_of(home: &Path, worktree: &Path) -> Option<PathBuf> {
    let repo = worktree
        .parent()?
        .strip_prefix(home.join("worktrees"))
        .ok()?;
    Some(home.join("repos").join(repo))
}

fn checked_out(job: &Job, scratch: &Path) -> Result<Ended, String> {
    // An agent wrote the commit, so it never reaches git as anything but a hex id.
    if job.head.len() < 4 || !job.head.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{:?} is not a commit id", job.head));
    }
    let clone = clone_of(job.home, job.worktree)
        .filter(|clone| clone.exists())
        .ok_or_else(|| format!("no clone holds the worktree {}", job.worktree.display()))?;
    let commit = format!("{}^{{commit}}", job.head);
    if run(git(&clone).args(["rev-parse", "--verify", "--quiet", &commit])).is_err() {
        return Err(format!("commit {} is not in {}", job.head, clone.display()));
    }
    let checkout = scratch.join("checkout");
    // A shared clone borrows the objects, so a commit no branch names any more still checks out.
    run(Command::new("git")
        .args(["clone", "--quiet", "--shared", "--no-checkout"])
        .arg(&clone)
        .arg(&checkout))?;
    run(git(&checkout).args(["checkout", "--quiet", "--detach", job.head]))?;
    let result = scratch.join("result.json");
    fs::write(&result, job.result).map_err(|error| format!("{}: {error}", result.display()))?;
    let log = scratch.join("output.log");
    let out = File::create(&log).map_err(|error| format!("{}: {error}", log.display()))?;
    let err = out.try_clone().map_err(|error| error.to_string())?;
    let mut child = Command::new(job.program)
        .env_clear()
        .env("PRIORY_SCORE_BASE", job.base)
        .env("PRIORY_SCORE_HEAD", job.head)
        .env("PRIORY_SCORE_RESULT", &result)
        .current_dir(&checkout)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|error| format!("{}: {error}", job.program.display()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < job.timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => break Err(format!("timed out after {}s", job.timeout.as_secs())),
            Err(error) => break Err(format!("cannot wait for it: {error}")),
        }
    };
    // Children the program left in the background end with it.
    script::kill(child.id());
    let _ = child.kill();
    let _ = child.wait();
    let output = script::collect(&log);
    let last = output.lines().rev().find(|line| !line.trim().is_empty());
    Ok(match status {
        Ok(status) => {
            let how = status.code().map_or_else(
                || "killed by a signal".to_owned(),
                |code| format!("exit {code}"),
            );
            Ended {
                code: status.code(),
                reason: match last {
                    Some(line) => format!("{how}: {}", line.trim()),
                    None => how,
                },
            }
        }
        Err(reason) => Ended { code: None, reason },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_that_is_not_a_hex_id_never_reaches_git() {
        let home = std::env::temp_dir().join(format!("priory-score-{}", std::process::id()));
        let job = Job {
            home: &home,
            worktree: &home.join("worktrees/owner/repo/1"),
            program: Path::new("/bin/true"),
            base: "abc0",
            head: "--output=/tmp/x",
            result: b"{}",
            timeout: Duration::from_secs(1),
        };

        let refused = run_in_checkout(&job).err();

        assert_eq!(
            refused.as_deref(),
            Some("\"--output=/tmp/x\" is not a commit id")
        );
        assert_eq!(fs::read_dir(home.join("scores")).unwrap().count(), 0);
        fs::remove_dir_all(&home).unwrap();
    }
}
