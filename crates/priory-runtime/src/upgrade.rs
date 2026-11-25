//! The journal of an upgrade in progress, which the helper that runs it, the daemon and the intake
//! read. It stays until the upgrade ended, so whoever starts next knows where it stopped.

use std::fs::{self, File};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// The new binary changes the schema of the state, or of the log.
    #[serde(default)]
    pub migrates_state: bool,
    #[serde(default)]
    pub migrates_bus: bool,
    /// The new binary may have opened the databases.
    #[serde(default)]
    pub opened: bool,
    pub exe: PathBuf,
    pub staged: PathBuf,
    /// The verified digest of `staged`, as lower-case hex.
    pub sha256: String,
    /// The release's name of the binary.
    pub asset: String,
    /// GitHub's attestations of `staged`, and of the binary it replaces.
    pub attestations: PathBuf,
    pub own: PathBuf,
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
    /// Past this second since the epoch the helper stops nothing any more.
    #[serde(default)]
    pub ready_by: Option<u64>,
    /// The address the running daemon's server holds.
    #[serde(default)]
    pub bound: Option<SocketAddr>,
    /// The intake ran without an end for as long as it would be watched.
    #[serde(default)]
    pub intake_stable: bool,
    /// Why it rolls back or stopped short.
    #[serde(default)]
    pub reason: Option<String>,
    /// The release is at fault, so it is never tried again.
    #[serde(default)]
    pub refuse: bool,
    /// Whether each unit ran before the helper stopped both; `None` until it does.
    #[serde(default)]
    pub running: Option<[bool; 2]>,
}

impl Journal {
    pub fn migrates(&self) -> bool {
        self.migrates_state || self.migrates_bus
    }
}

/// The seconds since the epoch.
pub fn now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.unwrap_or_default().as_secs()
}

/// The folder of the upgrade's own files.
pub fn dir(home: &Path) -> PathBuf {
    home.join("upgrade")
}

/// Makes that folder, which no other user may enter: a file in it becomes the daemon's binary.
pub fn private(home: &Path) -> Result<(), String> {
    fs::create_dir_all(home).map_err(|error| format!("{}: {error}", home.display()))?;
    crate::runtime::private_dir(&dir(home), crate::runtime::uid(), None)
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
    written.map_err(|error| {
        let _ = fs::remove_file(&next);
        format!("{}: {error}", path.display())
    })
}

/// Renames `from` over `to`, which needs no room on the disk.
pub fn rename(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to)
        .and_then(|()| sync_dir(to))
        .map_err(|error| format!("{} to {}: {error}", from.display(), to.display()))
}

/// Copies `from` over `to` so that a crash leaves either the old file or the new one, with the same
/// permissions as `from`.
pub fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let next = beside(to, "next");
    let copied = fs::copy(from, &next)
        .and_then(|_| File::open(&next)?.sync_all())
        .and_then(|()| fs::rename(&next, to))
        .and_then(|()| sync_dir(to));
    copied.map_err(|error| {
        let _ = fs::remove_file(&next);
        format!("{} to {}: {error}", from.display(), to.display())
    })
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
    private(home)?;
    let file = File::create(&path).map_err(|error| format!("{}: {error}", path.display()))?;
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
            if journal.migrates()
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
        if let Err(error) = start(home, &journal) {
            // Its journal is gone when nothing was changed yet, so the service goes on.
            return match read(home)?.map(|journal| journal.phase) {
                None | Some(Phase::Committed | Phase::Restarting) => Ok(()),
                Some(_) => Err(error),
            };
        }
        // The binary is settled, and the helper only has its end to record.
        if matches!(journal.phase, Phase::Committed | Phase::Restarting) {
            STARTED.store(daemon, Ordering::Relaxed);
            return Ok(());
        }
        return Err("an upgrade of priory was interrupted: its helper starts again".to_owned());
    }
    match journal.phase {
        Phase::Committed | Phase::Restarting => {}
        Phase::Starting if daemon || !journal.migrates() => {}
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
        Ok(Some(journal)) if journal.migrates() && journal.phase == Phase::Starting
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

/// How long the helper has to say its version, and then to take its lock.
const HELPER: Duration = Duration::from_secs(10);

/// Starts the helper as a transient user unit, so stopping the services does not stop it, and waits
/// until it runs. Without it, a journal that changed nothing yet is removed.
pub fn start(home: &Path, journal: &Journal) -> Result<(), String> {
    let unit = format!("priory-upgrade-{}-{}", journal.op, std::process::id());
    let launched = launch(home, journal, &unit).and_then(|()| {
        let until = Instant::now() + HELPER;
        loop {
            // A helper that already ended leaves no journal.
            if lock(home, Duration::ZERO)?.is_none() || read(home)?.is_none() {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(format!(
                    "the helper of the upgrade did not start within {} seconds",
                    HELPER.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let Err(error) = launched else { return Ok(()) };
    // Under the lock, so a helper that starts late finds no journal.
    let Some(_lock) = lock(home, Duration::ZERO)? else {
        return Ok(());
    };
    let _ = systemctl(&["stop", &unit]);
    match read(home)? {
        Some(journal) if journal.phase == Phase::Stopping => remove(home)?,
        _ => {}
    }
    Err(error)
}

fn launch(home: &Path, journal: &Journal, unit: &str) -> Result<(), String> {
    let helper = helper(home);
    let said = crate::runtime::fed_within(Command::new(&helper).arg("--version"), None, HELPER);
    let said = said.map_err(|error| format!("{}: {error}", helper.display()))?;
    let said = said.map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    if said != Some(format!("priory {}", journal.from)) {
        return Err(format!(
            "the helper of the upgrade does not run: it says {}",
            said.as_deref().unwrap_or("nothing")
        ));
    }
    let mut command = Command::new("systemd-run");
    command.args(["--user", "--quiet", "--collect"]);
    command.arg(format!("--unit={unit}"));
    command.args([
        "--property=Type=exec",
        "--property=Restart=on-failure",
        "--property=RestartSec=2",
        "--property=StartLimitIntervalSec=0",
    ]);
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
    command.arg("--").arg(helper).args(["upgrade", "resume"]);
    crate::runtime::run(&mut command).map(drop)
}

/// How long a helper that died has to be started again by its unit.
const GONE: Duration = Duration::from_secs(30);

/// How long past its time to get ready a helper may still stop the services.
const LATE: u64 = 10;

/// Waits while the helper works, until it tells this process to stop, puts the upgrade off or ran out of
/// time to stop anything. A helper that is gone for good before it changed anything is given up.
pub fn wait(home: &Path) -> Result<(), String> {
    let mut gone = None;
    while !crate::runtime::stopping()
        && let Some(journal) = read(home)?
    {
        let late = journal.ready_by.is_some_and(|by| now() > by + LATE);
        match lock(home, Duration::ZERO)? {
            None if late && journal.running.is_none() => return Ok(()),
            None => gone = None,
            Some(_lock) => {
                let since = *gone.get_or_insert_with(Instant::now);
                if since.elapsed() >= GONE {
                    // Read again, under the lock.
                    match read(home)? {
                        Some(journal) if journal.phase == Phase::Stopping => remove(home)?,
                        _ => {}
                    }
                    return Err("the helper of the upgrade is gone".to_owned());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
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
