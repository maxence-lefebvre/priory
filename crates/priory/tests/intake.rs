#[allow(dead_code)]
mod harness;

mod conform;

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
        priory_runtime::runtime::remove_sealed(&dir);
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
        let mut command = harness::priory();
        command
            .args(["intake", "--once"])
            .arg(self.dir.join("factory"))
            .env("PATH", path)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"));
        command
    }

    /// The source events and task requests on `factory.intake`, oldest first.
    fn published(&self) -> Vec<Stored> {
        let mut messages = self.output();
        messages.retain(|m| m.message.kind != "factory.intake.baseline");
        messages
    }

    /// Everything on `factory.intake`, oldest first, each checked against its schema.
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
        for stored in &messages {
            conform::conforms(&serde_json::to_value(&stored.message).unwrap());
        }
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

impl Setup {
    /// `priory desk --json` on the intake's state file.
    fn desk(&self) -> serde_json::Value {
        let output = harness::priory()
            .args(["desk", "--json"])
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .output()
            .unwrap();
        succeeded(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

/// The `what` of each report item on `desk`.
fn reports(desk: &serde_json::Value) -> Vec<String> {
    desk["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "report")
        .map(|item| item["what"].as_str().unwrap().to_owned())
        .collect()
}

/// `PRIORY_NOW` for a poll `minutes` before the real time, so the desk's 5-minute check still sees
/// the last poll as recent.
fn minutes_ago(minutes: i64) -> String {
    let now = herdr_bus::Timestamp::now().as_second() - minutes * 60;
    herdr_bus::Timestamp::from_second(now).unwrap().to_string()
}

#[test]
fn a_failing_poll_reports_once_past_15_minutes_and_a_good_poll_clears_the_report_and_banner() {
    let setup = Setup::new("gh-fails-desk", &[]);
    Setup::fake_gh(
        &setup.dir,
        "#!/bin/sh\necho 'HTTP 401: Bad credentials' >&2\nexit 1\n",
    );
    let error = format!("gh api {EVENTS_PATH} failed: HTTP 401: Bad credentials");

    setup.intake_at(&minutes_ago(16));
    setup.intake_at(&minutes_ago(1));
    setup.intake_at(&minutes_ago(0));

    let desk = setup.desk();
    assert_eq!(reports(&desk), [format!("intake: github: {error}")]);
    let banner = desk["intake"].as_str().unwrap();
    assert!(
        banner.starts_with("the intake fails to poll github since ")
            && banner.ends_with(&format!(": {error}")),
        "{banner}"
    );
    setup.answer(&[]);
    succeeded(&setup.intake());
    let desk = setup.desk();
    assert_eq!(desk["intake"], serde_json::Value::Null);
    assert!(reports(&desk).is_empty(), "{desk}");
}

#[test]
fn a_poll_failing_for_less_than_15_minutes_posts_no_alarm_and_shows_a_calm_banner() {
    let setup = Setup::new("gh-fails-briefly", &[]);
    Setup::fake_gh(
        &setup.dir,
        "#!/bin/sh\necho 'HTTP 401: Bad credentials' >&2\nexit 1\n",
    );
    let error = format!("gh api {EVENTS_PATH} failed: HTTP 401: Bad credentials");
    let since = minutes_ago(14);

    setup.intake_at(&since);
    setup.intake_at(&minutes_ago(0));

    let desk = setup.desk();
    assert!(reports(&desk).is_empty(), "{desk}");
    assert_eq!(desk["next"], serde_json::Value::Null, "{desk}");
    assert_eq!(
        desk["intake"],
        format!("the intake retries github since {since}: {error}")
    );
    setup.answer(&[]);
    succeeded(&setup.intake());
    assert_eq!(setup.desk()["intake"], serde_json::Value::Null);
}

#[test]
fn intake_alarm_after_sets_how_long_a_poll_fails_before_it_reports() {
    let setup = Setup::new("gh-fails-alarm-after", &[]);
    write(
        &setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\n\n[intake]\nalarm_after = \"2m\"\n",
    );
    Setup::fake_gh(
        &setup.dir,
        "#!/bin/sh\necho 'HTTP 401: Bad credentials' >&2\nexit 1\n",
    );

    setup.intake_at(&minutes_ago(3));
    let calm = setup.desk()["revision"].as_i64().unwrap();
    setup.intake_at(&minutes_ago(0));

    let alarmed = setup.desk()["revision"].as_i64().unwrap();
    assert!(alarmed > calm, "{alarmed} after {calm}");
    let error = format!("gh api {EVENTS_PATH} failed: HTTP 401: Bad credentials");
    assert_eq!(reports(&setup.desk()), [format!("intake: github: {error}")]);
}

impl Setup {
    /// Makes the fake `gh` fail its first `failures` calls whose arguments start with `api
    /// {path}` with `stderr`, then answer every call from `answers`. `calls-{path}` counts them.
    fn fail_first(
        &self,
        path: &str,
        failures: u32,
        stderr: &str,
        answers: &[(&str, serde_json::Value)],
    ) {
        self.answer(answers);
        let gh = self.dir.join("bin/gh");
        let answering = fs::read_to_string(&gh).unwrap();
        let count = self.dir.join(format!("calls-{}", path.replace('/', "-")));
        let script = answering.replacen(
            "#!/bin/sh\n",
            &format!(
                "#!/bin/sh\ncase \"$*\" in\n  \"api {path}\"*)\n    n=$(($(cat '{count}' 2>/dev/null || echo 0) + 1)); echo $n > '{count}'\n    if [ $n -le {failures} ]; then echo '{stderr}' >&2; exit 1; fi ;;\nesac\n",
                count = count.display()
            ),
            1,
        );
        Setup::fake_gh(&self.dir, &script);
    }

    /// How many times the fake `gh` of `fail_first` was called for `path`.
    fn calls(&self, path: &str) -> u32 {
        let count = self.dir.join(format!("calls-{}", path.replace('/', "-")));
        fs::read_to_string(count).map_or(0, |count| count.trim().parse().unwrap())
    }
}

#[test]
fn a_504_from_the_graphql_query_is_retried_in_the_same_poll() {
    let setup = Setup::baseline("gh-504-once");
    setup.fail_first(
        "graphql",
        1,
        "gh: HTTP 504",
        &[(EVENTS_PATH, json!([labeled(101, 42, "agent")]))],
    );

    succeeded(&setup.intake());

    assert_eq!(setup.calls("graphql"), 2);
    assert_eq!(
        kinds(&setup.published()[1..]),
        ["github.issues.labeled", "factory.task.requested"]
    );
    assert!(reports(&setup.desk()).is_empty());
    assert_eq!(setup.desk()["intake"], serde_json::Value::Null);
}

#[test]
fn a_graphql_timeout_and_a_rest_502_are_retried_too() {
    let setup = Setup::baseline("gh-timeout-once");
    setup.fail_first(
        "graphql",
        1,
        "GraphQL: Something went wrong while executing your query. This may be the result of a timeout, or it could be a GitHub bug.",
        &[],
    );
    succeeded(&setup.intake());
    assert_eq!(setup.calls("graphql"), 2);

    setup.fail_first(EVENTS_PATH, 1, "HTTP 502: Bad Gateway", &[]);
    succeeded(&setup.intake());
    assert_eq!(setup.calls(EVENTS_PATH), 2);
}

#[test]
fn a_504_that_outlasts_two_retries_fails_the_poll() {
    let setup = Setup::baseline("gh-504-thrice");
    setup.fail_first("graphql", 3, "gh: HTTP 504", &[]);

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(setup.calls("graphql"), 3);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("gh api graphql failed: gh: HTTP 504"),
        "{output:?}"
    );
}

#[test]
fn a_client_error_is_not_retried() {
    let setup = Setup::baseline("gh-404-no-retry");
    setup.fail_first(EVENTS_PATH, 1, "HTTP 404: Not Found", &[]);

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(setup.calls(EVENTS_PATH), 1);
}

#[test]
fn a_failed_poll_and_its_recovery_move_the_revision_and_a_repeated_good_poll_does_not() {
    let setup = Setup::new("intake-revision", &[]);
    succeeded(&setup.intake());
    let revision = || setup.desk()["revision"].as_i64().unwrap();
    let polled = revision();

    succeeded(&setup.intake());
    let repeated = revision();
    Setup::fake_gh(&setup.dir, "#!/bin/sh\necho 'HTTP 502' >&2\nexit 1\n");
    setup.intake();
    let failed = revision();
    setup.answer(&[]);
    succeeded(&setup.intake());
    let recovered = revision();

    assert_eq!(repeated, polled);
    assert!(failed > repeated, "{failed} after {repeated}");
    assert!(recovered > failed, "{recovered} after {failed}");
}

#[test]
fn an_intake_that_has_not_polled_for_five_minutes_shows_a_banner() {
    let setup = Setup::new("intake-stopped", &[]);

    succeeded(&setup.intake_at("2026-01-01T00:00:00Z"));

    assert_eq!(
        setup.desk()["intake"],
        "the intake has not polled since 2026-01-01T00:00:00Z: waits on GitHub, Linear and Slack cannot fire"
    );
}

#[test]
fn a_gh_call_past_the_command_timeout_is_killed_and_the_poll_fails() {
    let setup = Setup::new("gh-hangs", &[]);
    write(
        &setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\ncommand_timeout = \"1s\"\n",
    );
    // A child that ignores TERM keeps stdout open, so only killing the whole group ends the call.
    Setup::fake_gh(
        &setup.dir,
        "#!/bin/sh\n(trap '' TERM; sleep 60) & sleep 60\n",
    );
    let started = std::time::Instant::now();

    let output = setup.intake();

    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "took {:?}: {output:?}",
        started.elapsed()
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("priory intake: gh api {EVENTS_PATH} timed out after 1s\n")
    );
    assert!(setup.published().is_empty());
}

#[test]
fn a_failing_repository_still_lets_the_others_publish() {
    let setup = Setup::new("gh-one-fails", &[]);
    write(
        &setup.dir.join("factory/triggers/other.toml"),
        &TRIGGER.replace("owner/repo", "other/repo"),
    );
    let events = setup.dir.join("other-events.json");
    write(
        &events,
        &json!([labeled(100, 41, "agent")])
            .to_string()
            .replace("owner/repo", "other/repo"),
    );
    Setup::fake_gh(
        &setup.dir,
        &format!(
            "#!/bin/sh\ncase \"$*\" in\n  *owner/repo*) echo 'HTTP 404: Not Found' >&2; exit 1 ;;\n  *other/repo/issues/events*) cat '{}' ;;\n  *) echo '[]' ;;\nesac\n",
            events.display()
        ),
    );

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("priory intake: gh api {EVENTS_PATH} failed: HTTP 404: Not Found\n")
    );
    let subjects: Vec<_> = setup
        .published()
        .into_iter()
        .map(|stored| stored.message.subject.unwrap_or_default())
        .collect();
    assert_eq!(subjects, ["other/repo#41"]);
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
    let source = labeled_source(101, false, json!(["agent-label"]));
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
    labeled_source(github_id, true, triggers)
}

/// A `github.issues.labeled` event of issue 42, as the intake publishes it.
fn labeled_source(github_id: u64, baseline: bool, triggers: serde_json::Value) -> Message {
    let url = "https://github.com/owner/repo/issues/42";
    Message::new(
        "github.issues.labeled",
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "title": "Issue 42",
            "url": url,
            "label": "agent",
            "actor": "someone",
            "time": "2026-11-17T10:00:00Z",
            "github_id": github_id,
            "baseline": baseline,
            "triggers": triggers
        }),
    )
    .subject("owner/repo#42")
    .link("issue", url)
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
        "state": if merged { "MERGED" } else { "OPEN" },
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
    let mut merged = pull(43, true, json!([]), json!([]));
    merged["mergeCommit"] = json!({ "oid": "def456" });
    merged["body"] = json!("Reverts owner/repo#7");
    setup.answer(&[
        (
            EVENTS_PATH,
            json!([
                pull_event(103, 43, "closed", None),
                pull_event(102, 43, "merged", None),
            ]),
        ),
        (PULLS, pulls(&[merged])),
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
    assert_eq!(
        (&data["merge_commit"], &data["body"]),
        (&json!("def456"), &json!("Reverts owner/repo#7"))
    );
}

#[test]
fn a_submitted_review_and_a_completed_check_publish_once() {
    let setup = Setup::baseline("pr-review-check");
    let reviews = json!([
        {
            "databaseId": 501,
            "state": "APPROVED",
            "body": "Ship it.",
            "submittedAt": "2026-11-17T12:00:00Z",
            "url": "https://github.com/owner/repo/pull/43#pullrequestreview-501",
            "commit": { "oid": "fff000" },
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
            "body": "Ship it.",
            "commit": "fff000",
            "actor": "reviewer",
            "open": true,
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
    let url = "https://github.com/owner/repo/pull/43";
    let source = Message::new(
        "github.pull_request.labeled",
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "title": "Pull request 43",
            "url": url,
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "label": "agent",
            "actor": "someone",
            "time": "2026-11-17T10:00:00Z",
            "github_id": 101,
            "baseline": false,
            "triggers": ["agent-label"]
        }),
    )
    .subject("owner/repo#43")
    .link("pull_request", url);
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

const REVIEW_COMMENTS: &str =
    "repos/owner/repo/pulls/comments?sort=created&direction=desc&per_page=100";

/// A review thread comment as `pulls/comments` lists it, by `login` of GitHub user `type`.
fn review_comment(id: u64, reply_to: Option<u64>, login: &str, kind: &str) -> serde_json::Value {
    json!({
        "id": id,
        "body": "Rename this.\nIt reads better.",
        "html_url": format!("https://github.com/owner/repo/pull/43#discussion_r{id}"),
        "created_at": "2026-11-17T12:10:00Z",
        "path": "src/lib.rs",
        "line": 12,
        "in_reply_to_id": reply_to,
        "pull_request_url": "https://api.github.com/repos/owner/repo/pulls/43",
        "user": { "login": login, "type": kind }
    })
}

const ISSUE_COMMENTS: &str =
    "repos/owner/repo/issues/comments?sort=created&direction=desc&per_page=100";

/// A conversation comment as `issues/comments` lists it, on issue or pull request `number`.
fn issue_comment(id: u64, number: u64, page: &str, kind: &str) -> serde_json::Value {
    json!({
        "id": id,
        "body": "Also handle the empty case.",
        "html_url": format!("https://github.com/owner/repo/{page}/{number}#issuecomment-{id}"),
        "issue_url": format!("https://api.github.com/repos/owner/repo/issues/{number}"),
        "created_at": "2026-11-17T12:00:00Z",
        "user": { "login": "ana", "type": kind }
    })
}

#[test]
fn a_comment_on_an_issue_and_its_close_publish_once_for_the_issue_and_bots_none() {
    let setup = Setup::baseline("issue-comment-close");
    let mut closed = labeled(102, 42, "agent");
    closed["event"] = json!("closed");
    closed["label"] = json!(null);
    closed["created_at"] = json!("2026-11-17T12:30:00Z");
    let mut after_close = issue_comment(904, 42, "issues", "User");
    after_close["created_at"] = json!("2026-11-17T12:45:00Z");
    setup.answer(&[
        (EVENTS_PATH, json!([closed])),
        (
            ISSUE_COMMENTS,
            json!([
                issue_comment(903, 42, "issues", "Bot"),
                // Pull request comments come with the pull request's own query.
                issue_comment(902, 43, "pull", "User"),
                after_close,
                issue_comment(901, 42, "issues", "User"),
            ]),
        ),
    ]);

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    // Oldest first with the close, so a comment after the close never arrives before it.
    assert_eq!(
        kinds(&published[1..]),
        [
            "github.issues.comment",
            "github.issues.closed",
            "github.issues.comment"
        ]
    );
    assert_eq!(published[3].message.data["github_id"], 904);
    assert_eq!(
        published[1].message.subject.as_deref(),
        Some("owner/repo#42")
    );
    assert_eq!(
        published[1].message.data,
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "url": "https://github.com/owner/repo/issues/42#issuecomment-901",
            "actor": "ana",
            "body": "Also handle the empty case.",
            "time": "2026-11-17T12:00:00Z",
            "github_id": 901,
            "baseline": false,
            "triggers": []
        })
    );
    assert_eq!(
        published[2].message.data,
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "title": "Fix the thing",
            "url": "https://github.com/owner/repo/issues/42",
            "actor": "max",
            "time": "2026-11-17T12:30:00Z",
            "github_id": 102,
            "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_comment_and_a_thread_comment_on_a_watched_pull_request_publish_once_and_bots_none() {
    let setup = Setup::baseline("pr-comments");
    let mut open = pull(43, false, json!([]), json!([]));
    open["comments"] = json!({ "nodes": [
        {
            "databaseId": 701,
            "body": "Can you split this?",
            "url": "https://github.com/owner/repo/pull/43#issuecomment-701",
            "createdAt": "2026-11-17T12:00:00Z",
            "author": { "login": "ana", "__typename": "User" }
        },
        {
            "databaseId": 702,
            "body": "Coverage: 80%",
            "url": "https://github.com/owner/repo/pull/43#issuecomment-702",
            "createdAt": "2026-11-17T12:01:00Z",
            "author": { "login": "codecov", "__typename": "Bot" }
        }
    ] });
    setup.answer(&[
        (OPEN_PULLS, open_pulls(&[open])),
        (
            REVIEW_COMMENTS,
            json!([
                review_comment(803, None, "github-actions[bot]", "Bot"),
                review_comment(802, Some(801), "ana", "User"),
                // Pull request 44 is neither open nor named by an event.
                json!({ "pull_request_url": "https://api.github.com/repos/owner/repo/pulls/44" }),
            ]),
        ),
    ]);

    succeeded(&setup.intake());
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published[1..]),
        [
            "github.pull_request.comment",
            "github.pull_request_review_comment"
        ]
    );
    assert_eq!(
        published[1].message.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "url": "https://github.com/owner/repo/pull/43#issuecomment-701",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "actor": "ana",
            "open": true,
            "body": "Can you split this?",
            "time": "2026-11-17T12:00:00Z",
            "github_id": 701,
            "baseline": false,
            "triggers": []
        })
    );
    assert_eq!(
        published[2].message.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "url": "https://github.com/owner/repo/pull/43#discussion_r802",
            "head_branch": "feature",
            "head_sha": "abc123",
            "base_branch": "main",
            "actor": "ana",
            "open": true,
            "body": "Rename this.\nIt reads better.",
            "path": "src/lib.rs",
            "line": 12,
            "thread": 801,
            "time": "2026-11-17T12:10:00Z",
            "github_id": 802,
            "baseline": false,
            "triggers": []
        })
    );
}

