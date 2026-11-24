//! `priory upgrade resume`: the helper that swaps the binary while the services are stopped, watches
//! the new daemon, and puts the old binary back when it fails. It runs from a copy of the old binary.

use crate::provenance::to_hex;
use crate::upgrade::{self, Kept, Outcome, Store};
use priory_runtime::runtime;
use priory_runtime::upgrade::{self as journal, Journal, Phase};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest as _, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How many upgrades keep their old binary and backups.
const KEPT: usize = 3;

/// Takes the upgrade in progress from where it stands to its end. A helper that was killed is
/// started again: past the point where the new binary may be in place, it rolls back.
pub fn resume() -> Result<(), String> {
    let home = runtime::home();
    let Some(_lock) = journal::lock(&home, Duration::from_secs(5))? else {
        return Err("another helper runs this upgrade".to_owned());
    };
    let Some(mut journal) = journal::read(&home)? else {
        return Ok(());
    };
    if matches!(journal.phase, Phase::Installing | Phase::Starting) {
        let reason = "the helper of the upgrade stopped".to_owned();
        to(&home, &mut journal, Phase::RollingBack, Some(reason))?;
    }
    loop {
        match journal.phase {
            Phase::Stopping => {
                let (phase, reason) = match prepare(&journal) {
                    Ok(()) => (Phase::Installing, None),
                    Err(reason) => (Phase::Restarting, Some(reason)),
                };
                to(&home, &mut journal, phase, reason)?;
            }
            Phase::Installing => match install(&journal) {
                Ok(()) => {
                    journal.opened = true;
                    to(&home, &mut journal, Phase::Starting, None)?;
                }
                Err(error) => to(&home, &mut journal, Phase::RollingBack, Some(error))?,
            },
            Phase::Starting => match healthy(&home, &journal) {
                Ok(()) => to(&home, &mut journal, Phase::Committed, None)?,
                Err(error) => to(&home, &mut journal, Phase::RollingBack, Some(error))?,
            },
            Phase::Committed => {
                start(&journal.units)?;
                let answered = answer(&home, &journal).err();
                return finish(&home, &journal, upgrade::INSTALLED, answered);
            }
            Phase::RollingBack => {
                roll_back(&home, &journal)?;
                to(&home, &mut journal, Phase::Restarting, None)?;
            }
            Phase::Restarting => {
                start(&journal.units)?;
                let answered = answer(&home, &journal).err();
                let kind = match Store::read(&home)?.refused.contains_key(&journal.to) {
                    true => upgrade::ROLLED_BACK,
                    false => upgrade::PUT_OFF,
                };
                return finish(&home, &journal, kind, answered);
            }
        }
    }
}

fn to(
    home: &Path,
    journal: &mut Journal,
    phase: Phase,
    reason: Option<String>,
) -> Result<(), String> {
    journal.phase = phase;
    if reason.is_some() {
        journal.reason = reason;
    }
    eprintln!(
        "priory upgrade {} to {}: {phase:?}{}",
        journal.from,
        journal.to,
        journal
            .reason
            .as_ref()
            .map_or(String::new(), |reason| format!(": {reason}"))
    );
    journal::write(home, journal)
}

/// Stops both services and makes the backups. Its error puts the upgrade off: a run or a grant is
/// busy after all, now that nothing new can start, or a backup failed.
fn prepare(journal: &Journal) -> Result<(), String> {
    stop(&journal.units)?;
    let db = read_only(&journal.state)?;
    if let Some(busy) = upgrade::busy(&db, journal.migrates).map_err(|error| error.to_string())? {
        return Err(busy);
    }
    drop(db);
    backup(&journal.state, &journal.state_backup)?;
    backup(&journal.bus, &journal.bus_backup)?;
    journal::copy(&journal.exe, &journal.kept)
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
}

fn install(journal: &Journal) -> Result<(), String> {
    if digest(&journal.staged)? != journal.sha256 {
        return Err("the downloaded binary changed after its check".to_owned());
    }
    journal::copy(&journal.staged, &journal.exe)?;
    let factory = journal.factory.as_path();
    let checked = run(Command::new(&journal.exe).arg("check").arg(factory));
    checked.map_err(|error| format!("priory check failed: {error}"))?;
    version_is(&journal.exe, &journal.to)
}

/// Whether the binary `exe` says it is `version`.
pub(crate) fn version_is(exe: &Path, version: &str) -> Result<(), String> {
    let printed = run(Command::new(exe).arg("--version"))?;
    match printed.trim() == format!("priory {version}") {
        true => Ok(()),
        false => Err(format!("it says it is {}, not {version}", printed.trim())),
    }
}

