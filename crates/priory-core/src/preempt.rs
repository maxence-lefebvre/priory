//! Urgent wakes: a head of the queue in a `preempt` band marks runs of lower bands, which end at
//! their next turn end so the head fits (docs/daemon/room.md#urgent-wakes).

use crate::room::{self, Need, Share};
use priory_factory::factory::{Effect, Factory, Session};
use priory_runtime::room::Room;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

/// A live run the head might stop.
struct Candidate {
    run: i64,
    task: i64,
    band: String,
    /// What the run reserves beyond its use, which its end takes off the promises.
    unused: Need,
    /// What the run uses now, which its end gives back to measured free memory.
    used: u64,
    protected: Option<String>,
    tmux: String,
}

/// The queue's head, which waits for memory or disk.
pub(crate) struct Head<'a> {
    pub task: i64,
    pub need: Need,
    pub measured: &'a Room,
    pub promised: Need,
}

/// A band's place, highest first. A task with no band is in the last one, and a band the factory
/// no longer has comes after it.
fn rank(bands: &[&str], band: Option<&str>) -> usize {
    match band {
        None => bands.len().saturating_sub(1),
        Some(band) => bands
            .iter()
            .position(|known| *known == band)
            .unwrap_or(bands.len()),
    }
}

/// The band `task` is in, as its reasons name it.
fn band_of(db: &Connection, factory: &Factory, task: i64) -> rusqlite::Result<String> {
    let band: Option<String> =
        db.query_row("SELECT band FROM tasks WHERE id = ?1", [task], |row| {
            row.get(0)
        })?;
    Ok(band.unwrap_or_else(|| {
        factory
            .bands()
            .last()
            .copied()
            .unwrap_or_default()
            .to_owned()
    }))
}

/// Why no one may stop the live run of `task` at `step` between two turns, if a reason holds: an
/// irreversible step, whose stop could leave a push without its pull request; a keyboard a
/// person holds; or a shared session. Urgent wakes, spend caps and the budget check inside a turn
/// all read it. A tmux client watching the run is the caller's test, since it runs tmux.
pub(crate) fn guarded(
    factory: &Factory,
    task: i64,
    workflow: &str,
    step: &str,
    agent: &str,
    keyboard_held: bool,
) -> Option<String> {
    let at = factory
        .workflows
        .get(workflow)
        .and_then(|flow| flow.value.step(step));
    let defined = factory.agents.get(agent).map(|agent| &agent.value);
    if at.is_some_and(|at| at.effect == Effect::Irreversible) {
        Some(format!("task {task} is at irreversible step {step}"))
    } else if keyboard_held {
        Some(format!("task {task}'s keyboard is held"))
    } else if defined.is_some_and(|agent| matches!(agent.session, Session::Shared)) {
        Some(format!("task {task} runs in a shared session"))
    } else {
        None
    }
}

/// Whether `run` has a keyboard grant a person holds now.
pub(crate) const KEYBOARD_HELD: &str = "EXISTS (SELECT 1 FROM keyboard_grants AS held
  WHERE held.run = runs.id AND held.ended IS NULL AND julianday(held.ends) > julianday('now'))";

