use priory_factory::factory::{After, Factory, Hold};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

/// Holds the queued or ready `task` with `hold`, and returns its new reason. A running or asleep
/// task is refused: its run already started, and its own wait would race the hold.
pub(crate) fn hold(
    tx: &Transaction,
    task: i64,
    hold: &Hold,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let Some(state) = tx
        .query_row("SELECT state FROM tasks WHERE id = ?1", [task], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
    else {
        return Ok(Err(format!("there is no task {task}")));
    };
    if state != "queued" && state != "ready" {
        return Ok(Err(format!(
            "task {task} is {state}: only a queued or ready task can be held"
        )));
    }
    let conditions = match conditions(tx, task, hold, factory)? {
        Ok(conditions) => conditions,
        Err(error) => return Ok(Err(error)),
    };
    let texts: Vec<&str> = conditions
        .iter()
        .map(|condition| &*condition.text)
        .collect();
    let reason = format!("held: {}, {}", hold.reason, texts.join(" and "));
    let mut ids = vec![];
    for condition in &conditions {
        tx.execute(
            "INSERT INTO holds (task, reason, kind, until, target, resource, if_closed, resume)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                task,
                hold.reason,
                condition.kind,
                condition.until,
                condition.target,
                condition.resource,
                condition.if_closed,
                state
            ],
        )?;
        ids.push(tx.last_insert_rowid());
    }
    tx.execute(
        "UPDATE tasks SET state = 'held', reason = ?2 WHERE id = ?1",
        params![task, reason],
    )?;
    due(tx)?;
    for &id in &ids {
        judge(tx, id)?;
    }
    let released: Option<String> = tx.query_row(
        "SELECT released_as FROM holds WHERE id = ?1",
        [ids[0]],
        |row| row.get(0),
    )?;
    Ok(Ok(match released {
        Some(how) => format!("task {task} {reason}; released at once: {how}"),
        None => format!("task {task} {reason}"),
    }))
}

/// Holds a task its request files held. A condition that fails leaves the hold to a person, so the
/// task still waits.
pub(crate) fn filed(
    tx: &Transaction,
    task: i64,
    held: &Hold,
    factory: &Factory,
) -> rusqlite::Result<()> {
    let Err(error) = hold(tx, task, held, factory)? else {
        return Ok(());
    };
    let by_person = Hold {
        reason: held.reason.clone(),
        ..Hold::default()
    };
    hold(tx, task, &by_person, factory)?.expect("a new task is queued, so a person can hold it");
    tx.execute(
        "UPDATE tasks SET reason = reason || ', since its condition failed: ' || ?2 WHERE id = ?1",
        params![task, error],
    )
    .map(drop)
}

/// What releases a hold, as the holds table stores it, and how a person reads it.
#[derive(Default)]
struct Condition {
    kind: &'static str,
    text: String,
    until: Option<String>,
    target: Option<i64>,
    resource: Option<String>,
    if_closed: Option<&'static str>,
}

