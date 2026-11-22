//! Every command that takes `[DIR]` reads the same factory folder without it: `PRIORY_FACTORY`
//! when set, else the current folder.

#[allow(dead_code)]
mod harness;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

/// A test folder with `env/`, a factory whose only problem names `bogus_key`, `cwd/`, an empty
/// folder to run from, and `good/`, a factory with no problem.
fn folders(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("factory-folder")
        .join(name);
    priory_runtime::runtime::remove_sealed(&dir);
    for folder in ["env", "cwd", "good", "repo"] {
        fs::create_dir_all(dir.join(folder)).unwrap();
    }
    fs::write(
        dir.join("env/factory.toml"),
        "name = \"env\"\nbogus_key = 1\n",
    )
    .unwrap();
    fs::write(dir.join("good/factory.toml"), "name = \"good\"\n").unwrap();
    dir
}

fn priory(dir: &Path, cwd: &str, factory: Option<&str>, args: &[&str]) -> Output {
    let mut command = harness::priory();
    command
        .args(args)
        .current_dir(dir.join(cwd))
        .env_remove("PRIORY_FACTORY")
        .env_remove("PRIORY_SESSION")
        .env("PRIORY_DB", dir.join("priory.db"))
        .env("PRIORY_HOME", dir.join("home"))
        .env("HERDR_BUS_DB", dir.join("bus.db"))
        .env("XDG_RUNTIME_DIR", dir);
    if let Some(factory) = factory {
        command.env("PRIORY_FACTORY", dir.join(factory));
    }
    command.output().unwrap()
}

/// Each command with `[DIR]`, given no `DIR`, with arguments that make it load the factory.
const COMMANDS: &[&[&str]] = &[
    &["check"],
    &["check", "--repo", "../repo"],
    &["draw", "implement"],
    &["intake", "--once"],
    &["intake", "pre", "some-trigger"],
    &["daemon", "--once"],
    &["daemon", "--view-only"],
    &["batch", "add", "ENG-1"],
    &["add", "fix the build"],
    &[
        "request",
        "--workflow",
        "implement",
        "--key",
        "k",
        "fix the build",
    ],
    &["serve"],
    &["queue"],
    &["schedules"],
    &["links"],
    &["flows", "--json"],
    &["views", "--json"],
    &["screen", "1"],
    &["memories"],
    &["memories", "accept", "candidate"],
    &["answer", "--revision", "1", "b:12", "ok"],
    &["dismiss", "--revision", "1", "b:12"],
    &["give", "b:key", "a"],
    &["resume", "b:key"],
    &["say", "b:key", "hello"],
    &["label", "1", "wrong", "--clear"],
    &["report", "--workflow", "implement", "--cost"],
    &["report", "risk"],
    &["costs"],
    &["score", "1"],
    &["score", "--calibrate", "judge", "--budget", "1"],
    &["calibrate", "triage"],
    &["bench", "capture", "1", "--into", "case"],
    &["bench", "replay", "case", "--budget", "1"],
];

#[test]
fn every_command_with_a_dir_reads_priory_factory_without_one() {
    let dir = folders("env");
    let mut missed = Vec::new();
    for args in COMMANDS {
        let output = priory(&dir, "cwd", Some("env"), args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("unknown field `bogus_key`") {
            missed.push(format!("priory {}: {stderr}", args.join(" ")));
        }
    }
    assert!(
        missed.is_empty(),
        "these read another folder:\n{}",
        missed.join("\n")
    );
}

#[test]
fn a_dir_comes_before_priory_factory_and_priory_factory_before_the_current_folder() {
    let dir = folders("order");
    let good = dir.join("good").display().to_string();

    let given = priory(&dir, "cwd", Some("env"), &["check", &good]);
    assert!(given.status.success(), "{given:?}");

    let env = priory(&dir, "good", Some("env"), &["check"]);
    assert!(!env.status.success(), "{env:?}");
    assert!(
        String::from_utf8_lossy(&env.stderr).contains("bogus_key"),
        "{env:?}"
    );

    let current = priory(&dir, "good", None, &["check"]);
    assert!(current.status.success(), "{current:?}");

    let unset = harness::priory()
        .arg("check")
        .current_dir(dir.join("good"))
        .env("PRIORY_FACTORY", "")
        .output()
        .unwrap();
    assert!(
        unset.status.success(),
        "an empty PRIORY_FACTORY counts as unset: {unset:?}"
    );
}
