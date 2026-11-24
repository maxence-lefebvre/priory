//! `priory upgrade resume`: the helper that swaps the binary while the services are stopped, watches
//! the new daemon, and puts the old binary back when it fails. It runs from a copy of the old binary.

use crate::provenance::{self, to_hex};
use crate::upgrade::{self, Kept, Outcome, Store};
use priory_runtime::upgrade::{self as journal, Journal, Phase};
use priory_runtime::{room, runtime};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How many upgrades keep their old binary and backups.
const KEPT: usize = 3;
/// How long one command of an upgrade may run.
const LIMIT: Duration = Duration::from_secs(60);
/// Below this much free disk, a failure is the disk's.
const SHORT: u64 = 64 << 20;
/// What the rollback's last records may take.
const RESERVE: usize = 64 << 10;

/// Why an upgrade stopped short.
#[derive(Debug, PartialEq)]
pub(crate) enum Fault {
    /// The release is bad, so it is refused.
    Release(String),
    /// This machine, a person or a moment is in the way, so the upgrade is put off.
    Machine(String),
}

impl Fault {
    pub(crate) fn because(self, what: &str) -> Fault {
        match self {
            Fault::Release(reason) => Fault::Release(format!("{what}: {reason}")),
            Fault::Machine(reason) => Fault::Machine(format!("{what}: {reason}")),
        }
    }
}

impl From<String> for Fault {
    fn from(reason: String) -> Fault {
        Fault::Machine(reason)
    }
}

/// Checks that GitHub's answer `attestations` proves the file of a version, a name and a sha256.
pub type Verify<'a> = &'a dyn Fn(&[u8], &str, &str, &str) -> Result<(), String>;

/// The check of a real upgrade: a release of Priory's own repository.
pub fn genuine(attestations: &[u8], version: &str, name: &str, sha256: &str) -> Result<(), String> {
    provenance::verify(attestations, upgrade::REPOSITORY, version, name, sha256)
}

/// Takes the upgrade in progress from where it stands to its end. A helper that was killed is
/// started again: past the point where the new binary may be in place, it rolls back.
pub fn resume(verify: Verify) -> Result<(), String> {
    let home = runtime::home();
    let Some(_lock) = journal::lock(&home, Duration::from_secs(5))? else {
        return Err("another helper runs this upgrade".to_owned());
    };
    let Some(mut journal) = journal::read(&home)? else {
        return Ok(());
    };
    if matches!(journal.phase, Phase::Installing | Phase::Starting) {
        journal.reason = Some("the helper of the upgrade stopped".to_owned());
        back(&home, &mut journal, Phase::RollingBack);
    }
    loop {
        match journal.phase {
            Phase::Stopping => {
                let ready = prepare(&home, &mut journal, verify)
                    .and_then(|()| Ok(forward(&home, &mut journal, Phase::Installing)?));
                if let Err(fault) = ready {
                    blame(&home, &mut journal, fault);
                    back(&home, &mut journal, Phase::Restarting);
                }
            }
            Phase::Installing => {
                let installed = install(&journal).and_then(|()| {
                    journal.opened = true;
                    Ok(forward(&home, &mut journal, Phase::Starting)?)
                });
                if let Err(fault) = installed {
                    blame(&home, &mut journal, fault);
                    back(&home, &mut journal, Phase::RollingBack);
                }
            }
            Phase::Starting => {
                let healthy = healthy(&home, &mut journal)
                    .and_then(|()| Ok(forward(&home, &mut journal, Phase::Committed)?));
                if let Err(fault) = healthy {
                    blame(&home, &mut journal, fault);
                    back(&home, &mut journal, Phase::RollingBack);
                }
            }
            Phase::Committed => {
                start(&ours(&journal))?;
                let answered = serves(&home, &journal);
                return finish(&home, &journal, upgrade::INSTALLED, answered);
            }
            Phase::RollingBack => {
                roll_back(&home, &journal)?;
                back(&home, &mut journal, Phase::Restarting);
            }
            Phase::Restarting => {
                // Nothing was stopped yet, so nothing is started.
                let answered = match journal.running {
                    None => None,
                    Some(_) => {
                        start(&ours(&journal))?;
                        serves(&home, &journal)
                    }
                };
                let kind = match journal.refuse {
                    true => upgrade::ROLLED_BACK,
                    false => upgrade::PUT_OFF,
                };
                return finish(&home, &journal, kind, answered);
            }
        }
    }
}

