//! The requests a server stores when it takes a webhook delivery for a trigger with a `pre`
//! script, and the thread that runs their scripts after the reply, oldest first.

use crate::{post, state};
use priory_factory::factory::Factory;
use priory_intake::intake::OUTPUT;
use priory_intake::log;
use priory_intake::pre::{self, Pending};
use rusqlite::{Connection, OptionalExtension, params};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

/// How long the queue waits for a new request before it looks at the state again.
const IDLE: Duration = Duration::from_secs(5);

/// Makes this process store each webhook request whose trigger has a `pre` script, and starts the
/// thread that runs them. Requests a stopped server left are run first, even when no trigger has a
/// `pre` script any more.
pub fn start(factory: &Factory) {
    if factory
        .triggers
        .values()
        .all(|trigger| trigger.value.pre.is_none())
        && !stored()
    {
        return;
    }
    let (wake, woken) = channel::<()>();
    pre::configure(pre::Context {
        dir: factory.dir.clone(),
        bands: factory.bands().into_iter().map(str::to_owned).collect(),
        report: Box::new(report),
        store: Some(Box::new(move |pending: &Pending| {
            let db = state::open()?;
            store(&db, pending).map_err(|error| format!("store a pending request: {error}"))?;
            let _ = wake.send(());
            Ok(())
        })),
    });
    let triggers = factory
        .triggers
        .iter()
        .map(|(name, trigger)| (name.clone(), trigger.value.clone()))
        .collect();
    let event_log = factory.log().clone();
    std::thread::spawn(move || drain(&triggers, &event_log, &woken));
}

/// Records a `pre` script's outcome on the desk.
fn report(trigger: &str, failure: Option<&str>) {
    let recorded = state::open().and_then(|mut db| post::pre_ran(&mut db, trigger, failure));
    if let Err(error) = recorded {
        eprintln!("priory: report the pre script of trigger {trigger}: {error}");
    }
}

/// Stores `pending` once, under its request's id.
pub fn store(db: &Connection, pending: &Pending) -> rusqlite::Result<()> {
    let text = serde_json::to_string(pending).expect("a pending request serializes");
    db.execute(
        "INSERT OR IGNORE INTO pending_requests (id, pending) VALUES (?1, ?2)",
        params![pending.request.id.to_string(), text],
    )?;
    Ok(())
}

/// Whether the state file holds a request a stopped server left. An error counts as one, so the
/// thread that runs them starts and reports it.
fn stored() -> bool {
    state::open()
        .and_then(|db| {
            db.query_row(
                "SELECT EXISTS (SELECT 1 FROM pending_requests)",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
        })
        .unwrap_or(true)
}

/// The oldest stored request, or why its row does not parse.
fn oldest(db: &Connection) -> rusqlite::Result<Option<(i64, Result<Pending, String>)>> {
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT seq, pending FROM pending_requests ORDER BY seq LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row.map(|(seq, text)| {
        let pending = serde_json::from_str(&text).map_err(|error| error.to_string());
        (seq, pending)
    }))
}

fn drain(
    triggers: &std::collections::BTreeMap<String, priory_factory::factory::Trigger>,
    event_log: &priory_factory::factory::EventLog,
    woken: &Receiver<()>,
) {
    loop {
        if let Err(error) = run_all(triggers, event_log) {
            eprintln!("priory: pending requests: {error}");
        }
        if woken.recv_timeout(IDLE) == Err(RecvTimeoutError::Disconnected) {
            return;
        }
    }
}

/// Runs each stored request's script, publishes what it decided and forgets the request.
fn run_all(
    triggers: &std::collections::BTreeMap<String, priory_factory::factory::Trigger>,
    event_log: &priory_factory::factory::EventLog,
) -> Result<(), String> {
    let db = state::open()?;
    let bus = log::open(event_log).map_err(|error| error.to_string())?;
    while let Some((seq, pending)) = oldest(&db).map_err(|error| error.to_string())? {
        // A row an older Priory wrote, or a broken one, would hold every later request behind it.
        let pending = match pending {
            Ok(pending) => pending,
            Err(error) => {
                post::report(
                    &db,
                    "priory",
                    &format!("dropped a stored webhook request that does not parse: {error}"),
                )?;
                db.execute("DELETE FROM pending_requests WHERE seq = ?1", [seq])
                    .map_err(|error| error.to_string())?;
                continue;
            }
        };
        let decided = match triggers.get(&pending.trigger) {
            Some(trigger) => pre::run_pending(&pending, trigger)?,
            // A trigger removed since: its request as the server built it.
            None => pending.request.clone(),
        };
        bus.publish(OUTPUT, &decided)
            .map_err(|error| format!("publish on {OUTPUT}: {error}"))?;
        db.execute("DELETE FROM pending_requests WHERE seq = ?1", [seq])
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
