use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TARGETS: [&str; 3] = [
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
];
const FAKE: &[u8] = b"#!/bin/sh\necho fake priory\n";

#[test]
fn version_prints_the_workspace_version() {
    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_priory"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "priory 0.1.0\n",
            "{flag}"
        );
    }
}

/// A release folder for a file URL: the fake binary for every target, with the checksum of
/// `checksummed` beside each.
fn release(name: &str, checksummed: &[u8]) -> PathBuf {
    let checksum: String = Sha256::digest(checksummed)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let dir = scratch(&format!("{name}-release"));
    for target in TARGETS {
        let asset = format!("priory-{target}");
        fs::write(dir.join(&asset), FAKE).unwrap();
        fs::write(
            dir.join(format!("{asset}.sha256")),
            format!("{checksum}  {asset}\n"),
        )
        .unwrap();
    }
    dir
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("install-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn install(release: &Path, prefix: &Path) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
    Command::new("sh")
        .arg(script)
        .arg("--prefix")
        .arg(prefix)
        .env(
            "PRIORY_RELEASE_URL",
            format!("file://{}", release.display()),
        )
        .output()
        .unwrap()
}

#[test]
fn install_puts_the_checked_binary_in_the_prefix() {
    let release = release("checked", FAKE);
    let prefix = scratch("checked-prefix");

    let output = install(&release, &prefix);

    assert!(output.status.success(), "{output:?}");
    let installed = prefix.join("bin/priory");
    assert_eq!(fs::read(&installed).unwrap(), FAKE);
    assert_eq!(
        fs::metadata(&installed).unwrap().permissions().mode() & 0o111,
        0o111
    );
}

#[test]
fn install_refuses_a_wrong_checksum() {
    let release = release("wrong", b"something else");
    let prefix = scratch("wrong-prefix");

    let output = install(&release, &prefix);

    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("checksum"),
        "{output:?}"
    );
    assert!(!prefix.join("bin/priory").exists());
}
