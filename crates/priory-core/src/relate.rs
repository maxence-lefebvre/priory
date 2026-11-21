use priory_intake::request::{RELATIONS, Relation};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

/// `relates` has no direction, so it matches its pair either way round.
const SAME: &str = "(task = ?1 AND kind = ?2 AND other = ?3)
  OR (?2 = 'relates' AND task = ?3 AND kind = ?2 AND other = ?1)";

/// Records that `task` builds on, fixes, takes up or relates to `other`, as `by`.
pub fn relate(
    db: &mut Connection,
    task: i64,
    kind: &str,
    other: i64,
    by: &str,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(refused) = refused(&tx, task, kind, other)? {
        return Ok(Err(refused));
    }
    insert(&tx, task, kind, other, by)?;
    tx.commit()?;
    Ok(Ok(format!("task {task} {kind} task {other}")))
}

/// Removes the relation `relate` records.
pub fn unrelate(
    db: &mut Connection,
    task: i64,
    kind: &str,
    other: i64,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(refused) = refused(&tx, task, kind, other)? {
        return Ok(Err(refused));
    }
    let removed = tx.execute(
        &format!("DELETE FROM relations WHERE {SAME}"),
        params![task, kind, other],
    )?;
    if removed == 0 {
        return Ok(Err(format!(
            "task {task} has no {kind} relation to task {other}"
        )));
    }
    tx.commit()?;
    Ok(Ok(format!("task {task} no longer {kind} task {other}")))
}

/// Why `task` cannot relate to `other` by `kind`, if it cannot.
fn refused(
    tx: &Transaction,
    task: i64,
    kind: &str,
    other: i64,
) -> rusqlite::Result<Option<String>> {
    if !RELATIONS.contains(&kind) {
        return Ok(Some(format!(
            "`{kind}` is not a relation: name one of {}",
            RELATIONS.join(", ")
        )));
    }
    if task == other {
        return Ok(Some("a task does not relate to itself".to_owned()));
    }
    for id in [task, other] {
        let found = tx
            .query_row("SELECT 1 FROM tasks WHERE id = ?1", [id], |_| Ok(()))
            .optional()?;
        if found.is_none() {
            return Ok(Some(format!("no task {id}")));
        }
    }
    Ok(None)
}

/// Stores the relation unless it is already stored; both tasks must exist.
pub(crate) fn insert(
    tx: &Transaction,
    task: i64,
    kind: &str,
    other: i64,
    by: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        &format!(
            "INSERT INTO relations (task, kind, other, by)
             SELECT ?1, ?2, ?3, ?4 WHERE ?1 != ?3
               AND EXISTS (SELECT 1 FROM tasks WHERE id = ?3)
               AND NOT EXISTS (SELECT 1 FROM relations WHERE {SAME})"
        ),
        params![task, kind, other, by],
    )
    .map(drop)
}

