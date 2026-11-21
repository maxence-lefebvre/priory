use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const FACTORY: &str = r#"name = "demo""#;

const AGENT: &str = r#"+++
description = "Implements one task."
harness = "claude-code"
session = "per-task"
mcpServers = ["github"]
permissions = { defaultMode = "auto", allow = ["Bash(cargo test:*)", "Edit"], deny = ["Bash(git push:*)"] }
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
    let nowhere = Path::new(env!("CARGO_TARGET_TMPDIR")).join("no-herdr");
    check_with_herdr(dir, &nowhere)
}

fn check_with_herdr(dir: &Path, herdr: &Path) -> Outcome {
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("check")
        .arg(dir)
        .env("HERDR_APP_HOME", herdr)
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

#[test]
fn a_yaml_rule_problem_names_its_line() {
    let files = [
        ("factory.yaml", "name: demo\n"),
        (
            "triggers/agent-label.yaml",
            "source: github\nrepo: owner/repo\non: issues.labeled\nworkflow: deploy\n",
        ),
        (
            "workflows/implement.yaml",
            "start: implement\nsteps:\n  - id: implement\n    decides: model\n    agent: reviewer\n    prompt: Implement the issue.\n    effect: reversible\n    next: { passed: done }\n",
        ),
    ];

    let outcome = check(&factory("yaml-lines", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.yaml:4: workflow `deploy` does not exist\n\
         workflows/implement.yaml:3: step `implement`: agent `reviewer` does not exist\n"
    );
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
        "agents/implementer/agent.md:4:1: unknown field `modle`, expected one of description, harness, model, effort, runner, session, compactAt, idleCompactAt, onChange, memory, disk, mcpServers, connectorSignIn, permissions\n"
    );
}

#[test]
fn an_unknown_permission_mode_names_its_file_and_line() {
    let agent = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\npermissions = { defaultMode = \"yolo\" }\n+++\n";

    let outcome = check(&factory(
        "unknown-mode",
        &with("agents/implementer/agent.md", agent),
    ));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("agents/implementer/agent.md:4:")
            && outcome.stderr.contains("yolo"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_codex_agent_passes_and_its_claude_only_permissions_are_refused_with_their_line() {
    let codex = "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\nmodel = \"gpt-6-astra\"\neffort = \"high\"\npermissions = { defaultMode = \"acceptEdits\" }\n+++\nMake the change.\n";
    let outcome = check(&factory(
        "codex",
        &with("agents/implementer/agent.md", codex),
    ));
    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));

    let claude_only = "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n\n[permissions]\ndefaultMode = \"plan\"\nallow = [\"Bash(cargo test:*)\"]\ndeny = [\"Bash(git push:*)\"]\n+++\n";
    let outcome = check(&factory(
        "codex-claude-only",
        &with("agents/implementer/agent.md", claude_only),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:5: a codex agent cannot take `permissions.defaultMode = \"plan\"`: use manual, acceptEdits, auto or dontAsk\n\
         agents/implementer/agent.md:5: a codex agent cannot take `permissions.allow`: Codex has no per-tool rules, so its sandbox and approvals follow `defaultMode` alone\n\
         agents/implementer/agent.md:5: a codex agent cannot take `permissions.deny`: Codex has no per-tool rules, so its sandbox and approvals follow `defaultMode` alone\n"
    );
}

#[test]
fn compact_at_takes_a_positive_token_count_on_a_claude_agent_only() {
    let agent = |harness: &str, compact_at: &str| {
        format!(
            "+++\ndescription = \"Leads.\"\nharness = \"{harness}\"\ncompactAt = {compact_at}\n+++\n"
        )
    };
    let outcome = check(&factory(
        "compact-at",
        &with(
            "agents/implementer/agent.md",
            &agent("claude-code", "150000"),
        ),
    ));
    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));

    for (name, compact_at) in [("zero", "0"), ("negative", "-5"), ("words", "\"lots\"")] {
        let outcome = check(&factory(
            &format!("compact-at-{name}"),
            &with(
                "agents/implementer/agent.md",
                &agent("claude-code", compact_at),
            ),
        ));
        assert_eq!(outcome.code, 1, "{compact_at}");
        assert!(
            outcome.stderr.starts_with("agents/implementer/agent.md:4:"),
            "{compact_at}: {}",
            outcome.stderr
        );
    }

    let outcome = check(&factory(
        "compact-at-codex",
        &with("agents/implementer/agent.md", &agent("codex", "150000")),
    ));
    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:4: a codex agent cannot take `compactAt`: Priory reads no Codex tokens, so it never knows the context size\n"
    );
}

#[test]
fn idle_compact_at_and_on_change_need_a_shared_claude_agent() {
    let agent = |harness: &str, session: &str, key: &str| {
        format!(
            "+++\ndescription = \"Leads.\"\nharness = \"{harness}\"\nsession = \"{session}\"\n{key}\n+++\n"
        )
    };
    let shared = agent(
        "claude-code",
        "shared",
        "idleCompactAt = 60000\nonChange = \"fresh\"",
    );
    let outcome = check(&factory(
        "idle-shared",
        &with("agents/implementer/agent.md", &shared),
    ));
    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));

    for (name, harness, session, key, refusal) in [
        (
            "idle-per-task",
            "claude-code",
            "per-task",
            "idleCompactAt = 60000",
            "agents/implementer/agent.md:5: `idleCompactAt` needs `session = \"shared\"`: only a shared session is resumed across tasks\n",
        ),
        (
            "fresh-per-task",
            "claude-code",
            "per-task",
            "onChange = \"fresh\"",
            "agents/implementer/agent.md:5: `onChange` needs `session = \"shared\"`: only a shared session is resumed across tasks\n",
        ),
        (
            "idle-codex",
            "codex",
            "shared",
            "idleCompactAt = 60000",
            "agents/implementer/agent.md:5: a codex agent cannot take `idleCompactAt`: Priory reads no Codex tokens, so it never knows the context size\n",
        ),
    ] {
        let outcome = check(&factory(
            name,
            &with("agents/implementer/agent.md", &agent(harness, session, key)),
        ));
        assert_eq!(
            (outcome.code, outcome.stderr.as_str()),
            (1, refusal),
            "{name}"
        );
    }
}

#[test]
fn the_bypass_permissions_mode_is_refused() {
    let agent = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\npermissions = { defaultMode = \"bypassPermissions\" }\n+++\n";

    let outcome = check(&factory(
        "bypass-mode",
        &with("agents/implementer/agent.md", agent),
    ));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("agents/implementer/agent.md:4:")
            && outcome
                .stderr
                .contains("Priory never skips permission checks"),
        "{}",
        outcome.stderr
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
        "triggers/agent-label.toml:4: workflow `deploy` does not exist\n"
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
        "workflows/implement.toml:4: step `implement`: agent `reviewer` does not exist\n"
    );
}

