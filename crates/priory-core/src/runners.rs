//! Which runner owns each task: claims, holds and leases read from the shared log.

use crate::trace::Trace;
use herdr_bus::Message;
use jiff::{SignedDuration, Timestamp};
use priory_factory::factory::Factory;
use priory_intake::intake::{
    self, ASK_ANSWERED as ANSWERED, OUTPUT, RUNNER_ALIVE as ALIVE, TASK_CLAIMED as CLAIMED,
    TASK_HELD as HELD,
};
use priory_intake::log::Log;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::json;
use uuid::Uuid;

/// How long a runner's lease lasts past its last alive message.
pub(crate) const LEASE: SignedDuration = SignedDuration::from_mins(30);
const IDS: Uuid = Uuid::from_u128(0x7b1f_c2d4_9e3a_4c55_8a10_3f6e_2d9b_41c7);

type Error = Box<dyn std::error::Error>;

/// This machine's runner, when the factory has several, so that they own tasks through the log.
pub(crate) fn runner(factory: &Factory, host: Option<&str>) -> Result<Option<String>, String> {
    if factory.runners.len() < 2 {
        return Ok(None);
    }
    match host {
        Some(host) if factory.runners.contains_key(host) => Ok(Some(host.to_owned())),
        host => Err(format!(
            "this machine, `{}`, has no runner file in runners/, and the factory has several runners",
            host.unwrap_or("?")
        )),
    }
}

/// The consumer a daemon reads the log as: each runner reads all of it.
pub(crate) fn consumer(runner: Option<&str>) -> String {
    runner.map_or_else(|| "priory".to_owned(), |runner| format!("priory.{runner}"))
}

