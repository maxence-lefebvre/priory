use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

/// Priory's own directory: `PRIORY_HOME`, else `$XDG_STATE_HOME/priory`, else `~/.local/state/priory`.
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
    state.join("priory")
}

/// Runs `command` and returns its stdout, or an error naming it with its stderr.
fn run(command: &mut Command) -> Result<String, String> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{command:?}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// The task's worktree on `priory/<task>`, from the default branch of Priory's own clone of `repo`
/// (`owner/name`). A worktree left by an interrupted start is reused when it is on that branch.
pub fn worktree(home: &Path, repo: &str, task: i64) -> Result<(PathBuf, String), String> {
    let clone = home.join("repos").join(format!("{repo}.git"));
    let branch = format!("priory/{task}");
    let path = home.join("worktrees").join(repo).join(task.to_string());
    if !clone.exists() {
        fs::create_dir_all(clone.parent().expect("clone has a parent"))
            .map_err(|error| error.to_string())?;
        run(Command::new("git")
            .args(["clone", "--quiet", "--bare"])
            .arg(format!("https://github.com/{repo}.git"))
            .arg(&clone))?;
        run(git(&clone).args([
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ]))?;
    }
    if path.exists() {
        let current = run(git(&path).args(["branch", "--show-current"]))?;
        if current.trim() != branch {
            return Err(format!(
                "{} exists on {:?}, not {branch}",
                path.display(),
                current.trim()
            ));
        }
        return Ok((path, branch));
    }
    run(git(&clone).args(["fetch", "--quiet", "origin"]))?;
    run(git(&clone).args(["remote", "set-head", "origin", "--auto"]))?;
    run(git(&clone)
        .args(["worktree", "add", "--quiet", "-b", &branch])
        .arg(&path)
        .arg("origin/HEAD"))?;
    Ok((path, branch))
}

fn tmux() -> Command {
    Command::new("tmux")
}

/// What `claude` needs to start a run.
pub struct Launch<'a> {
    pub tmux: &'a str,
    pub dir: &'a Path,
    pub home: &'a Path,
    pub session: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub body: &'a str,
}

/// Starts `claude` in a detached tmux session, with hooks that report to this daemon, and tags the
/// session with the run's harness session id so the daemon can tell its own sessions apart.
pub fn launch(launch: &Launch) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
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
        .args(["--", "claude", "--session-id", launch.session]);
    if let Some(model) = launch.model {
        command.args(["--model", model]);
    }
    if let Some(effort) = launch.effort {
        command.args(["--effort", effort]);
    }
    if !launch.body.is_empty() {
        command.args(["--append-system-prompt", launch.body]);
    }
    command.arg("--settings").arg(settings(&exe));
    run(&mut command)?;
    run(tmux().args([
        "set-option",
        "-t",
        &format!("={}:", launch.tmux),
        "@priory",
        launch.session,
    ]))?;
    Ok(())
}

/// Claude Code settings whose hooks call `priory hook <event>`.
fn settings(exe: &Path) -> String {
    let quoted = format!("'{}'", exe.display().to_string().replace('\'', r"'\''"));
    let hooks: serde_json::Map<_, _> = ["SessionStart", "UserPromptSubmit", "Stop"]
        .into_iter()
        .map(|event| {
            let command = format!("{quoted} hook {event}");
            (
                event.to_owned(),
                serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]),
            )
        })
        .collect();
    serde_json::json!({ "hooks": hooks }).to_string()
}

/// Whether the tmux session `name` lives and belongs to the run with harness session `session`.
pub fn owns(name: &str, session: &str) -> bool {
    run(tmux().args(["show-options", "-qv", "-t", &format!("={name}:"), "@priory"]))
        .is_ok_and(|tag| tag.trim() == session)
}

/// Pastes `text` into the session's pane as one bracketed paste, then presses Enter. tmux reads the
/// text from stdin, so none of it is taken for key names.
pub fn type_text(name: &str, text: &str) -> Result<(), String> {
    let buffer = format!("priory-{name}");
    let mut load = tmux()
        .args(["load-buffer", "-b", &buffer, "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    load.stdin
        .take()
        .expect("stdin is piped")
        .write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    let loaded = load.wait_with_output().map_err(|error| error.to_string())?;
    if !loaded.status.success() {
        return Err(String::from_utf8_lossy(&loaded.stderr).into_owned());
    }
    let pane = format!("={name}:");
    run(tmux().args(["paste-buffer", "-p", "-d", "-b", &buffer, "-t", &pane]))?;
    run(tmux().args(["send-keys", "-t", &pane, "Enter"]))?;
    Ok(())
}

fn socket(home: &Path) -> PathBuf {
    home.join("priory.sock")
}

fn spool(home: &Path) -> PathBuf {
    home.join("signals.spool")
}

/// The daemon's end of the socket: its lock, the signals spooled while no daemon listened, and a
/// channel of the lines that arrive from now on.
pub struct Listening {
    _lock: File,
    pub spooled: Vec<String>,
    pub lines: Receiver<String>,
}

/// Takes the daemon lock under `home`, binds the signal socket and reads the spool.
pub fn listen(home: &Path) -> Result<Listening, String> {
    let at = |error: std::io::Error| format!("{}: {error}", home.display());
    fs::create_dir_all(home).map_err(at)?;
    let lock = File::create(home.join("daemon.lock")).map_err(at)?;
    lock.try_lock()
        .map_err(|_| format!("{}: another daemon holds its lock", home.display()))?;
    let path = socket(home);
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(at)?;
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        }
    });
    // shortcut: a hook that opened the spool before this rename and writes after the read is lost; ack signals when slice 5 adds replies.
    let taken = home.join("signals.spool.taken");
    let spooled = match fs::rename(spool(home), &taken) {
        Ok(()) => {
            let lines = fs::read_to_string(&taken).map_err(at)?;
            fs::remove_file(&taken).map_err(at)?;
            lines.lines().map(str::to_owned).collect()
        }
        Err(_) => Vec::new(),
    };
    Ok(Listening {
        _lock: lock,
        spooled,
        lines,
    })
}

/// Sends one line to the daemon, or appends it to the spool when no daemon listens.
pub fn send(home: &Path, line: &str) -> Result<(), String> {
    let line = format!("{line}\n");
    if let Ok(mut stream) = UnixStream::connect(socket(home))
        && stream.write_all(line.as_bytes()).is_ok()
    {
        return Ok(());
    }
    fs::create_dir_all(home).map_err(|error| error.to_string())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(spool(home))
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|error| format!("{}: {error}", spool(home).display()))
}
