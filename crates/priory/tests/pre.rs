//! A trigger's `pre` script, as the intake and `priory intake pre` run it.

mod conform;

use herdr_bus::{Bus, MessageQuery, Stored};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
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

/// A factory whose trigger has the `[pre]` table `pre`, a fake `gh` and a bus file.
struct Setup {
    dir: PathBuf,
}

impl Setup {
    fn new(name: &str, pre: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("pre")
            .join(name);
        priory_runtime::runtime::remove_sealed(&dir);
        let trigger = format!("{TRIGGER}\n[pre]\n{pre}");
        let files = [
            ("factory/factory.toml", "name = \"demo\""),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
            ("factory/triggers/agent-label.toml", trigger.as_str()),
        ];
        for (path, content) in files {
            write(&dir.join(path), content);
        }
        let setup = Setup { dir };
        setup.script("#!/bin/sh\ncat >/dev/null\n");
        setup
    }

    /// A first poll of `owner/repo`, which only records where polling stands, then `events`
    /// answered on the next.
    fn polled(name: &str, pre: &str, events: &[Value]) -> Self {
        let setup = Setup::new(name, pre);
        setup.answer(&[labeled(100, 41, "bug")]);
        succeeded(&setup.intake());
        let mut answer = events.to_vec();
        answer.push(labeled(100, 41, "bug"));
        setup.answer(&answer);
        setup
    }

    fn factory(&self) -> PathBuf {
        self.dir.join("factory")
    }

    /// Writes `scripts/pre`, executable.
    fn script(&self, body: &str) {
        let file = self.factory().join("scripts/pre");
        write(&file, body);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Makes the fake `gh` answer the issue events with `events`, newest first, and `[]` otherwise.
    fn answer(&self, events: &[Value]) {
        let file = self.dir.join("events.json");
        write(&file, &Value::Array(events.to_vec()).to_string());
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  \"api {EVENTS_PATH}\"*) cat '{}' ;;\n  *) echo '[]' ;;\nesac\n",
            file.display()
        );
        let gh = self.dir.join("bin/gh");
        write(&gh, &script);
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .args(args)
            .env("PATH", path)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"));
        command
    }

    fn intake(&self) -> Output {
        let factory = self.factory();
        self.command(&["intake", "--once", factory.to_str().unwrap()])
            .output()
            .unwrap()
    }

