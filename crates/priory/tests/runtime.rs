use herdr_bus::{Bus, Message, Output};
use serde_json::json;
use std::fs;
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

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\nmodel = \"haiku\"\neffort = \"low\"\n+++\nYou implement one issue.\n";

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
        let factory = self.dir.join("factory");
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        self.daemon = Some(
            self.command(env!("CARGO_BIN_EXE_priory"))
                .args(["daemon", factory.to_str().unwrap()])
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
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
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
                "timed out on `priory {verb}`: {rows:?}\ndaemon: {}",
                fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
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

    fn hold(&self, at: &str) {
        fs::write(self.dir.join("fake").join(at), "").unwrap();
    }

    fn release(&self, at: &str) {
        fs::remove_file(self.dir.join("fake").join(at)).unwrap();
    }
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
    let key = format!("github:owner/repo#{issue}");
    Message::new(
        "factory.task.requested",
        json!({
            "workflow": "implement",
            "key": key,
            "band": null,
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
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
    ] {
        assert!(args.windows(2).any(|w| w == pair), "{pair:?} in {args:?}");
    }
    assert_eq!(setup.rows("tasks")[0][4], "running");
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

    let outbox = setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
    assert_eq!(outbox.len(), 1);
    let typed = setup.typed();
    assert_eq!(typed.len(), 1, "{typed:?}");
    let lines: Vec<_> = typed[0].lines().collect();
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0], "Implement the issue.");
    assert!(lines[1].starts_with("github.issues.labeled owner/repo#42 {"));
    assert!(lines[1].contains(r#""label":"agent""#));
    assert!(lines[2].contains(r#""label":"it's \"urgent\"; C-c Enter""#));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.typed().len(), 1);
}

#[test]
fn an_event_kept_while_the_run_is_running_is_typed_after_the_next_stop() {
    let mut setup = Setup::new("message");
    setup.publish(&requested(42));
    setup.hold("hold-stop");
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);

    setup.publish(&labeled(42));

    setup.until("outbox", |rows| rows.len() == 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("outbox")[1][3], "queued");
    assert_eq!(setup.typed().len(), 1);
    setup.release("hold-stop");
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    let typed = setup.typed();
    assert_eq!(typed.len(), 2);
    assert!(typed[1].starts_with("github.issues.labeled owner/repo#42 {"));
}

#[test]
fn a_restarted_daemon_reattaches_to_its_run_and_types_nothing_twice() {
    let mut setup = Setup::new("restart");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
    let runs = setup.rows("runs");

    setup.crash();
    setup.start();
    setup.publish(&labeled(42));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(setup.rows("runs"), runs);
    assert_eq!(setup.typed().len(), 2);
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);

    setup.crash();
    setup.release("hold-stop");
    setup.written("home/signals.spool");
    setup.publish(&labeled(42));
    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
}

#[test]
fn a_run_whose_tmux_session_dies_ends_exited_and_blocks_its_task() {
    let mut setup = Setup::new("exited");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);

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
    assert_eq!(tasks[0][7], "run 1 exited: tmux session priory-1 is gone");
}

#[test]
fn only_one_run_starts_while_one_is_running() {
    let mut setup = Setup::new("one-run");
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    setup.until("runs", |rows| rows.len() == 1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.rows("runs").len(), 1);
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][4], &tasks[1][4]], ["running", "queued"]);
}

