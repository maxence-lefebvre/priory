use crate::room::{self, Share};
use crate::state;
use priory_factory::factory::Factory;
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::time::Duration;

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
        /// How many seconds the command waited, when it gives up at the factory's limit.
        #[serde(default)]
        waited: Option<u64>,
    },
    /// Hands a held grant to the command's process `pid`, so it holds while the command runs even
    /// when `acquire` dies.
    Run { grant: i64, pid: u32 },
    /// Ends a grant. A `lapsed` waiting grant leaves its request time to a retry of its command.
    Release {
        grant: i64,
        #[serde(default)]
        lapsed: bool,
    },
}

/// What comes after the id in the reply to a poll whose wait passed the limit, before the reason.
pub const GAVE_UP: &str = " gave up: ";

/// Applies `gate` and replies with the grant's id and state: `<id> held` or `<id> waiting`, or
/// `GAVE_UP` once a poll that still waits has waited `give_up_after`.
pub(crate) fn gate(
    db: &mut Connection,
    gate: &Gate,
    factory: &Factory,
    share: &Share,
    give_up_after: Option<Duration>,
    room: impl FnOnce() -> Result<Room, String>,
) -> rusqlite::Result<Result<String, String>> {
    match gate {
        Gate::Release { grant, lapsed } => {
            if *lapsed {
                db.execute(
                    "INSERT INTO lapses (task, command, created)
                     SELECT task, command, created FROM grants
                     WHERE id = ?1 AND state = 'waiting' AND task IS NOT NULL",
                    [grant],
                )?;
            }
            db.execute("DELETE FROM grants WHERE id = ?1", [grant])?;
            Ok(Ok(format!("{grant} released")))
        }
        Gate::Run { grant, pid } => {
            let Some(started) = started(*pid) else {
                return Ok(Err(format!("process {pid} is not running")));
            };
            db.execute(
                "UPDATE grants SET pid = ?2, started = ?3 WHERE id = ?1",
                params![grant, pid, started],
            )?;
            Ok(Ok(format!("{grant} held")))
        }
        Gate::Poll { grant, waited } => {
            let reply = reply(db, *grant, factory, share, room())?;
            match (waited, give_up_after, &reply) {
                (Some(waited), Some(limit), Ok(state))
                    if *waited >= limit.as_secs() && !state.ends_with(" held") =>
                {
                    Ok(Ok(format!(
                        "{grant}{GAVE_UP}{}",
                        gave_up(db, *grant, limit)?
                    )))
                }
                _ => Ok(reply),
            }
        }
        Gate::Acquire {
            bytes,
            pid,
            session,
            command,
        } => {
            let Some(started) = started(*pid) else {
                return Ok(Err(format!("process {pid} is not running")));
            };
            // An `acquire` that lost the reply asks again, and keeps its grant.
            let asked: Option<i64> = db
                .query_row(
                    "SELECT id FROM grants WHERE pid = ?1 AND started = ?2",
                    params![pid, started],
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
            db.execute(
                "DELETE FROM lapses WHERE lapsed < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)",
                [LAPSE],
            )?;
            let task: Option<i64> = db
                .query_row(
                    "SELECT task FROM runs WHERE session = ?1 AND ended IS NULL",
                    [session],
                    |row| row.get(0),
                )
                .optional()?;
            db.execute(
                "INSERT INTO grants (bytes, pid, started, task, command, created)
                 VALUES (?1, ?2, ?3, ?4, ?5, coalesce(
                   (SELECT min(created) FROM lapses WHERE task = ?4 AND command = ?5),
                   strftime('%Y-%m-%dT%H:%M:%SZ', 'now')))",
                params![*bytes as i64, pid, started, task, command],
            )?;
            let grant = db.last_insert_rowid();
            db.execute(
                "DELETE FROM lapses WHERE task = ?1 AND command = ?2",
                params![task, command],
            )?;
            reply(db, grant, factory, share, Ok(room))
        }
    }
}

/// Why the waiting `grant` gives up at `limit`, with the held and earlier waiting grants before it.
fn gave_up(db: &Connection, grant: i64, limit: Duration) -> rusqlite::Result<String> {
    let (bytes, ahead): (i64, i64) = db.query_row(
        "SELECT bytes, (SELECT count(*) FROM grants AS other WHERE other.id != grants.id
           AND (other.state = 'held'
             OR (other.created, other.id) < (grants.created, grants.id)))
         FROM grants WHERE id = ?1",
        [grant],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let ahead = match ahead {
        0 => "with no grant ahead".to_owned(),
        1 => "behind 1 grant".to_owned(),
        count => format!("behind {count} grants"),
    };
    Ok(format!(
        "waited {:#} for {} GB {ahead}",
        jiff::SignedDuration::from_secs(limit.as_secs() as i64),
        (bytes as f64 / GB * 10.0).round() / 10.0,
    ))
}

/// How long the grant in the `grants` row has been in its state, such as `1h 51m`, `12m` or `40s`.
macro_rules! grant_age {
    () => {
        "(SELECT CASE WHEN age >= 3600 THEN printf('%dh %dm', age / 3600, age % 3600 / 60)
             WHEN age >= 60 THEN printf('%dm', age / 60) ELSE printf('%ds', age) END
           FROM (SELECT unixepoch('now')
             - unixepoch(coalesce(grants.granted, grants.created)) AS age))"
    };
}

/// A held grant that a person should look at: it is past an hour, the gate's default timeout, while
/// another waits. Priory keeps no usual time of a command to compare with.
macro_rules! grant_long {
    () => {
        "(grants.state = 'held'
           AND unixepoch('now') - unixepoch(coalesce(grants.granted, grants.created)) >= 3600
           AND EXISTS (SELECT 1 FROM grants AS other WHERE other.state = 'waiting'))"
    };
}
pub(crate) use {grant_age, grant_long};