fn say(journal: &Journal) {
    eprintln!(
        "priory upgrade {} to {}: {:?}{}",
        journal.from,
        journal.to,
        journal.phase,
        journal
            .reason
            .as_ref()
            .map_or(String::new(), |reason| format!(": {reason}"))
    );
}

/// Goes on to `phase`, which must be on disk first.
fn forward(home: &Path, journal: &mut Journal, phase: Phase) -> Result<(), String> {
    let before = std::mem::replace(&mut journal.phase, phase);
    say(journal);
    journal::write(home, journal).inspect_err(|_| journal.phase = before)
}

/// Goes back to `phase` whether or not the disk takes it: a helper that starts again from an older
/// phase ends the same way.
fn back(home: &Path, journal: &mut Journal, phase: Phase) {
    journal.phase = phase;
    say(journal);
    let written = spare(home, || journal::write(home, journal));
    let Err(error) = written else { return };
    eprintln!("priory upgrade: {error}");
    // The services refuse to start beside a journal that says the upgrade still runs.
    if phase == Phase::Restarting
        && let Err(error) = journal::remove(home)
    {
        eprintln!("priory upgrade: {error}");
    }
}

/// Runs `write`, and once more with the reserve given back to a full disk.
fn spare(home: &Path, write: impl Fn() -> Result<(), String>) -> Result<(), String> {
    write().or_else(|_| {
        let _ = fs::remove_file(reserve(home));
        write()
    })
}

fn reserve(home: &Path) -> PathBuf {
    journal::dir(home).join("reserve")
}

/// Notes why the upgrade ends, and whether the release is refused for it.
fn blame(home: &Path, journal: &mut Journal, fault: Fault) {
    let (reason, release) = match fault {
        Fault::Release(reason) => (reason, true),
        Fault::Machine(reason) => (reason, false),
    };
    let places = [home, &journal.exe, &journal.state, &journal.bus];
    let short = places.into_iter().any(short);
    journal.refuse = release && !short;
    journal.reason = Some(match short {
        true => format!("{reason} (the disk is almost full)"),
        false => reason,
    });
}

/// Whether the disk of `path` is almost full, so a failure there is the disk's.
pub(crate) fn short(path: &Path) -> bool {
    room::free(folder(path)).is_ok_and(|free| free < SHORT)
}

pub(crate) fn folder(path: &Path) -> &Path {
    match path.is_dir() {
        true => path,
        false => path.parent().unwrap_or(Path::new("/")),
    }
}

/// The units that ran before the upgrade, which alone it starts again.
fn ours(journal: &Journal) -> Vec<String> {
    let running = journal.running.unwrap_or([true, true]);
    let units = journal.units.iter().zip(running);
    units
        .filter(|(_, ran)| *ran)
        .map(|(unit, _)| unit.clone())
        .collect()
}

/// Why the daemon does not answer, when it should.
fn serves(home: &Path, journal: &Journal) -> Option<String> {
    let [daemon, _] = journal.running.unwrap_or([true, true]);
    if !daemon {
        return None;
    }
    match answer(home, journal) {
        Err(Fault::Release(error) | Fault::Machine(error)) => Some(error),
        Ok(_) => None,
    }
}

/// Checks the release again and makes what the swap and the rollback need, stops both services, and
/// makes the backups. Its fault puts the upgrade off.
fn prepare(home: &Path, journal: &mut Journal, verify: Verify) -> Result<(), Fault> {
    if journal.running.is_none() {
        ready(journal, verify)?;
        fs::write(reserve(home), vec![0xa5; RESERVE])
            .map_err(|error| format!("{}: {error}", reserve(home).display()))?;
        quiet(journal, &service_pids(journal))?;
        let running = [active(&journal.units[0])?, active(&journal.units[1])?];
        if !running[0] {
            return Err(Fault::Machine(format!("{} is stopped", journal.units[0])));
        }
        journal.running = Some(running);
        // A failed write stopped nothing, so nothing is started for it.
        journal::write(home, journal).inspect_err(|_| journal.running = None)?;
    }
    stop(&journal.units)?;
    quiet(journal, &[])?;
    backup(&journal.state, &journal.state_backup)?;
    backup(&journal.bus, &journal.bus_backup)?;
    Ok(())
}

