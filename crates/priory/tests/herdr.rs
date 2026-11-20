use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs `adapters/herdr/deliver` with a fake `hc` that records its arguments and answers `get`
/// with `fake/get.json` when it exists. Returns its exit code and each `hc` call's arguments.
fn deliver(dir: &Path, session: &str) -> (Option<i32>, Vec<Vec<String>>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/herdr/deliver");
    let log = dir.join("fake/hc.log");
    let _ = fs::remove_file(&log);
    let path = std::env::join_paths(
        std::iter::once(dir.join("fake")).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap(),
        )),
    )
    .unwrap();
    let status = Command::new(script)
        .args(["cache.ship", "2", session, "answered", "ship"])
        .env("PATH", path)
        .env("HOME", dir.join("home"))
        .env("FAKE_DIR", dir.join("fake"))
        .status()
        .unwrap();
    let calls = fs::read_to_string(&log)
        .unwrap_or_default()
        .split_terminator('\x1e')
        .map(|call| call.split_terminator('\0').map(str::to_owned).collect())
        .collect();
    (status.code(), calls)
}

fn setup(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("herdr").join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("fake")).unwrap();
    let hc = dir.join("fake/hc");
    fs::write(
        &hc,
        "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\"; done >> \"$FAKE_DIR/hc.log\"\n\
         printf '\\036' >> \"$FAKE_DIR/hc.log\"\n\
         if [ \"$1\" = get ]; then [ -e \"$FAKE_DIR/get.json\" ] || exit 1; cat \"$FAKE_DIR/get.json\"; fi\n",
    )
    .unwrap();
    fs::set_permissions(&hc, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

#[test]
fn deliver_types_into_a_live_claude_terminal_else_resumes_its_session_in_a_new_tab() {
    let dir = setup("deliver");
    let session = "8f3c2a10-0000-4000-8000-000000000001";
    fs::write(
        dir.join("fake/get.json"),
        r#"{"state": "live", "foreground": {"name": "claude"}, "labels": {"herdr.space": "work"}}"#,
    )
    .unwrap();
    let message = "A person answered ask cache.ship (revision 2): ship";

    let live = deliver(&dir, &format!("cache-worker {session}"));

    assert_eq!(
        live,
        (
            Some(0),
            vec![
                vec!["get", "cache-worker", "--json"],
                vec!["send", "cache-worker", message],
            ]
            .into_iter()
            .map(|call| call.into_iter().map(str::to_owned).collect())
            .collect()
        )
    );

    fs::remove_file(dir.join("fake/get.json")).unwrap();
    let project = dir.join("home/.claude/projects/-work-cache");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join(format!("{session}.jsonl")),
        "{\"type\":\"summary\"}\n{\"type\":\"user\",\"cwd\":\"/work/cache\"}\n",
    )
    .unwrap();

    let gone = deliver(&dir, &format!("cache-worker {session}"));
    let without_claude = deliver(&dir, "cache-worker");

    assert_eq!(gone.0, Some(0));
    assert_eq!(
        gone.1[1],
        [
            "run", "--bg", "--keep", "--no-parent", "--cwd", "/work/cache", "--label",
            "herdr.tab=ask-cache.ship", "--", "claude", "--resume", session, message,
        ]
    );
    assert_eq!(without_claude.0, Some(1));
    assert_eq!(without_claude.1.len(), 1);
}
