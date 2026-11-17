use herdr_bus::{Bus, MessageQuery, Stored};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

/// A factory with one workflow, a bus file and a Priory state file under the test's own directory.
struct Setup {
    dir: PathBuf,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("request")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        Setup { dir }
    }

    fn priory(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("USER", "max")
            .output()
            .unwrap()
    }

    /// `priory request <factory> <args>`.
    fn request(&self, args: &[&str]) -> Output {
        let factory = self.dir.join("factory");
        let mut all = vec!["request", factory.to_str().unwrap()];
        all.extend(args);
        self.priory(&all)
    }

    /// Everything on `factory.intake`, oldest first; none before anything was published.
    fn published(&self) -> Vec<Stored> {
        let Ok(bus) = Bus::open(&self.dir.join("bus.db")) else {
            return vec![];
        };
        let query = MessageQuery {
            output: Some("factory.intake".into()),
            limit: 1000,
            ..MessageQuery::default()
        };
        let mut messages = bus.messages(&query).unwrap_or_default();
        messages.reverse();
        messages
    }

    /// The rows `priory tasks` prints, each split on tabs, after its header.
    fn tasks(&self) -> Vec<Vec<String>> {
        let output = self.priory(&["tasks"]);
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_request_publishes_its_source_event_then_the_task_request() {
    let setup = Setup::new("publishes");

    succeeded(&setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--band",
        "today",
        "--repo",
        "owner/repo",
        "Fix the docs",
    ]));

    let published = setup.published();
    let kinds: Vec<_> = published.iter().map(|m| m.message.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["priory.task.requested.cli", "factory.task.requested"]
    );
    let (source, task) = (&published[0].message, &published[1].message);
    assert_eq!(source.subject.as_deref(), Some("cli:docs-1"));
    assert_eq!(source.data["text"], "Fix the docs");
    assert_eq!(source.data["person"], "max");
    assert_eq!(source.data["repo"], "owner/repo");
    assert!(source.data["time"].as_str().unwrap().ends_with('Z'));
    assert_eq!(
        task.data,
        json!({
            "workflow": "implement",
            "key": "cli:docs-1",
            "band": "today",
            "lane": null,
            "trigger": "cli",
            "source": source.id.to_string(),
        })
    );
    assert_eq!(task.subject.as_deref(), Some("cli:docs-1"));
}

#[test]
fn the_same_request_twice_publishes_nothing_new() {
    let setup = Setup::new("twice");
    let args = ["--workflow", "implement", "--key", "docs-1", "Fix the docs"];

    succeeded(&setup.request(&args));
    succeeded(&setup.request(&args));

    assert_eq!(setup.published().len(), 2);
}

#[test]
fn an_unknown_workflow_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("unknown");

    let output = setup.request(&["--workflow", "nope", "--key", "docs-1", "Fix the docs"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nope"));
    assert!(setup.published().is_empty());
}

#[test]
fn the_daemon_turns_a_request_into_one_queued_task() {
    let setup = Setup::new("daemon");
    succeeded(&setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--lane",
        "docs",
        "Fix the docs",
    ]));

    let factory = setup.dir.join("factory");
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    assert_eq!(
        setup.tasks(),
        [[
            "1",
            "cli:docs-1",
            "implement",
            "implement",
            "queued",
            "-",
            "docs"
        ]]
    );
}
