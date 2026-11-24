//! The daemon's part of an upgrade: it finds a release, verifies and stages it, waits for a quiet
//! moment, rehearses it on copies of the databases, and starts the helper that swaps the binary.

use crate::trace::{self, Trace};
use crate::{memory, post, provenance, swap};
use jiff::Timestamp;
use priory_factory::factory::{Channel, Upgrade};
use priory_runtime::upgrade::{self as journal, Journal, Phase};
use priory_runtime::{room, transcript};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

type Error = Box<dyn std::error::Error>;

pub(crate) const INSTALLED: &str = "factory.upgrade.installed";
pub(crate) const ROLLED_BACK: &str = "factory.upgrade.rolled_back";
pub(crate) const REFUSED: &str = "factory.upgrade.refused";
/// An upgrade the helper put off, which no record holds.
pub(crate) const PUT_OFF: &str = "put-off";

const REPOSITORY: &str = "maxence-lefebvre/priory";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const EVERY: Duration = Duration::from_secs(3600);
/// How often a staged release looks for a quiet moment.
const LOOK: Duration = Duration::from_secs(5);
/// How long a release waits after the helper put it off.
const PUT_OFF_FOR: Duration = Duration::from_secs(300);
/// How long a release waits for a quiet moment before the desk says why.
const PATIENCE: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);
const ANSWERS_WITHIN: u64 = 120;
const WATCH: u64 = 60;
/// The most lines `noted` remembers.
const NOTED: usize = 50;
const BINARY_MAX: u64 = 200 << 20;
const ANSWER_MAX: u64 = 8 << 20;

/// This machine's release target, where one exists.
fn target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

/// What outlives one upgrade, in `state.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Store {
    /// The versions never tried again, each with why.
    pub refused: BTreeMap<String, String>,
    /// The lines the desk already got.
    pub noted: Vec<String>,
    pub kept: Vec<Kept>,
}

/// The old binary and the backups one upgrade keeps.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Kept {
    pub op: String,
    pub files: Vec<PathBuf>,
}

impl Store {
    fn path(home: &Path) -> PathBuf {
        journal::dir(home).join("state.json")
    }

    pub(crate) fn read(home: &Path) -> Result<Store, String> {
        Ok(read_json(&Self::path(home))?.unwrap_or_default())
    }

    pub(crate) fn write(&self, home: &Path) -> Result<(), String> {
        write_json(&Self::path(home), self)
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("{}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    journal::replace(
        path,
        &serde_json::to_vec_pretty(value).expect("it serializes"),
    )
}

/// How an upgrade ended, which the daemon records once.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Outcome {
    pub op: String,
    pub kind: String,
    pub from: String,
    pub to: String,
    pub schema_from: i64,
    pub schema_to: i64,
    pub reason: Option<String>,
    pub backups: Vec<PathBuf>,
    pub kept: PathBuf,
}

fn outcomes(home: &Path) -> PathBuf {
    journal::dir(home).join("outcomes")
}

impl Outcome {
    pub(crate) fn write(&self, home: &Path) -> Result<(), String> {
        write_json(&outcomes(home).join(format!("{}.json", self.op)), self)
    }

    fn line(&self) -> String {
        let Outcome { from, to, .. } = self;
        let reason = self.reason.as_deref().unwrap_or("no reason");
        let kept = self.kept.display();
        if self.kind == INSTALLED {
            return format!("priory upgraded from {from} to {to}; the old binary is {kept}");
        }
        format!("priory went back from {to} to {from} and will not try {to} again: {reason}")
    }
}

/// A verified release that waits for a quiet moment.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Staged {
    version: String,
    sha256: String,
    since: Timestamp,
}

pub(crate) fn staged_file(home: &Path) -> PathBuf {
    journal::dir(home).join("staged.json")
}

fn staged_binary(home: &Path) -> PathBuf {
    journal::dir(home).join("staged")
}

/// What the new binary does to copies of the databases.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rehearsal {
    schema_from: i64,
    schema_to: i64,
    migrates: bool,
}

/// How a rehearsal failed: the release is at fault, or this machine is.
enum Unrehearsed {
    Release(String),
    Machine(String),
}

enum Work {
    Looking(Receiver<Result<Option<Staged>, String>>),
    Rehearsing(Receiver<Result<Rehearsal, Unrehearsed>>),
}