pub(crate) fn digest(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(to_hex(&Sha256::digest(bytes)))
}

/// Runs `command` within a minute and returns its stdout.
pub(crate) fn run(command: &mut Command) -> Result<String, String> {
    let output = runtime::fed_within(command, None, Duration::from_secs(60))
        .map_err(|error| error.to_string())?
        .ok_or("it did not end within 60 seconds")?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{}: {}", output.status, error.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Starts the new daemon, and the intake unless a migration waits for its commit, and watches that
/// they stay up.
fn healthy(home: &Path, journal: &Journal) -> Result<(), String> {
    let [daemon, intake] = &journal.units;
    start(if journal.migrates {
        std::slice::from_ref(daemon)
    } else {
        &journal.units
    })?;
    let answered = answer(home, journal)?;
    if answered != journal.to {
        return Err(format!("the daemon answers as {answered}"));
    }
    let pid = journal::property(daemon, "MainPID")?;
    let until = Instant::now() + Duration::from_secs(journal.watch);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_secs(1).min(until.duration_since(Instant::now())));
        if journal::property(daemon, "MainPID")? != pid {
            return Err("the daemon stopped after it answered".to_owned());
        }
    }
    if !journal.migrates && journal::property(intake, "ActiveState")? != "active" {
        return Err("the intake did not stay up".to_owned());
    }
    answer(home, journal).map(drop)
}

/// The version the daemon answers with, once it answers.
fn answer(home: &Path, journal: &Journal) -> Result<String, String> {
    let until = Instant::now() + Duration::from_secs(journal.answers_within);
    loop {
        match journal::version(home) {
            Ok(version) => return Ok(version),
            Err(error) if Instant::now() >= until => {
                return Err(format!(
                    "the daemon did not answer within {} seconds: {error}",
                    journal.answers_within
                ));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// Stops the services, refuses the version for good, and puts the old binary back, with the backups
/// when the new one may have migrated the databases.
fn roll_back(home: &Path, journal: &Journal) -> Result<(), String> {
    stop(&journal.units)?;
    let mut store = Store::read(home)?;
    let reason = journal.reason.clone().unwrap_or_default();
    store.refused.insert(journal.to.clone(), reason);
    store.write(home)?;
    journal::copy(&journal.kept, &journal.exe)?;
    if journal.migrates && journal.opened {
        restore(&journal.state_backup, &journal.state)?;
        restore(&journal.bus_backup, &journal.bus)?;
    }
    Ok(())
}

/// Puts `backup` in the place of the database `path`. Its WAL goes first, since SQLite would apply
/// it to the backup.
fn restore(backup: &Path, path: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_owned();
        side.push(suffix);
        match fs::remove_file(&side) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {error}", PathBuf::from(side).display()));
            }
            _ => {}
        }
    }
    journal::copy(backup, path)
}

fn stop(units: &[String]) -> Result<(), String> {
    journal::systemctl(&with("stop", units)).map(drop)
}

fn start(units: &[String]) -> Result<(), String> {
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
    let outcome = Outcome {
        op: journal.op.clone(),
        kind: kind.to_owned(),
        from: journal.from.clone(),
        to: journal.to.clone(),
        schema_from: journal.schema_from,
        schema_to: journal.schema_to,
        reason,
        backups: vec![journal.state_backup.clone(), journal.bus_backup.clone()],
        kept: journal.kept.clone(),
    };
    outcome.write(home)?;
    let mut store = Store::read(home)?;
    let mine = kind != upgrade::PUT_OFF;
    if mine && !store.kept.iter().any(|kept| kept.op == journal.op) {
        store.kept.push(Kept {
            op: journal.op.clone(),
            files: vec![
                journal.kept.clone(),
                journal.state_backup.clone(),
                journal.bus_backup.clone(),
            ],
        });
    }
    // A rollback may still be wanted by hand, so only older upgrades lose their files.
    while store.kept.len() > KEPT {
        for file in store.kept.remove(0).files {
            let _ = fs::remove_file(file);
        }
    }
    store.write(home)?;
    journal::remove(home)?;
    if !mine {
        for file in [&journal.kept, &journal.state_backup, &journal.bus_backup] {
            let _ = fs::remove_file(file);
        }
    }
    if mine {
        let _ = fs::remove_file(&journal.staged);
        let _ = fs::remove_file(upgrade::staged_file(home));
    }
    Ok(())
}
