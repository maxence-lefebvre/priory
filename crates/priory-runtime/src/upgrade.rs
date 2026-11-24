//! The journal of an upgrade in progress, which the helper that runs it, the daemon and the intake
//! read. It stays until the upgrade ended, so whoever starts next knows where it stopped.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// The services stop and the backups are made: nothing changed yet.
    Stopping,
    /// The new binary takes the old one's place and is checked.
    Installing,
    /// The new daemon starts and is watched.
    Starting,
    /// The new binary stays.
    Committed,
    /// The old binary, and after a migration the backups, take their places back.
    RollingBack,
    /// The old binary's services start again.
    Restarting,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    /// Names this upgrade in its files, its unit and its trace record.
    pub op: String,
    pub phase: Phase,
    pub from: String,
    pub to: String,
    pub schema_from: i64,
    pub schema_to: i64,
    /// The new binary changes the schema of the state or of the log.
    pub migrates: bool,
    /// The new binary may have opened the databases.
    #[serde(default)]
    pub opened: bool,
    pub exe: PathBuf,
    pub staged: PathBuf,
    /// The verified digest of `staged`, as lower-case hex.
    pub sha256: String,
    pub kept: PathBuf,
    pub factory: PathBuf,
    pub state: PathBuf,
    pub bus: PathBuf,
    pub state_backup: PathBuf,
    pub bus_backup: PathBuf,
    /// The systemd user units of the daemon and the intake.
    pub units: [String; 2],
    /// How many seconds the new daemon has to answer, and then how many it is watched.
    pub answers_within: u64,
    pub watch: u64,
    /// Why it rolls back or stopped short.
    #[serde(default)]
    pub reason: Option<String>,
}

/// The folder of the upgrade's own files.
pub fn dir(home: &Path) -> PathBuf {
    home.join("upgrade")
}

fn path(home: &Path) -> PathBuf {
    dir(home).join("journal.json")
}

/// The journal of the upgrade in progress, if one is.
pub fn read(home: &Path) -> Result<Option<Journal>, String> {
    let path = path(home);
    let at = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| at(&error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(at(&error)),
    }
}

pub fn write(home: &Path, journal: &Journal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(journal).expect("a journal serializes");
    replace(&path(home), &bytes)
}

pub fn remove(home: &Path) -> Result<(), String> {
    let path = path(home);
    fs::remove_file(&path)
        .and_then(|()| sync_dir(&path))
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Writes `bytes` to `path` so that a crash leaves either the old file or the new one.
pub fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let next = beside(path, "next");
    let written = fs::create_dir_all(path.parent().unwrap_or(Path::new("/")))
        .and_then(|()| File::create(&next))
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| fs::rename(&next, path))
        .and_then(|()| sync_dir(path));
    written.map_err(|error| format!("{}: {error}", path.display()))
}

/// Copies `from` over `to` so that a crash leaves either the old file or the new one, with the same
/// permissions as `from`.
pub fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let next = beside(to, "next");
    let copied = fs::copy(from, &next)
        .and_then(|_| File::open(&next)?.sync_all())
        .and_then(|()| fs::rename(&next, to))
        .and_then(|()| sync_dir(to));
    copied.map_err(|error| format!("{} to {}: {error}", from.display(), to.display()))
}

/// `path` with `.suffix` after its name.
pub fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{suffix}"));
    PathBuf::from(name)
}

fn sync_dir(path: &Path) -> std::io::Result<()> {
    File::open(path.parent().unwrap_or(Path::new("/")))?.sync_all()
}