/// The daemon's upgrades: what it looks for, has staged and rehearsed.
#[derive(Default)]
pub(crate) struct Upgrader {
    work: Option<Work>,
    looked: Option<Instant>,
    checked: Option<Instant>,
    /// Not before, after an upgrade was put off.
    after: Option<Instant>,
    rehearsed: Option<(String, Rehearsal)>,
    error: Option<String>,
}

impl Upgrader {
    /// Moves an upgrade on. Returns whether its helper started, so the daemon must stop.
    pub(crate) fn tick(
        &mut self,
        db: &mut Connection,
        trace: &Trace,
        home: &Path,
        working: bool,
    ) -> Result<bool, Error> {
        match self.moved(db, trace, home, working) {
            Ok(started) => Ok(started),
            // An upgrade that cannot go on never stops the daemon.
            Err(error) => {
                let error = error.to_string();
                if self.error.as_ref() != Some(&error) {
                    eprintln!("priory daemon: upgrade: {error}");
                    self.error = Some(error);
                }
                Ok(false)
            }
        }
    }

    fn moved(
        &mut self,
        db: &mut Connection,
        trace: &Trace,
        home: &Path,
        working: bool,
    ) -> Result<bool, Error> {
        self.import(db, trace, home)?;
        let runner = trace
            .machine
            .as_deref()
            .and_then(|machine| trace.factory.runners.get(machine));
        let Some(upgrade) = trace.factory.upgrade() else {
            return Ok(false);
        };
        if runner.is_some_and(|runner| !runner.value.upgrade) {
            return Ok(false);
        }
        self.landed(db, trace, home)?;
        if self.work.is_some() || journal::read(home)?.is_some() {
            return Ok(false);
        }
        let every = upgrade.every.unwrap_or(EVERY);
        if self.looked.is_none_or(|at| at.elapsed() >= every) {
            self.looked = Some(Instant::now());
            match host(upgrade) {
                Ok(_) => self.look(upgrade, home)?,
                Err(reason) => {
                    let line = format!("priory cannot upgrade itself here: {reason}");
                    note(db, trace, home, &line, json!({ "reason": reason }))?;
                }
            }
            return Ok(false);
        }
        if self.checked.is_some_and(|at| at.elapsed() < LOOK)
            || self.after.is_some_and(|after| Instant::now() < after)
        {
            return Ok(false);
        }
        self.checked = Some(Instant::now());
        let Some(staged) = read_json::<Staged>(&staged_file(home))? else {
            return Ok(false);
        };
        self.attempt(db, trace, home, upgrade, &staged, working)
    }

    /// Takes a staged release through its quiet moment, its rehearsal and the start of its helper.
    fn attempt(
        &mut self,
        db: &mut Connection,
        trace: &Trace,
        home: &Path,
        upgrade: &Upgrade,
        staged: &Staged,
        working: bool,
    ) -> Result<bool, Error> {
        let now = Timestamp::now();
        // A person may have installed it, or narrowed the channel, since it was staged.
        if !Store::read(home)?.accepts(upgrade.channel, &staged.version) {
            fs::remove_file(staged_file(home))?;
            return Ok(false);
        }
        if upgrade
            .window
            .as_ref()
            .is_some_and(|window| window.holds(now))
        {
            return Ok(false);
        }
        let rehearsed = self
            .rehearsed
            .as_ref()
            .filter(|(version, _)| *version == staged.version)
            .map(|(_, rehearsal)| *rehearsal);
        memory::reclaim(db)?;
        let busy = match working {
            true => Some("the daemon runs a command".to_owned()),
            false => busy(db, rehearsed.is_some_and(|rehearsal| rehearsal.migrates))?,
        };
        if let Some(busy) = busy {
            if now.duration_since(staged.since) >= PATIENCE {
                let version = &staged.version;
                let line = format!("priory {version} waits for a quiet moment for a day: {busy}");
                note_once(db, home, &format!("waits {version}"), &line)?;
            }
            return Ok(false);
        }
        let host = host(upgrade)?;
        let Some(rehearsal) = rehearsed else {
            self.rehearse(home, staged, &host, &trace.factory.dir);
            return Ok(false);
        };
        let journal = plan(home, staged, rehearsal, host, upgrade, &trace.factory.dir)?;
        room_for(home, &journal)?;
        journal::copy(&journal.exe, &journal::helper(home))?;
        journal::write(home, &journal)?;
        if let Err(error) = journal::start(home, &journal) {
            journal::remove(home)?;
            return Err(error.into());
        }
        println!("priory daemon: upgrading to {}", journal.to);
        Ok(true)
    }

