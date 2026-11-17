use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FACTORY: &str = r#"name = "demo""#;

const AGENT: &str = r#"+++
description = "Implements one task."
harness = "claude-code"
session = "per-task"
mcpServers = ["github"]
+++
# Implementation
Make the smallest correct change.
"#;

const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
"#;

const WORKFLOW: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
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

const RUNNER: &str = r#"platform = { os = "linux" }"#;

fn valid() -> Vec<(&'static str, &'static str)> {
    vec![
        ("factory.toml", FACTORY),
        ("agents/implementer/agent.md", AGENT),
        ("triggers/agent-label.toml", TRIGGER),
        ("workflows/implement.toml", WORKFLOW),
        ("runners/xps13.toml", RUNNER),
    ]
}

fn factory(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("check")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    for (path, content) in files {
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

fn check(dir: &Path) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("check")
        .arg(dir)
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[test]
fn a_valid_factory_passes() {
    let outcome = check(&factory("valid", &valid()));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner\n"
    );
}

#[test]
fn a_factory_in_yaml_passes() {
    let files = [
        ("factory.yaml", "name: demo\n"),
        (
            "agents/implementer/agent.md",
            "---\ndescription: Implements one task.\nharness: claude-code\n---\n# Implementation\n",
        ),
        (
            "triggers/agent-label.yaml",
            "source: github\nrepo: owner/repo\non: issues.labeled\nworkflow: implement\n",
        ),
        (
            "workflows/implement.yaml",
            "start: implement\nsteps:\n  - id: implement\n    decides: model\n    agent: implementer\n    prompt: Implement the issue.\n    effect: reversible\n    next: { passed: done, failed: stop }\n",
        ),
        ("runners/xps13.yaml", "platform: { os: linux }\n"),
    ];

    let outcome = check(&factory("yaml", &files));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

fn with<'a>(path: &'a str, content: &'a str) -> Vec<(&'a str, &'a str)> {
    let mut files: Vec<_> = valid()
        .into_iter()
        .filter(|(file, _)| *file != path)
        .collect();
    files.push((path, content));
    files
}

#[test]
fn an_unknown_field_names_its_file_and_line() {
    let agent = "---\ndescription: Implements one task.\nharness: claude-code\nmodle: opus\n---\n";

    let outcome = check(&factory(
        "unknown-field",
        &with("agents/implementer/agent.md", agent),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:4:1: unknown field `modle`, expected one of description, harness, model, effort, runner, session, memory, mcpServers\n"
    );
}

#[test]
fn a_toml_syntax_error_names_its_file_and_line() {
    let runner = "platform = { os = \"linux\" }\nagents = { cpu = 50% }\n";

    let outcome = check(&factory("toml-syntax", &with("runners/xps13.toml", runner)));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("runners/xps13.toml:2:"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_trigger_must_name_an_existing_workflow() {
    let trigger = "source = \"github\"\nrepo = \"owner/repo\"\non = \"issues.labeled\"\nworkflow = \"deploy\"\n";

    let outcome = check(&factory(
        "trigger-workflow",
        &with("triggers/agent-label.toml", trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml: workflow `deploy` does not exist\n"
    );
}

#[test]
fn a_model_step_must_name_an_existing_agent() {
    let workflow = WORKFLOW.replace("agent = \"implementer\"", "agent = \"reviewer\"");

    let outcome = check(&factory(
        "step-agent",
        &with("workflows/implement.toml", &workflow),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `implement`: agent `reviewer` does not exist\n"
    );
}

fn workflow_with(name: &str, from: &str, to: &str) -> Outcome {
    let workflow = WORKFLOW.replace(from, to);
    check(&factory(name, &with("workflows/implement.toml", &workflow)))
}

#[test]
fn next_must_name_a_step_done_or_stop() {
    let outcome = workflow_with(
        "next-target",
        "failed = \"stop\" }\n\n[[steps]]\nid = \"approve-pr\"",
        "failed = \"retry\" }\n\n[[steps]]\nid = \"approve-pr\"",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `implement`: outcome `failed` leads to `retry`, which is not a step, `done` or `stop`\n"
    );
}

#[test]
fn start_must_name_a_step() {
    let outcome = workflow_with("start", "start = \"implement\"", "start = \"plan\"");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: start `plan` is not a step\n"
    );
}

#[test]
fn every_step_must_be_reachable_from_start() {
    let outcome = workflow_with(
        "unreachable",
        "start = \"implement\"",
        "start = \"approve-pr\"",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `implement`: no path from start reaches it\n"
    );
}

#[test]
fn a_human_step_maps_each_option_to_a_next_step() {
    let outcome = workflow_with(
        "human-options",
        "options = [\"open\", \"stop\"]",
        "options = [\"open\", \"later\"]",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `approve-pr`: option `later` has no `next`\n\
         workflows/implement.toml: step `approve-pr`: `next` has outcome `stop`, which is not an option\n"
    );
}

#[test]
fn a_code_step_needs_an_action() {
    let outcome = workflow_with("code-action", "action = \"open-pr\"\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `open-pr`: a code step needs an `action`\n"
    );
}

#[test]
fn a_model_step_needs_an_agent() {
    let outcome = workflow_with("model-agent", "agent = \"implementer\"\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `implement`: a model step needs an `agent`\n"
    );
}

const UNAPPROVED: &str = "workflows/implement.toml: step `open-pr`: irreversible, and a path from start reaches it with no human step that approves it\n";

#[test]
fn an_irreversible_step_needs_approval() {
    let outcome = workflow_with("no-approval", "approves = [\"open-pr\"]\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.stderr, UNAPPROVED);
}

#[test]
fn every_path_to_an_irreversible_step_needs_approval() {
    let outcome = workflow_with(
        "bypass",
        "next = { passed = \"approve-pr\", failed = \"stop\" }",
        "next = { passed = \"approve-pr\", failed = \"open-pr\" }",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.stderr, UNAPPROVED);
}

#[test]
fn an_autonomous_step_needs_no_approval() {
    let workflow = WORKFLOW.replace("approves = [\"open-pr\"]\n", "").replace(
        "effect = \"irreversible\"",
        "effect = \"irreversible\"\nautonomous = \"draft PRs only\"",
    );

    let outcome = check(&factory(
        "autonomous",
        &with("workflows/implement.toml", &workflow),
    ));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

#[test]
fn approves_must_name_an_irreversible_step() {
    let outcome = workflow_with(
        "approves-target",
        "approves = [\"open-pr\"]",
        "approves = [\"open-pr\", \"implement\"]",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `approve-pr`: approves `implement`, which is not an irreversible step\n"
    );
}

#[test]
fn step_ids_are_unique() {
    let workflow = format!(
        "{WORKFLOW}\n[[steps]]\nid = \"open-pr\"\ndecides = \"code\"\naction = \"comment\"\neffect = \"reversible\"\nnext = {{ done = \"done\" }}\n"
    );

    let outcome = check(&factory(
        "duplicate",
        &with("workflows/implement.toml", &workflow),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `open-pr` is defined twice\n"
    );
}

#[test]
fn a_factory_needs_its_settings_file() {
    let files: Vec<_> = valid()
        .into_iter()
        .filter(|(file, _)| *file != "factory.toml")
        .collect();

    let outcome = check(&factory("no-settings", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "factory.toml: missing: a factory needs factory.toml or factory.yaml\n"
    );
}

#[test]
fn an_autonomous_step_needs_a_reason() {
    let workflow = WORKFLOW.replace("approves = [\"open-pr\"]\n", "").replace(
        "effect = \"irreversible\"",
        "effect = \"irreversible\"\nautonomous = \" \"",
    );

    let outcome = check(&factory(
        "blank-autonomous",
        &with("workflows/implement.toml", &workflow),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        format!(
            "workflows/implement.toml: step `open-pr`: `autonomous` needs a reason\n{UNAPPROVED}"
        )
    );
}

#[test]
fn a_model_step_needs_a_prompt() {
    let outcome = workflow_with("model-prompt", "prompt = \"Implement the issue.\"\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `implement`: a model step needs a `prompt`\n"
    );
}

#[test]
fn every_step_needs_a_next() {
    let outcome = workflow_with(
        "empty-next",
        "next = { opened = \"done\", failed = \"stop\" }",
        "next = {}",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `open-pr`: `next` is empty\n"
    );
}

#[test]
fn a_step_holds_only_its_deciders_fields() {
    let outcome = workflow_with(
        "foreign-field",
        "action = \"open-pr\"",
        "action = \"open-pr\"\nagent = \"implementer\"",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step `open-pr`: only a model step has an `agent`\n"
    );
}

#[test]
fn done_and_stop_are_not_step_ids() {
    let workflow = format!(
        "{WORKFLOW}\n[[steps]]\nid = \"stop\"\ndecides = \"code\"\naction = \"comment\"\neffect = \"reversible\"\nnext = {{ ok = \"done\" }}\n"
    );

    let outcome = check(&factory(
        "reserved-id",
        &with("workflows/implement.toml", &workflow),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml: step id `stop` is reserved for the end of a workflow\n"
    );
}

#[test]
fn one_name_has_one_definition() {
    let mut files = valid();
    files.push(("workflows/implement.yaml", "start: a\nsteps: []\n"));

    let outcome = check(&factory("two-formats", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.yaml: `implement` is also defined in workflows/implement.toml\n"
    );
}

#[test]
fn an_agent_name_keeps_its_dots() {
    let mut files: Vec<_> = valid()
        .into_iter()
        .filter(|(file, _)| !file.starts_with("agents/"))
        .collect();
    files.push(("agents/implementer.v2/agent.md", AGENT));
    let workflow = WORKFLOW.replace("agent = \"implementer\"", "agent = \"implementer.v2\"");
    files.retain(|(file, _)| *file != "workflows/implement.toml");
    files.push(("workflows/implement.toml", &workflow));

    let outcome = check(&factory("dotted-agent", &files));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

#[cfg(unix)]
#[test]
fn an_unreadable_folder_fails_the_check() {
    use std::os::unix::fs::PermissionsExt;
    let dir = factory("unreadable", &valid());
    let workflows = dir.join("workflows");
    fs::set_permissions(&workflows, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&workflows).is_ok() {
        // root reads a 000 folder anyway, so this case cannot be set up.
        fs::set_permissions(&workflows, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let outcome = check(&dir);
    fs::set_permissions(&workflows, fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("workflows: "),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_github_trigger_names_its_repo() {
    let outcome = check(&factory(
        "github-repo",
        &with(
            "triggers/agent-label.toml",
            &TRIGGER.replace("repo = \"owner/repo\"\n", ""),
        ),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml: a GitHub trigger needs `repo = \"owner/name\"`\n"
    );
}

#[test]
fn a_github_trigger_uses_a_supported_event_and_filter() {
    let trigger = TRIGGER
        .replace("issues.labeled", "issues.opened")
        .replace("key = \"issue.number\"", "key = \"issue.id\"");

    let outcome = check(&factory(
        "github-on",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml: `filter` works only with `on = \"issues.labeled\"`, on `label`\n\
         triggers/agent-label.toml: a GitHub trigger's `key` is `issue.number`\n"
    );
}

#[test]
fn backfill_is_a_boolean() {
    let trigger = format!("{TRIGGER}backfill = \"yes\"\n");

    let outcome = check(&factory(
        "backfill-type",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("triggers/agent-label.toml:8:")
            && outcome.stderr.contains("expected a boolean"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn only_a_github_trigger_has_backfill() {
    let trigger = "source = \"linear.view\"\nview = \"agent-ready\"\non = \"issue.entered\"\nworkflow = \"implement\"\nbackfill = true\n";

    let outcome = check(&factory(
        "backfill-source",
        &with("triggers/agent-label.toml", trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml: only a GitHub trigger has `backfill`\n"
    );
}
