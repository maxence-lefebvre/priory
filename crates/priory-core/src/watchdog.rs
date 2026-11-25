//! Says in the log and on the desk when a tick of the loop takes long, and what it runs.

use priory_runtime::ticks;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A tick that takes over this long is reported.
const OVER: Duration = Duration::from_secs(10);

/// What `<home>/tick.json` keeps for the desk: the last long tick.
#[derive(serde::Serialize, serde::Deserialize)]
struct Noted {
    at: jiff::Timestamp,
    what: String,
}

/// Serializes the watcher and the loop, which write one file.
static NOTING: Mutex<()> = Mutex::new(());

fn noting() -> MutexGuard<'static, ()> {
    NOTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn note(home: &Path, what: String) {
    eprintln!("priory daemon: {what}");
    let file = home.join("tick.json");
    let at = jiff::Timestamp::now();
    let json = serde_json::to_vec(&Noted { at, what }).expect("serializes");
    let staging: PathBuf = file.with_extension("json.partial");
    let written = std::fs::write(&staging, json).and_then(|()| std::fs::rename(&staging, &file));
    if let Err(error) = written {
        eprintln!("priory daemon: recording {}: {error}", file.display());
    }
}

/// Watches the loop from a thread, which still speaks while a tick holds the loop. Each long tick
/// is reported once while it runs.
pub(crate) fn watch(home: &Path) {
    let home = home.to_path_buf();
    std::thread::spawn(move || {
        let mut told: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if let Some((tick, what)) = ticks::stuck(OVER)
                && told != Some(tick)
            {
                told = Some(tick);
                let _noting = noting();
                if ticks::stuck(OVER).is_some_and(|(now, _)| now == tick) {
                    note(&home, what);
                }
            }
        }
    });
}

/// Ends the tick, and reports a long one with what took longest.
pub(crate) fn ended(home: &Path) {
    let ended = ticks::end(OVER);
    if let Some(what) = ended {
        let _noting = noting();
        note(home, what);
    }
}

/// The desk's line on the last long tick, for a day.
pub(crate) fn banner(home: &Path) -> Option<String> {
    let noted: Noted = serde_json::from_slice(&std::fs::read(home.join("tick.json")).ok()?).ok()?;
    let day = jiff::SignedDuration::from_hours(24);
    (noted.at + day > jiff::Timestamp::now()).then(|| {
        let at = noted.at.round(jiff::Unit::Second).unwrap_or(noted.at);
        format!("{} at {at}", noted.what)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desk_shows_the_last_long_tick_for_a_day() {
        let home = std::env::temp_dir().join(format!("priory-tick-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        assert_eq!(banner(&home), None);

        note(
            &home,
            "a tick took 42s: sweep for 40s, git fetch for 30s".to_owned(),
        );
        let shown = banner(&home).unwrap();
        assert!(
            shown.starts_with("a tick took 42s: sweep for 40s, git fetch for 30s at 20"),
            "{shown}"
        );

        let old = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(25);
        let json = format!(r#"{{"at":"{old}","what":"old"}}"#);
        std::fs::write(home.join("tick.json"), json).unwrap();
        assert_eq!(banner(&home), None);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