    /// Records what the helpers of ended upgrades left, each once.
    fn import(&mut self, db: &mut Connection, trace: &Trace, home: &Path) -> Result<(), Error> {
        let Ok(entries) = fs::read_dir(outcomes(home)) else {
            return Ok(());
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(outcome) = read_json::<Outcome>(&path)? else {
                continue;
            };
            if outcome.kind == PUT_OFF {
                self.after = Some(Instant::now() + PUT_OFF_FOR);
                fs::remove_file(&path)?;
                continue;
            }
            record(db, trace, &outcome)?;
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Takes what the background thread found.
    fn landed(&mut self, db: &mut Connection, trace: &Trace, home: &Path) -> Result<(), Error> {
        match self.work.take() {
            Some(Work::Looking(receiver)) => match receiver.try_recv() {
                Ok(Ok(Some(staged))) => {
                    write_json(&staged_file(home), &staged)?;
                    self.error = None;
                }
                Ok(Ok(None)) => self.error = None,
                Ok(Err(error)) if error.starts_with(UNVERIFIED) => {
                    note(db, trace, home, &error, json!({ "reason": error }))?;
                }
                Ok(Err(error)) => return Err(error.into()),
                Err(TryRecvError::Empty) => self.work = Some(Work::Looking(receiver)),
                Err(TryRecvError::Disconnected) => {}
            },
            Some(Work::Rehearsing(receiver)) => match receiver.try_recv() {
                Ok(Ok(rehearsal)) => {
                    if let Some(staged) = read_json::<Staged>(&staged_file(home))? {
                        self.rehearsed = Some((staged.version, rehearsal));
                        self.checked = None;
                    }
                }
                Ok(Err(Unrehearsed::Machine(error))) => {
                    self.after = Some(Instant::now() + PUT_OFF_FOR);
                    return Err(error.into());
                }
                Ok(Err(Unrehearsed::Release(reason))) => {
                    let Some(staged) = read_json::<Staged>(&staged_file(home))? else {
                        return Ok(());
                    };
                    let mut store = Store::read(home)?;
                    store.refused.insert(staged.version.clone(), reason.clone());
                    store.write(home)?;
                    fs::remove_file(staged_file(home))?;
                    let version = staged.version;
                    let line = format!("priory refuses {version} and will not try it: {reason}");
                    note(
                        db,
                        trace,
                        home,
                        &line,
                        json!({ "to": version, "reason": reason }),
                    )?;
                }
                Err(TryRecvError::Empty) => self.work = Some(Work::Rehearsing(receiver)),
                Err(TryRecvError::Disconnected) => {}
            },
            None => {}
        }
        Ok(())
    }

    fn look(&mut self, upgrade: &Upgrade, home: &Path) -> Result<(), Error> {
        let store = Store::read(home)?;
        let staged = read_json::<Staged>(&staged_file(home))?;
        let (home, channel) = (home.to_path_buf(), upgrade.channel);
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(find(&home, channel, &store, staged));
        });
        self.work = Some(Work::Looking(receiver));
        Ok(())
    }

    fn rehearse(&mut self, home: &Path, staged: &Staged, host: &Host, factory: &Path) {
        let (home, staged, host) = (home.to_path_buf(), staged.clone(), host.clone());
        let factory = factory.to_path_buf();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(rehearse(&home, &staged, &host, &factory));
        });
        self.work = Some(Work::Rehearsing(receiver));
    }
}

impl Store {
    /// Whether `channel` takes `version` over the running one, and no upgrade to it failed.
    fn accepts(&self, channel: Channel, version: &str) -> bool {
        newer(channel, CURRENT, version) && !self.refused.contains_key(version)
    }
}

/// Whether `channel` takes `to` over `from`. Both are plain `X.Y.Z`: a pre-release never parses.
fn newer(channel: Channel, from: &str, to: &str) -> bool {
    let (Some(from), Some(to)) = (parts(from), parts(to)) else {
        return false;
    };
    let same = match channel {
        Channel::Patch => from[..2] == to[..2],
        Channel::Minor => from[0] == to[0],
    };
    same && to > from
}

