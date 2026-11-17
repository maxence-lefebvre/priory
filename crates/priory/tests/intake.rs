use herdr_bus::{Bus, Message, MessageQuery, Stored};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
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

const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
band = "today"
"#;

const EVENTS_PATH: &str = "repos/owner/repo/issues/events?per_page=100&page=1";
const ISSUES_PATH: &str =
    "repos/owner/repo/issues?state=all&sort=created&direction=desc&per_page=100&page=1";

/// A factory, a fake `gh` that answers each `gh api <path>` from `answers`, and a bus file.
struct Setup {
    dir: PathBuf,
}

impl Setup {
    fn new(name: &str, answers: &[(&str, serde_json::Value)]) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("intake")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
            ("factory/triggers/agent-label.toml", TRIGGER),
        ];
        for (path, content) in files {
            write(&dir.join(path), content);
        }
        let setup = Setup { dir };
        setup.answer(answers);
        setup
    }

    /// Makes the fake `gh` answer each `gh api <path>` from `answers`, and `[]` otherwise.
    fn answer(&self, answers: &[(&str, serde_json::Value)]) {
        let mut script = String::from("#!/bin/sh\ncase \"$*\" in\n");
        for (path, answer) in answers {
            let file = self.dir.join(format!("answer-{}.json", script.len()));
            write(&file, &answer.to_string());
            script.push_str(&format!("  \"api {path}\") cat '{}' ;;\n", file.display()));
        }
        script.push_str("  *) echo '[]' ;;\nesac\n");
        Setup::fake_gh(&self.dir, &script);
    }

    /// A first poll of `owner/repo`, which only records where polling stands.
    fn baseline(name: &str) -> Self {
        let setup = Setup::new(name, &[(EVENTS_PATH, json!([labeled(100, 41, "bug")]))]);
        succeeded(&setup.intake());
        setup
    }

    fn fake_gh(dir: &Path, script: &str) {
        let gh = dir.join("bin/gh");
        write(&gh, script);
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn intake(&self) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(["intake", "--once"])
            .arg(self.dir.join("factory"))
            .env("PATH", path)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .output()
            .unwrap()
    }

    /// The source events and task requests on `factory.intake`, oldest first.
    fn published(&self) -> Vec<Stored> {
        let mut messages = self.output();
        messages.retain(|m| m.message.kind != "factory.intake.baseline");
        messages
    }

    /// Everything on `factory.intake`, oldest first.
    fn output(&self) -> Vec<Stored> {
        let bus = Bus::open(&self.dir.join("bus.db")).unwrap();
        let mut messages = bus
            .messages(&MessageQuery {
                output: Some("factory.intake".into()),
                limit: 1000,
                ..MessageQuery::default()
            })
            .unwrap();
        messages.reverse();
        messages
    }
}

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn labeled(id: u64, number: u64, label: &str) -> serde_json::Value {
    json!({
        "id": id,
        "event": "labeled",
        "created_at": "2026-11-17T10:00:00Z",
        "actor": { "login": "max" },
        "label": { "name": label, "color": "ededed" },
        "issue": {
            "id": 9000 + number,
            "number": number,
            "title": "Fix the thing",
            "html_url": format!("https://github.com/owner/repo/issues/{number}"),
            "user": { "login": "max" },
            "created_at": "2026-11-17T09:00:00Z"
        }
    })
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_labeled_event_that_matches_a_trigger_requests_a_task() {
    let setup = Setup::baseline("labeled-match");
    setup.answer(&[(
        EVENTS_PATH,
        json!([labeled(101, 42, "agent"), labeled(100, 41, "bug")]),
    )]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published[1..]),
        ["github.issues.labeled", "factory.task.requested"]
    );
    let (source, task) = (&published[1].message, &published[2].message);
    assert_eq!(
        source.data,
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "title": "Fix the thing",
            "url": "https://github.com/owner/repo/issues/42",
            "label": "agent",
            "actor": "max",
            "time": "2026-11-17T10:00:00Z",
            "github_id": 101,
            "baseline": false,
            "triggers": ["agent-label"]
        })
    );
    assert_eq!(
        task.data,
        json!({
            "workflow": "implement",
            "key": "github:owner/repo#42",
            "band": "today",
            "lane": null,
            "trigger": "agent-label",
            "source": source.id.to_string()
        })
    );
}