#[test]
fn a_prompt_reaches_the_agent_pane_when_a_person_opened_another_pane() {
    let mut setup = Setup::new("pane");
    setup.publish(&requested(42));
    setup.start();
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);

    assert!(
        setup
            .tmux(&["split-window", "-t", "=priory-1:", "--", "cat"])
            .status
            .success()
    );
    setup.publish(&labeled(42));

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(setup.typed().len(), 2);
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    let typed = setup.typed();
    assert_eq!(typed[1], "Implement the plan.\n", "{typed:?}");
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);
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
    assert_eq!(setup.typed().len(), 1);
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
    let runs = setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(runs.len(), 2);
    let runs = setup.rows("runs");
    assert_eq!([&runs[1][2], &runs[1][4]], ["implement", &session]);
    let args = fs::read_to_string(setup.dir.join("fake/args.log")).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert!(
        args.windows(2).any(|w| w == ["--resume", session.as_str()]),
        "{args:?}"
    );
    assert_eq!(setup.typed()[1], "Implement the plan.\n");
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
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
    assert_eq!(setup.typed()[1], "Review the change.\n");
    let tasks = setup.rows("tasks");
    assert_eq!([&tasks[0][3], &tasks[0][4]], ["review", "running"]);
}

#[test]
fn a_workflow_coming_back_to_an_agent_resumes_that_agent_session() {
    let mut setup = Setup::with_workflow("back-to-agent", TWO_AGENTS);
    setup.calls(&[result("passed"), result("changes")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 3);
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
    assert_eq!(setup.typed()[2], "Implement the issue.\n");
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 3);
    let resumed = setup.rows("runs");
    assert_eq!([&resumed[2][2], &resumed[2][4]], ["review", &runs[1][4]]);
    assert_ne!(runs[0][4], runs[1][4]);
    let args = launched(&setup);
    assert!(has(&args, ["--resume", &runs[1][4]]), "{args:?}");
    assert!(
        has(&args, ["--append-system-prompt", "You review one change."]),
        "{args:?}"
    );
    assert_eq!(setup.typed()[2], "sqlite\n");
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(setup.typed()[1], "duckdb, actually\n");
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(setup.typed()[1], "sqlite\n");
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    let typed = setup.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
    assert!(
        typed[1].starts_with("Implement the plan.\ngithub.issues.labeled owner/repo#42 {"),
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.typed(), ["Plan the change.\n", "sqlite\n"]);
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
        "head_sha": "abc",
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
    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 1);

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
        "unknown condition \"moon\": wait on pull_request, checks, review, time or task"
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
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

        setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    std::thread::sleep(Duration::from_millis(1500));
    let typed = setup.typed();
    assert_eq!(typed.len(), 2, "{typed:?}");
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
    on["head_sha"] = json!("def");
    let setup = asleep_on("checks-head", on);

    setup.publish(&check(1, 0));
    stays_asleep(&setup);
    let mut last = check(2, 0);
    last.data["head_sha"] = json!("def");
    setup.publish(&last);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    let typed = setup.typed();
    assert!(typed[1].contains(r#""github_id":2"#), "{typed:?}");
}

#[test]
fn a_time_wait_fires_after_its_instant_and_not_before() {
    let setup = asleep_on("time", json!({ "type": "time", "until": "4 seconds" }));
    let waits = setup.rows("waits");
    assert_eq!(waits[0][3], "time");

    stays_asleep(&setup);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
    assert_eq!(
        setup.typed()[1],
        format!("wait 1 fired: {} passed\n", waits[0][4])
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
    setup.calls(&[wait(json!({ "type": "task", "task": 2 })), result("passed")]);
    setup.publish(&requested(42));
    setup.publish(&requested(43));

    setup.start();

    setup.until("tasks", |rows| rows.len() == 2 && rows[1][4] == "done");
    setup.until("outbox", |rows| {
        rows.iter().any(|row| {
            row[1] == "1" && row[3] == "confirmed" && row[4] == "wait 1 fired: task 2 is done"
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
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
/// reaches `done` at commit `abc`. Returns once that run ended.
fn gated(name: &str) -> Setup {
    let mut setup = Setup::new(name);
    setup.calls(&[wait(on_pull_7("review")), passed_at("abc")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.publish(&reviewed());
    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] != "-");
    setup.gone("priory-1");
    setup
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
fn a_red_check_at_the_result_commit_wakes_the_agent_with_the_failure() {
    let setup = gated("gate-red");
    let mut red = check(1, 1);
    red.data["name"] = json!("test");
    red.data["conclusion"] = json!("failure");
    red.data["url"] = json!("https://github.com/owner/repo/runs/1");
    setup.calls(&[passed_at("def")]);
    setup.hold("hold-submit");

    setup.publish(&red);
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    // A check of the old commit finishes while the agent fixes it.
    setup.publish(&check(3, 0));
    setup.until("inbox", |rows| rows.len() == 4);
    setup.release("hold-submit");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 3);
    let typed = setup.typed();
    let lines: Vec<_> = typed[2].lines().collect();
    assert_eq!(
        lines[0],
        "wait 2 fired: check test at abc concluded failure https://github.com/owner/repo/runs/1",
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
    green.data["head_sha"] = json!("def");
    setup.publish(&green);
    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_head_that_moved_from_the_result_commit_wakes_the_agent() {
    let setup = gated("gate-moved");
    let mut moved = on_pull("github.pull_request.labeled", json!({ "github_id": 500 }));
    moved.data["head_sha"] = json!("def");

    setup.publish(&moved);

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 3);
    let typed = setup.typed();
    let lines: Vec<_> = typed[2].lines().collect();
    assert_eq!(
        lines[0],
        "wait 2 fired: the head of github:owner/repo#7 moved to def, away from your result's commit abc",
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 2);
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
fn green_checks_kept_before_the_final_stop_mark_the_task_done_when_the_gate_is_stored() {
    let mut setup = Setup::new("gate-early");
    setup.calls(&[wait(on_pull_7("review")), passed_at("abc")]);
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
    setup.calls(&[wait(on_pull_7("review")), passed_at("abc")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.hold("hold-stop");
    setup.publish(&reviewed());
    setup.until("runs", |_| setup.replies().len() == 2);
    setup.publish(&check(1, 0));
    let mut moved = on_pull("github.pull_request.labeled", json!({ "github_id": 500 }));
    moved.data["head_sha"] = json!("def");
    setup.publish(&moved);
    setup.until("inbox", |rows| rows.len() == 4);

    setup.release("hold-stop");

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 3);
    assert!(
        setup.typed()[2].starts_with("wait 2 fired: the head of github:owner/repo#7 moved to def"),
        "{:?}",
        setup.typed()
    );
}

#[test]
fn green_checks_the_agent_already_waited_on_mark_the_task_done_at_its_result() {
    let mut setup = Setup::new("gate-seen");
    setup.calls(&[wait(on_pull_7("checks")), passed_at("abc")]);
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
    setup.calls(&[passed_at("abc")]);
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
    setup.calls(&[wait(on_pull_7("review")), passed_at("abc")]);
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
}

#[test]
fn a_green_check_counting_a_failure_wakes_the_agent_with_the_failed_check() {
    let mut setup = Setup::new("gate-failing");
    setup.calls(&[wait(on_pull_7("review")), passed_at("abc")]);
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

    setup.until("outbox", |rows| with(rows, 3, "confirmed") == 3);
    let typed = setup.typed();
    assert!(
        typed[2].starts_with("wait 2 fired: check test at abc concluded failure\n"),
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
    setup.calls(&[wait(on_pull_7("review")), passed_at("abc")]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("runs", |rows| rows.len() == 1 && rows[0][7] == "asleep");
    setup.hold("hold-submit");
    setup.publish(&reviewed());
    setup.until("outbox", |rows| with(rows, 3, "attempted") == 1);
    setup.publish(&check(1, 0));
    let mut moved = on_pull("github.pull_request.labeled", json!({ "github_id": 500 }));
    moved.data["head_sha"] = json!("def");
    setup.publish(&moved);
    setup.until("inbox", |rows| rows.len() == 4);

    setup.release("hold-submit");

    setup.until("runs", |rows| rows.len() == 2 && rows[1][7] == "asleep");
    stays_asleep_after(&setup, 2);
}