fn parts(version: &str) -> Option<[u64; 3]> {
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| {
            let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
            part.parse().ok().filter(|_| digits)
        })
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

/// The newest release `channel` takes, from GitHub's list.
fn newest(releases: &[u8], channel: Channel, store: &Store) -> Result<Option<String>, String> {
    let releases: Vec<Release> = serde_json::from_slice(releases)
        .map_err(|error| format!("GitHub's releases do not parse: {error}"))?;
    Ok(releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| release.tag_name.strip_prefix('v'))
        .filter(|version| store.accepts(channel, version))
        .max_by_key(|version| parts(version))
        .map(str::to_owned))
}

/// How the error of a release that fails its checks starts.
const UNVERIFIED: &str = "priory refuses a release it cannot verify";

/// Looks for a release, downloads it and checks it. `None` without one newer than `staged`.
fn find(
    home: &Path,
    channel: Channel,
    store: &Store,
    staged: Option<Staged>,
) -> Result<Option<Staged>, String> {
    let Some(target) = target() else {
        return Ok(None);
    };
    let dir = journal::dir(home);
    fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let api = format!("https://api.github.com/repos/{REPOSITORY}");
    let answer = dir.join("answer");
    let get = |url: &str, most: u64| -> Result<Vec<u8>, String> {
        download(url, &answer, most)?;
        fs::read(&answer).map_err(|error| error.to_string())
    };
    let releases = get(&format!("{api}/releases?per_page=100"), ANSWER_MAX)?;
    let Some(version) = newest(&releases, channel, store)? else {
        return Ok(None);
    };
    if staged.is_some_and(|staged| staged.version == version) {
        return Ok(None);
    }
    let name = format!("priory-{target}");
    let asset = format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{name}");
    let binary = dir.join("download");
    download(&asset, &binary, BINARY_MAX)?;
    let sha256 = swap::digest(&binary)?;
    let refused = |reason: &str| format!("{UNVERIFIED}, v{version}: {reason}");
    let sum = get(&format!("{asset}.sha256"), ANSWER_MAX)?;
    if String::from_utf8_lossy(&sum).split_whitespace().next() != Some(&sha256) {
        return Err(refused("its checksum differs from its .sha256"));
    }
    let attestations = get(&format!("{api}/attestations/sha256:{sha256}"), ANSWER_MAX)?;
    provenance::verify(&attestations, REPOSITORY, &version, &name, &sha256)
        .map_err(|error| refused(&error))?;
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
        .and_then(|()| fs::rename(&binary, staged_binary(home)))
        .map_err(|error| format!("{}: {error}", binary.display()))?;
    Ok(Some(Staged {
        version,
        sha256,
        since: Timestamp::now(),
    }))
}

/// Downloads `url` to `path` with curl, over HTTPS alone, refusing more than `most` bytes.
fn download(url: &str, path: &Path, most: u64) -> Result<(), String> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
        ])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args([
            "--connect-timeout",
            "10",
            "--max-time",
            "600",
            "--max-filesize",
        ])
        .arg(most.to_string())
        .args(["--user-agent", "priory", "--output"])
        .arg(path)
        .arg(url)
        .output()
        .map_err(|error| format!("curl: {error}"))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("cannot download {url}: {}", error.trim()));
    }
    let size = fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    match size.len() <= most {
        true => Ok(()),
        false => Err(format!("{url} is over {most} bytes")),
    }
}

/// Where this daemon's binary and databases are.
#[derive(Debug, Clone)]
struct Host {
    exe: PathBuf,
    state: PathBuf,
    bus: PathBuf,
}

/// Where this daemon runs from, or why it cannot replace itself.
fn host(upgrade: &Upgrade) -> Result<Host, String> {
    if target().is_none() {
        return Err("no Linux release is built for this machine".to_owned());
    }
    if cfg!(feature = "model") {
        return Err("it is built with the model feature, which releases leave out".to_owned());
    }
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    for unit in &upgrade.units {
        let start = journal::property(unit, "ExecStart")?;
        let path = start
            .split_once("path=")
            .and_then(|(_, rest)| rest.split_once(" ;"))
            .map(|(path, _)| PathBuf::from(path))
            .ok_or_else(|| format!("the user unit {unit} is not installed"))?;
        if path != exe {
            return Err(format!(
                "the unit {unit} runs {}, but this daemon is {}",
                path.display(),
                exe.display()
            ));
        }
    }
    let pid = journal::property(&upgrade.units[0], "MainPID")?;
    if pid != std::process::id().to_string() {
        return Err(format!(
            "this daemon is not the one {} runs",
            upgrade.units[0]
        ));
    }
    let probe = journal::beside(&exe, "probe");
    fs::write(&probe, "")
        .and_then(|()| fs::remove_file(&probe))
        .map_err(|error| format!("{} is not writable: {error}", probe.display()))?;
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| format!("{name} does not name a file by its absolute path"))
    };
    Ok(Host {
        exe,
        state: absolute("PRIORY_DB")?,
        bus: absolute("HERDR_BUS_DB")?,
    })
}

