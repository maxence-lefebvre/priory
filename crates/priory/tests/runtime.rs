mod conform;
mod tmux_dir;

use herdr_bus::{Bus, Message, Output};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output as Exit};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

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

/// Takes a state file back before schema 28, which added search, 29, which added grades, 30,
/// which added snoozes, 31, which added read reports, 32, which added the pause, 34, which added
/// reports of sections, 36, which added deliveries, 37, which added progress, and 39, which added
/// run actions. Schema 38's visits table is created only if missing, so it stays.
const UNDO_SINCE_SEARCH: &str = "ALTER TABLE actions DROP COLUMN stdout;
  ALTER TABLE actions DROP COLUMN stderr; ALTER TABLE actions DROP COLUMN pid;
  ALTER TABLE actions DROP COLUMN pid_started; ALTER TABLE actions DROP COLUMN ended;
  ALTER TABLE tasks DROP COLUMN progress; DROP TABLE deliveries; DROP TABLE reports; DROP TABLE pause;
  ALTER TABLE outbox DROP COLUMN wakes;
  DROP TRIGGER snooze_ends_on_a_new_revision;
  DROP TRIGGER snooze_ends_on_a_task_state_change; DROP TABLE snoozes;
  ALTER TABLE inbox DROP COLUMN read; ALTER TABLE results DROP COLUMN reason; ALTER TABLE tasks DROP COLUMN grade;
  ALTER TABLE tasks DROP COLUMN grade_reason;
  DROP TRIGGER search_task_created; DROP TRIGGER search_tldr_changed; DROP TRIGGER search_result_stored;
  DROP VIEW task_text; DROP TABLE search; DROP TABLE summaries;
  DROP TABLE embedder; DROP TABLE vectors; ALTER TABLE tasks DROP COLUMN tldr;";

/// A factory, bus, state, Priory home, origin repository and fake `claude`, all under the test's
/// own directory, and a tmux server in its own short directory. Dropping it kills the daemon and
/// the tmux server.
struct Setup {
    dir: PathBuf,
    daemon: Option<Child>,
    tmux: TmuxDir,
}

impl Setup {
    fn new(name: &str) -> Self {
        Self::with_workflow(name, WORKFLOW)
    }

    fn with_workflow(name: &str, workflow: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rt").join(name);
        let tmux = TmuxDir::new(&dir);
        let setup = Setup {
            dir,
            daemon: None,
            tmux,
        };
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
            .env_remove("OTEL_RESOURCE_ATTRIBUTES")
            .env("PATH", path)
            .env("TMUX_TMPDIR", self.tmux.path())
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

    fn start(&mut self) {
        self.start_as(Path::new(env!("CARGO_BIN_EXE_priory")));
    }

    /// Starts the daemon through the path `exe`, as a person or a service invokes it.
    fn start_as(&mut self, exe: &Path) {
        self.start_with(exe, &[]);
    }

    fn start_with(&mut self, exe: &Path, env: &[(&str, &str)]) {
        let factory = self.dir.join("factory");
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        self.daemon = Some(
            self.command(exe.to_str().unwrap())
                .envs(env.iter().copied())
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
        self.listed(&[verb])
    }

    fn listed(&self, args: &[&str]) -> Vec<Vec<String>> {
        let output = self.priory(args);
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

    /// Waits until the fake `claude` took all but `left` of its queued tool calls.
    fn calls_left(&self, left: usize) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let path = self.dir.join("fake/calls");
        while fs::read_to_string(&path).unwrap().lines().count() > left {
            assert!(Instant::now() < deadline, "the fake never took its calls");
            std::thread::sleep(Duration::from_millis(50));
        }
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
        self.tmux.kill();
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
    let mcp = args.windows(2).find(|w| w[0] == "--mcp-config").unwrap()[1];
    let mcp: serde_json::Value = serde_json::from_str(mcp).unwrap();
    assert_eq!(
        mcp["mcpServers"]["priory"]["env"],
        json!({
            "PRIORY_HOME": setup.dir.join("home"),
            "PRIORY_DB": setup.dir.join("priory.db"),
        })
    );
    assert_eq!(setup.rows("tasks")[0][4], "running");
}

#[test]
fn a_cli_request_starts_its_run_in_the_repository_it_names() {
    let mut setup = Setup::new("cli-repo");
    let factory = setup.dir.join("factory");
    let requested = setup.priory(&[
        "request",
        factory.to_str().unwrap(),
        "--workflow",
        "implement",
        "--key",
        "fix-it",
        "--repo",
        "owner/repo",
        "Fix it.",
    ]);
    assert!(
        requested.status.success(),
        "{}",
        String::from_utf8_lossy(&requested.stderr)
    );

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "running")
    });
    assert!(
        setup
            .dir
            .join("home/worktrees/owner/repo/1/README.md")
            .is_file()
    );
}

#[test]
fn a_cli_task_stored_before_tasks_kept_a_repository_starts_in_its_requests_repository() {
    let mut setup = Setup::new("cli-repo-upgrade");
    let factory = setup.dir.join("factory");
    let requested = setup.priory(&[
        "request",
        factory.to_str().unwrap(),
        "--workflow",
        "implement",
        "--key",
        "fix-it",
        "--repo",
        "owner/repo",
        "--hold",
        "later",
        "Fix it.",
    ]);
    assert!(requested.status.success());
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "held")
    });
    setup.crash();
    // The state file as version 23 left it: tasks without a repository.
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(UNDO_SINCE_SEARCH).unwrap();
    db.execute_batch("ALTER TABLE tasks DROP COLUMN repo; ALTER TABLE outbox DROP COLUMN compact; PRAGMA user_version = 23;")
        .unwrap();
    drop(db);
    let _ = fs::remove_file(setup.dir.join("home/priory.sock"));

    setup.start();
    listening(&setup);
    person(&setup, "release", &["1"]);

    setup.until("tasks", |rows| rows[0][4] == "running");
}

#[test]
fn a_linear_task_starts_its_run_in_its_triggers_repository() {
    let mut setup = Setup::new("linear-repo");
    fs::write(
        setup.dir.join("factory/triggers/linear.toml"),
        "source = \"linear\"\nrepo = \"owner/repo\"\non = \"issue.created\"\nworkflow = \"implement\"\n",
    )
    .unwrap();
    let mut request = requested(42);
    request.data["key"] = json!("linear:ENG-7");
    request.data["trigger"] = json!("linear");
    setup.publish(&request);

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "running")
    });
    assert!(
        setup
            .dir
            .join("home/worktrees/owner/repo/1/README.md")
            .is_file()
    );
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
            "env": {
                "PRIORY_HOME": setup.dir.join("home"),
                "PRIORY_DB": setup.dir.join("priory.db"),
            },
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
fn a_compaction_mid_turn_leaves_the_turn_busy_so_no_prompt_is_typed_into_it() {
    let mut setup = Setup::new("compact");
    setup.publish(&requested(42));
    setup.hold("compact");
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    setup.publish(&labeled(42));

    setup.until("outbox", |rows| rows.len() == 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("outbox")[1][3], "queued");
    assert_eq!(setup.typed().len(), 1);
}

fn compact(focus: &str) -> serde_json::Value {
    json!({ "name": "compact", "arguments": { "focus": focus } })
}

const CONTINUED: &str = "Priory compacted this conversation, as you asked. You are on task 1, at step implement. Carry on with the step.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n";

#[test]
fn the_compact_tool_types_compact_once_idle_then_a_continuation_after_the_compaction() {
    let mut setup = Setup::new("compact-tool");
    setup.calls(&[compact("the failing test")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert_eq!(typed[1], "/compact the failing test\n", "{typed:?}");
    assert_eq!(typed[2], CONTINUED, "{typed:?}");
    assert_eq!(
        typed
            .iter()
            .filter(|prompt| prompt.starts_with("/compact"))
            .count(),
        1,
        "{typed:?}"
    );
}

#[test]
fn a_second_compact_before_the_first_is_typed_is_refused() {
    let mut setup = Setup::new("compact-twice");
    setup.calls(&[compact("the plan")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/tools.log");
    let session = setup.rows("runs")[0][4].clone();

    // The same turn asks again before its Stop, so the first is still queued.
    let replies = setup.mcp(
        &session,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": compact("again")})],
    );

    let (text, error) = reply(&replies[0]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        "a compact is already queued for this run; end your turn"
    );
    assert_eq!(setup.typed().len(), 1, "nothing is typed into a busy turn");
    setup.release("hold-stop");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert_eq!(typed[1], "/compact the plan\n", "{typed:?}");
    assert_eq!(
        typed
            .iter()
            .filter(|prompt| prompt.starts_with("/compact"))
            .count(),
        1,
        "{typed:?}"
    );
}

#[test]
fn a_result_after_a_compact_in_the_same_turn_is_refused_so_the_compact_reaches_its_own_conversation()
 {
    let mut setup = Setup::with_workflow("compact-result", STEPS);
    setup.calls(&[compact("the plan")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/tools.log");
    let session = setup.rows("runs")[0][4].clone();

    let replies = setup.mcp(
        &session,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": result("ready")})],
    );

    let (text, error) = reply(&replies[0]);
    assert!(error, "{text}");
    assert_eq!(
        text,
        "a compact is queued for this run; end your turn and call result after the compaction"
    );
}

#[test]
fn a_queued_answer_and_its_nudge_are_typed_before_the_compact_asked_ahead_of_them() {
    let mut setup = Setup::with_workflow("compact-answer", STEPS);
    setup.calls(&[compact("the plan")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/tools.log");
    let session = setup.rows("runs")[0][4].clone();
    setup.mcp(
        &session,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": ask()})],
    );
    setup.release("hold-stop");
    setup.until("tasks", |rows| rows[0][4] == "asleep");

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 5);
    let typed = setup.typed();
    assert!(typed[1].starts_with("sqlite\n"), "{typed:?}");
    assert!(typed[2].starts_with("Your turn ended"), "{typed:?}");
    assert_eq!(typed[3], "/compact the plan\n", "{typed:?}");
    assert!(
        typed[4].starts_with(
            "Priory compacted this conversation, as you asked. You are on task 1, at step plan."
        ),
        "{typed:?}"
    );
}

#[test]
fn a_codex_run_gets_a_bare_compact_and_its_post_compact_hook_brings_the_continuation() {
    let mut setup = Setup::new("compact-codex");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.calls(&[compact("the failing test")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert_eq!(typed[1], "/compact\n", "{typed:?}");
    assert_eq!(typed[2], CONTINUED, "{typed:?}");
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
fn a_prompt_attempted_before_a_crash_becomes_uncertain_is_not_typed_again_and_its_late_submit_confirms_it()
 {
    let mut setup = Setup::new("uncertain");
    setup.publish(&requested(42));
    setup.hold("hold-submit");
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");

    setup.crash();
    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    setup.release("hold-submit");
    setup.until("outbox", |rows| rows[0][3] == "confirmed");
    assert_eq!(setup.typed().len(), 1);
}

#[test]
fn a_prompt_submitted_while_the_daemon_is_down_is_confirmed_when_it_comes_back() {
    let mut setup = Setup::new("uncertain-spooled");
    setup.publish(&requested(42));
    setup.hold("hold-submit");
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");

    setup.crash();
    setup.release("hold-submit");
    setup.written("home/signals.spool");
    setup.start();

    setup.until("outbox", |rows| rows[0][3] == "confirmed");
    assert_eq!(setup.typed().len(), 1);
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
fn a_run_whose_tmux_session_dies_ends_exited_and_its_repair_restarts_the_agent() {
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

    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert_eq!([&runs[0][7], &runs[1][7]], ["exited", "-"]);
    assert_ne!(runs[0][6], "-");
    setup.until("tasks", |rows| rows[0][4] == "running");
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
    // The second exit follows a repair with no progress, so the task fails onto the desk.
    for typed in ["Implement the issue.", "Priory repaired this task"] {
        setup.until("outbox", |rows| {
            rows.iter()
                .any(|row| row[3] == "confirmed" && row[4].starts_with(typed))
        });
        assert!(
            setup
                .tmux(&["kill-session", "-t", "priory-1"])
                .status
                .success()
        );
    }
    setup.until("tasks", |rows| rows[0][4] == "failed");
    let desk = setup.priory(&["desk"]);
    let desk = String::from_utf8(desk.stdout).unwrap();
    let mut lines = desk.lines();
    assert_eq!(
        lines.next(),
        Some("since\tkind\tid\ttask\tlane\twhat\tgeneration\tsnoozed_until\tback_early")
    );
    let cells: Vec<_> = lines.next().unwrap().split('\t').collect();
    assert_eq!(cells[1..5], ["failed", "1", "1", "FND/desktop"]);
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
    fs::write(
        &lanes,
        "[[rules]]\nbranch = \"priory/*\"\nlane = \"OPS/other\"\n",
    )
    .unwrap();
    setup.crash();
    setup.start();

    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );

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
fn a_crash_on_a_full_disk_the_sweep_cannot_free_fails_without_a_restart() {
    let mut setup = Setup::new("repair-disk");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    setup.disk(10);

    assert!(
        setup
            .tmux(&["kill-session", "-t", "priory-1"])
            .status
            .success()
    );

    let tasks = setup.until("tasks", |rows| rows[0][4] == "failed");
    let cause = "disk full (10.0 GB of disk free, keeps 20.0 GB free)";
    assert_eq!(
        tasks[0][7],
        format!(
            "run 1 exited: claude no longer runs in tmux session priory-1. Cause: {cause}. Action: sweep. The cause persists, so Priory did not restart the agent."
        )
    );
    assert_eq!(
        repairs(&setup),
        [[cause, "sweep", "failed"].map(str::to_owned)]
    );
    setup.disk(100);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(setup.rows("runs").len(), 1);
    let desk = setup.rows("desk");
    assert_eq!([&desk[0][1], &desk[0][3]], ["failed", "1"]);
}

#[test]
fn a_crash_after_its_repair_leaves_the_task_failed_until_a_revive() {
    let mut setup = Setup::new("crash");
    setup.hold("crash");
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "failed")
    });
    assert_eq!(
        tasks[0][7],
        "run 2 exited: claude no longer runs in tmux session priory-1; it failed again after its repair, so Priory did not restart the agent"
    );
    let runs = setup.rows("runs");
    assert_eq!([&runs[0][7], &runs[1][7]], ["exited", "exited"]);
    assert!(setup.typed()[1].starts_with(
        "Priory repaired this task after: run 1 exited: claude no longer runs in tmux session priory-1. Cause: the harness crashed. Action: restart."
    ));
    assert_eq!(
        repairs(&setup),
        [["the harness crashed", "restart", "ready"].map(str::to_owned)]
    );
    setup.gone("priory-1");
    setup.release("crash");

    let revived = setup.priory(&["revive", "1"]);

    assert!(revived.status.success(), "{revived:?}");
    setup.until("runs", |rows| rows.len() == 3);
    setup.until("tasks", |rows| rows[0][4] == "running");
}

/// The `factory.task.repaired` records' cause, action and outcome, oldest first.
fn repairs(setup: &Setup) -> Vec<[String; 3]> {
    let output = setup.priory(&["trace"]);
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["type"] == "factory.task.repaired")
        .map(|event| {
            ["cause", "action", "outcome"]
                .map(|field| event["data"][field].as_str().unwrap().to_owned())
        })
        .collect()
}

#[test]
fn a_crash_out_of_memory_restarts_the_agent_once_room_is_back_with_the_cause_in_its_prompt() {
    let mut setup = Setup::new("repair-memory");
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/pending");
    setup.memory(1);
    setup.hold("crash");

    setup.release("hold-submit");

    let tasks = setup.until("tasks", |rows| {
        rows[0][4] == "ready" && rows[0][7].starts_with("waits for room: 1.0 GB of memory free")
    });
    assert_eq!(setup.rows("runs")[0][7], "exited");
    assert_eq!(setup.rows("runs").len(), 1, "{tasks:?}");
    let cause = "out of memory (1.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free)";
    assert_eq!(
        repairs(&setup),
        [[cause, "wait for room", "ready"].map(str::to_owned)]
    );
    setup.release("crash");
    setup.memory(60);
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        format!(
            "Priory repaired this task after: run 1 exited: claude no longer runs in tmux session priory-1. Cause: {cause}. Action: wait for room. Check the worktree, then continue.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n"
        )
    );
    assert_eq!(setup.rows("tasks")[0][4], "running");
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
fn teardown_kills_a_tmux_server_that_a_late_child_starts() {
    let setup = Setup::new("teardown-late-tmux");
    // As a daemon's child in its own process group, still starting a session when the test ends.
    let mut late = setup
        .command("sh")
        .args(["-c", "sleep 0.5; tmux new-session -d sleep 100"])
        .process_group(0)
        .spawn()
        .unwrap();
    let mut listed = setup.command("tmux");
    listed.arg("list-sessions");
    let mut killed = setup.command("tmux");
    killed.arg("kill-server");

    drop(setup);
    late.wait().unwrap();
    std::thread::sleep(Duration::from_millis(1000));

    let listed = listed.output().unwrap();
    let _ = killed.output();
    assert!(!listed.status.success(), "a tmux server outlived its test");
}

#[test]
fn a_test_that_panics_after_its_run_starts_leaves_no_tmux_server() {
    let mut server = None;
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut setup = Setup::new("teardown-after-panic");
        setup.publish(&requested(42));
        setup.start();
        setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
        let pid = setup.tmux(&["display-message", "-p", "#{pid}"]);
        let pid = String::from_utf8(pid.stdout).unwrap().trim().to_owned();
        server = Some((socket(&setup), pid));
        panic!("the test fails while its agent runs");
    }));

    assert!(caught.is_err());
    let (socket, pid) = server.expect("the run never got a tmux session");
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "tmux server {pid} outlived its test"
    );
    let listed = Command::new("tmux")
        .args(["-S", &socket, "list-sessions"])
        .output()
        .unwrap();
    assert!(!listed.status.success(), "a tmux server outlived its test");
    assert!(
        !Path::new(&socket).parent().unwrap().exists(),
        "the socket directory outlived its test"
    );
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

#[test]
fn a_head_that_waits_only_for_live_runs_to_end_opens_no_stall() {
    let mut setup = Setup::new("backpressure");
    fs::write(setup.dir.join("factory/factory.toml"), SWEEP_SCRIPTS).unwrap();
    setup.memory(12);
    setup.hold("hold-stop");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    let reason = "waits for room: 8.0 GB of memory free, needs 4.0 GB and keeps 6.4 GB free";
    setup.until("tasks", |rows| rows.len() == 2 && rows[1][7] == reason);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.banner(), None);
    assert!(!setup.dir.join("fake/swept").exists());
    setup.release("hold-stop");
    setup.until("tasks", |rows| rows[1][4] == "running");
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

#[test]
fn a_sweeper_asleep_on_its_approval_while_disk_stays_short_still_opens_its_issue() {
    let mut setup = stalled_with_sweeper("sweeper-short");
    setup.calls(&[json!({ "name": "result", "arguments": {
        "outcome": "cleaned",
        "summary": "Nothing freed\nThe disk is still short.",
    } })]);
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);

    // A daemon start sweeps and retires asleep tasks while the disk is short.
    setup.crash();
    setup.start();
    std::thread::sleep(Duration::from_millis(1500));
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");

    setup.until("tasks", |rows| rows[0][4] == "done");
    assert_eq!(setup.gh()[1][..2], ["issue", "create"]);
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
    // Task 2 waits only for task 1's promise to end, which is no stall.
    assert_eq!(setup.banner(), None);
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

/// The lines the fake `name` logged under `fake/`, one per call.
fn fake_log(setup: &Setup, name: &str) -> Vec<String> {
    fs::read_to_string(setup.dir.join("fake").join(name))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_run_starts_in_its_own_scope_inside_the_agents_slice_capped_at_90_percent() {
    let mut setup = Setup::new("slice");
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert_eq!(
        fake_log(&setup, "systemd-run.log"),
        ["--user --scope --quiet --collect --expand-environment=no --slice=priory-agents.slice"]
    );
    let cpus = std::thread::available_parallelism().unwrap().get();
    // CPU weights split the CPU between siblings, so the weight goes on the top-level slice.
    assert_eq!(
        fake_log(&setup, "systemctl.log"),
        [
            "--user set-property --runtime priory.slice CPUWeight=20".to_owned(),
            format!(
                "--user set-property --runtime priory-agents.slice CPUQuota={}% MemoryHigh=90%",
                cpus * 90
            )
        ]
    );
    assert_eq!(setup.rows("runs")[0][7], "-");
}

#[test]
fn agent_text_reaches_the_agent_as_written_through_a_scope_that_would_expand_it() {
    let mut setup = Setup::new("slice-literal");
    let agent = AGENT.replace("You implement one issue.", "Keep ${HOME} as written.");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), agent).unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    assert!(
        args.lines().any(|line| line == "Keep ${HOME} as written."),
        "{args}"
    );
}

#[test]
fn a_systemd_older_than_254_starts_runs_without_the_expand_environment_option() {
    let mut setup = Setup::new("slice-old");
    fs::write(setup.dir.join("fake/systemd-version"), "252\n").unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert_eq!(
        fake_log(&setup, "systemd-run.log"),
        ["--user --scope --quiet --collect --slice=priory-agents.slice"]
    );
}

#[test]
fn the_agents_slice_takes_its_name_from_priory_slice_and_its_caps_from_the_runner() {
    let mut setup = Setup::new("slice-share");
    let runner = "platform = { os = \"linux\" }\nagents = { cpu = \"50%\", memory = \"60%\" }\n";
    fs::create_dir_all(setup.dir.join("factory/runners")).unwrap();
    fs::write(setup.dir.join("factory/runners/runner.toml"), runner).unwrap();
    setup.publish(&requested(42));

    setup.start_with(
        Path::new(env!("CARGO_BIN_EXE_priory")),
        &[("PRIORY_SLICE", "scratch.slice")],
    );

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert_eq!(
        fake_log(&setup, "systemd-run.log"),
        ["--user --scope --quiet --collect --expand-environment=no --slice=scratch.slice"]
    );
    let cpus = std::thread::available_parallelism().unwrap().get();
    assert_eq!(
        fake_log(&setup, "systemctl.log"),
        [
            "--user set-property --runtime scratch.slice CPUWeight=20".to_owned(),
            format!(
                "--user set-property --runtime scratch.slice CPUQuota={}% MemoryHigh=60%",
                cpus * 50
            )
        ]
    );
}

#[test]
fn a_host_without_systemd_starts_its_runs_as_before_and_logs_why() {
    let mut setup = Setup::new("no-systemd");
    fs::write(setup.dir.join("fake/no-systemd"), "").unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert_eq!(setup.rows("tasks")[0][4], "running");
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert!(
        log.contains("no systemd user manager, so runs start outside priory-agents.slice"),
        "daemon log: {log}"
    );
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
    setup.gone("priory-1");
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

const GRADED: &str = r#"
start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change. Grade it red, yellow or green."
effect = "reversible"
next = { red = "regrade", yellow = "regrade", green = "regrade" }

[[steps]]
id = "regrade"
decides = "model"
agent = "implementer"
prompt = "Grade the plan again."
effect = "reversible"
next = { red = "implement", yellow = "implement", green = "implement" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the plan."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

/// The `grade` and `grade_reason` columns of `priory tasks` for its first task.
fn grade(tasks: &[Vec<String>]) -> [&str; 2] {
    [&tasks[0][11], &tasks[0][12]]
}

#[test]
fn a_step_ending_green_with_a_reason_sets_the_task_grade_and_reason() {
    let mut setup = Setup::with_workflow("grade-green", GRADED);
    setup.calls(&[json!({ "name": "result", "arguments": {
        "outcome": "green", "summary": "Plan: add a flag", "reason": "one file,\tcovered by tests"
    } })]);
    setup.publish(&requested(42));
    setup.hold("hold-stop");

    setup.start();

    setup.written("fake/tools.log");
    assert_eq!(
        grade(&setup.rows("tasks")),
        ["-", "-"],
        "graded before the step ended"
    );
    setup.release("hold-stop");
    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|task| task[3] == "regrade")
    });
    assert_eq!(grade(&tasks), ["green", "one file, covered by tests"]);
    let listed: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["tasks", "--json"]).stdout).unwrap();
    assert_eq!(
        [
            &listed["tasks"][0]["grade"],
            &listed["tasks"][0]["grade_reason"]
        ],
        [&json!("green"), &json!("one file, covered by tests")]
    );
}

