//! `priory hook PreToolUse --outside`, the hook of a person's own session, denies the verbs only a
//! person runs and nothing else.

#[allow(dead_code)]
mod harness;

use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    priory_runtime::runtime::remove_sealed(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the hook on `input` with no daemon to ask, and returns its exit and what it printed.
fn hook(dir: &Path, input: &[u8]) -> (Option<i32>, String) {
    let mut hook = harness::priory()
        .args(["hook", "PreToolUse", "--outside"])
        .env_remove("PRIORY_SESSION")
        .env_remove("PRIORY_GUARD")
        .env("PRIORY_HOME", dir.join("home"))
        .env("PRIORY_DB", dir.join("priory.db"))
        .env("XDG_RUNTIME_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin.take().unwrap().write_all(input).unwrap();
    let output = hook.wait_with_output().unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

fn bash(dir: &Path, command: &str) -> (Option<i32>, String) {
    let input = json!({ "session_id": "8f3c2a10", "tool_name": "Bash",
        "tool_input": { "command": command }, "cwd": dir });
    hook(dir, input.to_string().as_bytes())
}

/// The deny's reason, or `None` when the hook let the call through.
fn denied(printed: &str) -> Option<String> {
    if printed.trim().is_empty() {
        return None;
    }
    let output: Value = serde_json::from_str(printed).unwrap();
    let output = &output["hookSpecificOutput"];
    assert_eq!(output["hookEventName"], "PreToolUse");
    assert_eq!(output["permissionDecision"], "deny");
    output["permissionDecisionReason"]
        .as_str()
        .map(str::to_owned)
}

#[test]
fn the_outside_hook_denies_a_person_verb_and_names_the_line_the_person_can_run() {
    let dir = scratch("outside-hook-deny");
    for (command, call) in [
        ("priory answer 14 approved", "priory answer 14 approved"),
        ("env priory dismiss 14", "priory dismiss 14"),
        ("bash -c 'priory cancel 7'", "priory cancel 7"),
        (
            "priory report pick 3 skip --expected 2",
            "priory report pick 3 skip --expected 2",
        ),
    ] {
        let (code, printed) = bash(&dir, command);

        assert_eq!(code, Some(0), "{command}");
        assert_eq!(
            denied(&printed),
            Some(format!(
                "Priory denied `{call}`: only a person runs it, and a model decided this call. Tell the person, who can run it themselves: ! {call}"
            )),
            "{command}"
        );
    }
}

#[test]
fn the_outside_hook_passes_everything_but_a_person_verb() {
    let dir = scratch("outside-hook-pass");
    for command in [
        "priory desk",
        "priory add fix the typo",
        "gh pr merge 12",
        "git push",
    ] {
        let (code, printed) = bash(&dir, command);

        assert_eq!(code, Some(0), "{command}");
        assert_eq!(denied(&printed), None, "{command}");
    }
}

#[test]
fn the_outside_hook_denies_a_call_it_cannot_read() {
    let dir = scratch("outside-hook-unreadable");

    let (code, printed) = hook(&dir, b"not json");
    let nested = "sh -c 'bash -c \"priory answer 1 x\"'";
    let deep = format!("echo {}priory desk{}", "$(echo ".repeat(20), ")".repeat(20));
    let (_, nested) = bash(&dir, nested);
    let (_, deep) = bash(&dir, &deep);

    assert_eq!(code, Some(0));
    assert!(
        denied(&printed).is_some_and(
            |reason| reason.starts_with("Priory's person guard could not read this call (")
        ),
        "{printed}"
    );
    assert!(denied(&nested).is_some(), "{nested}");
    assert_eq!(
        denied(&deep),
        Some(
            "Priory denied a script nested more than 16 levels deep: the guard cannot read it, so it cannot tell whether it runs a verb only a person runs.".to_owned()
        )
    );
}