/// Puts the new binary beside the old one, checked again there, with two more names for the old.
fn ready(journal: &Journal, verify: Verify) -> Result<(), Fault> {
    let next = journal::beside(&journal.exe, "next");
    let at = |error: std::io::Error| format!("{}: {error}", next.display());
    fs::copy(&journal.staged, &next)
        .and_then(|_| fs::File::open(&next)?.sync_all())
        .map_err(at)?;
    let read = |path: &Path| fs::read(path).map_err(|error| format!("{}: {error}", path.display()));
    if digest(&next)? != journal.sha256 {
        drop_staged(journal);
        return Err(Fault::Machine(
            "the downloaded binary changed after its check".to_owned(),
        ));
    }
    let (to, asset, sha256) = (&journal.to, &journal.asset, &journal.sha256);
    if let Err(error) = verify(&read(&journal.attestations)?, to, asset, sha256) {
        drop_staged(journal);
        return Err(Fault::Machine(format!(
            "its attestation does not verify: {error}"
        )));
    }
    let own = digest(&journal.exe)?;
    if let Err(error) = verify(&read(&journal.own)?, &journal.from, asset, &own) {
        // The next look fetches the attestation of the binary that is there now.
        drop_staged(journal);
        return Err(Fault::Machine(format!(
            "the installed binary is not a release: {error}"
        )));
    }
    check(&journal.exe, &next, &journal.factory)?;
    version_is(&next, to)?;
    link(&journal.exe, &journal.kept)?;
    link(&journal.kept, &journal::beside(&journal.exe, "back"))?;
    Ok(())
}

fn drop_staged(journal: &Journal) {
    for file in [&journal.staged, &journal.attestations] {
        let _ = fs::remove_file(file);
    }
    let _ = fs::remove_file(journal.staged.with_file_name("staged.json"));
}

/// Gives `from` the second name `to`, which takes no room. A filesystem without links cannot upgrade.
fn link(from: &Path, to: &Path) -> Result<(), String> {
    if same(from, to) {
        return Ok(());
    }
    let _ = fs::remove_file(to);
    fs::hard_link(from, to).map_err(|error| format!("{}: {error}", to.display()))
}

fn same(one: &Path, other: &Path) -> bool {
    let id = |path: &Path| fs::metadata(path).ok().map(|file| (file.dev(), file.ino()));
    id(one).is_some_and(|found| Some(found) == id(other))
}

