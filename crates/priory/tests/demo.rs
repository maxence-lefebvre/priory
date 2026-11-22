use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

/// The demo's other runners take writes, so its screenshots show what a remote action does.
#[test]
fn the_demo_sends_an_answer_to_another_runners_ask() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("demo");
    if dir.exists() {
        Command::new("chmod")
            .arg("-R")
            .arg("u+w")
            .arg(&dir)
            .status()
            .unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    // serve.sh ends by serving for 15 minutes, which this stand-in for its binary skips.
    let bin = dir.join("priory");
    let real = env!("CARGO_BIN_EXE_priory");
    fs::write(
        &bin,
        format!("#!/bin/sh\n[ \"$1\" = serve ] && exit 0\nexec '{real}' \"$@\"\n"),
    )
    .unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    let demo = dir.join("demo");

    let seeded = Command::new("sh")
        .arg(root.join("scripts/demo/serve.sh"))
        .arg(&demo)
        .arg(&bin)
        .output()
        .unwrap();
    assert!(seeded.status.success(), "{seeded:?}");

    let sent = Command::new(real)
        .args(["answer"])
        .arg(demo.join("factory"))
        .args(["laptop:4", "split it", "--revision", "2"])
        .env("PRIORY_DB", demo.join("state.db"))
        .env("PRIORY_HOME", demo.join("home"))
        .output()
        .unwrap();
    assert!(sent.status.success(), "{sent:?}");
    assert!(
        String::from_utf8_lossy(&sent.stdout).contains("sent to laptop"),
        "{sent:?}"
    );
}
