//! Plays a scenario through a whole factory folder, from a task request to its final state, with
//! the fake `claude`, `codex` and `gh`. docs/testing-a-factory.md explains the scenario file.

mod tmux_dir;

use herdr_bus::{Bus, Message, Output};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output as Exit};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

const OUTPUT: &str = "factory.intake";
const HOST: &str = "runner";
const SETTLED: &[&str] = &["done", "dropped", "failed", "blocked"];

#[test]
fn the_groom_example_takes_a_green_issue_to_a_pull_request_whose_checks_pass() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/groom");

    let played = Play::run("ci", &examples, &examples.join("e2e/green.toml"));

    played.check();
}

#[test]
fn the_review_example_writes_a_pending_review_and_asks_to_submit_it() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/review");

    let played = Play::run("ci", &examples, &examples.join("e2e/pending.toml"));

    played.check();
}

#[test]
fn the_ship_and_watch_example_watches_a_merged_change_and_announces_it_once_approved() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/ship-and-watch");

    let played = Play::run("ci", &examples, &examples.join("e2e/merged.toml"));

    played.check();
}

#[test]
#[ignore = "plays the factory PRIORY_E2E_FACTORY names through the scenario PRIORY_E2E_SCENARIO names"]
fn a_factory_plays_the_scenario_the_environment_names() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/groom");
    let factory = std::env::var_os("PRIORY_E2E_FACTORY").map_or(examples, PathBuf::from);
    let scenario = std::env::var_os("PRIORY_E2E_SCENARIO")
        .map_or_else(|| factory.join("e2e/green.toml"), PathBuf::from);

    let played = Play::run("env", &factory, &scenario);

    println!("{}", played.report());
    played.check();
}

/// A factory copied under the test's directory, with its runners replaced by this host, a bare
/// origin per repository, the fakes first on `PATH`, and its daemon. Dropping it kills the daemon
/// and every process the test started.
struct Play {
    dir: PathBuf,
    scenario: Value,
    daemon: Option<Child>,
    tmux: TmuxDir,
}

impl Play {
    fn run(test: &str, factory: &Path, file: &Path) -> Self {
        let scenario: toml::Value = fs::read_to_string(file)
            .map_err(|error| error.to_string())
            .and_then(|text| toml::from_str(&text).map_err(|error| error.to_string()))
            .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("e2e")
            .join(test)
            .join(scenario_name(factory, file));
        let _ = fs::remove_dir_all(&dir);
        let tmux = TmuxDir::new(&dir);
        let mut play = Play {
            dir,
            scenario: serde_json::to_value(scenario).unwrap(),
            daemon: None,
            tmux,
        };
        play.copy(factory);
        play.host();
        play.fakes();
        play.origins();
        play.bus().declare(&Output::new(OUTPUT)).unwrap();
        play.queue_turns();
        // A new state file switches to WAL on its first open, which does not wait for another opener.
        play.json(&["tasks", "--json"]);
        play.start();
        play.request();
        play.drive();
        play
    }

    fn factory(&self) -> PathBuf {
        self.dir.join("factory")
    }

    /// Copies the factory and writes the scenario's `files` over it. A path starting with `~/`
    /// goes under the test's home instead.
    fn copy(&self, factory: &Path) {
        fs::create_dir_all(&self.dir).unwrap();
        let copied = Command::new("cp")
            .arg("-R")
            .arg(factory)
            .arg(self.factory())
            .status()
            .unwrap();
        assert!(copied.success(), "cp {}", factory.display());
        for (path, content) in self.table("files") {
            let (root, relative) = match path.strip_prefix("~/") {
                Some(home) => (self.dir.join("home"), home),
                None => (self.factory(), path.as_str()),
            };
            assert!(
                Path::new(relative)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
                "{path} leaves the factory copy and the test's home"
            );
            let file = root.join(relative);
            write(
                &file,
                content.as_str().expect("a file's content is a string"),
            );
        }
    }

