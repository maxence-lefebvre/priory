use std::fs::{self, File};
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
    let at = |error: std::io::Error| format!("{}: {error}", log.display());
    fs::create_dir_all(log.parent().unwrap_or(home)).map_err(at)?;
    let out = File::create(log).map_err(at)?;
    let err = out.try_clone().map_err(at)?;
    let child = Command::new("sh")
        .args(["-c", run])
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0)
        .spawn()
        .map_err(|error| format!("sh: {error}"))?;
    Ok(Script {
        child,
        log: log.to_path_buf(),
        timeout,
        started: Instant::now(),
    })
}

impl Script {
    /// Its status and the tail of its output once it ended, killing it with its children past its
    /// timeout; `None` while it runs.
    pub fn poll(&mut self) -> Option<(String, String)> {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => status.code().map_or_else(
                || "killed by a signal".to_owned(),
                |code| format!("exit {code}"),
            ),
            Ok(None) if self.started.elapsed() < self.timeout => return None,
            Ok(None) => {
                let group = format!("-{}", self.child.id());
                let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
                let _ = self.child.wait();
                format!("timed out after {}s", self.timeout.as_secs())
            }
            Err(error) => format!("cannot wait for it: {error}"),
        };
        let output = fs::read(&self.log).unwrap_or_default();
        let _ = fs::remove_file(&self.log);
        let tail = String::from_utf8_lossy(&output[output.len().saturating_sub(KEPT)..]);
        Some((status, tail.trim().to_owned()))
    }
}