#[test]
fn a_second_grading_step_overwrites_the_grade_and_the_trace_keeps_both() {
    let mut setup = Setup::with_workflow("grade-twice", GRADED);
    setup.calls(&[
        json!({ "name": "result", "arguments": {
            "outcome": "green", "summary": "Plan: add a flag", "reason": "one file"
        } }),
        json!({ "name": "result", "arguments": {
            "outcome": "red", "summary": "It touches the schema\n\nA migration is needed."
        } }),
    ]);
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|task| task[3] == "implement")
    });
    assert_eq!(grade(&tasks), ["red", "It touches the schema"]);
    let grades: Vec<_> = setup
        .trace()
        .iter()
        .filter(|record| record["type"] == "factory.step.ended")
        .map(|record| {
            (
                record["data"]["step"].clone(),
                record["data"]["grade"].clone(),
            )
        })
        .collect();
    assert_eq!(
        grades,
        [
            (json!("plan"), json!("green")),
            (json!("regrade"), json!("red"))
        ]
    );
}

#[test]
fn a_step_ending_with_another_outcome_leaves_the_grade() {
    let mut setup = Setup::with_workflow("grade-kept", GRADED);
    setup.telemetry("http://127.0.0.1:4318");
    setup.calls(&[
        json!({ "name": "result", "arguments": {
            "outcome": "yellow", "summary": "Plan", "reason": "two crates"
        } }),
        result("green"),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|task| task[4] == "done")
    });
    assert_eq!(grade(&tasks), ["green", "green it"]);
    let spans = setup.until_spans(6);
    let exported = |step: &str| {
        let span = spans.iter().find(|span| span["name"] == step).unwrap();
        attribute(&span["attributes"], "priory.grade")
    };
    assert_eq!(
        [exported("plan"), exported("regrade"), exported("implement")],
        [json!("yellow"), json!("green"), json!(null)]
    );
}

#[test]
fn tasks_filtered_by_grade_lists_only_that_grade() {
    let mut setup = Setup::with_workflow("grade-filter", GRADED);
    setup.calls(&[result("green")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|task| task[3] == "regrade")
    });
    setup.publish(&requested(43));
    setup.until("tasks", |rows| rows.len() == 2);

    let listed = |grade: &str| {
        let output = setup.priory(&["tasks", "--grade", grade]);
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split('\t').next().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(listed("green"), ["1"]);
    assert!(listed("red").is_empty());
    assert!(!setup.priory(&["tasks", "--grade", "blue"]).status.success());
}

#[test]
fn a_task_request_for_a_workflow_the_factory_lacks_waits_queued_and_starts_once_it_is_back() {
    let mut setup = Setup::new("missing-workflow");
    let mut triage = requested(42);
    triage.data["workflow"] = json!("triage");
    setup.publish(&triage);
    setup.publish(&requested(43));

    setup.start();

    setup.until("tasks", |rows| {
        rows.len() == 2 && rows[1][4] == "running" && rows[0][7] != "-"
    });
    let tasks = setup.rows("tasks");
    assert_eq!(
        [&tasks[0][2], &tasks[0][4], &tasks[0][7]],
        [
            "triage",
            "queued",
            "waits for workflow triage, which the factory lacks"
        ]
    );
    setup.crash();
    fs::write(setup.dir.join("factory/workflows/triage.toml"), WORKFLOW).unwrap();
    setup.start();
    let tasks = setup.until("tasks", |rows| rows[0][4] == "running");
    assert_eq!(tasks[0][3], "implement");
}

#[test]
fn a_queued_task_at_a_step_its_workflow_lost_is_blocked_with_the_reason() {
    let mut setup = Setup::with_workflow("lost-step", STEPS);
    setup.publish(&requested(42));
    route(&setup);
    fs::write(setup.dir.join("factory/workflows/implement.toml"), WORKFLOW).unwrap();

    setup.start();

    let tasks = setup.until("tasks", |rows| rows[0][4] == "blocked");
    assert_eq!(tasks[0][7], "step plan is not in workflow implement");
}

#[test]
fn a_workflow_that_starts_at_a_human_step_asks_its_question_when_the_task_is_created() {
    let workflow = STEPS
        .replace("start = \"plan\"", "start = \"approve\"")
        .replace("approved = \"implement\"", "approved = \"plan\"");
    let mut setup = Setup::with_workflow("human-start", &workflow);
    setup.publish(&requested(42));

    setup.start();

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(
        [&asks[0][3], &asks[0][4]],
        ["approve", r#"["approved","rejected"]"#]
    );
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["approve", "asleep"]);
}

#[test]
fn a_task_queued_before_schema_11_ends_its_first_step_once_upgraded() {
    let mut setup = Setup::with_workflow("legacy-entered", STEPS);
    setup.calls(&[result("ready")]);
    setup.publish(&requested(42));
    route(&setup);
    // Schema 10 inserted a queued task with no `changed`, which migration 11 copied into `entered`.
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(UNDO_SINCE_SEARCH).unwrap();
    db.execute_batch(
        "UPDATE tasks SET entered = NULL; ALTER TABLE outbox DROP COLUMN compact;
         PRAGMA user_version = 24;",
    )
    .unwrap();

    setup.start();

    let tasks = setup.until("tasks", |rows| rows[0][3] == "implement");
    assert_eq!(tasks[0][4], "running");
    assert_eq!(setup.trace()[0]["data"]["step"], "plan");
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
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
        [&asks[0][..7]],
        [[
            "1",
            "1",
            "1",
            "approve: unsure it",
            r#"["approved","rejected"]"#,
            "-",
            "-"
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
        [[
            "1",
            "1",
            "1",
            "Which database?",
            r#"["sqlite","postgres"]"#,
            "-",
            "-",
            "-",
            "-",
            "-",
            "-",
            "-",
            "-",
            "github:owner/repo#42",
            "-",
            "-"
        ]]
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

/// The desk's rows that wait on a person, without the worktrees the sweep keeps.
fn desk(setup: &Setup, args: &[&str]) -> Vec<Vec<String>> {
    let mut args = args.to_vec();
    args.insert(0, "desk");
    setup
        .listed(&args)
        .into_iter()
        .filter(|row| row[1] != "worktree")
        .collect()
}

fn asleep_on_an_ask(name: &str) -> Setup {
    let mut setup = Setup::with_workflow(name, STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup
}

#[test]
fn a_snoozed_ask_leaves_the_desk_and_asks_until_its_time_while_the_task_stays_asleep() {
    let setup = asleep_on_an_ask("snooze");

    let snoozed = setup.priory(&["snooze", "ask", "1", "--for", "3s"]);

    assert!(snoozed.status.success(), "{snoozed:?}");
    assert_eq!(desk(&setup, &[]), Vec::<Vec<String>>::new());
    assert_eq!(setup.rows("asks"), Vec::<Vec<String>>::new());
    let all = desk(&setup, &["--all"]);
    assert_eq!([&all[0][1], &all[0][2]], ["ask", "1"]);
    assert_ne!(all[0][7], "-", "{all:?}");
    let json: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["desk", "--json"]).stdout).unwrap();
    assert_eq!(json["counts"]["snoozed"], 1);
    assert!(
        json["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["kind"] != "ask")
    );
    let json: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["desk", "--all", "--json"]).stdout).unwrap();
    assert_eq!(json["counts"]["snoozed"], 1);
    let json: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["asks", "--json"]).stdout).unwrap();
    let json = &json["asks"];
    assert_eq!(json[0]["snoozed_until"].as_str(), Some(all[0][7].as_str()));
    assert!(json[0]["snoozed_at"].is_string(), "{json}");
    assert!(json[0]["back_early"].is_null(), "{json}");
    assert_eq!(setup.listed(&["asks", "--all"]).len(), 1);
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["plan", "asleep"]);

    std::thread::sleep(Duration::from_secs(4));

    let back = desk(&setup, &[]);
    assert_eq!([&back[0][1], &back[0][2], &back[0][7]], ["ask", "1", "-"]);
    assert_eq!(setup.rows("asks").len(), 1);
}