    /// Replaces the factory's runners with one for this host, which the daemon reads from
    /// `PRIORY_PROC`, and makes room for every agent.
    fn host(&self) {
        let runners = self.factory().join("runners");
        let _ = fs::remove_dir_all(&runners);
        write(
            &runners.join(format!("{HOST}.toml")),
            "platform = { os = \"linux\" }\n",
        );
        let proc = self.dir.join("proc");
        write(&proc.join("sys/kernel/hostname"), &format!("{HOST}\n"));
        write(
            &proc.join("meminfo"),
            &format!(
                "MemTotal:       {} kB\nMemFree:        1024 kB\nMemAvailable:   {} kB\n",
                64 << 20,
                60 << 20
            ),
        );
        write(&proc.join("loadavg"), "0.00 0.00 0.00 1/100 12345\n");
    }

    /// Links each shared fake into the test's `bin`, the first directory on `PATH`, since a fake
    /// finds the real program by dropping that first entry. Its `gh` answers each call that starts
    /// with a key of the scenario's `[gh]`, the longest first, with its value, or runs a value that
    /// starts with `#!` with the call's arguments. It hands any other call to the shared fake.
    /// `priory` is this build.
    fn fakes(&self) {
        fs::create_dir_all(self.dir.join("fake")).unwrap();
        let bin = self.dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for fake in fs::read_dir(fake_dir()).unwrap().flatten() {
            if fake.file_name() != "gh" {
                std::os::unix::fs::symlink(fake.path(), bin.join(fake.file_name())).unwrap();
            }
        }
        let mut script = String::from("#!/usr/bin/env bash\ncase \"$*\" in\n");
        let mut answers = self.table("gh");
        answers.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        for (index, (prefix, answer)) in answers.iter().enumerate() {
            let file = self.dir.join(format!("gh/{index}"));
            let answer = answer.as_str().expect("a gh answer is a string");
            write(&file, answer);
            let file = shell_quote(&file.display().to_string());
            let reply = if answer.starts_with("#!") {
                format!("exec {file} \"$@\"")
            } else {
                format!("cat {file}")
            };
            script.push_str(&format!(
                "  {}*) jq -nc '$ARGS.positional' --args -- \"$@\" >> \"$FAKE_DIR/gh.log\"; {reply} ;;\n",
                shell_quote(prefix),
            ));
        }
        script.push_str(&format!(
            "  *) exec {} \"$@\" ;;\nesac\n",
            shell_quote(&fake_dir().join("gh").display().to_string())
        ));
        write(&bin.join("gh"), &script);
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_priory"), bin.join("priory")).unwrap();
    }

    /// A bare repository with one commit on `main` for each repository the triggers or the request
    /// name, which git's `insteadOf` serves for GitHub.
    fn origins(&self) {
        let work = self.dir.join("work");
        write(&work.join("README.md"), "e2e\n");
        for args in [
            &["init", "-q", "-b", "main"][..],
            &["add", "README.md"],
            &["commit", "-qm", "init"],
        ] {
            assert!(git(self, &work).args(args).status().unwrap().success());
        }
        for repo in self.repos() {
            let bare = self.dir.join(format!("origin/{repo}.git"));
            let cloned = git(self, &self.dir)
                .args(["clone", "-q", "--bare"])
                .arg(&work)
                .arg(&bare)
                .status()
                .unwrap();
            assert!(cloned.success(), "clone {repo}");
        }
    }

    fn repos(&self) -> Vec<String> {
        let mut repos: Vec<String> = fs::read_dir(self.factory().join("triggers"))
            .unwrap()
            .flatten()
            .filter_map(|entry| {
                let trigger: toml::Value =
                    toml::from_str(&fs::read_to_string(entry.path()).ok()?).ok()?;
                Some(trigger.get("repo")?.as_str()?.to_owned())
            })
            .chain(self.scenario["request"]["repo"].as_str().map(str::to_owned))
            .collect();
        repos.sort();
        repos.dedup();
        repos
    }