#[test]
fn a_first_poll_is_a_baseline_that_requests_no_task() {
    let setup = Setup::new(
        "baseline",
        &[(EVENTS_PATH, json!([labeled(101, 42, "agent")]))],
    );

    succeeded(&setup.intake());

    let output = setup.output();
    assert_eq!(
        kinds(&output),
        [
            "github.issues.labeled",
            "factory.intake.baseline",
            "factory.intake.baseline"
        ]
    );
    assert_eq!(output[0].message.data["baseline"], true);
    assert_eq!(output[0].message.data["triggers"], json!([]));
    assert_eq!(
        output[1].message.data,
        json!({ "repo": "owner/repo", "stream": "github.issues.labeled", "position": 101 })
    );
}

#[test]
fn a_restart_after_a_baseline_publishes_nothing() {
    let setup = Setup::new(
        "baseline-restart",
        &[(EVENTS_PATH, json!([labeled(101, 42, "agent")]))],
    );
    succeeded(&setup.intake());
    setup.answer(&[]);

    succeeded(&setup.intake());

    assert_eq!(kinds(&setup.published()), ["github.issues.labeled"]);
}

fn kinds(published: &[Stored]) -> Vec<&str> {
    published.iter().map(|m| m.message.kind.as_str()).collect()
}

#[test]
fn a_label_that_matches_no_trigger_publishes_only_the_source_event() {
    let setup = Setup::baseline("labeled-no-match");
    setup.answer(&[(EVENTS_PATH, json!([labeled(101, 42, "bug")]))]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(kinds(&published[1..]), ["github.issues.labeled"]);
    assert_eq!(published[1].message.data["triggers"], json!([]));
}

#[test]
fn a_second_poll_of_the_same_events_publishes_nothing_new() {
    let setup = Setup::new(
        "repeat",
        &[(
            EVENTS_PATH,
            json!([labeled(102, 43, "agent"), labeled(101, 42, "agent")]),
        )],
    );
    succeeded(&setup.intake());
    let first = setup.published();

    succeeded(&setup.intake());

    assert_eq!(first.len(), 2);
    assert_eq!(setup.published().len(), first.len());
}

#[test]
fn events_publish_oldest_first() {
    let setup = Setup::new(
        "order",
        &[(
            EVENTS_PATH,
            json!([labeled(102, 43, "bug"), labeled(101, 42, "bug")]),
        )],
    );

    succeeded(&setup.intake());

    let issues: Vec<_> = setup
        .published()
        .iter()
        .map(|m| m.message.data["issue"].clone())
        .collect();
    assert_eq!(issues, [json!(42), json!(43)]);
}

#[test]
fn a_failing_gh_exits_non_zero_and_publishes_nothing() {
    let setup = Setup::new("gh-fails", &[]);
    Setup::fake_gh(
        &setup.dir,
        "#!/bin/sh\necho 'HTTP 404: Not Found' >&2\nexit 1\n",
    );

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("priory intake: gh api {EVENTS_PATH} failed: HTTP 404: Not Found\n")
    );
    assert!(setup.published().is_empty());
}

#[test]
fn a_backfill_trigger_requests_tasks_for_the_first_poll() {
    let setup = Setup::new(
        "opened",
        &[(
            ISSUES_PATH,
            json!([{
                "id": 9042,
                "number": 42,
                "title": "Fix the thing",
                "html_url": "https://github.com/owner/repo/issues/42",
                "user": { "login": "max" },
                "created_at": "2026-11-17T09:00:00Z"
            }, {
                "id": 9041,
                "number": 41,
                "title": "A pull request",
                "html_url": "https://github.com/owner/repo/pull/41",
                "user": { "login": "max" },
                "created_at": "2026-11-17T08:00:00Z",
                "pull_request": { "url": "https://api.github.com/repos/owner/repo/pulls/41" }
            }]),
        )],
    );

    write(
        &setup.dir.join("factory/triggers/opened.toml"),
        "source = \"github\"\nrepo = \"owner/repo\"\non = \"issues.opened\"\nworkflow = \"implement\"\nbackfill = true\n",
    );

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        ["github.issues.opened", "factory.task.requested"]
    );
    assert_eq!(published[1].message.data["trigger"], "opened");
    assert_eq!(
        published[0].message.data,
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "title": "Fix the thing",
            "url": "https://github.com/owner/repo/issues/42",
            "actor": "max",
            "time": "2026-11-17T09:00:00Z",
            "github_id": 9042,
            "baseline": true,
            "triggers": ["opened"]
        })
    );
}

