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

    /// Makes the fake `gh` answer each `gh api <path>...` from `answers`, and `[]` otherwise.
    fn answer(&self, answers: &[(&str, serde_json::Value)]) {
        let mut script = String::from("#!/bin/sh\ncase \"$*\" in\n");
        for (path, answer) in answers {
            let file = self.dir.join(format!("answer-{}.json", script.len()));
            write(&file, &answer.to_string());
            script.push_str(&format!("  \"api {path}\"*) cat '{}' ;;\n", file.display()));
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
        self.command().env("LINEAR_API_KEY", KEY).output().unwrap()
    }

    /// Runs `priory intake --once` with the clock set to `now`.
    fn intake_at(&self, now: &str) -> Output {
        self.command().env("PRIORY_NOW", now).output().unwrap()
    }

    fn command(&self) -> Command {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .args(["intake", "--once"])
            .arg(self.dir.join("factory"))
            .env("PATH", path)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"));
        command
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
            "factory.intake.baseline",
            "factory.intake.baseline",
            "github.issues.labeled",
            "factory.intake.baseline",
            "factory.intake.baseline"
        ]
    );
    assert_eq!(output[2].message.data["baseline"], true);
    assert_eq!(output[2].message.data["triggers"], json!([]));
    assert_eq!(
        output[3].message.data,
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
        [
            "github.pull_request.opened",
            "github.issues.opened",
            "factory.task.requested"
        ]
    );
    assert_eq!(published[2].message.data["trigger"], "opened");
    assert_eq!(
        published[1].message.data,
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

const OPEN_PULLS: &str = "graphql --paginate";
const PULLS: &str = "graphql -f";

/// A pull request as `issues` lists it.
fn pull_issue(number: u64) -> serde_json::Value {
    json!({
        "id": 9000 + number,
        "number": number,
        "title": "Add the thing",
        "html_url": format!("https://github.com/owner/repo/pull/{number}"),
        "user": { "login": "dev" },
        "created_at": "2026-11-17T11:00:00Z",
        "pull_request": { "url": format!("https://api.github.com/repos/owner/repo/pulls/{number}") }
    })
}

/// A pull request as the GraphQL queries return it.
fn pull(
    number: u64,
    merged: bool,
    reviews: serde_json::Value,
    checks: serde_json::Value,
) -> serde_json::Value {
    json!({
        "number": number,
        "headRefName": "feature",
        "headRefOid": "abc123",
        "baseRefName": "main",
        "merged": merged,
        // Within any grace period: a head with no check runs yet publishes nothing.
        "updatedAt": "2999-01-01T00:00:00Z",
        "reviews": { "nodes": reviews },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "contexts": {
            "totalCount": checks.as_array().map_or(0, Vec::len),
            "nodes": checks
        } } } }] }
    })
}

/// The answer to the query for the pull requests some events name.
fn pulls(pulls: &[serde_json::Value]) -> serde_json::Value {
    let repository: serde_json::Map<_, _> = pulls
        .iter()
        .map(|pull| (format!("pr{}", pull["number"]), pull.clone()))
        .collect();
    json!({ "data": { "repository": repository } })
}

/// The answer to the paginated query for open pull requests.
fn open_pulls(pulls: &[serde_json::Value]) -> serde_json::Value {
    json!([{ "data": { "repository": { "pullRequests": { "nodes": pulls } } } }])
}

fn pr_trigger(setup: &Setup, name: &str, toml: &str) {
    write(
        &setup.dir.join(format!("factory/triggers/{name}.toml")),
        &format!(
            "source = \"github\"\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nkey = \"pull_request.number\"\n{toml}"
        ),
    );
}

#[test]
fn an_opened_pull_request_that_matches_a_trigger_requests_a_task() {
    let setup = Setup::baseline("pr-opened");
    pr_trigger(&setup, "pr-opened", "on = \"pull_request.opened\"\n");
    setup.answer(&[
        (ISSUES_PATH, json!([pull_issue(43)])),
        (PULLS, pulls(&[pull(43, false, json!([]), json!([]))])),
    ]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published[1..]),
        ["github.pull_request.opened", "factory.task.requested"]
    );
    let (source, task) = (&published[1].message, &published[2].message);
    assert_eq!(
        source.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "title": "Add the thing",
            "url": "https://github.com/owner/repo/pull/43",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "actor": "dev",
            "time": "2026-11-17T11:00:00Z",
            "github_id": 9043,
            "baseline": false,
            "triggers": ["pr-opened"]
        })
    );
    assert_eq!(source.subject.as_deref(), Some("owner/repo#43"));
    assert_eq!(task.data["key"], "github:owner/repo#43");
    assert_eq!(task.data["trigger"], "pr-opened");
}

