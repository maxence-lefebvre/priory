use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

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
/// (`owner/name`). The clone has no checkout: a person trusts it once in Claude, and its worktrees
/// inherit that trust. A worktree of that clone left on that branch by an interrupted start is reused.
pub fn worktree(home: &Path, repo: &str, task: i64) -> Result<(PathBuf, String), String> {
    let clone = home.join("repos").join(repo);
    let branch = format!("priory/{task}");
    let path = home.join("worktrees").join(repo).join(task.to_string());
    if !clone.exists() {
        // A clone interrupted before the rename leaves only the partial directory, which is replaced.
        let partial = PathBuf::from(format!("{}.partial", clone.display()));
        let _ = fs::remove_dir_all(&partial);
        fs::create_dir_all(clone.parent().expect("clone has a parent"))
            .map_err(|error| error.to_string())?;
        run(Command::new("git")
            .args(["clone", "--quiet", "--no-checkout"])
            .arg(format!("https://github.com/{repo}.git"))
            .arg(&partial))?;
        fs::rename(&partial, &clone).map_err(|error| error.to_string())?;
    }
    if path.exists() {
        let common =
            run(git(&path).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
        let current = run(git(&path).args(["branch", "--show-current"]))?;
        let ours =
            fs::canonicalize(common.trim()).ok() == fs::canonicalize(clone.join(".git")).ok();
        if !ours || current.trim() != branch {
            return Err(format!(
                "{} exists but is not {branch} of {}",
                path.display(),
                clone.display()
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

/// Starts `claude` in a detached tmux session, with hooks that report to this daemon. The same tmux
/// command tags the new pane with the run's harness session id, so a crash cannot leave it untagged.
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
    command.args([";", "set-option", "-p", "@priory", launch.session]);
    run(&mut command).map(drop)
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

/// The pane of the tmux session `name` that runs the harness session `session`, if it lives.
pub fn pane(name: &str, session: &str) -> Option<String> {
    let panes = run(tmux().args([
        "list-panes",
        "-s",
        "-t",
        &format!("={name}:"),
        "-F",
        "#{pane_id} #{@priory}",
    ]))
    .ok()?;
    panes.lines().find_map(|line| {
        let (pane, tag) = line.split_once(' ')?;
        (tag == session).then(|| pane.to_owned())
    })
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
    let buffer = format!("priory{pane}");
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
    run(tmux().args(["paste-buffer", "-p", "-d", "-b", &buffer, "-t", pane]))?;
    run(tmux().args(["send-keys", "-t", pane, "Enter"]))?;
    Ok(())
}

fn socket(home: &Path) -> PathBuf {
    home.join("priory.sock")
}

fn spool(home: &Path) -> PathBuf {
    home.join("signals.spool")
}

/// One line from the socket, with the sender to call once it is applied.
pub struct Line {
    pub text: String,
    pub applied: Sender<()>,
}

/// The daemon's end of the socket: its lock, and the lines that arrive.
pub struct Listening {
    _lock: File,
    pub lines: Receiver<Line>,
}

/// Takes the daemon lock under `home` and binds the signal socket. Each connection's line is
/// answered `ok` once the daemon applied it.
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
        for mut stream in listener.incoming().flatten() {
            let mut text = String::new();
            if BufReader::new(&stream).read_line(&mut text).is_err() {
                continue;
            }
            let (applied, done) = mpsc::channel();
            if sender.send(Line { text, applied }).is_err() {
                return;
            }
            if done.recv().is_ok() {
                let _ = stream.write_all(b"ok\n");
            }
        }
    });
    Ok(Listening { _lock: lock, lines })
}

/// The signals spooled while no daemon listened. `apply` runs on each before the spool is removed.
pub fn replay(
    home: &Path,
    mut apply: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    let taken = home.join("signals.spool.taken");
    let at = |error: std::io::Error| format!("{}: {error}", taken.display());
    loop {
        // shortcut: a hook that opened the spool before this rename and writes after the read is lost; move the spool into SQLite if that shows up.
        if !taken.exists() && fs::rename(spool(home), &taken).is_err() {
            return Ok(());
        }
        for line in fs::read_to_string(&taken).map_err(at)?.lines() {
            apply(line)?;
        }
        fs::remove_file(&taken).map_err(at)?;
    }
}

/// Sends one line to the daemon and waits up to 10 seconds for it to be applied. Only when no
/// daemon takes the line is it appended to the spool, so a slow daemon never gets it twice.
pub fn send(home: &Path, line: &str) -> Result<(), String> {
    let line = format!("{line}\n");
    let wait = Some(Duration::from_secs(10));
    let sent = UnixStream::connect(socket(home)).and_then(|mut stream| {
        stream.set_write_timeout(wait)?;
        stream.set_read_timeout(wait)?;
        stream.write_all(line.as_bytes())?;
        Ok(stream)
    });
    if let Ok(stream) = sent {
        let mut answer = String::new();
        if BufReader::new(stream).read_line(&mut answer).is_err() || answer != "ok\n" {
            eprintln!("priory hook: the daemon took the signal but has not applied it yet");
        }
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
