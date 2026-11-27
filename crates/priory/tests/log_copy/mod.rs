//! `priory log copy`: a herdr-bus log moved to Postgres keeps its history and cursors, so the
//! intake and every reader resume where they stood. Runs only when `PRIORY_TEST_POSTGRES_URL`
//! names a database. A module of `check.rs`, whose whole file CI's Postgres job runs.

use super::harness;

use herdr_bus::{Bus, Message, Output, Seq, Stored};
use priory_intake::log::Log;
use priory_intake::log::postgres::{Postgres, client};
use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output as Exit;
use std::time::Duration;

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

/// Fires at 09:00 Paris time, 08:00 UTC in November.
const SCHEDULE: &str = r#"
source = "schedule"
repo = "owner/repo"
at = "09:00"
timezone = "Europe/Paris"
workflow = "implement"
band = "today"
"#;

const BACKFILL: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.opened"
workflow = "implement"
backfill = true
"#;

const ISSUES_PATH: &str =
    "repos/owner/repo/issues?state=all&sort=created&direction=desc&per_page=100&page=1";

const INTAKE: &str = "factory.intake";

/// A schema of its own in the test database, and a factory folder with a herdr-bus file.
struct Setup {
    url: String,
    schema: String,
    dir: PathBuf,
}

impl Setup {
    /// None when `PRIORY_TEST_POSTGRES_URL` is not set: the test skips.
    fn new(test: &str) -> Option<Setup> {
        let Ok(url) = std::env::var("PRIORY_TEST_POSTGRES_URL") else {
            eprintln!("skipped: PRIORY_TEST_POSTGRES_URL is not set");
            return None;
        };
        let schema = format!("priory_copy_{}_{test}", std::process::id());
        let mut sql = client::Client::connect(&url, client::NoTls).unwrap();
        sql.batch_execute(&format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE; CREATE SCHEMA {schema}"
        ))
        .unwrap();
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("log_copy")
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("factory/factory.toml"), "name = \"demo\"\n");
        write(&dir.join("factory/agents/implementer/agent.md"), AGENT);
        write(&dir.join("factory/workflows/implement.toml"), WORKFLOW);
        Some(Setup { url, schema, dir })
    }

    /// The test database's URL, set to this test's schema.
    fn schema_url(&self) -> String {
        let separator = if self.url.contains('?') { '&' } else { '?' };
        format!(
            "{}{separator}options=-csearch_path%3D{}",
            self.url, self.schema
        )
    }

    fn postgres(&self) -> Postgres {
        Postgres::connect(&self.schema_url(), Duration::from_secs(10), false).unwrap()
    }

    fn bus_path(&self) -> PathBuf {
        self.dir.join("bus.db")
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.bus_path()).unwrap()
    }

    /// Points the factory's `[log]` at Postgres, as an operator does before the copy.
    fn switch(&self) {
        write(
            &self.dir.join("factory/factory.toml"),
            "name = \"demo\"\n[log]\nbackend = \"postgres\"\n",
        );
    }

    fn copy(&self, extra: &[&str]) -> Exit {
        harness::priory()
            .args(["log", "copy"])
            .arg(self.dir.join("factory"))
            .args(["--from", "herdr-bus"])
            .arg(self.bus_path())
            .args(extra)
            .env("PRIORY_LOG_URL", self.schema_url())
            .output()
            .unwrap()
    }

    /// Runs `priory intake --once` at `now`, against a fake `gh` that answers `answers`, on
    /// whichever log the factory names.
    fn intake(&self, now: &str, answers: &[(&str, serde_json::Value)]) {
        let mut script = String::from("#!/bin/sh\ncase \"$*\" in\n");
        for (index, (path, answer)) in answers.iter().enumerate() {
            let file = self.dir.join(format!("answer-{index}.json"));
            write(&file, &answer.to_string());
            script.push_str(&format!("  \"api {path}\"*) cat '{}' ;;\n", file.display()));
        }
        script.push_str("  *) echo '[]' ;;\nesac\n");
        let gh = self.dir.join("bin/gh");
        write(&gh, &script);
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let exit = harness::priory()
            .args(["intake", "--once"])
            .arg(self.dir.join("factory"))
            .env("PATH", path)
            .env("PRIORY_NOW", now)
            .env("HERDR_BUS_DB", self.bus_path())
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_LOG_URL", self.schema_url())
            .output()
            .unwrap();
        succeeded(&exit);
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Ok(mut sql) = client::Client::connect(&self.url, client::NoTls) {
            let _ = sql.batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
        }
    }
}

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn succeeded(exit: &Exit) {
    assert!(
        exit.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&exit.stdout),
        String::from_utf8_lossy(&exit.stderr)
    );
}

/// Every message of `output` in `log`, oldest first.
fn oldest_first(log: &dyn Log, output: &str) -> Vec<Stored> {
    let mut messages = log.messages(output, None, 10_000).unwrap();
    messages.reverse();
    messages
}

fn seqs_and_messages(messages: &[Stored]) -> Vec<(Seq, &Message)> {
    messages
        .iter()
        .map(|stored| (stored.seq, &stored.message))
        .collect()
}

fn kinds(messages: &[Stored]) -> Vec<&str> {
    messages
        .iter()
        .map(|stored| stored.message.kind.as_str())
        .collect()
}