#[test]
fn plan_requires_needs_each_field_in_the_prompt_or_the_agent() {
    let required = WORKFLOW.replace(
        "prompt = \"Implement the issue.\"",
        "prompt = \"Plan the goal and its verification.\"\nplan_requires = [\"goal\", \"testing\", \"verification\"]",
    );

    let outcome = check(&factory(
        "plan-requires",
        &with("workflows/implement.toml", &required),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:4: step `implement`: `plan_requires` names `testing`, which neither the step's prompt nor its agent's instructions mention\n"
    );
    let met = required.replace("goal and its", "goal, its testing and its");
    assert_eq!(
        check(&factory(
            "plan-requires-met",
            &with("workflows/implement.toml", &met)
        ))
        .code,
        0
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
        "workflows/implement.toml:4: step `implement`: outcome `failed` leads to `retry`, which is not a step, `done` or `stop`\n"
    );
}

#[test]
fn start_must_name_a_step() {
    let outcome = workflow_with("start", "start = \"implement\"", "start = \"plan\"");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:2: start `plan` is not a step\n"
    );
}

#[test]
fn max_prs_must_be_a_positive_number() {
    let outcome = workflow_with(
        "max-prs-zero",
        "start = \"implement\"",
        "start = \"implement\"\nmax_prs = 0",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:3:11: invalid value: integer `0`, expected a nonzero usize\n"
    );
}

#[test]
fn a_workflow_with_max_prs_passes() {
    let outcome = workflow_with(
        "max-prs",
        "start = \"implement\"",
        "start = \"implement\"\nmax_prs = 3",
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn the_start_step_cannot_wait_for_a_merge() {
    let outcome = workflow_with(
        "start-after",
        "id = \"implement\"\ndecides",
        "id = \"implement\"\nafter = \"merged\"\ndecides",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:2: start `implement` waits for a merge, and a task starts before it owns a pull request\n"
    );
}

#[test]
fn reopen_must_name_a_model_step() {
    let outcome = workflow_with(
        "reopen",
        "start = \"implement\"",
        "start = \"implement\"\nreopen = \"approve-pr\"",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:3: reopen `approve-pr` is not a model step\n"
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
        "workflows/implement.toml:4: step `implement`: no path from start reaches it\n"
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
        "workflows/implement.toml:12: step `approve-pr`: option `later` has no `next`\n\
         workflows/implement.toml:12: step `approve-pr`: `next` has outcome `stop`, which is not an option\n"
    );
}

#[test]
fn a_code_step_needs_an_action() {
    let outcome = workflow_with("code-action", "action = \"open-pr\"\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:20: step `open-pr`: a code step needs an `action`\n"
    );
}

#[test]
fn a_model_step_needs_an_agent() {
    let outcome = workflow_with("model-agent", "agent = \"implementer\"\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:4: step `implement`: a model step needs an `agent`\n"
    );
}

fn unapproved(line: usize) -> String {
    format!(
        "workflows/implement.toml:{line}: step `open-pr`: irreversible, and a path from start reaches it with no human step that approves it\n"
    )
}

#[test]
fn an_irreversible_step_needs_approval() {
    let outcome = workflow_with("no-approval", "approves = [\"open-pr\"]\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.stderr, unapproved(19));
}

#[test]
fn every_path_to_an_irreversible_step_needs_approval() {
    let outcome = workflow_with(
        "bypass",
        "next = { passed = \"approve-pr\", failed = \"stop\" }",
        "next = { passed = \"approve-pr\", failed = \"open-pr\" }",
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.stderr, unapproved(20));
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
        "workflows/implement.toml:12: step `approve-pr`: approves `implement`, which is not an irreversible step\n"
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
        "workflows/implement.toml:27: step `open-pr` is defined twice\n"
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
            "workflows/implement.toml:19: step `open-pr`: `autonomous` needs a reason\n{}",
            unapproved(19)
        )
    );
}

#[test]
fn a_model_step_needs_a_prompt() {
    let outcome = workflow_with("model-prompt", "prompt = \"Implement the issue.\"\n", "");

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:4: step `implement`: a model step needs a `prompt`\n"
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
        "workflows/implement.toml:20: step `open-pr`: `next` is empty\n"
    );
}

#[test]
fn a_run_action_needs_a_command_and_only_a_run_action_has_one() {
    let bare = workflow_with("run-bare", "action = \"open-pr\"", "action = \"run\"");
    let foreign = workflow_with(
        "run-foreign",
        "action = \"open-pr\"",
        "action = \"open-pr\"\nrun = \"true\"",
    );
    let model = workflow_with(
        "run-model",
        "agent = \"implementer\"",
        "agent = \"implementer\"\nrun = \"true\"",
    );

    assert_eq!(bare.code, 1);
    assert_eq!(
        bare.stderr,
        "workflows/implement.toml:20: step `open-pr`: a `run` action needs a `run` command\n"
    );
    assert_eq!(
        foreign.stderr,
        "workflows/implement.toml:20: step `open-pr`: only a `run` action has a `run` command\n"
    );
    assert_eq!(
        model.stderr,
        "workflows/implement.toml:4: step `implement`: only a code step has a `run` command\n"
    );
}

#[test]
fn a_human_steps_command_needs_a_program_a_known_target_and_runs_among_its_options() {
    let human = workflow_with(
        "step-command",
        "options = [\"open\", \"stop\"]",
        "options = [\"open\", \"stop\"]\ncommand = { argv = [], target = \"ops\", runs = [\"later\"] }",
    );
    let model = workflow_with(
        "model-command",
        "prompt = \"Implement the issue.\"",
        "prompt = \"Implement the issue.\"\ncommand = { argv = [\"true\"], runs = [\"passed\"] }",
    );

    assert_eq!(human.code, 1);
    assert_eq!(
        human.stderr,
        "workflows/implement.toml:12: step `approve-pr`: command argv must start with a program\n\
         workflows/implement.toml:12: step `approve-pr`: command target `ops` is not worktree nor a name in [asks.targets]\n\
         workflows/implement.toml:12: step `approve-pr`: command runs `later`, which is not an option\n"
    );
    assert_eq!(
        model.stderr,
        "workflows/implement.toml:4: step `implement`: only a human step has a `command`\n"
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
        "workflows/implement.toml:20: step `open-pr`: only a model step has an `agent`\n"
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
        "workflows/implement.toml:27: step id `stop` is reserved for the end of a workflow\n"
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
        "triggers/agent-label.toml:2: a GitHub trigger needs `repo = \"owner/name\"`\n"
    );
}

