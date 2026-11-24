#[allow(dead_code)]
mod harness;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Runs `adapters/herdr/deliver` with a fake `hc` that records its arguments and answers `get`
/// with `fake/get.json` when it exists. Returns its exit code and each `hc` call's arguments.
fn deliver(dir: &Path, session: &str) -> (Option<i32>, Vec<Vec<String>>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/herdr/deliver");
    let log = dir.join("fake/hc.log");
    let _ = fs::remove_file(&log);
    let path = std::env::join_paths(
        std::iter::once(dir.join("fake"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let status = harness::command(script)
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

/// Writes the file Claude keeps for its process `pid`, naming the session it runs.
fn claude_runs(dir: &Path, pid: u32, session: &str) {
    let sessions = dir.join("home/.claude/sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join(format!("{pid}.json")),
        format!(r#"{{"pid": {pid}, "sessionId": "{session}"}}"#),
    )
    .unwrap();
}

fn setup(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("herdr")
        .join(name);
    priory_runtime::runtime::remove_sealed(&dir);
    fs::create_dir_all(dir.join("fake")).unwrap();
    let hc = dir.join("fake/hc");
    fs::write(
        &hc,
        "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\0' \"$arg\"; done >> \"$FAKE_DIR/hc.log\"\n\
         printf '\\036' >> \"$FAKE_DIR/hc.log\"\n\
         if [ \"$1\" = get ]; then [ -e \"$FAKE_DIR/get.json\" ] || exit 1; cat \"$FAKE_DIR/get.json\"; fi\n\
         if [ \"$1\" = ls ]; then cat \"$FAKE_DIR/ls.json\" 2>/dev/null || echo '{\"terminals\": []}'; fi\n",
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
        r#"{"state": "live", "foreground": {"name": "claude", "pid": 4242}, "labels": {"herdr.space": "work"}}"#,
    )
    .unwrap();
    claude_runs(&dir, 4242, session);
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
            "run",
            "--bg",
            "--keep",
            "--no-parent",
            "--cwd",
            "/work/cache",
            "--label",
            "herdr.tab=ask-cache.ship",
            "--",
            "claude",
            "--resume",
            session,
            message,
        ]
    );
    assert_eq!(without_claude.0, Some(1));
    assert_eq!(without_claude.1.len(), 1);
}

#[test]
fn deliver_resumes_the_asking_session_when_its_terminal_runs_another_claude_session() {
    let dir = setup("deliver-other");
    let session = "8f3c2a10-0000-4000-8000-000000000001";
    fs::write(
        dir.join("fake/get.json"),
        r#"{"state": "live", "foreground": {"name": "claude", "pid": 4242}, "labels": {"herdr.space": "work"}}"#,
    )
    .unwrap();
    claude_runs(&dir, 4242, "8f3c2a10-0000-4000-8000-000000000002");
    let project = dir.join("home/.claude/projects/-work-cache");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join(format!("{session}.jsonl")),
        "{\"type\":\"user\",\"cwd\":\"/work/cache\"}\n",
    )
    .unwrap();

    let (code, calls) = deliver(&dir, &format!("cache-worker {session}"));

    assert_eq!(code, Some(0));
    assert!(calls.iter().all(|call| call[0] != "send"), "{calls:?}");
    assert_eq!(calls[1][0], "run");
    assert_eq!(calls[1][1..].iter().rev().nth(1).unwrap(), session);
    assert!(
        calls[1].contains(&"herdr.space=work".to_owned()),
        "{calls:?}"
    );

    fs::remove_file(dir.join("home/.claude/sessions/4242.json")).unwrap();
    let (code, calls) = deliver(&dir, &format!("cache-worker {session}"));

    assert_eq!(code, Some(0));
    assert_eq!(
        calls
            .iter()
            .map(|call| call[0].as_str())
            .collect::<Vec<_>>(),
        ["get", "run"]
    );
}

/// Runs `adapters/herdr/deliver` as `[asks] type`, in a daemon that itself runs in an hc terminal.
fn type_into(dir: &Path, args: &[&str]) -> (Option<i32>, Vec<Vec<String>>) {
    let log = dir.join("fake/hc.log");
    let _ = fs::remove_file(&log);
    let path = std::env::join_paths(
        std::iter::once(dir.join("fake"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let status = harness::command(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/herdr/deliver"),
    )
    .args(args)
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

#[test]
fn the_type_verbs_touch_only_the_terminal_that_runs_the_asking_claude_session() {
    let dir = setup("type");
    let session = "8f3c2a10-0000-4000-8000-000000000001";
    let asking = format!("t9 {session}");
    let terminal = |state: &str, name: &str, status: &str| {
        let shown = format!(
            r#"{{"state": "{state}", "status": "{status}", "foreground": {{"name": "{name}", "pid": 4242}}}}"#
        );
        fs::write(dir.join("fake/get.json"), shown).unwrap();
    };
    claude_runs(&dir, 4242, session);
    let last = |args: &[&str]| {
        let (code, calls) = type_into(&dir, args);
        (code, calls.last().unwrap().join(" "))
    };

    terminal("live", "claude", "idle");
    assert_eq!(last(&["check", &asking]), (Some(0), "get t9 --json".into()));
    assert_eq!(
        last(&["send", &asking, "/compact"]),
        (Some(0), "send t9 /compact --if idle".into())
    );
    assert_eq!(last(&["enter", &asking]), (Some(0), "keys t9 enter".into()));
    terminal("live", "claude", "done");
    assert_eq!(
        last(&["send", &asking, "/compact"]),
        (Some(0), "send t9 /compact".into())
    );
    terminal("live", "claude", "working");
    assert_eq!(
        last(&["send", &asking, "/compact"]),
        (Some(75), "get t9 --json".into())
    );

    let refused = || {
        for args in [
            &["check", &asking][..],
            &["send", &asking, "/compact"],
            &["enter", &asking],
        ] {
            assert_eq!(last(args), (Some(1), "get t9 --json".into()), "{args:?}");
        }
    };
    terminal("live", "zsh", "idle");
    refused();
    terminal("finished", "claude", "idle");
    refused();
    terminal("live", "claude", "idle");
    claude_runs(&dir, 4242, "8f3c2a10-0000-4000-8000-000000000002");
    refused();
    fs::remove_file(dir.join("fake/get.json")).unwrap();
    refused();
    assert_eq!(last(&["check", "t9"]).0, Some(1));
}

/// Runs `adapters/herdr/attach 7 12` with `path` as its PATH, space `main` and `exe` as the daemon's
/// `PRIORY_EXE`. Returns its exit code, its stderr and each `hc` call's arguments.
fn attach(
    dir: &Path,
    path: &std::ffi::OsStr,
    exe: Option<&Path>,
) -> (Option<i32>, String, Vec<Vec<String>>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/herdr/attach");
    let log = dir.join("fake/hc.log");
    let _ = fs::remove_file(&log);
    let mut command = harness::command(script);
    if let Some(exe) = exe {
        command.env("PRIORY_EXE", exe);
    }
    let output = command
        .args(["7", "12"])
        .env("PATH", path)
        .env("PRIORY_HERDR_SPACE", "main")
        .env("PRIORY_DB", "/state/priory.db")
        .env("TMUX_TMPDIR", "/tmp/t")
        .env("FAKE_DIR", dir.join("fake"))
        .output()
        .unwrap();
    let calls = fs::read_to_string(&log)
        .unwrap_or_default()
        .split_terminator('\x1e')
        .map(|call| call.split_terminator('\0').map(str::to_owned).collect())
        .collect();
    (
        output.status.code(),
        String::from_utf8(output.stderr).unwrap(),
        calls,
    )
}

/// Writes a fake daemon executable that runs `body`, outside PATH, and returns it.
fn daemon_exe(dir: &Path, body: &str) -> PathBuf {
    let exe = dir.join("daemon-priory");
    fs::write(&exe, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    exe
}

/// The test's PATH with `dir`'s fake `hc` in front.
fn faked_path(dir: &Path) -> std::ffi::OsString {
    std::env::join_paths(
        std::iter::once(dir.join("fake"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap()
}

#[test]
fn attach_opens_one_kept_terminal_per_run_in_the_space_and_a_tab_per_task() {
    let dir = setup("attach");
    let priory = daemon_exe(&dir, "[ \"$*\" = 'attach --run 12 --read-only --print' ]");
    let path = faked_path(&dir);

    let opened = attach(&dir, &path, Some(&priory));
    fs::write(
        dir.join("fake/ls.json"),
        r#"{"terminals": [{"name": "t3", "state": "live"}]}"#,
    )
    .unwrap();
    let again = attach(&dir, &path, Some(&priory));

    let listed = ["ls", "--json", "--label", "priory.run=12"].map(String::from);
    let priory = priory.display().to_string();
    assert_eq!(opened.0, Some(0), "{}", opened.1);
    assert_eq!(
        opened.2,
        [
            listed.to_vec(),
            [
                "run",
                "--bg",
                "--keep",
                "--no-parent",
                "--no-notify",
                "--label",
                "herdr.space=main",
                "--label",
                "herdr.tab=task-7",
                "--label",
                "priory.task=7",
                "--label",
                "priory.run=12",
                "--",
                "env",
                "PRIORY_DB=/state/priory.db",
                "TMUX_TMPDIR=/tmp/t",
                "sh",
                "-c",
                "stty cols 200 rows 51; exec \"$@\"",
                "sh",
                priory.as_str(),
                "attach",
                "--run",
                "12",
                "--read-only",
            ]
            .map(String::from)
            .to_vec(),
        ]
    );
    assert_eq!((again.0, again.2), (Some(0), vec![listed.to_vec()]));
}

#[test]
fn attach_without_hc_on_path_exits_non_zero_and_says_so() {
    let dir = setup("attach-no-hc");
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let python = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|folder| folder.join("python3"))
        .find(|python| python.is_file())
        .unwrap();
    std::os::unix::fs::symlink(python, bin.join("python3")).unwrap();

    let (code, stderr, calls) = attach(&dir, bin.as_os_str(), None);

    assert_eq!(
        (code, stderr.as_str(), calls.len()),
        (
            Some(1),
            "attach: hc is not on PATH; install herdr-core or fix the daemon's PATH\n",
            0
        )
    );
}

#[test]
fn attach_to_a_run_without_a_tagged_pane_opens_nothing_and_says_why() {
    let dir = setup("attach-no-pane");
    let exe = daemon_exe(&dir, "echo 'priory attach: no live pane' >&2; exit 1");

    let (code, stderr, calls) = attach(&dir, &faked_path(&dir), Some(&exe));

    assert_eq!(
        (code, stderr.as_str()),
        (Some(1), "priory attach: no live pane\n")
    );
    assert!(calls.iter().all(|call| call[0] != "run"), "{calls:?}");
}

#[test]
fn attach_without_the_daemons_executable_exits_non_zero_and_says_so() {
    let dir = setup("attach-no-exe");

    let (code, stderr, calls) = attach(&dir, &faked_path(&dir), None);

    assert_eq!(
        (code, stderr.as_str(), calls.len()),
        (
            Some(1),
            "attach: PRIORY_EXE is unset; run this from a daemon that sets it\n",
            0
        )
    );
}