    /// Queues the scenario's turns for the fake agent: one line of tool calls per turn, in order.
    /// A call with `run` instead of `arguments` takes as its arguments the JSON that shell
    /// command prints, run in the factory copy, as an agent passes on what a script printed.
    fn queue_turns(&self) {
        let mut turns = String::new();
        for turn in self.list("turns") {
            let calls: Vec<Value> = turn["calls"]
                .as_array()
                .expect("a turn's calls is a list")
                .iter()
                .map(|call| self.arguments(call))
                .collect();
            turns.push_str(&format!("{}\n", Value::from(calls)));
        }
        write(&self.dir.join("fake/calls"), &turns);
    }

    fn arguments(&self, call: &Value) -> Value {
        let Some(run) = call.get("run").and_then(Value::as_str) else {
            return call.clone();
        };
        let output = self
            .command("sh")
            .args(["-c", run])
            .current_dir(self.factory())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{run}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let arguments: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("{run} printed no JSON: {error}"));
        json!({ "name": call["name"], "arguments": arguments })
    }

    fn command(&self, program: &str) -> Command {
        let path = std::env::join_paths(
            std::iter::once(self.dir.join("bin"))
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut command = Command::new(program);
        // A cleared environment keeps the user's tokens from a factory script that would use them.
        command
            .env_clear()
            .env("PATH", path)
            .env("LINEAR_API_KEY", "e2e")
            .env("HOME", self.dir.join("home"))
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("XDG_RUNTIME_DIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("priory"))
            .env("PRIORY_FACTORY", self.factory())
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

    fn json(&self, args: &[&str]) -> Value {
        let output = self.priory(args);
        assert!(
            output.status.success(),
            "priory {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn start(&mut self) {
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        let factory = self.factory();
        self.daemon = Some(
            self.command(env!("CARGO_BIN_EXE_priory"))
                .arg("daemon")
                .arg(factory)
                .stderr(log)
                .spawn()
                .unwrap(),
        );
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    /// Asks for the task as the intake would: a poll of the intake at the time `[fire]` names,
    /// which fires its schedule triggers, or else a task request with the fields of `[request]`,
    /// whose workflow and band default to its trigger's.
    fn request(&self) {
        if let Some(fire) = self.scenario.get("fire") {
            let at = fire["at"].as_str().expect("fire.at is a time");
            let output = self
                .command(env!("CARGO_BIN_EXE_priory"))
                .args(["intake", "--once"])
                .arg(self.factory())
                .env("PRIORY_NOW", at)
                .output()
                .unwrap();
            // A source this scenario does not fake fails its poll; the report shows it.
            fs::write(self.dir.join("intake.log"), &output.stderr).unwrap();
            return;
        }
        let request = &self.scenario["request"];
        let trigger = request["trigger"].as_str().expect("request.trigger");
        let file = self.factory().join(format!("triggers/{trigger}.toml"));
        let defaults: toml::Value = toml::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        let mut data = json!({
            "workflow": defaults["workflow"].as_str(),
            "band": defaults.get("band").and_then(toml::Value::as_str),
            "lane": null,
            "source": uuid::Uuid::new_v4().to_string(),
        });
        data.as_object_mut()
            .unwrap()
            .extend(request.as_object().expect("[request] is a table").clone());
        let key = request["key"].as_str().expect("request.key");
        let message = Message::new("factory.task.requested", data).subject(key);
        self.bus().publish(OUTPUT, &message).unwrap();
    }

    /// The task the scenario follows, once it exists: the first task, or the first of the
    /// scenario's `workflow` when a firing starts several.
    fn task(&self) -> Option<Value> {
        let workflow = self.scenario.get("workflow");
        self.json(&["tasks", "--json"])["tasks"]
            .as_array()?
            .iter()
            .find(|task| workflow.is_none_or(|workflow| task["workflow"] == *workflow))
            .cloned()
    }

    fn id(&self) -> String {
        self.task()
            .map_or_else(String::new, |task| task["id"].to_string())
    }

    /// Plays the scenario's events in order, each once the task sleeps at the step its `at`
    /// names, or at any step without one, until the task settles or sleeps with none left.
    fn drive(&self) {
        let mut events = self.list("events").into_iter();
        let mut next = events.next();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            assert!(
                Instant::now() < deadline,
                "timed out: {:?}\n{}",
                self.task(),
                self.report()
            );
            std::thread::sleep(Duration::from_millis(100));
            let Some(task) = self.task() else { continue };
            if SETTLED.contains(&task["state"].as_str().unwrap_or_default()) {
                assert!(
                    next.is_none(),
                    "the task settled before {next:?} played\n{}",
                    self.report()
                );
                return;
            }
            if task["state"] != "asleep" {
                continue;
            }
            let Some(event) = &next else { return };
            if event.get("at").is_some_and(|at| *at != task["step"]) {
                continue;
            }
            match event.get("answer") {
                Some(option) => self.answer(option.as_str().expect("answer is an option")),
                None => self.publish(event),
            }
            assert!(
                self.woken(&task),
                "{event} did not wake the task\n{}",
                self.report()
            );
            next = events.next();
        }
    }

    /// Whether the task leaves the sleep `asleep` saw within 10 seconds.
    fn woken(&self, asleep: &Value) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let still = self
                .task()
                .is_some_and(|task| task["state"] == "asleep" && task["since"] == asleep["since"]);
            if !still {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Publishes one scenario event, with `{head}` in its data replaced by the commit the task's
    /// branch points to in its origin.
    fn publish(&self, event: &Value) {
        let mut data = event["data"].to_string();
        if data.contains("{head}") {
            data = data.replace("{head}", &self.head());
        }
        let message = Message::new(event["type"].as_str().expect("event.type"), {
            serde_json::from_str::<Value>(&data).unwrap()
        })
        .subject(event["subject"].as_str().expect("event.subject"));
        self.bus().publish(OUTPUT, &message).unwrap();
    }

    fn head(&self) -> String {
        let task = self.json(&["task", &self.id(), "--json"]);
        let (Some(repo), Some(branch)) = (task["repo"].as_str(), task["branch"].as_str()) else {
            panic!("{{head}} needs a task with a branch: {task}");
        };
        let output = git(self, &self.dir.join(format!("origin/{repo}.git")))
            .args(["rev-parse", branch])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Answers the task's open ask with `option`.
    fn answer(&self, option: &str) {
        let task = self.task().expect("a task asks");
        let asks = self.json(&["asks", "--json"]);
        let ask = asks["asks"]
            .as_array()
            .and_then(|asks| asks.iter().find(|ask| ask["task"] == task["id"]))
            .unwrap_or_else(|| panic!("task {} has no ask: {asks}", task["id"]));
        let (id, revision) = (ask["id"].to_string(), ask["revision"].to_string());
        let output = self.priory(&["answer", "--revision", &revision, &id, option]);
        assert!(output.status.success(), "answer: {output:?}");
    }

    /// The trace's records for the task, oldest first.
    fn trace(&self) -> Vec<Value> {
        let id = self.id();
        let output = self.priory(&["trace"]);
        let mut records: Vec<Value> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|record| record["data"]["task"] == id.as_str())
            .collect();
        records.sort_by_key(|record| record["data"]["started"].as_str().map(str::to_owned));
        records
    }

    /// The steps the task finished, in order, each with its outcome.
    fn path(&self) -> Vec<String> {
        self.trace()
            .iter()
            .filter(|record| record["data"]["outcome"].is_string())
            .map(|record| {
                format!(
                    "{}:{}",
                    record["data"]["step"].as_str().unwrap_or_default(),
                    record["data"]["outcome"].as_str().unwrap_or_default()
                )
            })
            .collect()
    }

    /// The commands the guard denied the task.
    fn denied(&self) -> Vec<String> {
        self.trace()
            .iter()
            .filter(|record| record["type"] == "factory.tool.denied")
            .map(|record| record["data"].to_string())
            .collect()
    }

    /// What the scenario played and where the task ended, for a person to read.
    fn report(&self) -> String {
        let mut report = format!(
            "path: {}\ntask: {}\ndenied: {:?}\n",
            self.path().join(" -> "),
            self.task().unwrap_or(Value::Null),
            self.denied()
        );
        for verb in ["runs", "actions", "asks", "waits"] {
            let output = self.priory(&[verb]);
            report.push_str(&format!(
                "\n$ priory {verb}\n{}",
                String::from_utf8_lossy(&output.stdout)
            ));
        }
        for log in ["fake/tools.log", "fake/gh.log", "daemon.log", "intake.log"] {
            if let Ok(text) = fs::read_to_string(self.dir.join(log)) {
                report.push_str(&format!("\n--- {log}\n{text}"));
            }
        }
        report
    }

    /// Asserts each of the scenario's expectations, a path and a state at least: the steps the task finished, with their
    /// outcomes, its final step and state, the commands the guard denied it or let through, the text the fake
    /// agent's tool calls got back, and the output of `priory` commands.
    fn check(&self) {
        let expect = &self.scenario["expect"];
        let task = self.task().unwrap_or(Value::Null);
        let report = self.report();
        let path = expect
            .get("path")
            .expect("a scenario expects a path, [] for none");
        let path: Vec<String> = serde_json::from_value(path.clone()).unwrap();
        assert_eq!(self.path(), path, "{report}");
        assert!(expect.get("state").is_some(), "a scenario expects a state");
        for field in ["state", "step"] {
            if let Some(value) = expect.get(field) {
                assert_eq!(&task[field], value, "{field}\n{report}");
            }
        }
        let denied = self.denied();
        for command in strings(expect.get("denied")) {
            assert!(
                denied.iter().any(|record| record.contains(&command)),
                "no denial of {command}: {denied:?}\n{report}"
            );
        }
        for command in strings(expect.get("allowed")) {
            assert!(
                !denied.iter().any(|record| record.contains(&command)),
                "the guard denied {command}\n{report}"
            );
        }
        let tools = fs::read_to_string(self.dir.join("fake/tools.log")).unwrap_or_default();
        for text in strings(expect.get("replies")) {
            assert!(tools.contains(&text), "no reply with {text}\n{report}");
        }
        for output in expect
            .get("output")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let args = strings(output.get("args"));
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let printed = self.priory(&args);
            let printed = String::from_utf8_lossy(&printed.stdout);
            for text in strings(output.get("contains")) {
                assert!(
                    printed.contains(&text),
                    "priory {args:?} lacks {text}:\n{printed}"
                );
            }
        }
    }

    fn table(&self, key: &str) -> Vec<(String, Value)> {
        self.scenario
            .get(key)
            .and_then(Value::as_object)
            .map(|table| table.clone().into_iter().collect())
            .unwrap_or_default()
    }

    fn list(&self, key: &str) -> Vec<Value> {
        self.scenario
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
}

impl Drop for Play {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
    }
}

/// The factory's folder name and the scenario file's own name, as the test's directory.
fn scenario_name(factory: &Path, scenario: &Path) -> String {
    let name = |path: &Path| {
        path.file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    format!("{}-{}", name(factory), name(scenario))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .map(|value| serde_json::from_value(value.clone()).expect("a list of strings"))
        .unwrap_or_default()
}

fn fake_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake")
}

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
    if content.starts_with("#!") {
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Git with the test's own environment and home, so no global or system config applies.
fn git(play: &Play, dir: &Path) -> Command {
    let mut command = play.command("git");
    command
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "e2e")
        .env("GIT_AUTHOR_EMAIL", "e2e@example.com")
        .env("GIT_COMMITTER_NAME", "e2e")
        .env("GIT_COMMITTER_EMAIL", "e2e@example.com");
    command
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
