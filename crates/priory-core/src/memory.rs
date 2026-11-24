use crate::room::{self, Share};
use crate::state;
use priory_factory::factory::Factory;
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// A message from `priory memory acquire`, which waits for a memory grant by polling.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "memory", rename_all = "lowercase")]
pub enum Gate {
    /// Queues a grant of `bytes` for the process `pid`, inside the run whose session is `session`.
    Acquire {
        bytes: u64,
        pid: u32,
        session: Option<String>,
        command: String,
    },
    Poll {
        grant: i64,
    },
    /// Hands a held grant to the command's process `pid`, so it holds while the command runs even
    /// when `acquire` dies.
    Run {
        grant: i64,
        pid: u32,
    },
    /// Ends a grant. A waiting grant leaves its request time to a retry of its command.
    Release {
        grant: i64,
    },
}

/// Applies `gate` and replies with the grant's id and state: `<id> held` or `<id> waiting`.
pub(crate) fn gate(
    db: &mut Connection,
    gate: &Gate,
    factory: &Factory,
    share: &Share,
    room: impl FnOnce() -> Result<Room, String>,
) -> rusqlite::Result<Result<String, String>> {
    let grant = match gate {
        Gate::Release { grant } => {
            end(db, *grant)?;
            return Ok(Ok(format!("{grant} released")));
        }
        Gate::Run { grant, pid } => {
            let Some(started) = started(*pid) else {
                return Ok(Err(format!("process {pid} is not running")));
            };
            db.execute(
                "UPDATE grants SET pid = ?2, started = ?3 WHERE id = ?1",
                params![grant, pid, started],
            )?;
            return Ok(Ok(format!("{grant} held")));
        }
        Gate::Poll { grant } => *grant,
        Gate::Acquire {
            bytes,
            pid,
            session,
            command,
        } => {
            let Some(started) = started(*pid) else {
                return Ok(Err(format!("process {pid} is not running")));
            };
            let task: Option<i64> = db
                .query_row(
                    "SELECT task FROM runs WHERE session = ?1 AND ended IS NULL",
                    [session],
                    |row| row.get(0),
                )
                .optional()?;
            // An `acquire` that lost the reply asks again, and keeps its grant even once its run
            // ended, which is why `session` finds an ended run here and not for a new grant. A pid alone does not
            // name it: a nested `acquire` takes its parent's pid, and a jail numbers its own.
            let asked: Option<i64> = db
                .query_row(
                    "SELECT id FROM grants WHERE pid = ?1 AND started = ?2 AND bytes = ?3
                       AND command = ?4 AND (task IS ?5
                         OR task = (SELECT task FROM runs WHERE session = ?6 ORDER BY id DESC))",
                    params![pid, started, *bytes as i64, command, task, session],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(grant) = asked {
                return reply(db, grant, factory, share, room());
            }
            let room = match room() {
                Ok(room) => room,
                Err(error) => return Ok(Err(format!("cannot measure room: {error}"))),
            };
            if *bytes as f64 > room::ceiling(&room, share) {
                return Ok(Err(format!(
                    "{:.1} GB never fits: this runner's agents may take {:.1} GB",
                    *bytes as f64 / GB,
                    room::ceiling(&room, share) / GB
                )));
            }
            // A killed waiter that no tick reclaimed yet must leave its lapse before this one takes it.
            reclaim(db)?;
            let tx = db.transaction()?;
            tx.execute(
                "DELETE FROM lapses WHERE lapsed < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)",
                [LAPSE],
            )?;
            tx.execute(
                "INSERT INTO grants (bytes, pid, started, task, command, created)
                 VALUES (?1, ?2, ?3, ?4, ?5, coalesce(
                   (SELECT min(created) FROM lapses WHERE task = ?4 AND command = ?5 AND bytes = ?1),
                   strftime('%Y-%m-%dT%H:%M:%SZ', 'now')))",
                params![*bytes as i64, pid, started, task, command],
            )?;
            let grant = tx.last_insert_rowid();
            tx.execute(
                "DELETE FROM lapses WHERE rowid = (SELECT rowid FROM lapses
                   WHERE task = ?1 AND command = ?2 AND bytes = ?3 ORDER BY created LIMIT 1)",
                params![task, command, *bytes as i64],
            )?;
            tx.commit()?;
            return reply(db, grant, factory, share, Ok(room));
        }
    };
    reply(db, grant, factory, share, room())
}