/// That no run or grant is busy and, for a migration, that no process but `but` holds a database.
fn quiet(journal: &Journal, but: &[u32]) -> Result<(), Fault> {
    let db = read_only(&journal.state)?;
    let busy = upgrade::busy(&db, journal.migrates()).map_err(|error| error.to_string())?;
    drop(db);
    if let Some(busy) = busy {
        return Err(Fault::Machine(busy));
    }
    if !journal.migrates() {
        return Ok(());
    }
    // A verb in passing is gone in a moment.
    let until = Instant::now() + Duration::from_secs(5);
    while let Some(holder) = holder(&[&journal.state, &journal.bus], but)? {
        if Instant::now() >= until {
            return Err(Fault::Machine(holder));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}

fn service_pids(journal: &Journal) -> Vec<u32> {
    let pid = |unit: &String| journal::property(unit, "MainPID").ok()?.parse().ok();
    journal.units.iter().filter_map(pid).collect()
}

/// A process of this user, other than this one and `but`, with one of `databases` open.
fn holder(databases: &[&Path], but: &[u32]) -> Result<Option<String>, String> {
    let mut open = BTreeMap::new();
    for database in databases {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = database.as_os_str().to_owned();
            path.push(suffix);
            if let Ok(file) = fs::metadata(&path) {
                open.insert((file.dev(), file.ino()), database);
            }
        }
    }
    let processes = fs::read_dir("/proc").map_err(|error| format!("/proc: {error}"))?;
    for process in processes.flatten() {
        let Some(pid) = process.file_name().to_string_lossy().parse::<u32>().ok() else {
            continue;
        };
        if pid == std::process::id() || but.contains(&pid) {
            continue;
        }
        // Another user's process shows no files, and opens none of ours.
        let Ok(files) = fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        for file in files.flatten() {
            let Ok(found) = fs::metadata(file.path()) else {
                continue;
            };
            if let Some(database) = open.get(&(found.dev(), found.ino())) {
                let name = fs::read_to_string(process.path().join("comm")).unwrap_or_default();
                let (name, database) = (name.trim(), database.display());
                return Ok(Some(format!("{name} (pid {pid}) has {database} open")));
            }
        }
    }
    Ok(None)
}

/// Whether `unit` runs or starts. One a person stopped, or that gave up, does neither.
fn active(unit: &str) -> Result<bool, String> {
    let state = journal::property(unit, "ActiveState")?;
    Ok(matches!(&*state, "active" | "activating" | "reloading"))
}

/// Whose fault it is that `unit` ended: one killed from elsewhere, as by the OOM killer, is not the
/// release's.
fn ended(unit: &str, what: &str) -> Result<Fault, String> {
    let killed = journal::property(unit, "ExecMainCode")? == "2"
        && matches!(&*journal::property(unit, "ExecMainStatus")?, "9" | "15");
    Ok(match killed {
        true => Fault::Machine(format!("{what}: it was killed")),
        false => Fault::Release(what.to_owned()),
    })
}

/// Whether `unit` was stopped, as `systemctl stop` does, rather than ended by itself.
fn was_stopped(unit: &str) -> Result<bool, String> {
    Ok(journal::property(unit, "ActiveState")? == "inactive"
        // CLD_KILLED, by SIGTERM: the services handle no signal.
        && journal::property(unit, "ExecMainCode")? == "2"
        && journal::property(unit, "ExecMainStatus")? == "15")
}

pub(crate) fn read_only(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY);
    db.and_then(|db| {
        db.busy_timeout(Duration::from_secs(5))?;
        Ok(db)
    })
    .map_err(|error| format!("{}: {error}", path.display()))
}

/// Copies the database `from` to the new file `to`, with what its WAL holds.
pub(crate) fn backup(from: &Path, to: &Path) -> Result<(), String> {
    let partial = journal::beside(to, "partial");
    let at =
        |error: &dyn std::fmt::Display| format!("{} to {}: {error}", from.display(), to.display());
    if partial.exists() {
        fs::remove_file(&partial).map_err(|error| at(&error))?;
    }
    let copied = || -> Result<(), String> {
        // VACUUM INTO fills an empty file, so the copy is as private as the database from its first byte.
        fs::File::create_new(&partial)
            .and_then(|file| file.set_permissions(fs::metadata(from)?.permissions()))
            .map_err(|error| at(&error))?;
        read_only(from)?
            .execute("VACUUM INTO ?1", [partial.to_string_lossy()])
            .map_err(|error| at(&error))?;
        fs::File::open(&partial)
            .and_then(|file| file.sync_all())
            .and_then(|()| fs::rename(&partial, to))
            .map_err(|error| at(&error))
    };
    copied().inspect_err(|_| drop(fs::remove_file(&partial)))
}

fn install(journal: &Journal) -> Result<(), Fault> {
    let next = journal::beside(&journal.exe, "next");
    journal::rename(&next, &journal.exe)?;
    check(&journal.kept, &journal.exe, &journal.factory)?;
    version_is(&journal.exe, &journal.to)
}

/// Runs `priory check` with the binary `new`. A factory the binary `old` refuses too is no fault of
/// the release.
pub(crate) fn check(old: &Path, new: &Path, factory: &Path) -> Result<(), Fault> {
    let check = |exe: &Path| run(Command::new(exe).arg("check").arg(factory));
    let old = || match check(old) {
        Ok(_) => Ok(()),
        Err(Fault::Release(error) | Fault::Machine(error)) => Err(Fault::Machine(format!(
            "the running version's priory check failed: {error}"
        ))),
    };
    old()?;
    let Err(first) = check(new) else {
        return Ok(());
    };
    if let Fault::Machine(_) = first {
        return Err(first.because("priory check failed"));
    }
    // The factory may have been edited in between: the release fails only beside a check that passes.
    old()?;
    let again = check(new).map(drop);
    again.map_err(|fault| fault.because("priory check failed"))
}

