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
            format!("priory {}\n", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A release folder for a file URL: the fake binary for every target, with `checksum` as the
/// digest in the `.sha256` beside each.
fn release(name: &str, checksum: &str) -> PathBuf {
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

/// Runs `install.sh`, with `path_first` ahead of `PATH` when given.
fn install(release: &Path, prefix: &Path, path_first: Option<&Path>) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../install.sh");
    let mut command = Command::new("sh");
    if let Some(dir) = path_first {
        let path = std::env::var("PATH").unwrap();
        command.env("PATH", format!("{}:{path}", dir.display()));
    }
    command
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
    let release = release("checked", &sha256(FAKE));
    let prefix = scratch("checked-prefix");

    let output = install(&release, &prefix, None);

    assert!(output.status.success(), "{output:?}");
    let installed = prefix.join("bin/priory");
    assert_eq!(fs::read(&installed).unwrap(), FAKE);
    assert_eq!(
        fs::metadata(&installed).unwrap().permissions().mode() & 0o111,
        0o111
    );
}

#[test]
fn install_refuses_a_wrong_or_missing_checksum_and_keeps_the_installed_binary() {
    let wrong = sha256(b"something else");
    let broken = scratch("broken-hasher");
    fs::write(broken.join("sha256sum"), "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(broken.join("sha256sum"), fs::Permissions::from_mode(0o755)).unwrap();
    for (name, checksum, path_first) in [
        ("wrong", wrong.as_str(), None),
        ("empty", "", None),
        ("garbled", "not-hex", None),
        ("unhashable", "", Some(broken.as_path())),
    ] {
        let release = release(name, checksum);
        let prefix = scratch(&format!("{name}-prefix"));
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::write(prefix.join("bin/priory"), b"old priory").unwrap();

        let output = install(&release, &prefix, path_first);

        assert!(!output.status.success(), "{name}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("checksum"),
            "{name}: {output:?}"
        );
        assert_eq!(fs::read(prefix.join("bin/priory")).unwrap(), b"old priory");
        assert_eq!(
            fs::read_dir(prefix.join("bin")).unwrap().count(),
            1,
            "{name}"
        );
    }
}