#[test]
fn clearing_a_snooze_brings_the_ask_back_and_the_trace_records_both() {
    let setup = asleep_on_an_ask("snooze-clear");
    let snoozed = setup.priory(&["snooze", "ask", "1", "--until", "2099-01-01T00:00:00+01:00"]);
    assert_eq!(
        String::from_utf8_lossy(&snoozed.stdout),
        "ask 1 is snoozed until 2098-12-31T23:00:00Z\n"
    );

    let cleared = setup.priory(&["snooze", "ask", "1", "--clear"]);

    assert_eq!(
        String::from_utf8_lossy(&cleared.stdout),
        "ask 1 is back on the desk\n"
    );
    assert_eq!(setup.rows("asks")[0][5], "-");
    let events: Vec<serde_json::Value> = String::from_utf8(setup.priory(&["trace"]).stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .filter(|event: &serde_json::Value| event["type"].as_str().unwrap().contains("snooze"))
        .collect();
    let kinds: Vec<_> = events.iter().map(|event| &event["type"]).collect();
    assert_eq!(kinds, ["factory.task.snoozed", "factory.task.unsnoozed"]);
    assert_eq!(events[0]["data"]["task"], "1");
    assert_eq!(
        events[0]["data"]["item"],
        json!({ "kind": "ask", "id": "1" })
    );
    assert_eq!(events[0]["data"]["until"], "2098-12-31T23:00:00Z");
    let again = setup.priory(&["snooze", "ask", "1", "--clear"]);
    assert_eq!(
        String::from_utf8_lossy(&again.stderr),
        "priory snooze: ask 1 is not snoozed\n"
    );
}

#[test]
fn a_snooze_until_a_past_time_is_refused() {
    let setup = asleep_on_an_ask("snooze-past");

    let past = setup.priory(&["snooze", "ask", "1", "--until", "2020-01-01T00:00:00Z"]);

    assert!(!past.status.success());
    assert_eq!(
        String::from_utf8_lossy(&past.stderr),
        "priory snooze: 2020-01-01T00:00:00Z is not in the future\n"
    );
    assert_eq!(setup.rows("asks")[0][5], "-");
}

#[test]
fn an_agent_ask_stores_and_lists_its_title_text_what_waits_next_action_and_command() {
    let mut setup = Setup::with_workflow("ask-fields", STEPS);
    setup.calls(&[json!({ "name": "ask", "arguments": {
        "question": "Which database?",
        "title": "Which database?",
        "text": "The cache needs a store.\n\nPick one.",
        "options": ["sqlite", "postgres"],
        "recommendation": "sqlite: one file",
        "tradeoff": "postgres scales, sqlite needs no server",
        "whatWaits": "the cache PR",
        "nextAction": "wire the store",
        "command": "cargo test",
    } })]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    assert_eq!(
        setup.rows("asks"),
        [[
            "1",
            "1",
            "1",
            "Which database?",
            r#"["sqlite","postgres"]"#,
            "-",
            "-",
            "Which database?",
            r"The cache needs a store.\n\nPick one.",
            "the cache PR",
            "wire the store",
            "cargo test",
            "-",
            "github:owner/repo#42",
            "-",
            "-",
        ]]
    );
}

fn fleet_ask(options: &[&str]) -> String {
    json!({
        "title": "Ship the cache today?",
        "text": "The cache is ready.\n\nThe docs are not.",
        "options": options,
        "recommendation": "ship",
        "tradeoff": "Docs can follow, but readers will miss the flag.",
        "whatWaits": "the cache PR",
        "nextAction": "merge the PR",
        "coordinator": "cache-lead",
        "key": "cache",
        "runId": "run-7",
        "session": "cache-worker",
        "colour": "blue",
    })
    .to_string()
}

#[test]
fn an_outside_ask_takes_fleets_json_shows_on_the_desk_and_a_changed_one_is_a_new_revision() {
    let setup = Setup::new("outside-ask");

    let asked = setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);

    assert!(asked.status.success(), "{asked:?}");
    assert_eq!(String::from_utf8_lossy(&asked.stdout), "cache.ship\n");
    assert_eq!(
        String::from_utf8_lossy(&asked.stderr),
        "priory ask: ignored unknown field colour\n"
    );
    let row = |revision: &'static str, options: &'static str| {
        vec![
            "cache.ship",
            "-",
            revision,
            "Ship the cache today?",
            options,
            "-",
            "-",
            "Ship the cache today?",
            r"The cache is ready.\n\nThe docs are not.",
            "the cache PR",
            "merge the PR",
            "-",
            "cache-lead",
            "cache",
            "run-7",
            "cache-worker",
        ]
    };
    assert_eq!(setup.rows("asks"), [row("1", r#"["ship","wait"]"#)]);
    let desk = setup.rows("desk");
    assert_eq!(
        [&desk[0][1], &desk[0][2], &desk[0][3]],
        ["ask", "cache.ship", "-"]
    );

    let same = setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);
    let changed = setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "drop"])]);

    assert!(same.status.success() && changed.status.success());
    assert_eq!(setup.rows("asks"), [row("2", r#"["ship","drop"]"#)]);
}

#[test]
fn a_dismissed_agent_ask_wakes_its_task_with_one_line_and_a_stale_dismiss_is_refused() {
    let mut setup = Setup::with_workflow("dismiss", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    let stale = setup.priory(&["dismiss", "--expected", "2", "1"]);
    let dismissed = setup.priory(&["dismiss", "--expected", "1", "1"]);
    let answered = setup.priory(&["answer", "--expected", "1", "--", "1", "sqlite"]);

    assert_eq!(
        String::from_utf8_lossy(&stale.stderr),
        "priory dismiss: ask 1 is at revision 1, not 2\n"
    );
    assert!(dismissed.status.success(), "{dismissed:?}");
    assert_eq!(
        String::from_utf8_lossy(&dismissed.stdout),
        "ask 1 dismissed\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&answered.stderr),
        "priory answer: ask 1 was dismissed\n"
    );
    assert!(setup.rows("asks").is_empty());
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        "A person dismissed ask 1 without an answer. Continue your task.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n"
    );
}

#[test]
fn an_outside_ask_takes_an_answer_or_a_dismiss_only_at_its_current_revision() {
    let mut setup = Setup::new("outside-answer");
    setup.start();
    listening(&setup);
    for options in [["ship", "wait"], ["ship", "drop"]] {
        setup.priory(&["ask", "cache.ship", &fleet_ask(&options)]);
        setup.priory(&["ask", "cache.notes", &fleet_ask(&options)]);
    }

    let stale = setup.priory(&["answer", "--expected", "1", "--", "cache.ship", "ship"]);
    let answered = setup.priory(&["answer", "--expected", "2", "--", "cache.ship", "ship"]);
    let stale_dismiss = setup.priory(&["dismiss", "--expected", "1", "cache.notes"]);
    let dismissed = setup.priory(&["dismiss", "cache.notes"]);
    setup.priory(&["ask", "-1", &fleet_ask(&["ship"])]);
    let negative = setup.priory(&["dismiss", "--", "-1"]);

    assert_eq!(
        String::from_utf8_lossy(&negative.stdout),
        "ask -1 dismissed\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&stale.stderr),
        "priory answer: ask cache.ship is at revision 2, not 1\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&answered.stdout),
        "ask cache.ship answered\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&stale_dismiss.stderr),
        "priory dismiss: ask cache.notes is at revision 2, not 1\n"
    );
    assert_eq!(
        String::from_utf8_lossy(&dismissed.stdout),
        "ask cache.notes dismissed\n"
    );
    assert!(setup.rows("asks").is_empty());
}

/// A fake `deliver` command that records its arguments, then runs `then`: each call ends with
/// `\x1e`, each argument with `\0`.
fn delivering(name: &str, then: &str) -> Setup {
    let setup = Setup::new(name);
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\n\n[asks]\ndeliver = \"bin/deliver\"\n",
    )
    .unwrap();
    let deliver = setup.dir.join("factory/bin/deliver");
    fs::create_dir_all(deliver.parent().unwrap()).unwrap();
    fs::write(
        &deliver,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\"; done >> \"$FAKE_DIR/deliver.log\"\nprintf '\\036' >> \"$FAKE_DIR/deliver.log\"\n{then}\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&deliver, fs::Permissions::from_mode(0o755)).unwrap();
    setup
}

/// The arguments of each call of the fake `deliver`, in order.
fn delivered(setup: &Setup) -> Vec<Vec<String>> {
    fs::read_to_string(setup.dir.join("fake/deliver.log"))
        .unwrap_or_default()
        .split_terminator('\x1e')
        .map(|call| call.split_terminator('\0').map(str::to_owned).collect())
        .collect()
}

/// Polls `priory asks --json` until the delivery of ask `name` satisfies `done`, and returns it.
fn delivery(
    setup: &Setup,
    name: &str,
    done: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let output = setup.priory(&["asks", "--json"]);
        let asks: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let found = asks["asks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ask| ask["id"] == name)
            .map(|ask| ask["delivery"].clone())
            .unwrap_or_default();
        if done(&found) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "timed out on the delivery of {name}: {found}\ndaemon: {}",
            fs::read_to_string(setup.dir.join("daemon.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_answered_outside_ask_runs_the_deliver_command_once_with_its_name_revision_session_and_answer()
{
    let mut setup = delivering("deliver-once", "exit 0");
    setup.start();
    listening(&setup);
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);

    let answered = setup.priory(&["answer", "--expected", "1", "--", "cache.ship", "ship"]);

    assert!(answered.status.success(), "{answered:?}");
    let done = delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "delivered"
    });
    assert_eq!(done["tries"], 1);
    assert_eq!(
        delivered(&setup),
        [["cache.ship", "1", "cache-worker", "answered", "ship"]]
    );
}

#[test]
fn a_failing_deliver_command_is_retried_with_a_backoff_then_its_failure_shows_on_the_desk() {
    let mut setup = delivering("deliver-fails", "echo 'hc is down' >&2; exit 3");
    setup.start();
    listening(&setup);
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);
    let started = Instant::now();

    setup.priory(&["answer", "--expected", "1", "--", "cache.ship", "ship"]);

    let failed = delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "failed"
    });
    assert_eq!(failed["tries"], 5);
    assert_eq!(failed["error"], "exit 3: hc is down");
    assert_eq!(delivered(&setup).len(), 5);
    assert!(
        started.elapsed() >= Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    let desk = setup.rows("desk");
    let item = desk.iter().find(|item| item[1] == "delivery").unwrap();
    assert_eq!(
        [&item[2], &item[5]],
        [
            "cache.ship",
            "the answer did not reach cache-worker after 5 tries: exit 3: hc is down"
        ]
    );
}

#[test]
fn a_daemon_restarted_while_a_delivery_runs_waits_for_that_command_and_delivers_once() {
    let mut setup = delivering(
        "deliver-restart",
        "while [ ! -e \"$FAKE_DIR/go\" ]; do sleep 0.05; done",
    );
    setup.start();
    listening(&setup);
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);
    setup.priory(&["answer", "--expected", "1", "--", "cache.ship", "ship"]);
    delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "running"
    });
    setup.written("fake/deliver.log");

    setup.crash();
    setup.start();
    listening(&setup);
    fs::write(setup.dir.join("fake/go"), "").unwrap();

    let done = delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "delivered"
    });
    assert_eq!(done["tries"], 1);
    assert_eq!(delivered(&setup).len(), 1);
}

#[test]
fn a_retried_delivery_goes_to_the_session_that_asked_even_after_the_ask_changed() {
    let mut setup = delivering(
        "deliver-session",
        "[ -e \"$FAKE_DIR/failed\" ] || { touch \"$FAKE_DIR/failed\"; exit 3; }",
    );
    setup.start();
    listening(&setup);
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);
    setup.priory(&["answer", "--expected", "1", "--", "cache.ship", "ship"]);
    setup.written("fake/deliver.log");

    let moved = fleet_ask(&["ship", "wait"]).replace("cache-worker", "other-worker");
    setup.priory(&["ask", "cache.ship", &moved]);

    delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "delivered"
    });
    let sessions: Vec<String> = delivered(&setup)
        .into_iter()
        .map(|call| call[2].clone())
        .collect();
    assert_eq!(sessions, ["cache-worker", "cache-worker"]);
}

#[test]
fn without_a_deliver_command_an_outside_answer_is_only_stored() {
    let mut setup = Setup::new("deliver-none");
    setup.start();
    listening(&setup);
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);

    setup.priory(&["answer", "--expected", "1", "--", "cache.ship", "ship"]);

    let output = setup.priory(&["asks", "--json"]);
    let asks: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let ask = &asks["asks"][0];
    assert_eq!([&ask["state"], &ask["answer"]], ["answered", "ship"]);
    assert_eq!(ask.get("delivery"), Some(&serde_json::Value::Null));
}

#[test]
fn an_answer_with_shell_metacharacters_reaches_the_deliver_command_unchanged() {
    let mut setup = delivering("deliver-quoting", "exit 0");
    setup.start();
    listening(&setup);
    let session = "worker'; touch \"$FAKE_DIR/pwned\" #";
    let ask = fleet_ask(&[]).replace("cache-worker", &session.replace('"', "\\\""));
    setup.priory(&["ask", "cache.ship", &ask]);
    let answer = "ship it; $(touch pwned) `id` \"quoted\" 'single' $HOME *\nand a second line\\";

    setup.priory(&["answer", "--expected", "1", "--", "cache.ship", answer]);

    delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "delivered"
    });
    assert_eq!(
        delivered(&setup),
        [["cache.ship", "1", session, "answered", answer]]
    );
    assert!(!setup.dir.join("fake/pwned").exists());
}

#[test]
fn a_dismissed_outside_ask_is_delivered_as_dismissed_with_no_answer() {
    let mut setup = delivering("deliver-dismiss", "exit 0");
    setup.start();
    listening(&setup);
    setup.priory(&["ask", "cache.ship", &fleet_ask(&["ship", "wait"])]);

    let dismissed = setup.priory(&["dismiss", "cache.ship"]);

    assert!(dismissed.status.success(), "{dismissed:?}");
    delivery(&setup, "cache.ship", |delivery| {
        delivery["state"] == "delivered"
    });
    assert_eq!(
        delivered(&setup),
        [["cache.ship", "1", "cache-worker", "dismissed", ""]]
    );
}

#[test]
fn an_outside_ask_without_a_session_records_the_hc_terminal_and_claude_session_that_asked() {
    let setup = Setup::new("ask-session");
    let ask = |json: &str| {
        setup
            .command(env!("CARGO_BIN_EXE_priory"))
            .args(["ask", "cache.ship", json])
            .env("HC_TERMINAL_ID", "t9")
            .env("CLAUDE_CODE_SESSION_ID", "8f3c2a10")
            .output()
            .unwrap()
    };

    let asked = ask(r#"{"title": "Ship?"}"#);
    assert!(asked.status.success(), "{asked:?}");
    assert_eq!(setup.rows("asks")[0][15], "t9 8f3c2a10");

    ask(r#"{"title": "Ship?", "session": "cache-worker"}"#);
    assert_eq!(setup.rows("asks")[0][15], "cache-worker");
}

#[test]
fn say_types_its_text_into_an_idle_run() {
    let mut setup = Setup::new("say-idle");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);

    let said = setup.priory(&["say", "1", "Also fix the typo in the README."]);

    assert!(said.status.success(), "{said:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert_eq!(
        setup.typed()[2],
        "Also fix the typo in the README.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n"
    );
}

#[test]
fn say_before_the_stop_of_a_turn_that_asked_is_typed_into_the_same_run() {
    let mut setup = Setup::with_workflow("say-before-stop", STEPS);
    setup.hold("hold-stop");
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);

    let said = setup.priory(&["say", "1", "Pick sqlite and go on."]);

    assert!(said.status.success(), "{said:?}");
    setup.release("hold-stop");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert!(
        setup.typed()[1].starts_with("Pick sqlite and go on.\n"),
        "{:?}",
        setup.typed()
    );
    assert_eq!(setup.rows("runs").len(), 1);
}

#[test]
fn say_to_a_ready_task_no_run_took_yet_comes_before_its_step_prompt() {
    let workflow = STEPS
        .replace("start = \"plan\"", "start = \"approve\"")
        .replace("approved = \"implement\"", "approved = \"plan\"");
    let mut setup = Setup::with_workflow("say-ready", &workflow);
    setup.publish(&requested(42));
    setup.memory(0);
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    assert!(
        setup
            .priory(&["answer", "1", "approved", "--revision", "1"])
            .status
            .success()
    );
    setup.until("tasks", |rows| rows[0][4] == "ready");

    let said = setup.priory(&["say", "1", "Keep it small."]);

    assert!(said.status.success(), "{said:?}");
    setup.memory(60);
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let typed = setup.typed();
    assert!(typed[0].starts_with("Keep it small.\n"), "{typed:?}");
    assert!(typed[0].contains("\nPlan the change.\n"), "{typed:?}");
}

#[test]
fn say_wakes_an_asleep_task_with_its_text_in_its_session() {
    let mut setup = Setup::with_workflow("say-asleep", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    let session = setup.rows("runs")[0][4].clone();

    let said = setup.priory(&["say", "1", "Pick sqlite and go on."]);

    assert!(said.status.success(), "{said:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        "Pick sqlite and go on.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: hopeless, ready, unsure.\n"
    );
    assert_eq!(setup.rows("runs")[1][4], session);
}

#[test]
fn pause_holds_queued_tasks_while_an_answer_still_wakes_its_task() {
    let mut setup = Setup::with_workflow("pause", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    let paused = setup.priory(&["pause", "draining for the switch"]);

    assert!(paused.status.success(), "{paused:?}");
    let banner = setup.banner().unwrap();
    assert!(banner.starts_with("paused since "), "{banner}");
    assert!(banner.ends_with(": draining for the switch"), "{banner}");
    setup.publish(&requested(43));
    setup.until("tasks", |rows| {
        rows.get(1)
            .is_some_and(|row| row[7] == "waits: priory is paused: draining for the switch")
    });
    assert!(
        setup
            .priory(&["answer", "1", "sqlite", "--revision", "1"])
            .status
            .success()
    );
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    std::thread::sleep(Duration::from_millis(1500));
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[1][4], &tasks[1][3]], ["queued", "plan"]);
    assert_eq!(setup.rows("runs").len(), 2, "only task 1 ran again");
}

#[test]
fn a_pause_survives_a_daemon_restart_and_unpause_admits_again() {
    let mut setup = Setup::new("unpause");
    assert!(setup.priory(&["pause"]).status.success());
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first()
            .is_some_and(|row| row[7] == "waits: priory is paused")
    });
    setup.crash();
    setup.start();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "queued");
    assert!(setup.banner().unwrap().starts_with("paused since "));

    let unpaused = setup.priory(&["unpause"]);

    assert!(unpaused.status.success(), "{unpaused:?}");
    setup.until("runs", |rows| rows.len() == 1);
    assert_eq!(setup.banner(), None);
}

