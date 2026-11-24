//! The suite runs as CI runs it, whatever the person's shell exports: every process a test starts
//! goes through `harness`, which keeps the host's Priory settings out.

#[allow(dead_code)]
mod harness;

use std::fs;
use std::path::Path;

/// What a person's shell may export. `PRIORY_TEST_*` configures the suite, so it is kept.
const EXPORTED: &[(&str, &str)] = &[
    ("PRIORY_FACTORY", "/nonexistent/factory"),
    ("PRIORY_HOME", "/nonexistent/home"),
    ("PRIORY_DB", "/nonexistent/priory.db"),
    ("HERDR_BUS_DB", "/nonexistent/bus.db"),
    ("PRIORY_TOKEN", "host-token"),
    ("PRIORY_LOG_URL", "postgresql://host/priory"),
    ("PRIORY_NOW", "2001-01-01T00:00:00Z"),
    ("PRIORY_SESSION", "host-session"),
    ("TMUX", "/tmp/host-tmux,1,0"),
    ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent/bus"),
    ("PRIORY_TEST_HOST_ENV", "1"),
];

#[test]
fn a_started_process_inherits_none_of_the_hosts_priory_settings() {
    let output = harness::command(std::env::current_exe().unwrap())
        .envs(EXPORTED.iter().copied())
        .args([
            "--exact",
            "inside_a_host_that_exports_priory_settings",
            "--include-ignored",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "a_started_process_inherits_none_of_the_hosts_priory_settings runs it with EXPORTED set"]
fn inside_a_host_that_exports_priory_settings() {
    if std::env::var_os("PRIORY_TEST_HOST_ENV").is_none() {
        return;
    }
    let output = harness::command("env").output().unwrap();
    let listed = String::from_utf8(output.stdout).unwrap();
    let names: Vec<&str> = listed
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .collect();
    for (name, _) in EXPORTED {
        let kept = name.starts_with("PRIORY_TEST_");
        assert_eq!(names.contains(name), kept, "{name} kept: {listed}");
    }
    let state = Path::new(env!("CARGO_TARGET_TMPDIR")).join("host-state");
    assert!(
        listed.contains(&format!("\nXDG_STATE_HOME={}\n", state.display())),
        "{listed}"
    );
    let run = Path::new(env!("CARGO_TARGET_TMPDIR")).join("host-run");
    assert!(
        listed.contains(&format!("\nXDG_RUNTIME_DIR={}\n", run.display())),
        "{listed}"
    );
    assert!(!run.join("systemd").exists() && !run.join("bus").exists());
}

#[test]
fn no_test_starts_a_process_but_through_the_harness() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut found = Vec::new();
    let mut folders = vec![tests.clone()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && path != tests.join("harness/mod.rs")
                && fs::read_to_string(&path)
                    .unwrap()
                    .contains(concat!("Command", "::new("))
            {
                found.push(path);
            }
        }
    }
    assert!(
        found.is_empty(),
        "start processes with harness::command or harness::priory, not Command::new: {found:?}"
    );
}
