use crate::room::{self, Share};
use crate::state;
use priory_factory::factory::Factory;
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, params};
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
            db.execute("DELETE FROM grants WHERE id = ?1", [grant])?;
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
                "INSERT INTO grants (bytes, pid, started, task, command)
                 VALUES (?1, ?2, ?3, (SELECT task FROM runs WHERE session = ?4 AND ended IS NULL),
                   ?5)",
                params![*bytes as i64, pid, started, session, command],
            )?;
            let grant = db.last_insert_rowid();
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
        admit(db, factory, share, room)?;
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

/// Grants waiting grants oldest first while each fits beside the live runs and held grants. A grant
/// does not count its own run's promise, which would make it wait for that run to end. The first that
/// does not fit holds back the rest, so a large build is never starved. Once it has waited past
/// `PATIENCE`, later grants that fit go first, and it keeps its place ahead of them for the next
/// room.
fn admit(db: &Connection, factory: &Factory, share: &Share, room: &Room) -> rusqlite::Result<()> {
    let mut statement = db.prepare(
        "SELECT id, bytes, task, created <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)
         FROM grants WHERE state = 'waiting' ORDER BY id",
    )?;
    let waiting = statement
        .query_map([PATIENCE], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, bytes, task, overdue) in waiting {
        let promised = state::promised(db, factory, task)?.memory;
        if !room::fits(room, share, promised, bytes) {
            if overdue {
                continue;
            }
            break;
        }
        db.execute(
            "UPDATE grants SET state = 'held', granted = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            [id],
        )?;
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

    /// Queues a waiting grant of `gb` GB requested `ago` seconds ago.
    fn waiting(db: &Connection, gb: u64, ago: u64) -> i64 {
        db.execute(
            "INSERT INTO grants (bytes, pid, started, command, created)
             VALUES (?1, 1, 1, 'build', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2))",
            params![(gb * GB) as i64, format!("-{ago} seconds")],
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
        // `created` has whole seconds, so the test stays well inside the bound.
        waiting(&db, 10, 240);
        waiting(&db, 4, 10);

        admit(&db, &Factory::default(), &share(), &room()).unwrap();

        assert_eq!(states(&db), [held(8), wait(10), wait(4)]);
    }

    #[test]
    fn a_head_that_cannot_fit_lets_a_smaller_grant_through_after_the_bound_and_keeps_its_place() {
        let db = state::memory();
        hold(&db, 8);
        waiting(&db, 10, 301);
        waiting(&db, 4, 10);
        waiting(&db, 6, 5);

        admit(&db, &Factory::default(), &share(), &room()).unwrap();

        // 19 - 8 = 11 GB keeps 3 GB free: 4 GB fits, then 6 GB does not, and it holds back no one.
        assert_eq!(states(&db), [held(8), wait(10), held(4), wait(6)]);
        db.execute("DELETE FROM grants WHERE state = 'held'", [])
            .unwrap();
        hold(&db, 4);

        admit(&db, &Factory::default(), &share(), &room()).unwrap();

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

        admit(&db, &Factory::default(), &share(), &room()).unwrap();

        assert_eq!(states(&db), [held(8), wait(10), wait(9), held(2)]);
    }
}