/// Grants what fits, then replies with `grant`'s state. An unmeasured room grants nothing.
fn reply(
    db: &Connection,
    grant: i64,
    factory: &Factory,
    share: &Share,
    room: Result<Room, String>,
) -> rusqlite::Result<Result<String, String>> {
    reclaim(db)?;
    if let Ok(room) = &room {
        let now: String =
            db.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
                row.get(0)
            })?;
        admit(db, factory, share, room, &now, &MEASURED)?;
    }
    let state: Option<String> = db
        .query_row("SELECT state FROM grants WHERE id = ?1", [grant], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(match (state, room) {
        (Some(state), _) if state == "held" => Ok(format!("{grant} held")),
        (Some(_), Ok(_)) => Ok(format!("{grant} waiting")),
        (Some(_), Err(error)) => Ok(format!("{grant} waiting: cannot measure room: {error}")),
        (None, _) => Err(format!("no memory grant {grant}")),
    })
}

const GB: f64 = (1u64 << 30) as f64;

/// Ends a grant. A task's grant that still waits leaves its request time to a retry of its command.
fn end(db: &Connection, grant: i64) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO lapses (task, command, bytes, created)
         SELECT task, command, bytes, created FROM grants
         WHERE id = ?1 AND state = 'waiting' AND task IS NOT NULL",
        [grant],
    )?;
    tx.execute("DELETE FROM grants WHERE id = ?1", [grant])?;
    tx.commit()
}

/// Removes the grants whose holder process is gone, so a killed holder frees its memory.
pub(crate) fn reclaim(db: &Connection) -> rusqlite::Result<()> {
    let mut statement = db.prepare("SELECT id, pid, started FROM grants")?;
    let grants = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // A pid the kernel reused has another start time.
    for (id, ..) in grants
        .into_iter()
        .filter(|(_, pid, at)| started(*pid) != Some(*at))
    {
        end(db, id)?;
    }
    Ok(())
}

/// When the live, unreaped process `pid` started, in clock ticks since boot, from `/proc/<pid>/stat`.
pub(crate) fn started(pid: u32) -> Option<i64> {
    let stat =
        std::fs::read_to_string(Path::new("/proc").join(pid.to_string()).join("stat")).ok()?;
    // The command name, in parentheses, may hold spaces; the fields after it are state, ppid, ...
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    (fields.first() != Some(&"Z")).then_some(())?;
    fields.get(19)?.parse().ok()
}

/// How long a waiting grant that does not fit holds back the grants behind it. A build it waits for ends
/// within minutes, so it is rarely overtaken, and a grant behind a head that cannot fit still has most
/// of its 1-hour default timeout left to run.
const PATIENCE: &str = "-300 seconds";

/// How long a grant waits before the grants behind it stop overtaking it, so held grants drain
/// towards it.
const STARVING: &str = "-900 seconds";

/// How long after its grant a held grant counts as one that will end. A head waits for no older one,
/// such as a hung test or a dev server's lease.
const RELEASE: &str = "-1800 seconds";

/// How long a timed-out grant's request time waits for a retry of its command.
const LAPSE: &str = "-3600 seconds";

/// What the prediction past `STARVING` measures.
struct Uses<'a> {
    /// The memory of a process and its descendants, except those it already counted in the set.
    tree: &'a dyn Fn(u32, &mut BTreeSet<u32>) -> u64,
    /// The memory a live run uses, by its id, tmux session and harness session.
    run: &'a dyn Fn(i64, &str, &str) -> Option<u64>,
}

const MEASURED: Uses = Uses {
    tree: &priory_runtime::room::tree,
    run: &crate::usage::memory,
};