/// Whether the binary `exe` says it is `version`.
pub(crate) fn version_is(exe: &Path, version: &str) -> Result<(), Fault> {
    let printed = run(Command::new(exe).arg("--version"))?;
    match printed.trim() == format!("priory {version}") {
        true => Ok(()),
        false => Err(Fault::Release(format!(
            "it says it is {}, not {version}",
            printed.trim()
        ))),
    }
}

pub(crate) fn digest(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(to_hex(&Sha256::digest(bytes)))
}

/// Runs `command` within a minute and returns its stdout. Only a failure it ended with itself is
/// the release's.
pub(crate) fn run(command: &mut Command) -> Result<String, Fault> {
    // SIGTERM and SIGKILL come from elsewhere, such as the OOM killer.
    let output = runtime::fed_within(command, None, LIMIT)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("it did not end within {} seconds", LIMIT.as_secs()))?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        let error = format!("{}: {}", output.status, error.trim());
        return Err(match output.status.signal() {
            Some(9 | 15) => Fault::Machine(error),
            _ => Fault::Release(error),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Starts the new daemon, and the intake unless a migration waits for its commit, and watches that
/// they stay up.
fn healthy(home: &Path, journal: &mut Journal) -> Result<(), Fault> {
    let [daemon, intake] = journal.units.clone();
    let with_intake = !journal.migrates() && journal.running.unwrap_or([true, true])[1];
    let units = match with_intake {
        true => vec![daemon.clone(), intake.clone()],
        false => vec![daemon.clone()],
    };
    start(&units)?;
    let watched = watch(home, journal, &daemon, with_intake.then_some(&*intake));
    // A service a person stopped meanwhile says nothing about the release, and stays stopped.
    let mut stopped = None;
    for (at, unit) in units.iter().enumerate() {
        if was_stopped(unit)? {
            journal.running.get_or_insert([true, true])[at] = false;
            stopped = Some(Fault::Machine(format!("{unit} was stopped meanwhile")));
        }
    }
    if let Some(stopped) = stopped {
        return Err(stopped);
    }
    watched
}

fn watch(home: &Path, journal: &Journal, daemon: &str, intake: Option<&str>) -> Result<(), Fault> {
    let answered = answer(home, journal)?;
    if answered != journal.to {
        return Err(Fault::Release(format!("the daemon answers as {answered}")));
    }
    let pid = journal::property(daemon, "MainPID")?;
    let intake_pid = intake
        .map(|unit| journal::property(unit, "MainPID"))
        .transpose()?;
    let until = Instant::now() + Duration::from_secs(journal.watch);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_secs(1).min(until.duration_since(Instant::now())));
        if journal::property(daemon, "MainPID")? != pid {
            return Err(ended(daemon, "the daemon stopped after it answered")?);
        }
    }
    if let Some(intake) = intake
        && (journal::property(intake, "ActiveState")? != "active"
            || Some(journal::property(intake, "MainPID")?) != intake_pid)
    {
        return Err(ended(intake, "the intake did not stay up")?);
    }
    answer(home, journal).map(drop)
}

/// The version the daemon answers with, once it answers. A daemon that ends instead is at fault,
/// one that is only slow is not.
fn answer(home: &Path, journal: &Journal) -> Result<String, Fault> {
    let daemon = &journal.units[0];
    let pid = journal::property(daemon, "MainPID")?;
    let until = Instant::now() + Duration::from_secs(journal.answers_within);
    loop {
        let error = match journal::version(home) {
            Ok(version) => return Ok(version),
            Err(error) => error,
        };
        if pid == "0" || journal::property(daemon, "MainPID")? != pid {
            return Err(ended(daemon, "the daemon stopped before it answered")?);
        }
        if Instant::now() >= until {
            return Err(Fault::Machine(format!(
                "the daemon did not answer within {} seconds: {error}",
                journal.answers_within
            )));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Stops the services and puts the old binary back, with the backup of each database the release
/// migrates when it may have opened them. It renames only, so a full disk cannot stop it.
fn roll_back(home: &Path, journal: &Journal) -> Result<(), String> {
    stop(&journal.units)?;
    if journal.refuse {
        refuse(home, journal);
    }
    let held = journal::beside(&journal.exe, "back");
    if !same(&journal.kept, &journal.exe) {
        link(&journal.kept, &held)?;
        journal::rename(&held, &journal.exe)?;
    }
    let _ = fs::remove_file(&held);
    if journal.opened && journal.migrates_state {
        restore(&journal.state_backup, &journal.state)?;
    }
    if journal.opened && journal.migrates_bus {
        restore(&journal.bus_backup, &journal.bus)?;
    }
    Ok(())
}

fn refuse(home: &Path, journal: &Journal) {
    let refused = spare(home, || {
        Store::update(home, |store| {
            let reason = journal.reason.clone().unwrap_or_default();
            store.refused.insert(journal.to.clone(), reason);
        })
    });
    if let Err(error) = refused {
        eprintln!("priory upgrade: {error}");
    }
}

/// Moves `backup` to the place of the database `path`. Its WAL goes first, since SQLite would apply
/// it to the backup. Without a backup, an earlier try already moved it.
fn restore(backup: &Path, path: &Path) -> Result<(), String> {
    if !backup.exists() {
        return Ok(());
    }
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        match fs::remove_file(&side) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(format!("{}: {error}", PathBuf::from(side).display()));
            }
            _ => {}
        }
    }
    journal::rename(backup, path)
}

