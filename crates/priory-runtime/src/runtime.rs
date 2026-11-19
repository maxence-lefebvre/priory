use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::json;

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
pub(crate) fn run(command: &mut Command) -> Result<String, String> {
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

pub(crate) fn git(dir: &Path) -> Command {
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
    // A revived task whose worktree the sweep removed carries on from the branch it left.
    if head(&clone, &branch).is_some() {
        run(git(&clone)
            .args(["worktree", "add", "--quiet"])
            .arg(&path)
            .arg(&branch))?;
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

/// The last commit that changed the files under `dir`, or `None` outside git or before any commit.
pub fn revision(dir: &Path) -> Option<String> {
    run(git(dir).args(["log", "-1", "--format=%H", "--", "."]))
        .ok()
        .map(|commit| commit.trim().to_owned())
        .filter(|commit| !commit.is_empty())
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

fn tmux() -> Command {
    Command::new("tmux")
}

/// What a Claude Code session sets for its children. User settings such as `CLAUDE_CODE_USE_BEDROCK` stay.
const NESTED: [&str; 12] = [
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
    /// The agent's Claude Code `permissions` settings, merged into Priory's own.
    pub permissions: serde_json::Value,
    /// The agent's `defaultMode`, or `manual`.
    pub permission_mode: &'a str,
}

/// A coding agent CLI that Priory runs in tmux, with hooks and tools that report to the daemon.
pub trait Harness {
    /// The program's name, as a person types it.
    fn program(&self) -> &'static str;
    /// The program's arguments for `launch`, whose hooks and MCP server run `exe`.
    fn args(&self, exe: &Path, launch: &Launch) -> Result<Vec<String>, String>;
    /// What a person does once, in `clone`, before the harness runs Priory's hooks there.
    fn trust(&self, exe: &Path, clone: &str) -> String;
}

pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn program(&self) -> &'static str {
        "claude"
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
        if !launch.body.is_empty() {
            args.extend(["--append-system-prompt".to_owned(), launch.body.to_owned()]);
        }
        // Without the flag, the user's own mode, such as auto, would decide the run's prompts (R7.2).
        args.extend([
            "--permission-mode".to_owned(),
            launch.permission_mode.to_owned(),
        ]);
        args.extend(["--settings".to_owned(), settings(exe, &launch.permissions)]);
        args.extend(["--mcp-config".to_owned(), mcp_config(exe, launch)]);
        Ok(args)
    }

    fn trust(&self, _exe: &Path, clone: &str) -> String {
        format!("trust the clone once: cd {clone} && claude, accept, exit")
    }
}

pub struct Codex;

impl Harness for Codex {
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
                    "args": ["mcp", "--session", launch.session],
                    "env": { "PRIORY_HOME": launch.home },
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
        if !launch.body.is_empty() {
            config.push(("developer_instructions", json!(launch.body)));
        }
        let mut args = match launch.resume {
            Some(thread) => vec!["resume".to_owned(), thread.to_owned()],
            None => Vec::new(),
        };
        for (key, value) in config {
            args.extend(["-c".to_owned(), override_of(key, value)?]);
        }
        Ok(args)
    }

    /// Codex runs only hooks a person trusted in its `/hooks` review, by their hash, so Priory's
    /// hooks never change between runs and this command shows the same ones.
    fn trust(&self, exe: &Path, clone: &str) -> String {
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
        (
            "hooks.PreToolUse",
            hook(respond(ASK_INSTEAD), Some("^request_user_input$")),
        ),
        ("hooks.PermissionRequest", hook(respond(DENY), None)),
    ]
}

/// Starts the harness in a detached tmux session, with hooks that report to this daemon. The same
/// tmux command tags the new pane with the run's session id, so a crash cannot leave it untagged.
pub fn launch(harness: &dyn Harness, launch: &Launch) -> Result<(), String> {
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
        .arg("-e")
        .arg(format!("PRIORY_SESSION={}", launch.session))
        .args(["--", "env"])
        .args(NESTED.iter().flat_map(|name| ["-u", name]))
        .arg(harness.program())
        .args(harness.args(&exe, launch)?);
    command.args([";", "set-option", "-p", "@priory", launch.session]);
    run(&mut command).map(drop)
}

/// The MCP server that serves Priory's tools to the run: `priory mcp --session <id>`.
fn mcp_config(exe: &Path, launch: &Launch) -> String {
    json!({ "mcpServers": { "priory": {
        "command": exe,
        "args": ["mcp", "--session", launch.session],
        "env": { "PRIORY_HOME": launch.home },
    } } })
    .to_string()
}

fn quote(exe: &Path) -> String {
    format!("'{}'", exe.display().to_string().replace('\'', r"'\''"))
}

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
        json!([{
            "matcher": "AskUserQuestion",
            "hooks": [{ "type": "command", "command": format!("printf '%s' '{ASK_INSTEAD}'") }],
        }]),
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
