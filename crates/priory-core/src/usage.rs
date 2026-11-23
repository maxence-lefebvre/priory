//! What each live run uses now, measured from its systemd scope's cgroup.

use priory_runtime::{cgroup, runtime, slice};
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Each live run's pane process, by run id and harness session, found once with tmux. The scope
/// execs the agent in place, so the process lasts as long as the run's agent.
static PANES: Mutex<BTreeMap<(i64, String), u32>> = Mutex::new(BTreeMap::new());

/// The memory run `run` uses now, in bytes: its scope's `memory.current`. None when the run has no
/// scope inside the agents slice, as on a host without systemd, or when it cannot be read.
pub(crate) fn memory(run: i64, tmux: &str, session: &str) -> Option<u64> {
    let key = (run, session.to_owned());
    let mut panes = PANES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let pid = match panes.get(&key) {
        Some(pid) => *pid,
        None => {
            let pid = runtime::pane_pid(tmux, session)?;
            panes.insert(key, pid);
            pid
        }
    };
    drop(panes);
    cgroup::memory(&cgroup::scope_of(pid, &slice::name())?)
}

/// Forgets the pane of every run that is not in `live`, so an ended run's pid is never read again.
pub(crate) fn keep(live: &[(i64, String)]) {
    let mut panes = PANES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    panes.retain(|key, _| live.contains(key));
}