/// Handles `message` when ownership decides it, and returns the tmux session to kill. `None` leaves
/// it to routing.
pub(crate) fn handle(
    tx: &Transaction,
    message: &Message,
    me: &str,
    trace: &Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Option<Option<String>>, Error> {
    let text = |field: &str| message.data[field].as_str().unwrap_or_default();
    let outcome = match message.kind.as_str() {
        ALIVE => {
            renew(tx, text("runner"), message.time)?;
            if text("runner") == me {
                take_over(tx, me, message)?;
            }
            "noted"
        }
        HELD => {
            tx.execute(
                "UPDATE claims SET hold = ?3 WHERE key = ?1 AND runner = ?2",
                params![text("key"), text("runner"), text("hold")],
            )?;
            renew(tx, text("runner"), message.time)?;
            "noted"
        }
        CLAIMED => return claimed(tx, message, me, trace, counter).map(Some),
        ANSWERED if text("runner") != me => "elsewhere",
        ANSWERED => {
            let answer: crate::tools::Answer = serde_json::from_value(message.data.clone())?;
            let head = |worktree: &std::path::Path, branch: &str| {
                priory_runtime::runtime::kept_head(
                    &priory_runtime::runtime::home(),
                    worktree,
                    branch,
                )
            };
            match crate::tools::answer_in_tx(tx, &answer, trace, &head)? {
                Ok(_) => "answered",
                Err(_) => "refused",
            }
        }
        _ if crate::route::requests(message) => match requested(tx, message, me)? {
            Some(outcome) => outcome,
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    record(tx, message, None, outcome)?;
    Ok(Some(None))
}

/// A request for a key no local task has: routed when this runner owns the key, else claimed.
fn requested(
    tx: &Transaction,
    message: &Message,
    me: &str,
) -> rusqlite::Result<Option<&'static str>> {
    let Some(key) = crate::route::resources(message).into_iter().next() else {
        return Ok(None);
    };
    if crate::state::keyed(tx, Some(&key))?.is_some() {
        return Ok(None);
    }
    match owner(tx, &key)? {
        Some(owner) if owner.hold != Hold::Ended && owner.runner == me => Ok(None),
        Some(owner) if owner.hold != Hold::Ended => Ok(Some("elsewhere")),
        _ => {
            let claim = json!({ "key": key, "runner": me, "request": message.id.to_string() });
            let id = format!("claim/{}/{me}", message.id);
            send(tx, &stamped(CLAIMED, claim, &id).subject(key))?;
            Ok(Some("claiming"))
        }
    }
}

/// Applies a claim read from the log: the first that wins a key makes its runner the owner, which
/// creates the task from its request, and the runner it took the key from drops its copy.
fn claimed(
    tx: &Transaction,
    message: &Message,
    me: &str,
    trace: &Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Option<String>, Error> {
    let data = &message.data;
    let (key, runner) = (
        data["key"].as_str().unwrap_or_default(),
        data["runner"].as_str().unwrap_or_default(),
    );
    let given = data["given"] == true;
    let current = owner(tx, key)?;
    let alive = match &current {
        Some(current) => lease(tx, &current.runner)?,
        None => None,
    };
    let claim = Claim {
        from: data["from"].as_str(),
        given,
        time: message.time,
    };
    let held = current.as_ref().map(|current| Owner {
        runner: &current.runner,
        hold: current.hold,
        alive,
    });
    if !wins(&claim, held.as_ref()) {
        record(tx, message, None, "refused")?;
        return Ok(None);
    }
    let request = data["request"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| current.as_ref().map(|current| current.request.clone()))
        .unwrap_or_default();
    tx.execute(
        "INSERT INTO claims (key, runner, request) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET runner = excluded.runner, request = excluded.request,
           hold = 'waiting', published = NULL",
        params![key, runner, request],
    )?;
    let mut kill = None;
    let previous = current.filter(|current| current.hold != Hold::Ended);
    if previous.is_some_and(|previous| previous.runner == me)
        && runner != me
        && let Some(task) = crate::state::keyed(tx, Some(&key.to_owned()))?
    {
        let reason = if given {
            format!("given to runner {runner}")
        } else {
            format!("moved to runner {runner}: its lease ended")
        };
        kill = crate::tools::cancel(tx, task.id, &reason)?;
    }
    if runner == me
        && let Some(request) = stored(tx, &request)?
    {
        tx.execute("DELETE FROM inbox WHERE id = ?1", [request.id.to_string()])?;
        kill = kill.or(crate::state::handle_in(tx, &request, trace, counter)?);
    }
    let task = crate::state::keyed(tx, Some(&key.to_owned()))?.map(|task| task.id);
    record(tx, message, task.filter(|_| runner == me), "claimed")?;
    Ok(kill)
}

/// The request `id` as the inbox stored it, while no task of this runner took it.
fn stored(tx: &Transaction, id: &str) -> Result<Option<Message>, Error> {
    let row = tx
        .query_row(
            "SELECT type, subject, time, data, links FROM inbox
             WHERE id = ?1 AND outcome IN ('claiming', 'elsewhere')",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, subject, time, data, links)) = row else {
        return Ok(None);
    };
    Ok(Some(Message {
        id: id.parse()?,
        kind,
        subject,
        time: time.parse()?,
        data: serde_json::from_str(&data)?,
        links: serde_json::from_str(&links)?,
    }))
}

/// Claims each waiting key of another runner whose lease ended before this runner's `alive`.
fn take_over(tx: &Transaction, me: &str, alive: &Message) -> rusqlite::Result<()> {
    let lapsed_keys = tx
        .prepare(
            "SELECT claims.key, claims.runner, claims.request, leases.alive
             FROM claims LEFT JOIN leases ON leases.runner = claims.runner
             WHERE claims.runner != ?1 AND claims.hold = 'waiting' ORDER BY claims.key",
        )?
        .query_map([me], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (key, from, request, renewed) in lapsed_keys {
        let renewed = renewed.and_then(|second| Timestamp::from_second(second).ok());
        if !lapsed(renewed, alive.time) {
            continue;
        }
        let claim = json!({ "key": key, "runner": me, "request": request, "from": from });
        let id = format!("takeover/{key}/{me}/{}", alive.id);
        send(
            tx,
            &stamped(CLAIMED, claim, &id).subject(key).time(alive.time),
        )?;
    }
    Ok(())
}

fn renew(tx: &Transaction, runner: &str, time: Timestamp) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO leases (runner, alive) VALUES (?1, ?2)
         ON CONFLICT (runner) DO UPDATE SET alive = max(alive, excluded.alive)",
        params![runner, time.as_second()],
    )?;
    Ok(())
}

fn lease(tx: &Transaction, runner: &str) -> rusqlite::Result<Option<Timestamp>> {
    Ok(tx
        .query_row(
            "SELECT alive FROM leases WHERE runner = ?1",
            [runner],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .and_then(|second| Timestamp::from_second(second).ok()))
}

struct Owned {
    runner: String,
    hold: Hold,
    request: String,
}

fn owner(tx: &Transaction, key: &str) -> rusqlite::Result<Option<Owned>> {
    tx.query_row(
        "SELECT runner, hold, request FROM claims WHERE key = ?1",
        [key],
        |row| {
            Ok(Owned {
                runner: row.get(0)?,
                hold: Hold::parse(&row.get::<_, String>(1)?),
                request: row.get(2)?,
            })
        },
    )
    .optional()
}

fn record(
    tx: &Transaction,
    message: &Message,
    task: Option<i64>,
    outcome: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO inbox (id, type, subject, time, data, links, task, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            message.id.to_string(),
            message.kind,
            message.subject,
            message.time.to_string(),
            message.data.to_string(),
            serde_json::to_string(&message.links).expect("links serialize"),
            task,
            outcome
        ],
    )?;
    Ok(())
}

/// A message of `kind` whose id derives from `name`, so a runner that sends it again adds nothing.
fn stamped(kind: &str, data: serde_json::Value, name: &str) -> Message {
    let time = intake::now().unwrap_or_else(|_| Timestamp::now());
    Message::new(kind, data)
        .id(Uuid::new_v5(&IDS, name.as_bytes()))
        .time(time)
}

/// Queues `message` for the log, in the transaction of the change it follows.
fn send(tx: &Transaction, message: &Message) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO sends (message) VALUES (?1)",
        [serde_json::to_string(message).expect("a message serializes")],
    )?;
    Ok(())
}

