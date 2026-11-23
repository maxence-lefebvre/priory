//! What each live run uses now, measured from its systemd scope's cgroup.

use priory_runtime::{cgroup, runtime, slice};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a run whose pane tmux did not find waits before tmux is asked again, so a dead pane
/// costs one `tmux list-panes` per run every 30 seconds, not one per room check.
const RETRY: Duration = Duration::from_secs(30);

/// What tmux said about a live run's pane.
#[derive(Clone, Copy)]
enum Pane {
    /// The pane's process and its start time, so a pid the kernel reused is never read.
    Found { pid: u32, started: i64 },
    /// No live pane when tmux was asked.
    Missing { at: Instant },
}

/// Each live run's pane, by run id and harness session. The scope execs the agent in place, so the
/// process lasts as long as the run's agent.
static PANES: Mutex<BTreeMap<(i64, String), Pane>> = Mutex::new(BTreeMap::new());

fn panes() -> std::sync::MutexGuard<'static, BTreeMap<(i64, String), Pane>> {
    PANES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The memory run `run` holds now that measured free memory does not count as free, in bytes: its
/// scope's anonymous and shared memory. None when the run has no scope inside the agents slice, as
/// on a host without systemd, or when it cannot be read.
pub(crate) fn memory(run: i64, tmux: &str, session: &str) -> Option<u64> {
    let key = (run, session.to_owned());
    let cached = panes().get(&key).copied();
    let pid = match cached {
        Some(Pane::Found { pid, started }) if crate::memory::started(pid) == Some(started) => pid,
        Some(Pane::Missing { at }) if at.elapsed() < RETRY => return None,
        // The lock is not held while tmux runs.
        _ => {
            let found = runtime::pane_pid(tmux, session)
                .and_then(|pid| Some((pid, crate::memory::started(pid)?)));
            let pane = match found {
                Some((pid, started)) => Pane::Found { pid, started },
                None => Pane::Missing { at: Instant::now() },
            };
            panes().insert(key, pane);
            found?.0
        }
    };
    cgroup::memory(&cgroup::scope_of(pid, &slice::name())?)
}

/// Forgets the pane of every run that is not in `live`, so an ended run's pid is never read again.
pub(crate) fn keep(live: &[(i64, String)]) {
    panes().retain(|key, _| live.contains(key));
}