/// An issue event on pull request `number`.
fn pull_event(id: u64, number: u64, name: &str, label: Option<&str>) -> serde_json::Value {
    let mut event = labeled(id, number, label.unwrap_or_default());
    event["event"] = json!(name);
    event["issue"] = pull_issue(number);
    if label.is_none() {
        event["label"] = json!(null);
    }
    event
}

#[test]
fn a_labeled_pull_request_requests_a_task_only_for_a_matching_label() {
    let setup = Setup::baseline("pr-labeled");
    pr_trigger(
        &setup,
        "pr-label",
        "on = \"pull_request.labeled\"\nfilter = { label = \"agent\" }\n",
    );
    setup.answer(&[
        (
            EVENTS_PATH,
            json!([
                pull_event(102, 43, "labeled", Some("bug")),
                pull_event(101, 43, "labeled", Some("agent")),
            ]),
        ),
        (PULLS, pulls(&[pull(43, false, json!([]), json!([]))])),
    ]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published[1..]),
        [
            "github.pull_request.labeled",
            "factory.task.requested",
            "github.pull_request.labeled"
        ]
    );
    assert_eq!(
        published[1].message.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "title": "Add the thing",
            "url": "https://github.com/owner/repo/pull/43",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "label": "agent",
            "actor": "max",
            "time": "2026-11-17T10:00:00Z",
            "github_id": 101,
            "baseline": false,
            "triggers": ["pr-label"]
        })
    );
    assert_eq!(published[2].message.data["key"], "github:owner/repo#43");
    assert_eq!(published[3].message.data["triggers"], json!([]));
}

#[test]
fn a_merged_pull_request_publishes_closed_with_merged() {
    let setup = Setup::baseline("pr-merged");
    setup.answer(&[
        (
            EVENTS_PATH,
            json!([
                pull_event(103, 43, "closed", None),
                pull_event(102, 43, "merged", None),
            ]),
        ),
        (PULLS, pulls(&[pull(43, true, json!([]), json!([]))])),
    ]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(kinds(&published[1..]), ["github.pull_request.closed"]);
    let data = &published[1].message.data;
    assert_eq!(
        (
            &data["pull_request"],
            &data["merged"],
            &data["actor"],
            &data["github_id"]
        ),
        (&json!(43), &json!(true), &json!("max"), &json!(103))
    );
}

#[test]
fn a_submitted_review_and_a_completed_check_publish_once() {
    let setup = Setup::baseline("pr-review-check");
    let reviews = json!([
        {
            "databaseId": 501,
            "state": "APPROVED",
            "submittedAt": "2026-11-17T12:00:00Z",
            "url": "https://github.com/owner/repo/pull/43#pullrequestreview-501",
            "author": { "login": "reviewer" }
        },
        { "databaseId": 502, "state": "PENDING", "submittedAt": null, "url": "u", "author": null }
    ]);
    let checks = json!([
        {
            "__typename": "CheckRun",
            "databaseId": 601,
            "name": "test (ubuntu)",
            "status": "COMPLETED",
            "conclusion": "FAILURE",
            "completedAt": "2026-11-17T12:05:00Z",
            "url": "https://github.com/owner/repo/runs/601",
            "checkSuite": { "app": { "slug": "github-actions" } }
        },
        {
            "__typename": "CheckRun",
            "databaseId": 602,
            "name": "lint",
            "status": "IN_PROGRESS",
            "conclusion": null,
            "completedAt": null,
            "url": "https://github.com/owner/repo/runs/602",
            "checkSuite": { "app": { "slug": "github-actions" } }
        },
        { "__typename": "StatusContext" }
    ]);
    setup.answer(&[(OPEN_PULLS, open_pulls(&[pull(43, false, reviews, checks)]))]);

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published[1..]),
        [
            "github.pull_request_review.submitted",
            "github.pull_request.check_runs",
            "github.check_run.completed"
        ]
    );
    assert_eq!(published[2].message.data["runs"], 2);
    assert_eq!(
        published[1].message.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "url": "https://github.com/owner/repo/pull/43#pullrequestreview-501",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "state": "approved",
            "actor": "reviewer",
            "time": "2026-11-17T12:00:00Z",
            "github_id": 501,
            "baseline": false,
            "triggers": []
        })
    );
    assert_eq!(
        published[3].message.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "url": "https://github.com/owner/repo/runs/601",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "name": "test (ubuntu)",
            "conclusion": "failure",
            "app": "github-actions",
            "pending": 1,
            "failing": 1,
            "time": "2026-11-17T12:05:00Z",
            "github_id": 601,
            "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_check_counts_the_runs_past_the_first_page_as_pending() {
    let setup = Setup::baseline("pr-check-page");
    let completed = json!({
        "__typename": "CheckRun",
        "databaseId": 601,
        "name": "test",
        "status": "COMPLETED",
        "conclusion": "SUCCESS",
        "completedAt": "2026-11-17T12:05:00Z",
        "url": "u",
        "checkSuite": null
    });
    let mut pull = pull(43, false, json!([]), json!([completed]));
    pull["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["totalCount"] =
        json!(101);
    setup.answer(&[(OPEN_PULLS, open_pulls(&[pull]))]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(published[1].message.data["runs"], 101);
    assert_eq!(published[2].message.data["pending"], 100);
    assert_eq!(published[2].message.data["failing"], 0);
}

#[test]
fn a_restart_requests_the_task_a_crash_left_out_for_a_pull_request() {
    let setup = Setup::new("pr-recover", &[]);
    succeeded(&setup.intake());
    let source = Message::new(
        "github.pull_request.labeled",
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "label": "agent",
            "github_id": 101,
            "baseline": false,
            "triggers": ["agent-label"]
        }),
    );
    seed(&setup, &source);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(kinds(&published[1..]), ["factory.task.requested"]);
    assert_eq!(published[1].message.data["key"], "github:owner/repo#43");
}