/// The conditions `hold` names, checked; a person's when it names none.
fn conditions(
    tx: &Transaction,
    task: i64,
    hold: &Hold,
    factory: &Factory,
) -> rusqlite::Result<Result<Vec<Condition>, String>> {
    if hold.if_closed.is_some() && hold.merge.is_empty() {
        return Ok(Err("if_closed goes with merge only".to_owned()));
    }
    let mut conditions = vec![];
    if let Some(until) = &hold.until {
        match crate::tools::instant(tx, until)? {
            Ok(at) => conditions.push(Condition {
                kind: "time",
                text: format!("until {at}"),
                until: Some(at),
                ..Condition::default()
            }),
            Err(error) => return Ok(Err(error)),
        }
    }
    for after in &hold.after {
        match after_condition(tx, task, after)? {
            Ok(condition) => conditions.push(condition),
            Err(error) => return Ok(Err(error)),
        }
    }
    for merge in &hold.merge {
        match merge_condition(tx, task, merge, hold.if_closed.as_deref(), factory)? {
            Ok(condition) => conditions.push(condition),
            Err(error) => return Ok(Err(error)),
        }
    }
    if hold.blockers {
        let issue = tx.query_row(
            "SELECT json_extract(inbox.data, '$.issue') FROM tasks JOIN inbox ON inbox.id = tasks.source
             WHERE tasks.id = ?1 AND inbox.type LIKE 'linear.%'",
            [task],
            |row| row.get::<_, Option<String>>(0),
        );
        let Some(issue) = issue.optional()?.flatten() else {
            return Ok(Err(format!(
                "task {task} comes from no Linear issue, so Priory cannot see its blockers"
            )));
        };
        let resource = format!("linear:{issue}");
        conditions.push(Condition {
            kind: "blockers",
            text: format!("until {resource} has no open blocker"),
            resource: Some(resource),
            ..Condition::default()
        });
    }
    if conditions.is_empty() {
        conditions.push(Condition {
            kind: "person",
            text: "until a person releases it".to_owned(),
            ..Condition::default()
        });
    }
    Ok(Ok(conditions))
}

/// The condition `after` names: another task that ends, or a Linear issue that closes.
fn after_condition(
    tx: &Transaction,
    task: i64,
    after: &After,
) -> rusqlite::Result<Result<Condition, String>> {
    let after = match after {
        After::Issue(text) => match text.parse() {
            Ok(after) => after,
            Err(error) => return Ok(Err(error)),
        },
        After::Task(other) => After::Task(*other),
    };
    let other = match after {
        After::Issue(issue) => {
            return Ok(Ok(Condition {
                kind: "issue",
                text: format!("until {issue} is Done or Canceled"),
                resource: Some(format!("linear:{issue}")),
                ..Condition::default()
            }));
        }
        After::Task(other) => other,
    };
    let known = tx
        .query_row("SELECT 1 FROM tasks WHERE id = ?1", [other], |_| Ok(()))
        .optional()?;
    if known.is_none() || other == task {
        return Ok(Err(format!("after must name another task, not {other}")));
    }
    Ok(Ok(Condition {
        kind: "task",
        text: format!("until task {other} ends"),
        target: Some(other),
        ..Condition::default()
    }))
}

