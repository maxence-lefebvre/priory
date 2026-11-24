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
/// The free disk under which the sweep deletes idle tasks' build output, when the factory sets none.
const RECLAIM: f64 = 40.0 * GB;

/// The share of this machine's memory and CPUs that its agents may take together.
pub(crate) struct Share {
    pub(crate) memory: f64,
    pub(crate) cpu: f64,
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

    pub(crate) fn minus(self, other: Need) -> Need {
        Need {
            memory: self.memory.saturating_sub(other.memory),
            disk: self.disk.saturating_sub(other.disk),
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

/// What a live run that declared `need` still reserves once it uses `used` bytes of memory: the
/// memory it declared and does not use yet, and all its disk. A run whose use is unknown keeps all
/// of `need`.
pub(crate) fn unused(need: Need, used: Option<u64>) -> Need {
    Need {
        memory: need.memory.saturating_sub(used.unwrap_or(0)),
        disk: need.disk,
    }
}

/// Whether a run needing `need` fits in this machine's share of memory and its disk above the floor
/// once nothing else runs.
pub(crate) fn fits_ever(room: &Room, share: &Share, need: Need) -> bool {
    need.memory as f64 <= ceiling(room, share)
        && (need.disk as f64) < room.disk_total as f64 - disk_floor(room)
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

/// Whether free disk is down to the floor the room keeps.
pub(crate) fn short_of_disk(room: &Room) -> bool {
    room.disk_free as f64 <= disk_floor(room)
}

/// Whether free disk is under `[sweep] reclaim`, 40 GB when unset, or under the floor.
pub(crate) fn under_reclaim(room: &Room, reclaim: Option<u64>) -> bool {
    let threshold = reclaim.map_or(RECLAIM, |bytes| bytes as f64);
    (room.disk_free as f64) < threshold.max(disk_floor(room))
}

/// The free disk the room keeps: 10 % of the disk, at most 20 GB.
pub(crate) fn disk_floor(room: &Room) -> f64 {
    // `agents` has no disk part, and R4.10's slice caps only CPU and memory.
    (room.disk_total as f64 * (1.0 - SHARE)).min(DISK)
}

/// The resource a run waits for, how much of it is missing, and the task's reason.
pub(crate) struct Short {
    pub resource: &'static str,
    pub missing: String,
    pub reason: String,
}

/// The reason of a task that waits for disk outside the queue, to add back its worktree.
pub(crate) const RESTORES: &str = "waits for room to add back its worktree";

/// Why tasks wait for room, as `priory.room.tasks.waiting` names it.
pub(crate) const CAUSES: [&str; 4] = ["load", "memory", "disk", UNMEASURED];
pub(crate) const UNMEASURED: &str = "unmeasured";

impl Short {
    /// The resource under the name of its cause in `CAUSES`.
    pub(crate) fn cause(&self) -> &'static str {
        match self.resource {
            "CPU" => "load",
            resource => resource,
        }
    }
}

/// Why a run needing `need` does not fit in `room`, where `promised` is already promised to runs
/// that started but have not reported yet.
pub(crate) fn wait(room: &Room, share: &Share, promised: Need, need: Need) -> Option<Short> {
    let free = room.memory_free.saturating_sub(promised.memory);
    let keep = keep(room, share);
    if !fits(room, share, promised.memory, need.memory) {
        return Some(Short {
            resource: "memory",
            missing: format!("{:.1} GB", (need.memory as f64 + keep - free as f64) / GB),
            reason: format!(
                "waits for room: {:.1} GB of memory free, needs {:.1} GB and keeps {:.1} GB free",
                free as f64 / GB,
                need.memory as f64 / GB,
                keep / GB
            ),
        });
    }
    let keep = disk_floor(room);
    let free = room.disk_free.saturating_sub(promised.disk) as f64;
    let need_disk = need.disk;
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