#[test]
fn a_failing_graphql_query_publishes_nothing() {
    let setup = Setup::baseline("graphql-fails");
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  \"api graphql\"*) echo 'GraphQL: rate limited' >&2; exit 1 ;;\n  \"api {EVENTS_PATH}\") echo '{}' ;;\n  *) echo '[]' ;;\nesac\n",
        json!([labeled(101, 42, "agent")])
    );
    Setup::fake_gh(&setup.dir, &script);

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(setup.published().len(), 1);
}

#[test]
fn only_the_last_close_of_a_merged_pull_request_is_merged() {
    let setup = Setup::baseline("pr-reopened");
    setup.answer(&[
        (
            EVENTS_PATH,
            json!([
                pull_event(103, 43, "closed", None),
                pull_event(102, 43, "reopened", None),
                pull_event(101, 43, "closed", None),
            ]),
        ),
        (PULLS, pulls(&[pull(43, true, json!([]), json!([]))])),
    ]);

    succeeded(&setup.intake());

    let merged: Vec<_> = setup.published()[1..]
        .iter()
        .map(|m| m.message.data["merged"].clone())
        .collect();
    assert_eq!(merged, [json!(false), json!(true)]);
}

#[test]
fn reviews_publish_before_the_close_that_ends_their_polling() {
    let setup = Setup::baseline("pr-close-order");
    let review = json!([{
        "databaseId": 501,
        "state": "APPROVED",
        "submittedAt": "2026-11-17T12:00:00Z",
        "url": "u",
        "author": null
    }]);
    setup.answer(&[
        (EVENTS_PATH, json!([pull_event(103, 43, "closed", None)])),
        (PULLS, pulls(&[pull(43, true, review, json!([]))])),
    ]);

    succeeded(&setup.intake());

    assert_eq!(
        kinds(&setup.published()[1..]),
        [
            "github.pull_request_review.submitted",
            "github.pull_request.closed"
        ]
    );
}

/// An RFC 3339 time `seconds` ago.
fn ago(seconds: i64) -> serde_json::Value {
    let now = herdr_bus::Timestamp::now().as_second();
    json!(
        herdr_bus::Timestamp::from_second(now - seconds)
            .unwrap()
            .to_string()
    )
}

fn queued() -> serde_json::Value {
    json!({
        "__typename": "CheckRun",
        "databaseId": 601,
        "name": "test",
        "status": "QUEUED",
        "conclusion": null,
        "completedAt": null,
        "url": "u",
        "checkSuite": null
    })
}