fn stop(units: &[String]) -> Result<(), String> {
    journal::systemctl(&with("stop", units)).map(drop)
}

fn start(units: &[String]) -> Result<(), String> {
    if units.is_empty() {
        return Ok(());
    }
    // A unit that failed too often refuses to start until this.
    let _ = journal::systemctl(&with("reset-failed", units));
    journal::systemctl(&with("start", units)).map(drop)
}

fn with<'a>(verb: &'a str, units: &'a [String]) -> Vec<&'a str> {
    [verb]
        .into_iter()
        .chain(units.iter().map(String::as_str))
        .collect()
}

/// Leaves the outcome for the daemon to record, drops what older upgrades kept, and ends the journal.
fn finish(
    home: &Path,
    journal: &Journal,
    kind: &str,
    unanswered: Option<String>,
) -> Result<(), String> {
    let mut reason = journal.reason.clone();
    if let Some(error) = unanswered {
        let first = reason.map_or(String::new(), |reason| format!("{reason}; "));
        reason = Some(format!("{first}the daemon does not answer: {error}"));
    }
    let ended = kind != upgrade::PUT_OFF;
    let backups = [&journal.state_backup, &journal.bus_backup];
    // A rollback uses up the backups it restores.
    let left: Vec<PathBuf> = backups
        .into_iter()
        .filter(|backup| ended && backup.exists())
        .cloned()
        .collect();
    let outcome = Outcome {
        op: journal.op.clone(),
        kind: kind.to_owned(),
        from: journal.from.clone(),
        to: journal.to.clone(),
        schema_from: Some(journal.schema_from),
        schema_to: Some(journal.schema_to),
        reason,
        backups: left.clone(),
        kept: ended.then(|| journal.kept.clone()),
    };
    let mut written = spare(home, || outcome.write(home));
    if ended {
        let stored = spare(home, || {
            Store::update(home, |store| {
                store.put_off.remove(&journal.to);
                if !store.kept.iter().any(|kept| kept.op == journal.op) {
                    let mut files = vec![journal.kept.clone()];
                    files.extend(left.iter().cloned());
                    store.kept.push(Kept {
                        op: journal.op.clone(),
                        files,
                    });
                }
                // A rollback may still be wanted by hand, so only older upgrades lose their files.
                while store.kept.len() > KEPT {
                    for file in store.kept.remove(0).files {
                        let _ = fs::remove_file(file);
                    }
                }
            })
        });
        written = written.and(stored);
    }
    // The services run, which counts for more than their record.
    if let Err(error) = written {
        eprintln!("priory upgrade: {error}");
    }
    match journal::remove(home) {
        Err(_) if journal::read(home).is_ok_and(|journal| journal.is_none()) => {}
        removed => removed?,
    }
    for suffix in ["next", "back"] {
        let _ = fs::remove_file(journal::beside(&journal.exe, suffix));
    }
    let _ = fs::remove_file(reserve(home));
    if ended {
        drop_staged(journal);
    } else {
        for file in backups.into_iter().chain([&journal.kept]) {
            let _ = fs::remove_file(file);
        }
    }
    Ok(())
}
