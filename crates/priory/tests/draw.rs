use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FACTORY: &str = r#"name = "demo""#;

const AGENT: &str = r#"+++
description = "Implements one task."
harness = "claude-code"
+++
# Implementation
"#;

const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
workflow = "implement"
"#;

const RUNNER: &str = r#"platform = { os = "linux" }"#;

const IMPLEMENT: &str = r#"start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change. Grade it red, yellow or green."
effect = "reversible"
next = { red = "approve-plan", yellow = "implement", green = "implement" }

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "implement", rejected = "stop" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the plan and run the tests."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

const EXPECTED: &str = r#"flowchart TD
    classDef irreversible stroke:#c00,stroke-width:3px
    n0(["plan<br/>model, reversible"])
    n1{{"approve-plan<br/>human, none"}}
    n2(["implement<br/>model, reversible"])
    n3{{"approve-pr<br/>human, none"}}
    n4["open-pr<br/>code, irreversible ⚠"]:::irreversible
    done((done))
    stop((stop))
    start((start)) --> n0
    n0 -->|"green"| n2
    n0 -->|"red"| n1
    n0 -->|"yellow"| n2
    n1 -->|"approved"| n2
    n1 -->|"rejected"| stop
    n2 -->|"failed"| stop
    n2 -->|"passed"| n3
    n3 -->|"open"| n4
    n3 -->|"stop"| stop
    n3 -.->|approves| n4
    n4 -->|"failed"| stop
    n4 -->|"opened"| done
    linkStyle 11,12 stroke:#555
    linkStyle 1,2,3,6,7 stroke:#06c
    linkStyle 4,5,8,9 stroke:#c60,stroke-width:3px
"#;

fn factory(name: &str, workflow: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("draw")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    for (path, content) in [
        ("factory.toml", FACTORY),
        ("agents/implementer/agent.md", AGENT),
        ("triggers/agent-label.toml", TRIGGER),
        ("workflows/implement.toml", workflow),
        ("runners/xps13.toml", RUNNER),
    ] {
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }
    dir
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

fn draw(dir: &Path, workflow: &str) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("draw")
        .arg(dir)
        .arg(workflow)
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[test]
fn the_example_workflow_renders_as_a_flowchart() {
    let outcome = draw(&factory("example", IMPLEMENT), "implement");

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    assert_eq!(outcome.stdout, EXPECTED);
}

#[test]
fn the_start_step_gets_an_entry_arrow_wherever_it_is_declared() {
    let cycle = r#"start = "second"

[[steps]]
id = "first"
decides = "model"
agent = "implementer"
prompt = "Go."
effect = "reversible"
next = { again = "second", passed = "done" }

[[steps]]
id = "second"
decides = "model"
agent = "implementer"
prompt = "Go."
effect = "reversible"
next = { again = "first" }
"#;
    let outcome = draw(&factory("start", cycle), "implement");

    assert!(outcome.stdout.contains("start((start)) --> n1\n"));
}

#[test]
fn an_outcome_with_syntax_characters_is_quoted_and_literal() {
    let odd = IMPLEMENT
        .replace("approved = ", r#""approved (#1 \"ok\")" = "#)
        .replace(r#""approved","#, r#""approved (#1 \"ok\")","#);
    let outcome = draw(&factory("odd", &odd), "implement");

    assert_eq!(outcome.code, 0);
    assert!(
        outcome
            .stdout
            .contains("n1 -->|\"approved (#35;1 #quot;ok#quot;)\"| n2\n")
    );
}

#[test]
fn an_unknown_workflow_fails_with_its_name() {
    let outcome = draw(&factory("unknown", IMPLEMENT), "ship");

    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.stderr, "priory draw: unknown workflow `ship`\n");
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_invalid_factory_fails_like_check() {
    let broken = IMPLEMENT.replace("opened = \"done\"", "opened = \"missing\"");
    let outcome = draw(&factory("invalid", &broken), "implement");

    assert_eq!(outcome.stdout, "");
    assert_ne!(outcome.stderr, "");
    assert_eq!(outcome.code, 1);
}

#[test]
fn an_explicit_by_restyles_its_edge() {
    let asleep = IMPLEMENT.replace(
        "next = { passed = \"approve-pr\", failed = \"stop\" }",
        "next = { passed = \"approve-pr\", failed = \"stop\" }\nby = { passed = \"asleep\" }",
    );
    let outcome = draw(&factory("by", &asleep), "implement");

    assert_eq!(outcome.stderr, "");
    assert!(
        outcome
            .stdout
            .contains("    linkStyle 1,2,3,6 stroke:#06c\n")
    );
    assert!(
        outcome
            .stdout
            .ends_with("    linkStyle 7 stroke:#888,stroke-dasharray:4 4\n")
    );
}