/// Queues this runner's alive message once a minute, and a held message for each task of its own
/// whose hold changed since it last said it.
pub(crate) fn tick(db: &mut Connection, me: &str, now: Timestamp) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    let minute = now.as_second().div_euclid(60);
    let said = tx
        .query_row(
            "SELECT 1 FROM leases WHERE runner = ?1 AND alive / 60 >= ?2",
            params![me, minute],
            |_| Ok(()),
        )
        .optional()?;
    if said.is_none() {
        let alive = stamped(
            ALIVE,
            json!({ "runner": me }),
            &format!("alive/{me}/{minute}"),
        );
        send(&tx, &alive.subject(me).time(now))?;
        renew(&tx, me, now)?;
    }
    let changed = tx
        .prepare(
            "SELECT claims.key, tasks.state,
               EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id), claims.published
             FROM claims JOIN tasks ON tasks.id =
               (SELECT max(id) FROM tasks WHERE tasks.key = claims.key)
             WHERE claims.runner = ?1",
        )?
        .query_map([me], |row| {
            Ok((
                row.get::<_, String>(0)?,
                hold(&row.get::<_, String>(1)?, row.get(2)?),
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (key, hold, published) in changed {
        if published.as_deref() == Some(hold.as_str()) {
            continue;
        }
        let held = json!({ "key": key, "runner": me, "hold": hold.as_str() });
        send(&tx, &Message::new(HELD, held).subject(&key).time(now))?;
        tx.execute(
            "UPDATE claims SET published = ?2 WHERE key = ?1",
            params![key, hold.as_str()],
        )?;
    }
    tx.commit()
}

/// Publishes the queued messages, oldest first. One published twice after a crash adds nothing,
/// since the log keeps the first message of each id.
pub(crate) fn flush(db: &Connection, log: &dyn Log) -> Result<(), Error> {
    let queued = db
        .prepare("SELECT id, message FROM sends ORDER BY id")?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, message) in queued {
        log.publish(OUTPUT, &serde_json::from_str(&message)?)?;
        db.execute("DELETE FROM sends WHERE id = ?1", [id])?;
    }
    Ok(())
}

/// Publishes a person's answer to ask `ask` of runner `runner`, which applies it when it reads it.
pub fn answer(runner: &str, answer: &crate::tools::Answer) -> Result<String, String> {
    let mut data = serde_json::to_value(answer).map_err(|error| error.to_string())?;
    data["runner"] = json!(runner);
    publish(&Message::new(ANSWERED, data).subject(format!("{runner}:{}", answer.ask)))?;
    Ok(format!(
        "answer to ask {} of runner {runner} sent: it applies when {runner} reads the log",
        answer.ask
    ))
}

/// Publishes a claim that gives the task of `key` to `runner`, from the runner that owns it.
pub fn give(db: &Connection, key: &str, runner: &str) -> Result<String, String> {
    let owned = db
        .query_row(
            "SELECT runner, request, hold FROM claims WHERE key = ?1",
            [key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((from, request, _)) = owned.filter(|(_, _, held)| held != "ended") else {
        return Err(format!("no runner holds {key}"));
    };
    if from == runner {
        return Err(format!("runner {runner} holds {key} already"));
    }
    let claim =
        json!({ "key": key, "runner": runner, "request": request, "from": from, "given": true });
    publish(&Message::new(CLAIMED, claim).subject(key))?;
    Ok(format!(
        "{key} given to runner {runner}: it moves when {from} reads the log"
    ))
}

fn publish(message: &Message) -> Result<(), String> {
    let log = priory_intake::log::open().map_err(|error| error.to_string())?;
    log.declare(&intake::output())
        .map_err(|error| error.to_string())?;
    log.publish(OUTPUT, &message.clone().time(intake::now()?))
        .map(|_| ())
        .map_err(|error| format!("publish on {OUTPUT}: {error}"))
}

/// What another runner may do with a task, as its owner last published it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// Queued or ready, and no run of it ever started: it moves once its runner's lease ends.
    Waiting,
    /// Started or asleep: it stays with its owner until a person gives it away.
    Kept,
    /// Done, failed or dropped: a new request for its key claims it again.
    Ended,
}

impl Hold {
    fn as_str(self) -> &'static str {
        match self {
            Hold::Waiting => "waiting",
            Hold::Kept => "kept",
            Hold::Ended => "ended",
        }
    }

    fn parse(text: &str) -> Hold {
        match text {
            "waiting" => Hold::Waiting,
            "ended" => Hold::Ended,
            _ => Hold::Kept,
        }
    }
}

/// The hold of a task in `state`, which `started` a run or not.
pub(crate) fn hold(state: &str, started: bool) -> Hold {
    match state {
        "done" | "failed" | "dropped" => Hold::Ended,
        "queued" | "ready" if !started => Hold::Waiting,
        _ => Hold::Kept,
    }
}

/// A key's owner, as the log says at a claim's place in it.
pub(crate) struct Owner<'a> {
    pub(crate) runner: &'a str,
    pub(crate) hold: Hold,
    /// The owner's last alive message, if any.
    pub(crate) alive: Option<Timestamp>,
}

/// A claim of a key, read from the log.
pub(crate) struct Claim<'a> {
    /// The owner it takes the key from, for a takeover or a gift.
    pub(crate) from: Option<&'a str>,
    /// A person gave the key, so the hold and the lease do not matter.
    pub(crate) given: bool,
    pub(crate) time: Timestamp,
}

