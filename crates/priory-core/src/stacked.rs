use crate::trace::Trace;
use priory_runtime::runtime;
use rusqlite::{Connection, TransactionBehavior, params};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long an entry waits after git timed out on it.
const AGAIN: Duration = Duration::from_secs(600);

/// Each entry git timed out on, and when to ask it again.
static STALLED: Mutex<BTreeMap<i64, Instant>> = Mutex::new(BTreeMap::new());

fn stalled() -> MutexGuard<'static, BTreeMap<i64, Instant>> {
    STALLED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Tells each stacked entry that started when the task it stacks on pushed its branch again, while
/// that branch has a pull request open: `task 31 pushed priory/31 to <sha>; rebase onto it`. The
/// line fires the entry's gate, reaches it while it runs, or reopens it when it is done.
pub(crate) fn pushed(db: &mut Connection, trace: &Trace, home: &Path) -> rusqlite::Result<()> {
    let stacked: Vec<(i64, i64, String, String, String)> = db
        .prepare(
            "SELECT tasks.id, parent.id, parent.branch, parent.worktree, tasks.parent_head
             FROM tasks JOIN tasks AS parent ON parent.id = tasks.base_task
             WHERE tasks.parent_head IS NOT NULL AND parent.worktree IS NOT NULL
             AND tasks.state NOT IN ('failed', 'dropped')
             AND EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id)
             AND EXISTS (SELECT 1 FROM pull_requests WHERE pull_requests.task = parent.id
               AND pull_requests.branch = parent.branch AND pull_requests.state = 'open')
             ORDER BY tasks.id",
        )?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (task, parent, branch, worktree, told) in stacked {
        if stalled()
            .get(&task)
            .is_some_and(|again| *again > Instant::now())
        {
            continue;
        }
        let tree = Path::new(&worktree);
        // The entry may have started from commits its parent had not pushed yet: an older push is
        // no news to it.
        let news = runtime::pushed_head(home, tree, &branch).and_then(|head| match head {
            Some(head) if head != told => {
                Ok((!runtime::is_ancestor(tree, &head, &told)?).then_some(head))
            }
            _ => Ok(None),
        });
        let head = match news {
            Ok(Some(head)) => head,
            Ok(None) => continue,
            // A hung disk would hold this tick for each entry, and each tick for this one.
            Err(why) => {
                eprintln!("priory daemon: task {task}: {why}; asked again in 10 minutes");
                stalled().insert(task, Instant::now() + AGAIN);
                return Ok(());
            }
        };
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let moved = tx.execute(
            "UPDATE tasks SET parent_head = ?3 WHERE id = ?1 AND parent_head = ?2",
            params![task, told, head],
        )?;
        if moved == 1 {
            let text = format!("task {parent} pushed {branch} to {head}; rebase onto it");
            crate::tools::tell_stacked(&tx, trace, task, &text, None)?;
        }
        tx.commit()?;
    }
    Ok(())
}
