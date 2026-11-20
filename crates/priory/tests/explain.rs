use serde_json::Value;
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

const DRAFT: &str = r#"start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change."
effect = "reversible"
next = { small = "open-pr", large = "approve-plan" }

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "open-pr", rejected = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
autonomous = "draft PRs only"
next = { opened = "done", failed = "stop" }
"#;

fn factory(name: &str, workflows: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("explain")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    let workflows = workflows
        .iter()
        .map(|(name, content)| (format!("workflows/{name}.toml"), *content));
    let files = [
        ("factory.toml".to_owned(), FACTORY),
        ("agents/implementer/agent.md".to_owned(), AGENT),
        ("runners/xps13.toml".to_owned(), RUNNER),
    ];
    for (path, content) in files.into_iter().chain(workflows) {
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

fn priory(args: &[&str], dir: &Path) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .args(args)
        .arg(dir)
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn explain(dir: &Path) -> Outcome {
    priory(&["check", "--explain"], dir)
}

#[test]
fn the_example_workflow_gates_open_pr_on_approve_pr() {
    let outcome = explain(&factory("example", &[("implement", IMPLEMENT)]));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\n\
         implement\topen-pr\tapprove-pr\n\
         \n\
         workflow\tpath with no human decision\n"
    );
}

#[test]
fn an_autonomous_step_lists_its_reason_and_the_path_with_no_human_decision() {
    let outcome = explain(&factory("autonomous", &[("draft", DRAFT)]));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\n\
         draft\topen-pr\tautonomous: draft PRs only\n\
         \n\
         workflow\tpath with no human decision\n\
         draft\tplan -> open-pr\n"
    );
}

#[test]
fn json_holds_the_same_rows_as_the_table() {
    let dir = factory("json", &[("implement", IMPLEMENT), ("draft", DRAFT)]);
    let table = explain(&dir).stdout;
    let outcome = priory(&["check", "--explain", "--json"], &dir);

    assert_eq!(outcome.code, 0);
    let json: Value = serde_json::from_str(&outcome.stdout).unwrap();
    let mut rebuilt = String::from("workflow\tstep\tgate\n");
    for gate in json["irreversible"].as_array().unwrap() {
        let gated = match gate["autonomous"].as_str() {
            Some(reason) => format!("autonomous: {reason}"),
            None => gate["approved_by"][0].as_str().unwrap().to_owned(),
        };
        rebuilt += &format!(
            "{}\t{}\t{gated}\n",
            gate["workflow"].as_str().unwrap(),
            gate["step"].as_str().unwrap()
        );
    }
    rebuilt += "\nworkflow\tpath with no human decision\n";
    for path in json["paths"].as_array().unwrap() {
        let steps: Vec<_> = path["path"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| step.as_str().unwrap())
            .collect();
        rebuilt += &format!(
            "{}\t{}\n",
            path["workflow"].as_str().unwrap(),
            steps.join(" -> ")
        );
    }
    assert_eq!(rebuilt, table);
    assert_eq!(json["irreversible"].as_array().unwrap().len(), 2);
}

#[test]
fn an_explicit_by_overrides_the_steps_decider() {
    let gated = DRAFT.replace(
        "next = { small = \"open-pr\", large = \"approve-plan\" }",
        "next = { small = \"open-pr\", large = \"approve-plan\" }\nby = { small = \"human\" }",
    );
    let outcome = explain(&factory("by", &[("draft", &gated)]));

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "workflow\tstep\tgate\n\
         draft\topen-pr\tautonomous: draft PRs only\n\
         \n\
         workflow\tpath with no human decision\n"
    );
}

#[test]
fn a_by_outcome_must_be_in_next() {
    let wrong = DRAFT.replace(
        "next = { small = \"open-pr\", large = \"approve-plan\" }",
        "next = { small = \"open-pr\", large = \"approve-plan\" }\nby = { tiny = \"asleep\" }",
    );
    let outcome = priory(&["check"], &factory("by-unknown", &[("draft", &wrong)]));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/draft.toml:3: step `plan`: `by` has outcome `tiny`, which is not in `next`\n"
    );
}
