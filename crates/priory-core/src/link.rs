//! Whether the daemon reaches its log, and when it tries again while it does not.

use std::time::{Duration, Instant};

use priory_intake::log::Unreachable;
use rusqlite::{Connection, OptionalExtension};

type Error = Box<dyn std::error::Error>;

/// The first wait after the log stops answering, doubled after each failed try up to `LONGEST`.
const FIRST: Duration = Duration::from_secs(2);
const LONGEST: Duration = Duration::from_secs(60);

/// The daemon's link to its log: when to try it next.
#[derive(Default)]
pub(crate) struct Link {
    wait: Duration,
    next: Option<Instant>,
}

impl Link {
    /// Whether the log is worth a try now.
    pub(crate) fn due(&self) -> bool {
        self.next.is_none_or(|next| Instant::now() >= next)
    }

    /// Records how a try on the log went. An unreachable log is no error: the daemon goes on with
    /// its own tasks, says so on the desk and tries again later. Any other error stands.
    pub(crate) fn tried(&mut self, db: &Connection, tried: Result<(), Error>) -> Result<(), Error> {
        let error = match tried {
            Ok(()) => {
                *self = Link::default();
                db.execute("DELETE FROM log_link", [])?;
                return Ok(());
            }
            Err(error) => error,
        };
        let Some(Unreachable(detail)) = error.downcast_ref::<Unreachable>() else {
            return Err(error);
        };
        self.wait = (self.wait * 2).clamp(FIRST, LONGEST);
        self.next = Some(Instant::now() + self.wait);
        eprintln!(
            "priory daemon: {error}; trying again in {}s",
            self.wait.as_secs()
        );
        db.execute(
            "INSERT INTO log_link (id, since, error)
             VALUES (1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?1)
             ON CONFLICT (id) DO UPDATE SET error = excluded.error WHERE error != excluded.error",
            [detail],
        )?;
        Ok(())
    }
}

/// `the log is unreachable since <time>: <error>` while the daemon cannot reach its log, with the
/// count of messages that wait to publish.
pub fn banner(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT 'the log is unreachable since ' || since || ': ' || error
           || coalesce('; ' || nullif((SELECT count(*) FROM sends), 0) || ' messages wait to publish', '')
         FROM log_link",
        [],
        |row| row.get(0),
    )
    .optional()
}

/// Refuses to publish a person's verb while the daemon cannot reach the log: this runner's view of
/// owners and asks is as old as its last read.
pub fn refuse_while_unreachable(db: &Connection) -> Result<(), String> {
    let since: Option<String> = db
        .query_row("SELECT since FROM log_link", [], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?;
    match since {
        Some(since) => Err(format!(
            "the log is unreachable since {since}: try again when it is back"
        )),
        None => Ok(()),
    }
}
