//! What this runner's intake would watch, posted on the log so an intake on another machine polls
//! for it.

use herdr_bus::Message;
use jiff::Timestamp;
use priory_intake::intake::WATCH_POSTED;
use rusqlite::Connection;
use serde_json::json;
use std::time::{Duration, Instant};

/// How long a changed list waits after the last posting, so a busy runner posts a few times a
/// minute at most.
const SETTLE: Duration = Duration::from_secs(30);
/// How often an unchanged list is posted again: an intake forgets a posting 7 days old.
const AGAIN: Duration = Duration::from_secs(24 * 60 * 60);

/// A watched resource, the time the intake reads it from, and its rank.
type Watched = (String, String, u8);

/// The watches this daemon last posted, and when.
#[derive(Default)]
pub(crate) struct Posted(Option<(Vec<Watched>, Instant)>);

/// Queues this runner's watches as `priory.watch.posted` on the first call after a start, when
/// they changed at least `SETTLE` after the last posting, and every `AGAIN`.
pub(crate) fn post(
    db: &mut Connection,
    runner: &str,
    now: Timestamp,
    posted: &mut Posted,
) -> rusqlite::Result<()> {
    if posted
        .0
        .as_ref()
        .is_some_and(|(_, at)| at.elapsed() < SETTLE)
    {
        return Ok(());
    }
    let watches = crate::state::watched(db)?;
    let last = posted
        .0
        .as_ref()
        .map(|(last, at)| (last.as_slice(), at.elapsed()));
    if !due(last, &watches) {
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
    posted.0 = Some((watches, Instant::now()));
    Ok(())
}

/// Whether `watches` go out, given the last posting and how long ago it was: on start, when they
/// changed `SETTLE` after it, and every `AGAIN`.
fn due(last: Option<(&[Watched], Duration)>, watches: &[Watched]) -> bool {
    last.is_none_or(|(posted, ago)| ago >= SETTLE && (posted != watches || ago >= AGAIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_posts_at_once_and_a_second_pass_within_the_settle_time_posts_nothing() {
        let db = &mut crate::state::memory();
        let mut posted = Posted::default();
        let queued = |db: &Connection| -> i64 {
            db.query_row("SELECT count(*) FROM sends", [], |row| row.get(0))
                .unwrap()
        };

        post(db, "a", Timestamp::now(), &mut posted).unwrap();
        post(db, "a", Timestamp::now(), &mut posted).unwrap();

        assert_eq!(queued(db), 1);
    }

    #[test]
    fn a_change_waits_for_the_settle_time_and_an_unchanged_list_for_a_day() {
        let one = [(
            "github:o/n#1".to_owned(),
            "2026-11-26T10:00:00Z".to_owned(),
            0,
        )];
        let second = Duration::from_secs(1);

        assert!(due(None, &one));
        assert!(!due(Some((&[], SETTLE - second)), &one));
        assert!(due(Some((&[], SETTLE)), &one));
        assert!(!due(Some((&one, AGAIN - second)), &one));
        assert!(due(Some((&one, AGAIN)), &one));
    }
}
