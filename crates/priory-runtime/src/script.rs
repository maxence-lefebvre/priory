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

/// Execs `argv` with `env` in `dir`, in its own process group, its stdout going to `log` and its
/// stderr to `errors`. It waits for `release`, and never runs if the caller dies first, so the
/// caller can record the process group before anything runs. `poll` returns the tail of stdout
/// alone.
pub fn spawn_held(
    argv: &[&str],
    env: &[(&str, &std::ffi::OsStr)],
    dir: &Path,
    log: &Path,
    errors: &Path,
    timeout: Duration,
) -> Result<Script, String> {
    let mut command = Command::new("sh");
    command
        .args(["-c", "read -r _ && exec \"$@\" </dev/null", "sh"])
        .args(argv)
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
        .stderr(Stdio::null())
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
        self.started = Instant::now();
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

/// How a script that [`run_bounded`] ran ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// It exited with this code.
    Exit(i32),
    /// A signal it did not send itself killed it.
    Signal,
    /// It ran past its timeout, and was killed with its process group.
    TimedOut,
    /// It printed more than the cap on `stdout` or `stderr`, and was killed with its process group.
    TooMuch(&'static str),
    /// It could not start.
    CannotStart(String),
}

impl std::fmt::Display for Ended {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Ended::Exit(code) => write!(formatter, "exit {code}"),
            Ended::Signal => write!(formatter, "killed by a signal"),
            Ended::TimedOut => write!(formatter, "timed out"),
            Ended::TooMuch(stream) => write!(formatter, "printed too much on {stream}"),
            Ended::CannotStart(error) => write!(formatter, "cannot start: {error}"),
        }
    }
}

/// What [`run_bounded`] kept of one run.
#[derive(Debug, Clone)]
pub struct Bounded {
    pub ended: Ended,
    /// At most the cap of its stdout.
    pub stdout: Vec<u8>,
    /// The last `KEPT` bytes of the at most cap bytes of stderr it read.
    pub stderr: String,
    pub seconds: f64,
}

/// Execs `argv` in `dir` in its own process group, with only `env` in its environment, writes
/// `input` to its stdin then closes it, and reads both streams as they come. It kills the group
/// once the script ends, at `timeout` counted from the start, or once a stream passes `cap` bytes.
pub fn run_bounded(
    argv: &[String],
    env: &[(String, std::ffi::OsString)],
    dir: &Path,
    input: &[u8],
    timeout: Duration,
    cap: usize,
) -> Bounded {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    let started = Instant::now();
    let failed = |error: String| Bounded {
        ended: Ended::CannotStart(error),
        stdout: Vec::new(),
        stderr: String::new(),
        seconds: started.elapsed().as_secs_f64(),
    };
    let Some((program, args)) = argv.split_first() else {
        return failed("no program".to_owned());
    };
    let spawned = Command::new(program)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(name, value)| (name, value)))
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return failed(format!("{program}: {error}")),
    };
    let over = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    for (name, stream) in [
        (
            "stdout",
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ),
        (
            "stderr",
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ),
    ] {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let (done, finished) = std::sync::mpsc::channel::<bool>();
        if let Some(mut stream) = stream {
            let (kept, over) = (Arc::clone(&kept), Arc::clone(&over));
            std::thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                let mut passed = false;
                loop {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                            if kept.len() + read > cap {
                                passed = true;
                                over.store(true, Ordering::SeqCst);
                                break;
                            }
                            kept.extend_from_slice(&buffer[..read]);
                        }
                    }
                }
                let _ = done.send(passed);
            });
        }
        readers.push((name, kept, finished));
    }
    if let Some(mut stdin) = child.stdin.take() {
        let input = input.to_vec();
        // A script that never reads its input holds this thread until it is killed.
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let pid = child.id();
    let ended = loop {
        if over.load(Ordering::SeqCst) {
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.code().map_or(Ended::Signal, Ended::Exit)),
            Ok(None) if started.elapsed() >= timeout => break Some(Ended::TimedOut),
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => break Some(Ended::CannotStart(format!("cannot wait for it: {error}"))),
        }
    };
    // Children it left in the background end with it, so its pipes close.
    kill(pid);
    let _ = child.kill();
    let _ = child.wait();
    let mut passed = None;
    let mut streams = Vec::new();
    for (name, kept, finished) in readers {
        if finished.recv_timeout(Duration::from_secs(1)) == Ok(true) {
            passed = passed.or(Some(name));
        }
        let kept = std::mem::take(&mut *kept.lock().unwrap_or_else(|e| e.into_inner()));
        streams.push(kept);
    }
    let ended = match (ended, passed) {
        (Some(Ended::Exit(code)), None) => Ended::Exit(code),
        (_, Some(stream)) => Ended::TooMuch(stream),
        (Some(ended), None) => ended,
        (None, None) => Ended::TooMuch("stdout"),
    };
    let stderr = streams.pop().unwrap_or_default();
    let stdout = streams.pop().unwrap_or_default();
    let tail = &stderr[stderr.len().saturating_sub(KEPT)..];
    Bounded {
        ended,
        stdout,
        stderr: String::from_utf8_lossy(tail).trim().to_owned(),
        seconds: started.elapsed().as_secs_f64(),
    }
}