/// What an upgrade must wait for, if anything: a memory grant, a compaction, or a run inside a tool
/// call, or with `none_live` any live run. A run whose transcript does not tell counts as busy.
pub(crate) fn busy(db: &Connection, none_live: bool) -> rusqlite::Result<Option<String>> {
    let count = |query: &str| db.query_row(query, [], |row| row.get::<_, i64>(0));
    if count("SELECT count(*) FROM grants")? > 0 {
        return Ok(Some("a memory grant is held or waits".to_owned()));
    }
    if count("SELECT count(*) FROM idle_compactions WHERE ended IS NULL")? > 0 {
        return Ok(Some("a shared session is compacted".to_owned()));
    }
    let mut statement =
        db.prepare("SELECT id, transcript FROM runs WHERE ended IS NULL ORDER BY id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let (run, path): (i64, Option<String>) = (row.get(0)?, row.get(1)?);
        if none_live {
            return Ok(Some(format!("run {run} is live and the release migrates")));
        }
        let read = path.and_then(|path| fs::read(path).ok());
        match read.and_then(|bytes| transcript::in_tool_call(&bytes)) {
            Some(false) => {}
            Some(true) => return Ok(Some(format!("run {run} is inside a tool call"))),
            None => return Ok(Some(format!("run {run}'s transcript cannot be read"))),
        }
    }
    Ok(None)
}

/// Runs the staged binary on copies of the databases, so a migration that fails, or a factory the
/// release refuses, shows before anything stops.
fn rehearse(
    home: &Path,
    staged: &Staged,
    host: &Host,
    factory: &Path,
) -> Result<Rehearsal, Unrehearsed> {
    let dir = journal::dir(home).join("rehearsal");
    let ran = rehearse_in(&dir, &staged_binary(home), staged, host, factory);
    let _ = fs::remove_dir_all(&dir);
    ran
}

fn rehearse_in(
    dir: &Path,
    binary: &Path,
    staged: &Staged,
    host: &Host,
    factory: &Path,
) -> Result<Rehearsal, Unrehearsed> {
    let machine = Unrehearsed::Machine;
    let release = Unrehearsed::Release;
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|error| machine(format!("{}: {error}", dir.display())))?;
    if swap::digest(binary).map_err(machine)? != staged.sha256 {
        let _ = fs::remove_file(staged_file_of(dir));
        return Err(machine(
            "the staged binary changed after its check".to_owned(),
        ));
    }
    let (state, bus) = (dir.join("state.db"), dir.join("bus.db"));
    swap::backup(&host.state, &state).map_err(machine)?;
    swap::backup(&host.bus, &bus).map_err(machine)?;
    swap::version_is(binary, &staged.version).map_err(release)?;
    swap::run(Command::new(binary).arg("check").arg(factory))
        .map_err(|error| release(format!("priory check failed: {error}")))?;
    let before = (
        schema(&state).map_err(machine)?,
        schema(&bus).map_err(machine)?,
    );
    swap::run(
        Command::new(binary)
            .args(["upgrade", "probe"])
            .arg(factory)
            .env("PRIORY_HOME", dir)
            .env("PRIORY_DB", &state)
            .env("HERDR_BUS_DB", &bus),
    )
    .map_err(|error| release(format!("it cannot open a copy of the databases: {error}")))?;
    let after = (
        schema(&state).map_err(machine)?,
        schema(&bus).map_err(machine)?,
    );
    Ok(Rehearsal {
        schema_from: before.0.0,
        schema_to: after.0.0,
        migrates: before != after,
    })
}

/// `staged.json` beside the rehearsal's folder.
fn staged_file_of(rehearsal: &Path) -> PathBuf {
    rehearsal.with_file_name("staged.json")
}