/// Grants waiting grants in request order while each fits beside the live runs and held grants, at
/// the time `now`. A grant does not count its own run's promise, which would make it wait for that
/// run to end. The first that does not fit holds back the rest, so a large build is never starved.
/// Once it has waited past `PATIENCE`, later grants that fit go first, and it keeps its place ahead
/// of them for the next room. Past `STARVING`, it holds them back again while the
/// held grants within `RELEASE` would make it fit by ending.
fn admit(
    db: &Connection,
    factory: &Factory,
    share: &Share,
    room: &Room,
    now: &str,
    uses: &Uses,
) -> rusqlite::Result<()> {
    let mut statement = db.prepare(
        "SELECT id, bytes, task, created <= strftime('%Y-%m-%dT%H:%M:%SZ', ?1, ?2),
           created <= strftime('%Y-%m-%dT%H:%M:%SZ', ?1, ?3)
         FROM grants WHERE state = 'waiting' ORDER BY created, id",
    )?;
    let waiting = statement
        .query_map(params![now, PATIENCE, STARVING], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, bytes, task, overdue, starving) in waiting {
        let promised = state::promised(db, factory, task)?.memory;
        if !room::fits(room, share, promised, bytes) {
            if !overdue
                || (starving && {
                    let (room, promised) = released(db, factory, room, now, task, uses)?;
                    room::fits(&room, share, promised, bytes)
                })
            {
                break;
            }
            continue;
        }
        db.execute(
            "UPDATE grants SET state = 'held', granted = ?2 WHERE id = ?1",
            params![id, now],
        )?;
    }
    Ok(())
}

