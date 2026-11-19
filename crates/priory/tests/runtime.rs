use herdr_bus::{Bus, Message, Output};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output as Exit};
use std::time::{Duration, Instant};

const WORKFLOW: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\nmodel = \"haiku\"\neffort = \"low\"\npermissions = { allow = [\"Bash(cargo test:*)\"], deny = [\"Bash(git push:*)\"] }\n+++\nYou implement one issue.\n";

const REVIEWER: &str = "+++\ndescription = \"Reviews one change.\"\nharness = \"claude-code\"\n+++\nYou review one change.\n";

const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
"#;

const OUTPUT: &str = "factory.intake";

/// A factory, bus, state, Priory home, tmux server, origin repository and fake `claude`, all under
/// the test's own directory. Dropping it kills the daemon and the tmux server.
struct Setup {
    dir: PathBuf,
    daemon: Option<Child>,
}

impl Setup {
    fn new(name: &str) -> Self {
        Self::with_workflow(name, WORKFLOW)
    }

    fn with_workflow(name: &str, workflow: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rt").join(name);
        let setup = Setup { dir, daemon: None };
        setup.kill_tmux();
        let _ = fs::remove_dir_all(&setup.dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/agents/reviewer/agent.md", REVIEWER),
            ("factory/workflows/implement.toml", workflow),
            ("factory/triggers/agent-label.toml", TRIGGER),
        ];
        for (path, content) in files {
            let file = setup.dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        fs::create_dir_all(setup.dir.join("fake")).unwrap();
        fs::create_dir_all(setup.dir.join("tmux")).unwrap();
        fs::create_dir_all(setup.dir.join("proc/sys/kernel")).unwrap();
        fs::write(setup.dir.join("proc/sys/kernel/hostname"), "runner\n").unwrap();
        setup.memory(60);
        setup.load(0.0);
        setup.origin();
        setup.bus().declare(&Output::new(OUTPUT)).unwrap();
        // A new state file switches to WAL on first open, which does not wait for a concurrent opener.
        setup.rows("tasks");
        setup
    }

    /// A bare `owner/repo` with one commit on `main`, which git's `insteadOf` serves for GitHub.
    fn origin(&self) {
        let work = self.dir.join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join("README.md"), "demo\n").unwrap();
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["add", "README.md"],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ],
        ] {
            assert!(git(&work).args(args).status().unwrap().success());
        }
        let bare = self.dir.join("origin/owner/repo.git");
        assert!(
            git(&self.dir)
                .args(["clone", "-q", "--bare"])
                .arg(&work)
                .arg(&bare)
                .status()
                .unwrap()
                .success()
        );
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    fn publish(&self, message: &Message) {
        self.bus().publish(OUTPUT, message).unwrap();
    }

    fn command(&self, program: &str) -> Command {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake");
        let path = std::env::join_paths(
            std::iter::once(fake).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut command = Command::new(program);
        command
            .env_remove("TMUX")
            .env("PATH", path)
            .env("TMUX_TMPDIR", self.dir.join("tmux"))
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("FAKE_DIR", self.dir.join("fake"))
            .env("PRIORY_PROC", self.dir.join("proc"))
            .env("GIT_CONFIG_COUNT", "1")
            .env(
                "GIT_CONFIG_KEY_0",
                format!(
                    "url.file://{}/.insteadOf",
                    self.dir.join("origin").display()
                ),
            )
            .env("GIT_CONFIG_VALUE_0", "https://github.com/");
        command
    }

    fn priory(&self, args: &[&str]) -> Exit {
        self.command(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .output()
            .unwrap()
    }

    fn tmux(&self, args: &[&str]) -> Exit {
        self.command("tmux").args(args).output().unwrap()
    }

    fn kill_tmux(&self) {
        let _ = self.tmux(&["kill-server"]);
    }

    fn start(&mut self) {
        self.start_as(Path::new(env!("CARGO_BIN_EXE_priory")));
    }

    /// Starts the daemon through the path `exe`, as a person or a service invokes it.
    fn start_as(&mut self, exe: &Path) {
        let factory = self.dir.join("factory");
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        self.daemon = Some(
            self.command(exe.to_str().unwrap())
                .args(["daemon", factory.to_str().unwrap()])
                .env("CLAUDECODE", "1")
                .env("CLAUDE_CODE_ENTRYPOINT", "cli")
                .env("CLAUDE_CODE_USE_BEDROCK", "0")
                .stderr(log)
                .spawn()
                .unwrap(),
        );
    }

    /// Kills the daemon without letting it clean up, as a crash does.
    fn crash(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.kill().unwrap();
            daemon.wait().unwrap();
        }
    }

    /// The rows `priory <verb>` prints, each split on tabs, after its header.
    fn rows(&self, verb: &str) -> Vec<Vec<String>> {
        let output = self.priory(&[verb]);
        assert!(
            output.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            // The desk's stall banner, before its header, has no tab.
            .skip_while(|line| !line.contains('\t'))
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }

    /// The banner line `priory desk` starts with, if any.
    fn banner(&self) -> Option<String> {
        let output = self.priory(&["desk"]);
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .next()
            .filter(|line| !line.contains('\t'))
            .map(str::to_owned)
    }

    /// Polls the desk's banner until `done` holds for it, and returns it.
    fn until_banner(&self, done: impl Fn(Option<&str>) -> bool) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let banner = self.banner();
            if done(banner.as_deref()) {
                return banner;
            }
            assert!(
                Instant::now() < deadline,
                "timed out on the desk banner: {banner:?}\ndaemon: {}",
                fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Polls `priory <verb>` until `done` holds for its rows, and returns them.
    fn until(&self, verb: &str, done: impl Fn(&[Vec<String>]) -> bool) -> Vec<Vec<String>> {
        self.within(15, verb, done)
    }

    fn within(
        &self,
        seconds: u64,
        verb: &str,
        done: impl Fn(&[Vec<String>]) -> bool,
    ) -> Vec<Vec<String>> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        loop {
            let rows = self.rows(verb);
            if done(&rows) {
                return rows;
            }
            assert!(
                Instant::now() < deadline,
                "timed out on `priory {verb}`: {rows:?}\ndaemon: {}\noutbox: {:?}\nfake: {:?}",
                fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default(),
                self.priory(&["outbox"]).stdout.escape_ascii().to_string(),
                fs::read_dir(self.dir.join("fake"))
                    .map(|dir| dir
                        .flatten()
                        .map(|entry| entry.file_name())
                        .collect::<Vec<_>>())
                    .unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The prompts the fake `claude` received, in order.
    fn typed(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join("fake/typed.log"))
            .unwrap_or_default()
            .split_terminator("---\n")
            .map(str::to_owned)
            .collect()
    }

    /// Waits until the file `path`, under the test's directory, has content.
    fn written(&self, path: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while fs::metadata(self.dir.join(path)).map_or(true, |file| file.len() == 0) {
            assert!(Instant::now() < deadline, "{path} never appeared");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Queues tool calls for the fake `claude`: one per turn, in order. An empty one skips a turn.
    fn calls(&self, calls: &[serde_json::Value]) {
        let lines: String = calls
            .iter()
            .map(|call| match call {
                serde_json::Value::Null => "\n".to_owned(),
                call => format!("{call}\n"),
            })
            .collect();
        fs::write(self.dir.join("fake/calls"), lines).unwrap();
    }

    /// The replies the fake `claude` got from its tool calls, one per line.
    fn replies(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.dir.join("fake/tools.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Sends `lines` to `priory mcp --session <session>` and returns its replies.
    fn mcp(&self, session: &str, lines: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let mut mcp = self
            .command(env!("CARGO_BIN_EXE_priory"))
            .args(["mcp", "--session", session])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut mcp.stdin.take().unwrap(), input.as_bytes()).unwrap();
        String::from_utf8(mcp.wait_with_output().unwrap().stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Waits until the tmux session `name` is gone.
    fn gone(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self
            .tmux(&["has-session", "-t", &format!("={name}")])
            .status
            .success()
        {
            assert!(Instant::now() < deadline, "{name} still runs");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Kills the daemon, the tmux server and the fake `claude`, as a reboot does.
    fn reboot(&mut self) {
        self.crash();
        let pid = fs::read_to_string(self.dir.join("fake/pid")).unwrap();
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        self.kill_tmux();
    }

    fn hold(&self, at: &str) {
        fs::write(self.dir.join("fake").join(at), "").unwrap();
    }

    fn release(&self, at: &str) {
        fs::remove_file(self.dir.join("fake").join(at)).unwrap();
    }

    /// Makes the daemon measure `available` GiB of free memory out of 64 GiB.
    fn memory(&self, available: u64) {
        let meminfo = format!(
            "MemTotal:       {} kB\nMemFree:        1024 kB\nMemAvailable:   {} kB\n",
            64 << 20,
            available << 20
        );
        write_atomic(&self.dir.join("proc/meminfo"), &meminfo);
    }

    /// Makes the daemon measure a 1-minute load average of `load`.
    fn load(&self, load: f64) {
        let loadavg = format!("{load:.2} 0.00 0.00 1/100 12345\n");
        write_atomic(&self.dir.join("proc/loadavg"), &loadavg);
    }

    /// Makes the daemon measure `free` GiB of disk free out of 1000 GiB.
    fn disk(&self, free: u64) {
        let df = format!(
            "Filesystem 1024-blocks Used Available Capacity Mounted on\nfake {} {} {} 50% /\n",
            1000 << 20,
            (1000 - free) << 20,
            free << 20
        );
        write_atomic(&self.dir.join("fake/df"), &df);
    }
}

/// Writes through a rename, so the daemon never reads a half-written file.
fn write_atomic(path: &Path, content: &str) {
    let partial = path.with_extension("partial");
    fs::write(&partial, content).unwrap();
    fs::rename(partial, path).unwrap();
}

impl Drop for Setup {
    fn drop(&mut self) {
        self.crash();
        self.kill_tmux();
    }
}

fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dir);
    command
}

fn labeled(issue: u64) -> Message {
    Message::new(
        "github.issues.labeled",
        json!({ "repo": "owner/repo", "issue": issue, "label": "agent", "github_id": 100 + issue }),
    )
    .subject(format!("owner/repo#{issue}"))
}

fn requested(issue: u64) -> Message {
    requested_in(issue, None)
}

fn requested_in(issue: u64, band: Option<&str>) -> Message {
    let key = format!("github:owner/repo#{issue}");
    Message::new(
        "factory.task.requested",
        json!({
            "workflow": "implement",
            "key": key,
            "band": band,
            "lane": null,
            "trigger": "agent-label",
            "source": "source"
        }),
    )
    .subject(key)
}

/// Rows whose column `column` is `value`.
fn with(rows: &[Vec<String>], column: usize, value: &str) -> usize {
    rows.iter().filter(|row| row[column] == value).count()
}

#[test]
fn a_queued_task_at_a_model_step_gets_a_worktree_a_tmux_session_and_a_run() {
    let mut setup = Setup::new("start");
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let runs = setup.rows("runs");
    let [id, task, step, tmux, session, _started, ended, end] = &runs[0][..] else {
        panic!("runs: {runs:?}");
    };
    assert_eq!(
        [id, task, step, tmux, ended, end].map(String::as_str),
        ["1", "1", "implement", "priory-1", "-", "-"]
    );
    assert!(uuid::Uuid::parse_str(session).is_ok(), "session {session}");
    assert!(
        setup
            .tmux(&["has-session", "-t", "priory-1"])
            .status
            .success()
    );
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    let branch = git(&worktree)
        .args(["branch", "--show-current"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(branch.stdout).unwrap(), "priory/1\n");
    assert!(worktree.join("README.md").is_file());
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    let args: Vec<_> = args.lines().collect();
    for pair in [
        ["--session-id", session.as_str()],
        ["--model", "haiku"],
        ["--effort", "low"],
        ["--append-system-prompt", "You implement one issue."],
        ["--permission-mode", "manual"],
    ] {
        assert!(args.windows(2).any(|w| w == pair), "{pair:?} in {args:?}");
    }
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn an_agent_in_auto_mode_gets_a_run_in_auto_mode_and_its_rules_join_priorys_own() {
    let mut setup = Setup::new("permissions");
    let agent = AGENT.replacen(
        "permissions = { ",
        "permissions = { defaultMode = \"auto\", ",
        1,
    );
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), agent).unwrap();
    let source = labeled(42);
    let mut request = requested(42);
    request.data["source"] = json!(source.id.to_string());
    for message in [&source, &request] {
        setup.publish(message);
    }

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let args = launched(&setup);
    let after = |flag: &str| -> Vec<String> {
        args.windows(2)
            .filter(|w| w[0] == flag)
            .map(|w| w[1].clone())
            .collect()
    };
    assert_eq!(after("--permission-mode"), ["auto"]);
    let settings: serde_json::Value = serde_json::from_str(&after("--settings")[0]).unwrap();
    assert_eq!(
        settings["permissions"],
        json!({
            "allow": ["mcp__priory", "Bash(cargo test:*)"],
            "deny": ["Bash(git push:*)"],
        })
    );
    assert!(settings["hooks"]["PermissionRequest"].is_array());
}

const CODEX: &str = "+++\ndescription = \"Reviews one change.\"\nharness = \"codex\"\nmodel = \"gpt-6-astra\"\neffort = \"low\"\n+++\nYou review one change.\n";

/// The `-c` overrides the fake `codex` got at its latest launch, merged into one table.
fn codex_config(setup: &Setup) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(setup.dir.join("fake/codex-config.json")).unwrap())
        .unwrap()
}

fn codex_args(setup: &Setup) -> Vec<String> {
    fs::read_to_string(setup.dir.join("fake/codex-args.log"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_codex_agent_runs_codex_with_priorys_hooks_mcp_server_and_a_sandbox_that_asks() {
    let mut setup = Setup::new("codex-start");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.publish(&requested(42));

    setup.start();

    // A nudge may follow the silent first turn before the poll sees it alone.
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let args = codex_args(&setup);
    assert_ne!(args.first().map(String::as_str), Some("resume"), "{args:?}");
    assert!(
        !args.iter().any(|arg| arg.contains("bypass")),
        "Priory never skips Codex's hook trust: {args:?}"
    );
    let config = codex_config(&setup);
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    assert_eq!(
        config["mcp_servers"]["priory"],
        json!({
            "command": exe,
            "args": ["mcp", "--session", tmux_tag(&setup)],
            "env": { "PRIORY_HOME": setup.dir.join("home") },
            "default_tools_approval_mode": "approve",
        })
    );
    for (key, value) in [
        ("model", "gpt-6-astra"),
        ("model_reasoning_effort", "low"),
        ("developer_instructions", "You review one change."),
        ("sandbox_mode", "read-only"),
        ("approval_policy", "on-request"),
        ("approvals_reviewer", "user"),
    ] {
        assert_eq!(config[key], value, "{key} in {config}");
    }
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "Stop",
        "PermissionRequest",
        "PreToolUse",
    ] {
        assert!(config["hooks"][event].is_array(), "{event} in {config}");
    }
    let hooks = config["hooks"].to_string();
    assert!(
        !hooks.contains(&tmux_tag(&setup)),
        "a person trusts the hooks once, so no run's session is in them: {hooks}"
    );
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_codex_run_starts_on_its_own_with_the_steps_prompt_as_its_last_argument() {
    // Longer than the 16 KiB tmux takes in one command.
    let long = "context ".repeat(2500);
    let workflow = WORKFLOW.replace(
        "prompt = \"Implement the issue.\"",
        &format!("prompt = '''\n- Read \"the issue\" and Max's notes.\n  Then implement it; $HOME stays literal.\n{long}\n'''"),
    );
    let mut setup = Setup::with_workflow("codex-prompt", &workflow);
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let prompt = format!(
        "- Read \"the issue\" and Max's notes.\n  Then implement it; $HOME stays literal.\n{long}\n\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed."
    );
    let args = codex_args(&setup);
    assert!(args.contains(&"--no-daemon".to_owned()), "{args:?}");
    assert_eq!(
        fs::read_to_string(setup.dir.join("fake/codex-prompt")).unwrap(),
        prompt
    );
    assert_eq!(setup.typed()[0], format!("{prompt}\n"));
}

#[test]
fn a_codex_run_with_no_session_start_names_the_codex_command_whose_hooks_to_trust() {
    let mut setup = Setup::new("codex-untrusted");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.publish(&requested(42));
    fs::write(
        setup.dir.join("fake/silent"),
        "  Hooks need review\n\n  3 hooks are new or changed\n  › 1. Review hooks\n",
    )
    .unwrap();
    setup.start();
    setup.until("runs", |rows| rows.len() == 1);

    let tasks = setup.within(75, "tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });

    let clone = setup.dir.join("home/repos/owner/repo");
    let reason = &tasks[0][7];
    let prefix = format!(
        "codex showed no SessionStart within 60s; trust the clone and Priory's hooks once: cd {} && codex -c ",
        clone.display()
    );
    assert!(reason.starts_with(&prefix), "{reason}");
    assert!(
        reason.ends_with(
            ", accept the folder, type /hooks and trust Priory's hooks, then exit. Its pane ended with: Hooks need review | 3 hooks are new or changed | › 1. Review hooks"
        ),
        "{reason}"
    );
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert!(log.contains("  3 hooks are new or changed\n"), "{log}");
    let outbox = setup.rows("outbox");
    assert_eq!(
        [outbox.len(), with(&outbox, 3, "queued")],
        [1, 1],
        "the next run gets the prompt this one never saw: {outbox:?}"
    );
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "Stop",
        "PreToolUse",
        "PermissionRequest",
    ] {
        assert!(
            reason.contains(&format!(" -c 'hooks.{event}=")),
            "{event} in {reason}"
        );
    }
}

#[test]
fn a_codex_run_names_priory_by_the_path_it_was_invoked_by_so_an_upgrade_keeps_hook_trust() {
    let mut setup = Setup::new("codex-stable-exe");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    let profile = setup.dir.join("profile/bin/priory");
    fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_priory"), &profile).unwrap();
    setup.publish(&requested(42));

    setup.start_as(&profile);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let config = codex_config(&setup);
    let hook = &config["hooks"]["SessionStart"][0]["hooks"][0]["command"];
    assert_eq!(
        hook.as_str().unwrap(),
        format!("'{}' hook SessionStart", profile.display())
    );
    assert_eq!(config["mcp_servers"]["priory"]["command"], json!(profile));
}

/// The `@priory` tag of task 1's pane: the id Priory gave its run, which its tools carry too.
fn tmux_tag(setup: &Setup) -> String {
    let out = setup.tmux(&["list-panes", "-s", "-t", "=priory-1:", "-F", "#{@priory}"]);
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn a_codex_run_stores_the_thread_codex_chose_and_an_answer_resumes_it() {
    let mut setup = Setup::with_workflow("codex-resume", STEPS);
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    let thread = fs::read_to_string(setup.dir.join("fake/thread")).unwrap();
    assert_eq!(
        runs[0][4], thread,
        "priory runs shows the thread Codex chose"
    );
    setup.gone("priory-1");

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let args = codex_args(&setup);
    assert_eq!(args[..2], ["resume", thread.as_str()], "{args:?}");
    assert!(args.contains(&"--no-daemon".to_owned()), "{args:?}");
    assert!(
        fs::read_to_string(setup.dir.join("fake/codex-prompt"))
            .unwrap()
            .starts_with("sqlite\n")
    );
    let runs = setup.rows("runs");
    assert_eq!([&runs[1][2], &runs[1][4]], ["plan", &thread]);
    assert!(
        setup.typed()[1].starts_with("sqlite\n"),
        "{:?}",
        setup.typed()
    );
}

#[test]
fn a_codex_run_carried_into_the_next_step_keeps_its_thread() {
    let mut setup = Setup::with_workflow("codex-carry", STEPS);
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.calls(&[result("ready")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let thread = fs::read_to_string(setup.dir.join("fake/thread")).unwrap();
    let runs = setup.rows("runs");
    assert_eq!([&runs[0][2], &runs[0][4]], ["plan", &thread]);
    assert_eq!([&runs[1][2], &runs[1][4]], ["implement", &thread]);
}

#[test]
fn a_workflow_with_a_claude_step_then_a_codex_step_runs_to_done() {
    let mut setup = Setup::with_workflow("mixed", TWO_AGENTS);
    fs::write(setup.dir.join("factory/agents/reviewer/agent.md"), CODEX).unwrap();
    setup.calls(&[result("passed"), result("approved")]);
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    assert_eq!(tasks[0][3], "review");
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!([&runs[0][2], &runs[0][7]], ["implement", "moved"]);
    assert_eq!([&runs[1][2], &runs[1][7]], ["review", "done"]);
    let thread = fs::read_to_string(setup.dir.join("fake/thread")).unwrap();
    assert_eq!(runs[1][4], thread);
    assert_ne!(runs[0][4], thread);
    assert_eq!(
        codex_config(&setup)["developer_instructions"],
        "You review one change."
    );
    let typed = setup.typed();
    assert_eq!(
        typed[1],
        format!("{}\n", prompted("Review the change.", "approved, changes"))
    );
    let replies = setup.replies();
    assert!(replies.iter().all(|r| !reply(r).1), "{replies:?}");
}

#[test]
fn a_run_starts_without_the_variables_of_the_claude_session_that_started_the_daemon() {
    let mut setup = Setup::new("nested-env");
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let env = fs::read_to_string(setup.dir.join("fake/env.log")).unwrap();
    let names: Vec<_> = env.lines().collect();
    assert!(names.contains(&"PRIORY_HOME"), "{names:?}");
    let session = [
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
    let leaked: Vec<_> = session.iter().filter(|name| names.contains(name)).collect();
    assert!(leaked.is_empty(), "{leaked:?} leaked: {names:?}");
    assert!(names.contains(&"CLAUDE_CODE_USE_BEDROCK"), "{names:?}");
}

#[test]
fn the_first_prompt_is_the_step_prompt_and_the_task_events_typed_once_and_confirmed() {
    let mut setup = Setup::new("first-prompt");
    let source = labeled(42);
    let mut request = requested(42);
    request.data["source"] = json!(source.id.to_string());
    let mut kept = labeled(42);
    kept.data["label"] = json!("it's \"urgent\"; C-c Enter");
    for message in [&source, &request, &kept] {
        setup.publish(message);
    }

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
    let lines: Vec<_> = typed[0].lines().collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0], "Implement the issue.");
    assert_eq!(
        lines[1],
        "End this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed."
    );
    assert!(lines[2].starts_with("github.issues.labeled owner/repo#42 {"));
    assert!(lines[2].contains(r#""label":"agent""#));
    assert!(lines[3].contains(r#""label":"it's \"urgent\"; C-c Enter""#));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.typed().len(), 2, "the silent turn gets one nudge");
    assert!(setup.typed()[1].starts_with("Your turn ended"));
}

#[test]
fn an_event_kept_while_the_run_is_running_is_typed_after_the_next_stop() {
    let mut setup = Setup::new("message");
    setup.publish(&requested(42));
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    setup.publish(&labeled(42));

    setup.until("outbox", |rows| rows.len() == 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("outbox")[1][3], "queued");
    assert_eq!(setup.typed().len(), 1);
    setup.release("hold-stop");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(typed[1].starts_with("Your turn ended"), "{typed:?}");
    assert!(
        typed[1].contains("\ngithub.issues.labeled owner/repo#42 {"),
        "{typed:?}"
    );
}

#[test]
fn a_restarted_daemon_reattaches_to_its_run_and_types_nothing_twice() {
    let mut setup = Setup::new("restart");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let runs = setup.rows("runs");

    setup.crash();
    setup.start();
    setup.publish(&labeled(42));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert_eq!(setup.rows("runs"), runs);
    let typed = setup.typed();
    assert!(typed[1].starts_with("Your turn ended"), "{typed:?}");
    assert!(typed[2].starts_with("github.issues.labeled"), "{typed:?}");
    assert_eq!(
        typed
            .iter()
            .filter(|prompt| prompt.contains("labeled"))
            .count(),
        1
    );
}

#[test]
fn a_signal_the_daemon_dies_holding_is_spooled_for_the_next_daemon() {
    let setup = Setup::new("dying");
    fs::create_dir_all(setup.dir.join("home")).unwrap();
    let listener =
        std::os::unix::net::UnixListener::bind(setup.dir.join("home/priory.sock")).unwrap();
    // A daemon killed after it accepted the line closes the connection with no reply.
    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(&stream), &mut line).unwrap();
    });

    let mut hook = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["hook", "Stop"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut hook.stdin.take().unwrap(), br#"{"session_id":"s1"}"#).unwrap();
    assert!(hook.wait().unwrap().success());
    daemon.join().unwrap();

    let spool = fs::read_to_string(setup.dir.join("home/signals.spool")).unwrap_or_default();
    assert_eq!(spool.lines().count(), 1, "{spool:?}");
}

#[test]
fn a_daemon_started_while_a_dead_daemons_lock_lingers_waits_for_it() {
    let mut setup = Setup::new("lingering-lock");
    fs::create_dir_all(setup.dir.join("home")).unwrap();
    let lock = fs::File::create(setup.dir.join("home/daemon.lock")).unwrap();
    lock.lock().unwrap();

    setup.publish(&requested(42));
    setup.start();
    std::thread::sleep(Duration::from_millis(1500));
    drop(lock);

    setup.until("runs", |rows| rows.len() == 1);
}

#[test]
fn a_submitted_prompt_with_text_after_its_pasted_content_tag_is_not_confirmed() {
    let mut setup = Setup::new("trailer");
    fs::write(setup.dir.join("fake/trailer"), " Wait for approval.").unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.written("fake/typed.log");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("outbox")[0][3], "attempted");
}

#[test]
fn a_prompt_attempted_but_not_confirmed_before_a_crash_becomes_uncertain_and_is_not_typed_again() {
    let mut setup = Setup::new("uncertain");
    setup.publish(&requested(42));
    setup.hold("hold-submit");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");

    setup.crash();
    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    setup.release("hold-submit");
    setup.written("fake/typed.log");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.typed().len(), 1);
    assert_eq!(setup.rows("outbox")[0][3], "uncertain");
}

#[test]
fn a_signal_sent_while_the_daemon_is_down_is_applied_when_it_comes_back() {
    let mut setup = Setup::new("spool");
    setup.publish(&requested(42));
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    setup.crash();
    setup.release("hold-stop");
    setup.written("home/signals.spool");
    setup.publish(&labeled(42));
    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
}

#[test]
fn a_run_whose_tmux_session_dies_ends_exited_and_blocks_its_task() {
    let mut setup = Setup::new("exited");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );

    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "exited")
    });
    assert_ne!(runs[0][6], "-");
    let tasks = setup.rows("tasks");
    assert_eq!(tasks[0][4], "blocked");
    assert_eq!(
        tasks[0][7],
        "run 1 exited: claude no longer runs in tmux session priory-1"
    );
}

#[test]
fn a_derived_lane_is_derived_again_when_the_first_run_gives_the_task_a_branch() {
    let mut setup = Setup::new("lane-branch");
    fs::write(
        setup.dir.join("factory/lanes.toml"),
        "fallback = \"{team}/misc\"\n[[rules]]\nbranch = \"priory/*\"\nlane = \"FND/desktop\"\n",
    )
    .unwrap();
    setup.publish(&requested(42));
    assert!(
        setup
            .priory(&[
                "daemon",
                setup.dir.join("factory").to_str().unwrap(),
                "--once"
            ])
            .status
            .success()
    );
    assert_eq!(setup.rows("tasks")[0][6], "none/misc");

    setup.start();

    setup.until("tasks", |rows| {
        rows[0][4] == "running" && rows[0][6] == "FND/desktop"
    });
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );
    setup.until("tasks", |rows| rows[0][4] == "blocked");
    let desk = setup.priory(&["desk"]);
    let desk = String::from_utf8(desk.stdout).unwrap();
    let mut lines = desk.lines();
    assert_eq!(
        lines.next(),
        Some("since\tkind\tid\ttask\tlane\twhat\tgeneration")
    );
    let cells: Vec<_> = lines.next().unwrap().split('\t').collect();
    assert_eq!(cells[1..5], ["blocked", "1", "1", "FND/desktop"]);
}

#[test]
fn a_derived_lane_stays_when_the_task_runs_again_after_lanes_toml_changed() {
    let mut setup = Setup::new("lane-once");
    let lanes = setup.dir.join("factory/lanes.toml");
    fs::write(
        &lanes,
        "[[rules]]\nbranch = \"priory/*\"\nlane = \"FND/desktop\"\n",
    )
    .unwrap();
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert_eq!(setup.rows("tasks")[0][6], "FND/desktop");
    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );
    setup.until("tasks", |rows| rows[0][4] == "blocked");
    fs::write(
        &lanes,
        "[[rules]]\nbranch = \"priory/*\"\nlane = \"OPS/other\"\n",
    )
    .unwrap();
    setup.crash();
    setup.start();

    let deadline = Instant::now() + Duration::from_secs(10);
    while !setup.priory(&["resume", "1"]).status.success() {
        assert!(
            Instant::now() < deadline,
            "the daemon never took the resume"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    setup.until("runs", |rows| rows.len() == 2);
    assert_eq!(setup.rows("tasks")[0][6], "FND/desktop");
}

#[test]
fn a_lane_the_request_set_stays_when_the_first_run_gives_the_task_a_branch() {
    let mut setup = Setup::new("lane-kept");
    fs::write(
        setup.dir.join("factory/lanes.toml"),
        "[[rules]]\nbranch = \"priory/*\"\nlane = \"FND/desktop\"\n",
    )
    .unwrap();
    let mut request = requested(42);
    request.data["lane"] = json!("OPS/chores");
    setup.publish(&request);

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert_eq!(setup.rows("tasks")[0][6], "OPS/chores");
}

#[test]
fn a_run_whose_agent_exits_mid_turn_ends_exited_and_blocks_its_task() {
    let mut setup = Setup::new("crash");
    setup.hold("crash");
    setup.publish(&requested(42));

    setup.start();

    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "exited")
    });
    assert_ne!(runs[0][6], "-");
    let tasks = setup.rows("tasks");
    assert_eq!(tasks[0][4], "blocked");
    assert_eq!(
        tasks[0][7],
        "run 1 exited: claude no longer runs in tmux session priory-1"
    );
    setup.gone("priory-1");
}

#[test]
fn a_run_that_ends_its_turn_with_a_result_or_an_ask_is_not_flagged_when_its_agent_exits() {
    let mut setup = Setup::with_workflow("crash-after-call", STEPS);
    setup.hold("crash");
    setup.calls(&[result("hopeless"), ask()]);
    setup.publish(&requested(42));

    setup.start();

    // The two runs share one fake `claude`, so the second task arrives after the first run ends.
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] != "-");
    setup.publish(&requested(43));
    let runs = setup.until("runs", |rows| rows.len() == 2 && rows[1][7] != "-");
    assert_eq!([&runs[0][7], &runs[1][7]], ["dropped", "asleep"]);
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][4], &tasks[1][4]], ["dropped", "asleep"]);
    setup.gone("priory-2");
}