#[test]
fn a_trigger_repo_that_could_leave_priorys_home_is_refused() {
    for repo in ["owner/..", "/tmp/x", "-x/y", "owner/a b"] {
        let outcome = check(&factory(
            "github-repo-escape",
            &with(
                "triggers/agent-label.toml",
                &TRIGGER.replace("owner/repo", repo),
            ),
        ));

        assert_eq!(outcome.code, 1, "{repo}");
        assert_eq!(
            outcome.stderr,
            "triggers/agent-label.toml:3: a GitHub trigger needs `repo = \"owner/name\"`\n",
            "{repo}"
        );
    }
}

#[test]
fn a_cli_trigger_repo_that_could_leave_priorys_home_is_refused() {
    let cli = "source = \"cli\"\nrepo = \"/tmp/victim\"\nworkflow = \"implement\"\n";
    let outcome = check(&factory("cli-repo-escape", &with("triggers/cli.toml", cli)));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/cli.toml:2: a trigger's `repo` is `owner/name`\n"
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
        "triggers/agent-label.toml:5: `filter` works only with `on = \"*.labeled\"`, on `label`\n\
         triggers/agent-label.toml:7: a GitHub trigger's `key` is `issue.number`\n"
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
fn only_a_github_linear_or_slack_trigger_has_backfill() {
    let trigger =
        "source = \"email\"\non = \"message\"\nworkflow = \"implement\"\nbackfill = true\n";

    let outcome = check(&factory(
        "backfill-source",
        &with("triggers/agent-label.toml", trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:4: only a GitHub, Linear or Slack trigger has `backfill`\n"
    );
}

#[test]
fn a_pull_request_trigger_starts_from_opened_or_labeled() {
    let labeled = TRIGGER
        .replace("issues.labeled", "pull_request.labeled")
        .replace("issue.number", "pull_request.number");
    let opened = "source = \"github\"\nrepo = \"owner/repo\"\non = \"pull_request.opened\"\nworkflow = \"implement\"\nkey = \"pull_request.number\"\n";
    let mut files = with("triggers/agent-label.toml", &labeled);
    files.push(("triggers/pr-opened.toml", opened));

    let outcome = check(&factory("pull-request-on", &files));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_pull_request_trigger_rejects_other_events_and_keys() {
    let trigger = "source = \"github\"\nrepo = \"owner/repo\"\non = \"pull_request.closed\"\nworkflow = \"implement\"\n";
    let keyed = TRIGGER.replace("issues.labeled", "pull_request.labeled");
    let mut files = with("triggers/agent-label.toml", trigger);
    files.push(("triggers/keyed.toml", &keyed));

    let outcome = check(&factory("pull-request-closed", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:3: a GitHub trigger handles `issues.labeled`, `issues.opened`, `pull_request.labeled`, `pull_request.opened` or `pull_request.review_requested`, not `pull_request.closed`\n\
         triggers/keyed.toml:7: a GitHub trigger's `key` is `pull_request.number`\n"
    );
}

#[test]
fn a_step_timeout_must_be_a_duration_in_seconds_minutes_or_hours() {
    let ok = workflow_with(
        "timeout-ok",
        "prompt = \"Implement the issue.\"\n",
        "prompt = \"Implement the issue.\"\ntimeout = \"45m\"\n",
    );
    let bad = workflow_with(
        "timeout-bad",
        "prompt = \"Implement the issue.\"\n",
        "prompt = \"Implement the issue.\"\ntimeout = \"soon\"\n",
    );

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(bad.code, 1);
    assert!(
        bad.stderr
            .contains("\"soon\" is not a duration such as 90s, 15m or 2h"),
        "{}",
        bad.stderr
    );
}

#[test]
fn a_step_timeout_too_large_to_count_is_refused() {
    let bad = workflow_with(
        "timeout-huge",
        "prompt = \"Implement the issue.\"\n",
        "prompt = \"Implement the issue.\"\ntimeout = \"18446744073709551615h\"\n",
    );

    assert_eq!(bad.code, 1);
    assert!(
        bad.stderr.contains("is not a duration such as"),
        "{}",
        bad.stderr
    );
}

const SCHEDULE: &str = r#"source = "schedule"
repo = "owner/repo"
at = "09:00"
days = ["mon", "tue", "wed", "thu", "fri"]
every = "1h"
until = "18:00"
timezone = "Europe/Paris"
key = "support-day"
prompt = "Sweep the support queue."
workflow = "implement"
"#;

fn schedule(name: &str, trigger: &str) -> Outcome {
    let mut files = valid();
    files.push(("triggers/support-day.toml", trigger));
    check(&factory(name, &files))
}

#[test]
fn a_schedule_trigger_passes() {
    let outcome = schedule("schedule-valid", SCHEDULE);

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

#[test]
fn a_bad_schedule_field_names_its_line() {
    let cases = [
        (
            "at = \"09:00\"",
            "at = \"9am\"",
            "triggers/support-day.toml:3:6: \"9am\" is not a time of day such as \"09:00\"\n",
        ),
        (
            "\"fri\"]",
            "\"fry\"]",
            "triggers/support-day.toml:4:8: \"fry\" is not a day: use mon, tue, wed, thu, fri, sat or sun\n",
        ),
        (
            "every = \"1h\"",
            "every = \"hourly\"",
            "triggers/support-day.toml:5:9: \"hourly\" is not a duration such as 90s, 15m or 2h\n",
        ),
        (
            "until = \"18:00\"",
            "until = \"evening\"",
            "triggers/support-day.toml:6:9: \"evening\" is not a time of day such as \"18:00\" or a duration such as 8h\n",
        ),
        (
            "Europe/Paris",
            "Europe/Atlantis",
            "triggers/support-day.toml:7:12: \"Europe/Atlantis\" is not a time zone such as \"Europe/Paris\"\n",
        ),
        (
            "until = \"18:00\"",
            "until = \"08:00\"",
            "triggers/support-day.toml:6: `until` is before `at`, so it never fires\n",
        ),
        (
            "at = \"09:00\"\n",
            "",
            "triggers/support-day.toml:1: a schedule trigger needs `at = \"HH:MM\"`\n",
        ),
        (
            "repo = \"owner/repo\"\n",
            "",
            "triggers/support-day.toml:1: a schedule trigger needs `repo = \"owner/name\"`\n",
        ),
    ];
    for (index, (from, to, error)) in cases.into_iter().enumerate() {
        let outcome = schedule(
            &format!("schedule-bad-{index}"),
            &SCHEDULE.replace(from, to),
        );

        assert_eq!(
            (outcome.code, outcome.stderr.as_str()),
            (1, error),
            "{to:?}"
        );
    }
}

#[test]
fn only_a_schedule_trigger_has_schedule_fields() {
    let trigger = format!("{TRIGGER}at = \"09:00\"\nprompt = \"Sweep.\"\n");

    let outcome = check(&factory(
        "schedule-fields-on-github",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(
        (outcome.code, outcome.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:8: only a schedule trigger has `at`\n\
             triggers/agent-label.toml:9: only a schedule trigger has `prompt`\n"
        )
    );
}

#[test]
fn a_schedule_repeats_at_most_once_a_minute() {
    let outcome = schedule(
        "schedule-every-seconds",
        &SCHEDULE.replace("every = \"1h\"", "every = \"30s\""),
    );

    assert_eq!(
        (outcome.code, outcome.stderr.as_str()),
        (
            1,
            "triggers/support-day.toml:5: `every` is shorter than the intake's 1-minute poll\n"
        )
    );
}

#[test]
fn a_github_trigger_without_on_names_its_source_line() {
    let trigger = TRIGGER.replace("on = \"issues.labeled\"\n", "");

    let outcome = check(&factory(
        "github-no-on",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome
            .stderr
            .starts_with("triggers/agent-label.toml:2: a GitHub trigger handles"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_negative_or_infinite_price_names_its_file_and_line() {
    for (name, rate) in [("negative-price", "-1.0"), ("infinite-price", "inf")] {
        let prices = format!(
            "[\"claude-opus-5-5\"]\ninput = 5.0\noutput = {rate}\ncache_read = 0.5\ncache_write = 6.25\n"
        );

        let outcome = check(&factory(name, &with("prices.toml", &prices)));

        assert_eq!(outcome.code, 1, "{name}");
        assert!(
            outcome.stderr.starts_with("prices.toml:3:")
                && outcome
                    .stderr
                    .contains("dollars per million tokens, zero or more"),
            "{}",
            outcome.stderr
        );
    }
}

const LANES: &str = r#"fallback = "{team}/misc"

[[rules]]
key = "3D-600"
lane = "3D/mobile"

[[rules]]
branch = "*desktop*"
lane = "FND/desktop"
"#;

#[test]
fn a_factory_with_lanes_passes() {
    let outcome = check(&factory("lanes-ok", &with("lanes.toml", LANES)));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

#[test]
fn a_lane_that_is_not_team_slash_theme_fails_with_its_line() {
    let lanes = LANES.replace("\"3D/mobile\"", "\"mobile\"");

    let outcome = check(&factory("lanes-bad", &with("lanes.toml", &lanes)));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "lanes.toml:3: lane `mobile` is not `<team>/<theme>`\n"
    );
}

#[test]
fn a_malformed_fallback_and_trigger_lane_fail_with_their_lines() {
    let trigger = format!("{TRIGGER}lane = \"mobile\"\n");
    let mut files = with("lanes.toml", "fallback = \"misc\"\n");
    files.retain(|(path, _)| *path != "triggers/agent-label.toml");
    files.push(("triggers/agent-label.toml", &trigger));

    let outcome = check(&factory("lanes-fallback", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:8: lane `mobile` is not `<team>/<theme>`\n\
         lanes.toml:1: lane `misc` is not `<team>/<theme>`\n"
    );
}

#[test]
fn a_lane_rule_with_an_unknown_field_fails_with_its_line() {
    let lanes = LANES.replace("branch =", "owner =");

    let outcome = check(&factory("lanes-field", &with("lanes.toml", &lanes)));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("lanes.toml:8:")
            && outcome.stderr.contains("unknown field `owner`"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_lane_rule_that_matches_on_nothing_fails_with_its_line() {
    let lanes = "[[rules]]\nlane = \"3D/mobile\"\n";

    let outcome = check(&factory("lanes-bare", &with("lanes.toml", lanes)));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "lanes.toml:1: a rule needs `key`, `team`, `project` or `branch`\n"
    );
}

#[test]
fn lanes_in_yaml_are_read_too() {
    let lanes = "rules:\n  - key: 3D-600\n    lane: mobile\n";

    let outcome = check(&factory("lanes-yaml", &with("lanes.yaml", lanes)));

    assert_eq!(
        outcome.stderr,
        "lanes.yaml:2: lane `mobile` is not `<team>/<theme>`\n"
    );
}

#[test]
fn a_lane_with_a_newline_fails_check() {
    let lanes = "fallback = \"OPS/chores\\nforged\"\n";

    let outcome = check(&factory("lanes-newline", &with("lanes.toml", lanes)));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.contains("is not `<team>/<theme>`"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_linear_trigger_uses_a_supported_event_filter_and_key() {
    let view = "source = \"linear.view\"\nview = \"6b3c2f9a1d4e\"\non = \"issue.entered\"\nworkflow = \"implement\"\nkey = \"issue.identifier\"\nbackfill = true\n";
    let issues = "source = \"linear\"\non = \"comment.created\"\nfilter = { team = \"ENG\", label = \"agent\" }\nworkflow = \"implement\"\n";
    let wrong = "source = \"linear\"\non = \"issue.entered\"\nfilter = { priority = \"x\" }\nworkflow = \"implement\"\nkey = \"issue.id\"\n";
    let mut files = with("triggers/agent-label.toml", view);
    files.push(("triggers/comments.toml", issues));
    files.push(("triggers/wrong.toml", wrong));

    let outcome = check(&factory("linear-triggers", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/wrong.toml:2: a `linear` trigger handles `issue.created`, `issue.state_changed`, `comment.created`, not `issue.entered`\n\
         triggers/wrong.toml:3: a `linear` trigger filters on `team`, `project`, `label` or `state`\n\
         triggers/wrong.toml:5: a Linear trigger's `key` is `issue.identifier`\n"
    );
}

#[test]
fn a_linear_or_slack_trigger_may_name_the_repo_its_tasks_work_in() {
    let linear = "source = \"linear\"\nrepo = \"owner/repo\"\non = \"issue.created\"\nworkflow = \"implement\"\n";
    let slack = "source = \"slack\"\nchannel = \"C0123\"\nrepo = \"owner/repo\"\non = \"message.posted\"\nworkflow = \"implement\"\n";
    let bare = "source = \"linear.view\"\nview = \"6b3c2f9a1d4e\"\nrepo = \"repo\"\non = \"issue.entered\"\nworkflow = \"implement\"\n";
    let mut files = with("triggers/agent-label.toml", linear);
    files.push(("triggers/slack.toml", slack));
    let ok = check(&factory("linear-slack-repo", &files));
    files.push(("triggers/bare.toml", bare));
    let bad = check(&factory("linear-bare-repo", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (bad.code, bad.stderr.as_str()),
        (
            1,
            "triggers/bare.toml:3: a Linear trigger's `repo` is `owner/name`\n"
        )
    );
}

#[test]
fn a_slack_trigger_names_its_channel_and_filters_on_text_only() {
    let good = "source = \"slack\"\nchannel = \"C0123\"\non = \"message.posted\"\nfilter = { text = \"deploy\" }\nworkflow = \"implement\"\nbackfill = true\n";
    let wrong = "source = \"slack\"\non = \"message\"\nfilter = { user = \"U1\" }\nworkflow = \"implement\"\nkey = \"ts\"\n";
    let stray = "source = \"linear\"\nchannel = \"C0123\"\non = \"comment.created\"\nworkflow = \"implement\"\n";
    let mut files = with("triggers/agent-label.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    files.push(("triggers/stray.toml", stray));

    let outcome = check(&factory("slack-triggers", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/stray.toml:2: only a Slack trigger names a `channel`\n\
         triggers/wrong.toml:2: a Slack trigger handles `message.posted`, not `message`\n\
         triggers/wrong.toml:1: a Slack trigger needs `channel = \"<channel id>\"`\n\
         triggers/wrong.toml:3: a Slack trigger filters on `text` only\n\
         triggers/wrong.toml:5: a Slack trigger's `key` is `message.ts`\n"
    );
}

#[test]
fn a_review_request_trigger_names_a_reviewer_and_no_repository() {
    let good = "source = \"github\"\non = \"pull_request.review_requested\"\nreviewer = \"octo-cat\"\nworkflow = \"implement\"\n";
    let wrong = "source = \"github\"\non = \"pull_request.review_requested\"\nreviewer = \"max:x\"\nrepo = \"owner/name\"\nworkflow = \"implement\"\nkey = \"pull_request.number\"\n";
    let stray = "source = \"github\"\nrepo = \"owner/name\"\non = \"issues.opened\"\nreviewer = \"max\"\nworkflow = \"implement\"\n";
    let mut files = with("triggers/agent-label.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    files.push(("triggers/stray.toml", stray));

    let outcome = check(&factory("review-triggers", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/stray.toml:4: only a GitHub `pull_request.review_requested` trigger names a `reviewer`\n\
         triggers/wrong.toml:3: a `pull_request.review_requested` trigger needs `reviewer = \"<GitHub login>\"`\n\
         triggers/wrong.toml:4: a `pull_request.review_requested` trigger takes no `repo`: its tasks work in the pull request's repository, keyed by it and the reviewer\n\
         triggers/wrong.toml:6: a `pull_request.review_requested` trigger takes no `key`: its tasks work in the pull request's repository, keyed by it and the reviewer\n"
    );
}

#[test]
fn a_review_request_trigger_names_its_teams_as_org_slash_team() {
    let good = "source = \"github\"\non = \"pull_request.review_requested\"\nreviewer = \"octo-cat\"\nteams = [\"acme/core_team\", \"acme/web-1\"]\nworkflow = \"implement\"\n";
    let wrong = "source = \"github\"\non = \"pull_request.review_requested\"\nreviewer = \"max\"\nteams = [\"core\", \"acme/a+b\"]\nworkflow = \"implement\"\n";
    let stray = "source = \"github\"\nrepo = \"owner/name\"\non = \"issues.opened\"\nteams = [\"acme/core\"]\nworkflow = \"implement\"\n";
    let mut files = with("triggers/agent-label.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    files.push(("triggers/stray.toml", stray));

    let outcome = check(&factory("review-teams", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/stray.toml:4: only a GitHub `pull_request.review_requested` trigger names `teams`\n\
         triggers/wrong.toml:4: team `core` is not `org/team`, a GitHub organization and team slug\n\
         triggers/wrong.toml:4: team `acme/a+b` is not `org/team`, a GitHub organization and team slug\n"
    );
}

#[test]
fn a_trigger_or_workflow_naming_an_unknown_band_names_its_line() {
    let trigger = format!("{TRIGGER}band = \"someday\"\n");
    let workflow = WORKFLOW.replacen(
        "start = \"implement\"\n",
        "start = \"implement\"\nband = \"urgent\"\n",
        1,
    );
    let mut files = with("triggers/agent-label.toml", &trigger);
    files.retain(|(file, _)| *file != "workflows/implement.toml");
    files.push(("workflows/implement.toml", &workflow));

    let outcome = check(&factory("unknown-band", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:8: band `someday` is not one of blocker, now, today, later\n\
         workflows/implement.toml:3: band `urgent` is not one of blocker, now, today, later\n"
    );
}

#[test]
fn factory_toml_renames_the_bands() {
    let trigger = format!("{TRIGGER}band = \"p1\"\n");
    let mut files = with("triggers/agent-label.toml", &trigger);
    files.retain(|(file, _)| *file != "factory.toml");
    files.push((
        "factory.toml",
        "name = \"demo\"\nbands = [\"p0\", \"p1\"]\n",
    ));
    let ok = check(&factory("renamed-bands", &files));
    files.retain(|(file, _)| *file != "factory.toml");
    files.push((
        "factory.toml",
        "name = \"demo\"\nbands = [\"p0\", \"p0\"]\n",
    ));
    let duplicate = check(&factory("duplicate-bands", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(duplicate.code, 1);
    assert_eq!(
        duplicate.stderr,
        "factory.toml:2: `bands` needs at least one band, each named once\n\
         triggers/agent-label.toml:8: band `p1` is not one of p0, p0\n"
    );
}

#[test]
fn a_sweep_table_passes_and_a_bad_one_names_its_line() {
    let good = "name = \"demo\"\n\n[sweep]\nevery = \"5m\"\nafter = \"30m\"\nagent = \"implementer\"\nrepo = \"owner/priory\"\nscripts = [{ run = \"docker system prune -f\", when = \"disk\", timeout = \"2m\" }]\n";
    let ok = check(&factory("sweep-ok", &with("factory.toml", good)));
    let resource = good.replace("when = \"disk\"", "when = \"swap\"");
    let bad_resource = check(&factory("sweep-resource", &with("factory.toml", &resource)));
    let agent = good.replace("agent = \"implementer\"", "agent = \"sweeper\"");
    let bad_agent = check(&factory("sweep-agent", &with("factory.toml", &agent)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(bad_resource.code, 1);
    assert!(
        bad_resource.stderr.starts_with(
            "factory.toml:8:53: unknown variant `swap`, expected one of `memory`, `disk`, `cpu`"
        ),
        "{}",
        bad_resource.stderr
    );
    assert_eq!(bad_agent.code, 1);
    assert_eq!(
        bad_agent.stderr,
        "factory.toml:3: the sweeper agent `sweeper` does not exist\n"
    );
    let repo = good.replace("owner/priory", "priory");
    let bad_repo = check(&factory("sweep-repo", &with("factory.toml", &repo)));
    assert_eq!(
        (bad_repo.code, bad_repo.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[sweep] repo` needs to be `owner/name`\n"
        )
    );
}

#[test]
fn an_agent_disk_that_is_not_a_size_names_its_line() {
    let agent = AGENT.replace(
        "session = \"per-task\"\n",
        "session = \"per-task\"\ndisk = \"lots\"\n",
    );

    let outcome = check(&factory(
        "bad-disk",
        &with("agents/implementer/agent.md", &agent),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:5:8: \"lots\" is not a size such as 512MB or 6GB\n"
    );
}

#[test]
fn a_handler_agent_passes_and_an_unknown_or_codex_one_names_its_line() {
    let good = "name = \"demo\"\nhandler = \"implementer\"\nstalled = \"10m\"\n";
    let ok = check(&factory("handler-ok", &with("factory.toml", good)));
    let unknown = good.replace("\"implementer\"", "\"watcher\"");
    let bad_agent = check(&factory("handler-agent", &with("factory.toml", &unknown)));
    let codex = "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\n";
    let mut files = with("factory.toml", good);
    files.retain(|(path, _)| *path != "agents/implementer/agent.md");
    files.push(("agents/implementer/agent.md", codex));
    let bad_harness = check(&factory("handler-codex", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (bad_agent.code, bad_agent.stderr.as_str()),
        (
            1,
            "factory.toml:2: the handler agent `watcher` does not exist\n"
        )
    );
    assert_eq!(
        (bad_harness.code, bad_harness.stderr.as_str()),
        (
            1,
            "factory.toml:2: the handler agent `implementer` needs harness = \"claude-code\"\n"
        )
    );
}

#[test]
fn a_search_table_names_a_known_summarizer_and_one_embedder() {
    let good = "name = \"demo\"\n[search]\ntldr = \"implementer\"\nembeddings = true\n";
    let ok = check(&factory("search-ok", &with("factory.toml", good)));
    let unknown = good.replace("\"implementer\"", "\"summarizer\"");
    let bad_agent = check(&factory("search-agent", &with("factory.toml", &unknown)));
    let both = format!("{good}model = \"m\"\ncommand = \"c\"\n");
    let two_embedders = check(&factory("search-both", &with("factory.toml", &both)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (bad_agent.code, bad_agent.stderr.as_str()),
        (
            1,
            "factory.toml:2: the summarizer agent `summarizer` does not exist\n"
        )
    );
    assert_eq!(
        (two_embedders.code, two_embedders.stderr.as_str()),
        (
            1,
            "factory.toml:2: [search] names both a model and a command: keep one\n"
        )
    );
}

#[test]
fn a_view_with_an_unknown_panel_names_its_file_and_line() {
    let view = "[layout]\ntabs = [\n  { panel = \"desk\" },\n  { panel = \"kanban\" },\n]\n";

    let outcome = check(&factory("bad-view", &with("views/default.toml", view)));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "views/default.toml:4:13: unknown variant `kanban`, expected one of `desk`, `asks`, `inbox`, `queue`, `tasks`, `runs`, `terminal`, `workflow`, `reports`, `costs`\n"
    );
}

#[test]
fn a_view_with_an_empty_container_names_its_file_and_line() {
    let view = "[layout]\nrow = [\n  { panel = \"desk\" },\n  { tabs = [] },\n]\n";

    let outcome = check(&factory("empty-view", &with("views/default.toml", view)));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "views/default.toml:4:3: a row, column or tabs needs at least one child\n"
    );
}

#[test]
fn check_refuses_a_bad_link_regex_a_missing_group_and_a_non_https_template() {
    let settings = r#"name = "demo"

[links]
'\b([A-Z]{2,5}-\d+)\b' = "https://linear.app/acme/issue/$1"
'(unclosed' = "https://example.com/$1"
'\bPR-(\d+)\b' = "https://example.com/$1/$2"
'\bdoc-(\d+)\b' = "http://example.com/$1"
'\bbig-(\d+)\b' = "https://example.com/$18446744073709551616"
'\bgo:(\S+)' = "https://$1/"
'\brepo:(\S+)' = "https://{repo}.example.com/$1"
'\bslash:(\S+)' = 'https://\/$1/'
"#;

    let outcome = check(&factory("bad-links", &with("factory.toml", settings)));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome
            .stderr
            .contains("factory.toml:3: `[links]` `(unclosed` is not a regex:"),
        "{}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains(
            "factory.toml:3: `[links]` `\\bPR-(\\d+)\\b` names group $2, but the regex has 1 group\n"
        ),
        "{}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains(
            "factory.toml:3: `[links]` `\\bdoc-(\\d+)\\b` links to `http://example.com/$1`: a link starts with `https://`\n"
        ),
        "{}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains(
            "factory.toml:3: `[links]` `\\bbig-(\\d+)\\b` names group $18446744073709551616, but the regex has 1 group\n"
        ),
        "{}",
        outcome.stderr
    );
    for (regex, url) in [
        ("\\bgo:(\\S+)", "https://$1/"),
        ("\\brepo:(\\S+)", "https://{repo}.example.com/$1"),
        ("\\bslash:(\\S+)", "https://\\/$1/"),
    ] {
        let message = format!(
            "factory.toml:3: `[links]` `{regex}` links to `{url}`: a link's host is fixed, with no group or `{{repo}}` in it\n"
        );
        assert!(outcome.stderr.contains(&message), "{}", outcome.stderr);
    }
    assert!(!outcome.stderr.contains("linear.app"), "{}", outcome.stderr);
}

#[test]
fn a_request_table_passes_and_an_unknown_workflow_or_band_is_refused() {
    let good = "name = \"demo\"\n\n[request]\nworkflow = \"implement\"\nband = \"today\"\n";
    let ok = check(&factory("request-ok", &with("factory.toml", good)));
    let unknown = good
        .replace("\"implement\"", "\"groom\"")
        .replace("\"today\"", "\"someday\"");
    let bad = check(&factory("request-unknown", &with("factory.toml", &unknown)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(bad.code, 1);
    assert_eq!(
        bad.stderr,
        "factory.toml:3: `[request] workflow` `groom` does not exist\n\
         factory.toml:3: `[request] band` `someday` is not one of blocker, now, today, later\n"
    );
}

#[test]
fn a_batch_workflow_needs_a_human_step_after_a_model_step() {
    let good = "name = \"demo\"\n\n[batch]\nworkflow = \"implement\"\nentries = \"implement\"\n";
    let ok = check(&factory("batch-ok", &with("factory.toml", good)));
    let plan_only = "start = \"plan\"\n\n[[steps]]\nid = \"plan\"\ndecides = \"model\"\nagent = \"implementer\"\nprompt = \"Plan.\"\neffect = \"read\"\nnext = { planned = \"done\" }\n";
    let mut files = with("factory.toml", good);
    files.push(("workflows/plan.toml", plan_only));
    let no_go_toml = good.replace("workflow = \"implement\"", "workflow = \"plan\"");
    files.retain(|(path, _)| *path != "factory.toml");
    files.push(("factory.toml", &no_go_toml));
    let no_go = check(&factory("batch-no-go", &files));
    let unknown = good.replace("entries = \"implement\"", "entries = \"groom\"");
    let bad = check(&factory("batch-unknown", &with("factory.toml", &unknown)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (no_go.code, no_go.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[batch] workflow` `plan` needs a human step after a model step: the go that starts the batch\n"
        )
    );
    assert_eq!(
        (bad.code, bad.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[batch] entries` `groom` does not exist\n"
        )
    );
}

#[test]
fn a_slack_shortcut_needs_users_with_slack_ids() {
    let good = "name = \"demo\"\n\n[slack.shortcut]\nusers = [\"U0123ABCD\", \"W0456\"]\n";
    let ok = check(&factory("shortcut-ok", &with("factory.toml", good)));
    let none = good.replace("\"U0123ABCD\", \"W0456\"", "");
    let empty = check(&factory("shortcut-empty", &with("factory.toml", &none)));
    let malformed = good.replace("W0456", "max");
    let bad = check(&factory("shortcut-bad", &with("factory.toml", &malformed)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (empty.code, empty.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[slack.shortcut] users` is empty, so nobody could use the shortcut\n"
        )
    );
    assert_eq!(
        (bad.code, bad.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[slack.shortcut] users` `max` is not a Slack user id such as U0123ABCD\n"
        )
    );
}

#[test]
fn a_hands_off_trigger_needs_a_lead() {
    let trigger = format!("{TRIGGER}intervention = \"hands-off\"\n");

    let outcome = check(&factory(
        "hands-off-no-lead",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:8: `intervention = \"hands-off\"` needs a `lead`: the agent that answers asks in the person's place\n"
    );
}

#[test]
fn a_lead_is_a_claude_code_agent_with_a_shared_session() {
    let trigger = format!("{TRIGGER}intervention = \"hands-off\"\nlead = \"implementer\"\n");

    let outcome = check(&factory(
        "per-task-lead",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:9: lead `implementer` needs harness = \"claude-code\" and session = \"shared\"\n"
    );
}

#[test]
fn a_hands_off_trigger_with_a_shared_lead_passes() {
    let lead = AGENT.replace("session = \"per-task\"", "session = \"shared\"");
    let trigger = format!("{TRIGGER}intervention = \"hands-off\"\nlead = \"lead\"\n");
    let mut files = with("triggers/agent-label.toml", &trigger);
    files.push(("agents/lead/agent.md", &lead));

    let outcome = check(&factory("shared-lead", &files));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

#[test]
fn telemetry_headers_env_names_a_variable_and_never_holds_the_headers() {
    let telemetry = "name = \"demo\"\n\n[telemetry]\notlp = \"http://127.0.0.1:4318\"\n";
    let named = format!("{telemetry}headers_env = \"OTEL_EXPORTER_OTLP_HEADERS\"\n");
    let pasted = format!("{telemetry}headers_env = \"x-honeycomb-team=s3cret\"\n");

    let ok = check(&factory("headers-env-ok", &with("factory.toml", &named)));
    let refused = check(&factory(
        "headers-env-pasted",
        &with("factory.toml", &pasted),
    ));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(refused.code, 1);
    assert_eq!(
        refused.stderr,
        "factory.toml:3: `headers_env` names the environment variable that holds the headers, \
         such as OTEL_EXPORTER_OTLP_HEADERS, never the headers themselves\n"
    );
}

#[test]
fn a_postgres_log_needs_a_build_with_the_postgres_feature() {
    let herdr = "name = \"demo\"\n[log]\nbackend = \"herdr-bus\"\n";
    let postgres = "name = \"demo\"\n[log]\nbackend = \"postgres\"\nurl_env = \"PRIORY_LOG_URL\"\n";
    let herdr = check(&factory("log-herdr", &with("factory.toml", herdr)));
    let postgres = check(&factory("log-postgres", &with("factory.toml", postgres)));

    assert_eq!((herdr.code, herdr.stderr.as_str()), (0, ""));
    let refused = "factory.toml:2: [log] backend = \"postgres\" needs a priory built with `--features postgres`\n";
    let expected = if cfg!(feature = "postgres") {
        (0, "")
    } else {
        (1, refused)
    };
    assert_eq!((postgres.code, postgres.stderr.as_str()), expected);
}

#[test]
fn a_log_url_env_names_a_variable_never_the_url() {
    let toml = "name = \"demo\"\n[log]\nurl_env = \"postgresql://user:secret@host/db\"\n";

    let outcome = check(&factory("log-url", &with("factory.toml", toml)));

    assert_eq!(
        (outcome.code, outcome.stderr.as_str()),
        (
            1,
            "factory.toml:2: `[log] url_env` names the environment variable that holds the \
             connection string, such as PRIORY_LOG_URL, never the string itself\n"
        )
    );
}

#[test]
fn the_deliver_command_is_an_executable_file_in_the_factory() {
    use std::os::unix::fs::PermissionsExt;
    let toml = "name = \"demo\"\n\n[asks]\ndeliver = \"bin/deliver\"\n";
    let missing = check(&factory("deliver-missing", &with("factory.toml", toml)));
    let mut files = with("factory.toml", toml);
    files.push(("bin/deliver", "#!/bin/sh\nexit 0\n"));
    let dir = factory("deliver-plain", &files);
    let plain = check(&dir);
    fs::set_permissions(dir.join("bin/deliver"), fs::Permissions::from_mode(0o755)).unwrap();

    let runnable = check(&dir);

    assert_eq!(
        (missing.code, missing.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[asks] deliver`: bin/deliver is not a file in the factory folder\n"
        )
    );
    assert_eq!(
        (plain.code, plain.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[asks] deliver`: bin/deliver is not executable: chmod +x it\n"
        )
    );
    assert_eq!((runnable.code, runnable.stderr.as_str()), (0, ""));
}

fn with_writing(name: &str, guide: Option<&str>) -> PathBuf {
    let mut files = valid();
    files[0] = ("factory.toml", "name = \"demo\"\nwriting = \"WRITING.md\"");
    if let Some(guide) = guide {
        files.push(("WRITING.md", guide));
    }
    factory(name, &files)
}

#[test]
fn a_writing_guide_that_does_not_exist_fails_the_check() {
    let outcome = check(&with_writing("writing-missing", None));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(
        outcome
            .stderr
            .starts_with("factory.toml:2: `writing` names WRITING.md, which does not exist"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_writing_guide_past_the_size_limit_warns_and_still_passes() {
    let at_limit = "a".repeat(4095) + "\n";
    let past = "a".repeat(4096) + "\n";

    let quiet = check(&with_writing("writing-at-limit", Some(&at_limit)));
    let warned = check(&with_writing("writing-past-limit", Some(&past)));

    assert_eq!((quiet.code, quiet.stderr.as_str()), (0, ""));
    assert_eq!(warned.code, 0, "{}", warned.stderr);
    assert_eq!(
        warned.stderr,
        "warning: WRITING.md is 4097 bytes: every agent's prompt carries it, so keep it under 4096\n"
    );
}

#[test]
fn a_writing_guide_within_the_limit_passes_quietly() {
    let outcome = check(&with_writing(
        "writing-small",
        Some("Lead with the question.\n"),
    ));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

/// Herdr's folder holding a `priory.json` whose root id is `id`.
fn herdr_with_layout(name: &str, id: &str) -> PathBuf {
    let app = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("herdr")
        .join(name);
    let _ = fs::remove_dir_all(&app);
    fs::create_dir_all(app.join("layouts")).unwrap();
    fs::write(
        app.join("layouts/priory.json"),
        format!(r#"{{"id":"{id}"}}"#),
    )
    .unwrap();
    app
}

/// The hash `adapters/herdr/layout` puts in a layout's root id for this factory's views.
fn views_hash(dir: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .args(["views", "--json"])
        .arg(dir)
        .output()
        .unwrap();
    let json = String::from_utf8(output.stdout).unwrap();
    Sha256::digest(json.trim().as_bytes())[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_herdr_layout_built_from_other_view_files_warns_and_still_passes() {
    let dir = factory("herdr-stale", &valid());
    let app = herdr_with_layout("stale", "priory-generated-000000000000");

    let outcome = check_with_herdr(&dir, &app);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr,
        format!(
            "warning: the Herdr layout {} was built from other view files: run `priory views {} --json | adapters/herdr/install` again, with --copy if you installed copies\n",
            app.join("layouts/priory.json").display(),
            dir.display()
        )
    );
}

#[test]
fn a_herdr_layout_built_from_these_view_files_or_not_by_priory_passes_quietly() {
    let dir = factory("herdr-fresh", &valid());
    let fresh = herdr_with_layout("fresh", &format!("priory-generated-{}", views_hash(&dir)));
    let foreign = herdr_with_layout("foreign", "mine");
    let lookalike = herdr_with_layout("lookalike", "priory-generated-custom");
    let missing = Path::new(env!("CARGO_TARGET_TMPDIR")).join("herdr/missing");

    for app in [fresh, foreign, lookalike, missing] {
        let outcome = check_with_herdr(&dir, &app);
        assert_eq!(
            (outcome.code, outcome.stderr.as_str()),
            (0, ""),
            "{}",
            app.display()
        );
    }
}

#[test]
fn a_herdr_layout_from_before_the_hash_warns() {
    let dir = factory("herdr-legacy", &valid());
    let app = herdr_with_layout("legacy", "priory-generated");

    let outcome = check_with_herdr(&dir, &app);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("was built from other view files"),
        "{}",
        outcome.stderr
    );
}

fn with_scorers(name: &str, scorers: &[(&str, &str)]) -> Outcome {
    let mut files = valid();
    files.extend_from_slice(scorers);
    let dir = factory(name, &files);
    let gate = dir.join("scripts/score-gate.sh");
    if gate.exists() {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o755)).unwrap();
    }
    check(&dir)
}

const PASSED: &str =
    "description = \"Passed.\"\non = \"result\"\nkind = \"outcome\"\nexpect = [\"passed\"]\n";
const GATE: &str = "description = \"The gate passes.\"\non = \"diff\"\nkind = \"command\"\nrun = \"scripts/score-gate.sh\"\ntimeout = \"20m\"\n";

#[test]
fn a_factory_with_a_scorer_of_each_kind_passes_and_counts_them() {
    let outcome = with_scorers(
        "scorers",
        &[
            ("scorers/passed.toml", PASSED),
            ("scorers/gate.toml", GATE),
            (
                "scorers/short-ask.yaml",
                "description: Short.\non: ask\nkind: ask-shape\nexpect: [nextAction]\n",
            ),
            ("scripts/score-gate.sh", "#!/bin/sh\nexit 0\n"),
        ],
    );

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 3 scorers\n"
    );
}

#[test]
fn a_judge_scorer_is_not_supported_yet_with_its_file_and_line() {
    let judge = "description = \"Does what was asked.\"\non = \"diff\"\nkind = \"judge\"\nmodel = \"sonnet\"\n";

    let outcome = with_scorers("scorer-judge", &[("scorers/judge.toml", judge)]);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "scorers/judge.toml:3: kind `judge` is not supported yet: use outcome, command or ask-shape\n"
    );
}

#[test]
fn an_unknown_scorer_kind_names_its_file_and_line() {
    let guess = "description = \"Guess.\"\non = \"result\"\nkind = \"vibes\"\n";

    let outcome = with_scorers("scorer-kind", &[("scorers/guess.toml", guess)]);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "scorers/guess.toml:3:8: unknown variant `vibes`, expected one of `outcome`, `command`, `ask-shape`\n"
    );
}

#[test]
fn an_expect_that_does_not_fit_the_kind_and_a_missing_run_name_their_lines() {
    let scorers = [
        (
            "scorers/a.toml",
            "description = \"A.\"\non = \"result\"\nkind = \"outcome\"\n",
        ),
        (
            "scorers/b.toml",
            "description = \"B.\"\non = \"ask\"\nkind = \"ask-shape\"\nexpect = [\"short\", \"polite\"]\n",
        ),
        (
            "scorers/c.toml",
            "description = \"C.\"\non = \"diff\"\nkind = \"command\"\nexpect = [\"passed\"]\n",
        ),
        (
            "scorers/d.toml",
            "description = \"D.\"\non = \"diff\"\nkind = \"command\"\nrun = \"scripts/gone.sh\"\n",
        ),
        (
            "scorers/e.toml",
            "description = \"E.\"\non = \"diff\"\nkind = \"outcome\"\nexpect = [\"passed\"]\nrun = \"x.sh\"\n",
        ),
        ("scorers/f.toml", GATE),
        ("scripts/score-gate.sh", "exit 0\n"),
    ];
    let mut files = valid();
    files.extend_from_slice(&scorers);

    let outcome = check(&factory("scorer-rules", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "scorers/a.toml: an `outcome` scorer needs `expect`: the outcomes that pass\n\
         scorers/b.toml:4: `polite` is not part of an ask's shape: use short, whatWaits, nextAction\n\
         scorers/c.toml:4: a `command` scorer takes no `expect`: its exit code is its answer\n\
         scorers/c.toml:3: a `command` scorer needs `run`: an executable in the factory folder\n\
         scorers/d.toml:4: `run` names scripts/gone.sh, which is missing from the factory folder\n\
         scorers/e.toml:2: kind `outcome` reads on = \"result\", not \"diff\"\n\
         scorers/e.toml:5: `run` is for kind `command`, not `outcome`\n\
         scorers/f.toml:4: `run` names scripts/score-gate.sh, which is not executable: chmod +x it\n"
    );
}
