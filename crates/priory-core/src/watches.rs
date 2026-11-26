//! What this runner's intake would watch, posted on the log so an intake on another machine polls
//! for it.

use herdr_bus::Message;
use jiff::Timestamp;
use priory_intake::intake::WATCH_POSTED;
use rusqlite::Connection;
use serde_json::json;

/// A watched resource, the time the intake reads it from, and its rank.
type Watched = (String, String, u8);

/// The watches this daemon last posted, and the day it posted them.
#[derive(Default)]
pub(crate) struct Posted(Option<(Vec<Watched>, i64)>);

/// Queues this runner's watches as `priory.watch.posted` on the first call after a start, when they
/// changed since, and once a day: an intake forgets a posting 7 days old.
pub(crate) fn post(
    db: &mut Connection,
    runner: &str,
    now: Timestamp,
    posted: &mut Posted,
) -> rusqlite::Result<()> {
    let watches = crate::state::watched(db)?;
    let day = now.as_second().div_euclid(24 * 60 * 60);
    if posted
        .0
        .as_ref()
        .is_some_and(|(last, on)| *last == watches && *on == day)
    {
        return Ok(());
    }
    let listed: Vec<_> = watches
        .iter()
        .map(
            |(resource, since, rank)| json!({ "resource": resource, "since": since, "rank": rank }),
        )
        .collect();
    let data = json!({ "runner": runner, "watches": listed });
    let tx = db.transaction()?;
    let message = Message::new(WATCH_POSTED, data).subject(runner).time(now);
    crate::runners::send(&tx, &message)?;
    tx.commit()?;
    posted.0 = Some((watches, day));
    Ok(())
}