/// Marks the runs whose promises, once gone, let `head` fit, and withdraws every other mark.
/// Returns what the head's reason adds: the runs that stop, or why none does.
pub(crate) fn plan(
    db: &Connection,
    factory: &Factory,
    share: &Share,
    head: &Head,
    watched: &dyn Fn(&str) -> bool,
) -> rusqlite::Result<Option<String>> {
    let bands = factory.bands();
    let band = band_of(db, factory, head.task)?;
    if !factory.preempt().contains(&band) {
        withdraw(db)?;
        return Ok(None);
    }
    let head_rank = rank(&bands, Some(&band));
    let mut statement = db.prepare(&format!(
        "SELECT runs.id, runs.task, runs.step, coalesce(runs.agent, ''), runs.tmux, tasks.band,
           tasks.workflow, tasks.trigger, tasks.preempted IS NOT NULL, {KEYBOARD_HELD},
           tasks.position, runs.session, coalesce(tasks.slot, tasks.id)
         FROM runs JOIN tasks ON tasks.id = runs.task WHERE runs.ended IS NULL"
    ))?;
    let mut rows = statement.query([])?;
    let mut candidates = Vec::new();
    while let Some(row) = rows.next()? {
        let (run, task, step, agent, tmux): (i64, i64, String, String, String) = (
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        );
        let (own_band, workflow, trigger): (Option<String>, String, String) =
            (row.get(5)?, row.get(6)?, row.get(7)?);
        let (stopped, held, position): (bool, bool, Option<i64>) =
            (row.get(8)?, row.get(9)?, row.get(10)?);
        let (session, slot): (String, i64) = (row.get(11)?, row.get(12)?);
        let defined = factory.agents.get(&agent).map(|agent| &agent.value);
        let own_rank = rank(&bands, own_band.as_deref());
        let own_band = own_band.unwrap_or_else(|| bands.last().copied().unwrap_or_default().into());
        let protected = if let Some(why) = guarded(factory, task, &workflow, &step, &agent, held) {
            Some(why)
        } else if trigger == crate::stall::TRIGGER {
            Some(format!("task {task} is the sweeper"))
        } else if own_rank <= head_rank {
            Some(format!("task {task} is in band {own_band}"))
        } else if stopped {
            Some(format!("task {task} was already stopped once"))
        } else {
            None
        };
        let used = crate::usage::memory(run, &tmux, &session);
        candidates.push((
            (own_rank, position.is_none(), position, slot),
            Candidate {
                run,
                task,
                band: own_band,
                unused: room::unused(room::need(defined), used),
                used: used.unwrap_or(0),
                protected,
                tmux,
            },
        ));
    }
    // Reverse start order: the lowest band, then the last in a person's order, then the youngest.
    candidates.sort_by(|(a, _), (b, _)| b.cmp(a));
    let mut candidates: Vec<Candidate> = candidates.into_iter().map(|(_, c)| c).collect();
    // Read only for the runs it may pick, since each read runs tmux.
    for candidate in candidates.iter_mut().filter(|c| c.protected.is_none()) {
        if watched(&candidate.tmux) {
            candidate.protected = Some(format!("task {} is watched in tmux", candidate.task));
        }
    }
    let mut freed = Need::default();
    let mut returned = 0u64;
    let mut picked = Vec::new();
    let mut fits = false;
    for candidate in candidates.iter().filter(|c| c.protected.is_none()) {
        picked.push(candidate);
        freed = freed.plus(candidate.unused);
        returned = returned.saturating_add(candidate.used);
        let measured = Room {
            memory_free: head.measured.memory_free.saturating_add(returned),
            ..*head.measured
        };
        if room::wait(&measured, share, head.promised.minus(freed), head.need).is_none() {
            fits = true;
            break;
        }
    }
    let eligible = !picked.is_empty();
    if !fits {
        picked.clear();
    }
    let protected: Vec<_> = candidates
        .iter()
        .filter_map(|c| Some(json!({ "task": c.task, "reason": c.protected.as_ref()? })))
        .collect();
    let runs: Vec<i64> = picked.iter().map(|c| c.run).collect();
    mark(db, head.task, &band, &runs)?;
    let protected_json = (!protected.is_empty()).then(|| json!(protected).to_string());
    db.execute(
        "UPDATE tasks SET protected = ?2 WHERE id = ?1 AND protected IS NOT ?2",
        params![head.task, protected_json],
    )?;
    db.execute(
        "UPDATE tasks SET protected = NULL WHERE protected IS NOT NULL AND id != ?1",
        [head.task],
    )?;
    let named: Vec<String> = picked
        .iter()
        .map(|c| format!("task {} ({})", c.task, c.band))
        .collect();
    Ok(match named.as_slice() {
        [] => {
            let mut why: Vec<String> = candidates
                .iter()
                .filter_map(|c| c.protected.clone())
                .collect();
            if eligible {
                why.push("the runs it may stop free too little".to_owned());
            }
            (!why.is_empty()).then(|| format!("stops no run: {}", why.join(", ")))
        }
        [one] => Some(format!("{one} stops at its next turn end")),
        many => Some(format!("{} stop at their next turn end", many.join(", "))),
    })
}

