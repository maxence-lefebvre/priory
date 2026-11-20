use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The most output kept from a script: its tail.
const KEPT: usize = 2000;

/// A sweep script running in its own process group, its output going to a log file.
pub struct Script {
    child: Child,
    log: PathBuf,
    timeout: Duration,
    started: Instant,
}

/// Starts `run` with `sh -c` in `home`, writing its output to `log`.
pub fn spawn(run: &str, home: &Path, log: &Path, timeout: Duration) -> Result<Script, String> {
    let mut command = Command::new("sh");
    command.args(["-c", run]);
    spawn_command(command, home, log, timeout)
}

/// Starts `command` in `dir` in its own process group, writing its output to `log`.
pub fn spawn_command(
    command: Command,
    dir: &Path,
    log: &Path,
    timeout: Duration,
) -> Result<Script, String> {
    start(command, dir, log, None, Stdio::null(), timeout)
}

/// Starts `run` with `sh -c` and `env` in `dir`, in its own process group, its stdout going to
/// `log` and its stderr to `errors`. `run` waits for `release`, and never runs if the caller dies
/// first, so the caller can record the process group before anything runs. `poll` returns the
/// tail of stdout alone.
pub fn spawn_held(
    run: &str,
    env: &[(&str, &std::ffi::OsStr)],
    dir: &Path,
    log: &Path,
    errors: &Path,
    timeout: Duration,
) -> Result<Script, String> {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "read -r _ && exec sh -c \"$1\" sh </dev/null",
            "sh",
            run,
        ])
        .envs(env.iter().copied());
    start(command, dir, log, Some(errors), Stdio::piped(), timeout)
}

fn start(
    mut command: Command,
    dir: &Path,
    log: &Path,
    errors: Option<&Path>,
    stdin: Stdio,
    timeout: Duration,
) -> Result<Script, String> {
    let at = |error: std::io::Error| format!("{}: {error}", log.display());
    fs::create_dir_all(log.parent().unwrap_or(dir)).map_err(at)?;
    let out = File::create(log).map_err(at)?;
    let err = match errors {
        Some(errors) => File::create(errors).map_err(at)?,
        None => out.try_clone().map_err(at)?,
    };
    let program = command.get_program().to_string_lossy().into_owned();
    let child = command
        .current_dir(dir)
        .stdin(stdin)
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;
    Ok(Script {
        child,
        log: log.to_path_buf(),
        timeout,
        started: Instant::now(),
    })
}

/// Kills the process group `pid` leads, with every child it left.
pub fn kill(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .status();
}

impl Script {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Lets a script from `spawn_held` run its command.
    pub fn release(&mut self) {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = std::io::Write::write_all(&mut stdin, b"\n");
        }
    }

    /// Its status and the tail of its output once it ended, killing its children, or killing it
    /// with them past its timeout; `None` while it runs.
    pub fn poll(&mut self) -> Option<(String, String)> {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => status.code().map_or_else(
                || "killed by a signal".to_owned(),
                |code| format!("exit {code}"),
            ),
            Ok(None) if self.started.elapsed() < self.timeout => return None,
            Ok(None) => {
                kill(self.child.id());
                let _ = self.child.kill();
                let _ = self.child.wait();
                format!("timed out after {}s", self.timeout.as_secs())
            }
            Err(error) => format!("cannot wait for it: {error}"),
        };
        // A script runs once; children it left in the background end with it.
        kill(self.child.id());
        Some((status, collect(&self.log)))
    }
}

/// The last `KEPT` bytes of `log`, read without loading the rest, then removes it.
pub fn collect(log: &Path) -> String {
    let mut kept = Vec::new();
    if let Ok(mut file) = File::open(log) {
        let length = file.metadata().map_or(0, |metadata| metadata.len());
        let _ = file.seek(SeekFrom::Start(length.saturating_sub(KEPT as u64)));
        let _ = file.take(KEPT as u64).read_to_end(&mut kept);
    }
    let _ = fs::remove_file(log);
    String::from_utf8_lossy(&kept).trim().to_owned()
}