/// The helper's lock, held while it runs; `None` when another process holds it past `wait`.
pub fn lock(home: &Path, wait: Duration) -> Result<Option<File>, String> {
    let path = dir(home).join("lock");
    let file = fs::create_dir_all(dir(home))
        .and_then(|()| File::create(&path))
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let deadline = Instant::now() + wait;
    while file.try_lock().is_err() {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(Some(file))
}

/// The line a person, or a verb, gets while an upgrade holds the databases.
pub const IN_PROGRESS: &str = "an upgrade of priory is in progress: try again in a few minutes";

/// Whether this process is the daemon the upgrade started, which may open the databases.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Refuses to open a database an upgrade may put back from its backup.
pub fn barrier() -> Result<(), String> {
    if STARTED.load(Ordering::Relaxed) {
        return Ok(());
    }
    match read(&crate::runtime::home())? {
        Some(journal)
            if journal.migrates
                && !matches!(journal.phase, Phase::Committed | Phase::Restarting) =>
        {
            Err(IN_PROGRESS.to_owned())
        }
        _ => Ok(()),
    }
}

/// Whether the service this process runs may start. `daemon` starts while it is being watched; the
/// intake waits for the commit of a migration. An upgrade nobody works on, as after a crash or a
/// reboot, starts its helper again and refuses.
pub fn admit(home: &Path, daemon: bool) -> Result<(), String> {
    let Some(journal) = read(home)? else {
        return Ok(());
    };
    if lock(home, Duration::ZERO)?.is_some() {
        start(home, &journal)?;
        return Err("an upgrade of priory was interrupted: its helper starts again".to_owned());
    }
    match journal.phase {
        Phase::Committed | Phase::Restarting => {}
        Phase::Starting if daemon || !journal.migrates => {}
        _ => return Err(IN_PROGRESS.to_owned()),
    }
    if daemon {
        STARTED.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Whether the upgrade's migration still waits for its commit, so the daemon does no work yet.
pub fn held(home: &Path) -> bool {
    matches!(
        read(home),
        Ok(Some(journal)) if journal.migrates && journal.phase == Phase::Starting
    )
}

/// Whether a held daemon still waits. A rollback never lets it go on: the helper stops it.
pub fn uncommitted(home: &Path) -> bool {
    !matches!(
        read(home),
        Ok(None)
            | Ok(Some(Journal {
                phase: Phase::Committed,
                ..
            }))
    )
}

/// The private copy of the old binary that runs the upgrade.
pub fn helper(home: &Path) -> PathBuf {
    dir(home).join("helper")
}

/// Starts the helper as a transient user unit, so stopping the services does not stop it.
pub fn start(home: &Path, journal: &Journal) -> Result<(), String> {
    let mut command = Command::new("systemd-run");
    command.args(["--user", "--quiet", "--collect"]);
    command.arg(format!(
        "--unit=priory-upgrade-{}-{}",
        journal.op,
        std::process::id()
    ));
    command.args(["--property=Restart=on-failure", "--property=RestartSec=2"]);
    let path = std::env::var_os("PATH").unwrap_or_default();
    for (name, value) in [
        ("PRIORY_HOME", home.as_os_str()),
        ("PRIORY_DB", journal.state.as_os_str()),
        ("HERDR_BUS_DB", journal.bus.as_os_str()),
        ("PATH", path.as_os_str()),
    ] {
        let mut setting = std::ffi::OsString::from(format!("--setenv={name}="));
        setting.push(value);
        command.arg(setting);
    }
    command
        .arg("--")
        .arg(helper(home))
        .args(["upgrade", "resume"]);
    crate::runtime::run(&mut command).map(drop)
}

/// Runs `systemctl --user` with `args` and returns what it printed.
pub fn systemctl(args: &[&str]) -> Result<String, String> {
    crate::runtime::run(Command::new("systemctl").arg("--user").args(args))
}

/// The value of `property` of the user unit `unit`.
pub fn property(unit: &str, property: &str) -> Result<String, String> {
    let name = format!("--property={property}");
    systemctl(&["show", "--value", &name, unit]).map(|value| value.trim().to_owned())
}

/// The version the daemon on `home` answers with.
pub fn version(home: &Path) -> Result<String, String> {
    let reply = crate::runtime::call(home, &json!({ "version": true }).to_string(), ANSWER)?;
    serde_json::from_str::<Result<String, String>>(&reply).map_err(|error| error.to_string())?
}

const ANSWER: Duration = Duration::from_secs(5);
