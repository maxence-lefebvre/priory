use priory_factory::factory::{Agent, Factory};
use priory_runtime::room::Room;

/// What an agent with no `memory` declares: Claude Code takes about 0.5 GB, and an agent runs builds
/// and tests beside it.
const MEMORY: u64 = 4 << 30;
/// The share of the machine a runner's agents may take when its file sets none.
const SHARE: f64 = 0.9;
const GB: f64 = (1u64 << 30) as f64;
/// The most disk the room keeps free, so a large disk with tens of GB free still admits runs.
const DISK: f64 = 20.0 * GB;

/// The share of this machine's memory and CPUs that its agents may take together.
pub(crate) struct Share {
    memory: f64,
    cpu: f64,
}

impl Share {
    /// The `agents` share of the runner named `host`, each part 90 % when unset.
    pub(crate) fn of(factory: &Factory, host: Option<&str>) -> Share {
        let agents = host
            .and_then(|host| factory.runners.get(host))
            .and_then(|runner| runner.value.agents.as_ref());
        Share {
            memory: agents.and_then(|share| share.memory).unwrap_or(SHARE),
            cpu: agents.and_then(|share| share.cpu).unwrap_or(SHARE),
        }
    }
}

/// The memory `agent` declares, in bytes.
pub(crate) fn need(agent: Option<&Agent>) -> u64 {
    agent.and_then(|agent| agent.memory).unwrap_or(MEMORY)
}

/// Whether a run needing `need` bytes fits in this machine's share once nothing else runs.
pub(crate) fn fits_ever(room: &Room, share: &Share, need: u64) -> bool {
    need as f64 <= ceiling(room, share)
}

/// The memory this machine's agents may take together, in bytes.
pub(crate) fn ceiling(room: &Room, share: &Share) -> f64 {
    room.memory_total as f64 * share.memory
}

/// Whether `need` bytes fit beside `promised` ones and still keep the memory this machine's share
/// leaves free.
pub(crate) fn fits(room: &Room, share: &Share, promised: u64, need: u64) -> bool {
    room.memory_free.saturating_sub(promised) as f64 >= need as f64 + keep(room, share)
}

fn keep(room: &Room, share: &Share) -> f64 {
    room.memory_total as f64 * (1.0 - share.memory)
}

/// How memory falls short of `need` bytes beside `promised` ones, if it does.
pub(crate) fn memory_short(room: &Room, share: &Share, promised: u64, need: u64) -> Option<String> {
    let free = room.memory_free.saturating_sub(promised);
    (!fits(room, share, promised, need)).then(|| {
        format!(
            "{:.1} GB of memory free, needs {:.1} GB and keeps {:.1} GB free",
            free as f64 / GB,
            need as f64 / GB,
            keep(room, share) / GB
        )
    })
}

/// How free disk falls short of what the room keeps, if it does.
pub(crate) fn disk_short(room: &Room) -> Option<String> {
    // `agents` has no disk part, and R4.10's slice caps only CPU and memory.
    let keep = (room.disk_total as f64 * (1.0 - SHARE)).min(DISK);
    ((room.disk_free as f64) <= keep).then(|| {
        format!(
            "{:.1} GB of disk free, keeps {:.1} GB free",
            room.disk_free as f64 / GB,
            keep / GB
        )
    })
}

/// Why a run needing `need` bytes does not fit in `room`, where `promised` bytes are already
/// promised to runs that started but have not reported yet.
pub(crate) fn wait(room: &Room, share: &Share, promised: u64, need: u64) -> Option<String> {
    if let Some(short) = memory_short(room, share, promised, need).or_else(|| disk_short(room)) {
        return Some(format!("waits for room: {short}"));
    }
    let ceiling = room.cpus as f64 * share.cpu;
    (room.load >= ceiling).then(|| {
        format!(
            "waits for room: load {:.2} on {} CPUs, keeps it under {ceiling:.2}",
            room.load, room.cpus
        )
    })
}