#[test]
fn a_head_with_no_check_runs_past_the_grace_period_publishes_a_snapshot_once() {
    let setup = Setup::baseline("pr-no-checks");
    let mut pull = pull(
        43,
        false,
        json!([]),
        json!([{ "__typename": "StatusContext" }]),
    );
    pull["updatedAt"] = json!("2026-11-17T12:00:00Z");
    setup.answer(&[(OPEN_PULLS, open_pulls(&[pull]))]);

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(kinds(&published[1..]), ["github.pull_request.check_runs"]);
    let message = &published[1].message;
    assert_eq!(message.subject.as_deref(), Some("owner/repo#43"));
    let mut data = message.data.clone();
    let time = data.as_object_mut().unwrap().remove("time").unwrap();
    assert_eq!(time.as_str(), Some(message.time.to_string().as_str()));
    assert_eq!(
        data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "url": "https://github.com/owner/repo/pull/43",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "runs": 0,
            "updated_at": "2026-11-17T12:00:00Z",
            "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_head_with_no_check_runs_within_the_grace_period_publishes_nothing_until_it_passes() {
    let setup = Setup::baseline("pr-no-checks-grace");
    let mut pull = pull(43, false, json!([]), json!([]));
    pull["updatedAt"] = ago(4 * 60);
    setup.answer(&[(OPEN_PULLS, open_pulls(std::slice::from_ref(&pull)))]);

    succeeded(&setup.intake());
    assert_eq!(kinds(&setup.published()[1..]), Vec::<&str>::new());

    pull["updatedAt"] = ago(6 * 60);
    setup.answer(&[(OPEN_PULLS, open_pulls(&[pull]))]);
    succeeded(&setup.intake());
    let published = setup.published();
    assert_eq!(kinds(&published[1..]), ["github.pull_request.check_runs"]);
    assert_eq!(published[1].message.data["runs"], 0);
}

#[test]
fn a_check_run_appearing_after_a_snapshot_of_none_publishes_a_snapshot_of_some() {
    let setup = Setup::baseline("pr-queued-check");
    let mut pull = pull(43, false, json!([]), json!([]));
    pull["updatedAt"] = ago(6 * 60);
    setup.answer(&[(OPEN_PULLS, open_pulls(std::slice::from_ref(&pull)))]);
    succeeded(&setup.intake());

    pull["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"] =
        json!({ "totalCount": 1, "nodes": [queued()] });
    setup.answer(&[(OPEN_PULLS, open_pulls(std::slice::from_ref(&pull)))]);
    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    let runs: Vec<_> = published[1..]
        .iter()
        .map(|stored| stored.message.data["runs"].clone())
        .collect();
    assert_eq!(runs, [json!(0), json!(1)]);
}

#[test]
fn a_poll_judges_a_pull_request_on_its_open_snapshot_when_an_event_names_it_too() {
    let setup = Setup::baseline("pr-two-snapshots");
    let mut named = pull(43, false, json!([]), json!([]));
    named["updatedAt"] = ago(6 * 60);
    let mut open = named.clone();
    open["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"] =
        json!({ "totalCount": 1, "nodes": [queued()] });
    setup.answer(&[
        (
            EVENTS_PATH,
            json!([pull_event(103, 43, "labeled", Some("bug"))]),
        ),
        (PULLS, pulls(&[named])),
        (OPEN_PULLS, open_pulls(&[open])),
    ]);

    succeeded(&setup.intake());

    let snapshots: Vec<_> = setup
        .published()
        .iter()
        .filter(|stored| stored.message.kind == "github.pull_request.check_runs")
        .map(|stored| stored.message.data["runs"].clone())
        .collect();
    assert_eq!(snapshots, [json!(1)]);
}

const SCHEDULE: &str = r#"
source = "schedule"
repo = "owner/repo"
at = "09:00"
timezone = "Europe/Paris"
workflow = "implement"
band = "today"
"#;

/// A factory with the `standup` schedule trigger, firing at 09:00 Paris time.
fn scheduled(name: &str) -> Setup {
    let setup = Setup::new(name, &[]);
    write(&setup.dir.join("factory/triggers/standup.toml"), SCHEDULE);
    setup
}

fn tasks(setup: &Setup) -> Vec<Message> {
    setup
        .published()
        .into_iter()
        .map(|stored| stored.message)
        .filter(|message| message.kind == "factory.task.requested")
        .collect()
}

#[test]
fn a_due_schedule_requests_one_task() {
    let setup = scheduled("schedule-due");

    succeeded(&setup.intake_at("2026-11-19T08:30:00Z"));

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        ["schedule.fired", "factory.task.requested"]
    );
    let (source, task) = (&published[0].message, &published[1].message);
    assert_eq!(
        source.data,
        json!({ "trigger": "standup", "scheduled": "2026-11-19T08:00:00Z" })
    );
    assert_eq!(
        task.data,
        json!({
            "workflow": "implement",
            "key": "schedule:standup@2026-11-19T08:00:00Z",
            "band": "today",
            "lane": null,
            "trigger": "standup",
            "source": source.id.to_string(),
            "scheduled": "2026-11-19T08:00:00Z",
            "prompt": null
        })
    );
}