/// The relations a new task starts with: those its request names, that it takes up the latest
/// dropped or failed task on its key, and that it fixes each task whose merged pull request its
/// request text links.
pub(crate) fn created(
    tx: &Transaction,
    task: i64,
    key: &str,
    relates: &[Relation],
) -> rusqlite::Result<()> {
    // A request comes off the bus, so a kind the table refuses is skipped, not an error.
    for relation in relates
        .iter()
        .filter(|relation| RELATIONS.contains(&relation.kind.as_str()))
    {
        insert(tx, task, &relation.kind, relation.task, "request")?;
    }
    let dropped = tx
        .query_row(
            "SELECT id FROM tasks WHERE key = ?1 AND id != ?2 AND state IN ('dropped', 'failed')
             ORDER BY id DESC LIMIT 1",
            params![key, task],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(dropped) = dropped {
        insert(tx, task, "takes-up", dropped, "priory")?;
    }
    let request: String = tx.query_row(
        "SELECT request FROM task_text WHERE task = ?1",
        [task],
        |row| row.get(0),
    )?;
    for pull in priory_intake::request::pull_requests(&request) {
        let merged_by = tx
            .prepare(
                "SELECT task FROM pull_requests WHERE resource = ?1 AND state = 'merged' AND task != ?2",
            )?
            .query_map(params![pull, task], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for other in merged_by {
            insert(tx, task, "fixes", other, "priory")?;
        }
    }
    Ok(())
}

/// Every relation `task` is in, either side, oldest first.
pub(crate) fn json(db: &Connection, task: i64) -> rusqlite::Result<Value> {
    let mut statement = db.prepare(
        "SELECT task, kind, other, by, created FROM relations WHERE task = ?1 OR other = ?1
         ORDER BY created, rowid",
    )?;
    let relations = statement
        .query_map([task], |row| {
            Ok(json!({
                "task": row.get::<_, i64>(0)?,
                "kind": row.get::<_, String>(1)?,
                "other": row.get::<_, i64>(2)?,
                "by": row.get::<_, String>(3)?,
                "created": row.get::<_, String>(4)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::Array(relations))
}

/// `priory tasks --graph`: the tasks related to `root`, or every related task without it, as an
/// indented tree from each component's lowest id, or as a Mermaid flowchart. A tree shows each task
/// once, so a relation that closes a cycle shows only in the flowchart.
pub fn graph(
    db: &Connection,
    root: Option<i64>,
    mermaid: bool,
) -> rusqlite::Result<Result<String, String>> {
    let relations = db
        .prepare("SELECT task, kind, other FROM relations ORDER BY created, rowid")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<Vec<(i64, String, i64)>>>()?;
    let label = |id: i64| {
        db.query_row(
            "SELECT id || ' ' || key || ' ' || state FROM tasks WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
    };
    let roots = match root {
        Some(root) if label(root)?.is_none() => return Ok(Err(format!("no task {root}"))),
        Some(root) => vec![root],
        None => {
            let mut ids: Vec<i64> = relations.iter().flat_map(|(a, _, b)| [*a, *b]).collect();
            ids.sort();
            ids.dedup();
            ids
        }
    };
    let mut seen = Vec::new();
    let mut tree = String::new();
    for root in roots {
        if !seen.contains(&root) {
            visit(root, 0, None, &relations, &mut seen, &mut tree, &label)?;
        }
    }
    if !mermaid {
        return Ok(Ok(tree));
    }
    let mut chart = String::from("flowchart TD\n");
    for id in &seen {
        let label = label(*id)?.unwrap_or_default();
        chart += &format!("    t{id}[\"{}\"]\n", label.replace('"', "#quot;"));
    }
    for (task, kind, other) in relations.iter().filter(|(task, ..)| seen.contains(task)) {
        chart += &format!("    t{task} -->|\"{kind}\"| t{other}\n");
    }
    Ok(Ok(chart))
}

/// Prints `id` at `depth`, with the relation it was reached by, then each task related to it that
/// the tree does not show yet.
fn visit(
    id: i64,
    depth: usize,
    via: Option<&(i64, String, i64)>,
    relations: &[(i64, String, i64)],
    seen: &mut Vec<i64>,
    tree: &mut String,
    label: &impl Fn(i64) -> rusqlite::Result<Option<String>>,
) -> rusqlite::Result<()> {
    seen.push(id);
    *tree += &"  ".repeat(depth);
    *tree += &label(id)?.unwrap_or_default();
    if let Some((task, kind, other)) = via {
        *tree += &format!(" ({task} {kind} {other})");
    }
    tree.push('\n');
    for relation @ (task, _, other) in relations {
        let next = match id {
            _ if *task == id => *other,
            _ if *other == id => *task,
            _ => continue,
        };
        if !seen.contains(&next) {
            visit(
                next,
                depth + 1,
                Some(relation),
                relations,
                seen,
                tree,
                label,
            )?;
        }
    }
    Ok(())
}

/// Answers the `relate` tool in the agent's own `priory mcp` process: it records how the caller's
/// task relates to `task`. Any other tool is someone else's: `None`.
pub fn tool(session: &str, tool: &str, arguments: &Value) -> Option<Result<String, String>> {
    if tool != "relate" {
        return None;
    }
    let read = || -> Result<(String, i64), String> {
        if let Some(name) = arguments.as_object().and_then(|fields| {
            fields
                .keys()
                .find(|name| !["kind", "task"].contains(&name.as_str()))
        }) {
            return Err(format!("relate takes kind and task, not {name}"));
        }
        let kind = arguments["kind"].as_str().ok_or("kind must be a string")?;
        let task = arguments["task"].as_i64().ok_or("task must be a task id")?;
        Ok((kind.to_owned(), task))
    };
    Some(read().and_then(|(kind, other)| {
        let mut db = crate::state::open()?;
        let task = db
            .query_row(
                "SELECT task FROM runs WHERE ended IS NULL AND session = ?1",
                [session],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no live run has session {session}"))?;
        relate(&mut db, task, &kind, other, "agent").map_err(|error| error.to_string())?
    }))
}