/// Whether `claim` takes its key from `owner`.
pub(crate) fn wins(claim: &Claim, owner: Option<&Owner>) -> bool {
    let Some(from) = claim.from else {
        return owner.is_none_or(|owner| owner.hold == Hold::Ended);
    };
    let Some(owner) = owner.filter(|owner| owner.runner == from) else {
        return false;
    };
    match owner.hold {
        Hold::Ended => false,
        _ if claim.given => true,
        Hold::Waiting => lapsed(owner.alive, claim.time),
        Hold::Kept => false,
    }
}

/// Whether a lease last renewed at `alive` is over at `at`.
pub(crate) fn lapsed(alive: Option<Timestamp>, at: Timestamp) -> bool {
    alive.is_none_or(|alive| at.duration_since(alive) >= LEASE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-11-21T10:00:00Z";

    fn at(minutes: i64) -> Timestamp {
        T0.parse::<Timestamp>().unwrap() + SignedDuration::from_mins(minutes)
    }

    fn owner(runner: &str, hold: Hold, alive: i64) -> Owner<'_> {
        Owner {
            runner,
            hold,
            alive: Some(at(alive)),
        }
    }

    fn claim(from: Option<&str>, given: bool, minutes: i64) -> Claim<'_> {
        Claim {
            from,
            given,
            time: at(minutes),
        }
    }

    #[test]
    fn a_first_claim_takes_an_unowned_key_or_one_whose_task_ended() {
        assert!(wins(&claim(None, false, 0), None));
        assert!(wins(
            &claim(None, false, 0),
            Some(&owner("a", Hold::Ended, 0))
        ));
    }

    #[test]
    fn a_second_claim_of_an_owned_key_loses_whatever_its_hold() {
        for hold in [Hold::Waiting, Hold::Kept] {
            assert!(!wins(&claim(None, false, 90), Some(&owner("a", hold, 0))));
        }
    }

    #[test]
    fn a_takeover_wins_a_waiting_key_only_once_its_owners_lease_is_over() {
        let waiting = owner("a", Hold::Waiting, 0);
        assert!(!wins(&claim(Some("a"), false, 29), Some(&waiting)));
        assert!(wins(&claim(Some("a"), false, 30), Some(&waiting)));
    }

    #[test]
    fn a_takeover_never_wins_a_kept_or_ended_key() {
        for hold in [Hold::Kept, Hold::Ended] {
            assert!(!wins(
                &claim(Some("a"), false, 600),
                Some(&owner("a", hold, 0))
            ));
        }
    }

    #[test]
    fn a_takeover_or_gift_from_a_runner_that_no_longer_owns_the_key_loses() {
        let other = owner("b", Hold::Waiting, 0);
        assert!(!wins(&claim(Some("a"), false, 600), Some(&other)));
        assert!(!wins(&claim(Some("a"), true, 600), Some(&other)));
        assert!(!wins(&claim(Some("a"), true, 0), None));
    }

    #[test]
    fn a_gift_takes_a_kept_key_from_a_live_runner() {
        assert!(wins(
            &claim(Some("a"), true, 1),
            Some(&owner("a", Hold::Kept, 0))
        ));
    }

    #[test]
    fn a_runner_never_seen_alive_holds_no_lease() {
        assert!(lapsed(None, at(0)));
        assert!(!lapsed(Some(at(0)), at(29)));
        assert!(lapsed(Some(at(0)), at(30)));
    }

    #[test]
    fn only_a_task_that_never_started_a_run_waits() {
        assert_eq!(hold("queued", false), Hold::Waiting);
        assert_eq!(hold("ready", false), Hold::Waiting);
        assert_eq!(hold("ready", true), Hold::Kept);
        for state in ["running", "blocked", "held", "asleep"] {
            assert_eq!(hold(state, false), Hold::Kept, "{state}");
        }
        for state in ["done", "failed", "dropped"] {
            assert_eq!(hold(state, true), Hold::Ended, "{state}");
        }
    }
}
