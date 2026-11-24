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
    if hold.parked {
        return parked_condition(tx, task, hold);
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

/// The one condition of a parked hold: the pull request that keys `task`, which a trigger's
/// request releases and whose merge or close ends the task.
fn parked_condition(
    tx: &Transaction,
    task: i64,
    hold: &Hold,
) -> rusqlite::Result<Result<Vec<Condition>, String>> {
    let others = Hold {
        reason: hold.reason.clone(),
        parked: true,
        ..Hold::default()
    };
    if *hold != others {
        return Ok(Err("parked takes no other condition".to_owned()));
    }
    let key: String = tx.query_row("SELECT key FROM tasks WHERE id = ?1", [task], |row| {
        row.get(0)
    })?;
    let pull_request = key
        .strip_prefix("github:")
        .and_then(|rest| rest.split_once('#'))
        .is_some_and(|(repo, number)| repo.contains('/') && number.parse::<u64>().is_ok());
    if !pull_request {
        return Ok(Err(format!(
            "parked needs a task keyed by a pull request, not {key}"
        )));
    }
    Ok(Ok(vec![Condition {
        kind: "parked",
        text: format!("until a comment or review trigger fires on {key}"),
        resource: Some(key),
        ..Condition::default()
    }]))
}

/// Releases `task` when it is parked, as the trigger request `trigger` for its pull request
/// arrived, and returns whether it was.
pub(crate) fn unpark(tx: &Transaction, task: i64, trigger: &str) -> rusqlite::Result<bool> {
    let parked: Option<i64> = tx
        .query_row(
            "SELECT holds.id FROM holds JOIN tasks ON tasks.id = holds.task
             WHERE holds.task = ?1 AND holds.kind = 'parked' AND holds.released IS NULL
             AND holds.met IS NULL AND tasks.state = 'held'",
            [task],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = parked else {
        return Ok(false);
    };
    meet(tx, id, &format!("trigger {trigger} fired"))?;
    Ok(true)
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

/// Judges every open `merged` or `waiting` condition, a batch entry's edge on another task.
/// `merged` is met once every pull request of the other task's stack merged, once that task
/// entered a step that waits for its merge, or once it is done owning no pull request. `waiting`
/// is met by those too, and once the other task waits on its open pull request. When a condition
/// can no longer be met, as the other task failed, was dropped or saw a pull request close
/// unmerged, it asks a person. A `waiting` condition met records the other task's pull request,
/// branch and head on the entry, as `head` reads it from a worktree and branch.
pub(crate) fn merged(
    tx: &Transaction,
    factory: &Factory,
    head: &dyn Fn(&str, &str) -> Option<String>,
) -> rusqlite::Result<()> {
    let open: Vec<(i64, i64, String, i64, Option<i64>)> = tx
        .prepare(
            "SELECT holds.id, holds.task, holds.kind, holds.target, holds.ask
             FROM holds JOIN tasks ON tasks.id = holds.task
             WHERE holds.kind IN ('merged', 'waiting') AND holds.released IS NULL
             AND holds.met IS NULL AND tasks.state = 'held' ORDER BY holds.id",
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
    for (id, task, kind, other, ask) in open {
        let (key, state, workflow, step, batch): (String, String, String, String, i64) = tx
            .query_row(
                "SELECT key, state, workflow, step, coalesce(batch, 0) FROM tasks WHERE id = ?1",
                [other],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )?;
        if let Some(how) = merged_as(tx, factory, other, &state, &workflow, &step)? {
            meet(tx, id, &how)?;
            continue;
        }
        if kind == "waiting" && waits_on_pull_request(tx, other, &state)? {
            record_parent(tx, task, other, head)?;
            meet(tx, id, &format!("task {other} waits on its pull request"))?;
            continue;
        }
        let lost = match state.as_str() {
            "failed" | "dropped" => format!("Task {other} ({key}) of batch {batch} {state}"),
            _ if crate::tools::closed_unmerged(tx, other)? => format!(
                "A pull request of task {other} ({key}) of batch {batch} closed without a merge"
            ),
            _ => continue,
        };
        if ask.is_some() {
            continue;
        }
        let question =
            format!("{lost}, and task {task} waits on it. Release the task, or drop it?");
        ask_person(tx, id, task, &question)?;
    }
    Ok(())
}

/// Records on the entry `task` the open pull request, branch and head of `parent`, the task it
/// stacks on, so that its start can tell whether that branch moved since.
fn record_parent(
    tx: &Transaction,
    task: i64,
    parent: i64,
    head: &dyn Fn(&str, &str) -> Option<String>,
) -> rusqlite::Result<()> {
    let open: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT pull_requests.resource, tasks.branch, tasks.worktree FROM tasks
             JOIN pull_requests ON pull_requests.task = tasks.id
               AND pull_requests.branch = tasks.branch AND pull_requests.state = 'open'
             WHERE tasks.id = ?1 ORDER BY pull_requests.created DESC LIMIT 1",
            [parent],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((pull_request, branch, worktree)) = open else {
        return Ok(());
    };
    let at = worktree.and_then(|worktree| head(&worktree, &branch));
    tx.execute(
        "UPDATE tasks SET parent_pull_request = ?2, parent_branch = ?3, parent_head = ?4
         WHERE id = ?1 AND base_task = ?5",
        params![task, pull_request, branch, at, parent],
    )
    .map(drop)
}

/// Holds the queued entry `task`, whose stacking parent `parent` moved `branch` from `was` to
/// `now` since the `waiting` moment, until a person answers whether to start it from `now`. The
/// entry forgets `was`, so the start after that answer, or after a release, takes the branch as
/// it is.
pub(crate) fn parent_moved(
    tx: &Transaction,
    task: i64,
    parent: i64,
    branch: &str,
    was: &str,
    now: &str,
) -> rusqlite::Result<()> {
    let reason = format!("task {parent} moved {branch}");
    tx.execute(
        "INSERT INTO holds (task, reason, kind, resume)
         SELECT id, ?2, 'person', state FROM tasks WHERE id = ?1",
        params![task, reason],
    )?;
    let id = tx.last_insert_rowid();
    let (was, now) = (short(was), short(now));
    tx.execute(
        "UPDATE tasks SET state = 'held', parent_head = NULL, reason = ?2 WHERE id = ?1",
        params![
            task,
            format!("held: {reason} from {was} to {now} before it started, until a person answers")
        ],
    )?;
    let question = format!(
        "Task {parent} moved {branch} from {was} to {now} before task {task} started on it. \
         Release the task to start from {now}, or drop it?"
    );
    ask_person(tx, id, task, &question)
}

/// The first 7 characters of the commit `sha`, as git abbreviates it.
fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Makes each unmet condition on the revived task `task` wait again, withdrawing the ask it put
/// to a person when `task` was lost.
pub(crate) fn revived(tx: &Transaction, task: i64) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE asks SET withdrawn = 1, withdrawn_reason = 'predecessor-revived'
         WHERE id IN (SELECT ask FROM holds WHERE target = ?1 AND released IS NULL AND met IS NULL)
         AND answer IS NULL AND withdrawn IS NULL",
        [task],
    )?;
    tx.execute(
        "UPDATE holds SET ask = NULL WHERE target = ?1 AND released IS NULL AND met IS NULL",
        [task],
    )
    .map(drop)
}

/// Asks a person whether to release or drop `task`, held on the condition `id` that can no
/// longer be met.
fn ask_person(tx: &Transaction, id: i64, task: i64, question: &str) -> rusqlite::Result<()> {
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

/// Whether task `task`, in `state`, finished its work and waits on its open pull request: done, or
/// asleep on the gates that `done` or a step after the merge stores.
pub(crate) fn waits_on_pull_request(
    db: &Connection,
    task: i64,
    state: &str,
) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM pull_requests WHERE task = ?1 AND state = 'open')
           AND (?2 = 'done' OR ?2 = 'asleep' AND EXISTS (SELECT 1 FROM waits WHERE task = ?1
             AND daemon AND fired IS NULL AND withdrawn IS NULL))",
        params![task, state],
        |row| row.get(0),
    )
}

