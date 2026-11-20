mod conform;

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

    /// Runs `priory request` in repository `owner/repo`; a later `--repo` in `args` wins.
    fn request(&self, args: &[&str]) -> Output {
        let factory = self.dir.join("factory");
        let mut all = vec!["request", factory.to_str().unwrap(), "--repo", "owner/repo"];
        all.extend(args);
        self.priory(&all)
    }

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
        for stored in &messages {
            conform::conforms(&serde_json::to_value(&stored.message).unwrap());
        }
        messages
    }

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
            "repo": "owner/repo",
        })
    );
    assert_eq!(task.subject.as_deref(), Some("cli:docs-1"));
}

#[test]
fn a_request_with_no_repo_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("no-repo");
    let factory = setup.dir.join("factory");

    let output = setup.priory(&[
        "request",
        factory.to_str().unwrap(),
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "Fix the docs",
    ]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory request: the request names no repo; pass --repo owner/name, or set repo in triggers/cli.toml\n"
    );
    assert!(setup.published().is_empty());
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
fn an_unknown_band_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("unknown-band");

    let output = setup.request(&[
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "--band",
        "someday",
        "Fix the docs",
    ]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory request: band `someday` is not one of blocker, now, today, later\n"
    );
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
            "later",
            "docs",
            "-",
            "1",
            "1",
            "-",
            "-",
            "-"
        ]]
    );
}

#[test]
fn a_colon_in_the_key_or_text_does_not_merge_two_requests() {
    let setup = Setup::new("collision");

    succeeded(&setup.request(&["--workflow", "implement", "--key", "a:b", "c"]));
    succeeded(&setup.request(&["--workflow", "implement", "--key", "a", "b:c"]));

    assert_eq!(setup.published().len(), 4);
}

#[test]
fn a_flag_without_its_value_is_a_usage_error_and_publishes_nothing() {
    let setup = Setup::new("missing-value");

    let output = setup.request(&["--workflow", "implement", "--key", "--band", "today"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(setup.published().is_empty());
}

#[test]
fn text_after_a_double_dash_is_literal() {
    let setup = Setup::new("literal");

    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "--", "--band"]));

    assert_eq!(setup.published()[0].message.data["text"], "--band");
}

#[test]
fn the_source_event_time_is_the_time_in_its_data() {
    let setup = Setup::new("time");
    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "text"]));

    let source = &setup.published()[0].message;

    assert_eq!(source.time.to_string(), source.data["time"]);
}

#[test]
fn a_second_request_for_a_key_is_kept_on_its_task() {
    let setup = Setup::new("follow-up");
    let factory = setup.dir.join("factory");
    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "first"]));
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));
    succeeded(&setup.request(&["--workflow", "implement", "--key", "k", "second"]));
    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    let inbox = String::from_utf8(setup.priory(&["inbox"]).stdout).unwrap();
    let outcomes: Vec<_> = inbox
        .lines()
        .skip(1)
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .collect();

    assert_eq!(outcomes.len(), 4);
    assert_eq!(outcomes[2][3..5], ["1", "kept"]);
    assert_eq!(setup.tasks().len(), 1);
}

#[test]
fn a_request_with_a_hold_files_its_task_held() {
    let setup = Setup::new("held");
    let request = ["--workflow", "implement", "--key"];
    succeeded(
        &setup.request(
            &[
                &request[..],
                &["docs-1", "--hold", "after the release"],
                &["--until", "2030-01-01T00:00:00Z", "Fix the docs"],
            ]
            .concat(),
        ),
    );
    succeeded(
        &setup.request(
            &[
                &request[..],
                &["docs-2", "--hold", "later", "--after", "99", "Fix more"],
            ]
            .concat(),
        ),
    );
    let factory = setup.dir.join("factory");

    succeeded(&setup.priory(&["daemon", factory.to_str().unwrap(), "--once"]));

    assert_eq!(
        setup.published()[1].message.data["hold"],
        json!({ "reason": "after the release", "until": "2030-01-01T00:00:00Z" })
    );
    let tasks: Vec<_> = setup
        .tasks()
        .into_iter()
        .map(|row| [row[4].clone(), row[7].clone()])
        .collect();
    assert_eq!(
        tasks,
        [
            [
                "held",
                "held: after the release, until 2030-01-01T00:00:00.000Z"
            ],
            [
                "held",
                "held: later, until a person releases it, since its condition failed: after must name another task, not 99"
            ]
        ]
    );
}