/// The room, and the memory promised in it to all but the run of `task`, once the held grants that
/// may still end soon have ended, at the time `now`. It only predicts: no grant is given by it.
fn released(
    db: &Connection,
    factory: &Factory,
    room: &Room,
    now: &str,
    task: Option<i64>,
    uses: &Uses,
) -> rusqlite::Result<(Room, u64)> {
    // A nested grant is the newer one, so its processes count for it and not for its parent's.
    let mut statement = db.prepare(
        "SELECT bytes, pid, task FROM grants
         WHERE state = 'held' AND granted > strftime('%Y-%m-%dT%H:%M:%SZ', ?1, ?2)
         ORDER BY id DESC",
    )?;
    let mut held = statement.query(params![now, RELEASE])?;
    let (mut seen, mut of_tasks) = (BTreeSet::new(), BTreeMap::<i64, u64>::new());
    let (mut returned, mut releasable) = (0u64, 0u64);
    while let Some(row) = held.next()? {
        let bytes = row.get::<_, i64>(0)? as u64;
        let used = (uses.tree)(row.get(1)?, &mut seen);
        releasable += bytes;
        // Copied-on-write pages count once per process, so a tree can sum to more than it holds.
        returned += used.min(bytes);
        if let Some(task) = row.get::<_, Option<i64>>(2)? {
            *of_tasks.entry(task).or_default() += used;
        }
    }
    let mut statement = db.prepare("SELECT id, task FROM runs WHERE ended IS NULL")?;
    let tasks = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<BTreeMap<i64, i64>>>()?;
    // A build runs inside its run's scope, so its end gives the run its unused promise back.
    let promised = state::promised_with(db, factory, task, |run, tmux, session| {
        let used = (uses.run)(run, tmux, session)?;
        let build = tasks.get(&run).and_then(|task| of_tasks.get(task));
        Some(used.saturating_sub(*build.unwrap_or(&0)))
    })?
    .memory
    .saturating_sub(releasable);
    let room = Room {
        memory_free: room.memory_free.saturating_add(returned),
        ..*room
    };
    Ok((room, promised))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    /// 30 GB of memory with 19 GB free, and the default 90 % share, which keeps 3 GB free.
    fn room() -> Room {
        Room {
            memory_free: 19 * GB,
            memory_total: 30 * GB,
            disk_free: 500 * GB,
            disk_total: 1000 * GB,
            load: 0.0,
            cpus: 8,
        }
    }

    fn share() -> Share {
        Share {
            memory: 0.9,
            cpu: 0.9,
        }
    }

    /// The daemon's clock in these tests.
    const NOW: &str = "2026-11-23T12:00:00Z";

    fn admit_now(db: &Connection) {
        admit_using(db, 0);
    }

    /// `admit_now` while each held grant's command uses `gb` GB, and each run none.
    fn admit_using(db: &Connection, gb: u64) {
        let held: u64 = db
            .query_row(
                "SELECT count(*) FROM grants WHERE state = 'held'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap() as u64;
        let mut room = room();
        room.memory_free -= held * gb * GB;
        let uses = Uses {
            tree: &|_, _| gb * GB,
            run: &|_, _, _| Some(0),
        };
        admit(db, &Factory::default(), &share(), &room, NOW, &uses).unwrap();
    }

    /// Queues a waiting grant of `gb` GB requested `ago` seconds before `NOW`.
    fn waiting(db: &Connection, gb: u64, ago: u64) -> i64 {
        db.execute(
            "INSERT INTO grants (bytes, pid, started, command, created)
             VALUES (?1, 1, 1, 'build', strftime('%Y-%m-%dT%H:%M:%SZ', ?2, ?3))",
            params![(gb * GB) as i64, NOW, format!("-{ago} seconds")],
        )
        .unwrap();
        db.last_insert_rowid()
    }

    fn states(db: &Connection) -> Vec<(u64, String)> {
        let mut statement = db
            .prepare("SELECT bytes, state FROM grants ORDER BY id")
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)? as u64 / GB, row.get(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn held(gb: u64) -> (u64, String) {
        (gb, "held".to_owned())
    }

    fn wait(gb: u64) -> (u64, String) {
        (gb, "waiting".to_owned())
    }

    /// A held grant of `gb` GB, which leaves the room 19 - `gb` GB for the rest.
    fn hold(db: &Connection, gb: u64) {
        hold_since(db, gb, 0);
    }

    /// `hold`, granted `ago` seconds before `NOW`.
    fn hold_since(db: &Connection, gb: u64, ago: u64) {
        db.execute(
            "INSERT INTO grants (bytes, pid, started, command, state, granted)
             VALUES (?1, 1, 1, 'build', 'held', strftime('%Y-%m-%dT%H:%M:%SZ', ?2, ?3))",
            params![(gb * GB) as i64, NOW, format!("-{ago} seconds")],
        )
        .unwrap();
    }

    #[test]
    fn a_head_that_cannot_fit_holds_back_a_smaller_grant_before_the_bound() {
        let db = state::memory();
        hold(&db, 8);
        waiting(&db, 10, 240);
        waiting(&db, 4, 10);

        admit_now(&db);

        assert_eq!(states(&db), [held(8), wait(10), wait(4)]);
    }

    #[test]
    fn a_head_that_cannot_fit_lets_a_smaller_grant_through_after_the_bound_and_keeps_its_place() {
        let db = state::memory();
        hold(&db, 8);
        waiting(&db, 10, 301);
        waiting(&db, 4, 10);
        waiting(&db, 6, 5);

        admit_now(&db);

        // 19 - 8 = 11 GB keeps 3 GB free: 4 GB fits, then 6 GB does not, and it holds back no one.
        assert_eq!(states(&db), [held(8), wait(10), held(4), wait(6)]);
        db.execute("DELETE FROM grants WHERE state = 'held'", [])
            .unwrap();
        hold(&db, 4);

        admit_now(&db);

        // 15 GB is enough for either the head or the 6 GB behind it, and the head goes first.
        assert_eq!(states(&db), [held(10), wait(6), held(4)]);
    }

    #[test]
    fn every_grant_past_the_bound_that_cannot_fit_lets_a_smaller_one_through() {
        let db = state::memory();
        hold(&db, 8);
        waiting(&db, 10, 400);
        waiting(&db, 9, 350);
        waiting(&db, 2, 10);

        admit_now(&db);

        assert_eq!(states(&db), [held(8), wait(10), wait(9), held(2)]);
    }

    #[test]
    fn past_the_second_bound_a_head_that_cannot_fit_holds_back_smaller_grants_while_releases_would_fit_it()
     {
        let db = state::memory();
        hold(&db, 8);
        waiting(&db, 10, 16 * 60);
        waiting(&db, 4, 10);

        admit_now(&db);

        assert_eq!(states(&db), [held(8), wait(10), wait(4)]);
    }

    #[test]
    fn between_the_bounds_a_head_that_cannot_fit_lets_a_smaller_grant_through() {
        let db = state::memory();
        hold(&db, 8);
        waiting(&db, 10, 6 * 60);
        waiting(&db, 4, 10);

        admit_now(&db);

        assert_eq!(states(&db), [held(8), wait(10), held(4)]);
    }

    #[test]
    fn a_starving_head_that_no_release_can_fit_lets_a_smaller_grant_through() {
        let db = state::memory();
        // 19 GB free keeps 3 GB: 17 GB fits under the 27 GB ceiling and never in the room.
        waiting(&db, 17, 16 * 60);
        waiting(&db, 4, 10);
        waiting(&db, 2, 5);

        admit_now(&db);

        assert_eq!(states(&db), [wait(17), held(4), held(2)]);
    }

    #[test]
    fn a_starving_head_that_releases_cannot_fit_lets_a_smaller_grant_through() {
        let db = state::memory();
        hold(&db, 2);
        // 17 GB plus the 3 GB kept is above 19 GB even once the 2 GB holder ends.
        waiting(&db, 17, 16 * 60);
        waiting(&db, 4, 10);

        admit_now(&db);

        assert_eq!(states(&db), [held(2), wait(17), held(4)]);
    }

    #[test]
    fn a_starving_head_stops_holding_back_for_a_holder_that_never_ends() {
        for (ago, small) in [(29 * 60, wait(4)), (30 * 60, held(4))] {
            let db = state::memory();
            hold_since(&db, 8, ago);
            // A retry keeps an old request time, which renews nothing.
            waiting(&db, 10, 3 * 3600);
            waiting(&db, 4, 10);

            admit_now(&db);

            assert_eq!(states(&db), [held(8), wait(10), small]);
        }
    }

    #[test]
    fn a_holder_that_never_ends_leaves_a_head_the_release_of_a_newer_grant() {
        let db = state::memory();
        hold_since(&db, 4, 2 * 3600);
        hold(&db, 6);
        waiting(&db, 10, 16 * 60);
        waiting(&db, 2, 10);

        admit_now(&db);

        assert_eq!(states(&db), [held(4), held(6), wait(10), wait(2)]);
    }

    #[test]
    fn a_starving_head_waits_for_the_memory_its_holders_use() {
        let db = state::memory();
        hold(&db, 4);
        waiting(&db, 14, 50 * 60);
        waiting(&db, 4, 10);

        // 15 GB is free, and the holder's end makes it 19: enough for 14 GB and the 3 GB kept.
        admit_using(&db, 4);

        assert_eq!(states(&db), [held(4), wait(14), wait(4)]);
        db.execute("DELETE FROM grants WHERE state = 'held'", [])
            .unwrap();

        admit_using(&db, 0);

        assert_eq!(states(&db), [held(14), wait(4)]);
    }

    #[test]
    fn a_starving_head_that_its_holders_memory_cannot_fit_lets_a_smaller_grant_through() {
        let db = state::memory();
        hold(&db, 4);
        waiting(&db, 17, 50 * 60);
        waiting(&db, 4, 10);

        admit_using(&db, 4);

        assert_eq!(states(&db), [held(4), wait(17), held(4)]);
    }

    #[test]
    fn a_holder_returns_at_most_its_grant() {
        let db = state::memory();
        hold(&db, 4);
        waiting(&db, 14, 50 * 60);
        waiting(&db, 2, 10);

        // It uses 8 GB of its 4: 11 GB is free and 4 GB counts as returned, so 14 GB does not fit.
        admit_using(&db, 8);

        assert_eq!(states(&db), [held(4), wait(14), held(2)]);
    }

    #[test]
    fn a_holder_past_its_release_returns_no_memory() {
        let db = state::memory();
        hold_since(&db, 4, 30 * 60);
        waiting(&db, 14, 50 * 60);
        waiting(&db, 4, 10);

        admit_using(&db, 4);

        assert_eq!(states(&db), [held(4), wait(14), held(4)]);
    }

    #[test]
    fn a_nested_grants_memory_is_returned_once() {
        let db = state::memory();
        hold(&db, 4);
        hold(&db, 4);
        waiting(&db, 14, 50 * 60);
        waiting(&db, 2, 10);
        let mut room = room();
        room.memory_free = 14 * GB;
        let uses = Uses {
            // Both grants' trees are the inner command's 4 GB.
            tree: &|pid, seen| if seen.insert(pid) { 4 * GB } else { 0 },
            run: &|_, _, _| Some(0),
        };

        admit(&db, &Factory::default(), &share(), &room, NOW, &uses).unwrap();

        // 18 GB once both end: 14 GB and 3 GB kept fit. Counted twice, 16 GB would fit too.
        assert_eq!(states(&db)[3], wait(2));
        db.execute(
            "UPDATE grants SET bytes = ?1 WHERE id = 3",
            [(16 * GB) as i64],
        )
        .unwrap();

        admit(&db, &Factory::default(), &share(), &room, NOW, &uses).unwrap();

        assert_eq!(states(&db)[3], held(2));
    }

    #[test]
    fn a_builds_end_gives_its_run_its_promise_back() {
        for (head, small) in [(10, wait(1)), (14, held(1))] {
            let db = one_run();
            hold(&db, 4);
            db.execute("UPDATE grants SET task = 1", []).unwrap();
            waiting(&db, head, 50 * 60);
            waiting(&db, 1, 10);
            let mut room = room();
            room.memory_free = 14 * GB;
            let uses = Uses {
                tree: &|_, _| 4 * GB,
                // The run's scope holds its agent's 1 GB and the build's 4 GB.
                run: &|_, _, _| Some(5 * GB),
            };

            admit(&db, &Factory::default(), &share(), &room, NOW, &uses).unwrap();

            // 18 GB less the 3 GB kept and the run's unused 3 of 4 GB leaves 12 GB, not 15.
            assert_eq!(states(&db), [held(4), wait(head), small]);
        }
    }

    /// Sends `message` to the gate with the test machine's room.
    fn send(db: &mut Connection, message: Gate) -> String {
        gate(db, &message, &Factory::default(), &share(), || Ok(room()))
            .unwrap()
            .unwrap()
    }

    /// An `acquire` of `gb` GB from this test's process, inside run `s1`.
    fn acquire(gb: u64, command: &str) -> Gate {
        Gate::Acquire {
            bytes: gb * GB,
            pid: std::process::id(),
            session: Some("s1".to_owned()),
            command: command.to_owned(),
        }
    }

    /// A grant of `gb` GB that the test's parent process holds, so `reclaim` keeps it.
    fn parents(db: &Connection, gb: u64, state: &str, created: &str) -> i64 {
        let pid = std::os::unix::process::parent_id();
        db.execute(
            "INSERT INTO grants (bytes, pid, started, command, state, created)
             VALUES (?1, ?2, ?3, 'other', ?4, ?5)",
            params![(gb * GB) as i64, pid, started(pid), state, created],
        )
        .unwrap();
        db.last_insert_rowid()
    }

    fn created(db: &Connection, grant: &str) -> String {
        let id: i64 = grant.split_once(' ').unwrap().0.parse().unwrap();
        db.query_row("SELECT created FROM grants WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap()
    }

    /// A state with task 1 and its live run `s1`.
    fn one_run() -> Connection {
        let db = state::memory();
        db.execute(
            "INSERT INTO tasks
               (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
             VALUES ('cli:1', 'build', 'implement', 'running', 'now', 'cli', 0, 'cli', 'a',
               strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO runs (task, step, agent, tmux, session)
             VALUES (1, 'implement', 'implementer', 'priory-1', 's1')",
            [],
        )
        .unwrap();
        db
    }

    #[test]
    fn an_acquire_sent_again_by_the_same_process_keeps_its_grant() {
        let mut db = one_run();
        parents(&db, 16, "held", "2026-11-23T12:00:00Z");

        let first = send(&mut db, acquire(4, "cargo build"));
        let again = send(&mut db, acquire(4, "cargo build"));

        assert!(first.ends_with(" waiting"), "{first}");
        assert_eq!(again, first);
        assert_eq!(states(&db), [held(16), wait(4)]);
    }

    #[test]
    fn an_acquire_sent_again_after_its_run_ended_keeps_its_grant() {
        let mut db = one_run();

        let first = send(&mut db, acquire(4, "make"));
        db.execute("UPDATE runs SET ended = '2026-11-23T12:00:00Z'", [])
            .unwrap();
        let again = send(&mut db, acquire(4, "make"));

        assert_eq!((first.as_str(), again.as_str()), ("1 held", "1 held"));
        // A new grant of the ended run is not its task's.
        send(&mut db, acquire(4, "other"));
        let task: Option<i64> = db
            .query_row("SELECT task FROM grants WHERE id = 2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(task, None);
    }

    #[test]
    fn two_lapsed_waiters_of_one_command_each_leave_a_retry_its_request_time() {
        let mut db = one_run();
        parents(&db, 16, "held", "2026-11-23T12:00:00Z");
        for at in ["10:30", "10:00"] {
            let grant = parents(&db, 4, "waiting", &format!("2026-11-23T{at}:00Z"));
            db.execute(
                "UPDATE grants SET task = 1, command = 'make' WHERE id = ?1",
                [grant],
            )
            .unwrap();
            send(&mut db, Gate::Release { grant });
        }

        let first = send(&mut db, acquire(4, "make"));
        assert_eq!(created(&db, &first), "2026-11-23T10:00:00Z");
        // Another process retries while the first one waits.
        let pid = std::os::unix::process::parent_id();
        db.execute(
            "UPDATE grants SET pid = ?1, started = ?2 WHERE id = ?3",
            params![pid, started(pid), id(&first)],
        )
        .unwrap();
        let second = send(&mut db, acquire(4, "make"));

        assert_eq!(created(&db, &second), "2026-11-23T10:30:00Z");
    }

    #[test]
    fn a_retry_of_a_lapsed_grant_keeps_its_request_time_and_goes_before_newer_grants() {
        let mut db = one_run();
        let filler = parents(&db, 16, "held", "2026-11-23T12:00:00Z");
        let lapsed = send(&mut db, acquire(4, "cargo build"));
        let id: i64 = lapsed.split_once(' ').unwrap().0.parse().unwrap();
        db.execute(
            "UPDATE grants SET created = '2026-11-23T10:00:00Z' WHERE id = ?1",
            [id],
        )
        .unwrap();
        send(&mut db, Gate::Release { grant: id });
        let other = send(&mut db, acquire(4, "cargo test"));
        assert_ne!(created(&db, &other), "2026-11-23T10:00:00Z");
        send(&mut db, Gate::Release { grant: id + 1 });
        parents(&db, 4, "waiting", "2026-11-23T11:00:00Z");
        // 19 - 12 GB leaves room for one 4 GB grant beside the 3 GB the share keeps.
        db.execute(
            "UPDATE grants SET bytes = ?2 WHERE id = ?1",
            params![filler, (12 * GB) as i64],
        )
        .unwrap();

        let retry = send(&mut db, acquire(4, "cargo build"));

        assert_eq!(retry, format!("{} held", id + 3));
        assert_eq!(created(&db, &retry), "2026-11-23T10:00:00Z");
        assert_eq!(states(&db), [held(12), wait(4), held(4)]);
    }

    fn id(reply: &str) -> i64 {
        reply.split_once(' ').unwrap().0.parse().unwrap()
    }

    #[test]
    fn a_nested_acquire_in_the_same_process_gets_its_own_grant() {
        let mut db = one_run();

        let outer = send(
            &mut db,
            acquire(4, "bash -c 'priory memory acquire -- make'"),
        );
        let command = send(&mut db, acquire(4, "make"));
        let bytes = send(&mut db, acquire(2, "make"));

        assert_eq!(outer, "1 held");
        assert_eq!(command, "2 held");
        assert_eq!(bytes, "3 held");
    }

    #[test]
    fn two_jailed_runs_with_one_pid_get_a_grant_each() {
        let mut db = one_run();
        db.execute_batch(
            "INSERT INTO tasks
               (key, workflow, step, state, band, lane, lane_derived, trigger, source, entered)
             SELECT 'cli:2', workflow, step, state, band, lane, lane_derived, trigger, source, entered
             FROM tasks;
             INSERT INTO runs (task, step, agent, tmux, session)
             VALUES (2, 'implement', 'implementer', 'priory-2', 's2');",
        )
        .unwrap();
        let mut other = acquire(4, "make");
        if let Gate::Acquire { session, .. } = &mut other {
            *session = Some("s2".to_owned());
        }

        assert_eq!(send(&mut db, acquire(4, "make")), "1 held");
        assert_eq!(send(&mut db, other), "2 held");
    }

    /// A waiting grant of task 1 requested at 10:00, beside a 16 GB grant that holds the room.
    fn old_waiter(db: &mut Connection) -> i64 {
        parents(db, 16, "held", "2026-11-23T12:00:00Z");
        let grant = id(&send(db, acquire(4, "make")));
        db.execute(
            "UPDATE grants SET created = '2026-11-23T10:00:00Z' WHERE id = ?1",
            [grant],
        )
        .unwrap();
        grant
    }

    #[test]
    fn a_waiter_reclaimed_before_its_release_leaves_its_request_time_once() {
        let mut db = one_run();
        let grant = old_waiter(&mut db);
        db.execute("UPDATE grants SET started = 0 WHERE id = ?1", [grant])
            .unwrap();

        reclaim(&db).unwrap();
        send(&mut db, Gate::Release { grant });
        let retry = send(&mut db, acquire(4, "make"));

        assert_eq!(created(&db, &retry), "2026-11-23T10:00:00Z");
        db.execute("DELETE FROM grants WHERE id = ?1", [id(&retry)])
            .unwrap();
        let later = send(&mut db, acquire(4, "make"));
        assert_ne!(created(&db, &later), "2026-11-23T10:00:00Z");
    }

    #[test]
    fn a_retry_that_arrives_before_any_reclaim_keeps_a_dead_waiters_request_time() {
        let mut db = one_run();
        let grant = old_waiter(&mut db);
        db.execute("UPDATE grants SET started = 0 WHERE id = ?1", [grant])
            .unwrap();

        let retry = send(&mut db, acquire(4, "make"));

        assert_eq!(created(&db, &retry), "2026-11-23T10:00:00Z");
    }

    #[test]
    fn a_retry_that_asks_for_other_bytes_queues_at_the_back() {
        let mut db = one_run();
        let grant = old_waiter(&mut db);
        send(&mut db, Gate::Release { grant });

        let other = send(&mut db, acquire(5, "make"));

        assert_ne!(created(&db, &other), "2026-11-23T10:00:00Z");
    }

    fn lapses(db: &Connection) -> i64 {
        db.query_row("SELECT count(*) FROM lapses", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_grant_and_its_lapse_change_together_or_not_at_all() {
        let mut db = one_run();
        let grant = old_waiter(&mut db);
        let refused = |db: &mut Connection, change: &str, message: Gate| {
            db.execute_batch(&format!(
                "CREATE TEMP TRIGGER refuse BEFORE {change}
                 BEGIN SELECT raise(ABORT, 'refused'); END;"
            ))
            .unwrap();
            gate(db, &message, &Factory::default(), &share(), || Ok(room())).unwrap_err();
            db.execute_batch("DROP TRIGGER refuse").unwrap();
        };

        refused(&mut db, "DELETE ON main.grants", Gate::Release { grant });
        assert_eq!((states(&db).len(), lapses(&db)), (2, 0));

        send(&mut db, Gate::Release { grant });
        refused(&mut db, "DELETE ON main.lapses", acquire(4, "make"));
        assert_eq!((states(&db).len(), lapses(&db)), (1, 1));
    }
}
