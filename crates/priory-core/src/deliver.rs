//! Delivers the answer of an ask from a session Priory did not start, through the factory's
//! `deliver` command. A delivery is a row of `deliveries`, stored with the answer.

use crate::memory;
use priory_factory::factory::Factory;
use priory_runtime::script::{self, Script};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long one delivery may run: `hc send` alone waits up to a minute for a person's draft.
const TIMEOUT: Duration = Duration::from_secs(180);
/// How many times a delivery runs its command before it fails.
const TRIES: i64 = 5;

/// Stores the delivery of ask `ask`'s answer, or of its dismissal when `answer` is none, if the
/// factory delivers answers and the ask is an outside one with a session.
pub(crate) fn queue(
    db: &Connection,
    ask: i64,
    answer: Option<&str>,
    factory: &Factory,
) -> rusqlite::Result<()> {
    if factory.deliver().is_none() {
        return Ok(());
    }
    db.execute(
        "INSERT INTO deliveries (ask, revision, answer) SELECT id, revision, ?2 FROM asks
         WHERE id = ?1 AND kind = 'outside' AND session IS NOT NULL",
        params![ask, answer],
    )?;
    Ok(())
}

/// Runs the command in `"$@"` and writes its exit code to the file `$0`, so a daemon that restarts
/// while it runs still learns how it ended. The arguments never pass through the shell's parser.
const WRAP: &str = r#""$@"; status=$?; echo "$status" > "$0"; exit "$status""#;

/// A delivery whose command runs: the daemon's own child, or one a stopped daemon started.
pub(crate) struct Delivering {
    id: i64,
    pid: u32,
    started: Option<i64>,
    script: Option<Script>,
}

/// The deliveries a stopped daemon left running, for [`tick`] to settle once their command ends.
pub(crate) fn adopt(db: &Connection) -> rusqlite::Result<Vec<Delivering>> {
    db.prepare("SELECT id, pid, pid_started FROM deliveries WHERE state = 'running'")?
        .query_map([], |row| {
            Ok(Delivering {
                id: row.get(0)?,
                pid: row.get::<_, Option<u32>>(1)?.unwrap_or_default(),
                started: row.get(2)?,
                script: None,
            })
        })?
        .collect()
}

/// Settles the deliveries whose command ended, then starts the due ones.
pub(crate) fn tick(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    running: &mut Vec<Delivering>,
) -> rusqlite::Result<()> {
    let mut still = Vec::new();
    for mut delivering in running.drain(..) {
        let ended = match &mut delivering.script {
            Some(script) => script.poll(),
            // A pid the kernel reused has another start time.
            None if delivering.started.is_some()
                && memory::started(delivering.pid) == delivering.started =>
            {
                None
            }
            None => Some(adopted_status(home, delivering.id)),
        };
        match ended {
            Some((status, output)) => settle(db, home, delivering.id, &status, &output)?,
            None => still.push(delivering),
        }
    }
    *running = still;
    let Some(deliver) = factory.deliver() else {
        return Ok(());
    };
    let due: Vec<(i64, String, i64, String, Option<String>)> = db
        .prepare(
            "SELECT deliveries.id, asks.name, deliveries.revision, asks.session, deliveries.answer
             FROM deliveries JOIN asks ON asks.id = deliveries.ask
             WHERE deliveries.state = 'queued'
               AND deliveries.due <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             ORDER BY deliveries.id",
        )?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, name, revision, session, answer) in due {
        db.execute(
            "UPDATE deliveries SET state = 'running', tries = tries + 1 WHERE id = ?1",
            [id],
        )?;
        let mut command = Command::new("sh");
        command
            .args(["-c", WRAP])
            .arg(file(home, id, "status"))
            .arg(&deliver)
            .args([
                name.as_str(),
                &revision.to_string(),
                &session,
                if answer.is_some() { "answered" } else { "dismissed" },
                answer.as_deref().unwrap_or_default(),
            ]);
        match script::spawn_command(command, &factory.dir, &file(home, id, "log"), TIMEOUT) {
            Ok(script) => {
                let (pid, started) = (script.pid(), memory::started(script.pid()));
                db.execute(
                    "UPDATE deliveries SET pid = ?2, pid_started = ?3 WHERE id = ?1",
                    params![id, pid, started],
                )?;
                running.push(Delivering {
                    id,
                    pid,
                    started,
                    script: Some(script),
                });
            }
            Err(error) => settle(db, home, id, &format!("cannot start: {error}"), "")?,
        }
    }
    Ok(())
}

/// How the command of an adopted delivery ended, from its status file and log.
fn adopted_status(home: &Path, id: i64) -> (String, String) {
    let code = std::fs::read_to_string(file(home, id, "status")).ok();
    let status = code.map_or_else(
        || INTERRUPTED.to_owned(),
        |code| format!("exit {}", code.trim()),
    );
    (status, script::collect(&file(home, id, "log")))
}

/// Why a delivery whose command left no exit code failed without another try.
const INTERRUPTED: &str = "a daemon restart interrupted its command, which may have reached the session";

/// Records how delivery `id`'s command ended, from its status and output. A failed try is tried
/// again 1, 2, then 4 and 8 seconds later; the last one fails the delivery. An interrupted one
/// fails at once, since another try could deliver the answer twice.
fn settle(db: &Connection, home: &Path, id: i64, status: &str, output: &str) -> rusqlite::Result<()> {
    let _ = std::fs::remove_file(file(home, id, "status"));
    if status == INTERRUPTED {
        db.execute("UPDATE deliveries SET tries = max(tries, ?2) WHERE id = ?1", params![id, TRIES])?;
    }
    if status == "exit 0" {
        db.execute(
            "UPDATE deliveries SET state = 'delivered', error = NULL,
               ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
            [id],
        )?;
        return Ok(());
    }
    let error = [status, output]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(": ");
    db.execute(
        "UPDATE deliveries SET error = ?2,
           state = CASE WHEN tries >= ?3 THEN 'failed' ELSE 'queued' END,
           ended = CASE WHEN tries >= ?3 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END,
           due = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', (1 << (tries - 1)) || ' seconds')
         WHERE id = ?1",
        params![id, error, TRIES],
    )?;
    Ok(())
}

/// Delivery `id`'s file with `extension`: `log` for its command's output, `status` for its exit code.
fn file(home: &Path, id: i64, extension: &str) -> PathBuf {
    home.join("deliveries").join(format!("{id}.{extension}"))
}
