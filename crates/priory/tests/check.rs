use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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
        "agents/implementer/agent.md:4:1: unknown field `modle`, expected one of description, harness, model, effort, runner, session, memory, mcpServers, permissions\n"
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
fn only_a_github_trigger_has_backfill() {
    let trigger = "source = \"linear.view\"\nview = \"agent-ready\"\non = \"issue.entered\"\nworkflow = \"implement\"\nbackfill = true\n";

    let outcome = check(&factory(
        "backfill-source",
        &with("triggers/agent-label.toml", trigger),
    ));

    assert_eq!(outcome.code, 1);
    assert_eq!(
        outcome.stderr,
        "triggers/agent-label.toml:5: only a GitHub trigger has `backfill`\n"
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
        "triggers/agent-label.toml:3: a GitHub trigger handles `issues.labeled`, `issues.opened`, `pull_request.labeled` or `pull_request.opened`, not `pull_request.closed`\n\
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
