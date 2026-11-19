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

/// The memory and disk a run reserves, in bytes.
#[derive(Default, Clone, Copy)]
pub(crate) struct Need {
    pub memory: u64,
    pub disk: u64,
}

impl Need {
    pub(crate) fn plus(self, other: Need) -> Need {
        Need {
            memory: self.memory.saturating_add(other.memory),
            disk: self.disk.saturating_add(other.disk),
        }
    }
}

/// The memory and disk `agent` declares, in bytes.
pub(crate) fn need(agent: Option<&Agent>) -> Need {
    Need {
        memory: agent.and_then(|agent| agent.memory).unwrap_or(MEMORY),
        disk: agent.and_then(|agent| agent.disk).unwrap_or(0),
    }
}

/// Whether a run needing `need` fits in this machine's share of memory once nothing else runs.
pub(crate) fn fits_ever(room: &Room, share: &Share, need: Need) -> bool {
    need.memory as f64 <= room.memory_total as f64 * share.memory
}

/// The resource a run waits for, how much of it is missing, and the task's reason.
pub(crate) struct Short {
    pub resource: &'static str,
    pub missing: String,
    pub reason: String,
}

/// Why a run needing `need` does not fit in `room`, where `promised` is already promised to runs
/// that started but have not reported yet.
pub(crate) fn wait(room: &Room, share: &Share, promised: Need, need: Need) -> Option<Short> {
    let free = room.memory_free.saturating_sub(promised.memory);
    let need_disk = need.disk;
    let need = need.memory;
    let keep = room.memory_total as f64 * (1.0 - share.memory);
    if (free as f64) < need as f64 + keep {
        return Some(Short {
            resource: "memory",
            missing: format!("{:.1} GB", (need as f64 + keep - free as f64) / GB),
            reason: format!(
                "waits for room: {:.1} GB of memory free, needs {:.1} GB and keeps {:.1} GB free",
                free as f64 / GB,
                need as f64 / GB,
                keep / GB
            ),
        });
    }
    // `agents` has no disk part, and R4.10's slice caps only CPU and memory.
    let keep = (room.disk_total as f64 * (1.0 - SHARE)).min(DISK);
    let free = room.disk_free.saturating_sub(promised.disk) as f64;
    if free - need_disk as f64 <= keep {
        let needs = match need_disk {
            0 => String::new(),
            disk => format!("needs {:.1} GB and ", disk as f64 / GB),
        };
        return Some(Short {
            resource: "disk",
            missing: format!("{:.1} GB", (keep + need_disk as f64 - free) / GB),
            reason: format!(
                "waits for room: {:.1} GB of disk free, {needs}keeps {:.1} GB free",
                free / GB,
                keep / GB
            ),
        });
    }
    let ceiling = room.cpus as f64 * share.cpu;
    (room.load >= ceiling).then(|| Short {
        resource: "CPU",
        missing: format!("load {:.2}", room.load - ceiling),
        reason: format!(
            "waits for room: load {:.2} on {} CPUs, keeps it under {ceiling:.2}",
            room.load, room.cpus
        ),
    })
}
