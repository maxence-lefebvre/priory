#[allow(dead_code)]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};

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
        ("runners/desktop.toml", RUNNER),
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
    let output = harness::priory()
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
        ("runners/desktop.yaml", "platform: { os: linux }\n"),
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
        "agents/implementer/agent.md:4:1: unknown field `modle`, expected one of description, harness, model, effort, runner, session, compactAt, idleCompactAt, onChange, memory, disk, mcpServers, skills, connectorSignIn, investigator, permissions\n"
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
fn compact_at_takes_a_positive_token_count_on_a_claude_or_codex_agent() {
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
    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
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
            "agents/implementer/agent.md:5: a codex agent cannot take `idleCompactAt`: a resumed Codex session sends no `SessionStart` before its first prompt, so Priory never knows when to type `/compact`\n",
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

    let outcome = check(&factory(
        "toml-syntax",
        &with("runners/desktop.toml", runner),
    ));

    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.starts_with("runners/desktop.toml:2:"),
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
fn a_human_steps_question_and_command_name_only_fields_of_the_tasks_event() {
    let named = workflow_with(
        "step-fields",
        "options = [\"open\", \"stop\"]",
        "options = [\"open\", \"stop\"]\nquestion = \"Compare {pull_request} at {head_sha} by {author}?\"\ncommand = { argv = [\"priory\", \"--head\", \"{head_sha}\", \"--post\", \"{repo}#{pull_request}\", \"{title}\", \"{}\"], target = \"worktree\", runs = [\"open\"] }",
    );
    let model = workflow_with(
        "model-question",
        "prompt = \"Implement the issue.\"",
        "prompt = \"Implement the issue.\"\nquestion = \"Implement?\"",
    );

    assert_eq!(named.code, 1);
    assert_eq!(
        named.stderr,
        "workflows/implement.toml:12: step `approve-pr`: question names `{author}`, which is not a field of the task's event: use {repo}, {pull_request}, {head_sha} or {base_branch}\n\
         workflows/implement.toml:12: step `approve-pr`: command argv names `{title}`, which is not a field of the task's event: use {repo}, {pull_request}, {head_sha} or {base_branch}\n"
    );
    assert_eq!(
        model.stderr,
        "workflows/implement.toml:4: step `implement`: only a human step has a `question`\n"
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
fn preempt_names_bands_the_factory_has_above_its_last() {
    let good = "name = \"demo\"\n\n[queue]\npreempt = [\"blocker\"]\n";
    let ok = check(&factory("preempt-ok", &with("factory.toml", good)));
    let unknown = good.replace("[\"blocker\"]", "[\"urgent\"]");
    let unknown = check(&factory("preempt-unknown", &with("factory.toml", &unknown)));
    let last = good.replace("[\"blocker\"]", "[\"blocker\", \"later\"]");
    let last = check(&factory("preempt-last", &with("factory.toml", &last)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(unknown.code, 1);
    assert_eq!(
        unknown.stderr,
        "factory.toml:3: `preempt` names band `urgent`, which is not one of blocker, now, today, later\n"
    );
    assert_eq!(last.code, 1);
    assert_eq!(
        last.stderr,
        "factory.toml:3: `preempt` names band `later`, the last band, which has no band below it\n"
    );
}

#[test]
fn queue_age_takes_a_duration() {
    let good = "name = \"demo\"\n\n[queue]\nage = \"24h\"\n";
    let ok = check(&factory("age-ok", &with("factory.toml", good)));
    let day = good.replace("\"24h\"", "\"1 day\"");
    let day = check(&factory("age-day", &with("factory.toml", &day)));
    let zero = good.replace("\"24h\"", "\"0h\"");
    let zero = check(&factory("age-zero", &with("factory.toml", &zero)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(day.code, 1);
    assert!(
        day.stderr
            .starts_with("factory.toml:4:7: \"1 day\" is not a duration such as 90s, 15m or 2h"),
        "{}",
        day.stderr
    );
    assert_eq!(zero.code, 1);
    assert!(
        zero.stderr.contains("\"0h\" is not a duration such as"),
        "{}",
        zero.stderr
    );
}

#[test]
fn review_limit_takes_a_positive_number() {
    let good = "name = \"demo\"\n\n[queue]\nreview_limit = 5\n";
    let ok = check(&factory("review-limit-ok", &with("factory.toml", good)));
    let zero = good.replace("= 5", "= 0");
    let zero = check(&factory("review-limit-zero", &with("factory.toml", &zero)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(zero.code, 1);
    assert_eq!(
        zero.stderr,
        "factory.toml:4:16: invalid value: integer `0`, expected a nonzero u32\n"
    );
}

#[test]
fn fair_lanes_and_max_running_pass_and_a_bad_one_names_its_line() {
    let settings = "name = \"demo\"\n\n[queue]\nfair = \"lane\"\n";
    let lanes = format!("{LANES}\n[lane.\"3D/mobile\"]\nmax_running = 2\n");
    let trigger = format!("{TRIGGER}max_running = 3\n");
    let files = |settings: &str, lanes: &str, trigger: &str| -> Vec<(String, String)> {
        let mut files: Vec<_> = valid()
            .into_iter()
            .filter(|(file, _)| !["factory.toml", "triggers/agent-label.toml"].contains(file))
            .map(|(file, content)| (file.to_owned(), content.to_owned()))
            .collect();
        files.push(("factory.toml".into(), settings.into()));
        files.push(("lanes.toml".into(), lanes.into()));
        files.push(("triggers/agent-label.toml".into(), trigger.into()));
        files
    };
    let run = |name: &str, files: Vec<(String, String)>| {
        let files: Vec<_> = files
            .iter()
            .map(|(file, content)| (file.as_str(), content.as_str()))
            .collect();
        check(&factory(name, &files))
    };

    let ok = run("fair-ok", files(settings, &lanes, &trigger));
    let unknown = run(
        "fair-unknown",
        files(
            &settings.replace("\"lane\"", "\"trigger\""),
            &lanes,
            &trigger,
        ),
    );
    let zero = run(
        "max-running-zero",
        files(settings, &lanes, &trigger.replace("= 3", "= 0")),
    );
    let bad_lane = run(
        "max-running-lane",
        files(
            settings,
            &lanes.replace("\"3D/mobile\"]", "\"mobile\"]"),
            &trigger,
        ),
    );

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(unknown.code, 1);
    assert!(
        unknown.stderr.starts_with("factory.toml:4:")
            && unknown
                .stderr
                .contains("unknown variant `trigger`, expected `lane`"),
        "{}",
        unknown.stderr
    );
    assert_eq!(zero.code, 1);
    assert!(
        zero.stderr.starts_with("triggers/agent-label.toml:8:")
            && zero
                .stderr
                .contains("`max_running` is 0: give at least 1, or hold the tasks instead"),
        "{}",
        zero.stderr
    );
    assert_eq!(bad_lane.code, 1);
    assert_eq!(
        bad_lane.stderr,
        "lanes.toml:11: `[lane]` `mobile` is not a lane such as `team/theme`\n"
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
fn repository_recipes_pass_and_a_bad_one_names_its_line() {
    let good = "name = \"demo\"\n\n[repos.\"owner/repo\"]\nsetup = { argv = [\"npm\", \"ci\"], timeout = \"15m\" }\nresume = { shell = \"npm ci --prefer-offline\" }\nteardown = { shell = \"docker compose down\" }\n";
    let ok = check(&factory("recipes-ok", &with("factory.toml", good)));
    let bad = |name: &str, from: &str, to: &str| {
        let text = good.replace(from, to);
        check(&factory(name, &with("factory.toml", &text)))
    };
    let both = bad(
        "recipes-both",
        "argv = [\"npm\", \"ci\"]",
        "argv = [\"npm\"], shell = \"npm ci\"",
    );
    let empty = bad("recipes-empty", "argv = [\"npm\", \"ci\"]", "argv = []");
    let repo = bad("recipes-repo", "owner/repo", "repo");
    let merge = bad(
        "recipes-merge",
        "docker compose down",
        "gh pr merge --squash",
    );
    let unknown = bad("recipes-unknown", "teardown =", "cleanup =");

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (both.code, both.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[repos.\"owner/repo\"] setup` needs one of `argv` or `shell`, not both\n"
        )
    );
    assert_eq!(
        (empty.code, empty.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[repos.\"owner/repo\"] setup` needs a program in `argv`\n"
        )
    );
    assert_eq!(
        (repo.code, repo.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[repos]` key `repo` needs to be `owner/name`\n"
        )
    );
    assert_eq!(
        (merge.code, merge.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[repos.\"owner/repo\"] teardown` runs `gh pr merge --squash`, which the guard `gh-pr-merge` holds irreversible: a recipe runs with no person's approval\n"
        )
    );
    assert_eq!(unknown.code, 1);
    assert!(
        unknown.stderr.contains("unknown field `cleanup`"),
        "{}",
        unknown.stderr
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
fn a_low_auto_answer_level_fails_with_its_line() {
    let trigger = format!("{TRIGGER}auto_answer = \"low\"\n");

    let outcome = check(&factory(
        "auto-answer-low",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:8: `auto_answer = \"low\"` would answer every ask an agent makes: use `high` or `medium`, or leave it unset\n"
    );
}

#[test]
fn an_unknown_auto_answer_level_fails_with_its_line() {
    let trigger = format!("{TRIGGER}auto_answer = \"always\"\n");

    let outcome = check(&factory(
        "auto-answer-unknown",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:8:15: unknown variant `always`, expected one of `low`, `medium`, `high`\n"
    );
}

#[test]
fn a_high_or_medium_auto_answer_level_passes() {
    for level in ["high", "medium"] {
        let trigger = format!("{TRIGGER}auto_answer = \"{level}\"\n");

        let outcome = check(&factory(
            &format!("auto-answer-{level}"),
            &with("triggers/agent-label.toml", &trigger),
        ));

        assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
    }
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
fn check_refuses_an_unknown_metrics_group_a_short_interval_and_a_zero_cap_naming_each_value() {
    let telemetry = "name = \"demo\"\n\n[telemetry]\notlp = \"http://127.0.0.1:4318\"\n";
    let ok = format!(
        "{telemetry}metrics = [\"process\", \"log\", \"runs\"]\nmetrics_every = \"10s\"\nmetrics_cap = 1\n"
    );
    let refused = format!(
        "{telemetry}metrics = [\"process\", \"nope\"]\nmetrics_every = \"9s\"\nmetrics_cap = 0\n"
    );

    let ok = check(&factory("metrics-ok", &with("factory.toml", &ok)));
    let refused = check(&factory("metrics-refused", &with("factory.toml", &refused)));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(refused.code, 1);
    assert_eq!(
        refused.stderr,
        "factory.toml:3: `metrics` names `nope`, which is not a metrics group: the groups are \
         process, log, runs\n\
         factory.toml:3: `metrics_every` is 9s: it needs to be 10s or more\n\
         factory.toml:3: `metrics_cap` is 0: it needs to be 1 or more\n"
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
    let output = harness::priory()
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

const JUDGE: &str = "description = \"Does what was asked.\"\non = \"diff\"\nkind = \"judge\"\nmodel = \"opus\"\nprompt = \"asked.md\"\nevaluation = \"asked.eval.toml\"\nagreement = 0.8\n";

/// `pass` labeled cases, then `fail` ones, as a judge's evaluation file.
fn labeled(pass: usize, fail: usize) -> String {
    (0..pass + fail)
        .map(|n| {
            let verdict = if n < pass { "pass" } else { "fail" };
            format!("[[cases]]\nname = \"c{n}\"\nrequest = \"Add a flag.\"\ndiff = \"+flag\"\nverdict = \"{verdict}\"\n")
        })
        .collect()
}

#[test]
fn a_judge_with_its_prompt_and_ten_labeled_cases_passes_and_its_cases_are_no_scorer() {
    let cases = labeled(5, 5);

    let outcome = with_scorers(
        "scorer-judge",
        &[
            ("scorers/asked.toml", JUDGE),
            (
                "scorers/asked.md",
                "Does the change do what the request asks?\n",
            ),
            ("scorers/asked.eval.toml", &cases),
        ],
    );

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 1 scorer\n"
    );
}

#[test]
fn a_judge_without_a_prompt_an_evaluation_or_enough_labeled_cases_is_refused() {
    let few = labeled(8, 2);
    let bare = "description = \"Bare.\"\non = \"diff\"\nkind = \"judge\"\n";
    let odd = "description = \"Odd.\"\non = \"ask\"\nkind = \"judge\"\nmodel = \"opus\"\nprompt = \"../asked.md\"\nevaluation = \"asked.eval.toml\"\nagreement = 1.5\nexpect = [\"pass\"]\n";
    let wrong = "description = \"Wrong.\"\non = \"result\"\nkind = \"outcome\"\nexpect = [\"passed\"]\nmodel = \"opus\"\n";

    let outcome = with_scorers(
        "scorer-judge-refused",
        &[
            ("scorers/asked.toml", JUDGE),
            ("scorers/asked.eval.toml", &few),
            ("scorers/bare.toml", bare),
            ("scorers/odd.toml", odd),
            ("scorers/wrong.toml", wrong),
        ],
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "scorers/asked.toml:5: `prompt` names asked.md, which is missing from scorers/\n\
         scorers/asked.toml:6: `evaluation` asked.eval.toml: a judge needs at least 10 labeled cases, 3 of them pass and 3 fail: this file has 10 (8 pass, 2 fail)\n\
         scorers/bare.toml:3: a `judge` scorer needs `model`: the model that grades\n\
         scorers/bare.toml:3: a `judge` scorer needs `prompt`: a file beside it with the judge's whole prompt\n\
         scorers/bare.toml:3: a `judge` scorer needs `evaluation`: a file beside it with its labeled cases\n\
         scorers/odd.toml:2: kind `judge` reads on = \"result\" or \"diff\", not \"ask\"\n\
         scorers/odd.toml:8: a `judge` scorer takes no `expect`: its answer is pass or fail\n\
         scorers/odd.toml:5: `prompt` names ../asked.md, which is not beside the scorer\n\
         scorers/odd.toml:6: `evaluation` asked.eval.toml: a judge needs at least 10 labeled cases, 3 of them pass and 3 fail: this file has 10 (8 pass, 2 fail)\n\
         scorers/odd.toml:7: `agreement` is 1.5: give a share above 0 and at most 1, such as 0.8\n\
         scorers/wrong.toml:5: `model` is for kind `judge`, not `outcome`\n"
    );
}

const SHORT: &str =
    "description = \"Short.\"\non = \"ask\"\nkind = \"ask-shape\"\nexpect = [\"short\"]\n";

/// The valid factory's workflow, with `gate` on its model step and `human` on its human step.
fn gated_workflow(gate: &str, human: &str) -> String {
    WORKFLOW
        .replace(
            "next = { passed = \"approve-pr\", failed = \"stop\" }",
            &format!("next = {{ passed = \"approve-pr\", failed = \"stop\" }}\ngate = {gate}"),
        )
        .replace(
            "next = { open = \"open-pr\", stop = \"stop\" }",
            &format!("next = {{ open = \"open-pr\", stop = \"stop\" }}\n{human}"),
        )
}

#[test]
fn a_step_gate_of_scorers_on_a_result_or_a_diff_passes() {
    let workflow = gated_workflow(r#"{ passed = ["passed", "gate"] }"#, "");
    let mut files = with("workflows/implement.toml", &workflow);
    files.extend_from_slice(&[
        ("scorers/passed.toml", PASSED),
        ("scorers/gate.toml", GATE),
        ("scripts/score-gate.sh", "#!/bin/sh\nexit 0\n"),
    ]);
    let dir = factory("gate-valid", &files);
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        dir.join("scripts/score-gate.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    let outcome = check(&dir);

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_gate_naming_an_unknown_scorer_an_ask_scorer_or_an_outcome_not_in_next_is_refused() {
    let workflow = gated_workflow(
        r#"{ passed = ["missing", "short"], merged = ["passed"] }"#,
        r#"gate = { open = ["passed"] }"#,
    );
    let mut files = with("workflows/implement.toml", &workflow);
    files.extend_from_slice(&[
        ("scorers/passed.toml", PASSED),
        ("scorers/short.toml", SHORT),
    ]);

    let outcome = check(&factory("gate-refused", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:4: step `implement`: `gate` has outcome `merged`, which is not in `next`\n\
         workflows/implement.toml:4: step `implement`: `gate` of outcome `passed` names `missing`, which is no scorer in scorers/\n\
         workflows/implement.toml:4: step `implement`: `gate` of outcome `passed` names `short`, which reads asks: a gate grades a result, so use a scorer on result or diff\n\
         workflows/implement.toml:13: step `approve-pr`: only a model step has a `gate`\n"
    );
}

/// A judge on `result`, with its prompt and ten labeled cases, as a plan's critic.
fn critic_files(on: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "scorers/critic.toml",
            format!(
                "description = \"The plan is sound.\"\non = \"{on}\"\nkind = \"judge\"\nmodel = \"opus\"\nprompt = \"critic.md\"\nevaluation = \"critic.eval.toml\"\n"
            ),
        ),
        ("scorers/critic.md", "Is the plan sound?\n".to_owned()),
        ("scorers/critic.eval.toml", labeled(5, 5)),
    ]
}

#[test]
fn a_critic_names_a_scorer_on_result_such_as_a_judge_of_the_plan_alone() {
    let critic = critic_files("result");
    let mut files = with("factory.toml", "name = \"demo\"\ncritic = \"critic\"\n");
    files.extend(critic.iter().map(|(path, text)| (*path, text.as_str())));

    let outcome = check(&factory("critic-valid", &files));

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 1 scorer\n"
    );
}

#[test]
fn a_critic_that_is_no_scorer_or_reads_a_diff_is_refused() {
    let missing = with("factory.toml", "name = \"demo\"\ncritic = \"nobody\"\n");
    let critic = critic_files("diff");
    let mut on_diff = with("factory.toml", "name = \"demo\"\ncritic = \"critic\"\n");
    on_diff.extend(critic.iter().map(|(path, text)| (*path, text.as_str())));

    let missing = check(&factory("critic-missing", &missing));
    let on_diff = check(&factory("critic-diff", &on_diff));

    assert_eq!(missing.code, 1);
    assert_eq!(
        missing.stderr,
        "factory.toml:2: the critic `nobody` is no scorer in scorers/\n"
    );
    assert_eq!(on_diff.code, 1);
    assert_eq!(
        on_diff.stderr,
        "factory.toml:2: the critic `critic` reads on = \"diff\": a critic reads a plan, which names no commit, so use one on result\n"
    );
}

#[test]
fn a_judge_whose_model_an_agent_also_runs_warns_and_passes() {
    let cases = labeled(5, 5);
    let mut files = valid();
    files[1] = (
        "agents/implementer/agent.md",
        "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\nmodel = \"opus\"\nsession = \"per-task\"\n+++\n# Implementation\n",
    );
    files.extend_from_slice(&[
        ("scorers/asked.toml", JUDGE),
        (
            "scorers/asked.md",
            "Does the change do what the request asks?\n",
        ),
        ("scorers/asked.eval.toml", &cases),
    ]);

    let outcome = check(&factory("scorer-judge-own", &files));

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr,
        "warning: scorers/asked.toml:4: the judge's model `opus` is also the model of agent `implementer`: a model tends to prefer its own work\n"
    );
}

#[test]
fn an_unknown_scorer_kind_names_its_file_and_line() {
    let guess = "description = \"Guess.\"\non = \"result\"\nkind = \"vibes\"\n";

    let outcome = with_scorers("scorer-kind", &[("scorers/guess.toml", guess)]);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "scorers/guess.toml:3:8: unknown variant `vibes`, expected one of `outcome`, `command`, `ask-shape`, `judge`, `held`, `intent`\n"
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
         scorers/e.toml:5: `run` is for kind `command` or `intent`, not `outcome`\n\
         scorers/f.toml:4: `run` names scripts/score-gate.sh, which is not executable: chmod +x it\n"
    );
}

const SHORT_ASK: &str =
    "description = \"Short.\"\non = \"ask\"\nkind = \"ask-shape\"\nexpect = [\"short\"]\n";

#[test]
fn a_factory_with_a_benchmark_passes_and_counts_it() {
    let benchmark = "description = \"Implement tasks.\"\nworkflow = \"implement\"\nprimary = \"passed\"\nscorers = [\"short-ask\"]\ncases = [\"cases/eng-412\"]\n";

    let outcome = with_scorers(
        "benchmark",
        &[
            ("scorers/passed.toml", PASSED),
            ("scorers/short-ask.toml", SHORT_ASK),
            ("benchmarks/implement.toml", benchmark),
        ],
    );

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 2 scorers, 1 benchmark\n"
    );
}

#[test]
fn a_benchmark_refuses_an_ask_scorer_as_primary_and_names_what_is_missing() {
    let benchmarks = [
        (
            "benchmarks/a.toml",
            "description = \"A.\"\nworkflow = \"implement\"\nprimary = \"short-ask\"\ncases = [\"cases/one\"]\n",
        ),
        (
            "benchmarks/b.toml",
            "description = \"B.\"\nworkflow = \"review\"\nprimary = \"gone\"\nscorers = [\"passed\", \"lost\"]\ncases = [\"../elsewhere\"]\n",
        ),
        (
            "benchmarks/c.toml",
            "description = \"C.\"\nworkflow = \"implement\"\nprimary = \"passed\"\ncases = []\n",
        ),
        ("scorers/passed.toml", PASSED),
        ("scorers/short-ask.toml", SHORT_ASK),
    ];

    let outcome = with_scorers("benchmark-rules", &benchmarks);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "benchmarks/a.toml:3: the primary scorer `short-ask` reads asks, which not every replay makes: use one on result or diff\n\
         benchmarks/b.toml:2: no workflow `review` in workflows/\n\
         benchmarks/b.toml:3: no scorer `gone` in scorers/\n\
         benchmarks/b.toml:4: no scorer `lost` in scorers/\n\
         benchmarks/b.toml:5: the case ../elsewhere is not inside the benchmarks folder\n\
         benchmarks/c.toml:4: a benchmark needs `cases`: folders that priory bench capture wrote\n"
    );
}

const HELD: &str = "description = \"Held a week.\"\non = \"merge\"\nkind = \"held\"\nafter = \"merged + 7d\"\nfails = [\"reverted\", \"red-after-merge\", \"fixed\"]\n";

#[test]
fn a_held_scorer_on_merge_with_after_and_fails_passes() {
    let outcome = with_scorers("scorer-held", &[("scorers/held.toml", HELD)]);

    assert_eq!(outcome.stderr, "");
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 1 scorer\n"
    );
}

#[test]
fn a_held_scorer_needs_merge_after_and_known_fails_and_no_other_kind_takes_them() {
    let bare = "description = \"Bare.\"\non = \"result\"\nkind = \"held\"\nexpect = [\"passed\"]\n";
    let odd = "description = \"Odd.\"\non = \"merge\"\nkind = \"held\"\nafter = \"merged + 1h\"\nfails = [\"reverted\", \"sentry\"]\n";
    let outcome_after = "description = \"Passed.\"\non = \"merge\"\nkind = \"outcome\"\nexpect = [\"passed\"]\nafter = \"merged + 7d\"\nfails = [\"reverted\"]\n";

    let outcome = with_scorers(
        "scorer-held-refused",
        &[
            ("scorers/bare.toml", bare),
            ("scorers/odd.toml", odd),
            ("scorers/passed.toml", outcome_after),
        ],
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "scorers/bare.toml:2: kind `held` reads on = \"merge\", not \"result\"\n\
         scorers/bare.toml:4: a `held` scorer takes no `expect`: name what fails it in `fails`\n\
         scorers/bare.toml:3: a `held` scorer needs `after`: when to grade, such as \"merged + 7d\"\n\
         scorers/bare.toml:3: a `held` scorer needs `fails`: some of changes-requested, red-after-merge, reverted, marked-wrong, fixed\n\
         scorers/odd.toml:5: `sentry` is no signal a `held` scorer reads: use changes-requested, red-after-merge, reverted, marked-wrong, fixed\n\
         scorers/passed.toml:2: kind `outcome` reads on = \"result\", not \"merge\"\n\
         scorers/passed.toml:5: `after` is for kind `held`, not `outcome`\n\
         scorers/passed.toml:6: `fails` is for kind `held`, not `outcome`\n"
    );
}

#[test]
fn a_held_scorer_s_after_must_be_a_delay_after_the_merge() {
    for (name, after) in [
        ("held-after-word", "merged in 7d"),
        ("held-after-zero", "merged + 0d"),
        ("held-after-weeks", "merged + 1w"),
        ("held-after-anchor", "done + 7d"),
    ] {
        let file = HELD.replace("merged + 7d", after);

        let outcome = with_scorers(name, &[("scorers/held.toml", &file)]);

        assert_eq!(outcome.code, 1, "{after}");
        assert_eq!(
            outcome.stderr,
            format!(
                "scorers/held.toml:4:9: {after:?} is not a moment such as \"merged + 7d\": a delay in m, h or d after the merge\n"
            ),
            "{after}"
        );
    }
}

#[test]
fn a_gate_or_a_benchmark_refuses_a_held_scorer() {
    let workflow = gated_workflow(r#"{ passed = ["held"] }"#, "");
    let mut files = with("workflows/implement.toml", &workflow);
    files.extend_from_slice(&[
        ("scorers/held.toml", HELD),
        ("scorers/passed.toml", PASSED),
        (
            "benchmarks/a.toml",
            "description = \"A.\"\nworkflow = \"implement\"\nprimary = \"held\"\nscorers = [\"passed\", \"held\"]\ncases = [\"cases/one\"]\n",
        ),
    ]);

    let outcome = check(&factory("held-gate", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "workflows/implement.toml:4: step `implement`: `gate` of outcome `passed` names `held`, which reads what followed a merge: a gate grades a result, so use a scorer on result or diff\n\
         benchmarks/a.toml:3: the scorer `held` reads what followed a merge, and a replay never merges: use one on result, diff or ask\n\
         benchmarks/a.toml:4: the scorer `held` reads what followed a merge, and a replay never merges: use one on result, diff or ask\n"
    );
}

const SKILL: &str = "---\nname: release-notes\ndescription: Writes release notes from merged pull requests.\n---\nList each change in one line.\n";

#[test]
fn a_factory_with_skills_and_instructions_passes_and_counts_its_skills() {
    let mut files = valid();
    files.push(("skills/release-notes/SKILL.md", SKILL));
    files.push(("skills/release-notes/template.md", "## Changes\n"));
    files.push(("instructions.md", "Run the gate before every result.\n"));

    let outcome = check(&factory("skills-valid", &files));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 1 skill\n"
    );
}

#[test]
fn a_skill_without_a_description_fails_the_check_with_its_file_and_line() {
    let mut files = valid();
    files.push((
        "skills/release-notes/SKILL.md",
        "---\nname: release-notes\n---\nList each change.\n",
    ));

    let outcome = check(&factory("skill-no-description", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(
        outcome.stderr.starts_with(
            "skills/release-notes/SKILL.md:1: a skill's frontmatter needs a `description`"
        ),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_skill_named_unlike_its_folder_fails_the_check() {
    let mut files = valid();
    files.push(("skills/notes/SKILL.md", SKILL));

    let outcome = check(&factory("skill-misnamed", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(
        outcome.stderr.starts_with(
            "skills/notes/SKILL.md:1: `name` is `release-notes`, but the skill's folder is skills/notes"
        ),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_path_glob_the_hook_cannot_match_fails_the_check_on_its_paths_line() {
    let mut files = valid();
    files.push((
        "skills/changelog/SKILL.md",
        "---\nname: changelog\ndescription: Edits the changelog.\npaths:\n  - CHANGELOG.md\n  - \"docs/**/*.{md,txt}\"\n---\nOne line per change.\n",
    ));
    files.push((
        "memories/gate-cost.md",
        "+++\nname = \"gate-cost\"\ndescription = \"How long the gate takes.\"\npaths = \"crates/*/tests/?.rs\"\n+++\nFour minutes.\n",
    ));
    files.push((
        "skills/release-notes/SKILL.md",
        "---\nname: release-notes\ndescription: Writes release notes.\npaths: [\"/etc/*.md\", \"src/../x\"]\n---\nList each change.\n",
    ));
    files.push((
        "memories/release-day.md",
        "---\nname: release-day\ndescription: When releases go out.\npaths:\n  - \"src/a**/b\"\n  - \"[ab].rs\"\n  - \"{a,{b,c}}\"\n  - \"\"\n---\nTuesdays.\n",
    ));

    let outcome = check(&factory("paths-invalid", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    for line in [
        "skills/release-notes/SKILL.md:4: the path glob `/etc/*.md` starts with /: a glob is relative to the repository root",
        "skills/release-notes/SKILL.md:4: the path glob `src/../x` has a `.` or `..` part",
        "memories/release-day.md:5: the path glob `src/a**/b` puts ** inside a part: ** stands alone between slashes",
        "memories/release-day.md:5: the path glob `[ab].rs` holds `[`: a glob takes *, **, ? and {a,b}",
        "memories/release-day.md:5: the path glob `{a,{b,c}}` nests braces or leaves one open",
        "memories/release-day.md:5: a path glob is empty",
    ] {
        assert!(
            outcome.stderr.contains(line),
            "{line} in {}",
            outcome.stderr
        );
    }
    assert_eq!(outcome.stderr.lines().count(), 6, "{}", outcome.stderr);
}

fn codex_with_instructions(name: &str, instructions: &str) -> Outcome {
    let mut files = valid();
    files[1] = (
        "agents/implementer/agent.md",
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\nMake the smallest correct change.\n",
    );
    files.push(("instructions.md", instructions));
    check(&factory(name, &files))
}

#[test]
fn a_codex_prompt_past_the_limit_fails_the_check_without_instructions_or_an_index() {
    let agent = format!(
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\n{}\n",
        "a".repeat(9000)
    );
    let mut files = valid();
    files[1] = ("agents/implementer/agent.md", &agent);

    let outcome = check(&factory("codex-prompt-long", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:3: a codex agent takes its own prompt on its command line, as `developer_instructions`: it is 9000 bytes, over the limit of 8192\n"
    );
}

#[test]
fn a_codex_prompt_at_the_limit_passes_since_no_separator_comes_without_instructions() {
    let agent = format!(
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\n{}\n",
        "a".repeat(8192)
    );
    let mut files = valid();
    files[1] = ("agents/implementer/agent.md", &agent);

    let outcome = check(&factory("codex-prompt-at-limit", &files));

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn instructions_past_the_codex_limit_fail_the_check_for_a_codex_agent_with_the_limit() {
    let long = "a".repeat(40 * 1024);

    let outcome = codex_with_instructions("codex-instructions-long", &long);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(
        outcome.stderr.starts_with(
            "agents/implementer/agent.md:3: a codex agent takes instructions.md and its own prompt on its command line, as `developer_instructions`: together they are 40995 bytes, over the limit of 8192"
        ),
        "{}",
        outcome.stderr
    );
}

#[test]
fn instructions_within_the_codex_limit_pass_for_a_codex_agent() {
    let outcome = codex_with_instructions("codex-instructions-short", "Run the gate.\n");

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

const MEMORY: &str = "---\nname: release-day\ndescription: When releases go out.\n---\nReleases go out on Tuesdays, never on a Friday.\n";

#[test]
fn a_factory_with_memories_passes_and_counts_its_memories() {
    let mut files = valid();
    files.push(("memories/release-day.md", MEMORY));
    files.push((
        "memories/gate-cost.md",
        "+++\nname = \"gate-cost\"\ndescription = \"How long the gate takes.\"\n+++\nThe gate takes about four minutes.\n",
    ));

    let outcome = check(&factory("memories-valid", &files));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 2 memories\n"
    );
}

#[test]
fn a_memory_md_index_in_the_memories_folder_is_skipped_not_loaded_as_a_memory() {
    let mut files = valid();
    files.push(("memories/release-day.md", MEMORY));
    files.push((
        "memories/MEMORY.md",
        "# Memory index\n\n- [release-day](release-day.md): When releases go out.\n",
    ));

    let outcome = check(&factory("memories-index-file", &files));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
    assert_eq!(
        outcome.stdout,
        "ok: 1 agent, 1 trigger, 1 workflow, 1 runner, 1 memory\n"
    );
}

/// The stderr of `priory check` on the valid factory with `files` added, which must fail.
fn memory_problems(name: &str, added: &[(&str, &str)]) -> String {
    let mut files = valid();
    files.extend_from_slice(added);
    let outcome = check(&factory(name, &files));
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    outcome.stderr
}

#[test]
fn a_memory_without_a_description_fails_the_check_with_its_file_and_line() {
    let stderr = memory_problems(
        "memory-no-description",
        &[(
            "memories/release-day.md",
            "---\nname: release-day\n---\nTuesdays.\n",
        )],
    );

    assert!(
        stderr
            .starts_with("memories/release-day.md:1: a memory's frontmatter needs a `description`"),
        "{stderr}"
    );
}

#[test]
fn a_memory_named_unlike_its_file_fails_the_check_on_its_name_line() {
    let stderr = memory_problems("memory-misnamed", &[("memories/release.md", MEMORY)]);

    assert!(
        stderr.starts_with(
            "memories/release.md:2: `name` is `release-day`, but the memory's file is memories/release.md"
        ),
        "{stderr}"
    );
}

#[test]
fn a_memory_over_the_size_limit_fails_the_check_with_its_size() {
    let long = format!(
        "---\nname: release-day\ndescription: When releases go out.\n---\n{}\n",
        "Tuesdays. ".repeat(500)
    );

    let stderr = memory_problems("memory-long", &[("memories/release-day.md", &long)]);

    assert!(
        stderr.starts_with(&format!(
            "memories/release-day.md:1: a memory holds one fact in at most 4096 bytes: this file is {} bytes",
            long.len()
        )),
        "{stderr}"
    );
}

#[test]
fn a_file_in_memories_that_is_not_a_named_markdown_file_fails_the_check() {
    let stderr = memory_problems(
        "memory-bad-names",
        &[
            ("memories/Release_Day.md", MEMORY),
            ("memories/notes.txt", "Tuesdays.\n"),
            ("memories/old/release-day.md", MEMORY),
        ],
    );

    for line in [
        "memories/Release_Day.md:1: a memory's file is named <name>.md, with lowercase letters, digits and hyphens",
        "memories/notes.txt: memories/ holds only <name>.md files, one memory each",
        "memories/old: memories/ holds only <name>.md files, one memory each",
    ] {
        assert!(stderr.contains(line), "{line} in {stderr}");
    }
}

#[test]
fn the_memory_index_counts_toward_the_codex_limit() {
    let description = "d".repeat(3000);
    let memories: Vec<(String, String)> = ["one", "two", "three"]
        .map(|name| {
            (
                format!("memories/{name}.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\nA fact.\n"),
            )
        })
        .into();
    let mut files: Vec<(&str, &str)> = valid();
    files[1] = (
        "agents/implementer/agent.md",
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\nMake the smallest correct change.\n",
    );
    files.extend(
        memories
            .iter()
            .map(|(file, text)| (file.as_str(), text.as_str())),
    );

    let outcome = check(&factory("codex-memory-index", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(
        outcome.stderr.starts_with(
            "agents/implementer/agent.md:3: a codex agent takes the memory index and its own prompt on its command line"
        ),
        "{}",
        outcome.stderr
    );
}

#[test]
fn a_sentry_trigger_needs_the_sentry_webhook_and_a_supported_event_filter_and_key() {
    let good = "source = \"sentry\"\non = \"issue.created\"\nfilter = { project = \"api\", level = \"error\" }\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nkey = \"issue.id\"\n";
    let wrong = "source = \"sentry\"\non = \"issue.resolved\"\nfilter = { team = \"x\" }\nworkflow = \"implement\"\nkey = \"issue.shortId\"\nbackfill = true\n";
    let hooked =
        "name = \"demo\"\n\n[serve]\n\n[webhooks.sentry]\nsecret_env = \"SENTRY_CLIENT_SECRET\"\n";
    let mut files = with("triggers/agent-label.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    let unhooked = check(&factory("sentry-unhooked", &files));
    files.retain(|(path, _)| *path != "factory.toml");
    files.push(("factory.toml", hooked));
    let hooked = check(&factory("sentry-triggers", &files));
    files.retain(|(path, _)| *path != "triggers/wrong.toml");
    let ok = check(&factory("sentry-ok", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(hooked.code, 1);
    assert_eq!(
        hooked.stderr,
        "triggers/wrong.toml:6: only a GitHub, Linear or Slack trigger has `backfill`\n\
         triggers/wrong.toml:2: a `sentry` trigger handles `issue.created`, not `issue.resolved`\n\
         triggers/wrong.toml:3: a `sentry` trigger filters on `project` or `level`\n\
         triggers/wrong.toml:5: a Sentry trigger's `key` is `issue.id`\n"
    );
    assert_eq!(unhooked.code, 1);
    assert!(
        unhooked.stderr.contains(
            "triggers/agent-label.toml:1: a `sentry` trigger needs `[webhooks.sentry]` in factory.toml: Sentry events arrive only by webhook\n"
        ),
        "{}",
        unhooked.stderr
    );
}

#[test]
fn a_cloudwatch_trigger_needs_the_cloudwatch_webhook_and_a_supported_event_filter_and_key() {
    let good = "source = \"cloudwatch\"\non = \"alarm.state_changed\"\nfilter = { name = \"ServerCpuTooHigh\", state = \"ALARM\", previous_state = \"OK\" }\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nkey = \"alarm.arn\"\n";
    let wrong = "source = \"cloudwatch\"\non = \"alarm.created\"\nfilter = { region = \"us-east-1\" }\nworkflow = \"implement\"\nkey = \"alarm.name\"\nbackfill = true\n";
    let hooked = "name = \"demo\"\n\n[serve]\n\n[webhooks.cloudwatch]\nsecret_env = \"CLOUDWATCH_WEBHOOK_SECRET\"\n";
    let mut files = with("triggers/alarms.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    let unhooked = check(&factory("cloudwatch-unhooked", &files));
    files.retain(|(path, _)| *path != "factory.toml");
    files.push(("factory.toml", hooked));
    let hooked = check(&factory("cloudwatch-triggers", &files));
    files.retain(|(path, _)| *path != "triggers/wrong.toml");
    let ok = check(&factory("cloudwatch-ok", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(hooked.code, 1);
    assert_eq!(
        hooked.stderr,
        "triggers/wrong.toml:6: only a GitHub, Linear or Slack trigger has `backfill`\n\
         triggers/wrong.toml:2: a `cloudwatch` trigger handles `alarm.state_changed`, not `alarm.created`\n\
         triggers/wrong.toml:3: a `cloudwatch` trigger filters on `name` or `state` or `previous_state`\n\
         triggers/wrong.toml:5: a CloudWatch trigger's `key` is `alarm.arn`\n"
    );
    assert_eq!(unhooked.code, 1);
    assert!(
        unhooked.stderr.contains(
            "triggers/alarms.toml:1: a `cloudwatch` trigger needs `[webhooks.cloudwatch]` in factory.toml: CloudWatch alarm state changes arrive only by webhook\n"
        ),
        "{}",
        unhooked.stderr
    );
}

#[test]
fn a_honeycomb_trigger_needs_the_honeycomb_webhook_and_a_supported_event_filter_and_key() {
    let good = "source = \"honeycomb\"\non = \"trigger.fired\"\nfilter = { environment = \"production\", trigger = \"JwwZxfcP5kH\" }\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nkey = \"trigger.id\"\n";
    let wrong = "source = \"honeycomb\"\non = \"slo.burn_alert\"\nfilter = { dataset = \"x\" }\nworkflow = \"implement\"\nkey = \"alert.id\"\nbackfill = true\n";
    let hooked = "name = \"demo\"\n\n[serve]\n\n[webhooks.honeycomb]\nsecret_env = \"HONEYCOMB_WEBHOOK_SECRET\"\n";
    let mut files = with("triggers/agent-label.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    let unhooked = check(&factory("honeycomb-unhooked", &files));
    files.retain(|(path, _)| *path != "factory.toml");
    files.push(("factory.toml", hooked));
    let hooked = check(&factory("honeycomb-triggers", &files));
    files.retain(|(path, _)| *path != "triggers/wrong.toml");
    let ok = check(&factory("honeycomb-ok", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(hooked.code, 1);
    assert_eq!(
        hooked.stderr,
        "triggers/wrong.toml:6: only a GitHub, Linear or Slack trigger has `backfill`\n\
         triggers/wrong.toml:2: a `honeycomb` trigger handles `trigger.fired`, not `slo.burn_alert`\n\
         triggers/wrong.toml:3: a `honeycomb` trigger filters on `environment` or `trigger`\n\
         triggers/wrong.toml:5: a Honeycomb trigger's `key` is `trigger.id`\n"
    );
    assert_eq!(unhooked.code, 1);
    assert!(
        unhooked.stderr.contains(
            "triggers/agent-label.toml:1: a `honeycomb` trigger needs `[webhooks.honeycomb]` in factory.toml: Honeycomb alerts arrive only by webhook\n"
        ),
        "{}",
        unhooked.stderr
    );
}

#[test]
fn a_figma_trigger_needs_the_figma_webhook_and_a_supported_event_filter_and_key() {
    let good = "source = \"figma\"\non = \"file.comment_created\"\nfilter = { file = \"zH44k2FUM629Fa4EMShiHL\" }\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nkey = \"file.key\"\n";
    let wrong = "source = \"figma\"\non = \"file.updated\"\nfilter = { author = \"x\" }\nworkflow = \"implement\"\nkey = \"comment.id\"\nbackfill = true\n";
    let hooked =
        "name = \"demo\"\n\n[serve]\n\n[webhooks.figma]\nsecret_env = \"FIGMA_WEBHOOK_PASSCODE\"\n";
    let mut files = with("triggers/comments.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    let unhooked = check(&factory("figma-unhooked", &files));
    files.retain(|(path, _)| *path != "factory.toml");
    files.push(("factory.toml", hooked));
    let hooked = check(&factory("figma-triggers", &files));
    files.retain(|(path, _)| *path != "triggers/wrong.toml");
    let ok = check(&factory("figma-ok", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(hooked.code, 1);
    assert_eq!(
        hooked.stderr,
        "triggers/wrong.toml:6: only a GitHub, Linear or Slack trigger has `backfill`\n\
         triggers/wrong.toml:2: a `figma` trigger handles `file.comment_created`, not `file.updated`\n\
         triggers/wrong.toml:3: a `figma` trigger filters on `file`\n\
         triggers/wrong.toml:5: a Figma trigger's `key` is `file.key`\n"
    );
    assert_eq!(unhooked.code, 1);
    assert!(
        unhooked.stderr.contains(
            "triggers/comments.toml:1: a `figma` trigger needs `[webhooks.figma]` in factory.toml: Figma comments arrive only by webhook\n"
        ),
        "{}",
        unhooked.stderr
    );
}

#[test]
fn an_intercom_trigger_needs_the_intercom_webhook_and_takes_no_filter() {
    let good = "source = \"intercom\"\non = \"conversation.created\"\nrepo = \"owner/repo\"\nworkflow = \"implement\"\nkey = \"conversation.id\"\n";
    let wrong = "source = \"intercom\"\non = \"conversation.replied\"\nfilter = { workspace = \"x\" }\nworkflow = \"implement\"\nkey = \"contact.id\"\nbackfill = true\n";
    let hooked = "name = \"demo\"\n\n[serve]\n\n[webhooks.intercom]\nworkspace = \"a86dr8yl\"\n";
    let mut files = with("triggers/agent-label.toml", good);
    files.push(("triggers/wrong.toml", wrong));
    let unhooked = check(&factory("intercom-unhooked", &files));
    files.retain(|(path, _)| *path != "factory.toml");
    files.push(("factory.toml", hooked));
    let hooked = check(&factory("intercom-triggers", &files));
    files.retain(|(path, _)| *path != "triggers/wrong.toml");
    let ok = check(&factory("intercom-ok", &files));
    files.retain(|(path, _)| *path != "factory.toml");
    files.push((
        "factory.toml",
        "name = \"demo\"\n\n[serve]\n\n[webhooks.intercom]\n",
    ));
    let nameless = check(&factory("intercom-nameless", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(hooked.code, 1);
    assert_eq!(
        hooked.stderr,
        "triggers/wrong.toml:6: only a GitHub, Linear or Slack trigger has `backfill`\n\
         triggers/wrong.toml:2: an `intercom` trigger handles `conversation.created`, not `conversation.replied`\n\
         triggers/wrong.toml:3: an `intercom` trigger takes no `filter`\n\
         triggers/wrong.toml:5: an Intercom trigger's `key` is `conversation.id`\n"
    );
    assert_eq!(unhooked.code, 1);
    assert!(
        unhooked.stderr.contains(
            "triggers/agent-label.toml:1: an `intercom` trigger needs `[webhooks.intercom]` in factory.toml: Intercom conversations arrive only by webhook\n"
        ),
        "{}",
        unhooked.stderr
    );
    assert_ne!(nameless.code, 0);
    assert!(
        nameless.stderr.contains("missing field `workspace`"),
        "{}",
        nameless.stderr
    );
}

#[test]
fn the_skill_index_counts_toward_the_codex_limit() {
    let description = "d".repeat(3000);
    let skills: Vec<(String, String)> = ["one", "two", "three"]
        .map(|name| {
            (
                format!("skills/{name}/SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\nDo it.\n"),
            )
        })
        .into();
    let mut files: Vec<(&str, &str)> = valid();
    files[1] = (
        "agents/implementer/agent.md",
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\nMake the smallest correct change.\n",
    );
    files.extend(
        skills
            .iter()
            .map(|(file, text)| (file.as_str(), text.as_str())),
    );

    let outcome = check(&factory("codex-skill-index", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert!(
        outcome.stderr.starts_with(
            "agents/implementer/agent.md:3: a codex agent takes the skill index and its own prompt on its command line"
        ),
        "{}",
        outcome.stderr
    );
}

/// A factory with the skills `a`, `b` and `review_notes`, `a` described in `size` bytes, and with
/// `[kit]` when `kit` holds, whose implementer runs on `harness` with the frontmatter line `line`.
fn with_named_skills(name: &str, harness: &str, line: &str, size: usize, kit: bool) -> Outcome {
    let skills: Vec<(String, String)> = [("a", size), ("b", 1500), ("review_notes", 1500)]
        .map(|(skill, size)| {
            let description = "d".repeat(size);
            (
                format!("skills/{skill}/SKILL.md"),
                format!("---\nname: {skill}\ndescription: {description}\n---\nDo it.\n"),
            )
        })
        .into();
    let agent = format!(
        "+++\ndescription = \"Implements one task.\"\nharness = \"{harness}\"\n{line}\n+++\nMake the smallest correct change.\n"
    );
    let toml = "name = \"demo\"\n\n[kit]\nrepo = \"git@github.com:owner/abbey.git\"\n";
    let mut files: Vec<(&str, &str)> = valid();
    files[1] = ("agents/implementer/agent.md", &agent);
    if kit {
        files[0] = ("factory.toml", toml);
    }
    files.extend(
        skills
            .iter()
            .map(|(file, text)| (file.as_str(), text.as_str())),
    );
    check(&factory(name, &files))
}

#[test]
fn a_skills_list_naming_no_skill_of_a_factory_without_a_kit_fails_the_check_with_the_factorys_skills()
 {
    let line = "skills = [\"a\", \"review-coments\"]";

    let outcome = with_named_skills("skills-unknown", "claude-code", line, 10, false);
    let kit = with_named_skills("skills-unknown-kit", "claude-code", line, 10, true);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:4: `skills` names `review-coments`, which is not a skill of this factory: a, b, review_notes\n"
    );
    assert_eq!(
        (kit.code, kit.stderr.as_str()),
        (0, ""),
        "the kit may hold it"
    );
}

#[test]
fn a_skills_list_with_a_name_twice_or_an_empty_name_fails_the_check() {
    let twice = with_named_skills(
        "skills-twice",
        "claude-code",
        "skills = [\"a\", \"a\"]",
        10,
        true,
    );
    let empty = with_named_skills("skills-empty", "claude-code", "skills = [\"\"]", 10, true);
    let underscore = with_named_skills(
        "skills-underscore",
        "claude-code",
        "skills = [\"review_notes\"]",
        10,
        false,
    );
    let none = with_named_skills("skills-none", "claude-code", "skills = []", 10, false);

    assert_eq!(twice.code, 1);
    assert_eq!(
        twice.stderr,
        "agents/implementer/agent.md:4: `skills` names `a` twice\n"
    );
    assert_eq!(empty.code, 1);
    assert_eq!(
        empty.stderr,
        "agents/implementer/agent.md:4: `skills` holds an empty name\n"
    );
    assert_eq!((underscore.code, underscore.stderr.as_str()), (0, ""));
    assert_eq!((none.code, none.stderr.as_str()), (0, ""));
}

#[test]
fn a_codex_agent_counts_only_the_skills_it_names_toward_its_limit() {
    let every = with_named_skills("skills-codex-every", "codex", "", 6000, false);
    let named = with_named_skills(
        "skills-codex-named",
        "codex",
        "skills = [\"b\", \"review_notes\"]",
        6000,
        false,
    );

    assert_eq!(every.code, 1, "{}", every.stderr);
    assert!(
        every.stderr.starts_with(
            "agents/implementer/agent.md:3: a codex agent takes the skill index and its own prompt on its command line, as `developer_instructions`: together they are"
        ),
        "{}",
        every.stderr
    );
    assert_eq!((named.code, named.stderr.as_str()), (0, ""));
}

#[test]
fn a_codex_agent_whose_own_prompt_passes_the_limit_fails_the_check_without_instructions_or_skills()
{
    let mut files = valid();
    let agent = format!(
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\nskills = []\n+++\n{}\n",
        "a".repeat(9000)
    );
    files[1] = ("agents/implementer/agent.md", &agent);
    files.push(("skills/release-notes/SKILL.md", SKILL));

    let outcome = check(&factory("codex-skills-none-prompt-long", &files));

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr,
        "agents/implementer/agent.md:3: a codex agent takes its own prompt on its command line, as `developer_instructions`: it is 9000 bytes, over the limit of 8192\n"
    );
}

#[test]
fn a_kit_takes_a_repository_and_paths_inside_it() {
    let toml = "name = \"demo\"\n\n[kit]\nrepo = \"git@github.com:owner/abbey.git\"\nref = \"main\"\ninstructions = \"claude/CLAUDE.md\"\nevery = \"10m\"\n";

    let outcome = check(&factory("kit-valid", &with("factory.toml", toml)));

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_kit_path_that_leaves_the_repository_or_a_ref_git_reads_as_an_option_fails_the_check() {
    let toml = "name = \"demo\"\n\n[kit]\nrepo = \"../abbey\"\nref = \"--upload-pack=touch\"\nskills = \"../skills\"\nmemories = \"/etc\"\n";

    let outcome = check(&factory("kit-leaves", &with("factory.toml", toml)));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "factory.toml:3: `[kit]` `ref` is `--upload-pack=touch`: it cannot start with `-`\n\
         factory.toml:3: `[kit]` `skills` is `../skills`: a kit path is relative and stays inside the repository\n\
         factory.toml:3: `[kit]` `memories` is `/etc`: a kit path is relative and stays inside the repository\n"
    );
}

#[test]
fn a_memories_repository_takes_a_repository_and_a_path_inside_it() {
    let toml = "name = \"demo\"\n\n[memories]\nrepo = \"git@github.com:owner/scriptorium.git\"\nref = \"main\"\npath = \"facts\"\nevery = \"10m\"\n";

    let outcome = check(&factory("memories-repo-valid", &with("factory.toml", toml)));

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_memories_path_that_leaves_the_repository_or_a_ref_git_reads_as_an_option_fails_the_check() {
    let toml = "name = \"demo\"\n\n[memories]\nrepo = \"-oProxyCommand=x\"\nref = \"\"\npath = \"../facts\"\n";

    let outcome = check(&factory(
        "memories-repo-leaves",
        &with("factory.toml", toml),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "factory.toml:3: `[memories]` `repo` is `-oProxyCommand=x`: it cannot start with `-`\n\
         factory.toml:3: `[memories]` `ref` is empty\n\
         factory.toml:3: `[memories]` `path` is `../facts`: a memories path is relative and stays inside the repository\n"
    );
}

const MEMORIES_WORKFLOW: &str = "start = \"open\"\n\n[[steps]]\nid = \"open\"\ndecides = \"code\"\naction = \"open-memories-pr\"\neffect = \"irreversible\"\nautonomous = \"a person reviews the pull request before any memory merges\"\nnext = { opened = \"done\", unchanged = \"done\", failed = \"stop\" }\n";

#[test]
fn an_open_memories_pr_step_needs_a_kit() {
    let mut files = valid();
    files.push(("workflows/memories.toml", MEMORIES_WORKFLOW));
    let without = check(&factory("memories-pr-no-kit", &files));
    let toml =
        "name = \"demo\"\n\n[kit]\nrepo = \"git@github.com:owner/abbey.git\"\nref = \"main\"\n";
    let mut files: Vec<_> = files
        .into_iter()
        .filter(|(file, _)| *file != "factory.toml")
        .collect();
    files.push(("factory.toml", toml));
    let with_kit = check(&factory("memories-pr-kit", &files));

    assert_eq!(without.code, 1);
    assert_eq!(
        without.stderr,
        "workflows/memories.toml:3: step `open`: an `open-memories-pr` action writes to the memories repository or the kit, so factory.toml needs a `[memories]` or a `[kit]`\n"
    );
    assert_eq!(with_kit.code, 0, "{}", with_kit.stderr);
}

#[test]
fn spend_caps_pass_and_a_cap_not_above_zero_or_a_malformed_lane_is_refused() {
    let good = "name = \"demo\"\n\n[caps]\ntask = 5.0\nday = 50\n\n[caps.lanes.\"core/intake\"]\nweek = 100.0\n";
    let trigger = format!("{TRIGGER}caps = {{ task = 2.5, day = 10 }}\n");
    let mut files = with("factory.toml", good);
    files.retain(|(path, _)| *path != "triggers/agent-label.toml");
    files.push(("triggers/agent-label.toml", &trigger));
    files.push(("prices.toml", PRICES));
    let ok = check(&factory("caps-ok", &files));
    let bad = "name = \"demo\"\n\n[caps]\ntask = 0\n\n[caps.lanes.intake]\nday = 1.0\n";
    let refused = check(&factory("caps-bad", &with("factory.toml", bad)));
    let negative = format!("{TRIGGER}caps = {{ week = -3.0 }}\n");
    let mut files = valid();
    files.retain(|(path, _)| *path != "triggers/agent-label.toml");
    files.push(("triggers/agent-label.toml", &negative));
    let trigger_refused = check(&factory("caps-trigger-bad", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(refused.code, 1);
    assert!(
        refused
            .stderr
            .contains("0 is not a spend cap: give dollars above zero"),
        "{}",
        refused.stderr
    );
    assert_eq!(trigger_refused.code, 1);
    assert!(
        trigger_refused
            .stderr
            .contains("-3 is not a spend cap: give dollars above zero"),
        "{}",
        trigger_refused.stderr
    );
    let lane = "name = \"demo\"\n\n[caps.lanes.intake]\nday = 1.0\n";
    let mut files = with("factory.toml", lane);
    files.push(("prices.toml", PRICES));
    let lane = check(&factory("caps-lane-bad", &files));
    assert_eq!(lane.code, 1);
    assert_eq!(
        lane.stderr,
        "factory.toml:3: `[caps.lanes]` `intake` is not a lane such as `team/theme`\n"
    );
}

/// The valid factory with `[pre]` table `pre` on its trigger, and an executable `scripts/pre`.
fn with_pre(name: &str, pre: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let trigger = format!("{TRIGGER}\n[pre]\n{pre}");
    let mut files = with("triggers/agent-label.toml", &trigger);
    files.push(("scripts/pre", "#!/bin/sh\nexit 0\n"));
    files.push(("scripts/plain", "#!/bin/sh\nexit 0\n"));
    let dir = factory(name, &files);
    fs::set_permissions(dir.join("scripts/pre"), fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

#[test]
fn a_pre_script_inside_the_factory_with_a_short_timeout_passes() {
    let outcome = check(&with_pre(
        "pre-valid",
        "argv = [\"scripts/pre\", \"--min-level\", \"error\"]\ntimeout = \"60s\"\nenv = [\"AMPLITUDE_READ_TOKEN\"]\non_error = \"drop\"\n",
    ));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

#[test]
fn check_refuses_a_pre_script_outside_the_factory_or_not_executable_and_a_long_timeout() {
    let cases = [
        (
            "pre-outside",
            "argv = [\"../outside\"]\n",
            "triggers/agent-label.toml:9: `[pre] argv`: ../outside leaves the factory folder\n",
        ),
        (
            "pre-absolute",
            "argv = [\"/bin/true\"]\n",
            "triggers/agent-label.toml:9: `[pre] argv`: /bin/true leaves the factory folder\n",
        ),
        (
            "pre-plain",
            "argv = [\"scripts/plain\"]\n",
            "triggers/agent-label.toml:9: `[pre] argv`: scripts/plain is not executable: chmod +x it\n",
        ),
        (
            "pre-missing",
            "argv = [\"scripts/missing\"]\n",
            "triggers/agent-label.toml:9: `[pre] argv`: scripts/missing is not a file in the factory folder\n",
        ),
        (
            "pre-slow",
            "argv = [\"scripts/pre\"]\ntimeout = \"2m\"\n",
            "triggers/agent-label.toml:9: `[pre] timeout` is 120s: a pre script runs at most 60s\n",
        ),
        (
            "pre-env",
            "argv = [\"scripts/pre\"]\nenv = [\"NOT-A-NAME\"]\n",
            "triggers/agent-label.toml:9: `[pre] env`: `NOT-A-NAME` is not a variable name\n",
        ),
    ];
    for (name, pre, expected) in cases {
        let outcome = check(&with_pre(name, pre));

        assert_eq!(
            (outcome.code, outcome.stderr.as_str()),
            (1, expected),
            "{name}"
        );
    }
}

#[test]
fn check_refuses_an_unknown_pre_on_error_with_its_line() {
    let outcome = check(&with_pre(
        "pre-on-error",
        "argv = [\"scripts/pre\"]\non_error = \"retry\"\n",
    ));

    assert_eq!(
        (outcome.code, outcome.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:11:12: unknown variant `retry`, expected `pass` or `drop`\n"
        )
    );
}

#[test]
fn check_refuses_a_pre_script_on_the_cli_trigger_which_nothing_runs() {
    use std::os::unix::fs::PermissionsExt;
    let cli = "source = \"cli\"\nrepo = \"owner/repo\"\nworkflow = \"implement\"\n\n[pre]\nargv = [\"scripts/pre\"]\n";
    let mut files = with("triggers/cli.toml", cli);
    files.push(("scripts/pre", "#!/bin/sh\nexit 1\n"));
    let dir = factory("pre-cli", &files);
    fs::set_permissions(dir.join("scripts/pre"), fs::Permissions::from_mode(0o755)).unwrap();

    let outcome = check(&dir);

    assert_eq!(
        (outcome.code, outcome.stderr.as_str()),
        (
            1,
            "triggers/cli.toml:5: `[pre]` runs on every source except `triggers/cli.toml`: priory add and priory request never run it\n"
        )
    );
}

#[test]
fn a_symlink_out_of_the_factory_is_refused_as_a_pre_script() {
    let dir = with_pre("pre-symlink", "argv = [\"scripts/link\"]\n");
    std::os::unix::fs::symlink("/bin/true", dir.join("scripts/link")).unwrap();

    let outcome = check(&dir);

    assert_eq!(
        (outcome.code, outcome.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:9: `[pre] argv`: scripts/link leaves the factory folder\n"
        )
    );
}

#[test]
fn a_budget_needs_a_price_table_and_an_amount_above_zero_on_either_harness() {
    let budgeted = WORKFLOW.replace(
        "prompt = \"Implement the issue.\"",
        "prompt = \"Implement the issue.\"\nbudget = \"1.00\"",
    );
    let prices =
        "[claude-sonnet-5-5]\ninput = 2.0\noutput = 10.0\ncache_read = 0.1\ncache_write = 2.5\n";
    let mut files = with("workflows/implement.toml", &budgeted);
    files.push(("prices.toml", prices));
    assert_eq!(check(&factory("budget", &files)).stderr, "");

    let unpriced = check(&factory(
        "budget-unpriced",
        &with("workflows/implement.toml", &budgeted),
    ));
    assert_eq!(unpriced.code, 1);
    assert_eq!(
        unpriced.stderr,
        "workflows/implement.toml:4: step `implement`: `budget` needs prices.toml, which turns the step's tokens into dollars\n"
    );

    let negative = budgeted.replace("\"1.00\"", "\"-1\"");
    let mut files = with("workflows/implement.toml", &negative);
    files.push(("prices.toml", prices));
    let refused = check(&factory("budget-negative", &files));
    assert_eq!(refused.code, 1);
    assert_eq!(
        refused.stderr,
        "workflows/implement.toml:4: step `implement`: `budget` \"-1\" is not an amount of dollars above zero, such as \"1.00\"\n"
    );

    let codex =
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\nMake the change.\n";
    let mut files = with("workflows/implement.toml", &budgeted);
    files.retain(|(path, _)| *path != "agents/implementer/agent.md");
    files.push(("agents/implementer/agent.md", codex));
    files.push(("prices.toml", prices));
    let codex = check(&factory("budget-codex", &files));
    assert_eq!((codex.code, codex.stderr.as_str()), (0, ""));
}

#[test]
fn an_open_memories_pr_step_takes_a_memories_repository_in_place_of_a_kit() {
    let mut files: Vec<_> = valid()
        .into_iter()
        .filter(|(file, _)| *file != "factory.toml")
        .collect();
    files.push(("workflows/memories.toml", MEMORIES_WORKFLOW));
    files.push((
        "factory.toml",
        "name = \"demo\"\n\n[memories]\nrepo = \"git@github.com:owner/scriptorium.git\"\n",
    ));

    let outcome = check(&factory("memories-pr-scriptorium", &files));

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// A `[decisions.triage]` table with `extra` keys, and labeled cases with `nothing` thanks.
fn decisions(name: &str, extra: &str, nothing: usize) -> Outcome {
    let toml = format!(
        "name = \"demo\"\n\n[decisions.triage]\nrun = \"python3 jev.py\"\nevaluation = \"triage.eval.toml\"\n{extra}"
    );
    let mut cases = String::new();
    for (label, count) in [
        ("nothing-to-do", nothing),
        ("trivial", 3),
        ("persons-call", 3),
    ] {
        for n in 0..count {
            cases.push_str(&format!(
                "[[cases]]\nname = \"{label}-{n}\"\nbody = \"b\"\nlabel = \"{label}\"\n"
            ));
        }
    }
    let mut files = with("factory.toml", &toml);
    files.push(("triage.eval.toml", Box::leak(cases.into_boxed_str())));
    check(&factory(name, &files))
}

#[test]
fn a_decision_model_with_its_key_named_and_ten_cases_passes() {
    let outcome = decisions("decisions-ok", "key_env = \"TYPESAFE_API_KEY\"\n", 4);

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

#[test]
fn a_decision_model_refuses_a_pasted_key_bad_shares_a_long_timeout_and_too_few_cases() {
    let extra = "key_env = \"sk-live-123\"\nagreement = 0.3\nthreshold = 1.5\ntimeout = \"60s\"\n";

    let outcome = decisions("decisions-bad", extra, 2);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "factory.toml:3: `[decisions.triage] key_env` names the environment variable that holds \
         the model's key, such as TYPESAFE_API_KEY, never the key itself\n\
         factory.toml:3: `[decisions.triage] agreement` is 0.3: give a share from 0.5 to 1, such as 0.9\n\
         factory.toml:3: `[decisions.triage] threshold` is 1.5: give a share from 0.5 to 1, such as 0.9\n\
         factory.toml:3: `[decisions.triage] timeout` is 60s: give at most 30s\n\
         factory.toml:3: `[decisions.triage] evaluation` triage.eval.toml: triage needs at least 10 \
         labeled cases, 3 of each class: this file has 8 (3 trivial, 2 nothing-to-do, 3 persons-call)\n"
    );
}

const DEBATER: &str = "+++\ndescription = \"Debates.\"\nharness = \"claude-code\"\nmodel = \"m1\"\n+++\nYou debate.\n";

const CODEX_DEBATER: &str =
    "+++\ndescription = \"Debates.\"\nharness = \"codex\"\nmodel = \"m2\"\n+++\nYou debate.\n";

const DEBATE_PRICES: &str = "[m1]\ninput = 1.0\noutput = 2.0\ncache_read = 0.1\ncache_write = 1.25\n\n[m2]\ninput = 2.0\noutput = 8.0\ncache_read = 0.2\ncache_write = 0.0\n";

/// A factory whose workflow starts with a debate step, with `debate` as its table and `extra`
/// keys after it.
fn debate_factory(name: &str, debate: &str, extra: &str) -> PathBuf {
    let workflow = format!(
        "start = \"debate\"\n\n[[steps]]\nid = \"debate\"\ndecides = \"model\"\ndebate = {debate}\nprompt = \"Daemon or process?\"\n{extra}next = {{ daemon = \"done\", process = \"stop\" }}\n"
    );
    factory(
        name,
        &[
            ("factory.toml", FACTORY),
            ("agents/architect/agent.md", DEBATER),
            ("agents/skeptic/agent.md", CODEX_DEBATER),
            ("prices.toml", DEBATE_PRICES),
            ("triggers/agent-label.toml", TRIGGER),
            ("workflows/implement.toml", &workflow),
            ("runners/xps13.toml", RUNNER),
        ],
    )
}

const TWO: &str = r#"{ agents = ["architect", "skeptic"], rounds = 3, cap = "$3.00" }"#;

const READ: &str = "effect = \"read\"\ntimeout = \"15m\"\n";

#[test]
fn a_debate_step_with_two_priced_agents_passes_and_explain_prints_its_worst_case() {
    let dir = debate_factory("debate-valid", TWO, READ);
    let outcome = check(&dir);
    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);

    let explained = harness::priory()
        .args(["check", "--explain"])
        .arg(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(explained.stdout).unwrap();
    assert!(
        stdout.ends_with(
            "workflow\tstep\tdebate\nimplement\tdebate\t2 agents, up to 8 turns, cap $3.00\n"
        ),
        "{stdout}"
    );
}

#[test]
fn explain_notes_a_debate_whose_agents_share_a_harness_and_a_model() {
    let dir = debate_factory("debate-same", TWO, READ);
    fs::write(dir.join("agents/skeptic/agent.md"), DEBATER).unwrap();
    let explained = harness::priory()
        .args(["check", "--explain"])
        .arg(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(explained.stdout).unwrap();
    assert!(
        stdout.contains("cap $3.00; both agents run claude-code with model m1, so the debate mostly agrees with itself\n"),
        "{stdout}"
    );
}

/// The one problem `priory check` reports for the debate step, on its line.
fn debate_problem(name: &str, debate: &str, extra: &str) -> String {
    let outcome = check(&debate_factory(name, debate, extra));
    assert_eq!(outcome.code, 1, "{}", outcome.stdout);
    outcome.stderr
}

fn at_debate(message: &str) -> String {
    format!("workflows/implement.toml:3: step `debate`: {message}\n")
}

#[test]
fn a_debate_needs_two_different_agents_and_one_to_five_rounds() {
    assert_eq!(
        debate_problem(
            "debate-one",
            r#"{ agents = ["architect"], rounds = 3, cap = "$3.00" }"#,
            READ
        ),
        at_debate("a debate needs exactly two agents, in speaking order, not 1")
    );
    assert_eq!(
        debate_problem(
            "debate-twice",
            r#"{ agents = ["architect", "architect"], rounds = 3, cap = "$3.00" }"#,
            READ
        ),
        at_debate("a debate's two agents must differ: `architect` is named twice")
    );
    assert_eq!(
        debate_problem(
            "debate-rounds",
            r#"{ agents = ["architect", "skeptic"], rounds = 6, cap = "$3.00" }"#,
            READ
        ),
        at_debate("`rounds` is 6; a debate takes 1 to 5 rounds")
    );
}

#[test]
fn a_debate_step_changes_nothing_and_needs_a_timeout() {
    assert_eq!(
        debate_problem(
            "debate-reversible",
            TWO,
            "effect = \"reversible\"\ntimeout = \"15m\"\n"
        ),
        at_debate(
            "a debate step's effect is `none` or `read`, not `reversible`: debaters judge and change nothing"
        )
    );
    assert_eq!(
        debate_problem(
            "debate-approves",
            TWO,
            "effect = \"read\"\ntimeout = \"15m\"\napproves = [\"debate\"]\n"
        ),
        at_debate("only a human step has `approves`")
            + &at_debate("approves `debate`, which is not an irreversible step")
    );
    assert_eq!(
        debate_problem("debate-timeout", TWO, "effect = \"read\"\n"),
        at_debate("a debate step needs a `timeout`: its cap cannot bound one turn")
    );
    assert_eq!(
        debate_problem(
            "debate-agent",
            TWO,
            "effect = \"read\"\ntimeout = \"15m\"\nagent = \"architect\"\n"
        ),
        at_debate("a debate step has no `agent`: its `debate` names both")
    );
}

#[test]
fn a_debates_cap_needs_a_price_for_each_agents_model() {
    let dir = debate_factory("debate-unpriced", TWO, READ);
    fs::write(
        dir.join("prices.toml"),
        "[m1]\ninput = 1.0\noutput = 2.0\ncache_read = 0.1\ncache_write = 1.25\n",
    )
    .unwrap();
    let outcome = check(&dir);
    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        at_debate(
            "agent `skeptic`'s model `m2` has no price in prices.toml, so the debate's cap cannot count it"
        )
    );
}

#[test]
fn a_debate_needs_a_cap() {
    let outcome = check(&debate_factory(
        "debate-no-cap",
        r#"{ agents = ["architect", "skeptic"], rounds = 3 }"#,
        READ,
    ));
    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.contains("missing field `cap`"),
        "{}",
        outcome.stderr
    );
}

const PRICES: &str =
    "[claude-sonnet-5-5]\ninput = 2.0\noutput = 10.0\ncache_read = 0.1\ncache_write = 2.5\n";

#[test]
fn a_spend_cap_needs_a_price_table() {
    let capped = format!("{FACTORY}\n\n[caps]\ntask = 5.0\nday = 50.0\n");
    let refused = check(&factory("caps-unpriced", &with("factory.toml", &capped)));
    assert_eq!(
        (refused.code, refused.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[caps]` needs prices.toml, which turns the steps' tokens into dollars\n"
        )
    );

    let trigger = format!("{TRIGGER}caps = {{ day = 10.0 }}\n");
    let refused = check(&factory(
        "caps-trigger-unpriced",
        &with("triggers/agent-label.toml", &trigger),
    ));
    assert_eq!(
        (refused.code, refused.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:8: `caps` needs prices.toml, which turns the steps' tokens into dollars\n"
        )
    );

    let lane = format!("{FACTORY}\n\n[caps.lanes.\"core/intake\"]\nday = 10.0\n");
    let refused = check(&factory("caps-lane-unpriced", &with("factory.toml", &lane)));
    assert_eq!(refused.code, 1, "{}", refused.stderr);

    let mut files = with("factory.toml", &capped);
    files.push(("prices.toml", PRICES));
    let priced = check(&factory("caps-priced", &files));
    assert_eq!((priced.code, priced.stderr.as_str()), (0, ""));
}

#[test]
fn a_spend_cap_warns_for_an_agent_whose_model_has_no_price_on_either_harness() {
    let capped = format!("{FACTORY}\n\n[caps]\nday = 50.0\n");
    let unpriced = AGENT.replace("session = ", "model = \"claude-opus-5-5\"\nsession = ");
    let mut files = with("factory.toml", &capped);
    files.retain(|(path, _)| *path != "agents/implementer/agent.md");
    files.push(("agents/implementer/agent.md", &unpriced));
    files.push(("prices.toml", PRICES));
    let warned = check(&factory("caps-model-unpriced", &files));
    assert_eq!(
        (warned.code, warned.stderr.as_str()),
        (
            0,
            "warning: agents/implementer/agent.md:4: the spend caps count nothing for agent `implementer`: prices.toml has no price for its model `claude-opus-5-5`\n"
        )
    );

    let alias = AGENT.replace("session = ", "model = \"sonnet\"\nsession = ");
    let mut files = with("factory.toml", &capped);
    files.retain(|(path, _)| *path != "agents/implementer/agent.md");
    files.push(("agents/implementer/agent.md", &alias));
    files.push(("prices.toml", PRICES));
    let quiet = check(&factory("caps-model-alias", &files));
    assert_eq!((quiet.code, quiet.stderr.as_str()), (0, ""));

    let codex =
        "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\n+++\nMake the change.\n";
    let mut files = with("factory.toml", &capped);
    files.retain(|(path, _)| *path != "agents/implementer/agent.md");
    files.push(("agents/implementer/agent.md", codex));
    files.push(("prices.toml", PRICES));
    let quiet = check(&factory("caps-codex", &files));
    assert_eq!((quiet.code, quiet.stderr.as_str()), (0, ""));

    // Codex names the model it was given, so a Claude alias is no exception there.
    for (name, model) in [
        ("caps-codex-unpriced", "gpt-6-astra"),
        ("caps-codex-alias", "sonnet"),
    ] {
        let agent = format!(
            "+++\ndescription = \"Implements one task.\"\nharness = \"codex\"\nmodel = \"{model}\"\n+++\nMake the change.\n"
        );
        let mut files = with("factory.toml", &capped);
        files.retain(|(path, _)| *path != "agents/implementer/agent.md");
        files.push(("agents/implementer/agent.md", &agent));
        files.push(("prices.toml", PRICES));
        let warned = check(&factory(name, &files));
        assert_eq!(
            (warned.code, warned.stderr),
            (
                0,
                format!(
                    "warning: agents/implementer/agent.md:4: the spend caps count nothing for agent `implementer`: prices.toml has no price for its model `{model}`\n"
                )
            ),
            "{name}"
        );
    }
}

/// `WORKFLOW` with `record = <path>` on its model step.
fn recording(path: &str) -> String {
    WORKFLOW.replace(
        "prompt = \"Implement the issue.\"\n",
        &format!("prompt = \"Implement the issue.\"\nrecord = {path:?}\n"),
    )
}

#[test]
fn a_record_command_is_an_executable_file_inside_the_factory_folder() {
    use std::os::unix::fs::PermissionsExt;
    let outside = recording("../record");
    let mut files = with("workflows/implement.toml", &outside);
    files.push(("bin/record", "#!/bin/sh\nexit 0\n"));
    let dir = factory("record-outside", &files);
    fs::write(dir.join("../record"), "#!/bin/sh\nexit 0\n").unwrap();
    let leaves = check(&dir);
    let inside = recording("bin/record");
    let mut files = with("workflows/implement.toml", &inside);
    files.push(("bin/record", "#!/bin/sh\nexit 0\n"));
    let dir = factory("record-inside", &files);
    std::os::unix::fs::symlink(dir.join("../record"), dir.join("bin/linked")).unwrap();
    let plain = check(&dir);
    fs::set_permissions(dir.join("bin/record"), fs::Permissions::from_mode(0o755)).unwrap();
    let runnable = check(&dir);
    fs::write(
        dir.join("workflows/implement.toml"),
        recording("bin/linked"),
    )
    .unwrap();
    let linked = check(&dir);

    assert_eq!(leaves.code, 1);
    assert!(
        leaves
            .stderr
            .ends_with("step `implement`: `record`: ../record leads outside the factory folder\n"),
        "{}",
        leaves.stderr
    );
    assert!(
        plain
            .stderr
            .ends_with("step `implement`: `record`: bin/record is not executable: chmod +x it\n"),
        "{}",
        plain.stderr
    );
    assert_eq!((runnable.code, runnable.stderr.as_str()), (0, ""));
    assert!(
        linked
            .stderr
            .ends_with("step `implement`: `record`: bin/linked leads outside the factory folder\n"),
        "{}",
        linked.stderr
    );
}

#[test]
fn only_a_model_step_has_a_record_command() {
    let on_code = WORKFLOW.replace(
        "action = \"open-pr\"\n",
        "action = \"open-pr\"\nrecord = \"bin/record\"\n",
    );

    let refused = check(&factory(
        "record-code-step",
        &with("workflows/implement.toml", &on_code),
    ));

    assert_eq!(refused.code, 1);
    assert!(
        refused
            .stderr
            .contains("step `open-pr`: only a model step has a `record` command\n"),
        "{}",
        refused.stderr
    );
}

#[test]
fn the_servers_url_is_an_http_address() {
    let toml = |url: &str| format!("name = \"demo\"\n\n[serve]\nurl = {url:?}\n");
    let ok = check(&factory(
        "serve-url",
        &with("factory.toml", &toml("https://runner.example.ts.net:7390")),
    ));
    let bad = check(&factory(
        "serve-url-bad",
        &with("factory.toml", &toml("runner:7390")),
    ));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(
        (bad.code, bad.stderr.as_str()),
        (
            1,
            "factory.toml:3: `[serve] url` \"runner:7390\" is not an address: write it as https://host:port\n"
        )
    );
}

const CONSULTING: &str = "name = \"demo\"\n\n[consult]\nallow = { implementer = [\"sentry\"] }\n\n[mcp.sentry]\nread_only = \"The token has the org:read and event:read scopes only.\"\n";

const INVESTIGATOR: &str = r#"+++
description = "Reads Sentry issues and answers questions about them."
harness = "claude-code"
model = "claude-haiku-5-5"
investigator = true
mcpServers = ["sentry"]
permissions = { defaultMode = "dontAsk", allow = ["mcp__sentry__get_issue_details", "Read"] }
+++
You read Sentry and answer one question.
"#;

const CONSULT_PRICES: &str =
    "[\"claude-haiku-5-5\"]\ninput = 1.0\noutput = 5.0\ncache_read = 0.1\ncache_write = 1.25\n";

/// The valid factory with the investigator sentry, which the implementer may consult, and
/// `changes` over its files.
fn consulting(name: &str, changes: &[(&str, &str)]) -> Outcome {
    let mut files = with("factory.toml", CONSULTING);
    files.push(("agents/sentry/agent.md", INVESTIGATOR));
    files.push(("prices.toml", CONSULT_PRICES));
    for (path, content) in changes {
        files.retain(|(file, _)| file != path);
        files.push((path, content));
    }
    check(&factory(name, &files))
}

#[test]
fn an_investigator_that_a_worker_may_consult_passes() {
    let outcome = consulting("investigator", &[]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn each_investigator_rule_fails_with_its_line() {
    let investigator = |from: &str, to: &str| INVESTIGATOR.replace(from, to);
    let cases = [
        (
            "investigator-codex",
            (
                "agents/sentry/agent.md",
                investigator("claude-code", "codex"),
            ),
            "agents/sentry/agent.md:3: investigator `sentry` needs harness = \"claude-code\": only Claude Code takes per-tool rules",
        ),
        (
            "investigator-mode",
            ("agents/sentry/agent.md", investigator("dontAsk", "auto")),
            "agents/sentry/agent.md:7: investigator `sentry` needs permissions.defaultMode = \"dontAsk\", so every tool it may call is named in `allow`",
        ),
        (
            "investigator-bash",
            (
                "agents/sentry/agent.md",
                investigator("\"Read\"", "\"Bash(gh issue view:*)\""),
            ),
            "agents/sentry/agent.md:7: investigator `sentry` may not allow `Bash(gh issue view:*)`: it names a tool that writes",
        ),
        (
            "investigator-write",
            (
                "agents/sentry/agent.md",
                investigator("\"Read\"", "\"Write\""),
            ),
            "agents/sentry/agent.md:7: investigator `sentry` may not allow `Write`: it names a tool that writes",
        ),
        (
            "investigator-agent",
            (
                "agents/sentry/agent.md",
                investigator("\"Read\"", "\"Agent(Explore)\""),
            ),
            "agents/sentry/agent.md:7: investigator `sentry` may not allow `Agent(Explore)`: it starts a subagent, which can carry its own permission mode",
        ),
        (
            "investigator-task",
            (
                "agents/sentry/agent.md",
                investigator("\"Read\"", "\"Task\""),
            ),
            "agents/sentry/agent.md:7: investigator `sentry` may not allow `Task`: it starts a subagent, which can carry its own permission mode",
        ),
        (
            "investigator-pattern",
            (
                "agents/sentry/agent.md",
                investigator("mcp__sentry__get_issue_details", "mcp__sentry__get_*"),
            ),
            "agents/sentry/agent.md:7: investigator `sentry` may not allow `mcp__sentry__get_*`: it holds a `*`: write out each tool by its exact name",
        ),
        (
            "investigator-server",
            (
                "agents/sentry/agent.md",
                investigator("mcp__sentry__get_issue_details", "mcp__sentry"),
            ),
            "agents/sentry/agent.md:7: investigator `sentry` may not allow `mcp__sentry`: it names a whole MCP server: write out each of its tools by its exact name",
        ),
        (
            "investigator-unpriced",
            (
                "agents/sentry/agent.md",
                investigator("claude-haiku-5-5", "claude-unpriced"),
            ),
            "agents/sentry/agent.md:4: investigator `sentry` runs model `claude-unpriced`, which prices.toml does not price, so its cap cannot be enforced",
        ),
        (
            "investigator-shared",
            (
                "agents/sentry/agent.md",
                investigator(
                    "investigator = true\n",
                    "investigator = true\nsession = \"shared\"\n",
                ),
            ),
            "agents/sentry/agent.md:6: investigator `sentry` cannot take session = \"shared\": each consult starts a fresh session, so no task's data reaches another's report",
        ),
        (
            "investigator-unstated",
            (
                "factory.toml",
                CONSULTING.replace("[mcp.sentry]", "[mcp.honeycomb]"),
            ),
            "agents/sentry/agent.md:6: investigator `sentry` names MCP server `sentry`, which needs a `read_only` sentence under [mcp.sentry] in factory.toml: where the service enforces that it only reads",
        ),
        (
            "investigator-step",
            (
                "workflows/implement.toml",
                WORKFLOW.replacen("agent = \"implementer\"", "agent = \"sentry\"", 1),
            ),
            "workflows/implement.toml:4: step `implement`: agent `sentry` is an investigator, which only answers consults",
        ),
        (
            "investigator-asker",
            (
                "factory.toml",
                CONSULTING.replace("implementer = [", "sentry = ["),
            ),
            "factory.toml:3: `[consult.allow]` names investigator `sentry` as an asker: an investigator cannot consult, so a consult is one hop",
        ),
        (
            "investigator-unknown-asker",
            (
                "factory.toml",
                CONSULTING.replace("implementer = [", "nobody = ["),
            ),
            "factory.toml:3: `[consult.allow]` names `nobody`, which is no agent",
        ),
        (
            "investigator-not-one",
            (
                "factory.toml",
                CONSULTING.replace("[\"sentry\"]", "[\"implementer\"]"),
            ),
            "factory.toml:3: `[consult.allow]` lets `implementer` consult `implementer`, which is no investigator: set `investigator = true` on it",
        ),
    ];
    for (name, (path, content), line) in cases {
        let outcome = consulting(name, &[(path, &content)]);

        assert_eq!(outcome.code, 1, "{name}: {}", outcome.stderr);
        assert!(
            outcome.stderr.lines().any(|problem| problem == line),
            "{name}: {line}\nin:\n{}",
            outcome.stderr
        );
    }
}

#[test]
fn a_lead_cannot_be_an_investigator() {
    let trigger = format!("{TRIGGER}intervention = \"hands-off\"\nlead = \"sentry\"\n");

    let outcome = consulting(
        "investigator-lead",
        &[("triggers/agent-label.toml", &trigger)],
    );

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:9: lead `sentry` is an investigator, which only answers consults\n"
    );
}

#[test]
fn a_pre_review_names_a_judge_and_a_lesson_names_an_approval_step() {
    let judge = critic_files("diff");
    let mut files = with("factory.toml", "name = \"demo\"\npre_review = \"critic\"\n");
    files.extend(judge.iter().map(|(path, text)| (*path, text.as_str())));
    files.push((
        "memories/lesson-tests.md",
        "---\nname: lesson-tests\ndescription: What a person rejected before.\nstep: implement/approve-pr\n---\nEvery pull request needs a test.\n",
    ));

    let outcome = check(&factory("pre-review-valid", &files));

    assert_eq!(outcome.stderr, "");
    assert_eq!(outcome.code, 0);
}

#[test]
fn a_pre_review_that_is_no_judge_and_a_lesson_on_no_approval_step_are_refused() {
    let mut files = with("factory.toml", "name = \"demo\"\npre_review = \"passed\"\n");
    files.extend_from_slice(&[
        ("scorers/passed.toml", PASSED),
        (
            "memories/lesson-tests.md",
            "---\nname: lesson-tests\ndescription: What a person rejected before.\nstep: implement/implement\n---\nEvery pull request needs a test.\n",
        ),
        (
            "memories/lesson-gone.md",
            "---\nname: lesson-gone\ndescription: What a person rejected before.\nstep: approve-pr\n---\nNo workflow.\n",
        ),
    ]);

    let outcome = check(&factory("pre-review-refused", &files));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "factory.toml:2: the pre-review `passed` is of kind outcome: a pre-review is a judge, which reads the lessons\n\
         memories/lesson-gone.md:4: `step` is `approve-pr`: a lesson names `<workflow>/<step>`, a human step with `approves`\n\
         memories/lesson-tests.md:4: `step` is `implement/implement`: a lesson names `<workflow>/<step>`, a human step with `approves`\n"
    );
}

#[test]
fn a_trigger_with_an_intent_and_noop_cases_passes() {
    let trigger = format!(
        "{TRIGGER}intent = \"Fix each labeled bug with a reviewed pull request.\"\nnoop = [\"The issue is a question, not a bug.\", \"A pull request already fixes it.\"]\n"
    );

    let outcome = check(&factory(
        "intent-ok",
        &with("triggers/agent-label.toml", &trigger),
    ));

    assert_eq!((outcome.code, outcome.stderr.as_str()), (0, ""));
}

#[test]
fn an_intent_is_one_line_and_each_noop_case_says_something() {
    let blank = format!("{TRIGGER}intent = \"  \"\n");
    let lines = format!("{TRIGGER}intent = \"\"\"\nFix bugs.\nAnd more.\"\"\"\n");
    let empty = format!("{TRIGGER}intent = \"Fix bugs.\"\nnoop = [\"A question.\", \"\"]\n");

    let blank = check(&factory(
        "intent-blank",
        &with("triggers/agent-label.toml", &blank),
    ));
    let lines = check(&factory(
        "intent-lines",
        &with("triggers/agent-label.toml", &lines),
    ));
    let empty = check(&factory(
        "noop-empty",
        &with("triggers/agent-label.toml", &empty),
    ));

    assert_eq!(
        (blank.code, blank.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:8: `intent` is empty: say in one sentence the outcome the trigger exists for\n"
        )
    );
    assert_eq!(
        (lines.code, lines.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:8: `intent` is one sentence on one line, not 2 lines\n"
        )
    );
    assert_eq!(
        (empty.code, empty.stderr.as_str()),
        (
            1,
            "triggers/agent-label.toml:9: `noop` case 2 is empty: say which case must create no ask and no write\n"
        )
    );
}

#[test]
fn an_intent_scorer_reads_results_with_a_decision_model_in_the_factory_folder() {
    use std::os::unix::fs::PermissionsExt;
    let good = "description = \"I.\"\non = \"result\"\nkind = \"intent\"\nrun = \"scripts/decide\"\nkey_env = \"TYPESAFE_API_KEY\"\ntimeout = \"10s\"\n";
    let mut files = valid();
    files.extend_from_slice(&[
        ("scorers/intent.toml", good),
        ("scripts/decide", "#!/bin/sh\nexit 0\n"),
    ]);
    let ok = factory("intent-scorer-ok", &files);
    fs::set_permissions(ok.join("scripts/decide"), fs::Permissions::from_mode(0o755)).unwrap();
    let mut files = valid();
    files.extend_from_slice(&[
        (
            "scorers/a.toml",
            "description = \"A.\"\non = \"diff\"\nkind = \"intent\"\nexpect = [\"YES\"]\n",
        ),
        (
            "scorers/b.toml",
            "description = \"B.\"\non = \"result\"\nkind = \"intent\"\nrun = \"scripts/gone\"\nkey_env = \"sk-123 secret\"\n",
        ),
        (
            "scorers/c.toml",
            "description = \"C.\"\non = \"result\"\nkind = \"outcome\"\nexpect = [\"passed\"]\nkey_env = \"TYPESAFE_API_KEY\"\n",
        ),
    ]);

    let ok = check(&ok);
    let bad = check(&factory("intent-scorer-bad", &files));

    assert_eq!((ok.code, ok.stderr.as_str()), (0, ""));
    assert_eq!(bad.code, 1);
    assert_eq!(
        bad.stderr,
        "scorers/a.toml:2: kind `intent` reads on = \"result\", not \"diff\"\n\
         scorers/a.toml:4: an `intent` scorer takes no `expect`: its questions come from the trigger\n\
         scorers/a.toml:3: an `intent` scorer needs `run`: a decision model in the factory folder\n\
         scorers/b.toml:5: `key_env` names the environment variable that holds the model's key, such as TYPESAFE_API_KEY, never the key itself\n\
         scorers/b.toml:4: `run` names scripts/gone, which is missing from the factory folder\n\
         scorers/c.toml:5: `key_env` is for kind `intent`, not `outcome`\n"
    );
}

#[test]
fn a_factory_folder_that_does_not_exist_is_named() {
    let dir = factory("no-such-folder", &[]).join("exampel");

    let outcome = check(&dir);

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        format!("{}: missing: no such folder\n", dir.display())
    );
}