const SUPPORT_DAY: &str = r#"
source = "schedule"
repo = "owner/repo"
at = "09:00"
every = "1h"
until = "12:00"
timezone = "Europe/Paris"
key = "support-day"
prompt = "Sweep the support queue."
workflow = "implement"
"#;

#[test]
fn a_recurring_schedule_fires_every_slot_until_its_end_under_one_key() {
    let setup = scheduled("schedule-until");
    write(
        &setup.dir.join("factory/triggers/standup.toml"),
        SUPPORT_DAY,
    );

    // 10:30, 12:30 and 13:30 in Paris: the 10:00 and 12:00 firings, then none past 12:00.
    for now in [
        "2026-11-19T09:30:00Z",
        "2026-11-19T11:30:00Z",
        "2026-11-19T12:30:00Z",
    ] {
        succeeded(&setup.intake_at(now));
    }

    let tasks = tasks(&setup);
    let fired: Vec<_> = tasks.iter().map(|task| &task.data["scheduled"]).collect();
    assert_eq!(fired, ["2026-11-19T09:00:00Z", "2026-11-19T11:00:00Z"]);
    assert!(
        tasks
            .iter()
            .all(|task| task.data["key"] == "schedule:support-day"
                && task.data["prompt"] == "Sweep the support queue."),
        "{tasks:?}"
    );
}

#[test]
fn a_second_poll_in_the_same_slot_requests_no_task() {
    let setup = scheduled("schedule-same-slot");
    succeeded(&setup.intake_at("2026-11-19T08:30:00Z"));

    succeeded(&setup.intake_at("2026-11-19T08:31:00Z"));

    assert_eq!(tasks(&setup).len(), 1);
}

#[test]
fn a_restart_after_several_missed_slots_requests_one_task() {
    let setup = scheduled("schedule-missed");
    succeeded(&setup.intake_at("2026-11-16T08:30:00Z"));

    succeeded(&setup.intake_at("2026-11-19T10:00:00Z"));

    let keys: Vec<_> = tasks(&setup)
        .into_iter()
        .map(|task| task.data["key"].clone())
        .collect();
    assert_eq!(
        keys,
        [
            "schedule:standup@2026-11-16T08:00:00Z",
            "schedule:standup@2026-11-19T08:00:00Z"
        ]
    );
}

#[test]
fn a_schedule_waits_for_its_time_and_its_days() {
    let setup = scheduled("schedule-days");
    let weekdays = format!("{SCHEDULE}days = [\"mon\", \"tue\", \"wed\", \"thu\", \"fri\"]\n");
    write(&setup.dir.join("factory/triggers/standup.toml"), &weekdays);

    // Saturday 2026-11-21, after 09:00 Paris: the latest firing is Friday's.
    succeeded(&setup.intake_at("2026-11-21T10:00:00Z"));
    // Monday before 09:00 Paris: still Friday's.
    succeeded(&setup.intake_at("2026-11-23T07:59:00Z"));

    let tasks = tasks(&setup);
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].data["key"],
        "schedule:standup@2026-11-20T08:00:00Z"
    );
}

#[test]
fn a_duration_until_ends_at_midnight() {
    let setup = scheduled("schedule-midnight");
    let late = "source = \"schedule\"\nrepo = \"owner/repo\"\nat = \"23:00\"\ndays = [\"fri\"]\nevery = \"1h\"\nuntil = \"2h\"\ntimezone = \"UTC\"\nworkflow = \"implement\"\n";
    write(&setup.dir.join("factory/triggers/standup.toml"), late);

    // Friday 23:30, then Saturday 00:30.
    succeeded(&setup.intake_at("2026-11-20T23:30:00Z"));
    succeeded(&setup.intake_at("2026-11-21T00:30:00Z"));

    let fired: Vec<_> = tasks(&setup)
        .into_iter()
        .map(|task| task.data["scheduled"].clone())
        .collect();
    assert_eq!(fired, ["2026-11-20T23:00:00Z"]);
}