/// A database's version and every table, index and trigger it holds.
fn schema(path: &Path) -> Result<(i64, Vec<String>), String> {
    let db = swap::read_only(path)?;
    let read = || -> rusqlite::Result<_> {
        let version = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let mut statement =
            db.prepare("SELECT coalesce(sql, name) FROM sqlite_schema ORDER BY type, name")?;
        let objects = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        Ok((version, objects))
    };
    read().map_err(|error| format!("{}: {error}", path.display()))
}

fn plan(
    home: &Path,
    staged: &Staged,
    rehearsal: Rehearsal,
    host: Host,
    upgrade: &Upgrade,
    factory: &Path,
) -> Result<Journal, String> {
    let factory = factory
        .canonicalize()
        .map_err(|error| format!("{}: {error}", factory.display()))?;
    let op = format!("{}-{}", Timestamp::now().as_second(), std::process::id());
    let backup = |path: &Path| journal::beside(path, &format!("upgrade-{CURRENT}-{op}"));
    // A file a person kept under the plain name stays theirs.
    let plain = journal::beside(&host.exe, CURRENT);
    let kept = match plain.exists() {
        true => backup(&host.exe),
        false => plain,
    };
    Ok(Journal {
        phase: Phase::Stopping,
        from: CURRENT.to_owned(),
        to: staged.version.clone(),
        schema_from: rehearsal.schema_from,
        schema_to: rehearsal.schema_to,
        migrates: rehearsal.migrates,
        opened: false,
        staged: staged_binary(home),
        sha256: staged.sha256.clone(),
        kept,
        factory,
        state_backup: backup(&host.state),
        bus_backup: backup(&host.bus),
        exe: host.exe,
        state: host.state,
        bus: host.bus,
        units: upgrade.units.clone(),
        answers_within: ANSWERS_WITHIN,
        watch: WATCH,
        reason: None,
        op,
    })
}

/// Refuses an upgrade whose backups and binaries could fill the disk.
fn room_for(home: &Path, journal: &Journal) -> Result<(), String> {
    let size = |path: &Path| fs::metadata(path).map_or(0, |file| file.len());
    let databases = size(&journal.state) + size(&journal.bus);
    let needed = 2 * (databases + size(&journal.exe) + size(&journal.staged));
    let free = room::measure(home)?.disk_free;
    match free >= needed {
        true => Ok(()),
        false => Err(format!(
            "{free} bytes of disk are free and it needs {needed}"
        )),
    }
}

/// Appends the trace record and the desk line of `outcome`, unless an earlier try did.
fn record(db: &mut Connection, trace: &Trace, outcome: &Outcome) -> Result<(), Error> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let recorded = tx
        .query_row(
            "SELECT 1 FROM trace WHERE json_extract(event, '$.data.op') = ?1",
            [&outcome.op],
            |_| Ok(()),
        )
        .optional()?;
    if recorded.is_none() {
        let mut data = serde_json::to_value(outcome)?;
        if let Some(fields) = data.as_object_mut() {
            fields.remove("kind");
        }
        append(&tx, trace, &outcome.kind, data)?;
        post::report(&tx, "upgrade", &outcome.line())?;
    }
    tx.commit()?;
    Ok(())
}

fn append(
    tx: &rusqlite::Transaction,
    trace: &Trace,
    kind: &str,
    mut data: serde_json::Value,
) -> rusqlite::Result<()> {
    data["factory"] = json!(trace.factory.name);
    data["factory_revision"] = json!(trace.revision);
    data["machine"] = json!(trace.machine);
    if let Some(fields) = data.as_object_mut() {
        fields.retain(|_, value| !value.is_null());
    }
    trace::record(tx, trace, kind, data)
}

/// Records that the daemon refused an upgrade, and says so on the desk, once per `line`.
fn note(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    line: &str,
    mut data: serde_json::Value,
) -> Result<(), Error> {
    if !note_once(db, home, line, line)? {
        return Ok(());
    }
    data["from"] = json!(CURRENT);
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    append(&tx, trace, REFUSED, data)?;
    tx.commit()?;
    Ok(())
}

