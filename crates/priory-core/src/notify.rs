//! Desktop notifications: `notify-send` with the title of each new next item on the desk that is an
//! ask or an alarm, and nothing else, as Web Push sends.

use crate::listing;
use rusqlite::Connection;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::process::{Child, Command, Stdio};

/// The longest title a notification carries, in characters, as Web Push's.
const TITLE: usize = 120;

/// What the daemon already told a person, and the `notify-send` runs not yet reaped.
#[derive(Default)]
pub(crate) struct Desktop {
    // shortcut: kept in memory, so a restarted daemon tells the current next item again; store it if that annoys.
    told: HashSet<(String, String)>,
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
    let item = (next["kind"].to_string(), next["id"].to_string());
    if !desktop.told.insert(item) {
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
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if !std::mem::replace(&mut desktop.missing, true) {
                eprintln!(
                    "priory daemon: desktop notifications are off: notify-send is not on PATH"
                );
            }
        }
        Err(error) => eprintln!("priory daemon: notify-send: {error}"),
    }
    Ok(())
}
