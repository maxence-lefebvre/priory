//! Whether the claude.ai account a runner's `claude` signs in with has the connectors an agent
//! names. Before such an agent's run starts, a background check asks `claude mcp get` for each
//! one, so a missing or doubled connector blocks the task with a reason instead of leaving the
//! agent without its tools.

use priory_runtime::{runtime, servers};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a found connector counts as found, so most runs start without a check.
const FOUND_FOR: Duration = Duration::from_secs(10 * 60);

/// How long one `claude mcp get` may take, with its fetch of the account's connector list.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The reason a task carries while its connectors are checked.
pub(crate) const CHECKING: &str = "checks its connectors with claude mcp get";

/// What a check found wrong, as the end of a block reason after `agent <name> `.
type Verdict = Result<(), String>;

/// A check of one task's connectors, which a thread fills in.
struct Check {
    connectors: Vec<String>,
    verdict: Arc<Mutex<Option<Verdict>>>,
}

/// The checks under way, by task, and when each connector was last found.
#[derive(Default)]
pub struct Probes {
    checks: HashMap<i64, Check>,
    found: HashMap<String, Instant>,
}

/// Where a task's run stands once its connectors are looked at.
pub(crate) enum Before {
    /// Every connector was found lately: the run starts.
    Go,
    /// A check runs: the run starts once it finds them.
    Waits,
    /// A connector is missing, doubled or could not be checked: the task is blocked with this.
    Blocked(String),
}

impl Probes {
    /// Whether a check of `task`'s connectors still runs.
    pub(crate) fn checking(&self, task: i64) -> bool {
        self.checks.get(&task).is_some_and(|check| {
            check
                .verdict
                .lock()
                .map(|verdict| verdict.is_none())
                .unwrap_or(false)
        })
    }

    /// Looks at the `connectors` agent `agent` names for `task`'s run in `dir`, and starts a check
    /// of those not found lately.
    pub(crate) fn before_run(
        &mut self,
        task: i64,
        agent: &str,
        connectors: &[String],
        dir: &Path,
    ) -> Before {
        if let Some(check) = self.checks.get(&task) {
            let verdict = check
                .verdict
                .lock()
                .ok()
                .and_then(|mut verdict| verdict.take());
            let Some(verdict) = verdict else {
                return Before::Waits;
            };
            let check = self.checks.remove(&task).expect("the check is there");
            if let Err(reason) = verdict {
                return Before::Blocked(format!("agent {agent} {reason}"));
            }
            for name in check.connectors {
                self.found.insert(name, Instant::now());
            }
        }
        let unchecked: Vec<String> = connectors
            .iter()
            .filter(|name| {
                self.found
                    .get(*name)
                    .is_none_or(|at| at.elapsed() >= FOUND_FOR)
            })
            .cloned()
            .collect();
        if unchecked.is_empty() {
            return Before::Go;
        }
        let verdict = Arc::new(Mutex::new(None));
        let (filled, names, dir) = (verdict.clone(), unchecked.clone(), dir.to_path_buf());
        std::thread::spawn(move || {
            let found = check(&names, &dir);
            if let Ok(mut verdict) = filled.lock() {
                *verdict = Some(found);
            }
        });
        self.checks.insert(
            task,
            Check {
                connectors: unchecked,
                verdict,
            },
        );
        Before::Waits
    }
}

/// Asks `claude mcp get` in `dir` for each of `names`, and for a second connector of that name,
/// which Claude Code names `claude.ai <name> (2)`.
fn check(names: &[String], dir: &Path) -> Verdict {
    for name in names {
        let server = servers::connector_server(name);
        let twin = format!("{server} (2)");
        let failed = |error: String| {
            format!("cannot check its connector {name}: claude mcp get \"{server}\" {error}")
        };
        if !runtime::claude_finds(dir, &server, TIMEOUT).map_err(failed)? {
            return Err(format!(
                "names connector {name}, which the claude.ai account that claude signs in with on this runner does not have: connect it in claude.ai under Settings, Connectors, then resume the task"
            ));
        }
        if runtime::claude_finds(dir, &twin, TIMEOUT).map_err(|error| {
            format!("cannot check its connector {name}: claude mcp get \"{twin}\" {error}")
        })? {
            return Err(format!(
                "names connector {name}, which the claude.ai account holds twice, as {server} and {twin}: rename one in claude.ai, then resume the task"
            ));
        }
    }
    Ok(())
}