#[test]
fn a_turn_longer_than_its_step_timeout_ends_the_run_and_blocks_its_task() {
    let mut setup = Setup::with_workflow(
        "timeout",
        &WORKFLOW.replace(
            "effect = \"reversible\"",
            "effect = \"reversible\"\ntimeout = \"2s\"",
        ),
    );
    setup.hold("hold-stop");
    setup.publish(&requested(42));

    setup.start();

    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "timeout")
    });
    assert_ne!(runs[0][6], "-");
    let tasks = setup.rows("tasks");
    assert_eq!(tasks[0][4], "blocked");
    assert_eq!(tasks[0][7], "run 1 timed out: its turn ran past 2s");
    setup.gone("priory-1");
}

#[test]
fn two_tasks_that_fit_both_start_without_waiting_for_each_other() {
    let mut setup = Setup::new("two-fit");
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    let tasks = setup.until("tasks", |rows| with(rows, 4, "running") == 2);
    assert_eq!([&tasks[0][7], &tasks[1][7]], ["-", "-"]);
    let runs = setup.rows("runs");
    assert_eq!(with(&runs, 6, "-"), 2, "two live runs: {runs:?}");
}

#[test]
fn a_task_that_does_not_fit_waits_with_its_reason_and_starts_when_room_appears() {
    let mut setup = Setup::new("no-room");
    setup.memory(8);
    setup.publish(&requested(42));

    setup.start();

    let reason = "waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";
    setup.until("tasks", |rows| rows.len() == 1 && rows[0][7] == reason);
    assert_eq!(setup.rows("tasks")[0][4], "queued");
    assert!(setup.rows("runs").is_empty());
    setup.memory(12);
    let tasks = setup.until("tasks", |rows| rows[0][4] == "running");
    assert_eq!(tasks[0][7], "-");
}

#[test]
fn two_admissions_in_one_tick_count_each_others_declared_memory() {
    let mut setup = Setup::new("one-tick");
    setup.memory(12);
    setup.hold("hold-stop");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    let reason = "waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";
    setup.until("tasks", |rows| rows.len() == 2 && rows[1][7] == reason);
    // The first run's promise outlives its SessionStart: it holds until the run ends.
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    std::thread::sleep(Duration::from_millis(1500));
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][4], &tasks[1][4]], ["running", "queued"]);
    assert_eq!(setup.rows("runs").len(), 1);
    setup.release("hold-stop");
    setup.until("tasks", |rows| {
        rows[0][4] == "done" && rows[1][4] == "running"
    });
}

#[test]
fn high_load_or_low_disk_holds_admission() {
    let mut setup = Setup::new("load-disk");
    setup.load(f64::NAN);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.len() == 1 && rows[0][7].starts_with("waits for room: cannot measure it: no load in ")
    });
    setup.load(100_000.0);

    setup.until("tasks", |rows| {
        rows.len() == 1 && rows[0][7].starts_with("waits for room: load 100000.00 on ")
    });
    setup.disk(10);
    setup.load(0.0);
    let reason = "waits for room: 10.0 GB of disk free, keeps 20.0 GB free";
    setup.until("tasks", |rows| rows[0][7] == reason);
    assert_eq!(setup.rows("tasks")[0][4], "queued");
    assert!(setup.rows("runs").is_empty());
    setup.disk(100);
    setup.until("tasks", |rows| rows[0][4] == "running");
}

#[test]
fn the_desk_starts_with_a_stall_banner_while_the_head_waits_for_room_and_drops_it_when_room_returns()
 {
    let mut setup = Setup::new("stall-banner");
    setup.disk(10);
    setup.publish(&requested(42));

    setup.start();

    let banner = setup.until_banner(|banner| banner.is_some()).unwrap();
    let (since, rest) = banner
        .strip_prefix("stalled since ")
        .and_then(|rest| rest.split_once(": "))
        .unwrap_or_else(|| panic!("{banner}"));
    assert!(since.ends_with('Z') && since.len() == 20, "{banner}");
    assert_eq!(rest, "short of disk by 10.0 GB; task 1 waits for room");
    // The stall's start survives a restart.
    setup.crash();
    setup.start();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.banner(), Some(banner));
    setup.disk(100);
    setup.until("tasks", |rows| rows[0][4] == "running");
    assert_eq!(setup.until_banner(|banner| banner.is_none()), None);
}

const SWEEP_SCRIPTS: &str = r#"name = "demo"

[sweep]
scripts = [
  { run = "echo once >> \"$FAKE_DIR/swept\"; echo freed 3 GB", when = "disk" },
  { run = "echo nope; exit 3", when = "disk" },
  { run = "sleep 30", when = "disk", timeout = "1s" },
  { run = "echo memory >> \"$FAKE_DIR/swept\"", when = "memory" },
]
"#;

#[test]
fn a_disk_stall_runs_the_disk_scripts_once_with_a_timeout_and_the_desk_shows_their_output() {
    let mut setup = Setup::new("stall-scripts");
    fs::write(setup.dir.join("factory/factory.toml"), SWEEP_SCRIPTS).unwrap();
    setup.disk(10);
    setup.publish(&requested(42));

    setup.start();

    let desk = setup.until("desk", |rows| {
        rows.iter()
            .filter(|row| row[1] == "sweep" && !row[5].ends_with(": running"))
            .count()
            == 3
    });
    let sweeps: Vec<_> = desk
        .iter()
        .filter(|row| row[1] == "sweep")
        .map(|row| [row[3].as_str(), row[5].as_str()])
        .collect();
    assert_eq!(
        sweeps,
        [
            [
                "1",
                "echo once >> \"$FAKE_DIR/swept\"; echo freed 3 GB: exit 0: freed 3 GB"
            ],
            ["1", "echo nope; exit 3: exit 3: nope"],
            ["1", "sleep 30: timed out after 1s"],
        ]
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        fs::read_to_string(setup.dir.join("fake/swept")).unwrap(),
        "once\n"
    );
    setup.disk(100);
    setup.until("tasks", |rows| rows[0][4] == "running");
    setup.until("desk", |rows| !rows.iter().any(|row| row[1] == "sweep"));
}

const SWEEPER: &str = r#"name = "demo"

[sweep]
after = "1s"
agent = "sweeper"
repo = "owner/repo"
scripts = [{ run = "echo nothing to free", when = "disk" }]
"#;

/// A factory with a sweeper agent, short of disk, and one task waiting for room.
fn stalled_with_sweeper(name: &str) -> Setup {
    let setup = Setup::new(name);
    fs::write(setup.dir.join("factory/factory.toml"), SWEEPER).unwrap();
    let agent = setup.dir.join("factory/agents/sweeper/agent.md");
    fs::create_dir_all(agent.parent().unwrap()).unwrap();
    fs::write(
        agent,
        "+++\ndescription = \"Frees room.\"\nharness = \"claude-code\"\n+++\nYou free room.\n",
    )
    .unwrap();
    setup.disk(10);
    setup.publish(&requested(42));
    setup
}

#[test]
fn a_stall_past_its_threshold_starts_one_sweeper_task_that_skips_the_room_check() {
    let mut setup = stalled_with_sweeper("sweeper-once");
    setup.hold("hold-stop");

    setup.start();

    // The sweeper takes the highest band, so it lists first.
    let tasks = setup.until("tasks", |rows| rows.len() == 2 && rows[0][4] == "running");
    assert_eq!(
        [&tasks[1][4], &tasks[0][1], &tasks[0][2], &tasks[0][6]],
        ["queued", "priory:sweep", "sweep", "priory/sweep"]
    );
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
    setup.written("fake/typed.log");
    let prompt = &setup.typed()[0];
    for part in [
        "priory.queue.stalled",
        r#""resource":"disk""#,
        r#""missing":"10.0 GB""#,
        r#""script":"echo nothing to free","status":"exit 0","output":"nothing to free""#,
    ] {
        assert!(prompt.contains(part), "{part} not in {prompt}");
    }
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(setup.rows("tasks").len(), 2);
}

#[test]
fn a_sweeper_that_cleaned_opens_an_issue_on_the_sweep_repository_once_a_person_approves() {
    let mut setup = stalled_with_sweeper("sweeper-issue");
    setup.calls(&[json!({ "name": "result", "arguments": {
        "outcome": "cleaned",
        "summary": "Sweep the Docker build cache\nRemoved 30 GB of build cache with docker builder prune.",
    } })]);

    setup.start();

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(asks[0][1], "2");
    assert!(asks[0][3].starts_with("approve-issue"), "{asks:?}");
    assert!(setup.gh().is_empty());
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.until("tasks", |rows| rows[0][4] == "done");
    let gh = setup.gh();
    assert_eq!(gh.len(), 2, "{gh:?}");
    assert_eq!(
        gh[0][..6],
        ["issue", "list", "-R", "owner/repo", "--state", "all"]
    );
    let (body, mark) = gh[1][7].rsplit_once("\n\n").unwrap();
    assert_eq!(
        gh[1][..7],
        [
            "issue",
            "create",
            "-R",
            "owner/repo",
            "--title",
            "Sweep the Docker build cache",
            "--body",
        ]
    );
    assert_eq!(
        body,
        "Sweep the Docker build cache\nRemoved 30 GB of build cache with docker builder prune."
    );
    assert!(
        mark.starts_with("Filed by Priory for event ") && gh[0][7].contains(mark),
        "{gh:?}"
    );
}

/// Two tasks on a disk with 30 GB free, which keeps 20 GB, for an implementer declaring `disk`.
fn two_on_disk(name: &str, disk: Option<&str>) -> Setup {
    let mut setup = Setup::new(name);
    if let Some(disk) = disk {
        let agent = AGENT.replace("effort", &format!("disk = \"{disk}\"\neffort"));
        fs::write(setup.dir.join("factory/agents/implementer/agent.md"), agent).unwrap();
    }
    setup.disk(30);
    setup.publish(&requested(42));
    setup.publish(&requested(43));
    setup.start();
    setup
}