/// The task requests on `factory.intake`, as (trigger, key), oldest first.
fn requests(setup: &Setup) -> Vec<(String, String)> {
    setup
        .published()
        .iter()
        .filter(|stored| stored.message.kind == "factory.task.requested")
        .map(|stored| {
            let data = &stored.message.data;
            (
                data["trigger"].as_str().unwrap().to_owned(),
                data["key"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// A conversation comment as the pull request query lists it.
fn pull_comment(id: u64, login: &str, kind: &str, association: &str) -> serde_json::Value {
    json!({
        "databaseId": id,
        "body": "Why this name?",
        "url": format!("https://github.com/owner/repo/pull/43#issuecomment-{id}"),
        "createdAt": "2026-11-17T12:00:00Z",
        "author": { "login": login, "__typename": kind },
        "authorAssociation": association
    })
}

/// A comment by `max`, the login `gh` runs as, that holds the agent comment marker.
fn agent_comment(id: u64) -> serde_json::Value {
    let mut comment = pull_comment(id, "max", "User", "OWNER");
    comment["body"] = json!("Fixed in abc123.\n\n<!-- agent-comment -->");
    comment
}

#[test]
fn a_comment_trigger_requests_a_task_for_a_comment_on_its_author_s_pull_request() {
    let setup = Setup::baseline("pr-comment-trigger");
    pr_trigger(
        &setup,
        "my-prs",
        "on = \"pull_request.comment\"\nfilter = { author = \"max\" }\n",
    );
    pr_trigger(
        &setup,
        "my-threads",
        "on = \"pull_request_review_comment\"\nfilter = { author = \"max\" }\n",
    );
    let mut mine = pull(43, false, json!([]), json!([]));
    mine["author"] = json!({ "login": "max" });
    mine["comments"] = json!({ "nodes": [
        pull_comment(701, "ana", "User", "MEMBER"),
        // Someone without write access starts nothing.
        pull_comment(703, "mallory", "User", "NONE"),
        // The login `gh` runs as, which agents post as too, starts nothing with the marker.
        agent_comment(705),
        // Without it, the comment is the person's.
        pull_comment(706, "max", "User", "OWNER"),
    ] });
    let mut theirs = pull(44, false, json!([]), json!([]));
    theirs["author"] = json!({ "login": "dev" });
    let mut other = pull_comment(702, "ana", "User", "MEMBER");
    other["url"] = json!("https://github.com/owner/repo/pull/44#issuecomment-702");
    theirs["comments"] = json!({ "nodes": [other] });
    // A comment on a merged pull request starts nothing.
    let mut merged = pull(45, true, json!([]), json!([]));
    merged["author"] = json!({ "login": "max" });
    let mut late = pull_comment(704, "ana", "User", "MEMBER");
    late["url"] = json!("https://github.com/owner/repo/pull/45#issuecomment-704");
    merged["comments"] = json!({ "nodes": [late] });
    setup.answer(&[
        ("user", json!({ "login": "max" })),
        (OPEN_PULLS, open_pulls(&[mine, theirs, merged])),
        (
            REVIEW_COMMENTS,
            json!([review_comment(802, None, "ana", "User")]),
        ),
    ]);

    succeeded(&setup.intake());

    assert_eq!(
        requests(&setup),
        [
            ("my-prs".to_owned(), "github:owner/repo#43".to_owned()),
            ("my-prs".to_owned(), "github:owner/repo#43".to_owned()),
            ("my-threads".to_owned(), "github:owner/repo#43".to_owned()),
        ]
    );
    let comment = setup
        .published()
        .into_iter()
        .find(|stored| stored.message.data["github_id"] == 701)
        .unwrap();
    assert_eq!(comment.message.data["author"], "max");
    let own = setup
        .published()
        .into_iter()
        .find(|stored| stored.message.data["github_id"] == 705)
        .unwrap();
    assert_eq!(own.message.data["own"], true);
    assert_eq!(own.message.data["agent"], true);
    assert_eq!(own.message.data["triggers"], json!([]));
    let person = setup
        .published()
        .into_iter()
        .find(|stored| stored.message.data["github_id"] == 706)
        .unwrap();
    assert_eq!(person.message.data["own"], true);
    assert_eq!(person.message.data["agent"], false);
    let ana = setup
        .published()
        .into_iter()
        .find(|stored| stored.message.data["github_id"] == 701)
        .unwrap();
    assert_eq!(ana.message.data["agent"], false);
}

#[test]
fn a_bot_review_starts_a_task_only_when_the_commenter_filter_names_the_bot() {
    let setup = Setup::baseline("pr-bot-review");
    pr_trigger(
        &setup,
        "my-reviews",
        "on = \"pull_request_review.submitted\"\nfilter = { author = \"max\" }\n",
    );
    pr_trigger(
        &setup,
        "bot-reviews",
        "on = \"pull_request_review.submitted\"\nfilter = { author = \"max\", commenter = \"rayon-bot\" }\n",
    );
    let review = |id: u64, login: &str, kind: &str| {
        json!({
            "databaseId": id,
            "state": "COMMENTED",
            "body": "Looks risky.",
            "submittedAt": "2026-11-17T12:00:00Z",
            "url": format!("https://github.com/owner/repo/pull/43#pullrequestreview-{id}"),
            "commit": { "oid": "abc123" },
            "author": { "login": login, "__typename": kind },
            "authorAssociation": "MEMBER"
        })
    };
    let mut mine = pull(
        43,
        false,
        json!([review(501, "rayon-bot", "Bot"), review(502, "ana", "User")]),
        json!([]),
    );
    mine["author"] = json!({ "login": "max" });
    setup.answer(&[(OPEN_PULLS, open_pulls(&[mine]))]);

    succeeded(&setup.intake());

    assert_eq!(
        requests(&setup),
        [
            ("bot-reviews".to_owned(), "github:owner/repo#43".to_owned()),
            ("my-reviews".to_owned(), "github:owner/repo#43".to_owned()),
        ]
    );
    let bot = setup
        .published()
        .into_iter()
        .find(|stored| stored.message.data["github_id"] == 501)
        .unwrap();
    assert_eq!(bot.message.data["bot"], true);
    assert_eq!(bot.message.data["triggers"], json!(["bot-reviews"]));
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
fn a_schedule_triggers_repository_is_polled_without_a_github_trigger() {
    let setup = Setup::new(
        "schedule-repo",
        &[(
            "repos/other/repo/issues/events",
            json!([labeled(100, 41, "bug")]),
        )],
    );
    fs::remove_file(setup.dir.join("factory/triggers/agent-label.toml")).unwrap();
    write(
        &setup.dir.join("factory/triggers/standup.toml"),
        &SCHEDULE.replace("owner/repo", "other/repo"),
    );

    succeeded(&setup.intake_at("2026-11-19T07:00:00Z"));

    let repos: Vec<_> = setup
        .output()
        .into_iter()
        .filter(|stored| stored.message.kind == "factory.intake.baseline")
        .map(|stored| stored.message.data["repo"].clone())
        .collect();
    assert!(repos.contains(&json!("other/repo")), "{repos:?}");
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

#[test]
fn a_trigger_lane_goes_into_its_task_request() {
    let setup = Setup::baseline("trigger-lane");
    write(
        &setup.dir.join("factory/triggers/agent-label.toml"),
        &format!("{TRIGGER}lane = \"3D/mobile\"\n"),
    );
    setup.answer(&[(EVENTS_PATH, json!([labeled(101, 42, "agent")]))]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(published[2].message.data["lane"], "3D/mobile");
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
    /// The issues that the updated issues block, as the `Blocked` query reads them by id.
    blocked: Vec<serde_json::Value>,
    /// The view's filter, as Linear reports it.
    filter: serde_json::Value,
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
        let page = |name: &str, field: &str, nodes: &[serde_json::Value]| {
            let answer = match field {
                "customView" => {
                    json!({ "data": { "customView": { "filterData": linear.filter, "issues": {
                    "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } } } })
                }
                field => json!({ "data": { field: {
                    "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } } } }),
            };
            let file = self.dir.join(format!("linear-{name}.json"));
            write(&file, &answer.to_string());
            file.display().to_string()
        };
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{dir}/curl-args'\ncat >> '{dir}/curl-stdin'\n\
             case \"$*\" in\n  *'query View'*) cat '{view}' ;;\n  *'query Issues'*) cat '{issues}' ;;\n\
             *'query Comments'*) cat '{comments}' ;;\n  *'query Blocked'*) cat '{blocked}' ;;\n  *) echo 'unexpected query' >&2; exit 2 ;;\nesac\n",
            dir = self.dir.display(),
            view = page("customView", "customView", &linear.view),
            issues = page("issues", "issues", &linear.issues),
            comments = page("comments", "comments", &linear.comments),
            blocked = page("blocked", "issues", &linear.blocked),
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
        "history": { "nodes": [] },
        "inverseRelations": { "nodes": [], "pageInfo": { "hasNextPage": false } }
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
            "project": null,
            "project_id": null,
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
            "team": "ENG",
            "project": null,
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
        "priory intake: LINEAR_API_KEY is not set: the Linear triggers and waits need a Linear API key\n"
    );
    assert!(setup.output().is_empty());
    let banner = setup.desk()["intake"].clone();
    assert!(
        banner.as_str().is_some_and(|banner| banner
            .starts_with("the intake fails to poll linear since ")
            && banner.ends_with(
                ": LINEAR_API_KEY is not set: the Linear triggers and waits need a Linear API key"
            )),
        "{banner}"
    );
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
            let output = harness::priory()
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
            "project": null,
            "project_id": null,
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
        "toState": { "name": to, "type": "started" },
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
            "project": null,
            "project_id": null,
            "state": "In Progress",
            "labels": ["agent"],
            "blockers": [],
            "from": "Todo",
            "to": "In Progress",
            "to_type": "started",
            "duplicate_of": null,
            "actor": "max",
            "time": "2026-11-17T11:00:00.000Z",
            "baseline": false,
            "triggers": ["started"]
        })
    );
    assert_eq!(task.data["key"], "linear:ENG-7");
    assert_eq!(published[3].message.data["triggers"], json!([]));
}

#[test]
fn a_move_to_duplicate_carries_its_state_type_and_the_issue_it_duplicates() {
    let setup = Setup::linear("linear-duplicate", &[("started", STATE_TRIGGER)]);
    setup.answer_linear(&Linear {
        issues: vec![linear_issue(1)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    let mut issue = moved(7, "history-7", "Duplicate", "2026-11-17T11:00:00.000Z");
    issue["history"]["nodes"][0]["toState"]["type"] = json!("canceled");
    issue["relations"] = json!({ "nodes": [
        { "type": "blocks", "relatedIssue": { "id": "issue-3", "identifier": "ENG-3" } },
        { "type": "duplicate", "relatedIssue": { "id": "issue-9", "identifier": "ENG-9" } }
    ] });
    setup.answer_linear(&Linear {
        issues: vec![issue],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    let published = setup.published();
    let change = published
        .iter()
        .find(|published| published.message.kind == "linear.issue.state_changed")
        .unwrap();
    assert_eq!(
        [
            &change.message.data["to"],
            &change.message.data["to_type"],
            &change.message.data["duplicate_of"]
        ],
        [&json!("Duplicate"), &json!("canceled"), &json!("ENG-9")]
    );
}

fn view_with(filter: serde_json::Value, issues: &[u64]) -> Linear {
    Linear {
        view: issues.iter().map(|number| linear_issue(*number)).collect(),
        filter,
        ..Linear::default()
    }
}

#[test]
fn a_view_whose_filter_changed_publishes_one_changed_event_and_no_entered_or_left() {
    let setup = Setup::linear("linear-refiltered", &[("agent-ready", VIEW_TRIGGER)]);
    let todo = json!({ "state": { "name": { "eq": "Todo" } } });
    setup.answer_linear(&view_with(todo.clone(), &[7]));
    succeeded(&setup.intake());
    let started = json!({ "state": { "name": { "eq": "In Progress" } } });
    setup.answer_linear(&view_with(started, &[8, 9]));

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        ["linear.issue.entered", "linear.view.changed"]
    );
    let changed = &published[1].message;
    assert_eq!(changed.subject.as_deref(), Some("6b3c2f9a1d4e"));
    assert_eq!(
        timeless(changed),
        json!({
            "view": "6b3c2f9a1d4e",
            "entered": ["ENG-8", "ENG-9"],
            "left": ["ENG-7"],
            "baseline": false,
            "triggers": []
        })
    );
    setup.inbox();
    let desk = harness::priory()
        .arg("desk")
        .env("PRIORY_DB", setup.dir.join("priory.db"))
        .output()
        .unwrap();
    succeeded(&desk);
    let desk = String::from_utf8(desk.stdout).unwrap();
    let (desk, _) = desk.split_once("\nsince you looked").unwrap_or((&desk, ""));
    let reports = desk
        .matches(
            "linear: view 6b3c2f9a1d4e changed its filter: 2 issues entered and 1 left, with no task requested",
        )
        .count();
    assert_eq!(reports, 1, "{desk}");
}

#[test]
fn a_view_with_the_same_filter_still_publishes_entered_and_left() {
    let setup = Setup::linear("linear-same-filter", &[("agent-ready", VIEW_TRIGGER)]);
    let todo = json!({ "state": { "name": { "eq": "Todo" } } });
    setup.answer_linear(&view_with(todo.clone(), &[7]));
    succeeded(&setup.intake());
    setup.answer_linear(&view_with(todo, &[8]));

    succeeded(&setup.intake());

    assert_eq!(
        kinds(&setup.published()),
        [
            "linear.issue.entered",
            "linear.issue.entered",
            "factory.task.requested",
            "linear.issue.left"
        ]
    );
}

#[test]
fn a_view_whose_filter_changes_between_pages_fails_the_poll_and_publishes_nothing() {
    let setup = Setup::linear("linear-filter-pages", &[("agent-ready", VIEW_TRIGGER)]);
    setup.answer_linear(&view_with(json!({ "state": "Todo" }), &[7]));
    succeeded(&setup.intake());
    let page = |filter: &str, issue: u64, next: bool| {
        let answer = json!({ "data": { "customView": { "filterData": { "state": filter }, "issues": {
            "nodes": [linear_issue(issue)],
            "pageInfo": { "hasNextPage": next, "endCursor": "page-2" } } } } });
        let file = setup.dir.join(format!("linear-view-{issue}.json"));
        write(&file, &answer.to_string());
        file.display().to_string()
    };
    let script = format!(
        "#!/bin/sh\ncat >/dev/null\ncase \"$*\" in\n  *'query View'*'page-2'*) cat '{second}' ;;\n  \
         *'query View'*) cat '{first}' ;;\n  *'query Comments'*) cat '{comments}' ;;\n  \
         *) cat '{issues}' ;;\nesac\n",
        first = page("Todo", 7, true),
        second = page("In Progress", 8, false),
        comments = setup.dir.join("linear-comments.json").display(),
        issues = setup.dir.join("linear-issues.json").display(),
    );
    write(&setup.dir.join("bin/curl"), &script);

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Linear view 6b3c2f9a1d4e: its filter changed while the intake read it"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(kinds(&setup.published()), ["linear.issue.entered"]);
}

/// A view marker written before the intake kept filters: no `filter`, and members without
/// `project_id`.
fn legacy_view_marker(setup: &Setup, members: serde_json::Value) {
    let marker = json!({ "stream": "linear.view:6b3c2f9a1d4e", "position": 1, "members": members });
    // The id the intake gives this marker, as docs/intake/envelope.md names it.
    let namespace = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        b"https://github.com/maxence-lefebvre/priory/intake",
    );
    let name = "factory.intake.baseline:linear.view:6b3c2f9a1d4e:1:null";
    seed(
        setup,
        &Message::new("factory.intake.baseline", marker)
            .id(uuid::Uuid::new_v5(&namespace, name.as_bytes()))
            .subject("linear.view:6b3c2f9a1d4e"),
    );
}

#[test]
fn a_view_with_a_legacy_marker_records_its_filter_at_the_first_poll() {
    let setup = Setup::linear("linear-legacy-filter", &[("agent-ready", VIEW_TRIGGER)]);
    legacy_view_marker(&setup, json!([]));
    setup.answer_linear(&view_with(json!({ "state": "Todo" }), &[]));
    succeeded(&setup.intake());
    setup.answer_linear(&view_with(json!({ "state": "In Progress" }), &[8]));

    succeeded(&setup.intake());

    assert_eq!(kinds(&setup.published()), ["linear.view.changed"]);
}

#[test]
fn an_issue_leaving_a_view_with_a_legacy_marker_carries_project_id() {
    let setup = Setup::linear("linear-legacy-left", &[("agent-ready", VIEW_TRIGGER)]);
    let mut member = json!({
        "issue": "ENG-7", "linear_id": "issue-7", "title": "Fix the thing",
        "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing", "team": "ENG",
        "project": null, "state": "Todo", "labels": ["agent"]
    });
    legacy_view_marker(&setup, json!([member.clone()]));
    setup.answer_linear(&view_with(json!({ "state": "Todo" }), &[]));

    succeeded(&setup.intake());

    let bus = Bus::open(&setup.dir.join("bus.db")).unwrap();
    let left = bus
        .messages(&MessageQuery {
            output: Some("factory.intake".into()),
            limit: 10,
            ..MessageQuery::default()
        })
        .unwrap()
        .into_iter()
        .find(|stored| stored.message.kind == "linear.issue.left")
        .unwrap();
    member["project_id"] = json!(null);
    member["view"] = json!("6b3c2f9a1d4e");
    member["baseline"] = json!(false);
    member["triggers"] = json!([]);
    assert_eq!(timeless(&left.message), member);
}

fn created_in(number: u64, id: &str, name: &str) -> serde_json::Value {
    let mut issue = linear_issue(number);
    issue["createdAt"] = json!("2026-11-17T11:00:00.000Z");
    issue["updatedAt"] = json!("2026-11-17T11:00:00.000Z");
    issue["project"] = json!({ "id": id, "name": name });
    issue
}

#[test]
fn a_project_filter_keeps_only_that_projects_issues() {
    let by_name = "source = \"linear\"\non = \"issue.created\"\nfilter = { project = \"Mobile\" }\nworkflow = \"implement\"\n";
    let by_id = "source = \"linear\"\non = \"issue.created\"\nfilter = { project = \"project-mobile\" }\nworkflow = \"implement\"\n";
    let setup = Setup::linear(
        "linear-project-filter",
        &[("by-name", by_name), ("by-id", by_id)],
    );
    setup.answer_linear(&Linear {
        issues: vec![linear_issue(1)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        issues: vec![
            created_in(7, "project-mobile", "Mobile"),
            created_in(8, "project-web", "Web"),
        ],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    let created: Vec<(String, serde_json::Value)> = setup
        .published()
        .into_iter()
        .filter(|stored| stored.message.kind == "linear.issue.created")
        .map(|stored| {
            let data = stored.message.data;
            (
                data["issue"].as_str().unwrap().to_owned(),
                data["triggers"].clone(),
            )
        })
        .collect();
    assert_eq!(
        created,
        [
            ("ENG-1".to_owned(), json!([])),
            ("ENG-7".to_owned(), json!(["by-id", "by-name"])),
            ("ENG-8".to_owned(), json!([])),
        ]
    );
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
    crashed.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });

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

#[test]
fn an_issue_that_left_a_view_during_a_crash_publishes_issue_left_and_can_enter_again() {
    let setup = Setup::linear("linear-crash-left", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    let entered = setup.published()[0].message.clone();
    let crashed = Setup::linear("linear-crash-left-replay", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&crashed.intake());
    seed(&crashed, &entered);
    succeeded(&crashed.intake());
    crashed.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });

    succeeded(&crashed.intake());

    assert_eq!(
        kinds(&crashed.published()),
        [
            "linear.issue.entered",
            "factory.task.requested",
            "linear.issue.left",
            "linear.issue.entered",
            "factory.task.requested"
        ]
    );
}

#[test]
fn a_trigger_added_after_a_crash_gets_no_task_for_an_event_already_published() {
    let setup = Setup::linear("linear-crash-trigger", &[("commented", COMMENT_TRIGGER)]);
    succeeded(&setup.intake());
    let comment = linear_comment("comment-1", 7, "2026-11-17T11:00:00.000Z");
    setup.answer_linear(&Linear {
        comments: vec![comment.clone()],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    let published = setup.published()[0].message.clone();
    let crashed = Setup::linear(
        "linear-crash-trigger-replay",
        &[("commented", COMMENT_TRIGGER)],
    );
    succeeded(&crashed.intake());
    seed(&crashed, &published);
    write(
        &crashed.dir.join("factory/triggers/also.toml"),
        COMMENT_TRIGGER,
    );
    crashed.answer_linear(&Linear {
        comments: vec![comment],
        ..Linear::default()
    });

    succeeded(&crashed.intake());

    let requests: Vec<_> = crashed
        .published()
        .into_iter()
        .filter(|stored| stored.message.kind == "factory.task.requested")
        .map(|stored| stored.message.data["trigger"].clone())
        .collect();
    assert_eq!(requests, [json!("commented")]);
}

#[test]
fn a_graphql_error_is_reported_without_the_key() {
    let setup = Setup::linear("linear-graphql-error", &[("agent-ready", VIEW_TRIGGER)]);
    let answer = json!({ "errors": [{ "message": format!("Invalid token {KEY}") }] });
    write(
        &setup.dir.join("linear-customView.json"),
        &answer.to_string(),
    );

    let output = setup.intake();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid token"), "{stderr}");
    assert!(!stderr.contains(KEY), "{stderr}");
}

#[test]
fn a_restart_after_a_crash_at_the_same_instant_keeps_the_events_already_handled() {
    let at = "2026-11-17T11:00:00.000Z";
    let setup = Setup::linear("linear-crash-instant", &[("commented", COMMENT_TRIGGER)]);
    setup.answer_linear(&Linear {
        comments: vec![linear_comment("comment-a", 7, at)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    let both = Linear {
        comments: vec![
            linear_comment("comment-a", 7, at),
            linear_comment("comment-b", 7, at),
        ],
        ..Linear::default()
    };
    setup.answer_linear(&both);
    let crashed = Setup::linear(
        "linear-crash-instant-replay",
        &[("commented", COMMENT_TRIGGER)],
    );
    crashed.answer_linear(&Linear {
        comments: vec![linear_comment("comment-a", 7, at)],
        ..Linear::default()
    });
    succeeded(&crashed.intake());
    succeeded(&setup.intake());
    let b = setup.published()[1].message.clone();
    assert_eq!(b.data["linear_id"], "comment-b");
    seed(&crashed, &b);
    crashed.answer_linear(&both);

    succeeded(&crashed.intake());

    let requests: Vec<_> = crashed
        .published()
        .into_iter()
        .filter(|stored| stored.message.kind == "factory.task.requested")
        .map(|stored| stored.message.subject.unwrap_or_default())
        .collect();
    assert_eq!(requests, ["linear:ENG-7"]);
    assert_eq!(
        crashed
            .published()
            .iter()
            .filter(|stored| stored.message.data["linear_id"] == "comment-a")
            .count(),
        1
    );
}

/// `issue`, blocked by each of `blockers`, an identifier and its state type, and related to ENG-5.
fn blocked_by(
    mut issue: serde_json::Value,
    blockers: &[(&str, &str)],
    more: bool,
) -> serde_json::Value {
    let mut nodes: Vec<_> = blockers
        .iter()
        .map(|(identifier, state)| {
            json!({ "type": "blocks", "issue": { "identifier": identifier, "state": { "type": state } } })
        })
        .collect();
    nodes.push(json!({ "type": "related", "issue": { "identifier": "ENG-5", "state": { "type": "started" } } }));
    issue["inverseRelations"] = json!({ "nodes": nodes, "pageInfo": { "hasNextPage": more } });
    issue
}

#[test]
fn a_blocker_that_changes_publishes_the_open_blockers_of_the_issue_it_blocks() {
    let setup = Setup::linear("linear-blockers", &[("started", STATE_TRIGGER)]);
    setup.answer_linear(&Linear {
        issues: vec![linear_issue(1)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    let time = "2026-11-17T11:00:00.000Z";
    let mut blocker = moved(3, "history-3", "Done", time);
    blocker["relations"] = json!({ "nodes": [
        { "type": "blocks", "relatedIssue": { "id": "issue-7" } },
        { "type": "related", "relatedIssue": { "id": "issue-9" } },
        { "type": "blocks", "relatedIssue": { "id": "issue-8" } }
    ] });
    let mut blocked = vec![
        blocked_by(
            linear_issue(7),
            &[("ENG-3", "completed"), ("ENG-4", "started")],
            false,
        ),
        blocked_by(linear_issue(8), &[("ENG-3", "completed")], true),
    ];
    // Linear answers a filter by id in its own order.
    blocked.reverse();
    setup.answer_linear(&Linear {
        issues: vec![blocker],
        blocked,
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
            "linear.issue.blockers",
            "linear.issue.blockers"
        ]
    );
    assert_eq!(published[1].message.data["blockers"], json!([]));
    // A blocker list past its first page could hide an open blocker, so it reads as unknown.
    assert_eq!(published[3].message.data["blockers"], json!(null));
    let blockers = &published[2].message;
    assert_eq!(blockers.subject.as_deref(), Some("ENG-7"));
    assert_eq!(
        blockers.data,
        json!({
            "issue": "ENG-7",
            "linear_id": "issue-7",
            "title": "Fix the thing",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing",
            "team": "ENG",
            "project": null,
            "project_id": null,
            "state": "Todo",
            "labels": ["agent"],
            "blockers": ["ENG-4"],
            "blocker": "ENG-3",
            "time": time,
            "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_trigger_with_a_hold_files_its_tasks_held() {
    let trigger =
        format!("{VIEW_TRIGGER}hold = {{ reason = \"triage first\", blockers = true }}\n");
    let setup = Setup::linear("linear-hold", &[("agent-ready", &trigger)]);
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![linear_issue(7)],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    let task = &setup.published()[1].message;
    assert_eq!(
        task.data["hold"],
        json!({ "reason": "triage first", "blockers": true })
    );
}

fn in_project(number: u64, project: &str) -> serde_json::Value {
    let mut issue = linear_issue(number);
    issue["project"] = json!({ "name": project });
    issue
}

#[test]
fn a_linear_task_request_carries_its_issues_team_and_project() {
    let setup = Setup::linear("linear-project", &[("agent-ready", VIEW_TRIGGER)]);
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![in_project(7, "Desktop")],
        ..Linear::default()
    });

    succeeded(&setup.intake());

    let published = setup.published();
    let (source, task) = (&published[0].message, &published[1].message);
    assert_eq!(source.data["project"], "Desktop");
    assert_eq!(task.data["team"], "ENG");
    assert_eq!(task.data["project"], "Desktop");
    let queries = fs::read_to_string(setup.dir.join("curl-args")).unwrap();
    assert!(queries.contains("project { id name }"), "{queries}");
}

#[test]
fn a_linear_task_lands_in_the_lane_its_team_and_project_derive() {
    let setup = Setup::linear("linear-lane", &[("agent-ready", VIEW_TRIGGER)]);
    write(
        &setup.dir.join("factory/lanes.toml"),
        "[[rules]]\nteam = \"ENG\"\nproject = \"Desktop\"\nlane = \"ENG/desktop\"\n",
    );
    succeeded(&setup.intake());
    setup.answer_linear(&Linear {
        view: vec![in_project(7, "Desktop"), linear_issue(8)],
        ..Linear::default()
    });
    succeeded(&setup.intake());
    setup.inbox();

    let tasks = harness::priory()
        .arg("tasks")
        .env("PRIORY_DB", setup.dir.join("priory.db"))
        .output()
        .unwrap();

    succeeded(&tasks);
    let lanes: Vec<[String; 2]> = String::from_utf8(tasks.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            [cells[1].to_owned(), cells[6].to_owned()]
        })
        .collect();
    assert_eq!(
        lanes,
        [
            ["linear:ENG-7", "ENG/desktop"],
            ["linear:ENG-8", "ENG/misc"],
        ]
        .map(|row| row.map(str::to_owned))
    );
}

const SLACK_TRIGGER: &str = r#"
source = "slack"
channel = "C0123"
on = "message.posted"
filter = { text = "deploy" }
workflow = "implement"
"#;

impl Setup {
    /// Makes the fake `curl` answer Slack's `auth.test` as bot U0BOT, and the channel's history
    /// with `messages`, newest first as Slack lists them.
    fn answer_slack(&self, messages: &[serde_json::Value]) {
        let auth = self.dir.join("slack-auth.json");
        write(
            &auth,
            &json!({ "ok": true, "user_id": "U0BOT", "bot_id": "B0BOT" }).to_string(),
        );
        let history = self.dir.join("slack-history.json");
        write(
            &history,
            &json!({ "ok": true, "messages": messages, "has_more": false }).to_string(),
        );
        let script = format!(
            "#!/bin/sh\necho \"$*\" >> '{dir}/curl-args'\ncat >> '{dir}/curl-stdin'\n\
             case \"$*\" in\n  *auth.test*) cat '{auth}' ;;\n  *conversations.history*) cat '{history}' ;;\n\
             *) echo 'unexpected call' >&2; exit 2 ;;\nesac\n",
            dir = self.dir.display(),
            auth = auth.display(),
            history = history.display(),
        );
        let curl = self.dir.join("bin/curl");
        write(&curl, &script);
        fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn slack_intake(&self) -> Output {
        self.command()
            .env("SLACK_BOT_TOKEN", "xoxb-secret-test-token")
            .output()
            .unwrap()
    }
}

fn slack_message(ts: &str, user: &str, text: &str) -> serde_json::Value {
    json!({ "type": "message", "ts": ts, "user": user, "text": text })
}

#[test]
fn a_failing_slack_channel_still_lets_the_others_publish() {
    let left = SLACK_TRIGGER.replace("C0123", "C0999");
    let setup = Setup::linear(
        "slack-one-fails",
        &[("deploys", SLACK_TRIGGER), ("left", &left)],
    );
    setup.answer_slack(&[slack_message("1763500000.000100", "U0PERSON", "deploy")]);
    let curl = setup.dir.join("bin/curl");
    let script = fs::read_to_string(&curl).unwrap().replace(
        "case \"$*\" in\n",
        "case \"$*\" in\n  *channel=C0999*) echo '{\"ok\":false,\"error\":\"not_in_channel\"}' ;;\n",
    );
    write(&curl, &script);

    let output = setup.slack_intake();

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not_in_channel"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(kinds(&setup.published()), ["slack.message.posted"]);
}

#[test]
fn a_slack_trigger_requests_one_task_per_matching_message_and_drops_a_duplicate() {
    let setup = Setup::linear("slack-trigger", &[("deploys", SLACK_TRIGGER)]);
    setup.answer_slack(&[slack_message(
        "1763500000.000100",
        "U0PERSON",
        "deploy of old",
    )]);
    succeeded(&setup.slack_intake());
    assert!(
        kinds(&setup.published()) == ["slack.message.posted"],
        "a first poll is a baseline: {:?}",
        kinds(&setup.published())
    );
    setup.answer_slack(&[
        slack_message("1763500003.000100", "U0BOT", "I deploy nothing"),
        slack_message("1763500002.000100", "U0PERSON", "lunch?"),
        slack_message("1763500001.000200", "U0PERSON", "Please DEPLOY the API"),
        slack_message("1763500001.000100", "U0PERSON", "deploy the web app"),
        slack_message("1763500000.000100", "U0PERSON", "deploy of old"),
    ]);

    succeeded(&setup.slack_intake());
    succeeded(&setup.slack_intake());

    let requested: Vec<serde_json::Value> = setup
        .published()
        .iter()
        .filter(|stored| stored.message.kind == "factory.task.requested")
        .map(|stored| stored.message.data["key"].clone())
        .collect();
    assert_eq!(
        requested,
        [
            json!("slack:C0123:1763500001.000100"),
            json!("slack:C0123:1763500001.000200")
        ]
    );
    assert_eq!(
        kinds(&setup.published()),
        [
            "slack.message.posted",
            "slack.message.posted",
            "factory.task.requested",
            "slack.message.posted",
            "factory.task.requested",
            "slack.message.posted",
        ]
    );
    setup.inbox();
    let tasks = harness::priory()
        .arg("tasks")
        .env("PRIORY_DB", setup.dir.join("priory.db"))
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(tasks.stdout).unwrap().lines().count(), 3);
}

const REVIEW_TRIGGER: &str = r#"
source = "github"
on = "pull_request.review_requested"
reviewer = "max"
workflow = "implement"
band = "today"
"#;

const REVIEW_SEARCH: &str =
    "search/issues?q=is%3Apr+is%3Aopen+user-review-requested%3Amax&per_page=100";

impl Setup {
    /// A factory with only the review request trigger for `max`, whose first poll found `pulls`.
    fn reviews(name: &str, pulls: &[(&str, u64)]) -> Self {
        let setup = Setup::new(name, &[]);
        fs::remove_file(setup.dir.join("factory/triggers/agent-label.toml")).unwrap();
        write(
            &setup.dir.join("factory/triggers/review.toml"),
            REVIEW_TRIGGER,
        );
        setup.requested(pulls);
        succeeded(&setup.intake());
        setup
    }

    /// Makes the fake `gh` answer the review request search with `pulls`, and the reviews query
    /// with no review from `max`.
    fn requested(&self, pulls: &[(&str, u64)]) {
        self.requested_after(pulls, &[]);
    }

    /// As `requested`, with `max`'s reviews submitted at `submitted`.
    fn requested_after(&self, pulls: &[(&str, u64)], submitted: &[&str]) {
        let items: Vec<serde_json::Value> = pulls
            .iter()
            .map(|(repo, number)| {
                json!({
                    "number": number,
                    "title": "Tidy the parser",
                    "html_url": format!("https://github.com/{repo}/pull/{number}"),
                    "repository_url": format!("https://api.github.com/repos/{repo}"),
                    "user": { "login": "alice" },
                    "updated_at": "2026-11-20T09:00:00Z",
                    "pull_request": { "url": format!("https://api.github.com/repos/{repo}/pulls/{number}") }
                })
            })
            .collect();
        let answer =
            json!({ "total_count": items.len(), "incomplete_results": false, "items": items });
        let nodes: Vec<_> = submitted
            .iter()
            .map(|time| json!({ "submittedAt": time }))
            .collect();
        let reviews = json!({ "data": { "repository": { "pullRequest": { "reviews": { "nodes": nodes } } } } });
        self.answer(&[(REVIEW_SEARCH, answer), ("graphql", reviews)]);
    }

    /// The `priory tasks` rows, as key and state.
    fn task_states(&self) -> Vec<[String; 2]> {
        let output = harness::priory()
            .arg("tasks")
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .output()
            .unwrap();
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| {
                let cells: Vec<&str> = line.split('\t').collect();
                [cells[1].to_owned(), cells[4].to_owned()]
            })
            .collect()
    }
}

#[test]
fn a_first_review_request_poll_is_a_baseline_that_requests_no_task() {
    let setup = Setup::reviews("review-baseline", &[("acme/app", 7)]);

    let published = setup.published();
    assert_eq!(kinds(&published), ["github.pull_request.review_requested"]);
    assert_eq!(published[0].message.data["baseline"], true);
    assert_eq!(published[0].message.data["triggers"], json!([]));
    let marker = setup.output().pop().unwrap().message;
    assert_eq!(marker.kind, "factory.intake.baseline");
    assert_eq!(marker.data["stream"], "github.review_requests:max");
}

const REVIEW_KEY: &str = "github:acme/app:pulls/7:review:max";

#[test]
fn a_new_review_request_publishes_the_event_and_creates_its_task() {
    let setup = Setup::reviews("review-requested", &[]);
    setup.requested(&[("acme/app", 7)]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        [
            "github.pull_request.review_requested",
            "factory.task.requested"
        ]
    );
    let (source, task) = (&published[0].message, &published[1].message);
    assert_eq!(source.data["since"], source.data["time"]);
    let mut data = timeless(source);
    data.as_object_mut().unwrap().remove("since");
    assert_eq!(
        data,
        json!({
            "repo": "acme/app",
            "pull_request": 7,
            "title": "Tidy the parser",
            "url": "https://github.com/acme/app/pull/7",
            "author": "alice",
            "reviewer": "max",
            "baseline": false,
            "triggers": ["review"]
        })
    );
    assert_eq!(source.subject.as_deref(), Some("acme/app#7"));
    assert_eq!(task.data["key"], REVIEW_KEY);
    assert_eq!(task.data["repo"], "acme/app");
    let routed: Vec<[String; 2]> = setup
        .inbox()
        .into_iter()
        .filter(|row| row[1] != "factory.intake.baseline")
        .map(|row| [row[3].clone(), row[4].clone()])
        .collect();
    assert_eq!(
        routed,
        [["-", "ignored"], ["1", "created"]].map(|row| row.map(str::to_owned))
    );
    assert_eq!(
        setup.task_states(),
        [[REVIEW_KEY, "queued"].map(str::to_owned)]
    );
}

/// A review request search answer of `pulls`, out of `total`.
fn search_page(pulls: std::ops::Range<u64>, total: u64) -> serde_json::Value {
    let items: Vec<serde_json::Value> = pulls
        .map(|number| {
            json!({
                "number": number,
                "title": "Tidy the parser",
                "html_url": format!("https://github.com/acme/app/pull/{number}"),
                "repository_url": "https://api.github.com/repos/acme/app",
                "user": { "login": "alice" },
            })
        })
        .collect();
    json!({ "total_count": total, "incomplete_results": false, "items": items })
}

#[test]
fn a_review_request_poll_reads_every_page_of_the_search() {
    let setup = Setup::reviews("review-pages", &[]);
    setup.answer(&[
        (
            &format!("{REVIEW_SEARCH}&page=2"),
            search_page(101..151, 150),
        ),
        (REVIEW_SEARCH, search_page(1..101, 150)),
    ]);

    succeeded(&setup.intake());

    let requested = setup
        .published()
        .iter()
        .filter(|stored| stored.message.kind == "github.pull_request.review_requested")
        .count();
    assert_eq!(requested, 150);
}

#[test]
fn a_review_request_to_a_team_of_the_reviewer_creates_its_task() {
    let setup = Setup::reviews("review-team", &[]);
    write(
        &setup.dir.join("factory/triggers/review.toml"),
        &format!("{REVIEW_TRIGGER}teams = [\"acme/core\"]\n"),
    );
    let team = "search/issues?q=is%3Apr+is%3Aopen+team-review-requested%3Aacme%2Fcore&per_page=100";
    setup.answer(&[
        (team, search_page(7..8, 1)),
        (REVIEW_SEARCH, search_page(0..0, 0)),
    ]);

    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        [
            "github.pull_request.review_requested",
            "factory.task.requested"
        ]
    );
    assert_eq!(published[1].message.data["key"], REVIEW_KEY);
}

#[test]
fn a_review_request_whose_repository_is_not_owner_slash_name_is_dropped() {
    let setup = Setup::reviews("review-bad-repo", &[]);
    setup.requested(&[("acme/..", 8), ("acme/app", 7)]);

    succeeded(&setup.intake());

    let subjects: Vec<_> = setup
        .published()
        .iter()
        .map(|stored| stored.message.subject.clone().unwrap_or_default())
        .collect();
    assert_eq!(subjects, ["acme/app#7", REVIEW_KEY]);
}

#[test]
fn a_removed_review_request_publishes_the_event_and_withdraws_its_task() {
    let setup = Setup::reviews("review-removed", &[]);
    setup.requested(&[("acme/app", 7)]);
    succeeded(&setup.intake());
    setup.inbox();
    setup.requested(&[]);

    succeeded(&setup.intake());

    let removed = setup.published().pop().unwrap().message;
    assert_eq!(removed.kind, "github.pull_request.review_request_removed");
    assert_eq!(removed.data["reviewer"], "max");
    assert_eq!(removed.data["triggers"], json!([]));
    let mut inbox = setup.inbox();
    inbox.retain(|row| row[1] != "factory.intake.baseline");
    let last = inbox.pop().unwrap();
    assert_eq!(last[1], "github.pull_request.review_request_removed");
    assert_eq!([&last[3], &last[4]], ["1", "withdrew"]);
    assert_eq!(
        setup.task_states(),
        [[REVIEW_KEY, "dropped"].map(str::to_owned)]
    );
}

#[test]
fn a_review_re_request_reaches_the_same_task() {
    let setup = Setup::reviews("review-re-requested", &[]);
    setup.requested(&[("acme/app", 7), ("acme/app", 8)]);
    succeeded(&setup.intake());
    // Max reviews 7, which leaves the set, and the author asks again.
    setup.requested_after(&[("acme/app", 8)], &["2999-01-01T00:00:00Z"]);
    succeeded(&setup.intake());
    setup.requested(&[("acme/app", 7), ("acme/app", 8)]);

    succeeded(&setup.intake());

    let routed: Vec<[String; 3]> = setup
        .inbox()
        .into_iter()
        .filter(|row| row[2].starts_with("github:acme/app:pulls/7") || row[2] == "acme/app#7")
        .map(|row| [row[1].clone(), row[3].clone(), row[4].clone()])
        .collect();
    // A queued task has no turn to wake: the request coalesces into it.
    assert_eq!(
        routed[routed.len() - 3..],
        [
            ["github.pull_request.review_request_removed", "1", "kept"],
            ["github.pull_request.review_requested", "1", "kept"],
            ["factory.task.requested", "1", "coalesced"],
        ]
        .map(|row| row.map(str::to_owned))
    );
    let tasks = setup.task_states();
    assert_eq!(tasks.len(), 2, "{tasks:?}");
    assert_eq!(tasks[0], [REVIEW_KEY, "queued"].map(str::to_owned));
}

#[test]
fn a_failed_review_search_publishes_nothing_and_removes_nothing() {
    let setup = Setup::reviews("review-failed", &[("acme/app", 7)]);
    let before = setup.output().len();
    Setup::fake_gh(&setup.dir, "#!/bin/sh\necho 'HTTP 502' >&2\nexit 1\n");

    let output = setup.intake();

    assert!(!output.status.success());
    assert_eq!(setup.output().len(), before);
    setup.requested(&[("acme/app", 7)]);
    succeeded(&setup.intake());
    assert_eq!(setup.output().len(), before, "7 never left the set");
}

#[test]
fn a_review_request_on_a_pull_request_gone_from_github_is_removed_unreviewed() {
    let setup = Setup::reviews("review-gone", &[("acme/app", 7)]);
    let empty = json!({ "total_count": 0, "incomplete_results": false, "items": [] });
    let answer = setup.dir.join("empty-search.json");
    write(&answer, &empty.to_string());
    Setup::fake_gh(
        &setup.dir,
        &format!(
            "#!/bin/sh\ncase \"$*\" in\n  \"api search/\"*) cat '{}' ;;\n  *) echo 'gh: Could not resolve to a Repository with the name acme/app.' >&2; exit 1 ;;\nesac\n",
            answer.display()
        ),
    );

    succeeded(&setup.intake());

    let removed = setup.published().pop().unwrap().message;
    assert_eq!(removed.kind, "github.pull_request.review_request_removed");
    assert_eq!(removed.data["reviewed"], false);
}

#[test]
fn a_review_request_a_crash_left_without_its_marker_still_leaves_and_asks_again() {
    let setup = Setup::reviews("review-crash", &[]);
    // The crash stopped after the event of pull request 7, before its task request and marker.
    seed(
        &setup,
        &Message::new(
            "github.pull_request.review_requested",
            json!({
                "repo": "acme/app",
                "pull_request": 7,
                "title": "Tidy the parser",
                "url": "https://github.com/acme/app/pull/7",
                "author": "alice",
                "since": "2026-11-20T09:00:00Z",
                "reviewer": "max",
                "time": "2026-11-20T09:00:00Z",
                "baseline": false,
                "triggers": ["review"]
            }),
        )
        .subject("acme/app#7"),
    );
    setup.requested(&[]);

    succeeded(&setup.intake());
    setup.requested(&[("acme/app", 7)]);
    succeeded(&setup.intake());

    let published = setup.published();
    assert_eq!(
        kinds(&published),
        [
            "github.pull_request.review_requested",
            "factory.task.requested",
            "github.pull_request.review_request_removed",
            "github.pull_request.review_requested",
            "factory.task.requested",
        ]
    );
    assert_eq!(published[1].message.data["key"], REVIEW_KEY);
    assert_eq!(published[2].message.data["since"], "2026-11-20T09:00:00Z");
}
