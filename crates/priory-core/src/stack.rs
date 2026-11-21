use rusqlite::{Connection, params};
use serde_json::{Value, json};

/// One pull request of a task's stack, as `pull_requests` stores it.
#[derive(Debug, Clone)]
pub(crate) struct Pull {
    pub resource: String,
    pub branch: String,
    pub base: String,
    pub state: String,
}

impl Pull {
    /// `58`, from `github:owner/name#58`.
    pub fn number(&self) -> &str {
        self.resource
            .rsplit_once('#')
            .map_or("", |(_, number)| number)
    }

    pub fn json(&self) -> Value {
        json!({
            "pull_request": self.resource,
            "number": self.number().parse::<u64>().ok(),
            "branch": self.branch,
            "base": self.base,
            "state": self.state,
        })
    }
}

/// The pull requests `task` opened or claimed, bottom first: each one's base is the branch of the
/// one before it. A chain that a retarget broke follows the one below it, in the order they were
/// added.
pub(crate) fn of(db: &Connection, task: i64) -> rusqlite::Result<Vec<Pull>> {
    let mut left = db
        .prepare(
            "SELECT resource, branch, base, state FROM pull_requests WHERE task = ?1
             ORDER BY created, rowid",
        )?
        .query_map(params![task], |row| {
            Ok(Pull {
                resource: row.get(0)?,
                branch: row.get(1)?,
                base: row.get(2)?,
                state: row.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stack: Vec<Pull> = Vec::with_capacity(left.len());
    while !left.is_empty() {
        // The next one sits on the last one placed, else it starts a chain of its own.
        let next = stack
            .last()
            .and_then(|below| left.iter().position(|pull| pull.base == below.branch))
            .or_else(|| {
                left.iter()
                    .position(|pull| !left.iter().any(|other| other.branch == pull.base))
            })
            .unwrap_or(0);
        stack.push(left.remove(next));
    }
    Ok(stack)
}

/// The stack of `task` as JSON, bottom first.
pub(crate) fn json(db: &Connection, task: i64) -> rusqlite::Result<Value> {
    Ok(of(db, task)?.iter().map(Pull::json).collect())
}

/// Where `resource` sits in the stack of `task`, as `#58, 2 of 3`; none outside a stack of two or
/// more.
pub(crate) fn place(
    db: &Connection,
    task: i64,
    resource: &str,
) -> rusqlite::Result<Option<String>> {
    let stack = of(db, task)?;
    let at = stack.iter().position(|pull| pull.resource == resource);
    Ok(at
        .filter(|_| stack.len() > 1)
        .map(|at| format!("#{}, {} of {}", stack[at].number(), at + 1, stack.len())))
}

/// Makes `resource` the pull request of `branch` in the stack of `task`: one that replaced a closed
/// pull request of that branch takes its place, and a reopened one is open again.
pub(crate) fn record(
    tx: &rusqlite::Transaction,
    task: i64,
    resource: &str,
    branch: &str,
    base: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM pull_requests WHERE task = ?1 AND branch = ?3 AND resource != ?2",
        params![task, resource, branch],
    )?;
    tx.execute(
        "INSERT INTO pull_requests (task, resource, branch, base) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (task, resource) DO UPDATE SET branch = ?3, base = ?4, state = 'open'",
        params![task, resource, branch, base],
    )?;
    Ok(())
}