#[test]
fn two_agents_declaring_disk_cannot_both_start_when_only_one_fits() {
    let setup = two_on_disk("disk-one-fits", Some("8GB"));

    let reason = "waits for room: 22.0 GB of disk free, needs 8.0 GB and keeps 20.0 GB free";
    let tasks = setup.until("tasks", |rows| rows.len() == 2 && rows[1][7] == reason);
    assert_eq!([&tasks[0][4], &tasks[1][4]], ["running", "queued"]);
    assert_eq!(
        setup.banner().as_deref(),
        Some(&*format!(
            "stalled since {}: short of disk by 6.0 GB; task 2 waits for room",
            setup.banner().unwrap()[14..34].to_owned()
        ))
    );
}

#[test]
fn a_head_that_declares_more_disk_than_the_runner_has_holds_back_no_one() {
    let mut setup = Setup::new("disk-never-fits");
    with_review(&setup, "4GB");
    // The disk is 1000 GB, so 2000 GB never fits.
    fs::write(
        setup.dir.join("factory/agents/implementer/agent.md"),
        AGENT.replace("effort", "disk = \"2000GB\"\neffort"),
    )
    .unwrap();
    setup.disk(500);
    setup.publish(&requested_in(42, Some("now")));
    setup.publish(&review_requested(43));

    setup.start();

    let tasks = setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "running");
    assert_eq!(tasks[0][4], "queued");
    assert_eq!(setup.banner(), None);
}

#[test]
fn an_agent_without_disk_reserves_none() {
    let setup = two_on_disk("disk-none", None);

    setup.until("tasks", |rows| with(rows, 4, "running") == 2);
}

#[test]
fn a_higher_band_starts_first_however_young_and_tasks_lists_the_queue_in_start_order() {
    let mut setup = Setup::new("band-order");
    // Room for one run: the second waits.
    setup.memory(12);
    setup.publish(&requested_in(42, Some("later")));
    setup.publish(&requested_in(43, None));
    setup.publish(&requested_in(44, Some("now")));

    setup.start();

    let tasks = setup.until("tasks", |rows| with(rows, 4, "running") == 1);
    let order: Vec<_> = tasks
        .iter()
        .map(|row| [&row[0], &row[4], &row[5], &row[8]])
        .collect();
    assert_eq!(
        order,
        [
            ["3", "running", "now", "-"],
            ["1", "queued", "later", "1"],
            ["2", "queued", "later", "2"],
        ]
    );
}

/// The rows of `priory tasks` as id, state, band and place in the queue, in the order it lists them.
fn queue(setup: &Setup) -> Vec<[String; 4]> {
    setup
        .rows("tasks")
        .into_iter()
        .map(|row| {
            [
                row[0].clone(),
                row[4].clone(),
                row[5].clone(),
                row[8].clone(),
            ]
        })
        .collect()
}

fn moved(setup: &Setup, args: &[&str]) {
    let output = setup.priory(&[&["move"], args].concat());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Routes what is pending without starting runs, so a test can order the tasks first.
fn route(setup: &Setup) {
    let factory = setup.dir.join("factory");
    let output = setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn a_band_comes_from_the_request_then_the_trigger_then_the_workflow() {
    let setup = Setup::new("band-source");
    let trigger = format!("{TRIGGER}band = \"today\"\n");
    fs::write(setup.dir.join("factory/triggers/agent-label.toml"), trigger).unwrap();
    let workflow = WORKFLOW.replacen("\n", "\nband = \"now\"\n", 1);
    fs::write(setup.dir.join("factory/workflows/implement.toml"), workflow).unwrap();
    let from_cli = |issue: u64, band: Option<&str>| {
        let mut message = requested_in(issue, band);
        message.data["trigger"] = json!("cli");
        message
    };
    setup.publish(&requested_in(42, Some("blocker")));
    setup.publish(&requested(43));
    setup.publish(&from_cli(44, None));
    // A band the factory lacks, as a message published before a rename carries.
    setup.publish(&from_cli(45, Some("todday")));

    route(&setup);

    let bands: Vec<_> = queue(&setup)
        .into_iter()
        .map(|[id, _, band, _]| format!("{id} {band}"))
        .collect();
    assert_eq!(bands, ["1 blocker", "3 now", "4 now", "2 today"]);
}

#[test]
fn a_task_from_before_bands_gets_the_last_band_and_keeps_its_age() {
    let setup = Setup::new("band-upgrade");
    setup.publish(&requested(42));
    route(&setup);
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute("UPDATE tasks SET band = NULL", []).unwrap();
    setup.publish(&requested(43));

    route(&setup);

    let bands: Vec<_> = queue(&setup)
        .into_iter()
        .map(|[id, _, band, place]| format!("{id} {band} {place}"))
        .collect();
    assert_eq!(bands, ["1 later 1", "2 later 2"]);
}

#[test]
fn a_person_order_inside_a_band_beats_age() {
    let mut setup = Setup::new("person-order");
    setup.memory(12);
    for issue in [42, 43, 44] {
        setup.publish(&requested(issue));
    }
    route(&setup);

    moved(&setup, &["3", "--top"]);
    moved(&setup, &["2", "--before", "1"]);
    setup.start();

    setup.until("tasks", |rows| with(rows, 4, "running") == 1);
    assert_eq!(
        queue(&setup),
        [
            ["3", "running", "later", "-"],
            ["2", "queued", "later", "1"],
            ["1", "queued", "later", "2"],
        ]
        .map(|row| row.map(str::to_owned))
    );
    let waits = "later: waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";
    let expected = [
        format!("queue 1 2 {waits}"),
        "queue 2 1 later: waits behind task 2, which waits for room".to_owned(),
    ];
    let queued = |rows: &[Vec<String>]| -> Vec<String> {
        rows.iter()
            .filter(|row| row[1] == "queue")
            .map(|row| {
                [&row[1], &row[2], &row[3], &row[5]]
                    .map(String::as_str)
                    .join(" ")
            })
            .collect()
    };
    // The reasons land on the daemon's next tick, after the run starts.
    setup.until("desk", |rows| queued(rows) == expected);
}

#[test]
fn moving_a_task_to_a_higher_band_makes_it_start_next() {
    let mut setup = Setup::new("move-band");
    setup.memory(12);
    setup.hold("hold-stop");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));
    setup.publish(&requested(44));
    setup.start();
    setup.until("tasks", |rows| {
        rows.len() == 3 && with(rows, 4, "running") == 1
    });

    moved(&setup, &["3", "--band", "today"]);
    moved(&setup, &["2", "--after", "3"]);
    setup.release("hold-stop");

    setup.until("tasks", |rows| {
        with(rows, 4, "done") == 1 && with(rows, 4, "running") == 1
    });
    assert_eq!(
        queue(&setup),
        [
            ["3", "running", "today", "-"],
            ["2", "queued", "today", "1"],
            ["1", "done", "later", "-"],
        ]
        .map(|row| row.map(str::to_owned))
    );
}

#[test]
fn a_renamed_band_set_orders_the_queue_and_moves() {
    let mut setup = Setup::new("renamed-bands");
    let settings = "name = \"demo\"\nbands = [\"urgent\", \"normal\"]\n";
    fs::write(setup.dir.join("factory/factory.toml"), settings).unwrap();
    setup.memory(12);
    setup.publish(&requested(42));
    setup.publish(&requested_in(43, Some("urgent")));
    setup.publish(&requested(44));
    route(&setup);

    let refused = setup.priory(&["move", "1", "--band", "later"]);
    moved(&setup, &["3", "--band", "urgent"]);
    setup.start();

    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "priory move: band `later` is not one of urgent, normal\n"
    );
    setup.until("tasks", |rows| with(rows, 4, "running") == 1);
    assert_eq!(
        queue(&setup),
        [
            ["2", "running", "urgent", "-"],
            ["3", "queued", "urgent", "1"],
            ["1", "queued", "normal", "2"],
        ]
        .map(|row| row.map(str::to_owned))
    );
}

/// A `review` workflow whose one step runs the reviewer, which declares the default 4 GB.
fn with_review(setup: &Setup, implementer_memory: &str) {
    let review = WORKFLOW.replace("agent = \"implementer\"", "agent = \"reviewer\"");
    fs::write(setup.dir.join("factory/workflows/review.toml"), review).unwrap();
    let agent = AGENT.replacen(
        "+++\n",
        &format!("+++\nmemory = \"{implementer_memory}\"\n"),
        1,
    );
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), agent).unwrap();
}

fn review_requested(issue: u64) -> Message {
    let mut message = requested(issue);
    message.data["workflow"] = json!("review");
    message
}

#[test]
fn a_smaller_task_waits_behind_the_head_of_the_queue_that_has_no_room() {
    let mut setup = Setup::new("head-of-line");
    with_review(&setup, "6GB");
    // 11 GB free keeps 6.4: room for the reviewer's 4 GB, not the implementer's 6.
    setup.memory(11);
    setup.publish(&requested_in(42, Some("now")));
    setup.publish(&review_requested(43));

    setup.start();

    let waits = "waits for room: 11.0 GB of memory free, needs 6.0 GB and keeps 6.4 GB free";
    let behind = "waits behind task 1, which waits for room";
    setup.until("tasks", |rows| {
        rows.len() == 2 && rows[0][7] == waits && rows[1][7] == behind
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert!(setup.rows("runs").is_empty());
    setup.memory(13);
    setup.until("tasks", |rows| rows[0][4] == "running");
    setup.memory(20);
    setup.until("tasks", |rows| rows[1][4] == "running");
}

#[test]
fn a_head_that_can_never_fit_holds_back_no_one() {
    let mut setup = Setup::new("never-fits");
    // The runner's 90 % share of 64 GB is 57.6 GB.
    with_review(&setup, "60GB");
    setup.publish(&requested_in(42, Some("now")));
    setup.publish(&review_requested(43));

    setup.start();

    let tasks = setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "running");
    assert_eq!(tasks[0][4], "queued");
}

#[test]
fn the_room_takes_the_agent_memory_and_the_share_of_the_runner_named_by_the_host() {
    let mut setup = Setup::new("share");
    let agent = AGENT.replacen("+++\n", "+++\nmemory = \"6GB\"\n", 1);
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), agent).unwrap();
    let runner = "platform = { os = \"linux\" }\nagents = { cpu = \"50%\", memory = \"50%\" }\n";
    fs::create_dir_all(setup.dir.join("factory/runners")).unwrap();
    fs::write(setup.dir.join("factory/runners/box.toml"), runner).unwrap();
    fs::create_dir_all(setup.dir.join("proc/sys/kernel")).unwrap();
    fs::write(setup.dir.join("proc/sys/kernel/hostname"), "box\n").unwrap();
    setup.memory(37);
    setup.publish(&requested(42));

    setup.start();

    let reason = "waits for room: 37.0 GB of memory free, needs 6.0 GB and keeps 32.0 GB free";
    setup.until("tasks", |rows| rows.len() == 1 && rows[0][7] == reason);
    setup.memory(38);
    setup.until("tasks", |rows| rows[0][4] == "running");
}

#[test]
fn a_prompt_reaches_the_agent_pane_when_a_person_opened_another_pane() {
    let mut setup = Setup::new("pane");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    assert!(
        setup
            .tmux(&["split-window", "-t", "=priory-1:", "--", "cat"])
            .status
            .success()
    );
    setup.publish(&labeled(42));

    setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[3] == "confirmed" && row[4].starts_with("github.issues.labeled"))
    });
    let events = setup
        .typed()
        .iter()
        .filter(|prompt| prompt.contains("github.issues.labeled"))
        .count();
    assert_eq!(events, 1);
}

#[test]
fn a_run_with_no_session_start_within_60_seconds_ends_and_names_the_clone_to_trust() {
    let mut setup = Setup::new("untrusted");
    setup.publish(&requested(42));
    setup.hold("silent");
    setup.start();
    setup.until("runs", |rows| rows.len() == 1);

    let runs = setup.within(75, "runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "exited")
    });

    assert_ne!(runs[0][6], "-");
    assert!(
        !setup
            .tmux(&["has-session", "-t", "=priory-1"])
            .status
            .success()
    );
    let clone = setup.dir.join("home/repos/owner/repo");
    let tasks = setup.rows("tasks");
    assert_eq!(tasks[0][4], "blocked");
    assert_eq!(
        tasks[0][7],
        format!(
            "claude showed no SessionStart within 60s; trust the clone once: cd {} && claude, accept, exit",
            clone.display()
        )
    );
    let outbox = setup.rows("outbox");
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0][3], "queued");
}

const STEPS: &str = r#"
start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change."
effect = "reversible"
next = { ready = "implement", unsure = "approve", hopeless = "stop" }

[[steps]]
id = "approve"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "implement", rejected = "stop" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the plan."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

/// A model step's first prompt: its own, then how to end the step.
fn prompted(prompt: &str, outcomes: &str) -> String {
    format!(
        "{prompt}\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: {outcomes}."
    )
}

fn result(outcome: &str) -> serde_json::Value {
    json!({ "name": "result", "arguments": { "outcome": outcome, "summary": format!("{outcome} it") } })
}

/// The text of a tool reply, and whether it is an error.
fn reply(reply: &serde_json::Value) -> (String, bool) {
    let result = &reply["result"];
    (
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        result["isError"].as_bool().unwrap_or(false),
    )
}

