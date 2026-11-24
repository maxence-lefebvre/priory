//! The one way a test starts a process. The host's own Priory settings stay out of it, so a
//! person whose shell exports `PRIORY_FACTORY` or `PRIORY_DB` runs the suite as CI does.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

/// The variables, besides every `PRIORY_*` and `HERDR_*` one, that Priory reads from the host and
/// a test must not inherit. A test that needs one sets it on the command it builds.
pub const HOST: &[&str] = &[
    "TMUX",
    "DBUS_SESSION_BUS_ADDRESS",
    "OTEL_RESOURCE_ATTRIBUTES",
    "HC_TERMINAL_ID",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "LINEAR_API_KEY",
    "SLACK_BOT_TOKEN",
];

/// Whether the host's variable `name` is kept from a test's process. `PRIORY_TEST_*` configures
/// the suite itself, such as the Postgres database CI provides.
pub fn removed(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    HOST.contains(&name.as_ref())
        || name.starts_with("HERDR_")
        || (name.starts_with("PRIORY_") && !name.starts_with("PRIORY_TEST_"))
}

/// `program`, without the host's Priory settings. `PRIORY_HOME` falls back to
/// `$XDG_STATE_HOME/priory-oss`, so that points into the build's scratch folder, never the
/// person's own state, and
/// `XDG_RUNTIME_DIR` points at a folder with no systemd user manager, so no test reaches the person's.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        if removed(&name) {
            command.env_remove(name);
        }
    }
    command.env(
        "XDG_STATE_HOME",
        Path::new(env!("CARGO_TARGET_TMPDIR")).join("host-state"),
    );
    let run = Path::new(env!("CARGO_TARGET_TMPDIR")).join("host-run");
    std::fs::create_dir_all(&run).unwrap();
    command.env("XDG_RUNTIME_DIR", run);
    command
}

/// The `priory` binary this build made, without the host's Priory settings.
pub fn priory() -> Command {
    command(env!("CARGO_BIN_EXE_priory"))
}