/// How task `other`, in `state` at `step` of `workflow`, has its code merged, if it does.
fn merged_as(
    tx: &Transaction,
    factory: &Factory,
    other: i64,
    state: &str,
    workflow: &str,
    step: &str,
) -> rusqlite::Result<Option<String>> {
    let owned = crate::tools::pull_request(tx, other)?;
    if owned.is_some() && crate::tools::all_merged(tx, other)? {
        return Ok(Some(format!("task {other} merged")));
    }
    let after_merge = factory
        .workflows
        .get(workflow)
        .and_then(|flow| flow.value.step(step))
        .is_some_and(|step| step.after == Some(priory_factory::factory::StepAfter::Merged));
    if after_merge && !["failed", "dropped"].contains(&state) {
        return Ok(Some(format!("task {other} entered {step} after its merge")));
    }
    if owned.is_none() && state == "done" {
        return Ok(Some(format!("task {other} is done with no pull request")));
    }
    Ok(None)
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
pub(crate) fn meet(tx: &Transaction, id: i64, how: &str) -> rusqlite::Result<()> {
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
    let listed = serde_json::to_string(resources).expect("resources serialize");
    let met: Vec<(i64, String, String)> = tx
        .prepare(
            "SELECT id, kind, resource FROM holds WHERE released IS NULL AND met IS NOT NULL
             AND kind IN ('issue', 'blockers') AND resource IN (SELECT value FROM json_each(?1))",
        )?
        .query_map([&listed], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
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
        .query_map([&listed], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in open {
        judge(tx, id)?;
    }
    for resource in resources {
        if let Some(state) = issue_state(tx, resource)?.filter(|state| closes(state)) {
            closed_outside(tx, resource, &state)?;
        }
    }
    Ok(())
}

/// Asks a person whether to drop or keep each batch entry on the Linear issue `resource`, which
/// went `state` before the entry started, holding the entry until they answer. The issue's state
/// is no proof of merged code, so it meets no condition of the entry's successors.
fn closed_outside(tx: &Transaction, resource: &str, state: &str) -> rusqlite::Result<()> {
    let entries: Vec<(i64, i64, String)> = tx
        .prepare(
            "SELECT id, batch, state FROM tasks WHERE batch_entry IS NOT NULL
             AND (key = ?1 OR substr(key, 1, length(?1) + 1) = ?1 || '.')
             AND state IN ('queued', 'ready', 'held')
             AND NOT EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id)
             AND NOT EXISTS (SELECT 1 FROM holds WHERE holds.task = tasks.id
               AND holds.kind = 'person' AND holds.ask IS NOT NULL AND holds.reason LIKE 'batch %')
             ORDER BY id",
        )?
        .query_map([resource], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let issue = resource.trim_start_matches("linear:");
    for (task, batch, held) in entries {
        let reason = format!("batch {batch}");
        tx.execute(
            "INSERT INTO holds (task, reason, kind, resume)
             VALUES (?1, ?2, 'person', coalesce((SELECT resume FROM holds
               WHERE task = ?1 AND released IS NULL LIMIT 1),
               (SELECT state FROM tasks WHERE id = ?1)))",
            params![task, reason],
        )?;
        let id = tx.last_insert_rowid();
        let waits = format!("until a person answers whether to keep it, since {issue} is {state}");
        let reason = match held.as_str() {
            "held" => format!(" and {waits}"),
            _ => format!("held: {reason}, {waits}"),
        };
        tx.execute(
            "UPDATE tasks SET state = 'held',
               reason = iif(state = 'held', coalesce(reason, '') || ?2, ?2) WHERE id = ?1",
            params![task, reason],
        )?;
        tx.execute(
            "INSERT INTO asks (task, step, kind, question, options)
             SELECT id, step, 'agent', ?2, '[\"drop\",\"keep\"]' FROM tasks WHERE id = ?1",
            params![
                task,
                format!("{issue} is {state} outside batch {batch}. Drop the entry?")
            ],
        )?;
        tx.execute(
            "UPDATE holds SET ask = ?2 WHERE id = ?1",
            params![id, tx.last_insert_rowid()],
        )?;
    }
    Ok(())
}

/// Asks a person whether to drop or keep each task keyed by the Linear issue `resource`, which left
/// `view` before the task's first run, holding the task until they answer. A task that ran hears
/// the leave as an event of its own key instead.
pub(crate) fn left(tx: &Transaction, resource: &str, origin: &str) -> rusqlite::Result<()> {
    let tasks: Vec<(i64, String)> = tx
        .prepare(
            "SELECT id, state FROM tasks
             WHERE (key = ?1 OR substr(key, 1, length(?1) + 1) = ?1 || '.')
             AND state IN ('queued', 'ready', 'held')
             AND NOT EXISTS (SELECT 1 FROM runs WHERE runs.task = tasks.id)
             AND NOT EXISTS (SELECT 1 FROM holds WHERE holds.task = tasks.id
               AND holds.kind = 'person' AND holds.released IS NULL AND holds.met IS NULL
               AND holds.ask IS NOT NULL
               AND (holds.reason = 'left ' || ?2
                 OR ?2 LIKE 'the view%' AND holds.reason LIKE 'left the view%'))
             ORDER BY id",
        )?
        .query_map([resource, origin], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let issue = resource.trim_start_matches("linear:");
    for (task, held) in tasks {
        let reason = format!("left {origin}");
        tx.execute(
            "INSERT INTO holds (task, reason, kind, resume)
             VALUES (?1, ?2, 'person', coalesce((SELECT resume FROM holds
               WHERE task = ?1 AND released IS NULL LIMIT 1),
               (SELECT state FROM tasks WHERE id = ?1)))",
            params![task, reason],
        )?;
        let id = tx.last_insert_rowid();
        let waits = "until a person answers whether to keep it";
        let reason = match held.as_str() {
            "held" => format!(" and {waits}"),
            _ => format!("held: {reason}, {waits}"),
        };
        tx.execute(
            "UPDATE tasks SET state = 'held',
               reason = iif(state = 'held', coalesce(reason, '') || ?2, ?2) WHERE id = ?1",
            params![task, reason],
        )?;
        tx.execute(
            "INSERT INTO asks (task, step, kind, question, options)
             SELECT id, step, 'agent', ?2, '[\"drop\",\"keep\"]' FROM tasks WHERE id = ?1",
            params![task, format!("{issue} left {origin}. Drop the task?")],
        )?;
        tx.execute(
            "UPDATE holds SET ask = ?2 WHERE id = ?1",
            params![id, tx.last_insert_rowid()],
        )?;
    }
    Ok(())
}

/// The view or the project a membership event is from, as a hold names it.
pub(crate) fn origin(data: &serde_json::Value) -> String {
    if let Some(view) = data["view"].as_str() {
        return format!("the view {view}");
    }
    let stream = data["stream"].as_str().unwrap_or_default();
    let Some(named) = stream.strip_prefix("linear.project:") else {
        return "the view".to_owned();
    };
    match named.split_once(':') {
        Some((project, filter)) => format!("the project {project} filtered by {filter}"),
        None => format!("the project {named}"),
    }
}

/// Answers `keep` for each open "left" hold of a task keyed by the Linear issue `resource`
/// that left `origin`, which it entered again, so the person need not also answer the ask.
/// Entering another view or project answers nothing.
pub(crate) fn entered(tx: &Transaction, resource: &str, origin: &str) -> rusqlite::Result<()> {
    let holds: Vec<i64> = tx
        .prepare(
            "SELECT holds.id FROM holds JOIN tasks ON tasks.id = holds.task
             WHERE (tasks.key = ?1 OR substr(tasks.key, 1, length(?1) + 1) = ?1 || '.')
             AND holds.kind = 'person' AND holds.released IS NULL AND holds.met IS NULL
             AND holds.ask IS NOT NULL AND holds.reason = 'left ' || ?2
             ORDER BY holds.id",
        )?
        .query_map(params![resource, origin], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let issue = resource.trim_start_matches("linear:");
    for id in holds {
        tx.execute(
            &format!(
                "UPDATE asks SET answer = 'keep', answered = {NOW}
                 WHERE id = (SELECT ask FROM holds WHERE id = ?1)
                 AND answer IS NULL AND withdrawn IS NULL"
            ),
            [id],
        )?;
        meet(tx, id, &format!("{issue} entered {origin} again"))?;
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
    if kind == "parked" {
        return parked_closed(tx, task, &resource);
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
            ask_person(tx, id, task, &question)
        }
        _ => Ok(()),
    }
}

/// Ends the parked `task` on its pull request `resource`'s newest closed event that no event seeing
/// it open follows: `done` when it merged, else `dropped`. Without one, it waits. A merge stands
/// whatever came after it.
fn parked_closed(tx: &Transaction, task: i64, resource: &str) -> rusqlite::Result<()> {
    let merged = tx
        .query_row(
            // A poll can see `open: false` before the closed event, which alone says `merged`, arrives.
            "WITH pull AS (SELECT seq, type, data FROM inbox
               WHERE 'github:' || json_extract(data, '$.repo') || '#' || json_extract(data, '$.pull_request') = ?1)
             SELECT json_extract(data, '$.merged') FROM pull AS closed
             WHERE type = 'github.pull_request.closed'
             AND (json_extract(data, '$.merged') = 1
               OR NOT EXISTS (SELECT 1 FROM pull WHERE pull.seq > closed.seq
                 AND type != 'github.pull_request.closed' AND json_extract(data, '$.open') = 1))
             ORDER BY json_extract(data, '$.merged') IS 1 DESC, seq DESC LIMIT 1",
            [resource],
            |row| row.get::<_, Option<bool>>(0),
        )
        .optional()?;
    match merged {
        None => Ok(()),
        Some(Some(true)) => {
            let why = format!("{resource} merged while parked");
            tx.execute(
                &format!(
                    "UPDATE holds SET released = {NOW}, released_as = 'merged'
                     WHERE task = ?1 AND released IS NULL"
                ),
                [task],
            )?;
            tx.execute(
                "UPDATE tasks SET state = 'done', reason = ?2 WHERE id = ?1",
                params![task, why],
            )
            .map(drop)
        }
        Some(_) => drop_task(tx, task, &format!("{resource} closed while parked")),
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

/// The issue conditions of held tasks, met or not, with the `linear:<issue>` each waits on, whose
/// current state the daemon reads: a met one is met no more if its issue reopens.
pub(crate) fn readable(db: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    db.prepare(
        "SELECT holds.id, holds.resource FROM holds JOIN tasks ON tasks.id = holds.task
         WHERE holds.kind = 'issue' AND holds.released IS NULL
         AND tasks.state = 'held' ORDER BY holds.id",
    )?
    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
    .collect()
}

/// Meets the issue condition `id` if `current`, the state the daemon read for its issue, closes
/// it, and makes a met one unmet if `current` no longer does.
pub(crate) fn read(db: &mut Connection, id: i64, current: &str) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found = tx
        .query_row(
            "SELECT holds.resource, holds.met IS NOT NULL
             FROM holds JOIN tasks ON tasks.id = holds.task WHERE holds.id = ?1 AND holds.released IS NULL AND tasks.state = 'held'",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?;
    match found {
        Some((_, true)) if !closes(current) => {
            tx.execute(
                "UPDATE holds SET met = NULL, met_as = NULL WHERE id = ?1",
                [id],
            )?;
        }
        Some((resource, false)) => closed(&tx, id, &resource, Some(current))?,
        _ => {}
    }
    tx.commit()
}

/// Ends the open hold of `task` by dropping it, with `why` as its reason, and closes the hold's
/// open asks.
pub(crate) fn drop_task(tx: &Transaction, task: i64, why: &str) -> rusqlite::Result<()> {
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
    if crate::spend::answered(tx, id, answer)? {
        return Ok(());
    }
    match answer {
        "release" => return meet(tx, id, "released by a person"),
        "keep" => return meet(tx, id, "kept by a person"),
        _ => {}
    }
    let (task, kind, reason): (i64, String, String) = tx.query_row(
        "SELECT task, kind, reason FROM holds WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let why = match kind.as_str() {
        "merged" => "dropped by a person after the task it waited on could not merge",
        "person" if reason.starts_with("left the view") => {
            "dropped by a person after its issue left the view"
        }
        "person" if reason.starts_with("left the project") => {
            "dropped by a person after its issue left the project"
        }
        "person" if reason.starts_with("task ") => {
            "dropped by a person after the branch it stacks on moved"
        }
        "person" => "dropped by a person after its issue closed outside its batch",
        _ => "dropped by a person after its pull request closed",
    };
    drop_task(tx, task, why)
}