#[test]
fn a_result_with_a_valid_outcome_moves_the_task_to_the_next_model_step_after_stop() {
    let mut setup = Setup::with_workflow("result", STEPS);
    setup.calls(&[result("ready")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert_eq!(
        typed[1],
        format!("{}\n", prompted("Implement the plan.", "failed, passed")),
        "{typed:?}"
    );
    let (text, error) = reply(&setup.replies()[0]);
    assert!(!error, "{text}");
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["implement", "running"]);
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!([&runs[0][2], &runs[0][7]], ["plan", "moved"]);
    assert_eq!([&runs[1][2], &runs[1][7]], ["implement", "-"]);
    assert_eq!(
        runs[0][4], runs[1][4],
        "one session carries the conversation"
    );
    assert_eq!(runs[0][3], runs[1][3]);
}

#[test]
fn a_result_with_an_unknown_outcome_is_refused_with_the_valid_outcomes_and_the_task_stays() {
    let mut setup = Setup::with_workflow("unknown-outcome", STEPS);
    setup.calls(&[result("maybe")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    setup.written("fake/tools.log");
    let (text, error) = reply(&setup.replies()[0]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        r#"outcome "maybe" is not one of step plan's outcomes: hopeless, ready, unsure"#
    );
    std::thread::sleep(Duration::from_millis(1500));
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["plan", "running"]);
    assert_eq!(setup.rows("runs").len(), 1);
    assert_eq!(
        setup.typed().len(),
        2,
        "the refused result leaves a silent turn"
    );
}

#[test]
fn a_second_result_for_a_run_is_refused() {
    let mut setup = Setup::with_workflow("second-result", STEPS);
    setup.calls(&[result("ready")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/tools.log");
    let session = setup.rows("runs")[0][4].clone();

    // The same turn calls result again before its Stop.
    let replies = setup.mcp(
        &session,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": result("unsure")})],
    );

    let (text, error) = reply(&replies[0]);
    assert!(error, "{text}");
    assert_eq!(text, "run 1 already has a result, ready; a result is final");
    setup.release("hold-stop");
    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[3] == "implement")
    });
    assert_eq!(tasks[0][4], "running");
}

#[test]
fn a_human_step_asks_ends_the_run_and_sleeps_until_an_answer_resumes_the_session() {
    let mut setup = Setup::with_workflow("human", STEPS);
    setup.calls(&[result("unsure")]);
    setup.publish(&requested(42));

    setup.start();

    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    let session = runs[0][4].clone();
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["approve", "asleep"]);
    setup.gone("priory-1");
    let asks = setup.rows("asks");
    assert_eq!(
        asks,
        [[
            "1",
            "1",
            "1",
            "approve: unsure it",
            r#"["approved","rejected"]"#
        ]]
    );

    let answered = setup.priory(&["answer", "1", "approved", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    assert!(setup.rows("asks").is_empty());
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let runs = setup.rows("runs");
    assert_eq!([&runs[1][2], &runs[1][4]], ["implement", &session]);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert!(
        args.windows(2).any(|w| w == ["--resume", session.as_str()]),
        "{args:?}"
    );
    assert_eq!(
        setup.typed()[1],
        format!("{}\n", prompted("Implement the plan.", "failed, passed"))
    );
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

const TWO_AGENTS: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "review", failed = "stop" }

[[steps]]
id = "review"
decides = "model"
agent = "reviewer"
prompt = "Review the change."
effect = "none"
next = { approved = "done", changes = "implement" }
"#;

/// The arguments the fake `claude` got at its latest launch.
fn launched(setup: &Setup) -> Vec<String> {
    fs::read_to_string(setup.dir.join("fake/args.log"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn has(args: &[String], pair: [&str; 2]) -> bool {
    args.windows(2).any(|w| w == pair)
}

#[test]
fn a_result_moving_to_another_agent_ends_the_run_and_starts_that_agent_in_a_new_session() {
    let mut setup = Setup::with_workflow("other-agent", TWO_AGENTS);
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 2, "{runs:?}");
    assert_eq!([&runs[0][2], &runs[0][7]], ["implement", "moved"]);
    assert_eq!([&runs[1][2], &runs[1][7]], ["review", "-"]);
    assert_ne!(runs[0][4], runs[1][4], "each agent has its own session");
    let args = launched(&setup);
    assert!(has(&args, ["--session-id", &runs[1][4]]), "{args:?}");
    assert!(
        has(&args, ["--append-system-prompt", "You review one change."]),
        "{args:?}"
    );
    assert!(!args.iter().any(|arg| arg == "--model"), "{args:?}");
    assert_eq!(
        setup.typed()[1],
        format!("{}\n", prompted("Review the change.", "approved, changes"))
    );
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["review", "running"]);
}

#[test]
fn a_workflow_coming_back_to_an_agent_resumes_that_agent_session() {
    let mut setup = Setup::with_workflow("back-to-agent", TWO_AGENTS);
    setup.calls(&[result("passed"), result("changes")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 3, "{runs:?}");
    assert_eq!([&runs[1][2], &runs[1][7]], ["review", "moved"]);
    assert_eq!([&runs[2][2], &runs[2][4]], ["implement", &runs[0][4]]);
    let args = launched(&setup);
    assert!(has(&args, ["--resume", &runs[0][4]]), "{args:?}");
    assert!(
        has(
            &args,
            ["--append-system-prompt", "You implement one issue."]
        ),
        "{args:?}"
    );
    assert_eq!(
        setup.typed()[2],
        format!("{}\n", prompted("Implement the issue.", "failed, passed"))
    );
}

#[test]
fn an_answer_on_the_second_agent_step_resumes_the_second_agent_session() {
    let mut setup = Setup::with_workflow("wake-agent", TWO_AGENTS);
    setup.calls(&[result("passed"), ask()]);
    setup.publish(&requested(42));
    setup.start();
    let runs = setup.until("runs", |rows| {
        rows.get(1).is_some_and(|row| row[7] == "asleep")
    });
    setup.gone("priory-1");

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let resumed = setup.rows("runs");
    assert_eq!([&resumed[2][2], &resumed[2][4]], ["review", &runs[1][4]]);
    assert_ne!(runs[0][4], runs[1][4]);
    let args = launched(&setup);
    assert!(has(&args, ["--resume", &runs[1][4]]), "{args:?}");
    assert!(
        has(&args, ["--append-system-prompt", "You review one change."]),
        "{args:?}"
    );
    assert_eq!(
        setup.typed()[2],
        format!(
            "sqlite\n{}\n",
            "End this step by calling the result tool of the priory MCP server with one of these outcomes: approved, changes."
        )
    );
}

fn ask() -> serde_json::Value {
    json!({ "name": "ask", "arguments": {
        "question": "Which database?",
        "options": ["sqlite", "postgres"],
        "recommendation": "sqlite: one file",
        "tradeoff": "postgres scales, sqlite needs no server",
    } })
}

#[test]
fn an_agent_ask_sleeps_the_task_and_the_answer_wakes_it_as_its_prompt() {
    let mut setup = Setup::with_workflow("agent-ask", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
    assert_eq!(
        setup.rows("asks"),
        [["1", "1", "1", "Which database?", r#"["sqlite","postgres"]"#]]
    );

    let answered = setup.priory(&["answer", "1", "duckdb, actually", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        "duckdb, actually\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n"
    );
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["plan", "running"]);
}

#[test]
fn an_answer_to_a_stale_revision_or_a_second_answer_is_refused() {
    let mut setup = Setup::with_workflow("stale", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    let stale = setup.priory(&["answer", "1", "sqlite", "--revision", "2"]);
    let first = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);
    let second = setup.priory(&["answer", "1", "postgres", "--revision", "1"]);

    assert!(!stale.status.success());
    assert_eq!(
        String::from_utf8_lossy(&stale.stderr),
        "priory answer: ask 1 is at revision 1, not 2\n"
    );
    assert!(first.status.success(), "{first:?}");
    assert!(!second.status.success());
    assert_eq!(
        String::from_utf8_lossy(&second.stderr),
        "priory answer: ask 1 is already answered: sqlite\n"
    );
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        "sqlite\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n"
    );
}

#[test]
fn a_step_ask_takes_only_its_options() {
    let mut setup = Setup::with_workflow("options", STEPS);
    setup.calls(&[result("unsure")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    let answered = setup.priory(&["answer", "1", "sure", "--revision", "1"]);

    assert!(!answered.status.success());
    assert_eq!(
        String::from_utf8_lossy(&answered.stderr),
        "priory answer: ask 1 takes one of: approved, rejected\n"
    );
    assert_eq!(setup.rows("asks").len(), 1);
}

#[test]
fn reaching_done_marks_the_task_done_and_reaching_stop_drops_it() {
    let mut setup = Setup::with_workflow("done", STEPS);
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();

    let runs = setup.until("runs", |rows| rows.len() == 2 && rows[1][7] == "done");
    assert_eq!(setup.rows("tasks")[0][4], "done");
    setup.gone("priory-1");
    assert_ne!(runs[1][6], "-");
    setup.crash();
    fs::remove_file(setup.dir.join("fake/typed.log")).unwrap();
    setup.calls(&[result("hopeless")]);
    setup.publish(&requested(43));
    setup.start();

    let runs = setup.until("runs", |rows| rows.len() == 3 && rows[2][7] == "dropped");
    assert_ne!(runs[2][6], "-");
    assert_eq!(setup.rows("tasks")[1][4], "dropped");
}

#[test]
fn the_built_in_question_tool_is_denied_and_points_to_ask() {
    let mut setup = Setup::new("deny");
    setup.calls(&[json!({ "name": "AskUserQuestion" })]);
    setup.publish(&requested(42));

    setup.start();

    setup.written("fake/tools.log");
    let denied = &setup.replies()[0]["hookSpecificOutput"];
    assert_eq!(denied["permissionDecision"], "deny");
    assert!(
        denied["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("ask tool")
    );
}

#[test]
fn a_permission_request_is_denied_and_tells_the_agent_to_go_on_without_it() {
    let mut setup = Setup::new("permission");
    setup.calls(&[json!({ "name": "Write", "arguments": { "file_path": "/etc/x" } })]);
    setup.publish(&requested(42));

    setup.start();

    setup.written("fake/tools.log");
    let output = &setup.replies()[0]["hookSpecificOutput"];
    assert_eq!(output["hookEventName"], "PermissionRequest");
    assert_eq!(output["decision"]["behavior"], "deny");
    let message = output["decision"]["message"].as_str().unwrap();
    assert!(message.contains("never approves"), "{message}");
    assert!(message.contains("without it"), "{message}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_run_that_shows_a_permission_prompt_ends_and_blocks_its_task() {
    let mut setup = Setup::new("prompted");
    setup.hold("undecided");
    setup.hold("hold-stop");
    setup.calls(&[json!({ "name": "Bash", "arguments": { "command": "rm -rf /" } })]);
    setup.publish(&requested(42));

    setup.start();

    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "permission")
    });
    assert_ne!(runs[0][6], "-");
    let tasks = setup.rows("tasks");
    assert_eq!(tasks[0][4], "blocked");
    assert_eq!(
        tasks[0][7],
        "run 1 stopped on a permission prompt: Claude needs your permission to use Bash"
    );
    setup.gone("priory-1");
}

#[test]
fn the_desk_lists_an_uncertain_prompt_a_blocked_task_and_an_open_ask_oldest_first() {
    let mut setup = Setup::with_workflow("desk", STEPS);
    setup.calls(&[serde_json::Value::Null, ask()]);
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");
    setup.crash();
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    setup.release("hold-submit");
    std::thread::sleep(Duration::from_millis(1100));
    setup.hold("hold-submit");

    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    setup.publish(&requested(43));
    std::thread::sleep(Duration::from_millis(1100));
    setup.release("hold-submit");
    setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "asleep");

    let desk: Vec<_> = setup
        .rows("desk")
        .into_iter()
        .filter(|row| row[1] != "worktree")
        .collect();
    let items: Vec<_> = desk
        .iter()
        .map(|row| [&row[1], &row[2], &row[3]].map(String::as_str))
        .collect();
    assert_eq!(
        items,
        [
            ["prompt", "1", "1"],
            ["blocked", "1", "1"],
            ["ask", "1", "2"]
        ]
    );
    assert_eq!(
        desk[0][5],
        prompted("Plan the change.", "hopeless, ready, unsure").replace('\n', "\\n")
    );
    assert_eq!(
        desk[1][5],
        "run 1 exited: claude no longer runs in tmux session priory-1"
    );
    assert_eq!(desk[2][5], r#"Which database? ["sqlite","postgres"]"#);
    assert!(
        desk.windows(2).all(|pair| pair[0][0] < pair[1][0]),
        "{desk:?}"
    );
}

#[test]
fn resume_restarts_a_blocked_task_in_its_session_and_cancel_drops_it_and_ends_its_run() {
    let mut setup = Setup::new("resume");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let session = setup.rows("runs")[0][4].clone();
    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });

    let resumed = setup.priory(&["resume", "1"]);

    assert!(resumed.status.success(), "{resumed:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let runs = setup.rows("runs");
    assert_eq!([&runs[1][4], &runs[1][7]], [&session, "-"]);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    assert!(args.contains(&format!("--resume\n{session}\n")), "{args}");
    assert!(args.contains("--permission-mode\nmanual\n"), "{args}");
    assert_eq!(
        setup.typed()[2],
        "A person resumed this task after: run 1 exited: claude no longer runs in tmux session priory-1. Check the worktree, then continue.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n"
    );
    let again = setup.priory(&["resume", "1"]);
    assert!(!again.status.success());
    assert_eq!(
        String::from_utf8_lossy(&again.stderr),
        "priory resume: task 1 is running, not blocked\n"
    );

    let cancelled = setup.priory(&["cancel", "1"]);

    assert!(cancelled.status.success(), "{cancelled:?}");
    let tasks = setup.rows("tasks");
    assert_eq!(
        [&tasks[0][4], &tasks[0][7]],
        ["dropped", "cancelled from the desk"]
    );
    assert_eq!(setup.rows("runs")[1][7], "cancelled");
    setup.gone("priory-1");
    assert!(setup.rows("desk").is_empty());
}

#[test]
fn resuming_a_task_whose_runs_never_started_queues_one_resume_line_and_one_ending() {
    let typed = resume_three_failed_launches(Setup::new("resume-unstarted"));
    assert_eq!(
        typed,
        "A person resumed this task. Check the worktree, then continue.\nImplement the issue.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n"
    );
}

#[test]
fn resuming_a_shared_agents_task_whose_runs_never_started_queues_one_resume_line() {
    let typed = resume_three_failed_launches(shared("resume-shared"));
    assert_eq!(
        typed.matches("A person resumed this task").count(),
        1,
        "{typed}"
    );
}

/// The first prompt a run takes after three launches that died and a fourth resume.
fn resume_three_failed_launches(mut setup: Setup) -> String {
    setup.publish(&requested(42));
    setup.hold("die");
    setup.start();
    let blocked = |rows: &[Vec<String>]| rows.first().is_some_and(|row| row[4] == "blocked");
    setup.until("tasks", blocked);
    for _ in 0..2 {
        assert!(setup.priory(&["resume", "1"]).status.success());
        setup.until("runs", |rows| {
            rows.len() >= 2 && rows.iter().all(|row| row[6] != "-")
        });
        setup.until("tasks", blocked);
    }
    setup.release("die");

    let resumed = setup.priory(&["resume", "1"]);

    assert!(resumed.status.success(), "{resumed:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    setup.typed().remove(0)
}

const INTERRUPTED: &str = "Priory restarted and found tmux session priory-2 gone, as after a reboot, so this run was interrupted. Check the worktree, then continue.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n";

#[test]
fn a_daemon_started_after_its_tmux_server_died_resumes_each_lost_run_once() {
    let mut setup = Setup::with_workflow("lost", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.hold("hold-submit");
    setup.publish(&requested(43));
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");
    let session = setup.rows("runs")[1][4].clone();
    setup.reboot();
    setup.release("hold-submit");

    setup.start();

    let outbox = setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(with(&outbox, 3, "uncertain"), 1);
    let runs = setup.rows("runs");
    assert_eq!(
        [&runs[1][7], &runs[2][4], &runs[2][7]],
        ["lost", &session, "-"]
    );
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    assert!(
        args.starts_with(&format!("--resume\n{session}\n")),
        "{args}"
    );
    assert_eq!(setup.typed()[1..2], [INTERRUPTED]);
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][4], &tasks[1][4]], ["asleep", "running"]);

    setup.reboot();
    setup.start();

    let tasks = setup.until("tasks", |rows| rows[1][4] == "blocked");
    assert_eq!(
        tasks[1][7],
        "run 3 was lost again before it made progress: tmux session priory-2 is gone"
    );
    assert_eq!(tasks[0][4], "asleep");
    let runs = setup.rows("runs");
    assert_eq!([runs.len(), with(&runs, 7, "lost")], [3, 2]);
}

#[test]
fn an_answer_to_the_ask_of_a_cancelled_task_is_refused() {
    let mut setup = Setup::with_workflow("cancel-ask", STEPS);
    setup.calls(&[result("unsure")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    assert!(setup.priory(&["cancel", "1"]).status.success());

    let answered = setup.priory(&["answer", "1", "approved", "--revision", "1"]);

    assert!(!answered.status.success());
    assert_eq!(
        String::from_utf8_lossy(&answered.stderr),
        "priory answer: task 1 is dropped\n"
    );
    assert_eq!(setup.rows("tasks")[0][4], "dropped");
}

#[test]
fn a_resumed_task_gets_its_prompts_although_its_exited_run_left_an_uncertain_one() {
    let mut setup = Setup::new("resume-uncertain");
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");
    setup.crash();
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    setup.release("hold-submit");
    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });

    assert!(setup.priory(&["resume", "1"]).status.success());

    let outbox = setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[3] == "confirmed" && row[4].starts_with("A person resumed"))
    });
    assert_eq!(with(&outbox, 3, "uncertain"), 1);
    assert!(
        setup
            .typed()
            .iter()
            .any(|prompt| prompt.starts_with("A person resumed"))
    );
}

#[test]
fn a_lost_run_that_asked_since_the_last_loss_resumes_again() {
    let mut setup = Setup::with_workflow("lost-progress", STEPS);
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    setup.reboot();
    setup.calls(&[ask()]);
    setup.hold("hold-stop");
    setup.start();
    setup.written("fake/tools.log");
    assert!(
        setup
            .priory(&["answer", "1", "sqlite", "--revision", "1"])
            .status
            .success()
    );

    setup.reboot();
    setup.release("hold-stop");
    setup.start();

    let runs = setup.until("runs", |rows| rows.len() == 3);
    assert_eq!(
        [&runs[0][7], &runs[1][7], &runs[2][7]],
        ["lost", "lost", "-"]
    );
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn the_mcp_server_lists_the_result_ask_and_wait_tools() {
    let setup = Setup::new("list");

    let replies = setup.mcp(
        "s",
        &[
            json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        ],
    );

    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    let instructions = replies[0]["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains(&format!(
            "{} memory acquire --gb N -- <command>",
            env!("CARGO_BIN_EXE_priory")
        )),
        "{instructions}"
    );
    let names: Vec<_> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["result", "ask", "wait"]);
}