const KEY: &str = "lin_api_secret_test_key";

const VIEW_TRIGGER: &str = r#"
source = "linear.view"
view = "6b3c2f9a1d4e"
on = "issue.entered"
workflow = "implement"
key = "issue.identifier"
band = "today"
"#;

/// What the fake Linear answers to each query: the view's issues, updated issues and comments.
#[derive(Default)]
struct Linear {
    view: Vec<serde_json::Value>,
    issues: Vec<serde_json::Value>,
    comments: Vec<serde_json::Value>,
}

impl Setup {
    /// A factory with only the Linear triggers `triggers`, and a fake Linear with nothing in it.
    fn linear(name: &str, triggers: &[(&str, &str)]) -> Self {
        let setup = Setup::new(name, &[]);
        fs::remove_file(setup.dir.join("factory/triggers/agent-label.toml")).unwrap();
        for (name, trigger) in triggers {
            write(
                &setup.dir.join(format!("factory/triggers/{name}.toml")),
                trigger,
            );
        }
        setup.answer_linear(&Linear::default());
        setup
    }

    /// Makes the fake `curl` answer Linear's GraphQL queries from `linear`, and record its
    /// arguments and standard input.
    fn answer_linear(&self, linear: &Linear) {
        let page = |field: &str, nodes: &[serde_json::Value]| {
            let answer = match field {
                "customView" => json!({ "data": { "customView": { "issues": {
                    "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } } } }),
                field => json!({ "data": { field: {
                    "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } } }),
            };
            let file = self.dir.join(format!("linear-{field}.json"));
            write(&file, &answer.to_string());
            file.display().to_string()
        };
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{dir}/curl-args'\ncat >> '{dir}/curl-stdin'\n\
             case \"$*\" in\n  *'query View'*) cat '{view}' ;;\n  *'query Issues'*) cat '{issues}' ;;\n\
             *'query Comments'*) cat '{comments}' ;;\n  *) echo 'unexpected query' >&2; exit 2 ;;\nesac\n",
            dir = self.dir.display(),
            view = page("customView", &linear.view),
            issues = page("issues", &linear.issues),
            comments = page("comments", &linear.comments),
        );
        let curl = self.dir.join("bin/curl");
        write(&curl, &script);
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Without `LINEAR_API_KEY` in the environment.
    fn intake_without_key(&self) -> Output {
        self.command()
            .env_remove("LINEAR_API_KEY")
            .output()
            .unwrap()
    }
}

fn linear_issue(number: u64) -> serde_json::Value {
    json!({
        "id": format!("issue-{number}"),
        "identifier": format!("ENG-{number}"),
        "title": "Fix the thing",
        "url": format!("https://linear.app/acme/issue/ENG-{number}/fix-the-thing"),
        "createdAt": "2026-11-17T09:00:00.000Z",
        "updatedAt": "2026-11-17T10:00:00.000Z",
        "team": { "key": "ENG" },
        "state": { "name": "Todo" },
        "labels": { "nodes": [{ "name": "agent" }] },
        "creator": { "displayName": "max" },
        "history": { "nodes": [] }
    })
}

/// The data of a Linear view event, without its `time`, which is when the intake polled.
fn timeless(message: &Message) -> serde_json::Value {
    let mut data = message.data.clone();
    data.as_object_mut().unwrap().remove("time");
    data
}

#[test]
fn an_issue_entering_a_view_publishes_the_event_and_one_task_request() {
    let setup = Setup::linear("linear-entered", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        ["linear.issue.entered", "factory.task.requested"]
    );
    let (source, task) = (&published[0].message, &published[1].message);
    assert_eq!(source.subject.as_deref(), Some("ENG-7"));
    assert_eq!(
        timeless(source),
        json!({
            "view": "6b3c2f9a1d4e",
            "issue": "ENG-7",
            "linear_id": "issue-7",
            "title": "Fix the thing",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing",
            "team": "ENG",
            "state": "Todo",
            "labels": ["agent"],
            "baseline": false,
            "triggers": ["agent-ready"]
        })
    );
    assert_eq!(
        task.data,
        json!({
            "workflow": "implement",
            "key": "linear:ENG-7",
            "band": "today",
            "lane": null,
            "trigger": "agent-ready",
            "source": source.id.to_string()
        })
    );
}

#[test]
fn an_issue_leaving_a_view_publishes_issue_left() {
    let setup = Setup::linear("linear-left", &[("agent-ready", VIEW_TRIGGER)]);
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7), linear_issue(8)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(8)],
        ..Linear::default()
    });

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        [
            "linear.issue.entered",
            "linear.issue.entered",
            "linear.issue.left"
        ]
    );
    let left = &published[2].message;
    assert_eq!(left.subject.as_deref(), Some("ENG-7"));
    assert_eq!(left.data["baseline"], false);
    assert_eq!(left.data["triggers"], json!([]));
}

#[test]
fn a_first_linear_poll_is_a_baseline_that_requests_no_task() {
    let setup = Setup::linear("linear-baseline", &[("agent-ready", VIEW_TRIGGER)]);
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(kinds(&published), ["linear.issue.entered"]);
    assert_eq!(published[0].message.data["baseline"], true);
    assert_eq!(published[0].message.data["triggers"], json!([]));
}

#[test]
fn a_backfill_view_trigger_requests_a_task_for_the_first_poll() {
    let trigger = format!("{VIEW_TRIGGER}backfill = true\n");
    let setup = Setup::linear("linear-backfill", &[("agent-ready", &trigger)]);
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    assert_eq!(
        kinds(&setup.published()),
        ["linear.issue.entered", "factory.task.requested"]
    );
}

#[test]
fn a_linear_trigger_without_the_key_fails_and_publishes_nothing() {
    let setup = Setup::linear("linear-no-key", &[("agent-ready", VIEW_TRIGGER)]);

    let output = setup.intake_without_key();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory intake: LINEAR_API_KEY is not set: the Linear triggers need a Linear API key\n"
    );
    assert!(setup.output().is_empty());
}

#[test]
fn the_key_reaches_linear_on_standard_input_only() {
    let setup = Setup::linear("linear-key", &[("agent-ready", VIEW_TRIGGER)]);

    let output = setup.intake();

    succeeded(&output);
    let stdin = fs::read_to_string(setup.dir.join("curl-stdin")).unwrap();
    assert!(stdin.contains(&format!("header = \"Authorization: {KEY}\"")));
    let args = fs::read_to_string(setup.dir.join("curl-args")).unwrap();
    assert!(args.lines().all(|line| line.starts_with("--disable ")));
    assert!(!args.contains(KEY));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(KEY));
}

fn linear_comment(id: &str, issue: u64, time: &str) -> serde_json::Value {
    json!({
        "id": id,
        "body": "Please also cover the empty case.",
        "url": format!("https://linear.app/acme/issue/ENG-{issue}/fix-the-thing#comment-{id}"),
        "createdAt": time,
        "user": { "displayName": "max" },
        "issue": linear_issue(issue)
    })
}

impl Setup {
    /// The rows of `priory inbox` after `priory daemon --once` reads the intake's output.
    fn inbox(&self) -> Vec<Vec<String>> {
        let priory = |args: &[&str]| {
            let output = Command::new(env!("CARGO_BIN_EXE_priory"))
                .args(args)
                .env("HERDR_BUS_DB", self.dir.join("bus.db"))
                .env("PRIORY_DB", self.dir.join("priory.db"))
                .output()
                .unwrap();
            succeeded(&output);
            String::from_utf8(output.stdout).unwrap()
        };
        priory(&[
            "daemon",
            self.dir.join("factory").to_str().unwrap(),
            "--once",
        ]);
        priory(&["inbox"])
            .lines()
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }
}

#[test]
fn a_comment_on_a_tasks_issue_reaches_its_task_in_the_inbox() {
    let setup = Setup::linear("linear-comment", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        comments: vec![linear_comment("comment-1", 7, "2026-11-17T11:00:00.000Z")],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    let comment = setup.published().pop().unwrap().message;
    assert_eq!(comment.kind, "linear.comment.created");
    assert_eq!(
        comment.data,
        json!({
            "issue": "ENG-7",
            "linear_id": "comment-1",
            "title": "Fix the thing",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-comment-1",
            "team": "ENG",
            "state": "Todo",
            "labels": ["agent"],
            "body": "Please also cover the empty case.",
            "actor": "max",
            "time": "2026-11-17T11:00:00.000Z",
            "baseline": false,
            "triggers": []
        })
    );
    let routed: Vec<[String; 3]> = setup
        .inbox()
        .into_iter()
        .filter(|row| row[1] != "factory.intake.baseline")
        .map(|row| [row[1].clone(), row[3].clone(), row[4].clone()])
        .collect();
    assert_eq!(
        routed,
        [
            ["linear.issue.entered", "-", "ignored"],
            ["factory.task.requested", "1", "created"],
            ["linear.comment.created", "1", "kept"],
        ]
        .map(|row| row.map(str::to_owned))
    );
}

const STATE_TRIGGER: &str = r#"
source = "linear"
on = "issue.state_changed"
filter = { team = "ENG", state = "In Progress" }
workflow = "implement"
"#;

fn moved(number: u64, id: &str, to: &str, time: &str) -> serde_json::Value {
    let mut issue = linear_issue(number);
    issue["createdAt"] = json!("2026-11-17T09:00:00.000Z");
    issue["updatedAt"] = json!(time);
    issue["state"] = json!({ "name": to });
    issue["history"] = json!({ "nodes": [{
        "id": id,
        "createdAt": time,
        "fromState": { "name": "Todo" },
        "toState": { "name": to },
        "actor": { "displayName": "max" }
    }] });
    issue
}

#[test]
fn a_state_change_that_matches_a_filter_requests_a_task_once() {
    let setup = Setup::linear("linear-state", &[("started", STATE_TRIGGER)]);
    setup.answer_linear(&Linear {
        issues: vec![linear_issue(1)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        issues: vec![
            moved(7, "history-7", "In Progress", "2026-11-17T11:00:00.000Z"),
            moved(8, "history-8", "Done", "2026-11-17T11:30:00.000Z"),
        ],
        ..Linear::default()
    });

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        [
            "linear.issue.created",
            "linear.issue.state_changed",
            "factory.task.requested",
            "linear.issue.state_changed"
        ]
    );
    assert_eq!(published[0].message.data["baseline"], true);
    let (change, task) = (&published[1].message, &published[2].message);
    assert_eq!(
        change.data,
        json!({
            "issue": "ENG-7",
            "linear_id": "history-7",
            "title": "Fix the thing",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing",
            "team": "ENG",
            "state": "In Progress",
            "labels": ["agent"],
            "from": "Todo",
            "to": "In Progress",
            "actor": "max",
            "time": "2026-11-17T11:00:00.000Z",
            "baseline": false,
            "triggers": ["started"]
        })
    );
    assert_eq!(task.data["key"], "linear:ENG-7");
    assert_eq!(published[3].message.data["triggers"], json!([]));
}

const COMMENT_TRIGGER: &str = r#"
source = "linear"
on = "comment.created"
workflow = "implement"
"#;

#[test]
fn a_restart_requests_no_task_for_a_baseline_comment_read_again() {
    let setup = Setup::linear("linear-restart-baseline", &[("commented", COMMENT_TRIGGER)]);
    setup.answer_linear(&Linear {
        comments: vec![linear_comment("comment-1", 7, "2026-11-17T11:00:00.000Z")],
        ..Linear::default()
    });
    succeeded(&setup.intake());

    succeeded(&setup.intake());

    assert_eq!(kinds(&setup.published()), ["linear.comment.created"]);
}

#[test]
fn a_restart_requests_the_task_a_crash_left_out_for_a_view() {
    let setup = Setup::linear("linear-crash", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    let entered = setup.published()[0].message.clone();
    let crashed = Setup::linear("linear-crash-replay", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&crashed.intake());
    seed(&crashed, &entered);

    succeeded(&crashed.intake());

    assert_eq!(
        kinds(&crashed.published()),
        ["linear.issue.entered", "factory.task.requested"]
    );
}

#[test]
fn a_failing_linear_query_reports_the_error_without_the_key() {
    let setup = Setup::linear("linear-fails", &[("agent-ready", VIEW_TRIGGER)]);
    let curl = setup.dir.join("bin/curl");
    let script = format!(
        "#!/bin/sh\ncat > /dev/null\necho \"> Authorization: {KEY}\" >&2\necho 'curl: (22) 401' >&2\nexit 22\n"
    );
    write(&curl, &script);

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("curl: (22) 401"), "{stderr}");
    assert!(!stderr.contains(KEY), "{stderr}");
    assert!(setup.published().is_empty());
}
