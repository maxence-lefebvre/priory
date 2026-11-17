use herdr_bus::{Bus, Message, Output};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output as Exit};

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

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\n+++\n";

const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
band = "today"
"#;

const OUTPUT: &str = "factory.intake";

/// A factory, a bus file and a Priory state file under the test's own directory.
struct Setup {
    dir: PathBuf,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("daemon")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
            ("factory/triggers/agent-label.toml", TRIGGER),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        let setup = Setup { dir };
        setup.bus().declare(&Output::new(OUTPUT)).unwrap();
        setup
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    fn publish(&self, message: &Message) {
        self.bus().publish(OUTPUT, message).unwrap();
    }

    fn priory(&self, args: &[&str]) -> Exit {
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .output()
            .unwrap()
    }

    fn daemon(&self) -> Exit {
        let factory = self.dir.join("factory");
        self.priory(&["daemon", factory.to_str().unwrap(), "--once"])
    }

    /// The rows `priory <verb>` prints, each split on tabs, after its header.
    fn rows(&self, verb: &str) -> Vec<Vec<String>> {
        let output = self.priory(&[verb]);
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }
}

fn succeeded(output: &Exit) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn labeled(issue: u64) -> Message {
    Message::new(
        "github.issues.labeled",
        json!({
            "repo": "owner/repo",
            "issue": issue,
            "label": "agent",
            "github_id": 100 + issue,
            "baseline": false,
            "triggers": ["agent-label"]
        }),
    )
    .subject(format!("owner/repo#{issue}"))
}

fn requested(source: &Message, issue: u64) -> Message {
    let key = format!("github:owner/repo#{issue}");
    Message::new(
        "factory.task.requested",
        json!({
            "workflow": "implement",
            "key": key,
            "band": "today",
            "lane": null,
            "trigger": "agent-label",
            "source": source.id.to_string()
        }),
    )
    .subject(key)
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|cell| (*cell).to_owned()).collect()
}

#[test]
fn a_task_request_creates_one_queued_task_at_its_workflow_start() {
    let setup = Setup::new("create");
    let source = labeled(42);
    setup.publish(&source);
    setup.publish(&requested(&source, 42));

    succeeded(&setup.daemon());

    assert_eq!(
        setup.rows("tasks"),
        [row(&[
            "1",
            "github:owner/repo#42",
            "implement",
            "implement",
            "queued",
            "today",
            "-"
        ])]
    );
    assert_eq!(
        setup.rows("inbox"),
        [
            row(&[
                "1",
                "github.issues.labeled",
                "owner/repo#42",
                "-",
                "ignored"
            ]),
            row(&[
                "2",
                "factory.task.requested",
                "github:owner/repo#42",
                "1",
                "created"
            ]),
        ]
    );
}

/// The `task` and `outcome` of each inbox entry, oldest first.
fn routes(setup: &Setup) -> Vec<(String, String)> {
    setup
        .rows("inbox")
        .into_iter()
        .map(|row| (row[3].clone(), row[4].clone()))
        .collect()
}

fn route(task: &str, outcome: &str) -> (String, String) {
    (task.to_owned(), outcome.to_owned())
}

#[test]
fn a_second_task_request_for_the_same_key_is_kept_on_the_task() {
    let setup = Setup::new("same-key");
    let first = labeled(42);
    let second = labeled(42);
    for message in [&first, &requested(&first, 42), &second] {
        setup.publish(message);
    }
    let mut again = requested(&second, 42);
    again.data["trigger"] = json!("agent-label-too");
    setup.publish(&again);

    succeeded(&setup.daemon());

    assert_eq!(setup.rows("tasks").len(), 1);
    assert_eq!(
        routes(&setup),
        [
            route("-", "ignored"),
            route("1", "created"),
            route("1", "kept"),
            route("1", "kept")
        ]
    );
}

#[test]
fn a_source_event_is_kept_on_the_task_that_owns_its_key_and_ignored_otherwise() {
    let setup = Setup::new("source-events");
    let source = labeled(42);
    for message in [&source, &requested(&source, 42), &labeled(42), &labeled(43)] {
        setup.publish(message);
    }

    succeeded(&setup.daemon());

    assert_eq!(
        routes(&setup),
        [
            route("-", "ignored"),
            route("1", "created"),
            route("1", "kept"),
            route("-", "ignored")
        ]
    );
}