#[test]
fn an_event_kept_during_a_step_reaches_the_agent_after_the_next_step_prompt() {
    let mut setup = Setup::with_workflow("event-order", STEPS);
    setup.calls(&[result("ready")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/tools.log");

    setup.publish(&labeled(42));
    setup.until("outbox", |rows| rows.len() == 2);
    setup.release("hold-stop");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(typed.len() >= 2, "{typed:?}");
    assert!(
        typed[1].starts_with(&format!(
            "{}\ngithub.issues.labeled owner/repo#42 {{",
            prompted("Implement the plan.", "failed, passed")
        )),
        "{typed:?}"
    );
    let inbox = setup.rows("inbox");
    assert_eq!(inbox.len(), 2);
}

#[test]
fn an_answer_that_beats_the_stop_is_typed_into_the_same_run_once() {
    let mut setup = Setup::with_workflow("early-answer", STEPS);
    setup.calls(&[ask()]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "running");
    assert_eq!(setup.rows("outbox")[1][3], "queued");
    assert_eq!(setup.typed().len(), 1);
    setup.release("hold-stop");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        setup.typed(),
        [
            format!(
                "{}\n",
                prompted("Plan the change.", "hopeless, ready, unsure")
            ),
            format!("{}\n", prompted("sqlite", "hopeless, ready, unsure")),
            NUDGE.replace("failed, passed", "hopeless, ready, unsure")
        ]
    );
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0][7], "-");
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_restarted_daemon_kills_the_session_of_a_run_that_ended() {
    let mut setup = Setup::with_workflow("orphan", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    let runs = setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    setup.gone("priory-1");
    setup.crash();
    // As if the daemon had crashed after ending the run and before killing its session.
    let session = &runs[0][4];
    assert!(
        setup
            .tmux(&[
                "new-session",
                "-d",
                "-s",
                "priory-1",
                "--",
                "cat",
                ";",
                "set-option",
                "-p",
                "@priory",
                session
            ])
            .status
            .success()
    );

    setup.start();

    setup.gone("priory-1");
}

/// A GitHub event on pull request 7 of `owner/repo`, from the branch `priory/1`.
fn on_pull(kind: &str, extra: serde_json::Value) -> Message {
    let mut data = json!({
        "repo": "owner/repo",
        "pull_request": 7,
        "head_branch": "priory/1",
        "head_sha": "abc0000000000000000000000000000000000000",
        "base_branch": "main",
    });
    data.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    Message::new(kind, data).subject("owner/repo#7")
}

#[test]
fn a_pull_request_event_naming_the_task_branch_reaches_the_task() {
    let mut setup = Setup::new("branch");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    setup.publish(&on_pull("github.pull_request.opened", json!({})));

    let inbox = setup.until("inbox", |rows| rows.len() == 2);
    assert_eq!(
        inbox[1][1..],
        ["github.pull_request.opened", "owner/repo#7", "1", "kept"]
    );
}

fn wait(on: serde_json::Value) -> serde_json::Value {
    json!({ "name": "wait", "arguments": { "on": on } })
}

#[test]
fn an_unknown_condition_is_refused_and_nothing_is_stored() {
    let mut setup = Setup::new("unknown-wait");
    setup.calls(&[wait(json!({ "type": "moon" }))]);
    setup.publish(&requested(42));

    setup.start();

    setup.written("fake/tools.log");
    let (text, error) = reply(&setup.replies()[0]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        "unknown condition \"moon\": wait on pull_request, checks, review, time, task, linear_comment, linear_state, slack_reply or disk"
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert!(setup.rows("waits").is_empty());
    assert_eq!(setup.rows("tasks")[0][4], "running");
    assert_eq!(setup.rows("runs")[0][7], "-");
}

fn on_pull_7(kind: &str) -> serde_json::Value {
    json!({ "type": kind, "repo": "owner/repo", "pull_request": 7 })
}

fn check(id: u64, pending: u64) -> Message {
    on_pull(
        "github.check_run.completed",
        json!({ "name": format!("check {id}"), "conclusion": "success", "github_id": id, "pending": pending, "failing": 0 }),
    )
}

/// Starts task 1, whose first turn calls `wait(on)`, and returns once its run ended asleep.
fn asleep_on(name: &str, on: serde_json::Value) -> Setup {
    let mut setup = Setup::new(name);
    setup.calls(&[wait(on)]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    setup.gone("priory-1");
    setup
}

/// Asserts the task stays asleep with no new run for a while.
fn stays_asleep(setup: &Setup) {
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
    assert_eq!(setup.rows("runs").len(), 1);
}

#[test]
fn a_run_waiting_on_checks_sleeps_and_only_the_last_check_wakes_it() {
    let setup = asleep_on("checks", on_pull_7("checks"));
    let (text, error) = reply(&setup.replies()[0]);
    assert!(!error, "{text}");
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
    assert_eq!(
        setup.rows("waits")[0][1..5],
        ["1", "1", "checks", "github:owner/repo#7"]
    );

    setup.publish(&check(1, 1));
    stays_asleep(&setup);
    setup.publish(&check(2, 0));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    let lines: Vec<_> = typed[1].lines().collect();
    assert!(
        lines[0].starts_with("github.check_run.completed owner/repo#7 {")
            && lines[0].contains(r#""github_id":2"#),
        "{typed:?}"
    );
    assert!(lines[1].contains(r#""github_id":1"#), "{typed:?}");
    assert!(setup.rows("waits").is_empty());
    let runs = setup.rows("runs");
    assert_eq!(runs[0][4], runs[1][4], "the woken run resumes the session");
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

fn reviewed() -> Message {
    on_pull(
        "github.pull_request_review.submitted",
        json!({ "state": "approved", "github_id": 300 }),
    )
}

fn closed() -> Message {
    on_pull(
        "github.pull_request.closed",
        json!({ "merged": true, "github_id": 400 }),
    )
}

#[test]
fn a_review_or_a_close_wakes_a_task_waiting_on_it() {
    for (kind, event, line) in [
        (
            "review",
            reviewed(),
            "github.pull_request_review.submitted owner/repo#7 {",
        ),
        (
            "pull_request",
            closed(),
            "github.pull_request.closed owner/repo#7 {",
        ),
    ] {
        let setup = asleep_on(&format!("wake-{kind}"), on_pull_7(kind));

        setup.publish(&event);

        setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
        let typed = setup.typed();
        assert!(typed[1].starts_with(line), "{kind}: {typed:?}");
        assert_eq!(setup.rows("tasks")[0][4], "running");
    }
}

#[test]
fn an_event_that_does_not_match_the_wait_stays_kept_and_reaches_the_next_run() {
    let setup = asleep_on("unmatched", on_pull_7("pull_request"));

    setup.publish(&reviewed());
    stays_asleep(&setup);
    assert_eq!(setup.rows("inbox")[1][4], "kept");
    setup.publish(&closed());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    std::thread::sleep(Duration::from_millis(1500));
    let typed = setup.typed();
    assert_eq!(typed.len(), 3, "{typed:?}");
    let lines: Vec<_> = typed[1].lines().collect();
    assert!(
        lines[0].starts_with("github.pull_request.closed owner/repo#7 {"),
        "{typed:?}"
    );
    assert!(
        lines[1].starts_with("github.pull_request_review.submitted owner/repo#7 {"),
        "{typed:?}"
    );
}

#[test]
fn a_checks_wait_on_a_head_ignores_checks_of_another_commit() {
    let mut on = on_pull_7("checks");
    on["head_sha"] = json!("def0000000000000000000000000000000000000");
    let setup = asleep_on("checks-head", on);

    setup.publish(&check(1, 0));
    stays_asleep(&setup);
    let mut last = check(2, 0);
    last.data["head_sha"] = json!("def0000000000000000000000000000000000000");
    setup.publish(&last);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(typed[1].contains(r#""github_id":2"#), "{typed:?}");
}

#[test]
fn a_time_wait_fires_after_its_instant_and_not_before() {
    let setup = asleep_on("time", json!({ "type": "time", "until": "4 seconds" }));
    let waits = setup.rows("waits");
    assert_eq!(waits[0][3], "time");

    stays_asleep(&setup);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        format!(
            "wait 1 fired: {} passed\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n",
            waits[0][4]
        )
    );
    let runs = setup.rows("runs");
    let (asleep, woken) = (&runs[0][6], &runs[1][5]);
    assert!(
        woken.as_str() >= waits[0][4].as_str(),
        "woke at {woken}, before {}",
        waits[0][4]
    );
    assert!(asleep.as_str() < waits[0][4].as_str());
}

#[test]
fn a_task_wait_fires_when_the_other_task_is_done() {
    let mut setup = Setup::new("task-wait");
    // Room for one run, so the two runs take the shared fake's calls in turn.
    setup.memory(12);
    setup.calls(&[wait(json!({ "type": "task", "task": 2 })), result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "done");
    setup.until("outbox", |rows| {
        rows.iter().any(|row| {
            row[1] == "1"
                && row[3] == "confirmed"
                && row[4].starts_with("wait 1 fired: task 2 is done\\nEnd this step")
        })
    });
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn an_event_that_already_satisfies_the_wait_when_it_is_stored_wakes_the_task_at_once() {
    let mut setup = Setup::new("already");
    setup.calls(&[wait(on_pull_7("pull_request"))]);
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/pending");
    // The pull request closes after the agent read its prompt and before it calls wait.
    setup.publish(&closed());
    setup.until("outbox", |rows| rows.len() == 2);

    setup.release("hold-submit");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let (text, error) = reply(&setup.replies()[0]);
    assert!(!error, "{text}");
    assert!(text.starts_with("wait 1 already holds"), "{text}");
    assert!(
        setup.typed()[1].starts_with("github.pull_request.closed owner/repo#7 {"),
        "{:?}",
        setup.typed()
    );
    assert!(setup.rows("waits").is_empty());
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 1, "the run never slept: {runs:?}");
    assert_eq!(runs[0][7], "-");
}

fn passed_at(commit: &str) -> serde_json::Value {
    json!({ "name": "result", "arguments": { "outcome": "passed", "summary": "passed it", "commit": commit } })
}

/// Task 1 owns pull request 7 through a review wait, then a review wakes it and its result
/// reaches `done` at commit `abc0000000000000000000000000000000000000`. Returns once that run ended.
fn gated(name: &str) -> Setup {
    gated_at(name, "abc0000000000000000000000000000000000000")
}

/// As `gated`, with the result naming `commit`.
fn gated_at(name: &str, commit: &str) -> Setup {
    let mut setup = Setup::new(name);
    setup.calls(&[wait(on_pull_7("review")), passed_at(commit)]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.publish(&reviewed());
    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] != "-");
    setup.gone("priory-1");
    setup
}

#[test]
fn a_result_commit_in_capitals_matches_the_same_head_in_lower_case() {
    let setup = gated_at("gate-capitals", "ABC0000000000000000000000000000000000000");

    setup.publish(&check(1, 0));

    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_checks_wait_on_an_abbreviated_head_is_refused_and_nothing_is_stored() {
    let mut on = on_pull_7("checks");
    on["head_sha"] = json!("def0000");
    let mut setup = Setup::new("checks-short-head");
    setup.calls(&[wait(on)]);
    setup.publish(&requested(42));

    setup.start();

    setup.written("fake/tools.log");
    let (text, error) = reply(&setup.replies()[0]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        "commit \"def0000\" is not a full commit SHA: send the 40 characters that git rev-parse HEAD prints"
    );
    assert!(setup.rows("waits").is_empty());
}

#[test]
fn reaching_done_with_an_owned_pull_request_sleeps_the_task_on_its_checks() {
    let setup = gated("gate");

    let (text, error) = reply(&setup.replies()[1]);
    assert!(!error, "{text}");
    assert_eq!(setup.rows("runs")[1][7], "asleep");
    stays_asleep_after(&setup, 2);
    assert_eq!(
        setup.rows("waits")[0][1..5],
        ["1", "2", "checks", "github:owner/repo#7"]
    );
}

/// Asserts the task stays asleep with `runs` runs for a while.
fn stays_asleep_after(setup: &Setup, runs: usize) {
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
    assert_eq!(setup.rows("runs").len(), runs);
}

#[test]
fn green_checks_at_the_result_commit_mark_the_task_done_and_start_no_run() {
    let setup = gated("gate-green");
    // A red check completed in the same poll, so its event comes next.
    let mut seen_red = check(4, 0);
    seen_red.data["failing"] = json!(1);

    setup.publish(&check(1, 1));
    setup.publish(&seen_red);
    stays_asleep_after(&setup, 2);
    setup.publish(&check(2, 0));

    setup.until("tasks", |rows| rows[0][4] == "done");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("runs").len(), 2);
    assert_eq!(setup.typed().len(), 2, "{:?}", setup.typed());
    assert!(setup.rows("waits").is_empty());
}

fn check_runs(runs: u64) -> Message {
    on_pull("github.pull_request.check_runs", json!({ "runs": runs }))
}

#[test]
fn a_head_with_no_check_runs_at_the_result_commit_marks_the_task_done_and_starts_no_run() {
    let setup = gated("gate-no-checks");

    setup.publish(&check_runs(0));

    setup.until("tasks", |rows| rows[0][4] == "done");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("runs").len(), 2);
    assert!(setup.rows("waits").is_empty());
}

#[test]
fn a_check_run_appearing_after_none_at_the_same_head_is_judged_as_any_check() {
    let mut setup = Setup::new("gate-late-check");
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.publish(&check_runs(0));
    setup.publish(&check_runs(1));
    setup.until("inbox", |rows| rows.len() == 3);

    setup.publish(&reviewed());
    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] != "-");
    setup.gone("priory-1");

    stays_asleep_after(&setup, 2);
    setup.publish(&check(2, 0));
    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_snapshot_with_runs_after_green_checks_leaves_the_checks_to_decide() {
    let mut setup = Setup::new("gate-upgrade");
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.publish(&check(1, 0));
    setup.publish(&check_runs(1));
    setup.until("inbox", |rows| rows.len() == 3);

    setup.publish(&reviewed());

    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_red_check_at_the_result_commit_wakes_the_agent_with_the_failure() {
    let setup = gated("gate-red");
    let mut red = check(1, 1);
    red.data["name"] = json!("test");
    red.data["conclusion"] = json!("failure");
    red.data["url"] = json!("https://github.com/owner/repo/runs/1");
    setup.calls(&[passed_at("def0000000000000000000000000000000000000")]);
    setup.hold("hold-submit");

    setup.publish(&red);
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    // A check of the old commit finishes while the agent fixes it.
    setup.publish(&check(3, 0));
    setup.until("inbox", |rows| rows.len() == 4);
    setup.release("hold-submit");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    let lines: Vec<_> = typed[2].lines().collect();
    assert_eq!(
        lines[0],
        "wait 2 fired: check test at abc0000000000000000000000000000000000000 concluded failure https://github.com/owner/repo/runs/1",
        "{typed:?}"
    );
    assert!(lines[1].starts_with("github.check_run.completed owner/repo#7 {"));
    let runs = setup.until("runs", |rows| rows.len() == 3 && rows[2][7] == "asleep");
    assert_eq!(
        setup.rows("waits")[0][2],
        runs[2][0],
        "the fix gets a gate of its own"
    );
    stays_asleep_after(&setup, 3);
    let mut green = check(2, 0);
    green.data["head_sha"] = json!("def0000000000000000000000000000000000000");
    setup.publish(&green);
    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_head_that_moved_from_the_result_commit_wakes_the_agent() {
    let setup = gated("gate-moved");
    let mut moved = on_pull("github.pull_request.labeled", json!({ "github_id": 500 }));
    moved.data["head_sha"] = json!("def0000000000000000000000000000000000000");

    setup.publish(&moved);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    let lines: Vec<_> = typed[2].lines().collect();
    assert_eq!(
        lines[0],
        "wait 2 fired: the head of github:owner/repo#7 moved to def0000000000000000000000000000000000000, away from your result's commit abc0000000000000000000000000000000000000",
        "{typed:?}"
    );
    assert!(lines[1].starts_with("github.pull_request.labeled owner/repo#7 {"));
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_result_reaching_done_without_a_commit_is_refused_when_the_task_owns_a_pull_request() {
    let mut setup = Setup::new("gate-no-commit");
    setup.calls(&[wait(on_pull_7("review")), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");

    setup.publish(&reviewed());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    setup.until("runs", |_| setup.replies().len() == 2);
    let (text, error) = reply(&setup.replies()[1]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        "task 1 owns github:owner/repo#7: send the result with the commit its head must match"
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_result_with_an_abbreviated_commit_is_refused() {
    let mut setup = Setup::new("short-commit");
    setup.calls(&[passed_at("3eebfbb")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("runs", |_| !setup.replies().is_empty());
    let (text, error) = reply(&setup.replies()[0]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        "commit \"3eebfbb\" is not a full commit SHA: send the 40 characters that git rev-parse HEAD prints"
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn green_checks_kept_before_the_final_stop_mark_the_task_done_when_the_gate_is_stored() {
    let mut setup = Setup::new("gate-early");
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.hold("hold-stop");
    setup.publish(&reviewed());
    setup.until("runs", |_| setup.replies().len() == 2);
    setup.publish(&check(1, 0));
    setup.until("inbox", |rows| rows.len() == 3);

    setup.release("hold-stop");

    setup.until("tasks", |rows| rows[0][4] == "done");
    assert_eq!(setup.rows("runs").len(), 2);
    assert_eq!(setup.rows("runs")[1][7], "asleep");
}

#[test]
fn a_head_that_moved_after_green_checks_kept_before_the_final_stop_wakes_the_agent() {
    let mut setup = Setup::new("gate-early-moved");
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.hold("hold-stop");
    setup.publish(&reviewed());
    setup.until("runs", |_| setup.replies().len() == 2);
    setup.publish(&check(1, 0));
    let mut moved = on_pull("github.pull_request.labeled", json!({ "github_id": 500 }));
    moved.data["head_sha"] = json!("def0000000000000000000000000000000000000");
    setup.publish(&moved);
    setup.until("inbox", |rows| rows.len() == 4);

    setup.release("hold-stop");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert!(
        setup.typed()[2].starts_with("wait 2 fired: the head of github:owner/repo#7 moved to def0000000000000000000000000000000000000"),
        "{:?}",
        setup.typed()
    );
}

#[test]
fn green_checks_the_agent_already_waited_on_mark_the_task_done_at_its_result() {
    let mut setup = Setup::new("gate-seen");
    setup.calls(&[
        wait(on_pull_7("checks")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");

    setup.publish(&check(1, 0));

    setup.until("tasks", |rows| rows[0][4] == "done");
    assert_eq!(setup.rows("runs").len(), 2);
}

#[test]
fn a_pull_request_opened_from_the_task_branch_holds_the_task_at_done() {
    let mut setup = Setup::new("gate-branch");
    setup.calls(&[passed_at("abc0000000000000000000000000000000000000")]);
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/pending");
    setup.publish(&on_pull(
        "github.pull_request.opened",
        json!({ "github_id": 600 }),
    ));
    setup.until("inbox", |rows| rows.len() == 2);

    setup.release("hold-submit");

    setup.until("runs", |rows| rows[0][7] == "asleep");
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
    assert_eq!(
        setup.rows("waits")[0][3..5],
        ["checks", "github:owner/repo#7"]
    );
}

const APPROVAL: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "approve", failed = "stop" }

[[steps]]
id = "approve"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "done", rejected = "stop" }
"#;

#[test]
fn an_answer_reaching_done_holds_the_task_on_the_checks_of_its_last_result() {
    let mut setup = Setup::with_workflow("gate-answer", APPROVAL);
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.publish(&reviewed());
    setup.until("asks", |rows| rows.len() == 1);

    let answered = setup.priory(&["answer", "1", "approved", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    stays_asleep_after(&setup, 2);
    assert_eq!(setup.rows("tasks")[0][3], "implement");
    let mut red = check(1, 1);
    red.data["conclusion"] = json!("failure");
    setup.publish(&red);
    let runs = setup.until("runs", |rows| rows.len() == 3);
    assert_eq!(runs[2][2], "implement");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(
        typed
            .last()
            .unwrap()
            .contains("\nEnd this step by calling the result tool"),
        "{typed:?}"
    );
}

#[test]
fn a_green_check_counting_a_failure_wakes_the_agent_with_the_failed_check() {
    let mut setup = Setup::new("gate-failing");
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.hold("hold-submit");
    setup.publish(&reviewed());
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    // Both checks completed in one poll: the red one, then a green one that counts it.
    let mut red = check(1, 0);
    red.data["name"] = json!("test");
    red.data["conclusion"] = json!("failure");
    red.data["failing"] = json!(1);
    let mut green = check(2, 0);
    green.data["failing"] = json!(1);
    setup.publish(&red);
    setup.publish(&green);
    setup.until("inbox", |rows| rows.len() == 4);

    setup.release("hold-submit");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(
        typed[2].starts_with("wait 2 fired: check test at abc0000000000000000000000000000000000000 concluded failure\n"),
        "{typed:?}"
    );
    assert_eq!(
        typed[2].matches(r#""github_id":1,"#).count(),
        1,
        "{typed:?}"
    );
}

#[test]
fn green_checks_at_the_result_commit_do_not_finish_the_task_when_the_head_moved_before_it() {
    let mut setup = Setup::new("gate-moved-before");
    setup.calls(&[
        wait(on_pull_7("review")),
        passed_at("abc0000000000000000000000000000000000000"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.hold("hold-submit");
    setup.publish(&reviewed());
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.publish(&check(1, 0));
    let mut moved = on_pull("github.pull_request.labeled", json!({ "github_id": 500 }));
    moved.data["head_sha"] = json!("def0000000000000000000000000000000000000");
    setup.publish(&moved);
    setup.until("inbox", |rows| rows.len() == 4);

    setup.release("hold-submit");

    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] == "asleep");
    stays_asleep_after(&setup, 2);
}

const OPEN_PR: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

fn passed_with_evidence() -> serde_json::Value {
    json!({ "name": "result", "arguments": {
        "outcome": "passed",
        "summary": "Add the widget\nIt draws a box.",
        "evidence": "cargo test passed",
    } })
}

impl Setup {
    /// Commits a file on the task's branch in its worktree and returns the commit.
    fn commit(&self, task: i64, file: &str) -> String {
        let worktree = self.dir.join(format!("home/worktrees/owner/repo/{task}"));
        fs::write(worktree.join(file), "change\n").unwrap();
        for args in [
            &["add", file][..],
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                file,
            ],
        ] {
            assert!(git(&worktree).args(args).status().unwrap().success());
        }
        let head = git(&worktree).args(["rev-parse", "HEAD"]).output().unwrap();
        String::from_utf8(head.stdout).unwrap().trim().to_owned()
    }

    /// The commit `branch` points to in the bare origin.
    fn pushed(&self, branch: &str) -> String {
        let bare = self.dir.join("origin/owner/repo.git");
        let output = git(&bare).args(["rev-parse", branch]).output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// The arguments of each call the fake `gh` got, in order.
    fn gh(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.dir.join("fake/gh.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

/// Starts task 1 on `workflow`, whose implement run passes, and returns once it asks `approve-pr`.
fn asked_to_open(name: &str, workflow: &str) -> Setup {
    let mut setup = Setup::with_workflow(name, workflow);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    setup.gone("priory-1");
    setup
}

#[test]
fn an_approved_open_pr_pushes_the_branch_opens_a_pull_request_and_the_task_owns_it() {
    let setup = asked_to_open("open-pr", OPEN_PR);
    let commit = setup.commit(1, "widget.txt");

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(
        actions[0][1..],
        [
            "1",
            "open-pr",
            "open-pr",
            "opened",
            "github:owner/repo#7",
            &commit,
            "-"
        ]
    );
    assert_eq!(setup.pushed("priory/1"), commit);
    let created = setup.gh().pop().unwrap();
    let wanted: Vec<String> = [
        "pr",
        "create",
        "-R",
        "owner/repo",
        "--base",
        "main",
        "--head",
        "priory/1",
        "--title",
        "Add the widget",
        "--body",
        "Add the widget\nIt draws a box.\n\ncargo test passed",
    ]
    .map(str::to_owned)
    .into();
    assert_eq!(created, wanted);
    let waits = setup.rows("waits");
    assert_eq!(waits[0][3..5], ["checks", "github:owner/repo#7"]);
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["implement", "asleep"]);
}

#[test]
fn a_failing_open_pr_follows_next_failed_with_the_error() {
    let setup = asked_to_open("open-pr-fails", OPEN_PR);
    setup.hold("gh-fail");

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4..7], ["failed", "-", "-"]);
    assert!(
        actions[0][7].ends_with("pull request create failed: GraphQL: denied"),
        "{actions:?}"
    );
    assert_eq!(setup.rows("tasks")[0][4], "dropped");
}

const APPROVE_THEN_POLISH: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "polish", stop = "stop" }

[[steps]]
id = "polish"
decides = "model"
agent = "implementer"
prompt = "Polish the change."
effect = "reversible"
next = { passed = "open-pr" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

#[test]
fn a_commit_made_after_the_approval_asks_again_instead_of_opening() {
    let setup = asked_to_open("open-pr-moved", APPROVE_THEN_POLISH);
    setup.calls(&[passed_with_evidence()]);
    setup.hold("hold-stop");
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.written("fake/tools.log");
    while setup.replies().len() < 2 {
        std::thread::sleep(Duration::from_millis(50));
    }
    let commit = setup.commit(1, "polish.txt");

    setup.release("hold-stop");

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(asks[0][2], "2");
    assert!(
        asks[0][3].contains(&format!(" to {commit} since")),
        "{asks:?}"
    );
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["approve-pr", "asleep"]);
    assert!(setup.rows("actions").is_empty());
    assert!(setup.gh().is_empty());

    setup.calls(&[passed_with_evidence()]);
    let stale = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    let again = setup.priory(&["answer", "1", "open", "--revision", "2"]);

    assert!(!stale.status.success(), "{stale:?}");
    assert!(again.status.success(), "{again:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4..6], ["opened", "github:owner/repo#7"]);
    assert_eq!(setup.pushed("priory/1"), actions[0][6]);
}

const AUTONOMOUS: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "open-pr", failed = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
autonomous = "a draft for review, which a person merges"
next = { opened = "done", failed = "stop" }
"#;

#[test]
fn an_autonomous_irreversible_step_runs_without_an_ask() {
    let mut setup = Setup::with_workflow("open-pr-autonomous", AUTONOMOUS);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));

    setup.start();

    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(
        actions[0][2..6],
        ["open-pr", "open-pr", "opened", "github:owner/repo#7"]
    );
    assert!(setup.rows("asks").is_empty());
}

const UNKNOWN_ACTION: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "merge", failed = "stop" }

[[steps]]
id = "merge"
decides = "code"
action = "merge-pr"
effect = "reversible"
next = { merged = "done" }
"#;

#[test]
fn an_action_that_is_not_built_blocks_the_task_with_its_name() {
    let mut setup = Setup::with_workflow("unknown-action", UNKNOWN_ACTION);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    assert_eq!(
        [&tasks[0][3], &tasks[0][7]],
        ["merge", "action merge-pr is not built yet"]
    );
    assert!(setup.rows("actions").is_empty());
}

#[test]
fn reusing_a_pull_request_another_active_task_owns_fails() {
    let mut setup = Setup::with_workflow("open-pr-owned", AUTONOMOUS);
    setup.calls(&[wait(on_pull_7("review")), passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });

    setup.publish(&requested(43));

    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][1..6], ["2", "open-pr", "open-pr", "failed", "-"]);
    assert_eq!(
        actions[0][7],
        "github:owner/repo#7 belongs to task 1, not this one"
    );
    assert_eq!(setup.rows("tasks")[1][4], "dropped");
}

const APPROVE_THEN_SHIP: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["ship"]
options = ["open", "stop"]
next = { open = "polish", stop = "stop" }

[[steps]]
id = "polish"
decides = "model"
agent = "implementer"
prompt = "Polish the change."
effect = "reversible"
next = { passed = "ship" }

[[steps]]
id = "ship"
decides = "model"
agent = "implementer"
prompt = "Ship it."
effect = "irreversible"
next = { passed = "done" }
"#;

#[test]
fn an_irreversible_model_step_asks_again_after_a_commit_instead_of_starting() {
    let setup = asked_to_open("ship-moved", APPROVE_THEN_SHIP);
    setup.calls(&[passed_with_evidence()]);
    setup.hold("hold-stop");
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.written("fake/tools.log");
    while setup.replies().len() < 2 {
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.commit(1, "polish.txt");

    setup.release("hold-stop");

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(asks[0][2], "2");
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["approve-pr", "asleep"]);
    let runs = setup.rows("runs");
    assert!(runs.iter().all(|run| run[2] != "ship"), "{runs:?}");

    setup.calls(&[passed_with_evidence()]);
    let again = setup.priory(&["answer", "1", "open", "--revision", "2"]);

    assert!(again.status.success(), "{again:?}");
    let ship = prompted("Ship it.", "passed");
    setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[4] == ship.replace('\n', "\\n") && row[3] == "confirmed")
    });
    let typed = setup.typed();
    assert_eq!(
        typed[2..4],
        [
            format!("{}\n", prompted("Polish the change.", "passed")),
            format!("{ship}\n")
        ]
    );
}

impl Setup {
    /// A setup whose tasks 1 to `n` ran to done, with the daemon stopped.
    fn ended(name: &str, n: u64) -> Setup {
        let mut setup = Setup::new(name);
        setup.calls(&vec![result("passed"); n as usize]);
        setup.start();
        for issue in 1..=n {
            setup.publish(&requested(issue));
            setup.until("tasks", |rows| with(rows, 4, "done") == issue as usize);
        }
        setup.crash();
        setup
    }

    fn worktree(&self, task: u64) -> PathBuf {
        self.dir.join(format!("home/worktrees/owner/repo/{task}"))
    }

    fn clone(&self) -> PathBuf {
        self.dir.join("home/repos/owner/repo")
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let output = self
            .command("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// Commits in the worktree of `task`, without pushing.
    fn work_on(&self, task: u64) {
        let args = [
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            &format!("work on {task}"),
        ];
        self.git(&self.worktree(task), &args);
    }

    fn push(&self, task: u64) {
        self.git(
            &self.worktree(task),
            &["push", "-q", "origin", &format!("priory/{task}")],
        );
    }

    /// Fast-forwards `main` of origin to the branch of `task`, as a merged pull request does.
    fn merge(&self, task: u64) {
        let bare = self.dir.join("origin/owner/repo.git");
        self.git(
            &bare,
            &[
                "update-ref",
                "refs/heads/main",
                &format!("refs/heads/priory/{task}"),
            ],
        );
    }

    fn branches(&self) -> String {
        self.git(&self.clone(), &["branch", "--list", "priory/*"])
    }

    /// Waits until the worktree of `task` is gone.
    fn removed(&self, task: u64) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self.worktree(task).exists() {
            assert!(Instant::now() < deadline, "worktree {task} stays");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[test]
fn a_sweep_removes_the_worktree_and_branch_of_a_done_task_whose_branch_merged() {
    let mut setup = Setup::ended("sweep-merged", 1);
    setup.work_on(1);
    setup.push(1);
    setup.merge(1);

    setup.start();

    setup.removed(1);
    let deadline = Instant::now() + Duration::from_secs(15);
    while !setup.branches().is_empty() {
        assert!(Instant::now() < deadline, "branch stays");
        std::thread::sleep(Duration::from_millis(50));
    }
}

impl Setup {
    /// The reason `priory desk` gives for keeping the worktree of `task`, if it lists one.
    fn stays(&self, task: u64) -> Option<String> {
        let wanted = self.worktree(task).display().to_string();
        self.rows("desk")
            .into_iter()
            .find(|row| row[1] == "worktree" && row[2] == wanted)
            .map(|row| row[5].clone())
    }

    /// Merges task 1, the control, starts the daemon, and waits until its sweep removed task 1.
    fn swept(&mut self) {
        self.work_on(1);
        self.push(1);
        self.merge(1);
        self.start();
        self.removed(1);
    }
}

/// Tasks 1 and 2 are done. The test spoils task 2 with `spoil`, which must leave it a worktree.
fn kept_by(name: &str, spoil: impl FnOnce(&Setup)) -> (Setup, Option<String>) {
    let mut setup = Setup::ended(name, 2);
    spoil(&setup);
    setup.swept();
    let reason = setup.stays(2);
    assert!(setup.worktree(2).is_dir());
    assert!(setup.branches().contains("priory/2"));
    (setup, reason)
}

#[test]
fn a_sweep_keeps_a_branch_that_is_not_merged_and_the_desk_says_so() {
    let (_setup, reason) = kept_by("sweep-unmerged", |setup| {
        setup.work_on(2);
        setup.push(2);
    });
    assert_eq!(reason.as_deref(), Some("not merged"));
}

#[test]
fn a_sweep_keeps_an_unpushed_commit_and_the_desk_says_so() {
    let (_setup, reason) = kept_by("sweep-unpushed", |setup| setup.work_on(2));
    assert_eq!(reason.as_deref(), Some("unpushed commits"));
}

#[test]
fn a_sweep_keeps_an_uncommitted_change_and_the_desk_says_so() {
    let (_setup, reason) = kept_by("sweep-dirty", |setup| {
        fs::write(setup.worktree(2).join("README.md"), "changed\n").unwrap();
    });
    assert_eq!(reason.as_deref(), Some("uncommitted changes"));
}

#[test]
fn a_sweep_keeps_an_untracked_file_and_the_desk_says_so() {
    let (_setup, reason) = kept_by("sweep-untracked", |setup| {
        fs::write(setup.worktree(2).join("notes.txt"), "x\n").unwrap();
    });
    assert_eq!(reason.as_deref(), Some("untracked files"));
}

#[test]
fn a_git_setting_that_hides_untracked_files_does_not_hide_them_from_the_sweep() {
    let (_setup, reason) = kept_by("sweep-hidden", |setup| {
        setup.git(
            &setup.clone(),
            &["config", "status.showUntrackedFiles", "no"],
        );
        fs::write(setup.worktree(2).join("notes.txt"), "x\n").unwrap();
    });
    assert_eq!(reason.as_deref(), Some("untracked files"));
}

#[test]
fn a_sweep_keeps_a_worktree_the_task_does_not_record() {
    let mut setup = Setup::ended("sweep-unrecorded", 1);
    let other = setup.dir.join("home/worktrees/other/repo/1");
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    setup.git(
        &setup.clone(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            other.to_str().unwrap(),
        ],
    );

    setup.start();
    setup.removed(1);

    let desk = setup.rows("desk");
    assert_eq!(desk[0][2], other.display().to_string());
    assert_eq!(desk[0][5], "not recorded as the worktree of task 1");
}

#[test]
fn a_sweep_keeps_a_worktree_a_process_runs_in() {
    let mut process = None;
    let (_setup, reason) = kept_by("sweep-process", |setup| {
        process = Some(
            Command::new("sleep")
                .arg("60")
                .current_dir(setup.worktree(2))
                .spawn()
                .unwrap(),
        );
    });
    let mut process = process.unwrap();
    process.kill().unwrap();
    process.wait().unwrap();
    assert_eq!(reason.as_deref(), Some("a process runs in it"));
}

#[test]
fn a_sweep_leaves_the_worktree_of_a_running_or_asleep_task() {
    let mut setup = Setup::new("sweep-active");
    let silent = serde_json::Value::Null;
    setup.calls(&[result("passed"), silent.clone(), silent, ask()]);
    setup.start();
    for (issue, state) in [(1, "done"), (2, "running"), (3, "asleep")] {
        setup.publish(&requested(issue));
        setup.until("tasks", |rows| {
            rows.len() == issue as usize && rows[issue as usize - 1][4] == state
        });
        if issue == 2 {
            // Task 2's silent turn and its nudge turn use up two calls before task 3 starts.
            setup.until("outbox", |rows| {
                rows.iter()
                    .filter(|row| row[1] == "2" && row[3] == "confirmed")
                    .count()
                    >= 2
            });
        }
    }
    setup.crash();

    setup.swept();

    assert_eq!(setup.stays(2).as_deref(), Some("task 2 still running"));
    assert_eq!(setup.stays(3).as_deref(), Some("task 3 still asleep"));
    assert!(setup.branches().contains("priory/2") && setup.branches().contains("priory/3"));
}

#[test]
fn a_sweep_removes_the_worktree_of_a_squash_merged_pull_request_and_git_keeps_the_branch() {
    let mut setup = Setup::ended("sweep-squashed", 1);
    setup.work_on(1);
    setup.push(1);
    let head = setup.git(&setup.worktree(1), &["rev-parse", "HEAD"]);
    setup.publish(&on_pull(
        "github.pull_request.closed",
        json!({ "merged": true, "head_sha": head.trim() }),
    ));
    setup.start();
    setup.until("inbox", |rows| {
        with(rows, 1, "github.pull_request.closed") == 1
    });
    setup.crash();

    setup.start();

    setup.removed(1);
    assert!(setup.branches().contains("priory/1"));
}

const LEAD: &str = "+++\ndescription = \"Leads every task.\"\nharness = \"claude-code\"\nsession = \"shared\"\n+++\nYou implement one issue.\n";

/// A setup whose implementer keeps one shared session across tasks.
fn shared(name: &str) -> Setup {
    let setup = Setup::new(name);
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), LEAD).unwrap();
    setup
}

#[test]
fn two_tasks_of_a_shared_agent_resume_one_session_one_after_the_other() {
    let mut setup = shared("shared-resume");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_eq!([&runs[0][1], &runs[0][7]], ["1", "done"], "{runs:?}");
    assert_eq!(runs[1][1], "2");
    assert_eq!(runs[0][4], runs[1][4], "one session serves both tasks");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert!(has(&launched(&setup), ["--resume", &runs[0][4]]));
    let typed = setup.typed();
    let worktree = setup.dir.join("home/worktrees/owner/repo/2");
    let intro = typed[1].lines().next().unwrap();
    for part in [
        "task 2",
        "github:owner/repo#43",
        "agent-label",
        worktree.to_str().unwrap(),
    ] {
        assert!(intro.contains(part), "{part} in {intro}");
    }
}

#[test]
fn a_task_waits_for_a_shared_agent_while_another_task_runs_it_then_starts() {
    let mut setup = shared("shared-wait");
    setup.calls(&[result("passed")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    let reason = "waits for agent implementer: its shared session serves task 1";
    let tasks = setup.until("tasks", |rows| rows.len() == 2 && rows[1][7] == reason);
    assert_eq!([&tasks[0][4], &tasks[1][4]], ["running", "queued"]);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("runs").len(), 1);
    setup.release("hold-stop");
    let tasks = setup.until("tasks", |rows| rows[1][4] == "running");
    assert_eq!([&tasks[0][4], &tasks[1][7]], ["done", "-"]);
}

#[test]
fn a_per_task_agent_gets_one_session_per_task() {
    let mut setup = Setup::new("per-task-sessions");
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    let runs = setup.until("runs", |rows| with(rows, 6, "-") == 2);
    assert_ne!(runs[0][4], runs[1][4]);
}

#[test]
fn an_agent_turned_back_to_per_task_starts_new_sessions_rather_than_resume_the_shared_one() {
    let mut setup = shared("unshared");
    setup.calls(&[ask(), ask()]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));
    setup.start();
    let runs = setup.until("runs", |rows| with(rows, 7, "asleep") == 2);
    let shared = runs[0][4].clone();
    setup.crash();
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), AGENT).unwrap();
    setup.start();

    let deadline = Instant::now() + Duration::from_secs(15);
    for ask in ["1", "2"] {
        while !setup
            .priory(&["answer", ask, "sqlite", "--revision", "1"])
            .status
            .success()
        {
            assert!(
                Instant::now() < deadline,
                "the daemon never took answer {ask}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    let runs = setup.until("runs", |rows| with(rows, 6, "-") == 2);
    let (first, second) = (&runs[2][4], &runs[3][4]);
    assert!(
        first != &shared && second != &shared && first != second,
        "{runs:?}"
    );
}

#[test]
fn a_listing_exits_quietly_when_its_reader_closes_the_pipe() {
    let setup = Setup::new("closed-pipe");
    let mut child = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .arg("desk")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

const SUPPORT_DAY: &str = r#"
source = "schedule"
repo = "owner/repo"
at = "09:00"
every = "1h"
until = "18:00"
key = "support-day"
prompt = "Sweep the support queue."
workflow = "implement"
"#;

fn support_day(name: &str) -> Setup {
    support_day_with(name, WORKFLOW)
}

fn support_day_with(name: &str, workflow: &str) -> Setup {
    let setup = Setup::with_workflow(name, workflow);
    fs::write(
        setup.dir.join("factory/triggers/support-day.toml"),
        SUPPORT_DAY,
    )
    .unwrap();
    setup
}

/// A firing of the `support-day` schedule, whose fixed key names the same task every time.
fn tick(scheduled: &str) -> Message {
    let key = "schedule:support-day";
    Message::new(
        "factory.task.requested",
        json!({
            "workflow": "implement",
            "key": key,
            "band": null,
            "lane": null,
            "trigger": "support-day",
            "source": format!("source-{scheduled}"),
            "scheduled": scheduled,
            "prompt": "Sweep the support queue."
        }),
    )
    .subject(key)
}

#[test]
fn a_fixed_key_firing_wakes_an_asleep_task_and_its_ask_stays_open() {
    let mut setup = support_day("tick-wakes");
    setup.calls(&[ask()]);
    setup.publish(&tick("2026-11-19T08:00:00Z"));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    setup.publish(&tick("2026-11-19T09:00:00Z"));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    let lines: Vec<_> = typed[1].lines().collect();
    assert_eq!(lines[0], "Sweep the support queue.", "{typed:?}");
    assert!(
        lines[1].starts_with("factory.task.requested schedule:support-day {")
            && lines[1].contains(r#""scheduled":"2026-11-19T09:00:00Z""#),
        "{typed:?}"
    );
    assert_eq!(setup.rows("tasks").len(), 1);
    assert_eq!(setup.rows("asks").len(), 1);

    // The tick's run sleeps again on the ask it inherits, so the next tick wakes it again.
    setup.until("tasks", |rows| rows[0][4] == "asleep");
    setup.publish(&tick("2026-11-19T10:00:00Z"));
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert_eq!(setup.rows("asks").len(), 1);
}

#[test]
fn a_firing_at_a_human_step_is_coalesced() {
    let mut setup = support_day_with("tick-human", STEPS);
    setup.calls(&[result("unsure")]);
    setup.publish(&tick("2026-11-19T08:00:00Z"));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    setup.publish(&tick("2026-11-19T09:00:00Z"));

    setup.until("inbox", |rows| rows.len() == 2);
    assert_eq!(setup.rows("inbox")[1][4], "coalesced");
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["approve", "asleep"]);
}

#[test]
fn a_firing_while_the_task_runs_is_coalesced_and_not_typed() {
    let mut setup = support_day("tick-coalesced");
    setup.publish(&tick("2026-11-19T08:00:00Z"));
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    setup.publish(&tick("2026-11-19T09:00:00Z"));

    setup.until("inbox", |rows| rows.len() == 2);
    assert_eq!(setup.rows("inbox")[1][4], "coalesced");
    setup.release("hold-stop");
    std::thread::sleep(Duration::from_millis(1500));
    let typed = setup.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
    assert!(typed[1].starts_with("Your turn ended"), "{typed:?}");
    assert!(!typed[1].contains("09:00"), "{typed:?}");
}

impl Setup {
    /// The records `priory trace` prints, one JSON event per line.
    fn trace(&self) -> Vec<serde_json::Value> {
        let output = self.priory(&["trace"]);
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[test]
fn each_step_that_ends_appends_one_step_ended_record_with_its_outcome_and_decider() {
    let setup = asked_to_open("trace", OPEN_PR);
    setup.commit(1, "widget.txt");

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("actions", |rows| rows.len() == 1);
    let records = setup.trace();
    let steps: Vec<_> = records
        .iter()
        .map(|record| {
            let data = &record["data"];
            [
                &data["step"],
                &data["decider"],
                &data["effect"],
                &data["outcome"],
            ]
            .map(|field| field.as_str().unwrap().to_owned())
        })
        .collect();
    assert_eq!(
        steps,
        [
            ["implement", "model", "reversible", "passed"],
            ["approve-pr", "human", "none", "open"],
            ["open-pr", "code", "irreversible", "opened"],
        ]
    );
    let model = &records[0];
    assert_eq!(model["specversion"], "1.0");
    assert_eq!(model["type"], "factory.step.ended");
    assert_eq!(model["datacontenttype"], "application/json");
    assert_eq!(model["source"], "factory://demo/runner");
    assert!(uuid::Uuid::parse_str(model["id"].as_str().unwrap()).is_ok());
    assert_ne!(model["id"], records[1]["id"]);
    let data = &model["data"];
    for (field, value) in [
        ("factory", json!("demo")),
        ("workflow", json!("implement")),
        ("task", json!("1")),
        ("key", json!("github:owner/repo#42")),
        ("run", json!("1")),
        ("generation", json!(1)),
        ("machine", json!("runner")),
        ("harness", json!("claude-code")),
    ] {
        assert_eq!(data[field], value, "{field} in {data}");
    }
    assert!(data["duration_ms"].is_u64(), "{data}");
    assert!(data["started"].is_string(), "{data}");
    for absent in ["lane", "factory_revision"] {
        assert!(data.get(absent).is_none(), "{absent} in {data}");
    }
    for later in &records[1..] {
        let data = &later["data"];
        for absent in ["run", "harness", "model", "tokens"] {
            assert!(data.get(absent).is_none(), "{absent} in {data}");
        }
    }
}

#[test]
fn a_model_step_record_sums_the_tokens_of_its_turns_from_the_session_transcript() {
    let mut setup = Setup::with_workflow("trace-tokens", STEPS);
    setup.calls(&[ask(), result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    setup.gone("priory-1");

    // The answer resumes the session, whose transcript already holds the first turn.
    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let records = setup.trace();
    let tokens: Vec<_> = records
        .iter()
        .map(|record| {
            let data = &record["data"];
            (
                data["step"].clone(),
                data["model"].clone(),
                data["tokens"].clone(),
            )
        })
        .collect();
    assert_eq!(
        tokens,
        [
            (
                json!("plan"),
                json!("claude-fake-5"),
                json!({ "input": 203, "output": 23, "cache_read": 2003, "cache_write": 103 })
            ),
            (
                json!("implement"),
                json!("claude-fake-5"),
                json!({ "input": 103, "output": 13, "cache_read": 1003, "cache_write": 53 })
            ),
        ]
    );
}

impl Setup {
    /// The rows `priory report` prints for the test's factory, each split on tabs, header included.
    fn report(&self, args: &[&str]) -> Vec<Vec<String>> {
        let factory = self.dir.join("factory");
        let mut all = vec!["report", factory.to_str().unwrap()];
        all.extend(args);
        let output = self.priory(&all);
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }
}

/// Runs task 1 through implement, review asking for changes, implement again and an approving review.
fn reviewed_twice(name: &str) -> Setup {
    let mut setup = Setup::with_workflow(name, TWO_AGENTS);
    setup.calls(&[
        result("passed"),
        result("changes"),
        result("passed"),
        result("approved"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    setup
}

#[test]
fn a_report_counts_each_steps_outcomes_with_the_runs_that_reached_the_step() {
    let setup = reviewed_twice("report");
    // A second task reaches implement and is still there.
    setup.publish(&requested(43));
    setup.until("runs", |rows| rows.iter().any(|row| row[1] == "2"));

    let report = setup.report(&["--workflow", "implement", "--since", "1 hour"]);

    assert_eq!(
        report,
        [
            ["step", "outcome", "count", "reached", "rate"],
            ["implement", "passed", "2", "3", "66.7%"],
            ["review", "approved", "1", "2", "50.0%"],
            ["review", "changes", "1", "2", "50.0%"],
        ]
    );
    let later = setup.report(&["--workflow", "implement", "--since", "2999-01-01T00:00:00Z"]);
    assert_eq!(later, [["step", "outcome", "count", "reached", "rate"]]);
}

#[test]
fn a_cost_report_prices_each_task_and_step_by_the_factory_price_table_and_names_a_missing_price() {
    let mut setup = Setup::with_workflow("report-cost", STEPS);
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let prices = setup.dir.join("factory/prices.toml");
    let header = [
        "task",
        "step",
        "model",
        "input",
        "output",
        "cache_read",
        "cache_write",
        "cost",
    ];

    fs::write(
        &prices,
        "[other-model]\ninput = 1.0\noutput = 1.0\ncache_read = 1.0\ncache_write = 1.0\n",
    )
    .unwrap();
    let missing = setup.report(&["--workflow", "implement", "--cost"]);

    let price = "price missing for claude-fake-5";
    assert_eq!(
        missing,
        [
            &header[..],
            &[
                "1",
                "plan",
                "claude-fake-5",
                "101",
                "11",
                "1001",
                "51",
                price
            ],
            &[
                "1",
                "implement",
                "claude-fake-5",
                "102",
                "12",
                "1002",
                "52",
                price
            ],
            &["1", "total", "-", "203", "23", "2003", "103", price],
        ]
    );

    // Dollars per million tokens of each kind.
    fs::write(
        &prices,
        "[claude-fake-5]\ninput = 3.0\noutput = 15.0\ncache_read = 0.3\ncache_write = 3.75\n",
    )
    .unwrap();
    let priced = setup.report(&["--workflow", "implement", "--cost"]);

    assert_eq!(
        priced,
        [
            &header[..],
            &[
                "1",
                "plan",
                "claude-fake-5",
                "101",
                "11",
                "1001",
                "51",
                "$0.000960"
            ],
            &[
                "1",
                "implement",
                "claude-fake-5",
                "102",
                "12",
                "1002",
                "52",
                "$0.000982"
            ],
            &["1", "total", "-", "203", "23", "2003", "103", "$0.001941"],
        ]
    );
}

#[test]
fn a_cost_report_keeps_a_model_step_whose_tokens_are_unknown_and_marks_its_total() {
    let mut setup = Setup::with_workflow("report-unknown", STEPS);
    setup.hold("no-transcript");
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });

    let report = setup.report(&["--workflow", "implement", "--cost"]);

    let unknown = "tokens unknown";
    assert_eq!(
        report[1..],
        [
            ["1", "plan", "haiku", "-", "-", "-", "-", unknown],
            ["1", "implement", "haiku", "-", "-", "-", "-", unknown],
            [
                "1",
                "total",
                "-",
                "0",
                "0",
                "0",
                "0",
                "tokens unknown for plan, implement"
            ],
        ]
    );
}

/// Runs `priory <verb> <args>` and returns its stdout, failing on a non-zero exit.
fn person(setup: &Setup, verb: &str, args: &[&str]) -> String {
    let output = setup.priory(&[&[verb], args].concat());
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// A running daemon with task 1 queued, which no run can start until memory comes back.
fn queued_without_room(name: &str) -> Setup {
    let mut setup = Setup::new(name);
    setup.memory(0);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| rows.len() == 1);
    setup
}

/// The state and reason of task `id`.
fn held_as(setup: &Setup, id: &str) -> [String; 2] {
    let tasks = setup.rows("tasks");
    let row = tasks.iter().find(|row| row[0] == id).unwrap();
    [row[4].clone(), row[7].clone()]
}

#[test]
fn a_held_task_never_starts_until_a_person_releases_it() {
    let setup = queued_without_room("hold-person");

    let reply = person(
        &setup,
        "hold",
        &["1", "--reason", "waits for the design review"],
    );
    setup.memory(60);
    std::thread::sleep(Duration::from_millis(2100));

    let reason = "held: waits for the design review, until a person releases it";
    assert_eq!(reply, format!("task 1 {reason}\n"));
    assert_eq!(held_as(&setup, "1"), ["held", reason]);
    assert!(setup.rows("runs").is_empty());
    let desk = setup.rows("desk");
    let held: Vec<_> = desk.iter().map(|row| &row[1..]).collect();
    assert_eq!(held, [["held", "1", "1", "none/misc", reason, "1"]]);

    assert_eq!(person(&setup, "release", &["1"]), "task 1 is queued\n");
    setup.until("tasks", |rows| rows[0][4] == "running");
}

#[test]
fn a_time_hold_releases_the_task_once_its_time_passes() {
    let setup = queued_without_room("hold-time");
    let past = person(
        &setup,
        "hold",
        &["1", "--reason", "late", "--until", "2000-01-01T00:00:00Z"],
    );
    assert_eq!(
        past,
        "task 1 held: late, until 2000-01-01T00:00:00.000Z; released at once: 2000-01-01T00:00:00.000Z passed\n"
    );
    assert_eq!(held_as(&setup, "1")[0], "queued");
    setup.memory(60);

    let reply = person(
        &setup,
        "hold",
        &["1", "--reason", "after lunch", "--until", "2 seconds"],
    );

    assert!(
        reply.starts_with("task 1 held: after lunch, until 20"),
        "{reply}"
    );
    assert_eq!(held_as(&setup, "1")[0], "held");
    setup.until("tasks", |rows| rows[0][4] == "running");
}

#[test]
fn a_task_hold_releases_the_task_when_the_other_task_ends() {
    let mut setup = Setup::new("hold-task");
    setup.memory(12);
    setup.hold("hold-stop");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));
    setup.start();
    setup.until("tasks", |rows| rows.len() == 2 && rows[0][4] == "running");

    let reply = person(
        &setup,
        "hold",
        &["2", "--reason", "same files", "--after", "1"],
    );
    setup.release("hold-stop");

    assert_eq!(reply, "task 2 held: same files, until task 1 ends\n");
    setup.until("tasks", |rows| {
        with(rows, 4, "done") == 1 && with(rows, 4, "running") == 1
    });
}

#[test]
fn a_running_task_cannot_be_held() {
    let mut setup = Setup::new("hold-running");
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "running")
    });

    let refused = setup.priory(&["hold", "1", "--reason", "too late"]);

    assert!(!refused.status.success());
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "priory hold: task 1 is running: only a queued or ready task can be held\n"
    );
}

/// Pull request 7 closed, from a branch no task owns.
fn closed_7(merged: bool, github_id: u64) -> Message {
    on_pull(
        "github.pull_request.closed",
        json!({ "merged": merged, "github_id": github_id, "head_branch": "feature" }),
    )
}

#[test]
fn a_merge_hold_releases_the_task_when_its_pull_request_merges() {
    let setup = queued_without_room("hold-merge");
    let reply = person(
        &setup,
        "hold",
        &["1", "--reason", "needs the schema", "--merge", "7"],
    );
    setup.memory(60);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(held_as(&setup, "1")[0], "held");

    setup.publish(&closed_7(true, 400));

    assert_eq!(
        reply,
        "task 1 held: needs the schema, until github:owner/repo#7 merges, and asks if it closes unmerged\n"
    );
    setup.until("tasks", |rows| rows[0][4] == "running");
}

#[test]
fn a_pull_request_closed_unmerged_drops_or_asks_as_its_hold_says() {
    let setup = queued_without_room("hold-closed");
    setup.publish(&requested(43));
    setup.until("tasks", |rows| rows.len() == 2);
    let merge = ["--merge", "owner/repo#7", "--if-closed"];
    person(
        &setup,
        "hold",
        &[&["1", "--reason", "a"], &merge[..], &["drop"]].concat(),
    );
    person(
        &setup,
        "hold",
        &[&["2", "--reason", "b"], &merge[..], &["ask"]].concat(),
    );

    setup.publish(&closed_7(false, 400));

    setup.until("tasks", |rows| rows[0][4] == "dropped");
    assert_eq!(
        held_as(&setup, "1"),
        ["dropped", "github:owner/repo#7 closed without a merge"]
    );
    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(
        asks[0][1..],
        [
            "2",
            "1",
            "Pull request github:owner/repo#7 closed without a merge, and task 2 was held until it merged: b. Release the task, or drop it?",
            r#"["release","drop"]"#
        ]
    );
    assert_eq!(held_as(&setup, "2")[0], "held");

    person(
        &setup,
        "answer",
        &[&asks[0][0], "release", "--revision", "1"],
    );

    // The next tick gives the released task its reason to wait for room.
    assert_eq!(held_as(&setup, "2")[0], "queued");
}

/// A Linear event about ENG-7 whose open blockers are `blockers`.
fn linear_7(kind: &str, blockers: &[&str]) -> Message {
    Message::new(
        kind,
        json!({ "issue": "ENG-7", "linear_id": "issue-7", "state": "Todo", "blockers": blockers }),
    )
    .subject("ENG-7")
}

#[test]
fn a_blockers_hold_releases_the_task_when_its_issue_has_no_open_blocker_left() {
    let mut setup = Setup::new("hold-blockers");
    setup.memory(0);
    // A view event carries no blockers: the hold waits for an event that does.
    let mut source = linear_7("linear.issue.entered", &[]);
    source.data.as_object_mut().unwrap().remove("blockers");
    let mut request = requested(42);
    request.data["key"] = json!("linear:ENG-7");
    request.data["source"] = json!(source.id.to_string());
    setup.publish(&source);
    setup.publish(&request);
    setup.publish(&requested(43));
    setup.start();
    setup.until("tasks", |rows| rows.len() == 2);

    let reply = person(&setup, "hold", &["1", "--reason", "blocked", "--blockers"]);
    let refused = setup.priory(&["hold", "2", "--reason", "blocked", "--blockers"]);
    setup.memory(60);
    setup.publish(&linear_7("linear.issue.blockers", &["ENG-4"]));
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(held_as(&setup, "1")[0], "held");
    setup.publish(&linear_7("linear.issue.blockers", &[]));

    assert_eq!(
        reply,
        "task 1 held: blocked, until linear:ENG-7 has no open blocker\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "priory hold: task 2 comes from no Linear issue, so Priory cannot see its blockers\n"
    );
    setup.until("tasks", |rows| rows[0][4] == "running");
}

const REVIVED: &str = "A person revived this task as generation 2 after it was dropped. The asks and waits of its earlier generation are withdrawn. Check the worktree, then continue.\nPlan the change.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n";

#[test]
fn a_dropped_task_revives_at_its_step_in_its_session_as_generation_2() {
    let mut setup = Setup::with_workflow("revive", STEPS);
    setup.calls(&[result("hopeless"), result("ready")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "dropped")
    });
    let session = setup.rows("runs")[0][4].clone();
    setup.gone("priory-1");

    let revived = setup.priory(&["revive", "1"]);

    assert!(revived.status.success(), "{revived:?}");
    assert_eq!(
        String::from_utf8_lossy(&revived.stdout),
        "task 1 is ready at plan, generation 2\n"
    );
    let tasks = setup.until("tasks", |rows| rows[0][3] == "implement");
    assert_eq!([&tasks[0][4], &tasks[0][9]], ["running", "2"]);
    assert_eq!(setup.rows("runs")[1][4], session);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    assert!(args.contains(&format!("--resume\n{session}\n")), "{args}");
    assert_eq!(setup.typed()[1], REVIVED);
    let ended: Vec<_> = setup
        .trace()
        .iter()
        .map(|record| {
            (
                record["data"]["outcome"].clone(),
                record["data"]["generation"].clone(),
            )
        })
        .collect();
    assert_eq!(
        ended,
        [(json!("hopeless"), json!(1)), (json!("ready"), json!(2))]
    );
}

#[test]
fn an_answer_to_the_ask_of_an_earlier_generation_is_refused() {
    let mut setup = Setup::with_workflow("revive-ask", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    assert!(setup.priory(&["cancel", "1"]).status.success());
    assert!(setup.priory(&["revive", "1"]).status.success());
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(!answered.status.success());
    assert_eq!(
        String::from_utf8_lossy(&answered.stderr),
        "priory answer: ask 1 was withdrawn when task 1 was revived as generation 2\n"
    );
    assert!(setup.rows("asks").is_empty());
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn an_event_for_the_wait_of_an_earlier_generation_does_not_wake_the_revived_task() {
    let mut setup = Setup::new("revive-wait");
    setup.calls(&[wait(on_pull_7("pull_request")), ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    assert!(setup.priory(&["cancel", "1"]).status.success());
    assert!(setup.priory(&["revive", "1"]).status.success());
    setup.until("runs", |rows| {
        rows.get(1).is_some_and(|row| row[7] == "asleep")
    });
    setup.gone("priory-1");
    assert!(setup.rows("waits").is_empty());
    let asked: Vec<_> = setup
        .rows("desk")
        .into_iter()
        .filter(|row| row[1] == "ask")
        .map(|row| [row[2].clone(), row[6].clone()])
        .collect();
    assert_eq!(asked, [["1", "2"]]);

    setup.publish(&closed());

    setup.until("inbox", |rows| with(rows, 4, "kept") == 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
    assert_eq!(setup.rows("runs").len(), 2);
}

#[test]
fn a_revive_on_a_key_that_another_task_holds_is_refused() {
    let mut setup = Setup::new("revive-key");
    setup.calls(&[result("failed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "dropped")
    });
    setup.publish(&requested(42));
    setup.until("tasks", |rows| rows.len() == 2);

    let revived = setup.priory(&["revive", "1"]);

    assert!(!revived.status.success());
    assert_eq!(
        String::from_utf8_lossy(&revived.stderr),
        "priory revive: task 2 is already active on key github:owner/repo#42\n"
    );
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][4], &tasks[0][9]], ["dropped", "1"]);
}

#[test]
fn a_failed_task_offers_revive_on_the_desk_and_revives_at_a_chosen_step_in_its_recreated_worktree()
{
    let mut setup = Setup::with_workflow("revive-failed", STEPS);
    setup.calls(&[result("ready"), result("failed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "dropped")
    });
    setup.gone("priory-1");
    let commit = setup.commit(1, "widget.txt");
    // Nothing fails a task yet (R7.5 will), so the test fails it by hand.
    rusqlite::Connection::open(setup.dir.join("priory.db"))
        .unwrap()
        .execute(
            "UPDATE tasks SET state = 'failed', reason = 'the repair found no cause' WHERE id = 1",
            [],
        )
        .unwrap();
    let clone = setup.dir.join("home/repos/owner/repo");
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    assert!(
        git(&clone)
            .args(["worktree", "remove", "--force"])
            .arg(&worktree)
            .status()
            .unwrap()
            .success()
    );
    let failed: Vec<_> = setup
        .rows("desk")
        .into_iter()
        .filter(|row| row[1] == "failed")
        .collect();
    assert_eq!(
        [&failed[0][2], &failed[0][3], &failed[0][5], &failed[0][6]],
        [
            "1",
            "1",
            "the repair found no cause; revive: priory revive 1",
            "1"
        ]
    );

    let revived = setup.priory(&["revive", "1", "--step", "plan"]);

    assert!(revived.status.success(), "{revived:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert_eq!(
        setup.typed()[2],
        "A person revived this task as generation 2 after it was failed: the repair found no cause. The asks and waits of its earlier generation are withdrawn. Check the worktree, then continue.\nPlan the change.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n"
    );
    let tasks = setup.rows("tasks");
    assert_eq!(
        [&tasks[0][3], &tasks[0][4], &tasks[0][9]],
        ["plan", "running", "2"]
    );
    let head = git(&worktree).args(["rev-parse", "HEAD"]).output().unwrap();
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), commit);
    assert!(!setup.rows("desk").iter().any(|row| row[1] == "failed"));
}

#[test]
fn a_report_by_lane_counts_each_lane_apart() {
    let mut setup = Setup::with_workflow("report-lane", TWO_AGENTS);
    setup.calls(&[
        result("passed"),
        result("changes"),
        result("passed"),
        result("approved"),
    ]);
    let in_lane = |issue, lane: &str| {
        let mut request = requested(issue);
        request.data["lane"] = json!(lane);
        request
    };
    setup.publish(&in_lane(42, "3D/mobile"));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    setup.publish(&in_lane(43, "FND/desktop"));
    setup.until("runs", |rows| rows.iter().any(|row| row[1] == "2"));

    let report = setup.report(&["--workflow", "implement", "--by", "lane"]);

    assert_eq!(
        report,
        [
            ["lane", "step", "outcome", "count", "reached", "rate"],
            ["3D/mobile", "implement", "passed", "2", "2", "100.0%"],
            ["3D/mobile", "review", "approved", "1", "2", "50.0%"],
            ["3D/mobile", "review", "changes", "1", "2", "50.0%"],
        ]
    );
}

#[test]
fn a_revive_at_an_irreversible_step_asks_its_withdrawn_approval_again() {
    let setup = asked_to_open("revive-approval", APPROVE_THEN_POLISH);
    setup.calls(&[passed_with_evidence()]);
    setup.hold("hold-stop");
    assert!(
        setup
            .priory(&["answer", "1", "open", "--revision", "1"])
            .status
            .success()
    );
    setup.written("fake/tools.log");
    while setup.replies().len() < 2 {
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.commit(1, "polish.txt");
    setup.release("hold-stop");
    setup.until("asks", |rows| rows.first().is_some_and(|row| row[2] == "2"));
    assert!(setup.priory(&["cancel", "1"]).status.success());

    let revived = setup.priory(&["revive", "1", "--step", "open-pr"]);

    assert!(revived.status.success(), "{revived:?}");
    let asks = setup.until("asks", |rows| rows.first().is_some_and(|row| row[2] == "3"));
    assert_eq!(asks[0][..2], ["1", "1"]);
    setup.calls(&[passed_with_evidence()]);
    let answered = setup.priory(&["answer", "1", "open", "--revision", "3"]);
    assert!(answered.status.success(), "{answered:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4], "opened");
}

const NUDGE: &str = "Your turn ended without calling wait, ask or result of the priory MCP server. Call one of them: result to end the step, ask to put a question to a person, or wait to sleep until an event.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n";

#[test]
fn a_silent_turn_gets_one_nudge_and_a_second_silent_turn_gets_none() {
    let mut setup = Setup::new("nudge-once");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);

    std::thread::sleep(Duration::from_millis(2500));

    assert_eq!(setup.typed()[1], NUDGE);
    assert_eq!(setup.rows("outbox").len(), 2);
    assert_eq!(setup.typed().len(), 2);
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_turn_that_called_wait_gets_no_nudge() {
    let setup = asleep_on("nudge-wait", on_pull_7("checks"));

    std::thread::sleep(Duration::from_millis(1500));

    assert_eq!(setup.rows("outbox").len(), 1);
    assert_eq!(setup.typed().len(), 1);
}

#[test]
fn a_turn_a_person_typed_into_the_pane_gets_no_nudge() {
    let mut setup = Setup::new("nudge-person");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    std::thread::sleep(Duration::from_millis(1000));

    assert!(
        setup
            .tmux(&["set-buffer", "hello from a person"])
            .status
            .success()
    );
    assert!(
        setup
            .tmux(&["paste-buffer", "-p", "-t", "priory-1"])
            .status
            .success()
    );
    assert!(
        setup
            .tmux(&["send-keys", "-t", "priory-1", "Enter"])
            .status
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while setup.typed().len() < 3 {
        assert!(
            Instant::now() < deadline,
            "the person's prompt never arrived"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(2500));

    assert_eq!(setup.typed().len(), 3, "{:?}", setup.typed());
    assert_eq!(setup.rows("outbox").len(), 2);
}

fn linear_event(kind: &str, issue: &str, extra: serde_json::Value) -> Message {
    let mut data = json!({
        "issue": issue,
        "linear_id": format!("{issue}-{kind}"),
        "title": "Fix the thing",
        "url": format!("https://linear.app/acme/issue/{issue}"),
        "team": "ENG",
        "project": null,
        "state": "In Progress",
        "labels": [],
        "time": herdr_bus::Timestamp::now().to_string(),
        "baseline": false,
        "triggers": [],
    });
    data.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    Message::new(kind, data).subject(issue)
}

#[test]
fn a_linear_comment_wait_fires_on_a_comment_on_its_issue_and_not_another() {
    let setup = asleep_on(
        "linear-comment",
        json!({ "type": "linear_comment", "issue": "ENG-7" }),
    );
    assert_eq!(
        setup.rows("waits")[0][3..5],
        ["linear_comment", "linear:ENG-7"]
    );

    setup.publish(&linear_event(
        "linear.comment.created",
        "ENG-8",
        json!({ "body": "elsewhere" }),
    ));
    stays_asleep(&setup);
    setup.publish(&linear_event(
        "linear.comment.created",
        "ENG-7",
        json!({ "body": "Please cover the empty case." }),
    ));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(
        typed[1].starts_with("linear.comment.created ENG-7 {")
            && typed[1].contains("Please cover the empty case."),
        "{typed:?}"
    );
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_linear_state_wait_fires_when_its_issue_moves_to_its_state_and_not_otherwise() {
    let setup = asleep_on(
        "linear-state",
        json!({ "type": "linear_state", "issue": "ENG-7", "state": "Done" }),
    );
    let moved = |issue: &str, to: &str| {
        linear_event(
            "linear.issue.state_changed",
            issue,
            json!({ "linear_id": format!("{issue}-{to}"), "from": "In Progress", "to": to }),
        )
    };

    setup.publish(&moved("ENG-8", "Done"));
    setup.publish(&moved("ENG-7", "In Review"));
    stays_asleep(&setup);
    setup.publish(&moved("ENG-7", "Done"));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    let lines: Vec<_> = typed[1].lines().collect();
    assert!(
        lines[0].starts_with("linear.issue.state_changed ENG-7 {")
            && lines[0].contains(r#""to":"Done""#),
        "{typed:?}"
    );
    assert!(lines[1].contains(r#""to":"In Review""#), "{typed:?}");
}

const THREAD: &str = "1763500000.000100";

/// A reply in `thread` of channel C0123, posted at `time`.
fn replied(thread: &str, ts: &str, time: herdr_bus::Timestamp) -> Message {
    Message::new(
        "slack.thread.replied",
        json!({ "channel": "C0123", "thread_ts": thread, "ts": ts, "user": "U0PERSON",
                "text": format!("reply {ts}"), "time": time.to_string() }),
    )
    .subject(format!("C0123:{thread}"))
    .time(time)
}

#[test]
fn a_slack_reply_wait_fires_on_a_new_reply_in_its_thread_only() {
    let setup = asleep_on(
        "slack-reply",
        json!({ "type": "slack_reply", "channel": "C0123", "thread_ts": THREAD }),
    );
    assert_eq!(
        setup.rows("waits")[0][3..5],
        ["slack_reply", format!("slack:C0123:{THREAD}").as_str()]
    );
    let now = herdr_bus::Timestamp::now();
    let hour_ago = now.checked_sub(Duration::from_secs(3600)).unwrap();

    setup.publish(&replied("1763500000.000200", "1763500001.000001", now));
    setup.publish(&replied(THREAD, "1763500001.000002", hour_ago));
    stays_asleep(&setup);
    setup.publish(&replied(THREAD, "1763500001.000003", now));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(
        typed[1].starts_with(&format!("slack.thread.replied C0123:{THREAD} {{"))
            && typed[1].contains("reply 1763500001.000003"),
        "{typed:?}"
    );
}

const SLACK_TOKEN: &str = "xoxb-secret-test-token";

impl Setup {
    /// Makes the fake `curl` answer each call whose arguments contain a pattern of `answers`, and
    /// record its arguments and standard input.
    fn answer_curl(&self, answers: &[(&str, serde_json::Value)]) {
        let bin = self.dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let mut script = format!(
            "#!/bin/sh\necho \"$*\" >> '{dir}/curl-args'\ncat >> '{dir}/curl-stdin'\ncase \"$*\" in\n",
            dir = self.dir.display()
        );
        for (index, (pattern, answer)) in answers.iter().enumerate() {
            let file = self.dir.join(format!("curl-answer-{index}.json"));
            fs::write(&file, answer.to_string()).unwrap();
            script.push_str(&format!("  *'{pattern}'*) cat '{}' ;;\n", file.display()));
        }
        script.push_str("  *) echo \"unexpected call $*\" >&2; exit 2 ;;\nesac\n");
        let curl = bin.join("curl");
        fs::write(&curl, script).unwrap();
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Makes the fake `curl` answer Slack's Web API: `auth.test` as bot U0BOT, and each
    /// `conversations.replies` with `replies`, the thread's first message included.
    fn answer_slack(&self, replies: &[serde_json::Value]) {
        let mut messages =
            vec![json!({ "ts": THREAD, "thread_ts": THREAD, "user": "U0PERSON", "text": "first" })];
        messages.extend_from_slice(replies);
        self.answer_curl(&[
            (
                "auth.test",
                json!({ "ok": true, "user_id": "U0BOT", "bot_id": "B0BOT" }),
            ),
            (
                "conversations.replies",
                json!({ "ok": true, "messages": messages, "has_more": false }),
            ),
        ]);
    }

    /// Runs one poll of `priory intake` on a factory with no trigger, against the fakes in `bin`.
    fn intake(&self) {
        let factory = self.dir.join("intake-factory");
        for (path, content) in [
            ("factory.toml", "name = \"demo\""),
            ("agents/implementer/agent.md", AGENT),
            ("workflows/implement.toml", WORKFLOW),
        ] {
            let file = factory.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let path = std::env::join_paths(
            std::iter::once(self.dir.join("bin"))
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let output = self
            .command(env!("CARGO_BIN_EXE_priory"))
            .args(["intake", "--once"])
            .arg(&factory)
            .env("PATH", path)
            .env("SLACK_BOT_TOKEN", SLACK_TOKEN)
            .env("LINEAR_API_KEY", "lin_api_secret_test_key")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "intake: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// A message in the thread, by `user`, posted at `ts`.
fn slack_message(ts: &str, user: &str) -> serde_json::Value {
    json!({ "ts": ts, "thread_ts": THREAD, "user": user, "text": format!("said {ts}") })
}

/// Slack's `ts` for `time`: seconds since the epoch, with six decimals.
fn ts(time: herdr_bus::Timestamp) -> String {
    format!("{}.{:06}", time.as_second(), time.subsec_microsecond())
}

#[test]
fn a_reply_posted_before_the_wait_wakes_the_task_and_the_bots_own_reply_never_does() {
    let setup = asleep_on(
        "slack-intake",
        json!({ "type": "slack_reply", "channel": "C0123", "thread_ts": THREAD }),
    );
    // The second the wait was stored in: after the agent's last prompt, before the intake polls.
    let stored: herdr_bus::Timestamp = setup.rows("waits")[0][5].parse().unwrap();
    let before_prompt = stored.checked_sub(Duration::from_secs(3600)).unwrap();
    let old = slack_message(&ts(before_prompt), "U0PERSON");
    let mine = slack_message(&ts(herdr_bus::Timestamp::now()), "U0BOT");
    setup.answer_slack(&[old.clone(), mine.clone()]);

    setup.intake();
    stays_asleep(&setup);
    let person = stored;
    setup.answer_slack(&[old, slack_message(&ts(person), "U0PERSON"), mine]);
    setup.intake();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(
        typed[1].starts_with(&format!("slack.thread.replied C0123:{THREAD} {{"))
            && typed[1].contains(&format!("said {}", ts(person))),
        "{typed:?}"
    );
    let stdin = fs::read_to_string(setup.dir.join("curl-stdin")).unwrap();
    assert!(stdin.contains(SLACK_TOKEN));
    let args = fs::read_to_string(setup.dir.join("curl-args")).unwrap();
    assert!(!args.contains(SLACK_TOKEN), "{args}");
}

/// A Linear GraphQL answer holding `nodes` at `field`.
fn linear_page(field: &str, nodes: serde_json::Value) -> serde_json::Value {
    json!({ "data": { field: { "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } } })
}

fn linear_comment(issue: &str, body: &str) -> serde_json::Value {
    json!({
        "id": format!("comment-{issue}"),
        "body": body,
        "url": format!("https://linear.app/acme/issue/{issue}#comment"),
        "createdAt": herdr_bus::Timestamp::now().to_string(),
        "user": { "displayName": "max" },
        "issue": {
            "id": format!("issue-{issue}"), "identifier": issue, "title": "Fix the thing",
            "url": format!("https://linear.app/acme/issue/{issue}"),
            "team": { "key": "ENG" }, "project": null, "state": { "name": "In Progress" },
            "labels": { "nodes": [] }
        }
    })
}

#[test]
fn a_linear_wait_makes_an_intake_without_linear_triggers_poll_linear_for_its_issue() {
    let setup = asleep_on(
        "linear-intake",
        json!({ "type": "linear_comment", "issue": "ENG-7" }),
    );
    setup.answer_curl(&[
        ("query Issues", linear_page("issues", json!([]))),
        (
            "query Comments",
            linear_page(
                "comments",
                json!([
                    linear_comment("ENG-8", "elsewhere"),
                    linear_comment("ENG-7", "Covered.")
                ]),
            ),
        ),
    ]);

    setup.intake();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let typed = setup.typed();
    assert!(
        typed[1].starts_with("linear.comment.created ENG-7 {") && typed[1].contains("Covered."),
        "{typed:?}"
    );
    assert!(!typed[1].contains("elsewhere"), "{typed:?}");
}

/// Waits until the daemon answers on its socket.
fn listening(setup: &Setup) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !setup.dir.join("home/priory.sock").exists() {
        assert!(Instant::now() < deadline, "the daemon never listened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Starts `priory memory acquire --gb <gb> -- sh -c <script>` in the test's `fake` directory.
fn acquire(setup: &Setup, gb: &str, script: &str) -> Child {
    setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["memory", "acquire", "--gb", gb, "--", "sh", "-c", script])
        .current_dir(setup.dir.join("fake"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

#[test]
fn a_gated_command_runs_and_its_exit_code_passes_through() {
    let mut setup = Setup::new("gate-exit");
    setup.start();
    listening(&setup);

    let done = acquire(&setup, "8", "echo built; exit 7")
        .wait_with_output()
        .unwrap();

    assert_eq!(
        done.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&done.stdout), "built\n");
    assert!(setup.rows("memory").is_empty());
}

/// A command that logs its start, runs while the file `hold-<name>` exists, then logs its end.
fn held(name: &str) -> String {
    format!(
        "echo start-{name} >> order; while [ -e hold-{name} ]; do sleep 0.05; done; echo end-{name} >> order"
    )
}

fn order(setup: &Setup) -> Vec<String> {
    fs::read_to_string(setup.dir.join("fake/order"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn acquires_that_do_not_fit_together_run_one_after_the_other_in_fifo_order() {
    let mut setup = Setup::new("gate-fifo");
    setup.start();
    listening(&setup);
    setup.hold("hold-a");
    setup.hold("hold-b");

    let a = acquire(&setup, "40", &held("a"));
    setup.until("memory", |rows| rows.len() == 1 && rows[0][2] == "held");
    let b = acquire(&setup, "40", &held("b"));
    setup.until("memory", |rows| rows.len() == 2);
    // 10 GB fits beside a, but waits behind b.
    let c = acquire(&setup, "10", &held("c"));
    setup.until("memory", |rows| rows.len() == 3);
    std::thread::sleep(Duration::from_millis(1000));

    let rows: Vec<_> = setup
        .rows("memory")
        .into_iter()
        .map(|row| [row[2].clone(), row[3].clone(), row[4].clone()])
        .collect();
    assert_eq!(
        rows,
        [
            [
                "held".to_owned(),
                "40.0".to_owned(),
                format!("sh -c {}", held("a"))
            ],
            [
                "waiting".to_owned(),
                "40.0".to_owned(),
                format!("sh -c {}", held("b"))
            ],
            [
                "waiting".to_owned(),
                "10.0".to_owned(),
                format!("sh -c {}", held("c"))
            ],
        ]
    );
    assert_eq!(order(&setup), ["start-a"]);
    let desk: Vec<_> = setup
        .rows("desk")
        .into_iter()
        .filter(|row| row[1] == "memory")
        .map(|row| [row[3].clone(), row[5].clone()])
        .collect();
    assert_eq!(
        desk,
        [
            [
                "-".to_owned(),
                format!("waits for 40.0 GB: sh -c {}", held("b"))
            ],
            [
                "-".to_owned(),
                format!("waits for 10.0 GB: sh -c {}", held("c"))
            ],
        ]
    );
    setup.release("hold-a");
    assert!(a.wait_with_output().unwrap().status.success());
    setup.until("memory", |rows| rows.iter().all(|row| row[2] == "held"));
    setup.release("hold-b");
    assert!(b.wait_with_output().unwrap().status.success());
    assert!(c.wait_with_output().unwrap().status.success());
    let order = order(&setup);
    let at = |line: &str| order.iter().position(|logged| logged == line).unwrap();
    assert_eq!(&order[..2], ["start-a", "end-a"]);
    assert!(at("start-b") > at("end-a") && at("start-c") > at("end-a"));
    assert!(setup.rows("memory").is_empty());
}

#[test]
fn a_grant_is_released_when_its_command_fails_or_is_killed_or_its_holder_dies() {
    let mut setup = Setup::new("gate-release");
    setup.start();
    listening(&setup);

    let failed = acquire(&setup, "40", "exit 1").wait_with_output().unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(setup.rows("memory").is_empty());
    let killed = acquire(&setup, "40", "kill -9 $$")
        .wait_with_output()
        .unwrap();
    assert_eq!(killed.status.code(), Some(137));
    assert!(setup.rows("memory").is_empty());

    setup.hold("hold-orphan");
    let mut holder = acquire(&setup, "40", &held("orphan"));
    setup.written("fake/order");
    holder.kill().unwrap();
    holder.wait().unwrap();
    // The orphaned command still runs, so its grant still holds.
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("memory")[0][2], "held");
    setup.release("hold-orphan");
    setup.until("memory", |rows| rows.is_empty());
    let next = acquire(&setup, "40", "exit 0").wait_with_output().unwrap();

    assert!(
        next.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&next.stderr)
    );
    assert!(setup.rows("memory").is_empty());
}

#[test]
fn without_a_daemon_the_command_fails_fast_and_never_runs() {
    let setup = Setup::new("gate-no-daemon");

    let done = acquire(&setup, "1", "echo ran > ran")
        .wait_with_output()
        .unwrap();

    assert_eq!(done.status.code(), Some(125));
    let stderr = String::from_utf8_lossy(&done.stderr);
    assert!(
        stderr.contains("no priory daemon listens") && stderr.contains("the command did not run"),
        "{stderr}"
    );
    assert!(!setup.dir.join("fake/ran").exists());
}

#[test]
fn an_acquire_that_waits_past_its_timeout_gives_up_and_leaves_no_grant() {
    let mut setup = Setup::new("gate-timeout");
    setup.start();
    listening(&setup);
    setup.hold("hold-a");
    let a = acquire(&setup, "40", &held("a"));
    setup.until("memory", |rows| rows.len() == 1 && rows[0][2] == "held");

    let late = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args([
            "memory",
            "acquire",
            "--gb",
            "40",
            "--timeout",
            "1s",
            "--",
            "true",
        ])
        .output()
        .unwrap();
    setup.release("hold-a");

    assert_eq!(late.status.code(), Some(125));
    assert!(a.wait_with_output().unwrap().status.success());
    assert!(setup.rows("memory").is_empty());
}

#[test]
fn a_run_waits_for_room_while_a_grant_holds_memory() {
    let mut setup = Setup::new("gate-run");
    setup.start();
    listening(&setup);
    setup.hold("hold-a");
    let a = acquire(&setup, "50", &held("a"));
    setup.until("memory", |rows| rows.len() == 1 && rows[0][2] == "held");

    setup.publish(&requested(42));

    let reason = "waits for room: 10.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";
    setup.until("tasks", |rows| rows.len() == 1 && rows[0][7] == reason);
    assert!(setup.rows("runs").is_empty());
    setup.release("hold-a");
    assert!(a.wait_with_output().unwrap().status.success());
    setup.until("tasks", |rows| rows[0][4] == "running");
}

/// Task 1 sleeps on pull request 7 with a commit on its branch, then the disk runs short while task
/// 2 waits for room, so the disk stall sweeps.
fn asleep_when_disk_runs_short(name: &str, spoil: impl FnOnce(&Setup)) -> (Setup, String) {
    let setup = asleep_on(name, on_pull_7("pull_request"));
    let commit = setup.commit(1, "widget.txt");
    spoil(&setup);
    setup.disk(10);
    setup.publish(&requested(43));
    setup.until_banner(|banner| banner.is_some());
    (setup, commit)
}

#[test]
fn a_clean_asleep_task_retires_when_disk_runs_short_and_resurrects_with_its_worktree_and_session() {
    let (setup, commit) = asleep_when_disk_runs_short("retire", |setup| {
        setup.calls(&[result("passed")]);
    });
    let session = setup.rows("runs")[0][4].clone();

    setup.removed(1);

    let tasks = setup.until("tasks", |rows| rows[0][10] != "-");
    assert_eq!(tasks[0][4], "asleep");
    assert!(setup.branches().contains("priory/1"));
    let retired: Vec<_> = setup
        .rows("desk")
        .into_iter()
        .filter(|row| row[1] == "retired")
        .map(|row| [row[2].clone(), row[5].clone()])
        .collect();
    assert_eq!(
        retired,
        [[
            "1".to_owned(),
            "its worktree was removed to free disk; it resurrects from priory/1 when its wait fires"
                .to_owned()
        ]]
    );

    setup.disk(100);
    setup.until("tasks", |rows| rows[1][4] == "done");
    setup.publish(&closed());

    setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[2] == "3" && row[3] == "confirmed")
    });
    assert_eq!(setup.rows("tasks")[0][10], "-");
    let head = setup.git(&setup.worktree(1), &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), commit);
    let runs = setup.rows("runs");
    assert_eq!([&runs[2][1], &runs[2][4]], ["1", &session]);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    assert!(args.contains(&format!("--resume\n{session}\n")), "{args}");
}

#[test]
fn a_dirty_asleep_task_never_retires_and_says_why() {
    let (setup, _) = asleep_when_disk_runs_short("retire-dirty", |setup| {
        fs::write(setup.worktree(1).join("notes.txt"), "x\n").unwrap();
    });

    let tasks = setup.until("tasks", |rows| rows[0][7] != "-");

    assert_eq!(
        [&tasks[0][4], &tasks[0][7], &tasks[0][10]],
        ["asleep", "cannot retire: untracked files", "-"]
    );
    assert!(setup.worktree(1).join("notes.txt").exists());
}

#[test]
fn a_task_that_waited_on_a_linear_issue_is_done_on_its_result_without_a_pull_request() {
    let mut setup = Setup::new("linear-done");
    setup.calls(&[
        wait(json!({ "type": "linear_comment", "issue": "ENG-7" })),
        result("passed"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });

    setup.publish(&linear_event(
        "linear.comment.created",
        "ENG-7",
        json!({ "body": "Go ahead." }),
    ));

    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_linear_comment_no_task_owned_before_the_wait_wakes_the_task_at_once() {
    let mut setup = Setup::new("linear-already");
    setup.calls(&[wait(json!({ "type": "linear_comment", "issue": "ENG-7" }))]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/submitted");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
    // The comment lands after the agent's prompt, while no task owns ENG-7 yet.
    setup.publish(&linear_event(
        "linear.comment.created",
        "ENG-7",
        json!({ "body": "Already answered." }),
    ));
    setup.until("inbox", |rows| rows.len() == 2 && rows[1][4] == "ignored");

    setup.release("hold-call");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let (text, error) = reply(&setup.replies()[0]);
    assert!(!error, "{text}");
    assert!(text.starts_with("wait 1 already holds"), "{text}");
    assert!(
        setup.typed()[1].starts_with("linear.comment.created ENG-7 {"),
        "{:?}",
        setup.typed()
    );
}

#[test]
fn a_free_disk_wait_fires_when_room_comes_back() {
    let mut setup = Setup::new("disk-wait");
    setup.disk(30);
    setup.calls(&[wait(json!({ "type": "disk", "gb": 50 }))]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    assert_eq!(setup.rows("waits")[0][3..5], ["disk", "50.0 GB free"]);
    stays_asleep(&setup);

    setup.disk(60);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        "wait 1 fired: 60.0 GB of disk free\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n"
    );
}

#[test]
fn a_revive_into_a_code_step_recreates_a_swept_worktree() {
    let mut setup = Setup::with_workflow("revive-code-step", AUTONOMOUS);
    let mut failed = passed_with_evidence();
    failed["arguments"]["outcome"] = json!("failed");
    setup.calls(&[failed]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "dropped")
    });
    setup.gone("priory-1");
    let commit = setup.commit(1, "widget.txt");
    setup.git(
        &setup.clone(),
        &["worktree", "remove", setup.worktree(1).to_str().unwrap()],
    );

    let revived = setup.priory(&["revive", "1", "--step", "open-pr"]);

    assert!(revived.status.success(), "{revived:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4..7], ["opened", "github:owner/repo#7", &commit]);
    assert_eq!(setup.pushed("priory/1"), commit);
}
