//! Desktop notifications: `notify-send` with the title of each new next item on the desk that is an
//! ask or an alarm, and nothing else, as Web Push sends.

use crate::listing;
use rusqlite::{Connection, OptionalExtension};
use std::io::ErrorKind;
use std::process::{Child, Command, Stdio};

/// The longest title a notification carries, in characters, as Web Push's.
const TITLE: usize = 120;

/// The `notify-send` runs not yet reaped. What was told is in the `notified` table.
#[derive(Default)]
pub(crate) struct Desktop {
    running: Vec<Child>,
    missing: bool,
}

/// Runs `notify-send` once for the desk's next item, when it is an ask or an alarm not told yet.
pub(crate) fn tick(db: &Connection, desktop: &mut Desktop) -> rusqlite::Result<()> {
    desktop
        .running
        .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
    let Some(next) = listing::next(db)? else {
        return Ok(());
    };
    if next["kind"] != "ask" && next["why"] != listing::ALARM {
        return Ok(());
    }
    let told = db.execute(
        "INSERT OR IGNORE INTO notified (kind, item) VALUES (?1, ?2)",
        [next["kind"].to_string(), told_as(db, &next)?],
    )?;
    if told == 0 {
        return Ok(());
    }
    let title: String = next["title"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(TITLE)
        .collect();
    // `--` keeps a title that starts with `-` from reading as an option.
    let run = Command::new("notify-send")
        .args(["--", &title])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match run {
        Ok(child) => desktop.running.push(child),
        Err(error) => {
            // A launch that failed told nobody: forget the item so a fixed `notify-send` tells it.
            forget(db, &next)?;
            if error.kind() != ErrorKind::NotFound {
                eprintln!("priory daemon: notify-send: {error}");
            } else if !std::mem::replace(&mut desktop.missing, true) {
                eprintln!(
                    "priory daemon: desktop notifications are off: notify-send is not on PATH"
                );
            }
        }
    }
    Ok(())
}

fn forget(db: &Connection, next: &serde_json::Value) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM notified WHERE kind = ?1 AND item = ?2",
        [next["kind"].to_string(), told_as(db, next)?],
    )?;
    Ok(())
}

/// The `notified` item of `next`: its id and its ask's revision, or the time it came on the desk,
/// so an ask asked again or a task blocked again is told again, as Web Push tells it.
fn told_as(db: &Connection, next: &serde_json::Value) -> rusqlite::Result<String> {
    let id = &next["id"];
    let when = if next["kind"] == "ask" {
        let revision: Option<i64> = match id {
            serde_json::Value::Number(number) => db.query_row(
                "SELECT revision FROM asks WHERE id = ?1 AND name IS NULL",
                [number.as_i64()],
                |row| row.get(0),
            ),
            _ => db.query_row(
                "SELECT revision FROM asks WHERE name = ?1",
                [id.as_str()],
                |row| row.get(0),
            ),
        }
        .optional()?
        .flatten();
        revision.map(|revision| revision.to_string())
    } else {
        next["since"].as_str().map(str::to_owned)
    };
    Ok(format!("{id}@{}", when.unwrap_or_default()))
}