    fn priory(&self, args: &[&str]) -> String {
        let output = self.command(args).output().unwrap();
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    /// Routes what the intake published, as `priory daemon --once` does.
    fn route(&self) {
        let factory = self.factory();
        self.priory(&["daemon", factory.to_str().unwrap(), "--once"]);
    }

    /// `priory intake pre agent-label` on `event`.
    fn pre(&self, event: &Value) -> Value {
        let factory = self.factory();
        let mut child = self
            .command(&["intake", "pre", factory.to_str().unwrap(), "agent-label"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = event.to_string();
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut stdin, input.as_bytes());
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        succeeded(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    /// Everything on `factory.intake` after the baseline poll, oldest first, each checked against
    /// its schema.
    fn published(&self) -> Vec<Stored> {
        let Ok(bus) = Bus::open(&self.dir.join("bus.db")) else {
            return Vec::new();
        };
        let mut messages = bus
            .messages(&MessageQuery {
                output: Some("factory.intake".into()),
                limit: 1000,
                ..MessageQuery::default()
            })
            .unwrap_or_default();
        messages.reverse();
        for stored in &messages {
            conform::conforms(&serde_json::to_value(&stored.message).unwrap());
        }
        messages.retain(|stored| {
            stored.message.kind != "factory.intake.baseline"
                && stored.message.data["baseline"] != true
        });
        messages
    }

    fn kinds(&self) -> Vec<String> {
        self.published()
            .iter()
            .map(|stored| stored.message.kind.clone())
            .collect()
    }

    fn of(&self, kind: &str) -> Vec<herdr_bus::Message> {
        self.published()
            .into_iter()
            .map(|stored| stored.message)
            .filter(|message| message.kind == kind)
            .collect()
    }

    fn desk_reports(&self) -> Vec<String> {
        let desk: Value = serde_json::from_str(&self.priory(&["desk", "--json"])).unwrap();
        desk["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["kind"] == "report")
            .map(|item| item["what"].as_str().unwrap().to_owned())
            .collect()
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("priory.db")).unwrap()
    }

    fn file(&self, name: &str) -> PathBuf {
        self.factory().join(name)
    }
}

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn labeled(id: u64, number: u64, label: &str) -> Value {
    labeled_titled(id, number, label, "Fix the thing")
}

fn labeled_titled(id: u64, number: u64, label: &str, title: &str) -> Value {
    json!({
        "id": id,
        "event": "labeled",
        "created_at": "2026-11-17T10:00:00Z",
        "actor": { "login": "max" },
        "label": { "name": label, "color": "ededed" },
        "issue": {
            "id": 9000 + number,
            "number": number,
            "title": title,
            "html_url": format!("https://github.com/owner/repo/issues/{number}"),
            "user": { "login": "max" },
            "created_at": "2026-11-17T09:00:00Z"
        }
    })
}

/// The id the intake gives trigger `trigger`'s request for the source event `source`.
fn request_id(trigger: &str, source: &uuid::Uuid) -> String {
    let namespace = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        b"https://github.com/maxence-lefebvre/priory/intake",
    );
    let name = format!("factory.task.requested:{trigger}:{source}");
    uuid::Uuid::new_v5(&namespace, name.as_bytes()).to_string()
}

/// A labeled event as the log holds it, for `priory intake pre`.
fn event(title: &str) -> Value {
    json!({
        "id": "0196f2a8-0000-7000-8000-000000000042",
        "type": "github.issues.labeled",
        "subject": "owner/repo#42",
        "time": "2026-11-17T10:00:00Z",
        "data": {
            "repo": "owner/repo",
            "issue": 42,
            "title": title,
            "url": "https://github.com/owner/repo/issues/42",
            "label": "agent",
            "actor": "max",
            "time": "2026-11-17T10:00:00Z",
            "github_id": 101,
            "baseline": false,
            "triggers": ["agent-label"]
        }
    })
}

/// Waits until `done` holds, at most `limit`.
fn until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

const PRE: &str = "argv = [\"scripts/pre\"]\n";

#[test]
fn a_script_that_exits_0_and_prints_nothing_leaves_the_request_as_it_was() {
    let setup = Setup::polled("pass", PRE, &[labeled(101, 42, "agent")]);

    succeeded(&setup.intake());

    assert_eq!(
        setup.kinds(),
        ["github.issues.labeled", "factory.task.requested"]
    );
    let published = setup.published();
    let (source, request) = (&published[0].message, &published[1].message);
    assert_eq!(
        request.data,
        json!({
            "workflow": "implement",
            "key": "github:owner/repo#42",
            "band": "today",
            "lane": null,
            "trigger": "agent-label",
            "source": source.id.to_string(),
            "pre": { "script": "scripts/pre" }
        })
    );
    assert_eq!(
        request.id.to_string(),
        request_id("agent-label", &source.id)
    );
}

#[test]
fn a_script_that_exits_1_drops_the_request_with_its_first_line_and_creates_no_task() {
    let setup = Setup::polled("drop", PRE, &[labeled(101, 42, "agent")]);
    setup.script(
        "#!/bin/sh\ncat >/dev/null\necho 'not ours: a bot labeled it'\necho more\nexit 1\n",
    );

    succeeded(&setup.intake());

    assert_eq!(
        setup.kinds(),
        ["github.issues.labeled", "factory.intake.dropped"]
    );
    let published = setup.published();
    let (source, dropped) = (&published[0].message, &published[1].message);
    assert_eq!(
        dropped.id.to_string(),
        request_id("agent-label", &source.id)
    );
    assert_eq!(dropped.subject.as_deref(), Some("github:owner/repo#42"));
    let seconds = dropped.data["seconds"].clone();
    assert!(seconds.is_f64(), "{seconds}");
    assert_eq!(
        dropped.data,
        json!({
            "trigger": "agent-label",
            "source": source.id.to_string(),
            "key": "github:owner/repo#42",
            "script": "scripts/pre",
            "exit": 1,
            "reason": "not ours: a bot labeled it",
            "seconds": seconds
        })
    );
    setup.route();
    assert_eq!(
        setup.priory(&["tasks"]).lines().count(),
        1,
        "only the header"
    );
    let inbox = setup.priory(&["inbox"]);
    let rows: Vec<Vec<&str>> = inbox
        .lines()
        .skip(1)
        .map(|l| l.split('\t').collect())
        .collect();
    let kinds: Vec<&str> = rows.iter().map(|row| row[1]).collect();
    assert_eq!(
        kinds[kinds.len() - 2..],
        ["github.issues.labeled", "factory.intake.dropped"]
    );
    assert_eq!(
        rows.last().unwrap()[5],
        "dropped by scripts/pre: not ours: a bot labeled it"
    );
}

#[test]
fn a_script_that_prints_five_fields_creates_a_task_with_each() {
    let setup = Setup::polled("five", PRE, &[labeled(101, 42, "agent")]);
    setup.script(
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"band\": \"later\", \"lane\": \"api/errors\", \"title\": \"Division by zero in /divide\", \"text\": \"Fix the division.\", \"context\": \"312 users hit it today.\"}'\n",
    );

    succeeded(&setup.intake());
    setup.route();

    let request = &setup.of("factory.task.requested")[0];
    assert_eq!(request.data["band"], "later");
    assert_eq!(request.data["lane"], "api/errors");
    assert_eq!(request.data["title"], "Division by zero in /divide");
    assert_eq!(request.data["text"], "Fix the division.");
    assert_eq!(request.data["context"], "312 users hit it today.");
    let pre = &request.data["pre"];
    assert_eq!(
        pre["changed"],
        json!(["title", "text", "context", "band", "lane"])
    );
    assert_eq!(pre["band"], json!({ "from": "today", "to": "later" }));
    assert_eq!(pre["lane"], json!({ "from": null, "to": "api/errors" }));
    assert_eq!(pre["title"], json!({ "from": 13, "to": 27 }));
    assert_eq!(pre["text"], json!({ "from": null, "to": 17 }));
    assert_eq!(pre["context"], json!({ "from": null, "to": 23 }));
    let task: (String, String, String, String, String, String) = setup
        .db()
        .query_row(
            "SELECT band, lane, pre_title, pre_text, pre_context, pre_script FROM tasks WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        task,
        (
            "later".into(),
            "api/errors".into(),
            "Division by zero in /divide".into(),
            "Fix the division.".into(),
            "312 users hit it today.".into(),
            "scripts/pre".into()
        )
    );
    let (title, request_text): (String, String) = setup
        .db()
        .query_row(
            "SELECT title, request FROM task_text WHERE task = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(title, "Division by zero in /divide");
    assert_eq!(request_text, "Fix the division.\n312 users hit it today.");
}

#[test]
fn two_events_whose_script_prints_the_same_key_create_one_task_and_the_second_routes_to_it() {
    let setup = Setup::polled(
        "same-key",
        PRE,
        &[labeled(102, 43, "agent"), labeled(101, 42, "agent")],
    );
    setup.script("#!/bin/sh\ncat >/dev/null\necho '{\"key\": \"fingerprint-7\"}'\n");

    succeeded(&setup.intake());
    setup.route();

    let requests = setup.of("factory.task.requested");
    let keys: Vec<&Value> = requests.iter().map(|r| &r.data["key"]).collect();
    assert_eq!(
        keys,
        [
            &json!("github:fingerprint-7"),
            &json!("github:fingerprint-7")
        ]
    );
    assert_eq!(
        requests[0].data["pre"]["key"],
        json!({ "from": "github:owner/repo#42", "to": "github:fingerprint-7" })
    );
    let tasks: i64 = setup
        .db()
        .query_row("SELECT count(*) FROM tasks", [], |row| row.get(0))
        .unwrap();
    assert_eq!(tasks, 1);
    let second: (Option<i64>, String) = setup
        .db()
        .query_row(
            "SELECT task, outcome FROM inbox WHERE id = ?1",
            [requests[1].id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(second, (Some(1), "kept".to_owned()));
}

#[test]
fn event_text_reaches_stdin_byte_for_byte_and_no_argument_or_variable() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pre/injection-target");
    let _ = fs::remove_file(dir.join("pwned"));
    fs::create_dir_all(&dir).unwrap();
    let title = format!(
        "Crash $(touch {}/pwned) `id` \"quoted\" 'single'\nsecond line\\n",
        dir.display()
    );
    let setup = Setup::polled(
        "injection",
        "argv = [\"scripts/pre\", \"--fixed\"]\n",
        &[labeled_titled(101, 42, "agent", &title)],
    );
    setup.script(
        "#!/bin/sh\ncat > \"$PRIORY_FACTORY/stdin.json\"\nprintf '%s\\n' \"$0\" \"$@\" > \"$PRIORY_FACTORY/argv\"\nenv > \"$PRIORY_FACTORY/env\"\n",
    );

    succeeded(&setup.intake());

    let stdin: Value =
        serde_json::from_str(&fs::read_to_string(setup.file("stdin.json")).unwrap()).unwrap();
    let source = &setup.of("github.issues.labeled")[0];
    assert_eq!(stdin["event"]["data"], source.data);
    assert_eq!(stdin["event"]["data"]["title"], json!(title));
    assert_eq!(stdin["event"]["id"], json!(source.id.to_string()));
    assert_eq!(
        stdin["request"],
        json!({
            "workflow": "implement",
            "key": "github:owner/repo#42",
            "band": "today",
            "lane": null,
            "trigger": "agent-label"
        })
    );
    assert_eq!(
        fs::read_to_string(setup.file("argv")).unwrap(),
        "scripts/pre\n--fixed\n"
    );
    let env = fs::read_to_string(setup.file("env")).unwrap();
    assert!(!env.contains("touch") && !env.contains("quoted"), "{env}");
    assert!(!dir.join("pwned").exists());
}

#[test]
fn the_script_gets_the_short_list_of_variables_and_its_env_names_only() {
    let setup = Setup::polled(
        "env",
        "argv = [\"scripts/pre\"]\nenv = [\"AMPLITUDE_READ_TOKEN\", \"NOT_SET_ANYWHERE\"]\n",
        &[labeled(101, 42, "agent")],
    );
    setup.script("#!/bin/sh\ncat >/dev/null\nenv > \"$PRIORY_FACTORY/env\"\n");
    let factory = setup.factory();

    let output = setup
        .command(&["intake", "--once", factory.to_str().unwrap()])
        .env("AMPLITUDE_READ_TOKEN", "token-value")
        .env("OTHER_SECRET", "other-value")
        .env("LANG", "C.UTF-8")
        .env("HOME", setup.dir.join("home"))
        .output()
        .unwrap();

    succeeded(&output);
    let env = fs::read_to_string(setup.file("env")).unwrap();
    let names: BTreeSet<&str> = env
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        // What `sh` sets for itself.
        .filter(|name| !["PWD", "OLDPWD", "SHLVL", "_"].contains(name))
        .collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "AMPLITUDE_READ_TOKEN",
            "HOME",
            "LANG",
            "PATH",
            "PRIORY_FACTORY",
            "PRIORY_TRIGGER"
        ])
    );
    assert!(env.contains("AMPLITUDE_READ_TOKEN=token-value\n"));
    assert!(env.contains("PRIORY_TRIGGER=agent-label\n"));
}

#[test]
fn output_that_breaks_a_rule_counts_as_a_failure_and_passes_the_request_unchanged() {
    let setup = Setup::new("refused", PRE);
    setup.script("#!/bin/sh\ncat >/dev/null\ncat \"$PRIORY_FACTORY/out\"\n");
    let long_text = json!({ "text": "x".repeat(16 * 1024 + 1) }).to_string();
    let cases = [
        (
            r#"{"workflow": "other"}"#.to_owned(),
            "stdout names `workflow`, which a pre script cannot set",
        ),
        (
            r#"{"band": "someday"}"#.to_owned(),
            "band `someday` is not one of the factory's: blocker, now, today, later",
        ),
        (
            r#"{"lane": "api"}"#.to_owned(),
            "lane `api` is not `<team>/<theme>`",
        ),
        (
            r#"{"title": "two\nlines"}"#.to_owned(),
            "`title` must be one line of at most 200 characters",
        ),
        (long_text, "`text` is over 16 KiB"),
        (
            "not json".to_owned(),
            "stdout is not JSON: expected ident at line 1 column 2",
        ),
        ("a".repeat(70 * 1024), "printed too much on stdout"),
    ];
    for (out, failed) in cases {
        write(&setup.file("out"), &out);

        let printed = setup.pre(&event("Fix the thing"));

        assert_eq!(printed["outcome"], "published", "{failed}");
        assert_eq!(printed["message"]["data"]["pre"]["failed"], failed);
        assert_eq!(printed["message"]["data"]["band"], "today");
        assert_eq!(printed["message"]["data"]["key"], "github:owner/repo#42");
        assert_eq!(
            printed["failed"],
            format!("pre of trigger agent-label: {failed}")
        );
    }
}

#[test]
fn a_failed_script_passes_the_request_by_default_and_drops_it_with_on_error_drop() {
    let scripts = [
        (
            "#!/bin/sh\ncat >/dev/null\necho boom >&2\nexit 2\n",
            "exit 2: boom",
            Some(2),
        ),
        ("#!/bin/sh\nsleep 30\n", "timed out", None),
        (
            "#!/nonexistent/interpreter\n",
            "cannot start: scripts/pre: No such file or directory (os error 2)",
            None,
        ),
    ];
    for (on_error, outcome) in [("pass", "published"), ("drop", "dropped")] {
        let setup = Setup::new(
            &format!("fails-{on_error}"),
            &format!("argv = [\"scripts/pre\"]\ntimeout = \"1s\"\non_error = \"{on_error}\"\n"),
        );
        for (script, failed, exit) in scripts {
            setup.script(script);
            let start = Instant::now();

            let printed = setup.pre(&event("Fix the thing"));

            assert!(start.elapsed() < Duration::from_secs(10));
            assert_eq!(printed["outcome"], outcome, "{failed}");
            let data = &printed["message"]["data"];
            match on_error {
                "pass" => assert_eq!(data["pre"]["failed"], failed),
                _ => {
                    assert_eq!(printed["message"]["type"], "factory.intake.dropped");
                    assert_eq!(data["failed"], failed);
                    assert_eq!(data["exit"], json!(exit));
                    assert_eq!(data["reason"], Value::Null);
                }
            }
        }
    }
}

#[test]
fn a_failure_posts_one_report_and_a_run_that_exits_0_or_1_marks_it_read() {
    let setup = Setup::polled("report", PRE, &[labeled(101, 42, "agent")]);
    setup.script("#!/bin/sh\ncat >/dev/null\necho 'cannot reach amplitude' >&2\nexit 2\n");

    succeeded(&setup.intake());
    // The next start decides the newest event again, and the same failure posts nothing new.
    succeeded(&setup.intake());

    let request = &setup.of("factory.task.requested")[0];
    assert_eq!(
        request.data["pre"]["failed"],
        "exit 2: cannot reach amplitude"
    );
    assert_eq!(
        setup.desk_reports(),
        ["intake: pre of trigger agent-label: exit 2: cannot reach amplitude"]
    );

    setup.script("#!/bin/sh\ncat >/dev/null\n");
    succeeded(&setup.intake());
    assert!(setup.desk_reports().is_empty());

    setup.script("#!/bin/sh\ncat >/dev/null\nexit 3\n");
    succeeded(&setup.intake());
    assert_eq!(
        setup.desk_reports(),
        ["intake: pre of trigger agent-label: exit 3"]
    );
    setup.script("#!/bin/sh\ncat >/dev/null\nexit 1\n");
    succeeded(&setup.intake());
    assert!(setup.desk_reports().is_empty());
}

#[test]
fn a_script_that_floods_stderr_is_killed_and_priory_keeps_its_tail() {
    let setup = Setup::new("flood", PRE);
    setup.script("#!/bin/sh\ncat >/dev/null\nhead -c 1048576 /dev/zero | tr '\\0' x >&2\n");

    let printed = setup.pre(&event("Fix the thing"));

    let failed = printed["message"]["data"]["pre"]["failed"]
        .as_str()
        .unwrap();
    assert!(
        failed.starts_with("printed too much on stderr: xxx"),
        "{}",
        &failed[..60]
    );
    assert!(failed.len() <= 64 * 1024, "{}", failed.len());
}

#[test]
fn a_script_that_never_reads_stdin_is_killed_at_its_timeout_for_a_large_event() {
    let setup = Setup::new("unread", "argv = [\"scripts/pre\"]\ntimeout = \"1s\"\n");
    setup.script("#!/bin/sh\nsleep 30\n");
    let start = Instant::now();

    let printed = setup.pre(&event(&"y".repeat(512 * 1024)));

    assert!(start.elapsed() < Duration::from_secs(10));
    assert_eq!(printed["message"]["data"]["pre"]["failed"], "timed out");
}

#[test]
fn a_child_the_script_leaves_running_past_its_timeout_is_killed_too() {
    let setup = Setup::new("child", "argv = [\"scripts/pre\"]\ntimeout = \"1s\"\n");
    setup.script(
        "#!/bin/sh\ncat >/dev/null\nsleep 300 &\necho $! > \"$PRIORY_FACTORY/child\"\nsleep 300\n",
    );

    let printed = setup.pre(&event("Fix the thing"));

    assert_eq!(printed["message"]["data"]["pre"]["failed"], "timed out");
    let pid = fs::read_to_string(setup.file("child")).unwrap();
    let stat = format!("/proc/{}/stat", pid.trim());
    let gone = until(Duration::from_secs(5), || {
        fs::read_to_string(&stat).map_or(true, |stat| stat.contains(") Z "))
    });
    assert!(gone, "the script's child still runs");
}

#[test]
fn an_intake_stopped_after_the_script_ran_runs_it_again_and_the_log_holds_one_decision() {
    let setup = Setup::polled("stopped", PRE, &[labeled(101, 42, "agent")]);
    setup.script(
        "#!/bin/sh\ncat >/dev/null\necho run >> \"$PRIORY_FACTORY/runs\"\nif [ ! -e \"$PRIORY_FACTORY/stopped\" ]; then\n  touch \"$PRIORY_FACTORY/stopped\"\n  kill -KILL $PPID\nfi\n",
    );

    let stopped = setup.intake();
    succeeded(&setup.intake());
    succeeded(&setup.intake());

    assert!(!stopped.status.success());
    assert_eq!(
        setup.kinds(),
        ["github.issues.labeled", "factory.task.requested"]
    );
    let runs = fs::read_to_string(setup.file("runs")).unwrap();
    assert_eq!(
        runs.lines().count(),
        3,
        "each start decides the newest event again"
    );
}

#[test]
fn the_first_decision_of_an_event_stands_whatever_a_second_run_says() {
    let flip = "#!/bin/sh\ncat >/dev/null\nif [ -e \"$PRIORY_FACTORY/ran\" ]; then exit SECOND; fi\ntouch \"$PRIORY_FACTORY/ran\"\nexit FIRST\n";
    for (name, first, second, kept) in [
        ("pass-then-drop", "0", "1", "factory.task.requested"),
        ("drop-then-pass", "1", "0", "factory.intake.dropped"),
    ] {
        let setup = Setup::polled(name, PRE, &[labeled(101, 42, "agent")]);
        setup.script(&flip.replace("FIRST", first).replace("SECOND", second));

        succeeded(&setup.intake());
        succeeded(&setup.intake());

        assert_eq!(setup.kinds(), ["github.issues.labeled", kept], "{name}");
    }
}

#[test]
fn an_event_about_an_existing_task_still_routes_to_it_when_pre_drops_the_request() {
    let setup = Setup::polled("existing", PRE, &[labeled(101, 42, "agent")]);
    setup.script("#!/bin/sh\ncat >/dev/null\nif [ -e \"$PRIORY_FACTORY/drop\" ]; then echo again; exit 1; fi\n");
    succeeded(&setup.intake());
    setup.route();
    write(&setup.file("drop"), "");
    setup.answer(&[
        labeled(102, 42, "agent"),
        labeled(101, 42, "agent"),
        labeled(100, 41, "bug"),
    ]);

    succeeded(&setup.intake());
    setup.route();

    let source = setup
        .of("github.issues.labeled")
        .into_iter()
        .find(|event| event.data["github_id"] == 102)
        .unwrap();
    assert_eq!(setup.of("factory.intake.dropped").len(), 1);
    let routed: (Option<i64>, String) = setup
        .db()
        .query_row(
            "SELECT task, outcome FROM inbox WHERE id = ?1",
            [source.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(routed, (Some(1), "kept".to_owned()));
}

#[test]
fn intake_pre_prints_the_changed_request_and_publishes_nothing() {
    let setup = Setup::new("dry-run", PRE);
    setup.script("#!/bin/sh\ncat >/dev/null\necho '{\"band\": \"blocker\"}'\n");

    let printed = setup.pre(&event("Fix the thing"));

    assert_eq!(printed["outcome"], "published");
    assert_eq!(printed["failed"], Value::Null);
    let message = &printed["message"];
    assert_eq!(message["type"], "factory.task.requested");
    assert_eq!(message["data"]["band"], "blocker");
    assert_eq!(message["data"]["pre"]["changed"], json!(["band"]));
    assert_eq!(
        message["id"],
        json!(request_id(
            "agent-label",
            &"0196f2a8-0000-7000-8000-000000000042".parse().unwrap()
        ))
    );
    assert!(setup.published().is_empty());
    assert!(!setup.dir.join("priory.db").exists());
}

#[test]
fn a_schedule_firing_runs_its_pre_script_once_per_firing_and_on_the_intakes_first_poll() {
    let setup = Setup::new("schedule", PRE);
    setup.answer(&[]);
    write(
        &setup.factory().join("triggers/agent-label.toml"),
        "source = \"schedule\"\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nat = \"09:00\"\ntimezone = \"UTC\"\nprompt = \"Triage the backlog.\"\n\n[pre]\nargv = [\"scripts/pre\"]\n",
    );
    setup.script(
        "#!/bin/sh\ncat > \"$PRIORY_FACTORY/stdin.json\"\necho run >> \"$PRIORY_FACTORY/runs\"\necho '{\"band\": \"now\"}'\n",
    );
    let factory = setup.factory();
    let intake = || {
        let output = setup
            .command(&["intake", "--once", factory.to_str().unwrap()])
            .env("PRIORY_NOW", "2026-11-17T10:00:00Z")
            .output()
            .unwrap();
        succeeded(&output);
    };

    intake();
    intake();

    assert_eq!(setup.kinds(), ["schedule.fired", "factory.task.requested"]);
    let request = &setup.of("factory.task.requested")[0];
    assert_eq!(request.data["band"], "now");
    assert_eq!(request.data["prompt"], "Triage the backlog.");
    assert_eq!(
        request.data["key"],
        "schedule:agent-label@2026-11-17T09:00:00Z"
    );
    let stdin: Value =
        serde_json::from_str(&fs::read_to_string(setup.file("stdin.json")).unwrap()).unwrap();
    assert_eq!(stdin["event"]["type"], "schedule.fired");
    let runs = fs::read_to_string(setup.file("runs")).unwrap();
    assert_eq!(runs.lines().count(), 2, "the first poll of each start");
}

#[test]
fn the_bug_investigation_starters_sentry_script_drops_issues_below_error() {
    let starter = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/bug-investigation");
    for (level, outcome) in [
        ("warning", "dropped"),
        ("error", "published"),
        ("fatal", "published"),
    ] {
        let event = json!({
            "type": "sentry.issue.created",
            "subject": "PYTHON-Y",
            "data": { "issue": "4812", "project": "my-project", "level": level, "title": "\"level\":\"debug\"" }
        });
        let mut child = Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(["intake", "pre", starter.to_str().unwrap(), "sentry-issue"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(
            &mut child.stdin.take().unwrap(),
            event.to_string().as_bytes(),
        )
        .unwrap();
        let output = child.wait_with_output().unwrap();
        succeeded(&output);
        let printed: Value = serde_json::from_slice(&output.stdout).unwrap();

        assert_eq!(printed["outcome"], outcome, "{level}");
        assert_eq!(printed["failed"], Value::Null);
    }
}