/// The query of `priory memory`.
pub const LIST: &str = concat!(
    "SELECT id, task, state, printf('%.1f', bytes / 1073741824.0),
       replace(command, char(10), '\\n'), ",
    grant_age!(),
    ", pid, CASE WHEN ",
    grant_long!(),
    " THEN 'long' ELSE '-' END FROM grants ORDER BY created, id"
);

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
        admit(db, factory, share, room, &now)?;
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
        db.execute("DELETE FROM grants WHERE id = ?1", [id])?;
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

/// How long a timed-out grant's request time waits for a retry of its command.
const LAPSE: &str = "-3600 seconds";

/// Grants waiting grants in request order while each fits beside the live runs and held grants, at
/// the time `now`. A grant does not count its own run's promise, which would make it wait for that
/// run to end. The first that does not fit holds back the rest, so a large build is never starved.
/// Once it has waited past `PATIENCE`, later grants that fit go first, and it keeps its place ahead
/// of them for the next room. Past `STARVING`, it holds them back again while any grant is held:
/// only a release can make room for it, and a grant nothing held can free never blocks the queue.
fn admit(
    db: &Connection,
    factory: &Factory,
    share: &Share,
    room: &Room,
    now: &str,
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
    let mut holding: bool = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM grants WHERE state = 'held')",
        [],
        |row| row.get(0),
    )?;
    for (id, bytes, task, overdue, starving) in waiting {
        let promised = state::promised(db, factory, task)?.memory;
        if !room::fits(room, share, promised, bytes) {
            if !overdue || (starving && holding) {
                break;
            }
            continue;
        }
        db.execute(
            "UPDATE grants SET state = 'held', granted = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        holding = true;
    }
    Ok(())
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
        admit(db, &Factory::default(), &share(), &room(), NOW).unwrap();
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
        db.execute(
            "INSERT INTO grants (bytes, pid, started, command, state)
             VALUES (?1, 1, 1, 'build', 'held')",
            [(gb * GB) as i64],
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
    fn past_the_second_bound_a_head_that_cannot_fit_holds_back_smaller_grants_while_grants_are_held()
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

    /// Sends `message` to the gate with the test machine's room.
    fn send(db: &mut Connection, message: Gate) -> String {
        gate(db, &message, &Factory::default(), &share(), None, || {
            Ok(room())
        })
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

    /// Polls `grant` as a command that waited `waited` seconds, where the limit is 10 minutes.
    fn poll(db: &mut Connection, grant: i64, waited: Option<u64>) -> String {
        let limit = Some(Duration::from_secs(600));
        let poll = Gate::Poll { grant, waited };
        gate(db, &poll, &Factory::default(), &share(), limit, || {
            Ok(room())
        })
        .unwrap()
        .unwrap()
    }

    /// A grant of `gb` GB that waits behind a held one of 16 GB.
    fn behind_one(gb: u64) -> (Connection, i64) {
        let mut db = one_run();
        parents(&db, 16, "held", "2026-11-23T12:00:00Z");
        let reply = send(&mut db, acquire(gb, "cargo build"));
        let grant = reply.split_once(' ').unwrap().0.parse().unwrap();
        (db, grant)
    }

    #[test]
    fn a_poll_gives_up_once_it_has_waited_the_limit_and_again_after_a_lost_reply() {
        let (mut db, grant) = behind_one(8);
        parents(&db, 4, "waiting", "2026-11-23T11:00:00Z");
        parents(&db, 4, "waiting", "2099-01-01T00:00:00Z");

        assert_eq!(poll(&mut db, grant, Some(599)), format!("{grant} waiting"));
        let gave_up = format!("{grant} gave up: waited 10m for 8 GB behind 2 grants");
        assert_eq!(poll(&mut db, grant, Some(600)), gave_up);
        assert_eq!(poll(&mut db, grant, Some(601)), gave_up);
    }

    #[test]
    fn a_poll_that_says_no_wait_never_gives_up() {
        let (mut db, grant) = behind_one(8);

        assert_eq!(poll(&mut db, grant, None), format!("{grant} waiting"));
    }

    #[test]
    fn a_grant_that_fits_at_the_limit_is_held() {
        let (mut db, grant) = behind_one(8);
        db.execute("DELETE FROM grants WHERE id != ?1", [grant])
            .unwrap();

        assert_eq!(poll(&mut db, grant, Some(600)), format!("{grant} held"));
    }

    #[test]
    fn a_grant_with_an_old_request_time_gives_up_by_its_own_wait() {
        let (mut db, grant) = behind_one(8);
        db.execute(
            "UPDATE grants SET created = '2026-11-23T10:00:00Z' WHERE id = ?1",
            [grant],
        )
        .unwrap();

        assert_eq!(poll(&mut db, grant, Some(5)), format!("{grant} waiting"));
    }

    #[test]
    fn a_grant_that_only_runs_hold_back_gives_up_with_no_grant_ahead() {
        let (mut db, grant) = behind_one(4);
        db.execute("DELETE FROM grants WHERE id != ?1", [grant])
            .unwrap();
        db.execute("UPDATE grants SET bytes = ?1", [(17 * GB) as i64])
            .unwrap();

        assert_eq!(
            poll(&mut db, grant, Some(600)),
            format!("{grant} gave up: waited 10m for 17 GB with no grant ahead")
        );
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
        send(
            &mut db,
            Gate::Release {
                grant: id,
                lapsed: true,
            },
        );
        let other = send(&mut db, acquire(4, "cargo test"));
        assert_ne!(created(&db, &other), "2026-11-23T10:00:00Z");
        send(
            &mut db,
            Gate::Release {
                grant: id + 1,
                lapsed: false,
            },
        );
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
}
