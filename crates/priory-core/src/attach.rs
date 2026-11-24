//! Runs the factory's `[runs] attach` command once per new run of a step with `attach = true`, so a
//! view host can open the run. The command never holds or fails the run: a failure is a
//! `factory.run.attach_failed` record.

use crate::trace::Trace;
use priory_runtime::script::{self, Script};
use rusqlite::Connection;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// How long the command may run: opening a terminal takes seconds.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A command the daemon started for run `run`, polled each tick.
pub(crate) struct Attaching {
    run: i64,
    script: Script,
}

/// Starts the factory's attach command for run `run` of task `task` in tmux session `tmux`, or
/// records why it could not start. A factory without one runs nothing.
pub(crate) fn start(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    task: i64,
    run: i64,
    tmux: &str,
    running: &mut Vec<Attaching>,
) -> rusqlite::Result<()> {
    let Some(command) = trace.factory.attach() else {
        return Ok(());
    };
    let exe = match priory_runtime::runtime::exe() {
        Ok(exe) => exe,
        Err(reason) => return failed(db, trace, run, &reason, ""),
    };
    let mut attach = Command::new(command);
    attach
        .args([task.to_string(), run.to_string()])
        .env("PRIORY_TASK", task.to_string())
        .env("PRIORY_RUN", run.to_string())
        .env("PRIORY_TMUX", tmux)
        .env("PRIORY_EXE", exe);
    let log = home.join("attach").join(format!("{run}.log"));
    match script::spawn_command(attach, &trace.factory.dir, &log, TIMEOUT) {
        Ok(script) => running.push(Attaching { run, script }),
        Err(reason) => failed(db, trace, run, &reason, "")?,
    }
    Ok(())
}

/// Records each command that ended with a failure since the last tick.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    running: &mut Vec<Attaching>,
) -> rusqlite::Result<()> {
    let mut still = Vec::new();
    for mut attaching in running.drain(..) {
        match attaching.script.poll() {
            None => still.push(attaching),
            Some((status, _)) if status == "exit 0" => {}
            Some((status, output)) => failed(db, trace, attaching.run, &status, &output)?,
        }
    }
    *running = still;
    Ok(())
}

fn failed(
    db: &mut Connection,
    trace: &Trace,
    run: i64,
    reason: &str,
    output: &str,
) -> rusqlite::Result<()> {
    eprintln!("priory daemon: attaching run {run}: {reason}");
    let tx = db.transaction()?;
    crate::trace::attach_failed(&tx, trace, run, reason, output)?;
    tx.commit()
}
