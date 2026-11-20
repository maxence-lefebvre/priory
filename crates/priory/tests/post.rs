use serde_json::Value;
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
            .join("post")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Setup { dir }
    }

    fn priory(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .output()
            .unwrap()
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = self.priory(args);
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.stdout(args)).unwrap()
    }

    fn post(&self, source: &str, line: &str) -> String {
        self.stdout(&["post", source, line]).trim().to_owned()
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn reports(desk: &Value) -> Vec<&str> {
    desk["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "report")
        .map(|item| item["what"].as_str().unwrap())
        .collect()
}

#[test]
fn a_posted_line_shows_on_the_inbox_and_the_desk() {
    let setup = Setup::new("shows");

    let seq = setup.post("ci-report", "/tmp/ci.md: 3 failures");

    assert_eq!(seq, "1");
    let inbox = setup.stdout(&["inbox"]);
    let row = inbox.lines().nth(1).unwrap();
    assert!(
        row.starts_with("1\tpriory.report\tci-report\t-\treport"),
        "{row}"
    );
    assert!(row.ends_with("/tmp/ci.md: 3 failures"), "{row}");
    assert_eq!(
        reports(&setup.json(&["desk", "--json"])),
        ["ci-report: /tmp/ci.md: 3 failures"]
    );
    let desk = setup.stdout(&["desk"]);
    assert!(
        desk.contains("\treport\t1\t-\t-\tci-report: /tmp/ci.md: 3 failures\t"),
        "{desk}"
    );
}

#[test]
fn a_daemon_source_is_refused() {
    let setup = Setup::new("refused");

    for source in ["github", "priory", "schedule", "Not Valid"] {
        let output = setup.priory(&["post", source, "a forged event"]);
        assert!(!output.status.success(), "{source} was accepted");
    }
    let refused = setup.priory(&["post", "github", "a forged event"]);
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr).trim(),
        "priory post: only priory itself posts as github"
    );
    assert_eq!(
        setup.json(&["inbox", "--json"])["inbox"],
        Value::Array(vec![])
    );
}

#[test]
fn inbox_since_prints_only_newer_entries() {
    let setup = Setup::new("since");
    for line in ["first", "second", "third"] {
        setup.post("systemd", line);
    }

    let since = &setup.json(&["inbox", "--since", "1", "--json"])["inbox"];

    let seqs: Vec<_> = since
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["seq"])
        .collect();
    assert_eq!(seqs, [2, 3]);
    assert_eq!(since[0]["line"], "second");
    let table = setup.stdout(&["inbox", "--since", "2"]);
    assert_eq!(table.lines().count(), 2, "{table}");
    assert!(table.lines().nth(1).unwrap().starts_with("3\t"));
}

#[test]
fn a_read_report_leaves_the_desk() {
    let setup = Setup::new("read");
    setup.post("systemd", "backup.service failed");
    setup.post("systemd", "sync.service failed");

    succeeded(&setup.priory(&["inbox", "read", "1"]));

    assert_eq!(
        reports(&setup.json(&["desk", "--json"])),
        ["systemd: sync.service failed"]
    );
    let inbox = &setup.json(&["inbox", "--json"])["inbox"];
    assert!(inbox[0]["read"].is_string());
    assert_eq!(inbox[1]["read"], Value::Null);
    assert!(!setup.priory(&["inbox", "read", "9"]).status.success());
}

#[test]
fn the_daemon_routes_no_report_and_only_a_report_can_be_read() {
    let setup = Setup::new("unrouted");
    let files = [
        ("factory/factory.toml", "name = \"demo\""),
        ("factory/agents/implementer/agent.md", AGENT),
        ("factory/workflows/implement.toml", WORKFLOW),
    ];
    for (path, content) in files {
        let file = setup.dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }
    let factory = setup.dir.join("factory");
    let factory = factory.to_str().unwrap();
    setup.post("systemd", "backup.service failed");
    setup.stdout(&[
        "request",
        factory,
        "--repo",
        "owner/repo",
        "--workflow",
        "implement",
        "--key",
        "k",
        "Fix it",
    ]);

    setup.stdout(&["daemon", factory, "--once"]);

    let inbox = &setup.json(&["inbox", "--json"])["inbox"];
    assert_eq!(inbox[0]["task"], Value::Null);
    assert_eq!(inbox[0]["outcome"], "report");
    assert_eq!(
        setup.json(&["tasks", "--json"])["tasks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let refused = setup.priory(&["inbox", "read", "2"]);
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr).trim(),
        "priory inbox read: 2 is not a report"
    );
}