/// Marks `runs` for `head`, in band `band`, keeping an earlier mark's time, and withdraws every
/// other mark.
fn mark(db: &Connection, head: i64, band: &str, runs: &[i64]) -> rusqlite::Result<()> {
    let picked = json!(runs).to_string();
    db.execute(
        "UPDATE tasks SET reason = NULL WHERE state = 'running'
           AND reason LIKE 'stops at its next turn end for task %'
           AND id IN (SELECT task FROM runs WHERE ended IS NULL AND preempted_for IS NOT NULL
             AND id NOT IN (SELECT value FROM json_each(?1)))",
        [&picked],
    )?;
    db.execute(
        "UPDATE runs SET preempted_for = NULL, marked = NULL WHERE ended IS NULL
           AND preempted_for IS NOT NULL AND id NOT IN (SELECT value FROM json_each(?1))",
        [&picked],
    )?;
    let reason = format!("stops at its next turn end for task {head} ({band})");
    for run in runs {
        db.execute(
            "UPDATE runs SET marked = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), preempted_for = ?2
             WHERE id = ?1 AND preempted_for IS NOT ?2",
            params![run, head],
        )?;
        db.execute(
            "UPDATE tasks SET reason = ?2 WHERE id = (SELECT task FROM runs WHERE id = ?1)
               AND reason IS NOT ?2",
            params![run, reason],
        )?;
    }
    Ok(())
}

/// Withdraws every mark and every head's protected runs: no head waits in a `preempt` band.
pub(crate) fn withdraw(db: &Connection) -> rusqlite::Result<()> {
    mark(db, 0, "", &[])?;
    db.execute(
        "UPDATE tasks SET protected = NULL WHERE protected IS NOT NULL",
        [],
    )
    .map(drop)
}

/// Withdraws the mark of `run`, which a person now types in or watches.
fn unmark(db: &Connection, run: i64) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE tasks SET reason = NULL WHERE reason LIKE 'stops at its next turn end for task %'
           AND id = (SELECT task FROM runs WHERE id = ?1)",
        [run],
    )?;
    db.execute(
        "UPDATE runs SET preempted_for = NULL, marked = NULL WHERE id = ?1",
        [run],
    )
    .map(drop)
}

/// The head task the live run `run` stops for at this turn end, if it still should. A run with an
/// attempted, unconfirmed prompt keeps its mark for its next turn end; a run a person now types
/// in or watches, or whose head no longer waits, loses it.
pub(crate) fn due(
    db: &Connection,
    run: i64,
    watched: &dyn Fn(&str) -> bool,
) -> rusqlite::Result<Option<i64>> {
    let Some((head, tmux)) = db
        .query_row(
            "SELECT preempted_for, tmux FROM runs WHERE id = ?1 AND ended IS NULL
               AND preempted_for IS NOT NULL",
            [run],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let (unconfirmed, held, waits): (bool, bool, bool) = db.query_row(
        "SELECT EXISTS (SELECT 1 FROM outbox WHERE run = ?1 AND state IN ('attempted', 'uncertain')),
           EXISTS (SELECT 1 FROM keyboard_grants WHERE run = ?1 AND ended IS NULL
             AND julianday(ends) > julianday('now')),
           EXISTS (SELECT 1 FROM tasks WHERE id = ?2 AND state IN ('queued', 'ready'))",
        params![run, head],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if unconfirmed {
        return Ok(None);
    }
    if held || !waits || watched(&tmux) {
        unmark(db, run)?;
        return Ok(None);
    }
    Ok(Some(head))
}

/// The live runs marked to stop that sit idle between turns with no prompt attempted: none of
/// them has a turn end to wait for.
pub(crate) fn idle(db: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT session FROM runs WHERE ended IS NULL AND preempted_for IS NOT NULL
           AND idle = 1 AND ready = 1
           AND NOT EXISTS (SELECT 1 FROM outbox WHERE outbox.run = runs.id
             AND outbox.state IN ('attempted', 'uncertain'))
         ORDER BY id",
    )?;
    statement.query_map([], |row| row.get(0))?.collect()
}

/// The band of the head `head` and its stop prompt.
pub(crate) fn prompt(
    db: &Connection,
    factory: &Factory,
    head: i64,
) -> rusqlite::Result<(String, String)> {
    let band = band_of(db, factory, head)?;
    let text = format!(
        "Priory stopped your last run at the end of your turn to make room for task {head}, in band {band}.\nYour session and worktree are as you left them. Continue."
    );
    Ok((band, text))
}