#[test]
fn a_message_of_an_unknown_type_is_ignored_and_acked() {
    let setup = Setup::new("unknown-type");
    setup.publish(&Message::new(
        "factory.intake.baseline",
        json!({ "repo": "owner/repo", "stream": "github.issues.labeled", "position": 0 }),
    ));

    succeeded(&setup.daemon());

    assert_eq!(routes(&setup), [route("-", "ignored")]);
    let connections = setup.bus().connections().unwrap();
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0].lag, 0);
}

#[test]
fn a_task_request_for_a_workflow_the_factory_lacks_waits_unacked_until_it_is_back() {
    let setup = Setup::new("unknown-workflow");
    let source = labeled(42);
    let mut request = requested(&source, 42);
    request.data["workflow"] = json!("triage");
    setup.publish(&request);

    let output = setup.daemon();

    assert_eq!(output.status.code(), Some(1));
    assert!(setup.rows("inbox").is_empty());
    assert_eq!(setup.bus().connections().unwrap()[0].lag, 1);
    fs::write(setup.dir.join("factory/workflows/triage.toml"), WORKFLOW).unwrap();
    succeeded(&setup.daemon());
    assert_eq!(setup.rows("tasks")[0][2], "triage");
}

#[test]
fn a_state_file_that_vanishes_on_exit_is_refused() {
    let setup = Setup::new("memory-db");
    let factory = setup.dir.join("factory");
    for db in ["", ":memory:", "file::memory:"] {
        let output = Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(["daemon", factory.to_str().unwrap(), "--once"])
            .env("HERDR_BUS_DB", setup.dir.join("bus.db"))
            .env("PRIORY_DB", db)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(1), "PRIORY_DB={db:?}");
    }
    assert!(setup.bus().connections().unwrap().is_empty());
}

/// A task request and its source, handled by one `priory daemon --once`.
fn handled(name: &str) -> Setup {
    let setup = Setup::new(name);
    let source = labeled(42);
    setup.publish(&source);
    setup.publish(&requested(&source, 42));
    succeeded(&setup.daemon());
    setup
}

#[test]
fn a_second_run_over_the_same_messages_changes_nothing() {
    let setup = handled("second-run");
    let (inbox, tasks) = (setup.rows("inbox"), setup.rows("tasks"));

    succeeded(&setup.daemon());

    assert_eq!(setup.rows("inbox"), inbox);
    assert_eq!(setup.rows("tasks"), tasks);
}

#[test]
fn a_message_handled_but_not_acked_is_not_handled_twice() {
    let setup = handled("not-acked");
    let (inbox, tasks) = (setup.rows("inbox"), setup.rows("tasks"));
    let bus = setup.bus();
    // Dropping the cursor delivers every message again, as a crash between commit and ack does.
    bus.disconnect(bus.connections().unwrap()[0].id).unwrap();
    bus.connect("priory", OUTPUT, herdr_bus::Start::Beginning)
        .unwrap();
    assert_eq!(bus.pull("priory", 10).unwrap().len(), 2);

    succeeded(&setup.daemon());

    assert_eq!(setup.rows("inbox"), inbox);
    assert_eq!(setup.rows("tasks"), tasks);
    assert_eq!(bus.connections().unwrap()[0].lag, 0);
}

#[test]
fn a_factory_with_problems_stops_the_daemon() {
    let setup = Setup::new("problems");
    fs::write(
        setup.dir.join("factory/workflows/implement.toml"),
        "start = \"missing\"\nsteps = []\n",
    )
    .unwrap();

    let output = setup.daemon();

    assert_eq!(output.status.code(), Some(1));
    assert!(!setup.dir.join("priory.db").exists());
}

#[test]
fn a_pull_request_event_is_kept_on_the_task_that_owns_its_key() {
    let setup = Setup::new("pull-request-events");
    let source = labeled(42);
    let check = Message::new(
        "github.check_run.completed",
        json!({
            "repo": "owner/repo",
            "pull_request": 42,
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "name": "test",
            "conclusion": "success",
            "github_id": 601,
            "baseline": false,
            "triggers": []
        }),
    );
    for message in [&source, &requested(&source, 42), &check] {
        setup.publish(message);
    }

    succeeded(&setup.daemon());

    assert_eq!(
        routes(&setup),
        [
            route("-", "ignored"),
            route("1", "created"),
            route("1", "kept")
        ]
    );
}
