use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

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

/// The commit `branch` points to in the repository of `worktree`.
pub fn head(worktree: &Path, branch: &str) -> Option<String> {
    let commit = format!("refs/heads/{branch}^{{commit}}");
    run(git(worktree).args(["rev-parse", "--verify", "--quiet", &commit]))
        .ok()
        .map(|head| head.trim().to_owned())
}

/// Pushes `commit` from `worktree` to `branch` on its `origin`, then reuses the pull request open
/// from that branch on `repo`, or opens one against the remote's default branch. Returns its number.
pub fn open_pull_request(
    worktree: &Path,
    repo: &str,
    branch: &str,
    commit: &str,
    title: &str,
    body: &str,
) -> Result<u64, String> {
    let target = format!("{commit}:refs/heads/{branch}");
    run(git(worktree).args(["push", "--quiet", "origin", &target]))?;
    let open = run(Command::new("gh").args([
        "pr", "list", "-R", repo, "--head", branch, "--state", "open", "--json", "number",
    ]))?;
    let open: Vec<serde_json::Value> = serde_json::from_str(&open)
        .map_err(|error| format!("gh pr list printed {open:?}: {error}"))?;
    if let Some(number) = open.first().and_then(|pull| pull["number"].as_u64()) {
        return Ok(number);
    }
    let base = run(git(worktree).args(["rev-parse", "--abbrev-ref", "origin/HEAD"]))?;
    let base = base.trim().trim_start_matches("origin/");
    let url = run(Command::new("gh").args([
        "pr", "create", "-R", repo, "--base", base, "--head", branch, "--title", title, "--body",
        body,
    ]))?;
    url.trim()
        .rsplit('/')
        .next()
        .and_then(|number| number.parse().ok())
        .ok_or_else(|| format!("gh pr create printed {url:?}, not a pull request URL"))
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
    /// Continue `session`, which an earlier run of the task started, instead of creating it.
    pub resume: bool,
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
        .args(["--", "claude"])
        .arg(if launch.resume {
            "--resume"
        } else {
            "--session-id"
        })
        .arg(launch.session);
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
    command.arg("--mcp-config").arg(mcp_config(&exe, launch));
    command.args([";", "set-option", "-p", "@priory", launch.session]);
    run(&mut command).map(drop)
}

/// The MCP server that serves Priory's tools to the run: `priory mcp --session <id>`.
fn mcp_config(exe: &Path, launch: &Launch) -> String {
    serde_json::json!({ "mcpServers": { "priory": {
        "command": exe,
        "args": ["mcp", "--session", launch.session],
        "env": { "PRIORY_HOME": launch.home },
    } } })
    .to_string()
}

const ASK_INSTEAD: &str = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Priory runs this session unattended. Call the ask tool of the priory MCP server instead."}}"#;

const DENY: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Priory runs this session unattended and never approves a permission prompt, so it denied this one. Go on without it. If the task cannot go on without it, call the ask or result tool of the priory MCP server."}}}"#;

/// Claude Code settings whose hooks call `priory hook <event>`, that allow Priory's tools, deny the
/// built-in question tool in favour of `ask` (R6.4), deny every permission request (R7.2), and
/// report a permission prompt that shows anyway.
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
    let mut hooks = hooks;
    hooks.insert(
        "PreToolUse".to_owned(),
        serde_json::json!([{
            "matcher": "AskUserQuestion",
            "hooks": [{ "type": "command", "command": format!("printf '%s' '{ASK_INSTEAD}'") }],
        }]),
    );
    hooks.insert(
        "PermissionRequest".to_owned(),
        serde_json::json!([{
            "hooks": [{ "type": "command", "command": format!("printf '%s' '{DENY}'") }],
        }]),
    );
    hooks.insert(
        "Notification".to_owned(),
        serde_json::json!([{
            "matcher": "permission_prompt",
            "hooks": [{ "type": "command", "command": format!("{quoted} hook PermissionPrompt") }],
        }]),
    );
    serde_json::json!({ "hooks": hooks, "permissions": { "allow": ["mcp__priory"] } }).to_string()
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
            if let Ok(reply) = done.recv() {
                let _ = stream.write_all(format!("{reply}\n").as_bytes());
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

/// Sends one line to the daemon and returns its reply. Nothing is spooled: with no daemon, it fails.
pub fn call(home: &Path, line: &str) -> Result<String, String> {
    let wait = Some(Duration::from_secs(10));
    let path = socket(home);
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("no priory daemon listens on {}: {error}", path.display()))?;
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
    let sent = UnixStream::connect(socket(home)).and_then(|mut stream| {
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
    fs::create_dir_all(home).map_err(|error| error.to_string())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(spool(home))
        .and_then(|mut file| file.write_all(line.as_bytes()))
        .map_err(|error| format!("{}: {error}", spool(home).display()))
}