#[test]
fn a_copy_keeps_every_message_and_every_cursor() {
    let Some(setup) = Setup::new("cursors") else {
        return;
    };
    let bus = setup.bus();
    for output in ["first", "second"] {
        Log::declare(&bus, &Output::new(output)).unwrap();
    }
    for n in 0..6 {
        let output = if n % 2 == 0 { "first" } else { "second" };
        let message = Message::new("test.event", json!({ "n": n })).subject(format!("event {n}"));
        Log::publish(&bus, output, &message).unwrap();
    }
    // Two readers read all of their output; a third stops after the first message of `second`.
    for (consumer, output, acked) in [
        ("reader.a", "first", 3),
        ("reader.b", "second", 3),
        ("reader.c", "second", 1),
    ] {
        for delivery in Log::read(&bus, consumer, output, acked).unwrap() {
            Log::ack(&bus, consumer, &delivery).unwrap();
        }
    }
    let before: Vec<Stored> = ["first", "second"]
        .iter()
        .flat_map(|output| oldest_first(&bus, output))
        .collect();
    drop(bus);
    setup.switch();

    let exit = setup.copy(&[]);
    succeeded(&exit);
    let report = String::from_utf8_lossy(&exit.stdout);
    assert!(report.contains("copied 6 messages"), "{report}");
    assert!(
        report.contains("first: 3") && report.contains("second: 3"),
        "{report}"
    );
    assert!(report.contains("reader.c on second: 2"), "{report}");

    let log = setup.postgres();
    let after: Vec<Stored> = ["first", "second"]
        .iter()
        .flat_map(|output| oldest_first(&log, output))
        .collect();
    assert_eq!(after.len(), before.len());
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(
            (old.seq, &old.output, &old.message),
            (new.seq, &new.output, &new.message)
        );
        // Postgres keeps microseconds.
        let drift = old.published.as_nanosecond() - new.published.as_nanosecond();
        assert!(drift.abs() < 1000, "{} {}", old.published, new.published);
    }
    assert!(log.read("reader.a", "first", 10).unwrap().is_empty());
    assert!(log.read("reader.b", "second", 10).unwrap().is_empty());
    let unread: Vec<_> = log.read("reader.c", "second", 10).unwrap();
    assert_eq!(
        unread
            .iter()
            .map(|delivery| delivery.seq)
            .collect::<Vec<_>>(),
        [after[4].seq, after[5].seq]
    );

    let next = log
        .publish("first", &Message::new("test.event", json!({ "n": 6 })))
        .unwrap();
    assert!(next > after.iter().map(|stored| stored.seq).max().unwrap());
}

#[test]
fn a_schedule_that_fired_does_not_fire_again_after_the_copy() {
    let Some(setup) = Setup::new("schedule") else {
        return;
    };
    write(&setup.dir.join("factory/triggers/standup.toml"), SCHEDULE);
    setup.intake("2026-11-19T08:30:00Z", &[]);
    let fired = oldest_first(&setup.bus(), INTAKE);
    assert!(
        kinds(&fired).contains(&"schedule.fired"),
        "{:?}",
        kinds(&fired)
    );

    setup.switch();
    succeeded(&setup.copy(&[]));
    setup.intake("2026-11-19T08:45:00Z", &[]);

    // A firing again would append a schedule.fired and a request past the copied seqs.
    let after = oldest_first(&setup.postgres(), INTAKE);
    assert_eq!(seqs_and_messages(&after), seqs_and_messages(&fired));
}

#[test]
fn a_backfilled_repository_is_no_baseline_again_after_the_copy() {
    let Some(setup) = Setup::new("backfill") else {
        return;
    };
    write(&setup.dir.join("factory/triggers/opened.toml"), BACKFILL);
    let issues = [(
        ISSUES_PATH,
        json!([{
            "id": 9042,
            "number": 42,
            "title": "Fix the thing",
            "html_url": "https://github.com/owner/repo/issues/42",
            "user": { "login": "someone" },
            "created_at": "2026-11-17T09:00:00Z"
        }]),
    )];
    setup.intake("2026-11-19T08:30:00Z", &issues);
    let polled = oldest_first(&setup.bus(), INTAKE);
    let kinds_before = kinds(&polled);
    assert!(
        kinds_before.contains(&"factory.intake.baseline"),
        "{kinds_before:?}"
    );
    assert!(
        kinds_before.contains(&"factory.task.requested"),
        "{kinds_before:?}"
    );

    setup.switch();
    succeeded(&setup.copy(&[]));
    setup.intake("2026-11-19T08:35:00Z", &issues);

    // A baseline again would append a second factory.intake.baseline past the copied seqs.
    let after = oldest_first(&setup.postgres(), INTAKE);
    assert_eq!(seqs_and_messages(&after), seqs_and_messages(&polled));
}

#[test]
fn a_copy_refuses_a_log_that_holds_a_message_and_a_dry_run_writes_nothing() {
    let Some(setup) = Setup::new("refuse") else {
        return;
    };
    let bus = setup.bus();
    Log::declare(&bus, &Output::new("first")).unwrap();
    Log::publish(&bus, "first", &Message::new("test.event", json!({}))).unwrap();
    drop(bus);
    setup.switch();

    let dry = setup.copy(&["--dry-run"]);
    succeeded(&dry);
    let report = String::from_utf8_lossy(&dry.stdout);
    assert!(report.contains("would copy 1 messages"), "{report}");
    let log = setup.postgres();
    assert!(log.messages("first", None, 10).unwrap().is_empty());

    let held = Message::new("test.held", json!({}));
    let seq = log.publish("other", &held).unwrap();
    let refused = setup.copy(&[]);
    assert!(!refused.status.success());
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(error.contains("only fills an empty log"), "{error}");
    let other = log.messages("other", None, 10).unwrap();
    assert_eq!((other.len(), other[0].seq), (1, seq));
    assert!(log.messages("first", None, 10).unwrap().is_empty());
}
