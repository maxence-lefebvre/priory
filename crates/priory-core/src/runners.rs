//! Which runner owns each task: claims, holds and leases read from the shared log.

use crate::trace::Trace;
use herdr_bus::Message;
use jiff::{SignedDuration, Timestamp};
use priory_factory::factory::Factory;
use priory_intake::intake::{
    self, ASK_ANSWERED as ANSWERED, ASK_POSTED as POSTED, OUTPUT, RUNNER_ALIVE as ALIVE,
    TASK_CLAIMED as CLAIMED, TASK_HELD as HELD,
};
use priory_intake::log::Log;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
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

/// Handles `message` when ownership decides it, and returns the tmux sessions to kill. `None` leaves
/// it to routing.
pub(crate) fn handle(
    tx: &Transaction,
    message: &Message,
    me: &str,
    trace: &Trace,
    counter: Option<&crate::embed::Counter>,
) -> Result<Option<Vec<String>>, Error> {
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
        POSTED => {
            if text("runner") != me {
                mirror(tx, message)?;
            }
            "noted"
        }
        ANSWERED if text("runner") != me => "elsewhere",
        ANSWERED => {
            // Anyone can publish on the log, so a malformed answer is refused, never fatal.
            let Ok(answer) = serde_json::from_value::<crate::tools::Answer>(message.data.clone())
            else {
                crate::state::record(tx, message, None, "refused", None)?;
                return Ok(Some(Vec::new()));
            };
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
    crate::state::record(tx, message, None, outcome, None)?;
    Ok(Some(Vec::new()))
}

/// Keeps another runner's post of an ask when it is newer than the one held.
fn mirror(tx: &Transaction, message: &Message) -> rusqlite::Result<()> {
    let data = &message.data;
    let (Some(runner), Some(ask), Some(version)) = (
        data["runner"].as_str(),
        data["ask"].as_i64(),
        data["version"].as_i64(),
    ) else {
        return Ok(());
    };
    let mut listed = data["listed"].clone();
    if !listed.is_object() {
        return Ok(());
    }
    listed["kind"] = data["kind"].clone();
    tx.execute(
        "INSERT INTO remote_asks (runner, ask, version, data) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (runner, ask) DO UPDATE SET version = excluded.version, data = excluded.data
         WHERE excluded.version > remote_asks.version",
        params![runner, ask, version, listed.to_string()],
    )?;
    Ok(())
}

/// Queues a post of each ask of this runner whose version moved past the one it posted last.
fn post(tx: &Transaction, me: &str, now: Timestamp) -> rusqlite::Result<()> {
    let moved = tx
        .prepare(
            "SELECT ask_versions.ask, ask_versions.version,
               CASE WHEN asks.task IS NULL THEN 'outside' ELSE asks.kind END
             FROM ask_versions JOIN asks ON asks.id = ask_versions.ask
             WHERE ask_versions.posted IS NULL OR ask_versions.posted < ask_versions.version
             ORDER BY ask_versions.ask",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (ask, version, kind) in moved {
        let Some(listed) = crate::listing::ask(tx, ask)? else {
            continue;
        };
        let data = json!({
            "runner": me, "ask": ask, "version": version, "kind": kind, "listed": listed,
        });
        let id = format!("post/{me}/{ask}/{version}");
        let posted = stamped(POSTED, data, &id)
            .subject(format!("{me}:{ask}"))
            .time(now);
        send(tx, &posted)?;
        tx.execute(
            "UPDATE ask_versions SET posted = ?2 WHERE ask = ?1",
            params![ask, version],
        )?;
    }
    Ok(())
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
) -> Result<Vec<String>, Error> {
    let data = &message.data;
    let (key, runner) = (
        data["key"].as_str().unwrap_or_default(),
        data["runner"].as_str().unwrap_or_default(),
    );
    let given = data["given"] == true;
    let ended = data["ended"] == true;
    let current = owner(tx, key)?;
    let alive = match &current {
        Some(current) => lease(tx, &current.runner)?,
        None => None,
    };
    let request = data["request"].as_str().unwrap_or_default();
    // Every runner records the log in its order, so inbox positions compare alike everywhere.
    let at = tx
        .query_row("SELECT seq FROM inbox WHERE id = ?1", [request], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?;
    let won = current.as_ref().and_then(|current| current.won);
    let fresh = at.is_some_and(|at| won.is_none_or(|won| at > won));
    let claim = Claim {
        from: data["from"].as_str(),
        given,
        fresh,
        time: message.time,
    };
    let held = current.as_ref().map(|current| Owner {
        runner: &current.runner,
        hold: current.hold,
        alive,
    });
    if !trace.factory.runners.contains_key(runner) || !wins(&claim, held.as_ref()) {
        crate::state::record(tx, message, None, "refused", None)?;
        // A copy here whose own claim lost can never run: the log gives its key to another runner.
        let Some(owner) = current.filter(|current| runner == me && current.runner != me) else {
            return Ok(Vec::new());
        };
        let Some(task) = crate::state::keyed(tx, Some(&key.to_owned()))? else {
            return Ok(Vec::new());
        };
        let reason = format!("owned by runner {}", owner.runner);
        return Ok(crate::tools::cancel(tx, task.id, &reason)?
            .into_iter()
            .collect());
    }
    let seq = crate::state::record(tx, message, None, "claimed", None)?;
    tx.execute(
        "INSERT INTO claims (key, runner, request, won, hold) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (key) DO UPDATE SET runner = excluded.runner, request = excluded.request,
           hold = excluded.hold, published = NULL, won = excluded.won",
        params![
            key,
            runner,
            request,
            seq,
            if ended { "ended" } else { "waiting" }
        ],
    )?;
    // The request, then each later one of the key that no task here took, are this task's.
    let mut pending: Vec<String> = tx
        .prepare(
            "SELECT id FROM inbox WHERE subject = ?1 AND id != ?2
               AND outcome IN ('claiming', 'elsewhere')
               AND seq > (SELECT seq FROM inbox WHERE id = ?2)
             ORDER BY seq",
        )?
        .query_map([key, request], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    pending.insert(0, request.to_owned());
    let mut kill = Vec::new();
    // Any copy of the task here stops, so the key runs on one runner only.
    if runner != me
        && let Some(task) = crate::state::keyed(tx, Some(&key.to_owned()))?
    {
        let reason = if given {
            format!("given to runner {runner}")
        } else if claim.from.is_some() {
            format!("moved to runner {runner}: its lease ended")
        } else {
            format!("claimed by runner {runner}")
        };
        kill.extend(crate::tools::cancel(tx, task.id, &reason)?);
    }
    for id in &pending {
        if runner != me {
            tx.execute(
                "UPDATE inbox SET outcome = 'elsewhere' WHERE id = ?1 AND outcome = 'claiming'",
                [id],
            )?;
            continue;
        }
        // A runner that adopts its own task, or is given it back, already has it; an ended one
        // needs none.
        if id == request && (ended || crate::state::keyed(tx, Some(&key.to_owned()))?.is_some()) {
            continue;
        }
        let Some(request) = stored(tx, id)? else {
            continue;
        };
        kill.extend(crate::state::route_in(tx, &request, trace, counter)?);
    }
    if runner == me {
        let task = crate::state::keyed(tx, Some(&key.to_owned()))?.map(|task| task.id);
        tx.execute(
            "UPDATE inbox SET task = ?2 WHERE seq = ?1",
            params![seq, task],
        )?;
    }
    Ok(kill)
}

/// The request `id` as the inbox stored it.
fn stored(tx: &Transaction, id: &str) -> Result<Option<Message>, Error> {
    let row = tx
        .query_row(
            "SELECT type, subject, time, data, links FROM inbox WHERE id = ?1",
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
        // Rounded up, so a lease stored in whole seconds never ends before its full length.
        params![
            runner,
            time.as_second() + i64::from(time.subsec_nanosecond() > 0)
        ],
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
    /// Where in the log the claim that made it the owner sits.
    won: Option<i64>,
}

fn owner(tx: &Transaction, key: &str) -> rusqlite::Result<Option<Owned>> {
    tx.query_row(
        "SELECT runner, hold, won FROM claims WHERE key = ?1",
        [key],
        |row| {
            Ok(Owned {
                runner: row.get(0)?,
                hold: Hold::parse(&row.get::<_, String>(1)?),
                won: row.get(2)?,
            })
        },
    )
    .optional()
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
/// whose hold changed since it last said it. `said` is the minute of the last alive message: the
/// lease itself changes only as the log is read, so every runner judges it alike.
pub(crate) fn tick(
    db: &mut Connection,
    me: &str,
    now: Timestamp,
    said: &mut Option<i64>,
) -> rusqlite::Result<()> {
    adopt(db, me)?;
    let tx = db.transaction()?;
    let minute = now.as_second().div_euclid(60);
    if *said != Some(minute) {
        let alive = stamped(
            ALIVE,
            json!({ "runner": me }),
            &format!("alive/{me}/{minute}"),
        );
        send(&tx, &alive.subject(me).time(now))?;
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
    post(&tx, me, now)?;
    for (key, hold, published) in changed {
        // A task reserved to start stays kept until it starts.
        let hold = match (published.as_deref(), hold) {
            (Some("kept"), Hold::Waiting) => Hold::Kept,
            _ => hold,
        };
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
    tx.commit()?;
    *said = Some(minute);
    Ok(())
}

/// Whether this runner may start the first run of `task` now, or `None` while no claim of its key
/// is on the log yet, as for a task it adopts. A task the log still holds as `waiting` could be
/// taken over, so its runner first says it keeps it and starts it only once it read that back: a
/// takeover published before then wins, and drops the task here.
pub(crate) fn cleared(db: &mut Connection, me: &str, task: i64) -> rusqlite::Result<Option<bool>> {
    let tx = db.transaction()?;
    let claim = tx
        .query_row(
            "SELECT claims.key, claims.hold, claims.published FROM claims
             JOIN tasks ON tasks.key = claims.key WHERE tasks.id = ?1 AND claims.runner = ?2",
            params![task, me],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((key, hold, published)) = claim else {
        return Ok(None);
    };
    if hold != "waiting" {
        return Ok(Some(true));
    }
    if published.as_deref() != Some("kept") {
        let held = json!({ "key": key, "runner": me, "hold": "kept" });
        let now = intake::now().unwrap_or_else(|_| Timestamp::now());
        send(&tx, &Message::new(HELD, held).subject(&key).time(now))?;
        tx.execute(
            "UPDATE claims SET published = 'kept' WHERE key = ?1",
            [&key],
        )?;
    }
    tx.commit()?;
    Ok(Some(false))
}

/// Claims the keys of this runner's active tasks that no runner holds yet, such as the tasks of a
/// factory that had one runner before or one it reopened, so that another runner does not claim
/// them too. Until the claim is read back, admission skips the task. A key whose last task ended
/// before the factory had runners is claimed once as ended, so no runner that reads its request
/// later runs it again.
fn adopt(db: &mut Connection, me: &str) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    let unclaimed = tx
        .prepare(
            "SELECT tasks.key, inbox.id, tasks.generation,
               tasks.state IN ('done', 'failed', 'dropped') FROM tasks
               JOIN inbox ON inbox.task = tasks.id AND inbox.outcome = 'created'
             WHERE CASE WHEN tasks.state IN ('done', 'failed', 'dropped')
               THEN tasks.id = (SELECT max(id) FROM tasks AS later WHERE later.key = tasks.key)
                 AND NOT EXISTS (SELECT 1 FROM claims WHERE claims.key = tasks.key)
               ELSE NOT EXISTS (SELECT 1 FROM claims WHERE claims.key = tasks.key
                 AND claims.runner = ?1) END",
        )?
        .query_map([me], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (key, request, generation, ended) in unclaimed {
        let mut claim = json!({ "key": key, "runner": me, "request": request });
        if ended {
            claim["ended"] = json!(true);
        }
        // A reopened task claims afresh: its first claim's id is on the log already.
        let id = format!("adopt/{request}/{me}/{generation}");
        send(&tx, &stamped(CLAIMED, claim, &id).subject(key))?;
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

/// Records which runner this state file belongs to, or that the factory has one runner only.
pub(crate) fn name(db: &Connection, me: Option<&str>) -> rusqlite::Result<()> {
    match me {
        Some(me) => db.execute(
            "INSERT INTO runner (id, name) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name WHERE name IS NOT excluded.name",
            [me],
        )?,
        None => db.execute("DELETE FROM runner", [])?,
    };
    Ok(())
}

/// The runner this state file belongs to, in a factory of several.
pub(crate) fn me(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT name FROM runner WHERE id = 1", [], |row| row.get(0))
        .optional()
}

/// A mirrored ask as the listings show it: `<runner>:<ask>` as its id, its runner, and when that
/// runner was last seen alive.
/// Its `task` stays the task's id on its own runner.
fn remote_row(
    db: &Connection,
    runner: &str,
    ask: i64,
    mut listed: Value,
) -> rusqlite::Result<Value> {
    listed["id"] = json!(format!("{runner}:{ask}"));
    listed["runner"] = json!(runner);
    listed["last_seen"] = json!(
        db.query_row(
            "SELECT alive FROM leases WHERE runner = ?1",
            [runner],
            |row| { row.get::<_, i64>(0) }
        )
        .optional()?
        .and_then(|second| Timestamp::from_second(second).ok())
        .map(|at| at.to_string())
    );
    Ok(listed)
}

/// Every mirrored ask, open or closed since `since` when given, oldest first.
fn remote_rows(db: &Connection, since: Option<&str>) -> rusqlite::Result<Vec<Value>> {
    let mirrored = db
        .prepare("SELECT runner, ask, data FROM remote_asks ORDER BY runner, ask")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut rows = Vec::new();
    for (runner, ask, data) in mirrored {
        let Ok(listed) = serde_json::from_str::<Value>(&data) else {
            continue;
        };
        let closed = ["answered_at", "withdrawn_at", "since"]
            .into_iter()
            .find_map(|field| listed[field].as_str());
        let shown = listed["state"] == "open"
            || since.is_none_or(|since| closed.is_some_and(|closed| closed >= since));
        if shown {
            rows.push(remote_row(db, &runner, ask, listed)?);
        }
    }
    rows.sort_by(|a, b| a["since"].as_str().cmp(&b["since"].as_str()));
    Ok(rows)
}

/// In a factory of several runners, names this runner on each of `asks` and adds the asks other
/// runners posted, open or closed since `since`.
pub(crate) fn with_remote_asks(
    db: &Connection,
    asks: &mut Value,
    since: Option<&str>,
) -> rusqlite::Result<()> {
    let (Some(me), Some(list)) = (me(db)?, asks.as_array_mut()) else {
        return Ok(());
    };
    for ask in list.iter_mut() {
        ask["runner"] = json!(me);
    }
    list.extend(remote_rows(db, since)?);
    Ok(())
}

/// The desk items of other runners' open asks, ranked as a local ask of the same kind would be.
pub(crate) fn remote_desk_items(db: &Connection) -> rusqlite::Result<Vec<Value>> {
    if me(db)?.is_none() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for ask in remote_rows(db, None)? {
        if ask["state"] != "open" {
            continue;
        }
        let risk = ask["risk"].as_array().cloned().unwrap_or_default();
        let key = ask["key"].as_str();
        let urgency = if risk.contains(&json!("irreversible")) {
            "irreversible"
        } else if key.is_some_and(|key| key.contains(":review:")) {
            "review"
        } else if ask["kind"] == "step" {
            "plan"
        } else {
            "ask"
        };
        let routed = ask["routed_to"]
            .as_str()
            .map(|to| format!("routed to {to}: "))
            .unwrap_or_default();
        let what = format!(
            "{routed}{} {}",
            ask["question"].as_str().unwrap_or_default(),
            ask["options"]
        );
        let title = ask["title"].as_str().map(str::to_owned).unwrap_or_else(|| {
            match (key, ask["step"].as_str()) {
                (Some(key), Some(step)) => format!("task {key} asks at {step}"),
                _ => format!("ask {}", ask["id"].as_str().unwrap_or_default()),
            }
        });
        items.push(json!({
            "since": ask["since"], "kind": "ask", "id": ask["id"], "task": null, "key": key,
            "step": ask["step"], "lane": ask["lane"], "what": what, "routed_to": ask["routed_to"],
            "generation": null,
            "snoozed_until": null, "snoozed_at": null, "back_early": null, "title": title,
            "urgency": urgency, "runner": ask["runner"], "last_seen": ask["last_seen"],
        }));
    }
    Ok(items)
}

/// Publishes a person's answer to ask `ask` of runner `runner`, which applies it when it reads it.
pub fn answer(
    factory: &Factory,
    runner: &str,
    answer: &crate::tools::Answer,
) -> Result<String, String> {
    if !factory.runners.contains_key(runner) {
        return Err(format!("runner {runner} has no runner file in runners/"));
    }
    let mut data = serde_json::to_value(answer).map_err(|error| error.to_string())?;
    data["runner"] = json!(runner);
    publish(
        factory,
        &Message::new(ANSWERED, data).subject(format!("{runner}:{}", answer.ask)),
    )?;
    Ok(format!(
        "answer to ask {} of runner {runner} sent: it applies when {runner} reads the log",
        answer.ask
    ))
}

/// Publishes a claim that gives the task of `key` to `runner`, from the runner that owns it.
pub fn give(factory: &Factory, db: &Connection, key: &str, runner: &str) -> Result<String, String> {
    if !factory.runners.contains_key(runner) {
        return Err(format!("runner {runner} has no runner file in runners/"));
    }
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
    publish(factory, &Message::new(CLAIMED, claim).subject(key))?;
    Ok(format!(
        "{key} given to runner {runner}: {runner} takes it and {from} drops it as each reads the log"
    ))
}

fn publish(factory: &Factory, message: &Message) -> Result<(), String> {
    let log = priory_intake::log::open(factory.log()).map_err(|error| error.to_string())?;
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
    /// No claim of its request won a key before, so it is not a late copy of one that did.
    pub(crate) fresh: bool,
    pub(crate) time: Timestamp,
}

/// Whether `claim` takes its key from `owner`.
pub(crate) fn wins(claim: &Claim, owner: Option<&Owner>) -> bool {
    let Some(from) = claim.from else {
        return claim.fresh && owner.is_none_or(|owner| owner.hold == Hold::Ended);
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

/// `message` at the log's own time when the log gives one, so leases compare one clock rather than
/// each runner's. Only the runners' messages take it: routing reads the others' event times.
pub(crate) fn clocked(mut message: Message, logged: Option<Timestamp>) -> Message {
    if let Some(logged) = logged
        && [ALIVE, CLAIMED, HELD].contains(&message.kind.as_str())
    {
        message.time = logged;
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_runner_message_takes_the_logs_time_and_an_event_keeps_its_own() {
        let alive = Message::new(ALIVE, json!({ "runner": "a" })).time(at(0));
        let request = Message::new("factory.task.requested", json!({})).time(at(0));
        assert_eq!(clocked(alive.clone(), Some(at(5))).time, at(5));
        assert_eq!(clocked(alive, None).time, at(0));
        assert_eq!(clocked(request, Some(at(5))).time, at(0));
    }

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
            fresh: true,
            time: at(minutes),
        }
    }

    #[test]
    fn a_late_claim_of_a_request_that_already_won_loses_even_once_its_task_ended() {
        let late = Claim {
            fresh: false,
            ..claim(None, false, 0)
        };
        assert!(!wins(&late, None));
        assert!(!wins(&late, Some(&owner("a", Hold::Ended, 0))));
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