#[test]
fn say_to_a_done_task_is_refused() {
    let mut setup = Setup::new("say-done");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });

    let said = setup.priory(&["say", "1", "One more thing."]);

    assert!(!said.status.success());
    assert_eq!(
        String::from_utf8_lossy(&said.stderr),
        "priory say: task 1 is done\n"
    );
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
    setup.calls(&[
        json!({ "name": "Bash", "arguments": { "command": "rm -rf /" } }),
        ask(),
    ]);
    setup.hold("undecided");
    setup.hold("hold-stop");
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");
    setup.crash();
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    std::thread::sleep(Duration::from_millis(1100));
    // The agent submits other text, so the prompt stays uncertain.
    fs::write(setup.dir.join("fake/trailer"), " Wait.").unwrap();
    setup.release("hold-submit");

    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    fs::remove_file(setup.dir.join("fake/trailer")).unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    setup.hold("hold-submit");
    setup.release("hold-stop");
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
        "run 1 stopped on a permission prompt: Claude needs your permission to use Bash"
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
    setup.hold("undecided");
    setup.hold("hold-stop");
    setup.calls(&[json!({ "name": "Bash", "arguments": { "command": "rm -rf /" } })]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let session = setup.rows("runs")[0][4].clone();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });

    let resumed = setup.priory(&["resume", "1"]);

    assert!(resumed.status.success(), "{resumed:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let runs = setup.rows("runs");
    assert_eq!([&runs[1][4], &runs[1][7]], [&session, "-"]);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    assert!(args.contains(&format!("--resume\n{session}\n")), "{args}");
    assert!(args.contains("--permission-mode\nmanual\n"), "{args}");
    assert_eq!(
        setup.typed()[1],
        "A person resumed this task after: run 1 stopped on a permission prompt: Claude needs your permission to use Bash. Check the worktree, then continue.\nEnd this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.\n"
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
fn a_snoozed_blocked_task_that_blocks_again_comes_back_early_with_a_new_failure() {
    let mut setup = Setup::new("snooze-blocked");
    setup.hold("undecided");
    setup.hold("hold-stop");
    let prompt = json!({ "name": "Bash", "arguments": { "command": "rm -rf /" } });
    setup.calls(&[prompt.clone(), prompt]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    let snoozed = setup.priory(&["snooze", "blocked", "1", "--for", "1d"]);
    assert!(snoozed.status.success(), "{snoozed:?}");
    assert!(desk(&setup, &[]).is_empty());
    assert_eq!(setup.rows("tasks")[0][4], "blocked");

    let resumed = setup.priory(&["resume", "1"]);

    assert!(resumed.status.success(), "{resumed:?}");
    setup.until("runs", |rows| rows.len() == 2);
    setup.until("tasks", |rows| rows[0][4] == "blocked");
    let back = desk(&setup, &[]);
    assert_eq!([&back[0][1], &back[0][2]], ["blocked", "1"]);
    assert_eq!(back[0][7], "-");
    assert!(back[0][8].contains(r#""reason":"new failure""#), "{back:?}");
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
fn a_worktree_a_person_deleted_is_added_back_from_its_branch_on_the_next_run() {
    let mut setup = Setup::new("deleted-worktree");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    setup.reboot();
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::remove_dir_all(&worktree).unwrap();

    setup.start();

    let runs = setup.until("runs", |rows| rows.len() == 2 && rows[1][7] == "-");
    assert_eq!(runs[0][7], "lost");
    assert!(worktree.join("README.md").exists());
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
    setup.hold("undecided");
    setup.hold("hold-stop");
    setup.calls(&[json!({ "name": "Bash", "arguments": { "command": "rm -rf /" } })]);
    setup.hold("hold-submit");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.written("fake/pending");
    setup.crash();
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    // The agent submits other text, so the prompt stays uncertain.
    fs::write(setup.dir.join("fake/trailer"), " Wait.").unwrap();
    setup.release("hold-submit");
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    fs::remove_file(setup.dir.join("fake/trailer")).unwrap();

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
fn a_run_moved_into_another_agents_step_with_an_unconfirmed_prompt_does_not_hold_back_the_next_run()
{
    let workflow = STEPS.replace(
        "agent = \"implementer\"\nprompt = \"Implement the plan.\"",
        "agent = \"reviewer\"\nprompt = \"Implement the plan.\"",
    );
    let mut setup = Setup::with_workflow("moved-unconfirmed", &workflow);
    // The agent submits other text, so its first prompt stays attempted.
    fs::write(setup.dir.join("fake/trailer"), " Wait.").unwrap();
    setup.calls(&[result("ready")]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/submitted");
    fs::remove_file(setup.dir.join("fake/trailer")).unwrap();
    setup.release("hold-call");

    let outbox = setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[3] == "confirmed" && row[4].starts_with("Implement the plan."))
    });

    assert_eq!(outbox[0][3], "attempted");
    assert_eq!(setup.rows("runs")[0][7], "moved");
}

#[test]
fn a_run_carried_into_the_next_step_gets_no_prompt_while_its_process_may_hold_an_unconfirmed_one() {
    let mut setup = Setup::with_workflow("carried-unconfirmed", STEPS);
    // The agent submits other text, so its first prompt stays attempted.
    fs::write(setup.dir.join("fake/trailer"), " Wait.").unwrap();
    setup.calls(&[result("ready")]);
    setup.publish(&requested(42));
    setup.start();

    let runs = setup.until("runs", |rows| rows.len() == 2);
    std::thread::sleep(Duration::from_millis(1500));

    assert_eq!(runs[0][7], "moved");
    assert_eq!(setup.typed().len(), 1);
    assert_eq!(setup.rows("outbox")[1][3], "queued");
}

#[test]
fn an_upgrade_hands_the_unconfirmed_prompt_of_a_moved_run_to_the_live_run_that_carries_its_process()
{
    let mut setup = Setup::with_workflow("carried-upgrade", STEPS);
    fs::write(setup.dir.join("fake/trailer"), " Wait.").unwrap();
    setup.calls(&[result("ready")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 2);
    setup.crash();
    // The state as the version before this fix left it.
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(UNDO_SINCE_SEARCH).unwrap();
    db.execute_batch(
        "UPDATE outbox SET run = 1 WHERE id = 1; ALTER TABLE tasks DROP COLUMN repo; ALTER TABLE outbox DROP COLUMN compact;
         PRAGMA user_version = 22;",
    )
    .unwrap();

    setup.start();

    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.typed().len(), 1);
    assert_eq!(setup.rows("outbox")[1][3], "queued");
}

#[test]
fn a_task_blocked_before_its_first_run_gets_its_step_prompt_when_resumed() {
    let mut setup = Setup::new("blocked-unstarted");
    let origin = setup.dir.join("origin/owner/repo.git");
    let away = setup.dir.join("origin/away.git");
    fs::rename(&origin, &away).unwrap();
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    fs::rename(&away, &origin).unwrap();

    assert!(setup.priory(&["resume", "1"]).status.success());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let first = &setup.typed()[0];
    assert!(first.starts_with("Implement the issue.\n"), "{first}");
}

#[test]
fn a_codex_run_lost_before_its_session_start_hands_its_first_prompt_to_the_next_run() {
    let mut setup = Setup::new("codex-lost-unstarted");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.hold("silent");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/pid");
    setup.reboot();
    setup.release("silent");

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let prompt = fs::read_to_string(setup.dir.join("fake/codex-prompt")).unwrap();
    assert!(prompt.starts_with("Priory restarted"), "{prompt}");
    assert!(prompt.contains("\nImplement the issue.\n"), "{prompt}");
}

#[test]
fn a_revived_task_gets_none_of_the_prompts_its_earlier_generation_left_queued() {
    let mut setup = Setup::new("revive-queued");
    setup.publish(&requested(42));
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    setup.publish(&labeled(42));
    setup.until("outbox", |rows| rows.len() == 2);
    assert!(setup.priory(&["cancel", "1"]).status.success());

    assert!(setup.priory(&["revive", "1"]).status.success());

    setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[3] == "confirmed" && row[4].starts_with("A person revived"))
    });
    let revived = setup.typed().pop().unwrap();
    assert!(revived.contains("\nImplement the issue.\n"), "{revived}");
    assert!(!revived.contains("github.issues.labeled"), "{revived}");
}

#[test]
fn a_codex_run_that_exits_before_its_session_start_after_a_daemon_restart_keeps_its_first_prompt() {
    let mut setup = Setup::new("codex-exit-unstarted");
    fs::write(setup.dir.join("factory/agents/implementer/agent.md"), CODEX).unwrap();
    setup.hold("silent");
    setup.publish(&requested(42));
    setup.start();
    setup.written("fake/pid");
    setup.crash();
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "uncertain") == 1);
    let pid = fs::read_to_string(setup.dir.join("fake/pid")).unwrap();
    // The restarted daemon checks the panes it found before this one exits.
    std::thread::sleep(Duration::from_millis(1500));
    let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    setup.until("tasks", |rows| rows[0][4] == "blocked");
    setup.release("silent");

    assert!(setup.priory(&["resume", "1"]).status.success());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let prompt = fs::read_to_string(setup.dir.join("fake/codex-prompt")).unwrap();
    assert!(prompt.contains("Implement the issue.\n"), "{prompt}");
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
fn the_mcp_server_lists_its_tools() {
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
    assert_eq!(
        names,
        [
            "result", "ask", "wait", "compact", "tldr", "report", "search", "progress"
        ]
    );
    let on = &replies[1]["result"]["tools"][2]["inputSchema"]["properties"]["on"];
    assert_eq!(
        on["properties"]["type"]["enum"],
        json!([
            "pull_request",
            "checks",
            "review",
            "time",
            "task",
            "linear_comment",
            "linear_state",
            "slack_reply",
            "disk"
        ])
    );
    for field in ["issue", "state", "channel", "thread_ts"] {
        assert!(on["properties"][field].is_object(), "{field} in {on}");
    }
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
fn a_state_file_whose_asks_an_approval_names_upgrades_to_outside_asks() {
    let mut setup = Setup::with_workflow("asks-upgrade", STEPS);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.crash();
    // The state file as version 32 left it: every ask belongs to a task.
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "ALTER TABLE actions DROP COLUMN stdout; ALTER TABLE actions DROP COLUMN stderr;
         ALTER TABLE actions DROP COLUMN pid; ALTER TABLE actions DROP COLUMN pid_started;
         ALTER TABLE actions DROP COLUMN ended;
         ALTER TABLE tasks DROP COLUMN progress;
         DROP TABLE deliveries;
         DROP TABLE reports;
         PRAGMA legacy_alter_table = ON;
         CREATE TABLE asks_32 (
           id INTEGER PRIMARY KEY AUTOINCREMENT, task INTEGER NOT NULL REFERENCES tasks (id),
           run INTEGER REFERENCES runs (id), step TEXT NOT NULL,
           kind TEXT NOT NULL CHECK (kind IN ('step', 'agent')), revision INTEGER NOT NULL DEFAULT 1,
           question TEXT NOT NULL, options TEXT NOT NULL, recommendation TEXT, tradeoff TEXT,
           answer TEXT, answered TEXT, created TEXT NOT NULL, withdrawn INTEGER);
         INSERT INTO asks_32 SELECT id, task, run, step, kind, revision, question, options,
           recommendation, tradeoff, answer, answered, created, withdrawn FROM asks;
         DROP TABLE asks;
         ALTER TABLE asks_32 RENAME TO asks;
         INSERT INTO approvals (task, step, ask, revision) VALUES (1, 'plan', 1, 1);
         PRAGMA user_version = 32;",
    )
    .unwrap();
    drop(db);

    let asks = setup.rows("asks");

    assert_eq!(asks.len(), 1, "{asks:?}");
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    let approvals: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'approvals'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(approvals.contains("REFERENCES asks (id)"), "{approvals}");
}

#[test]
fn a_dismiss_that_beats_the_stop_is_typed_into_the_same_run() {
    let mut setup = Setup::with_workflow("early-dismiss", STEPS);
    setup.calls(&[ask()]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);

    let dismissed = setup.priory(&["dismiss", "1"]);

    assert!(dismissed.status.success(), "{dismissed:?}");
    setup.release("hold-stop");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(
        setup.typed()[1],
        format!(
            "{}\n",
            prompted(
                "A person dismissed ask 1 without an answer. Continue your task.",
                "hopeless, ready, unsure"
            )
        )
    );
    assert_eq!(setup.rows("runs").len(), 1);
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
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
        inbox[1][1..5],
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
        json!({ "state": "approved", "actor": "ana", "github_id": 300 }),
    )
}

fn closed() -> Message {
    on_pull(
        "github.pull_request.closed",
        json!({ "merged": true, "github_id": 400 }),
    )
}

#[test]
fn a_review_a_comment_or_a_close_wakes_a_task_waiting_on_it() {
    for (at, case) in [
        (
            "review",
            reviewed(),
            "wait 1 fired: ana reviewed github:owner/repo#7: approved\ngithub.pull_request_review.submitted owner/repo#7 {",
        ),
        (
            "pull_request",
            closed(),
            "github.pull_request.closed owner/repo#7 {",
        ),
        (
            "review",
            thread_comment(),
            "wait 1 fired: ana commented on src/lib.rs:12 of github:owner/repo#7: Rename this. https://github.com/owner/repo/pull/7#discussion_r802\n",
        ),
        (
            "pull_request",
            on_pull(
                "github.pull_request.comment",
                json!({ "actor": "ana", "body": "Split this?", "url": "u", "github_id": 701 }),
            ),
            "wait 1 fired: ana commented on github:owner/repo#7: Split this? u\n",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (kind, event, line) = case;
        let setup = asleep_on(&format!("wake-{at}"), on_pull_7(kind));

        setup.publish(&event.time(herdr_bus::Timestamp::now()));

        setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
        let typed = setup.typed();
        assert!(typed[1].starts_with(line), "{kind}: {typed:?}");
        assert_eq!(setup.rows("tasks")[0][4], "running");
    }
}

#[test]
fn a_task_request_for_a_key_another_task_owns_as_a_resource_creates_its_task() {
    let setup = asleep_on("request-owned", on_pull_7("review"));

    setup.publish(&requested(7));

    let tasks = setup.until("tasks", |rows| rows.len() == 2);
    assert_eq!(tasks[1][1], "github:owner/repo#7");
    assert_eq!(setup.rows("inbox")[1][4], "created");
}

#[test]
fn an_event_on_a_key_goes_to_its_task_and_still_fires_another_owners_wait() {
    let setup = asleep_on("request-owned-event", on_pull_7("review"));
    setup.calls(&[wait(on_pull_7("checks"))]);
    setup.publish(&requested(7));
    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] == "asleep");
    setup.calls(&[wait(on_pull_7("pull_request"))]);
    setup.hold("hold-call");

    setup.publish(&reviewed());

    let inbox = setup.until("inbox", |rows| rows.len() == 3);
    assert_eq!(inbox[2][3..5], ["2", "kept"]);
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    // Task 2 keeps the close; task 1, mid-turn, waits on it next and is caught up.
    setup.publish(&closed());
    setup.until("inbox", |rows| rows.len() == 4);
    setup.release("hold-call");
    setup.until("runs", |_| setup.replies().len() == 3);
    let (text, error) = reply(&setup.replies()[2]);
    assert!(!error, "{text}");
    assert!(text.starts_with("wait 3 already holds"), "{text}");
    // Task 2 still reads the review that fired task 1's wait.
    setup.publish(&check(1, 0));
    let outbox = setup.until("outbox", |rows| {
        rows.iter()
            .any(|row| row[1] == "2" && row[4].contains("check_run"))
    });
    let woke = outbox.iter().rev().find(|row| row[1] == "2").unwrap();
    assert!(
        woke[4].contains("pull_request_review.submitted"),
        "{outbox:?}"
    );
}

