use crate::trace::Trace;
use priory_runtime::runtime;
use rusqlite::{Connection, TransactionBehavior, params};
use std::path::Path;

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
        // The entry may have started from commits its parent had not pushed yet: an older push is
        // no news to it.
        let Some(head) = runtime::pushed_head(home, Path::new(&worktree), &branch).filter(|head| {
            *head != told && !runtime::is_ancestor(Path::new(&worktree), head, &told)
        }) else {
            continue;
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
