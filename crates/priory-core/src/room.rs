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

/// Why a run needing `need` bytes does not fit in `room`, where `promised` bytes are already
/// promised to runs that started but have not reported yet.
pub(crate) fn wait(room: &Room, share: &Share, promised: u64, need: u64) -> Option<String> {
    let free = room.memory_free.saturating_sub(promised);
    let keep = room.memory_total as f64 * (1.0 - share.memory);
    if (free as f64) < need as f64 + keep {
        return Some(format!(
            "waits for room: {:.1} GB of memory free, needs {:.1} GB and keeps {:.1} GB free",
            free as f64 / GB,
            need as f64 / GB,
            keep / GB
        ));
    }
    // `agents` has no disk part, and R4.10's slice caps only CPU and memory.
    let keep = (room.disk_total as f64 * (1.0 - SHARE)).min(DISK);
    if (room.disk_free as f64) <= keep {
        return Some(format!(
            "waits for room: {:.1} GB of disk free, keeps {:.1} GB free",
            room.disk_free as f64 / GB,
            keep / GB
        ));
    }
    let ceiling = room.cpus as f64 * share.cpu;
    (room.load >= ceiling).then(|| {
        format!(
            "waits for room: load {:.2} on {} CPUs, keeps it under {ceiling:.2}",
            room.load, room.cpus
        )
    })
}