#[test]
fn an_event_that_does_not_match_the_wait_stays_kept_and_reaches_the_next_run() {
    let setup = asleep_on("unmatched", on_pull_7("pull_request"));

    setup.publish(&check(1, 0));
    setup.until("inbox", |rows| rows.len() == 2);
    stays_asleep(&setup);
    assert_eq!(setup.rows("inbox")[1][4], "kept");
    setup.publish(&closed());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    std::thread::sleep(Duration::from_millis(1500));
    let typed = setup.typed();
    assert_eq!(typed.len(), 3, "{typed:?}");
    let lines: Vec<_> = typed[1].lines().collect();
    assert!(
        lines[0].starts_with("github.pull_request.closed owner/repo#7 {"),
        "{typed:?}"
    );
    assert!(
        lines[1].starts_with("github.check_run.completed owner/repo#7 {"),
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

#[test]
fn a_merge_with_a_check_still_running_marks_the_task_done() {
    let setup = gated("gate-merged");
    setup.publish(&check(1, 1));
    stays_asleep_after(&setup, 2);

    setup.publish(&closed());

    setup.until("tasks", |rows| rows[0][4] == "done");
    assert_eq!(setup.rows("runs").len(), 2);
}

#[test]
fn a_close_without_a_merge_with_a_check_still_running_wakes_the_agent() {
    let setup = gated("gate-closed");
    setup.publish(&check(1, 1));
    stays_asleep_after(&setup, 2);

    setup.publish(&on_pull(
        "github.pull_request.closed",
        json!({ "merged": false, "github_id": 400 }),
    ));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(
        typed[2].starts_with(
            "wait 2 fired: github:owner/repo#7 closed without a merge\ngithub.pull_request.closed owner/repo#7 {"
        ),
        "{typed:?}"
    );
}

fn thread_comment() -> Message {
    on_pull(
        "github.pull_request_review_comment",
        json!({
            "actor": "ana",
            "body": "Rename this.\nIt reads better.",
            "path": "src/lib.rs",
            "line": 12,
            "thread": 801,
            "url": "https://github.com/owner/repo/pull/7#discussion_r802",
            "github_id": 802
        }),
    )
}

#[test]
fn a_thread_comment_from_someone_else_wakes_the_agent_asleep_on_the_done_gate() {
    let setup = gated("gate-comment");

    setup.publish(&thread_comment());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    let lines: Vec<_> = typed[2].lines().collect();
    assert_eq!(
        lines[0],
        "wait 2 fired: ana commented on src/lib.rs:12 of github:owner/repo#7: Rename this. https://github.com/owner/repo/pull/7#discussion_r802",
        "{typed:?}"
    );
    assert!(lines[1].starts_with("github.pull_request_review_comment owner/repo#7 {"));
}

#[test]
fn a_comment_written_before_the_done_gate_wakes_nothing_and_a_later_one_wakes_the_agent() {
    let setup = gated("gate-own-comment");
    // The agent's own comment, posted in the turn that reached done, reaches the intake later.
    let written = herdr_bus::Timestamp::now().as_second() - 60;
    let own = thread_comment().time(herdr_bus::Timestamp::from_second(written).unwrap());

    let mut other = thread_comment();
    other.data["actor"] = json!("max");
    other.data["github_id"] = json!(803);

    setup.publish(&own);
    setup.until("inbox", |rows| rows.len() == 3);
    stays_asleep_after(&setup, 2);
    setup.publish(&other);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(
        typed[2].starts_with("wait 2 fired: max commented"),
        "{typed:?}"
    );
}

/// A comment from ana on pull request 7, written now.
fn commented() -> Message {
    on_pull(
        "github.pull_request.comment",
        json!({ "actor": "ana", "body": "One more thing.", "url": "u", "github_id": 701 }),
    )
    .time(herdr_bus::Timestamp::now())
}

#[test]
fn a_done_task_whose_open_pull_request_gets_a_comment_reopens_and_resumes_its_session() {
    let setup = gated("reopen");
    setup.publish(&check(1, 0));
    setup.until("tasks", |rows| rows[0][4] == "done");
    // The comment comes after the task was done, to the second.
    std::thread::sleep(Duration::from_millis(1100));

    setup.publish(&commented());

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    let typed = setup.typed();
    assert!(
        typed[2].starts_with(
            "Feedback reopened this done task as generation 2: ana commented on github:owner/repo#7: One more thing. u\ngithub.pull_request.comment owner/repo#7 {"
        ),
        "{typed:?}"
    );
    let runs = setup.rows("runs");
    assert_eq!(
        runs[2][4], runs[0][4],
        "the reopened run resumes the session"
    );
    let reopened: Vec<_> = setup
        .trace()
        .into_iter()
        .filter(|record| record["type"] == "factory.task.reopened")
        .collect();
    assert_eq!(reopened.len(), 1);
    assert_eq!(reopened[0]["data"]["generation"], 2);
    assert_eq!(reopened[0]["data"]["step"], "implement");
    assert_eq!(reopened[0]["data"]["pull_request"], "github:owner/repo#7");
}

#[test]
fn a_comment_on_the_merged_pull_request_of_a_done_task_reopens_nothing() {
    let setup = gated("reopen-merged");
    setup.publish(&closed());
    setup.until("tasks", |rows| rows[0][4] == "done");
    std::thread::sleep(Duration::from_millis(1100));

    setup.publish(&commented());

    setup.until("inbox", |rows| rows.len() == 4);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "done");
    assert_eq!(setup.rows("runs").len(), 2);
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
            "-",
            "-",
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

#[test]
fn a_gh_call_past_the_command_timeout_is_killed_and_its_action_fails() {
    let mut setup = Setup::with_workflow("open-pr-hangs", OPEN_PR);
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\ncommand_timeout = \"1s\"\n",
    )
    .unwrap();
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("asks", |rows| rows.len() == 1);
    setup.gone("priory-1");
    setup.hold("gh-hang");

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4], "failed");
    assert!(actions[0][7].ends_with("timed out after 1s"), "{actions:?}");
}

/// A workflow whose one step runs `command` and routes `found`, `running`, `ok` and `failed`.
fn run_step(command: &str, timeout: &str) -> String {
    format!(
        r#"
start = "poll"

[[steps]]
id = "poll"
decides = "code"
action = "run"
run = {command:?}
effect = "reversible"
{timeout}
next = {{ found = "done", running = "done", ok = "stop", failed = "stop" }}
"#
    )
}

/// Starts task 1 on a run step of `command` and returns once its action ended.
fn ran(name: &str, command: &str, timeout: &str) -> (Setup, Vec<Vec<String>>) {
    let mut setup = Setup::with_workflow(name, &run_step(command, timeout));
    setup.publish(&requested(42));
    setup.start();
    let actions = setup.until("actions", |rows| rows.len() == 1 && rows[0][4] != "running");
    (setup, actions)
}

#[test]
fn a_run_step_whose_last_line_names_an_outcome_routes_there() {
    let (setup, actions) = ran(
        "run-found",
        "echo checking; echo \"task $PRIORY_TASK\" >&2; echo found",
        "",
    );

    assert_eq!(actions[0][1..5], ["1", "poll", "run", "found"]);
    assert_eq!(actions[0][8..], ["checking\\nfound", "task 1"]);
    assert_eq!(setup.rows("tasks")[0][4], "done");
}

#[test]
fn a_run_step_routes_ok_on_exit_0_and_failed_on_another_exit() {
    let (_quiet, quiet) = ran("run-ok", "echo nothing new", "");
    let (setup, broken) = ran("run-failed", "echo found; exit 3", "");

    assert_eq!(quiet[0][4..8], ["ok", "-", "-", "-"]);
    // A named last line routes even on a failing exit, which the error keeps.
    assert_eq!(broken[0][4..9], ["found", "-", "-", "exit 3", "found"]);
    let (_crashed, crashed) = ran("run-crashed", "echo oops >&2; exit 1", "");
    assert_eq!(
        crashed[0][4..10],
        ["failed", "-", "-", "exit 1", "", "oops"]
    );
    assert_eq!(setup.rows("tasks")[0][4], "done");
}

#[test]
fn a_run_step_past_its_timeout_is_killed_with_its_children_and_fails() {
    let (setup, actions) = ran(
        "run-timeout",
        "sleep 60 & echo $! > child; echo found; wait",
        "timeout = \"1s\"",
    );

    assert_eq!(actions[0][4], "failed");
    assert_eq!(actions[0][7], "timed out after 1s");
    let child = fs::read_to_string(setup.dir.join("factory/child")).unwrap();
    assert!(!alive(child.trim()), "the backgrounded sleep still runs");
    assert_eq!(setup.rows("tasks")[0][4], "dropped");
}

#[test]
fn a_restart_while_a_run_step_runs_kills_it_and_fails_it_without_running_it_again() {
    let mut setup = Setup::with_workflow(
        "run-restart",
        &run_step(
            "echo started >> starts; echo $$ > pid; sleep 60; echo found",
            "",
        ),
    );
    setup.publish(&requested(42));
    setup.start();
    setup.written("factory/pid");
    let pid = fs::read_to_string(setup.dir.join("factory/pid")).unwrap();

    setup.crash();
    setup.start();

    let actions = setup.until("actions", |rows| rows[0][4] != "running");
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0][4..8],
        ["failed", "-", "-", "interrupted by a daemon restart"]
    );
    assert!(!alive(pid.trim()), "the interrupted command still runs");
    let starts = fs::read_to_string(setup.dir.join("factory/starts")).unwrap();
    assert_eq!(starts, "started\n");
    assert_eq!(setup.rows("tasks")[0][4], "dropped");
}

#[test]
fn an_outcome_named_running_is_an_ended_action_a_restart_leaves_alone() {
    let mut setup = Setup::with_workflow("run-named-running", &run_step("echo running", ""));
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let actions = setup.rows("actions");

    setup.crash();
    setup.start();
    // A second task's action ends after the restarted daemon reaped its predecessor's commands.
    setup.publish(&requested(43));
    setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "done");

    assert_eq!(actions[0][4], "running");
    assert_eq!(setup.rows("actions")[0], actions[0]);
}

/// Whether process `pid` is alive, not a zombie.
fn alive(pid: &str) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit_once(')')
            .unwrap()
            .1
            .trim_start()
            .starts_with('Z')
    })
}

#[test]
fn a_tmux_load_buffer_past_the_command_timeout_is_killed_and_logged() {
    let mut setup = Setup::new("load-buffer-hangs");
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\ncommand_timeout = \"1s\"\n",
    )
    .unwrap();
    setup.hold("tmux-hang");
    setup.publish(&requested(42));

    setup.start();

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap_or_default();
        if log.contains("load-buffer") && log.contains("timed out after 1s") {
            break;
        }
        assert!(Instant::now() < deadline, "daemon log: {log}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_git_fetch_past_the_command_timeout_is_killed_and_its_task_blocks() {
    let mut setup = Setup::new("fetch-hangs");
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\ncommand_timeout = \"1s\"\n",
    )
    .unwrap();
    setup.hold("git-hang");
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "blocked")
    });
    assert!(tasks[0][7].ends_with("timed out after 1s"), "{tasks:?}");
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
fn a_new_revision_of_a_snoozed_ask_brings_it_back_early() {
    let setup = asked_to_open("snooze-revision", APPROVE_THEN_POLISH);
    let snoozed = setup.priory(&["snooze", "ask", "1", "--for", "1d"]);
    assert!(snoozed.status.success(), "{snoozed:?}");
    assert!(setup.rows("asks").is_empty());
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
    assert_eq!(asks[0][5], "-");
    let json: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["asks", "--json"]).stdout).unwrap();
    let json = &json["asks"];
    assert_eq!(json[0]["back_early"]["reason"], "new revision", "{json}");
    assert!(json[0]["snoozed_until"].is_null(), "{json}");
}

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
fn pause_holds_a_new_task_whose_workflow_starts_at_a_code_step() {
    let workflow = UNKNOWN_ACTION
        .replace("start = \"implement\"", "start = \"merge\"")
        .replace("merged = \"done\"", "merged = \"implement\"");
    let mut setup = Setup::with_workflow("pause-code", &workflow);
    assert!(setup.priory(&["pause"]).status.success());
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| rows.len() == 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("tasks")[0][4], "queued");

    assert!(setup.priory(&["unpause"]).status.success());

    setup.until("tasks", |rows| rows[0][4] == "blocked");
}

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
            setup.calls_left(1);
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
fn a_woken_run_can_wait_on_something_else_and_so_withdraws_the_ask_it_took_over() {
    let mut setup = support_day("tick-wait-instead");
    setup.calls(&[ask(), wait(json!({ "type": "time", "until": "1 hour" }))]);
    setup.publish(&tick("2026-11-19T08:00:00Z"));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    setup.publish(&tick("2026-11-19T09:00:00Z"));

    setup.until("waits", |rows| rows.len() == 1);
    setup.until("tasks", |rows| rows[0][4] == "asleep");
    assert!(setup.rows("asks").is_empty());
    let answer = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);
    assert_eq!(
        String::from_utf8_lossy(&answer.stderr),
        "priory answer: ask 1 was withdrawn when a later run of task 1 called another tool\n"
    );
}

#[test]
fn a_woken_run_that_waits_on_another_condition_is_caught_up_on_the_events_of_its_turn() {
    let mut setup = support_day("tick-rewait");
    setup.calls(&[wait(on_pull_7("review")), wait(on_pull_7("pull_request"))]);
    setup.publish(&tick("2026-11-19T08:00:00Z"));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.gone("priory-1");
    setup.hold("hold-call");
    setup.publish(&tick("2026-11-19T09:00:00Z"));
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    setup.publish(&closed());
    setup.until("inbox", |rows| rows.len() == 3);

    setup.release("hold-call");

    setup.until("runs", |_| setup.replies().len() == 2);
    let (text, error) = reply(&setup.replies()[1]);
    assert!(!error, "{text}");
    assert!(text.starts_with("wait 2 already holds"), "{text}");
}

