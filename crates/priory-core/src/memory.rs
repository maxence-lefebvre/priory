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

/// Grants waiting grants oldest first while each fits beside the live runs and held grants. The
/// first that does not fit holds back the rest, so a large build is never starved.
fn admit(db: &Connection, factory: &Factory, share: &Share, room: &Room) -> rusqlite::Result<()> {
    let mut promised = state::promised(db, factory)?.memory;
    let mut statement =
        db.prepare("SELECT id, bytes FROM grants WHERE state = 'waiting' ORDER BY id")?;
    let waiting = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? as u64))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, bytes) in waiting {
        if !room::fits(room, share, promised, bytes) {
            break;
        }
        db.execute(
            "UPDATE grants SET state = 'held', granted = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            [id],
        )?;
        promised += bytes;
    }
    Ok(())
}