#[test]
fn polling_resumes_where_it_stopped() {
    let setup = Setup::new("resume", &[(EVENTS_PATH, json!([labeled(101, 42, "bug")]))]);
    succeeded(&setup.intake());
    let full: Vec<_> = (0..100).map(|n| labeled(300 - n, 60, "bug")).collect();
    let second = json!([labeled(200, 50, "bug"), labeled(101, 42, "bug")]);
    let page_two = "repos/owner/repo/issues/events?per_page=100&page=2";
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  \"api {EVENTS_PATH}\") echo '{}' ;;\n  \"api {page_two}\") echo '{}' ;;\n  *) echo '[]' ;;\nesac\n",
        json!(full),
        second
    );
    Setup::fake_gh(&setup.dir, &script);

    succeeded(&setup.intake());

    assert_eq!(setup.published().len(), 102);
}

#[test]
fn a_restart_requests_the_tasks_a_crash_left_out() {
    let setup = Setup::new("recover", &[]);
    succeeded(&setup.intake());
    let bus = Bus::open(&setup.dir.join("bus.db")).unwrap();
    let source = Message::new(
        "github.issues.labeled",
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "label": "agent",
            "github_id": 101,
            "baseline": false,
            "triggers": ["agent-label"]
        }),
    );
    bus.publish("factory.intake", &source).unwrap();

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        ["github.issues.labeled", "factory.task.requested"]
    );
    assert_eq!(published[1].message.data["source"], source.id.to_string());
    assert_eq!(published[1].message.data["key"], "github:owner/repo#42");
}

#[test]
fn a_trigger_added_later_gets_no_task_for_events_already_dispatched() {
    let setup = Setup::baseline("added-trigger");
    setup.answer(&[(
        EVENTS_PATH,
        json!([labeled(101, 42, "agent"), labeled(100, 41, "bug")]),
    )]);
    succeeded(&setup.intake());
    let before = setup.published().len();
    write(
        &setup.dir.join("factory/triggers/agent-label-too.toml"),
        TRIGGER,
    );
    setup.answer(&[]);

    succeeded(&setup.intake());

    assert_eq!(setup.published().len(), before);
}

#[test]
fn a_first_poll_with_no_events_still_sets_the_baseline() {
    let setup = Setup::new("empty-baseline", &[]);
    succeeded(&setup.intake());
    setup.answer(&[(EVENTS_PATH, json!([labeled(101, 42, "agent")]))]);

    succeeded(&setup.intake());

    let tasks = kinds(&setup.published())
        .into_iter()
        .filter(|kind| *kind == "factory.task.requested")
        .count();
    assert_eq!(tasks, 1);
}

/// A bus holding `source` on `factory.intake`, as a crash would leave it.
fn seed(setup: &Setup, source: &Message) {
    let bus = Bus::open(&setup.dir.join("bus.db")).unwrap();
    bus.declare(&herdr_bus::Output::new("factory.intake"))
        .unwrap();
    bus.publish("factory.intake", source).unwrap();
}

fn baseline_source(github_id: u64, triggers: serde_json::Value) -> Message {
    Message::new(
        "github.issues.labeled",
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "label": "agent",
            "github_id": github_id,
            "baseline": true,
            "triggers": triggers
        }),
    )
}

#[test]
fn a_baseline_a_crash_interrupted_stays_a_baseline() {
    let setup = Setup::new(
        "interrupted-baseline",
        &[(
            EVENTS_PATH,
            json!([labeled(102, 43, "agent"), labeled(101, 42, "agent")]),
        )],
    );
    seed(&setup, &baseline_source(101, json!([])));

    succeeded(&setup.intake());

    let published = setup.published();
    assert!(!kinds(&published).contains(&"factory.task.requested"));
    assert!(
        published
            .iter()
            .filter(|m| m.message.kind == "github.issues.labeled")
            .all(|m| m.message.data["baseline"] == true)
    );
}

#[test]
fn a_restart_never_dispatches_a_baseline_source() {
    let setup = Setup::new("baseline-guard", &[]);
    seed(&setup, &baseline_source(101, json!(["agent-label"])));

    succeeded(&setup.intake());

    assert!(!kinds(&setup.published()).contains(&"factory.task.requested"));
}

#[test]
fn a_baseline_requests_tasks_only_for_backfill_triggers() {
    let setup = Setup::new(
        "mixed-backfill",
        &[(EVENTS_PATH, json!([labeled(101, 42, "agent")]))],
    );
    write(
        &setup.dir.join("factory/triggers/agent-backfill.toml"),
        &format!("{TRIGGER}backfill = true\n"),
    );

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        published[0].message.data["triggers"],
        json!(["agent-backfill"])
    );
    let tasks: Vec<_> = published
        .iter()
        .filter(|m| m.message.kind == "factory.task.requested")
        .map(|m| m.message.data["trigger"].clone())
        .collect();
    assert_eq!(tasks, [json!("agent-backfill")]);
}