#[test]
fn an_event_that_satisfies_the_wait_of_a_woken_task_waiting_for_room_fires_it() {
    let mut setup = support_day("tick-wait");
    setup.calls(&[wait(on_pull_7("pull_request")), result("failed")]);
    setup.publish(&tick("2026-11-19T08:00:00Z"));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.gone("priory-1");
    setup.memory(0);
    setup.publish(&tick("2026-11-19T09:00:00Z"));
    setup.until("tasks", |rows| rows[0][4] == "ready");

    setup.publish(&closed());

    setup.until("waits", |rows| rows.is_empty());
    setup.memory(60);
    setup.until("tasks", |rows| rows[0][4] == "dropped");
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
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
        ("lane", json!("none/misc")),
        ("machine", json!("runner")),
        ("harness", json!("claude-code")),
    ] {
        assert_eq!(data[field], value, "{field} in {data}");
    }
    assert!(data["duration_ms"].is_u64(), "{data}");
    assert!(data["started"].is_string(), "{data}");
    assert!(data.get("factory_revision").is_none(), "{data}");
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
    assert_eq!(
        held,
        [["held", "1", "1", "none/misc", reason, "1", "-", "-"]]
    );

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
    setup.memory(60);
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
        asks[0][1..7],
        [
            "2",
            "1",
            "Pull request github:owner/repo#7 closed without a merge, and task 2 was held until it merged: b. Release the task, or drop it?",
            r#"["release","drop"]"#,
            "-",
            "-"
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

#[test]
fn a_blockers_hold_stays_when_the_newest_blockers_are_unknown() {
    let mut setup = Setup::new("hold-blockers-unknown");
    setup.memory(0);
    let source = linear_7("linear.issue.blockers", &[]);
    let mut request = requested(42);
    request.data["key"] = json!("linear:ENG-7");
    request.data["source"] = json!(source.id.to_string());
    setup.publish(&source);
    let mut unknown = linear_7("linear.issue.blockers", &[]);
    unknown.data["blockers"] = json!(null);
    setup.publish(&unknown);
    setup.publish(&request);
    setup.start();
    setup.until("tasks", |rows| rows.len() == 1);

    let reply = person(&setup, "hold", &["1", "--reason", "blocked", "--blockers"]);

    assert_eq!(
        reply,
        "task 1 held: blocked, until linear:ENG-7 has no open blocker\n"
    );
    assert_eq!(held_as(&setup, "1")[0], "held");
}

#[test]
fn a_close_that_does_not_say_whether_it_merged_asks_rather_than_drops() {
    let setup = queued_without_room("hold-merged-unknown");
    person(
        &setup,
        "hold",
        &["1", "--reason", "a", "--merge", "7", "--if-closed", "drop"],
    );

    setup.publish(&on_pull(
        "github.pull_request.closed",
        json!({ "merged": null, "github_id": 400, "head_branch": "feature" }),
    ));

    let asks = setup.until("asks", |rows| rows.len() == 1);
    assert_eq!(
        asks[0][3],
        "Pull request github:owner/repo#7 closed, and GitHub no longer says whether it merged, and task 1 was held until it merged: a. Release the task, or drop it?"
    );
    assert_eq!(held_as(&setup, "1")[0], "held");
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
fn a_revive_while_another_task_owns_one_of_its_resources_is_refused() {
    let mut setup = Setup::new("revive-resource");
    let on = on_pull_7("review");
    setup.calls(&[wait(on.clone()), result("failed"), wait(on)]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup.publish(&reviewed());
    setup.until("tasks", |rows| rows[0][4] == "dropped");
    setup.publish(&requested(43));
    setup.until("waits", |rows| rows.len() == 1);

    let revived = setup.priory(&["revive", "1"]);

    assert!(!revived.status.success());
    assert_eq!(
        String::from_utf8_lossy(&revived.stderr),
        "priory revive: task 2 is already active on github:owner/repo#7\n"
    );
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

fn wait_any(conditions: &[serde_json::Value]) -> serde_json::Value {
    json!({ "name": "wait", "arguments": { "any": conditions } })
}

fn linear_done(issue: &str) -> serde_json::Value {
    json!({ "type": "linear_state", "issue": issue, "state": "Done" })
}

fn moved_to_done(issue: &str) -> Message {
    linear_event(
        "linear.issue.state_changed",
        issue,
        json!({ "linear_id": format!("{issue}-Done"), "from": "In Progress", "to": "Done" }),
    )
}

#[test]
fn a_wait_on_a_merge_or_a_linear_state_wakes_on_the_first_and_withdraws_the_other() {
    let mut setup = Setup::new("any-of");
    setup.calls(&[wait_any(&[on_pull_7("pull_request"), linear_done("ENG-7")])]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    setup.gone("priory-1");
    let (text, error) = reply(&setup.replies()[0]);
    assert!(!error, "{text}");
    assert_eq!(
        text,
        "waits 1, 2 stored: end your turn now; Priory wakes you with the first event that satisfies one of them"
    );
    let open = setup.rows("waits");
    assert_eq!(
        [&open[0][3], &open[0][4], &open[1][3], &open[1][4]],
        [
            "pull_request",
            "github:owner/repo#7",
            "linear_state",
            "linear:ENG-7"
        ]
    );
    fs::write(setup.dir.join("fake/hold-call"), "").unwrap();

    setup.publish(&closed());
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert_eq!(setup.rows("waits"), Vec::<Vec<String>>::new());
    setup.publish(&moved_to_done("ENG-7"));
    std::thread::sleep(Duration::from_millis(1500));

    let typed = setup.typed();
    assert!(
        typed[1].starts_with("github.pull_request.closed owner/repo#7 {"),
        "{typed:?}"
    );
    assert_eq!(setup.rows("waits"), Vec::<Vec<String>>::new());
}

#[test]
fn a_linear_state_wait_on_an_issue_already_in_that_state_wakes_the_task_at_once() {
    let mut setup = Setup::new("linear-state-already");
    setup.calls(&[wait(linear_done("ENG-7"))]);
    fs::write(
        setup.dir.join("fake/linear"),
        json!({ "data": { "issue": { "state": { "name": "Done" } } } }).to_string(),
    )
    .unwrap();
    setup.publish(&requested(42));

    setup.start_with(
        Path::new(env!("CARGO_BIN_EXE_priory")),
        &[("LINEAR_API_KEY", "lin_api_test")],
    );

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let (text, error) = reply(&setup.replies()[0]);
    assert_eq!(
        (text.as_str(), error),
        (
            "wait 1 stored: end your turn now; Priory wakes you with the event that satisfies it",
            false
        )
    );
    assert!(
        setup.typed()[1].starts_with("wait 1 fired: ENG-7 is already Done\n"),
        "{:?}",
        setup.typed()
    );
}

#[test]
fn an_answer_beside_a_wait_wakes_the_task_and_withdraws_the_wait() {
    let mut setup = Setup::new("ask-and-wait");
    setup.calls(&[json!([ask(), wait(on_pull_7("pull_request"))])]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    let replies = setup.replies();
    assert!(
        replies.iter().all(|reply| !self::reply(reply).1),
        "{replies:?}"
    );
    assert_eq!(setup.rows("waits").len(), 1);
    fs::write(setup.dir.join("fake/hold-call"), "").unwrap();

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert!(
        setup.typed()[1].starts_with("sqlite\n"),
        "{:?}",
        setup.typed()
    );
    assert_eq!(setup.rows("waits"), Vec::<Vec<String>>::new());
    setup.publish(&closed());
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("waits"), Vec::<Vec<String>>::new());
    assert_eq!(setup.typed().len(), 2, "{:?}", setup.typed());
}

#[test]
fn an_event_beside_an_ask_wakes_the_task_and_a_late_answer_is_refused() {
    let mut setup = Setup::new("wait-and-ask");
    setup.calls(&[json!([wait(on_pull_7("pull_request")), ask()])]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    fs::write(setup.dir.join("fake/hold-call"), "").unwrap();

    setup.publish(&closed());
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);

    assert!(!answered.status.success(), "{answered:?}");
    assert!(
        String::from_utf8_lossy(&answered.stderr)
            .contains("ask 1 was withdrawn when a wait of its run fired first"),
        "{answered:?}"
    );
    assert_eq!(setup.rows("asks"), Vec::<Vec<String>>::new());
    assert_eq!(setup.typed().len(), 2, "{:?}", setup.typed());
}

#[test]
fn a_pull_request_wait_on_one_already_merged_wakes_the_task_at_once() {
    let mut setup = Setup::new("pull-already");
    setup.calls(&[wait_any(&[
        json!({ "type": "time", "until": "2 hours" }),
        on_pull_7("pull_request"),
    ])]);
    fs::write(setup.dir.join("fake/pr-state"), "MERGED\n").unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert!(
        setup.typed()[1].starts_with("wait 2 fired: github:owner/repo#7 is already merged\n"),
        "{:?}",
        setup.typed()
    );
    assert_eq!(setup.rows("waits"), Vec::<Vec<String>>::new());
}

#[test]
fn an_answer_before_the_stop_of_a_run_beside_a_wait_keeps_the_run_going() {
    let mut setup = Setup::new("answer-before-stop");
    setup.calls(&[json!([ask(), wait(on_pull_7("pull_request"))])]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    setup.start();
    setup.until("waits", |rows| rows.len() == 1);

    let answered = setup.priory(&["answer", "1", "sqlite", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.release("hold-stop");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert!(
        setup.typed()[1].starts_with("sqlite\n"),
        "{:?}",
        setup.typed()
    );
    assert_eq!(setup.rows("runs").len(), 1);
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
        self.intake_with(&[]);
    }

    fn intake_with(&self, env: &[(&str, &str)]) {
        let output = self.poll(env);
        assert!(
            output.status.success(),
            "intake: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// One poll of `priory intake`, which may fail.
    fn poll(&self, env: &[(&str, &str)]) -> Exit {
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
        self.command(env!("CARGO_BIN_EXE_priory"))
            .args(["intake", "--once"])
            .arg(&factory)
            .env("PATH", path)
            .env("SLACK_BOT_TOKEN", SLACK_TOKEN)
            .env("LINEAR_API_KEY", "lin_api_secret_test_key")
            .envs(env.iter().copied())
            .output()
            .unwrap()
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

#[test]
fn a_failing_slack_channel_still_lets_a_watched_thread_publish_its_reply() {
    let setup = asleep_on(
        "slack-channel-fails",
        json!({ "type": "slack_reply", "channel": "C0123", "thread_ts": THREAD }),
    );
    let trigger = setup.dir.join("intake-factory/triggers/left.toml");
    fs::create_dir_all(trigger.parent().unwrap()).unwrap();
    fs::write(
        trigger,
        "source = \"slack\"\nchannel = \"C0999\"\non = \"message.posted\"\nworkflow = \"implement\"\n",
    )
    .unwrap();
    let stored: herdr_bus::Timestamp = setup.rows("waits")[0][5].parse().unwrap();
    // The fake answers no `conversations.history`, so the trigger's channel fails.
    setup.answer_slack(&[slack_message(&ts(stored), "U0PERSON")]);

    let output = setup.poll(&[]);

    assert_eq!(output.status.code(), Some(1));
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    assert!(setup.typed()[1].starts_with(&format!("slack.thread.replied C0123:{THREAD} {{")));
}

#[test]
fn the_intake_stops_polling_the_thread_of_a_cancelled_tasks_wait() {
    let setup = asleep_on(
        "slack-cancelled",
        json!({ "type": "slack_reply", "channel": "C0123", "thread_ts": THREAD }),
    );
    assert!(setup.priory(&["cancel", "1"]).status.success());
    setup.answer_slack(&[]);

    setup.intake();

    let args = fs::read_to_string(setup.dir.join("curl-args")).unwrap_or_default();
    assert!(!args.contains("conversations.replies"), "{args}");
}

#[test]
fn a_rate_limited_poll_starts_with_a_different_thread_each_minute() {
    let mut setup = Setup::new("slack-starve");
    let other = "1763500009.000100";
    setup.calls(&[
        wait(json!({ "type": "slack_reply", "channel": "C0123", "thread_ts": THREAD })),
        wait(json!({ "type": "slack_reply", "channel": "C0123", "thread_ts": other })),
    ]);
    setup.publish(&requested(42));
    setup.start();
    // One run at a time, so each takes its own call from the fake's queue.
    setup.until("runs", |rows| {
        rows.first().is_some_and(|row| row[7] == "asleep")
    });
    setup.publish(&requested(43));
    setup.until("runs", |rows| {
        rows.len() == 2 && rows.iter().all(|row| row[7] == "asleep")
    });
    let later = |seconds| {
        ts(herdr_bus::Timestamp::now()
            .checked_add(Duration::from_secs(seconds))
            .unwrap())
    };
    let (first, second) = (later(3600), later(3601));
    let bin = setup.dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let auth = json!({ "ok": true, "user_id": "U0BOT", "bot_id": "B0BOT" });
    // Slack lets a new app read one thread a minute: every later call in a poll is refused.
    let script = format!(
        "#!/bin/sh\ncat > /dev/null\ncase \"$*\" in\n  *auth.test*) echo '{auth}' ;;\n  *conversations.replies*)\n    \
         if [ -e '{dir}/called' ]; then echo '{{\"ok\":false,\"error\":\"ratelimited\"}}'; exit 0; fi\n    \
         touch '{dir}/called'\n    thread=$(echo \"$*\" | sed 's/.*ts=\\([0-9.]*\\).*oldest.*/\\1/')\n    \
         reply={first}; [ \"$thread\" = {other} ] && reply={second}\n    \
         echo \"{{\\\"ok\\\":true,\\\"has_more\\\":false,\\\"messages\\\":[{{\\\"ts\\\":\\\"$reply\\\",\\\"thread_ts\\\":\\\"$thread\\\",\\\"user\\\":\\\"U0PERSON\\\",\\\"text\\\":\\\"done\\\"}}]}}\" ;;\nesac\n",
        dir = setup.dir.display(),
    );
    fs::write(bin.join("curl"), script).unwrap();
    fs::set_permissions(bin.join("curl"), fs::Permissions::from_mode(0o755)).unwrap();

    for now in ["2026-11-19T10:00:00Z", "2026-11-19T10:01:00Z"] {
        let _ = fs::remove_file(setup.dir.join("called"));
        setup.intake_with(&[("PRIORY_NOW", now)]);
    }

    let replied: Vec<String> = setup
        .bus()
        .messages(&herdr_bus::MessageQuery {
            output: Some(OUTPUT.into()),
            limit: 100,
            ..herdr_bus::MessageQuery::default()
        })
        .unwrap()
        .into_iter()
        .filter(|stored| stored.message.kind == "slack.thread.replied")
        .inspect(|stored| conform::conforms(&serde_json::to_value(&stored.message).unwrap()))
        .map(|stored| {
            stored.message.data["thread_ts"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert!(
        replied.contains(&THREAD.to_owned()) && replied.contains(&other.to_owned()),
        "{replied:?}"
    );
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

#[test]
fn a_pull_request_wait_makes_an_intake_without_github_triggers_poll_its_repository() {
    let setup = asleep_on("review-intake", on_pull_7("review"));
    let log = fake_gh(&setup);

    setup.intake();

    let calls = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        calls.contains("api repos/owner/repo/issues/events"),
        "{calls}"
    );
}

#[test]
fn a_merge_hold_makes_the_intake_poll_its_pull_requests_repository() {
    let setup = queued_without_room("hold-merge-intake");
    person(
        &setup,
        "hold",
        &["1", "--reason", "needs it", "--merge", "other/repo#7"],
    );
    let log = fake_gh(&setup);

    setup.intake();

    let calls = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        calls.contains("api repos/other/repo/issues/events"),
        "{calls}"
    );
}

/// Installs a `gh` that answers `[]` and logs its arguments to the returned file.
fn fake_gh(setup: &Setup) -> std::path::PathBuf {
    let bin = setup.dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = setup.dir.join("gh-args");
    fs::write(
        bin.join("gh"),
        format!("#!/bin/sh\necho \"$*\" >> '{}'\necho '[]'\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
    log
}

#[test]
fn a_restarted_intake_resumes_a_wait_only_repository_where_it_stopped() {
    let setup = asleep_on("review-resume", on_pull_7("review"));
    let bin = setup.dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let events = setup.dir.join("events.json");
    fs::write(&events, "[]").unwrap();
    fs::write(
        bin.join("gh"),
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *issues/events*) cat '{}' ;;\n  *) echo '[]' ;;\nesac\n",
            events.display()
        ),
    )
    .unwrap();
    fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
    setup.intake();
    let event = json!([{
        "id": 100, "event": "labeled", "created_at": "2026-11-19T10:00:00Z",
        "actor": { "login": "max" }, "label": { "name": "bug" },
        "issue": { "id": 9041, "number": 41, "title": "Fix it", "html_url": "https://github.com/owner/repo/issues/41",
                   "user": { "login": "max" }, "created_at": "2026-11-19T09:00:00Z" }
    }]);
    fs::write(&events, event.to_string()).unwrap();

    setup.intake();

    let labeled: Vec<_> = setup
        .bus()
        .messages(&herdr_bus::MessageQuery {
            output: Some(OUTPUT.into()),
            limit: 100,
            ..herdr_bus::MessageQuery::default()
        })
        .unwrap()
        .into_iter()
        .filter(|stored| stored.message.kind == "github.issues.labeled")
        .map(|stored| stored.message.data["baseline"].clone())
        .collect();
    assert_eq!(labeled, [json!(false)]);
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

#[test]
fn an_acquire_without_gb_reserves_10_gb() {
    let mut setup = Setup::new("gate-default");
    setup.start();
    listening(&setup);
    setup.hold("hold-a");

    let mut command = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["memory", "acquire", "--", "sh", "-c", &held("a")])
        .current_dir(setup.dir.join("fake"))
        .spawn()
        .unwrap();

    let rows = setup.until("memory", |rows| rows.len() == 1 && rows[0][2] == "held");
    assert_eq!(rows[0][3], "10.0");
    setup.release("hold-a");
    assert!(command.wait().unwrap().success());
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

#[test]
fn a_grant_inside_a_run_does_not_wait_for_that_run_own_promise() {
    let mut setup = Setup::new("gate-own-run");
    setup.memory(20);
    setup.hold("hold-stop");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    let runs = setup.until("runs", |rows| rows.len() == 1);
    let session = runs[0][4].clone();

    // 20 GB free keeps 6.4 GB: 12 GB fits only without the run's own 4 GB promise.
    let gated = |session: Option<&str>| {
        let mut command = setup.command(env!("CARGO_BIN_EXE_priory"));
        if let Some(session) = session {
            command.env("PRIORY_SESSION", session);
        }
        command
            .args([
                "memory",
                "acquire",
                "--gb",
                "12",
                "--timeout",
                "2s",
                "--",
                "true",
            ])
            .output()
            .unwrap()
    };
    let inside = gated(Some(&session));
    let outside = gated(None);
    setup.release("hold-stop");

    assert!(
        inside.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&inside.stderr)
    );
    assert_eq!(outside.status.code(), Some(125));
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

/// A factory whose runs count as stalled after 2 seconds without a hook event or a pane change, and
/// whose handler is the implementer agent, on `haiku`.
fn stalls(name: &str, workflow: &str) -> Setup {
    let setup = Setup::with_workflow(name, workflow);
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\nhandler = \"implementer\"\nstalled = \"2s\"\n",
    )
    .unwrap();
    setup
}

impl Setup {
    /// What the handler was asked, one prompt per call.
    fn handled(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join("fake/handler.log"))
            .unwrap_or_default()
            .split_terminator("---\n")
            .map(str::to_owned)
            .collect()
    }

    /// Queues the handler's answers, one per call.
    fn answers(&self, answers: &[serde_json::Value]) {
        let lines: String = answers.iter().map(|answer| format!("{answer}\n")).collect();
        fs::write(self.dir.join("fake/handler"), lines).unwrap();
    }

    /// The `factory.handler.called` records of the trace.
    fn handler_records(&self) -> Vec<serde_json::Value> {
        self.trace()
            .into_iter()
            .filter(|record| record["type"] == "factory.handler.called")
            .collect()
    }
}

#[test]
fn a_stalled_run_gets_a_handler_call_whose_nudge_reaches_its_pane_and_whose_cost_is_reported() {
    let mut setup = stalls("handler-nudge", WORKFLOW);
    setup.answers(&[json!({ "action": "nudge", "text": "Run the tests, then call result." })]);
    setup.publish(&requested(42));

    setup.start();

    // The silent turn, its Stop nudge, then the handler's nudge.
    setup.within(20, "outbox", |rows| with(rows, 3, "confirmed") == 3);
    let typed = setup.typed();
    assert!(
        typed[2].contains("Run the tests, then call result."),
        "{typed:?}"
    );
    let asked = setup.handled();
    assert_eq!(asked.len(), 1, "{asked:?}");
    for part in ["Implement the issue.", "fake turn 2 ended", "\"action\""] {
        assert!(asked[0].contains(part), "{part} not in {}", asked[0]);
    }
    let args = fs::read_to_string(setup.dir.join("fake/handler-args.log")).unwrap();
    assert!(args.contains("haiku"), "{args}");
    let records = setup.handler_records();
    assert_eq!(records.len(), 1, "{records:?}");
    let data = &records[0]["data"];
    assert_eq!(
        [&data["task"], &data["run"], &data["action"], &data["model"]],
        ["1", "1", "nudge", "claude-fake-haiku"]
    );
    fs::write(
        setup.dir.join("factory/prices.toml"),
        "[claude-fake-haiku]\ninput = 1.0\noutput = 5.0\ncache_read = 0.1\ncache_write = 1.25\n",
    )
    .unwrap();
    let report = setup.report(&["--workflow", "implement", "--cost"]);
    assert!(
        report.contains(
            &[
                "1",
                "handler",
                "claude-fake-haiku",
                "300",
                "20",
                "0",
                "40",
                "$0.000450"
            ]
            .map(str::to_owned)
            .to_vec()
        ),
        "{report:?}"
    );
}

#[test]
fn the_handler_reads_the_agent_pane_when_a_person_opened_another_pane() {
    let mut setup = stalls("handler-pane", WORKFLOW);
    setup.answers(&[json!({ "action": "nudge", "text": "Run the tests, then call result." })]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    // The new pane is active and changes all the time, while the agent's pane stays still.
    let ticker = "while :; do echo tick-$(date +%s%N); sleep 0.2; done";
    assert!(
        setup
            .tmux(&["split-window", "-t", "=priory-1:", "--", "sh", "-c", ticker])
            .status
            .success()
    );

    setup.written("fake/handler.log");
    let asked = setup.handled();
    assert!(asked[0].contains("fake turn"), "{asked:?}");
    assert!(!asked[0].contains("tick-"), "{asked:?}");
}

#[test]
fn a_restart_answer_ends_the_stalled_run_and_resumes_its_session_in_a_new_run() {
    let mut setup = stalls("handler-restart", WORKFLOW);
    setup.answers(&[json!({ "action": "restart", "why": "its input looks wedged" })]);
    setup.publish(&requested(42));

    setup.start();

    let runs = setup.within(20, "runs", |rows| rows.len() == 2);
    assert_eq!(runs[0][7], "restarted");
    assert_eq!(runs[1][4], runs[0][4]);
    // The new run's silent turn earns a Stop nudge, which may already be confirmed too.
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert!(has(&launched(&setup), ["--resume", &runs[0][4]]));
    let typed = setup.typed();
    assert!(
        typed[2].starts_with(
            "Priory's handler restarted this run after it stalled: its input looks wedged."
        ),
        "{typed:?}"
    );
    assert_eq!(setup.handler_records()[0]["data"]["action"], "restart");
}

#[test]
fn a_run_asleep_on_a_wait_never_gets_a_handler_call() {
    let mut setup = stalls("handler-asleep", WORKFLOW);
    setup.calls(&[wait(json!({ "type": "time", "until": "1 hour" }))]);
    // The run stays live on its open wait until its Stop.
    setup.hold("hold-stop");
    setup.publish(&requested(42));

    setup.start();

    setup.until("waits", |rows| rows.len() == 1);
    std::thread::sleep(Duration::from_secs(4));
    setup.release("hold-stop");
    setup.until("tasks", |rows| rows[0][4] == "asleep");
    std::thread::sleep(Duration::from_secs(3));
    assert!(setup.handled().is_empty(), "{:?}", setup.handled());
    assert!(setup.handler_records().is_empty());
}

#[test]
fn a_run_gets_at_most_three_handler_calls() {
    let mut setup = stalls("handler-cap", WORKFLOW);
    let nudge = json!({ "action": "nudge", "text": "Carry on." });
    setup.answers(&[nudge.clone(), nudge.clone(), nudge.clone(), nudge]);
    setup.publish(&requested(42));

    setup.start();

    // The silent turn, its Stop nudge, then one prompt per handler call.
    setup.within(30, "outbox", |rows| with(rows, 3, "confirmed") == 5);
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(setup.handled().len(), 3);
    assert_eq!(setup.handler_records().len(), 3);
    assert_eq!(setup.rows("outbox").len(), 5);
}

#[test]
fn a_left_stall_gets_no_second_call_even_after_a_daemon_restart() {
    let mut setup = stalls("handler-leave", WORKFLOW);
    setup.answers(&[json!({ "action": "leave", "why": "it reads a long log" })]);
    setup.publish(&requested(42));

    setup.start();

    setup.within(20, "outbox", |rows| with(rows, 3, "confirmed") == 2);
    let deadline = Instant::now() + Duration::from_secs(15);
    while setup.handler_records().is_empty() {
        assert!(Instant::now() < deadline, "no handler call");
        std::thread::sleep(Duration::from_millis(100));
    }
    setup.crash();
    setup.start();
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(setup.handled().len(), 1);
    let records = setup.handler_records();
    assert_eq!(
        [&records[0]["data"]["action"], &records[0]["data"]["text"]],
        ["leave", "it reads a long log"]
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

#[test]
fn an_approval_answered_while_retired_opens_the_pull_request_once_disk_allows() {
    let mut setup = asked_to_open("retire-approval", OPEN_PR);
    let commit = setup.commit(1, "widget.txt");
    setup.crash();
    setup.disk(10);
    setup.start();
    setup.removed(1);

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("tasks", |rows| {
        rows[0][4] == "ready" && rows[0][7] == "waits for room to add back its worktree"
    });
    assert!(setup.rows("actions").is_empty());
    setup.disk(100);
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4..7], ["opened", "github:owner/repo#7", &commit]);
}

#[test]
fn a_handler_call_that_fails_never_acts_but_its_cost_is_traced() {
    let mut setup = stalls("handler-fail", WORKFLOW);
    setup.answers(&[json!({ "action": "restart", "why": "wedged" })]);
    setup.hold("handler-fail");
    setup.publish(&requested(42));

    setup.start();

    let deadline = Instant::now() + Duration::from_secs(20);
    while setup.handler_records().is_empty() {
        assert!(Instant::now() < deadline, "no handler call");
        std::thread::sleep(Duration::from_millis(100));
    }
    let data = &setup.handler_records()[0]["data"];
    assert_eq!(data["action"], "leave");
    assert_eq!(data["text"], "the handler call failed: exit 1");
    assert_eq!(data["tokens"]["input"], 300);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(setup.rows("runs").len(), 1);
}

#[test]
fn a_factory_without_a_handler_never_calls_one() {
    let mut setup = Setup::new("handler-unset");
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\nstalled = \"2s\"\n",
    )
    .unwrap();
    setup.publish(&requested(42));

    setup.start();

    setup.within(20, "outbox", |rows| with(rows, 3, "confirmed") == 2);
    std::thread::sleep(Duration::from_secs(5));
    assert!(setup.handled().is_empty(), "{:?}", setup.handled());
    assert!(setup.handler_records().is_empty());
}

#[test]
fn the_tldr_tool_refuses_a_tldr_over_the_limit_with_its_count_then_stores_one_the_search_tool_finds()
 {
    let mut setup = Setup::new("tldr-tool");
    let long = vec!["gate"; 181].join(" ");
    setup.calls(&[
        json!({ "name": "tldr", "arguments": { "text": long } }),
        json!({ "name": "tldr", "arguments": { "text": "Rework the done gate so it ends on a close." } }),
    ]);
    setup.publish(&requested(42));

    setup.start();

    setup.calls_left(0);
    let deadline = Instant::now() + Duration::from_secs(15);
    while setup.replies().len() < 2 {
        assert!(Instant::now() < deadline, "{:?}", setup.replies());
        std::thread::sleep(Duration::from_millis(50));
    }
    let (refused, error) = reply(&setup.replies()[0]);
    assert!(
        error && refused.contains("181 words") && refused.contains("180"),
        "{refused}"
    );
    let (stored, error) = reply(&setup.replies()[1]);
    assert!(!error, "{stored}");
    let session = setup.rows("runs")[0][4].clone();
    let replies = setup.mcp(
        &session,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search", "arguments": {"query": "who touches the done gate?"}}})],
    );
    let (found, error) = reply(&replies[0]);
    assert!(!error, "{found}");
    let rows: Vec<&str> = found.lines().skip(1).collect();
    assert_eq!(rows.len(), 1, "{found}");
    assert!(
        rows[0].starts_with("1\t")
            && rows[0].ends_with("Rework the done gate so it ends on a close."),
        "{found}"
    );
}

#[test]
fn the_progress_tool_stores_the_four_fields_on_the_agents_task() {
    let mut setup = Setup::new("progress-tool");
    let progress = json!({
        "goal": "Rework the done gate",
        "acceptanceCriteria": [{ "id": "AC1", "description": "a close ends it", "required": true }],
        "latestChange": "Moved the gate to the close event",
        "nextAction": "Run the tests"
    });
    setup.calls(&[json!({ "name": "progress", "arguments": progress })]);
    setup.publish(&requested(42));

    setup.start();

    setup.calls_left(0);
    let deadline = Instant::now() + Duration::from_secs(15);
    while setup.replies().is_empty() {
        assert!(Instant::now() < deadline, "no reply");
        std::thread::sleep(Duration::from_millis(50));
    }
    let (stored, error) = reply(&setup.replies()[0]);
    assert!(!error, "{stored}");
    let tasks: serde_json::Value =
        serde_json::from_slice(&setup.priory(&["tasks", "--json"]).stdout).unwrap();
    assert_eq!(tasks["tasks"][0]["progress"], progress);
}
impl Setup {
    /// Turns the OTLP exporter on in `factory.toml`, sending to `endpoint`.
    fn telemetry(&self, endpoint: &str) {
        let factory = format!("name = \"demo\"\n\n[telemetry]\notlp = \"{endpoint}\"\n");
        fs::write(self.dir.join("factory/factory.toml"), factory).unwrap();
    }

    /// The OTLP requests the fake `curl` received, each `{url, body}`, oldest first.
    fn posts(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(self.dir.join("fake/otlp.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Waits until the requests hold `count` spans, and returns them.
    fn until_spans(&self, count: usize) -> Vec<serde_json::Value> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let spans = spans(&self.posts());
            if spans.len() >= count {
                return spans;
            }
            assert!(
                Instant::now() < deadline,
                "only {} spans: {spans:?}\ndaemon: {}",
                spans.len(),
                fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Every span in `posts`, in the order they were sent.
fn spans(posts: &[serde_json::Value]) -> Vec<serde_json::Value> {
    posts
        .iter()
        .flat_map(|post| post["body"]["resourceSpans"].as_array().unwrap().clone())
        .flat_map(|resource| resource["scopeSpans"].as_array().unwrap().clone())
        .flat_map(|scope| scope["spans"].as_array().unwrap().clone())
        .collect()
}

/// The value of attribute `key` in an OTLP attribute list, as JSON.
fn attribute(attributes: &serde_json::Value, key: &str) -> serde_json::Value {
    attributes
        .as_array()
        .unwrap()
        .iter()
        .find(|attribute| attribute["key"] == key)
        .map_or(serde_json::Value::Null, |attribute| {
            let value = &attribute["value"];
            value
                .get("stringValue")
                .cloned()
                .or_else(|| {
                    value
                        .get("intValue")
                        .and_then(|int| int.as_str())
                        .map(|int| json!(int.parse::<i64>().unwrap()))
                })
                .unwrap_or_else(|| value.clone())
        })
}

fn is_hex(id: &serde_json::Value, digits: usize) -> bool {
    id.as_str()
        .is_some_and(|id| id.len() == digits && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[test]
fn a_finished_task_sends_one_trace_with_a_span_per_run_and_a_span_per_step_under_its_run() {
    let mut setup = Setup::with_workflow("otlp", STEPS);
    setup.telemetry("http://127.0.0.1:4318");
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });

    let spans = setup.until_spans(4);

    let posts = setup.posts();
    for post in &posts {
        assert_eq!(post["url"], "http://127.0.0.1:4318/v1/traces");
        let resource = &post["body"]["resourceSpans"][0]["resource"]["attributes"];
        assert_eq!(attribute(resource, "service.name"), "priory");
        assert_eq!(attribute(resource, "host.name"), "runner");
    }
    assert_eq!(spans.len(), 4, "{spans:?}");
    let trace = &spans[0]["traceId"];
    assert!(is_hex(trace, 32), "{trace}");
    assert!(spans.iter().all(|span| &span["traceId"] == trace));
    let ids: std::collections::HashSet<_> =
        spans.iter().map(|span| span["spanId"].clone()).collect();
    assert_eq!(ids.len(), 4);
    assert!(ids.iter().all(|id| is_hex(id, 16)));
    let records = setup.trace();
    assert_eq!(records.len(), 2);
    for record in &records {
        let data = &record["data"];
        let step = spans
            .iter()
            .find(|span| span["name"] == data["step"])
            .unwrap_or_else(|| panic!("no span for step {}: {spans:?}", data["step"]));
        let run = spans
            .iter()
            .find(|span| {
                span["name"] == "invoke_agent implementer"
                    && attribute(&span["attributes"], "priory.run.id") == data["run"]
            })
            .unwrap_or_else(|| panic!("no span for run {}: {spans:?}", data["run"]));
        assert_eq!(step["parentSpanId"], run["spanId"]);
        assert!(
            run.get("parentSpanId").is_none_or(|parent| parent == ""),
            "{run}"
        );
        let attributes = &step["attributes"];
        assert_eq!(attribute(attributes, "priory.task.id"), "1");
        assert_eq!(attribute(attributes, "priory.run.id"), data["run"]);
        assert_eq!(attribute(attributes, "priory.step"), data["step"]);
        assert_eq!(attribute(attributes, "priory.outcome"), data["outcome"]);
        assert_eq!(attribute(attributes, "priory.lane"), "none/misc");
        assert_eq!(attribute(attributes, "gen_ai.request.model"), data["model"]);
        let tokens = &data["tokens"];
        let input = ["input", "cache_read", "cache_write"]
            .map(|kind| tokens[kind].as_i64().unwrap())
            .iter()
            .sum::<i64>();
        assert_eq!(attribute(attributes, "gen_ai.usage.input_tokens"), input);
        assert_eq!(
            attribute(attributes, "gen_ai.usage.cache_read.input_tokens"),
            tokens["cache_read"]
        );
        assert_eq!(
            attribute(attributes, "gen_ai.usage.output_tokens"),
            data["tokens"]["output"]
        );
        assert_eq!(
            attribute(&run["attributes"], "gen_ai.agent.name"),
            "implementer"
        );
        assert_eq!(attribute(&run["attributes"], "priory.task.id"), "1");
        let start: u64 = step["startTimeUnixNano"].as_str().unwrap().parse().unwrap();
        let end: u64 = step["endTimeUnixNano"].as_str().unwrap().parse().unwrap();
        assert_eq!(
            (end - start) / 1_000_000,
            data["duration_ms"].as_u64().unwrap()
        );
    }
}

#[test]
fn without_telemetry_nothing_is_sent_and_runs_get_no_otel_resource_attributes() {
    let mut setup = Setup::with_workflow("otlp-off", STEPS);
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    std::thread::sleep(Duration::from_secs(2));

    assert!(setup.posts().is_empty(), "{:?}", setup.posts());
    assert_eq!(
        fs::read_to_string(setup.dir.join("fake/otel.log")).unwrap(),
        ""
    );
}

#[test]
fn a_run_gets_its_task_and_run_ids_in_otel_resource_attributes_after_the_daemons_own() {
    let mut setup = Setup::new("otlp-resource");
    setup.telemetry("http://127.0.0.1:4318");
    setup.publish(&requested(42));
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    setup.start_with(exe, &[("OTEL_RESOURCE_ATTRIBUTES", "team=core")]);

    setup.written("fake/otel.log");

    assert_eq!(
        fs::read_to_string(setup.dir.join("fake/otel.log")).unwrap(),
        "OTEL_RESOURCE_ATTRIBUTES=team=core,priory.task.id=1,priory.run.id=1\n"
    );
}

/// A task run to `done` through two model steps, its spans sent to `endpoint`.
fn exported(name: &str, endpoint: &str) -> (Setup, Vec<serde_json::Value>) {
    let mut setup = Setup::with_workflow(name, STEPS);
    setup.telemetry(endpoint);
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    let spans = setup.until_spans(4);
    (setup, spans)
}

#[test]
fn a_restarted_daemon_resends_no_span() {
    let (mut setup, _) = exported("otlp-restart", "http://127.0.0.1:4318");
    // The cursor moves on the tick after curl ends; a crash before it resends, with the same ids.
    std::thread::sleep(Duration::from_millis(1500));
    let posts = setup.posts().len();

    setup.crash();
    setup.start();
    setup.until("tasks", |rows| rows.len() == 1);
    std::thread::sleep(Duration::from_secs(3));

    assert_eq!(setup.posts().len(), posts, "{:?}", setup.posts());
    assert_eq!(spans(&setup.posts()).len(), 4);
}

#[test]
fn a_new_endpoint_gets_every_record_with_the_same_trace_and_span_ids() {
    let (mut setup, first) = exported("otlp-ids", "http://127.0.0.1:4318");
    setup.crash();
    setup.telemetry("http://127.0.0.1:4319/");
    setup.start();

    let all = setup.until_spans(8);

    let ids = |spans: &[serde_json::Value]| {
        let mut ids: Vec<_> = spans
            .iter()
            .map(|span| {
                [&span["traceId"], &span["spanId"], &span["parentSpanId"]].map(|id| id.to_string())
            })
            .collect();
        ids.sort();
        ids
    };
    let resent: Vec<_> = setup
        .posts()
        .into_iter()
        .filter(|post| post["url"] == "http://127.0.0.1:4319/v1/traces")
        .collect();
    assert_eq!(ids(&spans(&resent)), ids(&first));
    assert_eq!(all.len(), 8);
}

#[test]
fn a_failed_send_is_retried_and_its_spans_arrive_once() {
    let mut setup = Setup::with_workflow("otlp-retry", STEPS);
    setup.telemetry("http://127.0.0.1:4318");
    setup.hold("curl-fail");
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });

    setup.until_spans(4);

    assert!(!setup.dir.join("fake/curl-fail").exists());
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert!(
        log.contains("sending spans to http://127.0.0.1:4318 failed"),
        "{log}"
    );
    assert!(log.contains("curl: (7)"), "{log}");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(spans(&setup.posts()).len(), 4);
}

#[test]
fn a_batch_the_endpoint_rejects_as_malformed_is_skipped_rather_than_retried() {
    let mut setup = Setup::with_workflow("otlp-rejected", STEPS);
    setup.telemetry("http://127.0.0.1:4318");
    fs::write(setup.dir.join("fake/curl-status"), "400").unwrap();
    setup.calls(&[result("ready"), result("passed")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "done")
    });
    std::thread::sleep(Duration::from_secs(7));

    let calls = fs::read_to_string(setup.dir.join("fake/curl.log")).unwrap();
    assert_eq!(calls.lines().count(), setup.posts().len() + 1, "{calls}");
    // A retry would deliver the rejected spans, making all four arrive.
    assert!(spans(&setup.posts()).len() < 4, "{:?}", setup.posts());
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert_eq!(
        log.matches("rejected spans with HTTP 400").count(),
        1,
        "{log}"
    );
}

/// The socket path of the test's tmux server, as tmux names it.
fn socket(setup: &Setup) -> String {
    let out = setup.tmux(&["display-message", "-p", "#{socket_path}"]);
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn attach_print_names_the_live_runs_tmux_socket_and_session_for_a_task_or_a_run() {
    let mut setup = Setup::new("attach-print");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    let by_task = setup.priory(&["attach", "1", "--print"]);
    let by_run = setup.priory(&["attach", "--run", "1", "--print"]);

    for printed in [by_task, by_run] {
        assert!(printed.status.success(), "{printed:?}");
        assert_eq!(
            String::from_utf8(printed.stdout).unwrap(),
            format!("tmux -S {} attach-session -t '=priory-1'\n", socket(&setup))
        );
    }
}

#[test]
fn attach_read_only_adds_tmuxs_read_only_flag() {
    let mut setup = Setup::new("attach-read-only");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    let printed = setup.priory(&["attach", "1", "--read-only", "--print"]);

    assert!(printed.status.success(), "{printed:?}");
    assert_eq!(
        String::from_utf8(printed.stdout).unwrap(),
        format!(
            "tmux -S {} attach-session -r -t '=priory-1'\n",
            socket(&setup)
        )
    );
}

#[test]
fn attach_refuses_an_ended_run_a_task_with_no_live_run_and_an_unknown_id() {
    let mut setup = Setup::new("attach-ended");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    assert!(setup.priory(&["cancel", "1"]).status.success());

    for (args, error) in [
        (&["attach", "1"][..], "task 1 is dropped, with no live run"),
        (
            &["attach", "--run", "1", "--print"],
            "run 1 ended as cancelled",
        ),
        (&["attach", "9", "--print"], "no task 9"),
        (&["attach", "--run", "9"], "no run 9"),
    ] {
        let refused = setup.priory(args);

        assert_eq!(refused.status.code(), Some(1), "{args:?}: {refused:?}");
        assert_eq!(
            String::from_utf8_lossy(&refused.stderr),
            format!("priory attach: {error}\n")
        );
    }
}

#[test]
fn attach_refuses_a_same_named_session_on_another_tmux_server_that_runs_no_priory_agent() {
    let mut setup = Setup::new("attach-other-server");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);
    let other = TmuxDir::new(&setup.dir.join("other-tmux"));
    let started = setup
        .command("tmux")
        .env("TMUX_TMPDIR", other.path())
        .args(["new-session", "-d", "-s", "priory-1"])
        .status()
        .unwrap();
    assert!(started.success());

    let refused = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .env("TMUX_TMPDIR", other.path())
        .args(["attach", "1", "--print"])
        .output()
        .unwrap();

    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "priory attach: tmux session priory-1 on this tmux server runs no live pane of run 1's agent; \
         run attach with the daemon's TMUX and TMUX_TMPDIR\n"
    );
}

const REVIEW_TRIGGER: &str = r#"
source = "github"
on = "pull_request.review_requested"
reviewer = "max"
workflow = "implement"
"#;

const REVIEW_KEY: &str = "github:owner/repo:pulls/7:review:max";

fn reviews(name: &str) -> Setup {
    let setup = Setup::new(name);
    fs::write(
        setup.dir.join("factory/triggers/review.toml"),
        REVIEW_TRIGGER,
    )
    .unwrap();
    setup
}

/// A review request event for pull request 7, as the intake publishes it at the `round`th poll.
fn review_event(kind: &str, round: u32) -> Message {
    Message::new(
        kind,
        json!({
            "repo": "owner/repo",
            "pull_request": 7,
            "title": "Tidy the parser",
            "url": "https://github.com/owner/repo/pull/7",
            "author": "alice",
            "since": "2026-11-20T09:00:00Z",
            "reviewer": "max",
            "reviewed": false,
            "time": format!("2026-11-20T10:0{round}:00Z"),
            "baseline": false,
            "triggers": ["review"]
        }),
    )
    .subject("owner/repo#7")
}

/// A review request event and the intake's task request for it.
fn review_request(round: u32) -> [Message; 2] {
    let source = review_event("github.pull_request.review_requested", round);
    let request = Message::new(
        "factory.task.requested",
        json!({
            "workflow": "implement",
            "key": REVIEW_KEY,
            "band": null,
            "lane": null,
            "trigger": "review",
            "source": source.id.to_string(),
            "repo": "owner/repo"
        }),
    )
    .subject(REVIEW_KEY);
    [source, request]
}

impl Setup {
    fn publish_all(&self, messages: &[Message]) {
        messages.iter().for_each(|message| self.publish(message));
    }
}

#[test]
fn a_review_re_request_wakes_its_asleep_task() {
    let mut setup = reviews("review-wakes");
    setup.calls(&[ask()]);
    setup.publish_all(&review_request(1));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    setup.publish_all(&review_request(2));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 2);
    let inbox = setup.rows("inbox");
    assert_eq!([&inbox[2][4], &inbox[3][4]], ["kept", "woke"]);
    assert_eq!(setup.rows("tasks").len(), 1);
}

#[test]
fn a_removed_review_request_drops_its_running_task_and_ends_its_run() {
    let mut setup = reviews("review-withdrawn");
    setup.hold("hold-stop");
    setup.publish_all(&review_request(1));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 1);

    setup.publish(&review_event(
        "github.pull_request.review_request_removed",
        2,
    ));

    let tasks = setup.until("tasks", |rows| rows[0][4] == "dropped");
    assert_eq!(tasks[0][7], "review request removed");
    assert_eq!(setup.rows("runs")[0][7], "cancelled");
    setup.gone("priory-1");
}

#[test]
fn a_review_task_finishes_without_owning_its_pull_request() {
    let mut setup = reviews("review-done");
    setup.calls(&[result("passed")]);
    setup.publish_all(&review_request(1));
    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first()
            .is_some_and(|row| row[4] != "queued" && row[4] != "running")
    });

    assert_eq!([&tasks[0][4], &tasks[0][7]], ["done", "-"]);
}

impl Setup {
    /// A setup whose factory is the starter `examples/<starter>`, as a person copies it.
    fn starter(name: &str, starter: &str) -> Setup {
        let setup = Setup::new(name);
        let factory = setup.dir.join("factory");
        fs::remove_dir_all(&factory).unwrap();
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let copied = Command::new("cp")
            .arg("-r")
            .arg(examples.join(starter))
            .arg(&factory)
            .status()
            .unwrap();
        assert!(copied.success());
        setup
    }
}

/// A task request for `workflow`, as the intake publishes it for `trigger`.
fn starter_request(workflow: &str, trigger: &str, key: &str) -> Message {
    Message::new(
        "factory.task.requested",
        json!({
            "workflow": workflow,
            "key": key,
            "band": null,
            "lane": null,
            "trigger": trigger,
            "source": "source",
            "repo": "owner/repo"
        }),
    )
    .subject(key)
}

#[test]
fn the_review_starter_writes_its_pending_review_then_asks_the_person_to_submit_it() {
    let mut setup = Setup::starter("starter-review", "review");
    setup.calls(&[result("written")]);
    let key = "github:owner/repo:pulls/7:review:your-login";
    setup.publish(&starter_request("review", "review-requests", key));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    assert_eq!(tasks[0][3], "submit");
    assert_eq!(setup.rows("runs")[0][2], "review");
    let asks = setup.rows("asks");
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert!(asks[0][3].starts_with("submit"), "{asks:?}");
}

/// A groom task whose groom step ends `grade`, and whose later turns take `then`.
fn groomed(name: &str, grade: &str, then: &[serde_json::Value]) -> Setup {
    let mut setup = Setup::starter(name, "groom");
    let groom = json!({ "name": "result", "arguments": {
        "outcome": grade, "summary": "Brief: fix the typo", "reason": "one line in the README"
    } });
    setup.calls(&[&[groom], then].concat());
    setup.publish(&starter_request(
        "groom",
        "groom-label",
        "github:owner/repo#42",
    ));
    setup.start();
    setup
}

#[test]
fn a_green_groom_goes_straight_to_implement() {
    let setup = groomed("starter-groom-green", "green", &[]);

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[3] == "implement-green")
    });

    assert_eq!(grade(&tasks), ["green", "one line in the README"]);
    assert!(setup.rows("asks").is_empty());
}

#[test]
fn a_yellow_groom_is_reviewed_on_codex_then_described_before_its_pull_request_opens() {
    let then = [result("passed"), result("clean"), result("described")];
    let setup = groomed("starter-groom-yellow", "yellow", &then);

    setup.until("actions", |rows| !rows.is_empty());

    let steps: Vec<_> = setup
        .rows("runs")
        .into_iter()
        .map(|run| run[2].clone())
        .collect();
    assert_eq!(steps, ["groom", "implement", "review", "describe"]);
    assert!(
        setup.dir.join("fake/codex-args.log").exists(),
        "review ran on codex"
    );
    assert_eq!(setup.rows("actions")[0][2], "open-pr");
}

#[test]
fn a_red_groom_stops_at_approve_plan() {
    let setup = groomed("starter-groom-red", "red", &[]);

    let tasks = setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });

    assert_eq!(tasks[0][3], "approve-plan");
    assert_eq!(grade(&tasks), ["red", "one line in the README"]);
    let asks = setup.rows("asks");
    assert!(asks[0][3].starts_with("approve-plan"), "{asks:?}");
}

const PICK: &str = "2026-11-20_sentry_fnd-2409";

/// A report with one section and one pick, as the bonjour workflow writes it.
fn bonjour(requests: serde_json::Value) -> serde_json::Value {
    let mut report = json!({
        "date": "2026-11-20",
        "since": "2026-11-19T18:00:00+01:00",
        "greeting": "Bonjour, Max",
        "sections": [
            { "id": "sentry", "title": "Sentry", "count": 1, "rows": [{
                "id": "fnd-2409", "key": "FND-2409", "text": "Release build pauses",
                "meta": null, "state": null, "owner": null, "time": null,
                "pick": { "id": PICK, "options": ["Take", "Later"], "recommended": "Take",
                          "chosen": null, "applied": null },
                "comments": []
            }] },
            { "id": "moved", "title": "Moved", "count": null, "rows": [] }
        ]
    });
    if !requests.is_null() {
        report["sections"][0]["rows"][0]["pick"]["requests"] = requests;
    }
    report
}

/// A task whose agent wrote `report` with the report tool, then slept on a wait.
fn asleep_on_a_report(name: &str, report: &serde_json::Value) -> Setup {
    let mut setup = Setup::with_workflow(name, STEPS);
    setup.calls(&[
        json!({ "name": "report", "arguments": report }),
        wait(json!({ "type": "time", "until": "2 hours" })),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first().is_some_and(|row| row[4] == "asleep")
    });
    setup
}

fn shown(setup: &Setup) -> serde_json::Value {
    let output = setup.priory(&["report", "show", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let shown: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    shown["report"].clone()
}

#[test]
fn a_report_the_report_tool_writes_reads_back_in_order() {
    let report = bonjour(serde_json::Value::Null);
    let setup = asleep_on_a_report("report-tool", &report);

    assert_eq!(
        setup.replies()[0]["result"]["content"][0]["text"],
        "task 1's report is at revision 1"
    );
    let shown = shown(&setup);
    assert_eq!([&shown["task"], &shown["revision"]], [1, 1]);
    assert_eq!(shown["sections"], report["sections"]);
}

#[test]
fn a_pick_at_the_reports_revision_records_the_choice_and_wakes_the_task_with_one_line() {
    let setup = asleep_on_a_report("report-pick", &bonjour(serde_json::Value::Null));

    let picked = setup.priory(&["report", "pick", PICK, "Take", "--expected", "1"]);

    assert!(picked.status.success(), "{picked:?}");
    assert_eq!(
        String::from_utf8_lossy(&picked.stdout),
        format!("pick {PICK} is Take at revision 2\n")
    );
    let shown = shown(&setup);
    assert_eq!(shown["revision"], 2);
    assert_eq!(shown["sections"][0]["rows"][0]["pick"]["chosen"], "Take");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") >= 3);
    assert_eq!(
        setup.typed()[2].lines().next(),
        Some(format!("The person picked Take on {PICK} in your report: apply it, then write the report again with the pick's applied filled.").as_str())
    );
}

#[test]
fn a_pick_at_a_stale_revision_is_refused_and_changes_nothing() {
    let setup = asleep_on_a_report("report-stale", &bonjour(serde_json::Value::Null));

    let stale = setup.priory(&["report", "pick", PICK, "Take", "--expected", "7"]);

    assert!(!stale.status.success());
    assert_eq!(
        String::from_utf8_lossy(&stale.stderr),
        "priory report pick: task 1's report is at revision 1, not 7\n"
    );
    let shown = shown(&setup);
    assert_eq!(shown["revision"], 1);
    assert_eq!(
        shown["sections"][0]["rows"][0]["pick"]["chosen"],
        serde_json::Value::Null
    );
    assert_eq!(setup.rows("tasks")[0][4], "asleep");
}

#[test]
fn a_pick_option_that_requests_a_task_creates_it_and_fills_applied() {
    let requests = json!({ "Take": {
        "workflow": "implement", "key": "fnd-2409", "text": "Fix the DWARF pause", "repo": "owner/repo"
    } });
    let setup = asleep_on_a_report("report-request", &bonjour(requests));

    let picked = setup.priory(&["report", "pick", PICK, "Take", "--expected", "1"]);

    assert!(picked.status.success(), "{picked:?}");
    let tasks = setup.until("tasks", |rows| rows.len() == 2);
    assert_eq!(tasks[1][1], "cli:fnd-2409");
    assert_eq!(tasks[0][4], "asleep");
    let pick = &shown(&setup)["sections"][0]["rows"][0]["pick"];
    assert_eq!(
        [&pick["chosen"], &pick["applied"]],
        ["Take", "requested task cli:fnd-2409"]
    );
}

#[test]
fn a_rewrite_that_drops_the_chosen_option_is_refused() {
    let setup = asleep_on_a_report("report-drop-chosen", &bonjour(serde_json::Value::Null));
    let picked = setup.priory(&["report", "pick", PICK, "Take", "--expected", "1"]);
    assert!(picked.status.success(), "{picked:?}");
    let mut rewrite = bonjour(serde_json::Value::Null);
    rewrite["sections"][0]["rows"][0]["pick"]["options"] = json!(["Later"]);
    rewrite["sections"][0]["rows"][0]["pick"]["recommended"] = json!("Later");
    let file = setup.dir.join("rewrite.json");
    fs::write(&file, rewrite.to_string()).unwrap();

    let put = setup.priory(&["report", "put", "1", file.to_str().unwrap()]);

    assert!(!put.status.success());
    assert_eq!(
        String::from_utf8_lossy(&put.stderr),
        format!(
            "priory report put: the report is refused: pick \"{PICK}\" was chosen as \"Take\", which its options drop\n"
        )
    );
    assert_eq!(shown(&setup)["revision"], 2);
}

#[test]
fn a_pick_on_a_task_at_a_step_no_agent_runs_is_refused_and_records_nothing() {
    let mut setup = Setup::with_workflow("report-human-step", STEPS);
    setup.calls(&[
        json!({ "name": "report", "arguments": bonjour(serde_json::Value::Null) }),
        json!({ "name": "result", "arguments": { "outcome": "unsure", "summary": "Unsure." } }),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.first()
            .is_some_and(|row| row[3] == "approve" && row[4] == "asleep")
    });

    let picked = setup.priory(&["report", "pick", PICK, "Take", "--expected", "1"]);

    assert!(!picked.status.success());
    assert_eq!(
        String::from_utf8_lossy(&picked.stderr),
        format!(
            "priory report pick: pick {PICK}: task 1 is at step approve, which no agent runs\n"
        )
    );
    let shown = shown(&setup);
    assert_eq!(shown["revision"], 1);
    assert_eq!(
        shown["sections"][0]["rows"][0]["pick"]["chosen"],
        serde_json::Value::Null
    );
}