/// The condition that the pull request `merge` merges, doing `if_closed` if it closes unmerged.
fn merge_condition(
    tx: &Transaction,
    task: i64,
    merge: &str,
    if_closed: Option<&str>,
    factory: &Factory,
) -> rusqlite::Result<Result<Condition, String>> {
    let (if_closed, does) = match if_closed {
        Some("release") => ("release", "releases it"),
        Some("drop") => ("drop", "drops it"),
        None | Some("ask") => ("ask", "asks"),
        Some(other) => {
            return Ok(Err(format!(
                "if_closed takes release, drop or ask, not {other:?}"
            )));
        }
    };
    let (trigger, repo): (String, Option<String>) = tx.query_row(
        "SELECT trigger, repo FROM tasks WHERE id = ?1",
        [task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let repo = crate::daemon::repo_of(factory, &trigger, repo.as_deref());
    let Some(resource) = pull_request(merge, repo) else {
        return Ok(Err(format!(
            "merge takes owner/name#number, or a number in the task's repository, not {merge:?}"
        )));
    };
    Ok(Ok(Condition {
        kind: "merge",
        text: format!("until {resource} merges, and {does} if it closes unmerged"),
        resource: Some(resource),
        if_closed: Some(if_closed),
        ..Condition::default()
    }))
}

/// Meets every open condition whose time passed or whose other task ended.
pub(crate) fn due(tx: &Transaction) -> rusqlite::Result<()> {
    let due: Vec<(i64, String)> = tx
        .prepare(
            "SELECT holds.id, iif(holds.kind = 'time', holds.until || ' passed',
               'task ' || target.id || ' is ' || target.state)
             FROM holds JOIN tasks ON tasks.id = holds.task
             LEFT JOIN tasks AS target ON target.id = holds.target
             WHERE holds.released IS NULL AND holds.met IS NULL AND tasks.state = 'held'
             AND ((holds.kind = 'time' AND holds.until <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
               OR (holds.kind = 'task' AND target.state IN ('done', 'failed', 'dropped')))
             ORDER BY holds.id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, how) in due {
        meet(tx, id, &how)?;
    }
    Ok(())
}

/// A person's release of the held `task`, back to the state it was held from, whatever its
/// conditions.
pub(crate) fn release_task(
    tx: &Transaction,
    task: i64,
) -> rusqlite::Result<Result<String, String>> {
    let resume = tx
        .query_row(
            "SELECT holds.resume FROM holds JOIN tasks ON tasks.id = holds.task
             WHERE holds.task = ?1 AND holds.released IS NULL AND tasks.state = 'held' LIMIT 1",
            [task],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(resume) = resume else {
        return Ok(Err(format!("task {task} is not held")));
    };
    let unmet: Vec<i64> = tx
        .prepare("SELECT id FROM holds WHERE task = ?1 AND released IS NULL AND met IS NULL")?
        .query_map([task], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in unmet {
        meet(tx, id, "released by a person")?;
    }
    Ok(Ok(format!("task {task} is {resume}")))
}

/// Meets the condition `id` as `how`, answering its ask, and releases its task once every
/// condition of its hold is met.
fn meet(tx: &Transaction, id: i64, how: &str) -> rusqlite::Result<()> {
    tx.execute(
        &format!("UPDATE holds SET met = {NOW}, met_as = ?2 WHERE id = ?1 AND met IS NULL"),
        params![id, how],
    )?;
    tx.execute(
        &format!(
            "UPDATE asks SET answer = ?2, answered = {NOW}
             WHERE id = (SELECT ask FROM holds WHERE id = ?1) AND answer IS NULL AND withdrawn IS NULL"
        ),
        params![id, how],
    )?;
    let task: i64 = tx.query_row("SELECT task FROM holds WHERE id = ?1", [id], |row| {
        row.get(0)
    })?;
    let unmet: bool = tx.query_row(
        "SELECT count(*) > 0 FROM holds WHERE task = ?1 AND released IS NULL AND met IS NULL",
        [task],
        |row| row.get(0),
    )?;
    if unmet {
        return Ok(());
    }
    tx.execute(
        &format!(
            "UPDATE holds SET released = {NOW}, released_as = (SELECT group_concat(met_as, ' and ')
               FROM (SELECT met_as FROM holds WHERE task = ?1 AND released IS NULL ORDER BY id))
             WHERE task = ?1 AND released IS NULL"
        ),
        [task],
    )?;
    tx.execute(
        "UPDATE tasks SET state = (SELECT resume FROM holds WHERE id = ?2), reason = NULL
         WHERE id = ?1 AND state = 'held'",
        [task, id],
    )
    .map(drop)
}

/// The pull request `merge` names, `owner/name#42` or `42` in `repo`, as a resource.
fn pull_request(merge: &str, repo: Option<&str>) -> Option<String> {
    let (repo, number) = match merge.split_once('#') {
        Some((repo, number)) if repo.contains('/') => (repo, number),
        Some(_) => return None,
        None => (repo?, merge),
    };
    Some(crate::route::pull_request(repo, number.parse().ok()?))
}

/// Judges every condition of an open hold on a resource that the message just stored in the inbox
/// names: an issue or blockers condition met earlier is met no more once its issue reopens or gets
/// a blocker back, and every unmet one is judged.
pub(crate) fn event(tx: &Transaction, resources: &[String]) -> rusqlite::Result<()> {
    let resources = serde_json::to_string(resources).expect("resources serialize");
    let met: Vec<(i64, String, String)> = tx
        .prepare(
            "SELECT id, kind, resource FROM holds WHERE released IS NULL AND met IS NOT NULL
             AND kind IN ('issue', 'blockers') AND resource IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&resources], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, kind, resource) in met {
        let reopened = match kind.as_str() {
            "issue" => issue_state(tx, &resource)?.is_some_and(|state| !closes(&state)),
            _ => unblocked(tx, &resource)? == Some(false),
        };
        if reopened {
            tx.execute(
                "UPDATE holds SET met = NULL, met_as = NULL WHERE id = ?1",
                [id],
            )?;
        }
    }
    let open: Vec<i64> = tx
        .prepare(
            "SELECT id FROM holds WHERE released IS NULL AND met IS NULL
             AND resource IN (SELECT value FROM json_each(?1)) ORDER BY id",
        )?
        .query_map([resources], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in open {
        judge(tx, id)?;
    }
    Ok(())
}

/// Judges the condition `id` on the newest event in the inbox about its resource: a merge meets it,
/// and a close without one does what the hold says; an issue with no open blocker, or one Done or
/// Canceled, meets it.
fn judge(tx: &Transaction, id: i64) -> rusqlite::Result<()> {
    let open = tx
        .query_row(
            "SELECT holds.task, holds.kind, holds.reason, holds.resource, holds.if_closed, holds.ask
             FROM holds JOIN tasks ON tasks.id = holds.task
             WHERE holds.id = ?1 AND holds.released IS NULL AND holds.met IS NULL
             AND tasks.state = 'held' AND holds.resource IS NOT NULL",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((task, kind, reason, resource, if_closed, ask)) = open else {
        return Ok(());
    };
    if kind == "blockers" {
        if unblocked(tx, &resource)? == Some(true) {
            meet(tx, id, &format!("{resource} has no open blocker"))?;
        }
        return Ok(());
    }
    if kind == "issue" {
        let state = issue_state(tx, &resource)?;
        return closed(tx, id, &resource, state.as_deref());
    }
    let merged = tx
        .query_row(
            "SELECT json_extract(data, '$.merged') FROM inbox
             WHERE type = 'github.pull_request.closed'
             AND 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1
             ORDER BY seq DESC LIMIT 1",
            [&resource],
            |row| row.get::<_, Option<bool>>(0),
        )
        .optional()?;
    let unmerged = format!("{resource} closed without a merge");
    match (merged, if_closed.as_deref()) {
        (None, _) => Ok(()),
        (Some(Some(true)), _) => meet(tx, id, &format!("{resource} merged")),
        (Some(Some(false)), Some("release")) => meet(tx, id, &unmerged),
        (Some(Some(false)), Some("drop")) => drop_task(tx, task, &unmerged),
        (Some(merged), _) if ask.is_none() => {
            let closed = match merged {
                Some(_) => unmerged,
                None => format!("{resource} closed, and GitHub no longer says whether it merged"),
            };
            let question = format!(
                "Pull request {closed}, and task {task} was held until it merged: {reason}. Release the task, or drop it?"
            );
            tx.execute(
                "INSERT INTO asks (task, step, kind, question, options)
                 SELECT id, step, 'agent', ?2, '[\"release\",\"drop\"]' FROM tasks WHERE id = ?1",
                params![task, question],
            )?;
            tx.execute(
                "UPDATE holds SET ask = ?2 WHERE id = ?1",
                params![id, tx.last_insert_rowid()],
            )
            .map(drop)
        }
        _ => Ok(()),
    }
}

/// Whether the newest blockers the inbox holds for the Linear issue `resource` are none; `None`
/// when no event said.
fn unblocked(tx: &Transaction, resource: &str) -> rusqlite::Result<Option<bool>> {
    tx.query_row(
        // A null snapshot is unknown, and hides the older ones behind it.
        "SELECT json_type(data, '$.blockers') = 'array'
           AND json_array_length(data, '$.blockers') = 0 FROM inbox
         WHERE 'linear:' || json_extract(data, '$.issue') = ?1
         AND json_type(data, '$.blockers') IS NOT NULL ORDER BY seq DESC LIMIT 1",
        [resource],
        |row| row.get(0),
    )
    .optional()
}

/// The newest state the inbox holds for the Linear issue `resource`: a state change's `to`, else
/// the state another issue event saw at its poll. A leaving view event can carry an earlier
/// poll's fields.
fn issue_state(tx: &Transaction, resource: &str) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT coalesce(json_extract(data, '$.to'), json_extract(data, '$.state')) AS state
         FROM inbox WHERE type LIKE 'linear.issue.%' AND type != 'linear.issue.left'
         AND 'linear:' || json_extract(data, '$.issue') = ?1 AND state IS NOT NULL
         ORDER BY seq DESC LIMIT 1",
        [resource],
        |row| row.get(0),
    )
    .optional()
}

// shortcut: matches Linear's default state names; a team that renames them needs the state's type.
fn closes(state: &str) -> bool {
    matches!(state, "Done" | "Canceled")
}

/// Meets the issue condition `id` when `state`, its Linear issue's newest, closes the issue.
fn closed(tx: &Transaction, id: i64, resource: &str, state: Option<&str>) -> rusqlite::Result<()> {
    let Some(state) = state.filter(|state| closes(state)) else {
        return Ok(());
    };
    let issue = resource.trim_start_matches("linear:");
    meet(tx, id, &format!("{issue} is {state}"))
}

/// The unmet issue conditions of held tasks, with the `linear:<issue>` each waits on, whose
/// current state the daemon reads.
pub(crate) fn readable(db: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    db.prepare(
        "SELECT holds.id, holds.resource FROM holds JOIN tasks ON tasks.id = holds.task
         WHERE holds.kind = 'issue' AND holds.released IS NULL AND holds.met IS NULL
         AND tasks.state = 'held' ORDER BY holds.id",
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// Meets the issue condition `id` if `current`, the state the daemon read for its issue, closes it.
pub(crate) fn read(db: &mut Connection, id: i64, current: &str) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let resource = tx
        .query_row(
            "SELECT holds.resource FROM holds JOIN tasks ON tasks.id = holds.task
             WHERE holds.id = ?1 AND holds.released IS NULL AND holds.met IS NULL
             AND tasks.state = 'held'",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(resource) = resource {
        closed(&tx, id, &resource, Some(current))?;
    }
    tx.commit()
}

/// Ends the open hold of `task` by dropping it, with `why` as its reason, and closes the hold's
/// open asks.
fn drop_task(tx: &Transaction, task: i64, why: &str) -> rusqlite::Result<()> {
    close_asks(tx, task)?;
    tx.execute(
        &format!(
            "UPDATE holds SET released = {NOW}, released_as = 'dropped'
             WHERE task = ?1 AND released IS NULL"
        ),
        [task],
    )?;
    tx.execute(
        "UPDATE tasks SET state = 'dropped', reason = ?2 WHERE id = ?1",
        params![task, why],
    )
    .map(drop)
}

/// Answers `drop` to the open asks of `task`'s open hold, as the task is dropped.
pub(crate) fn close_asks(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    tx.execute(
        &format!(
            "UPDATE asks SET answer = 'drop', answered = {NOW}
             WHERE id IN (SELECT ask FROM holds WHERE task = ?1 AND released IS NULL)
             AND answer IS NULL AND withdrawn IS NULL"
        ),
        [task],
    )
    .map(drop)
}

/// The open hold whose ask is `ask`, which a person's answer releases or drops.
pub(crate) fn of_ask(tx: &Transaction, ask: i64) -> rusqlite::Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM holds WHERE ask = ?1 AND released IS NULL",
        [ask],
        |row| row.get(0),
    )
    .optional()
}

/// Applies a person's answer to a hold's ask: `release` or `drop`.
pub(crate) fn answered(tx: &Transaction, id: i64, answer: &str) -> rusqlite::Result<()> {
    if answer == "release" {
        return meet(tx, id, "released by a person");
    }
    let task: i64 = tx.query_row("SELECT task FROM holds WHERE id = ?1", [id], |row| {
        row.get(0)
    })?;
    drop_task(
        tx,
        task,
        "dropped by a person after its pull request closed",
    )
}