/// Says `line` on the desk unless `key` already did, and whether it said it.
fn note_once(db: &Connection, home: &Path, key: &str, line: &str) -> Result<bool, Error> {
    let mut store = Store::read(home)?;
    if store.noted.iter().any(|noted| noted == key) {
        return Ok(false);
    }
    store.noted.push(key.to_owned());
    if store.noted.len() > NOTED {
        store.noted.remove(0);
    }
    store.write(home)?;
    post::report(db, "upgrade", line)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state;

    #[test]
    fn a_channel_takes_only_its_own_newer_releases() {
        use Channel::{Minor, Patch};
        for (to, patch, minor) in [
            ("1.3.4", true, true),
            ("1.3.10", true, true),
            ("1.4.0", false, true),
            ("2.0.0", false, false),
            ("1.3.3", false, false),
            ("1.3.2", false, false),
            ("1.4.0-rc1", false, false),
            ("1.4", false, false),
            ("1.+3.9", false, false),
            ("1.3.4.0", false, false),
        ] {
            assert_eq!(newer(Patch, "1.3.3", to), patch, "patch {to}");
            assert_eq!(newer(Minor, "1.3.3", to), minor, "minor {to}");
        }
    }

    #[test]
    fn the_newest_release_leaves_out_drafts_pre_releases_and_refused_versions() {
        let [major, minor, patch] = parts(CURRENT).unwrap();
        let version = |more: u64| format!("{major}.{minor}.{}", patch + more);
        let release = |more: u64, draft: bool, prerelease: bool| {
            json!({
                "tag_name": format!("v{}", version(more)), "draft": draft, "prerelease": prerelease,
            })
        };
        let list = json!([
            release(1, false, false),
            release(2, false, false),
            release(3, false, false),
            release(4, true, false),
            release(5, false, true),
            { "tag_name": "nightly", "draft": false, "prerelease": false },
        ])
        .to_string();
        let mut store = Store::default();
        let newest = |store: &Store| newest(list.as_bytes(), Channel::Patch, store).unwrap();
        assert_eq!(newest(&store), Some(version(3)));
        store.refused.insert(version(3), "it failed".to_owned());
        assert_eq!(newest(&store), Some(version(2)));
    }

    #[test]
    fn any_release_list_parses_or_is_refused() {
        proptest::proptest!(|(text in ".{0,200}")| {
            let _ = newest(text.as_bytes(), Channel::Minor, &Store::default());
        });
    }

    #[test]
    fn a_held_or_waiting_grant_is_not_quiet() {
        let db = state::memory();
        assert_eq!(busy(&db, true).unwrap(), None);
        for state in ["held", "waiting"] {
            db.execute("DELETE FROM grants", []).unwrap();
            db.execute(
                "INSERT INTO grants (bytes, pid, started, command, state) VALUES (1, 1, 1, 'x', ?1)",
                [state],
            )
            .unwrap();
            let busy = busy(&db, false).unwrap();
            assert_eq!(
                busy.as_deref(),
                Some("a memory grant is held or waits"),
                "{state}"
            );
        }
    }

    #[test]
    fn a_live_run_is_quiet_only_between_tool_calls_and_never_for_a_migration() {
        let db = state::memory();
        let path = std::env::temp_dir().join(format!("priory-upgrade-{}", std::process::id()));
        db.execute_batch(
            "INSERT INTO tasks (id, key, workflow, step, state, trigger, source)
               VALUES (1, 'k', 'w', 's', 'running', 't', 's');
             INSERT INTO runs (id, task, step, tmux, session) VALUES (7, 1, 's', 'm', 'n');",
        )
        .unwrap();
        let reason = |migrates: bool| busy(&db, migrates).unwrap();
        assert_eq!(
            reason(false).as_deref(),
            Some("run 7's transcript cannot be read")
        );
        db.execute("UPDATE runs SET transcript = ?1", [path.to_str().unwrap()])
            .unwrap();
        let call =
            r#"{"type":"assistant","message":{"id":"a","content":[{"type":"tool_use","id":"t"}]}}"#;
        fs::write(&path, call).unwrap();
        assert_eq!(
            reason(false).as_deref(),
            Some("run 7 is inside a tool call")
        );
        let result =
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t"}]}}"#;
        fs::write(&path, format!("{call}\n{result}")).unwrap();
        assert_eq!(reason(false), None);
        assert_eq!(
            reason(true).as_deref(),
            Some("run 7 is live and the release migrates")
        );
        fs::remove_file(&path).unwrap();
    }
}
