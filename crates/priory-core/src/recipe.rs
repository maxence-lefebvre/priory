//! A repository's recipes: `setup` after the daemon adds a task's new worktree, `resume` after it
//! adds back one a retirement removed, and `teardown` before the sweep or a retirement removes it.
//! Each runs in the worktree in its own process group, bounded by its timeout, and its output's
//! tail is kept.

use crate::memory;
use crate::state;
use crate::trace::Trace;
use priory_factory::factory::{Factory, Moment, Recipe};
use priory_runtime::runtime::Added;
use priory_runtime::script::{self, Script};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};

/// A run's status when it stopped while a stopped daemon's recipe ran.
const INTERRUPTED: &str = "interrupted by a daemon restart";

/// The most lines of a recipe's output a block reason or the desk shows.
const TAIL_LINES: usize = 5;

/// The recipe row the daemon wrote last for a task.
pub(crate) struct Latest {
    pub id: i64,
    pub moment: String,
    pub argv: String,
    pub started: bool,
    pub ended: bool,
    pub status: Option<String>,
    pub shown: Option<String>,
}

impl Latest {
    fn ok(&self) -> bool {
        self.status.as_deref() == Some("exit 0")
    }

    fn failed(&self) -> bool {
        self.ended && !self.ok()
    }

    /// `<moment> failed: <status>`, then the tail of its output when it printed any.
    pub fn failure(&self) -> String {
        let status = self.status.as_deref().unwrap_or("no status");
        match self.shown.as_deref().filter(|shown| !shown.is_empty()) {
            Some(shown) => format!(
                "{} failed: {status}. Its output ended with: {shown}",
                self.moment
            ),
            None => format!("{} failed: {status}", self.moment),
        }
    }
}

/// The recipe row the daemon wrote last for `task`, if any.
pub(crate) fn latest(db: &Connection, task: i64) -> rusqlite::Result<Option<Latest>> {
    db.query_row(
        "SELECT id, moment, argv, started IS NOT NULL, ended IS NOT NULL, status, shown
         FROM recipes WHERE task = ?1 ORDER BY id DESC LIMIT 1",
        [task],
        |row| {
            Ok(Latest {
                id: row.get(0)?,
                moment: row.get(1)?,
                argv: row.get(2)?,
                started: row.get(3)?,
                ended: row.get(4)?,
                status: row.get(5)?,
                shown: row.get(6)?,
            })
        },
    )
    .optional()
}

/// Records that `task` owes `recipe` at `moment` in `worktree`, to start later.
pub(crate) fn owe(
    db: &Connection,
    factory: &Factory,
    task: i64,
    moment: Moment,
    worktree: &Path,
    recipe: &Recipe,
) -> rusqlite::Result<i64> {
    let argv = serde_json::to_string(&recipe.exec()).expect("strings serialize");
    let timeout = recipe.timeout.unwrap_or(factory.command_timeout());
    db.execute(
        "INSERT INTO recipes (task, moment, worktree, argv, timeout) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            task,
            moment.name(),
            worktree.to_string_lossy(),
            argv,
            i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX)
        ],
    )?;
    Ok(db.last_insert_rowid())
}

/// The recipes running in the background, each with its row.
#[derive(Default)]
pub struct Running(Vec<(i64, Script)>);

impl Running {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Where a task's run stands once its recipes are looked at.
pub(crate) enum Before {
    /// No recipe is owed: the run starts.
    Go,
    /// A recipe runs: the run starts once it succeeds.
    Waits,
    /// The recipe could not start: the task is blocked with this reason.
    Blocked(String),
}

/// Looks at the recipes owed before `task`'s next run in `worktree` of `repo`, which `added` says
/// how the daemon found, and starts the one owed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn before_run(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    task: i64,
    repo: &str,
    worktree: &Path,
    added: &Added,
    running: &mut Running,
) -> Result<Before, Box<dyn std::error::Error>> {
    let latest = latest(db, task)?;
    // A new worktree owes its setup, and one added back, or kept after its teardown, its resume.
    // A failed setup or resume blocked the task, so its next run comes after a person resumed it.
    let owed = match (added, &latest) {
        (Added::New(_), _) => Some(Moment::Setup),
        (Added::Restored, _) => Some(Moment::Resume),
        (Added::Kept(_), Some(row)) if row.moment == "teardown" && row.ended => {
            Some(Moment::Resume)
        }
        (Added::Kept(_), Some(row)) if row.moment != "teardown" && row.failed() => {
            Some(if row.moment == "setup" {
                Moment::Setup
            } else {
                Moment::Resume
            })
        }
        (Added::Kept(_), _) => None,
    };
    if let Some(recipe) = owed.and_then(|moment| Some((moment, factory.recipe(repo, moment)?))) {
        owe(db, factory, task, recipe.0, worktree, recipe.1)?;
    }
    let Some(row) = self::latest(db, task)? else {
        return Ok(Before::Go);
    };
    if row.moment == "teardown" || row.ended {
        return Ok(Before::Go);
    }
    if row.started {
        return Ok(Before::Waits);
    }
    match start(db, factory, home, row.id, running)? {
        Ok(()) => {
            let reason = format!("runs its {}", row.moment);
            db.execute(
                "UPDATE tasks SET reason = ?2 WHERE id = ?1",
                params![task, reason],
            )?;
            Ok(Before::Waits)
        }
        Err(()) => Ok(Before::Blocked(
            self::latest(db, task)?
                .map(|row| row.failure())
                .unwrap_or_default(),
        )),
    }
}

/// Where recipe `id` writes its output.
fn log_of(home: &Path, id: i64) -> PathBuf {
    home.join("recipes").join(format!("{id}.log"))
}

/// Starts recipe `id` in its worktree, recording its process group before it runs. `Err` when it
/// cannot start, which ends it with the reason.
fn start(
    db: &Connection,
    factory: &Factory,
    home: &Path,
    id: i64,
    running: &mut Running,
) -> Result<Result<(), ()>, Box<dyn std::error::Error>> {
    let (task, worktree, argv, timeout): (i64, String, String, i64) = db.query_row(
        "SELECT task, worktree, argv, timeout FROM recipes WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let argv: Vec<String> = serde_json::from_str(&argv)?;
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    db.execute(
        "UPDATE recipes SET started = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
        [id],
    )?;
    let env = crate::daemon::command_env(home, factory, task)?;
    let env: Vec<_> = env
        .iter()
        .map(|(name, value)| (*name, value.as_os_str()))
        .collect();
    let timeout = std::time::Duration::from_secs(timeout.unsigned_abs());
    match script::spawn_held_merged(
        &argv,
        &env,
        Path::new(&worktree),
        &log_of(home, id),
        timeout,
    ) {
        Ok(mut child) => {
            db.execute(
                "UPDATE recipes SET pid = ?2, pid_started = ?3 WHERE id = ?1",
                params![id, child.pid(), memory::start_time(child.pid())],
            )?;
            child.release();
            running.0.push((id, child));
            Ok(Ok(()))
        }
        Err(error) => {
            ended(db, factory, id, &format!("cannot start: {error}"), "")?;
            Ok(Err(()))
        }
    }
}

/// Records recipe `id`'s end: its status, its output's tail, and that tail's last lines with
/// secrets hidden, as a run's screen hides them.
fn ended(
    db: &Connection,
    factory: &Factory,
    id: i64,
    status: &str,
    output: &str,
) -> rusqlite::Result<()> {
    let secrets = crate::screen::secrets(&factory.secret_envs());
    // When a secret still shows after redaction, no line of the output is kept.
    let hidden = crate::screen::redact(output, &secrets, None)
        .map_or_else(|| "[redacted]".to_owned(), |(shown, _)| shown);
    let lines: Vec<&str> = hidden
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let shown = lines[lines.len().saturating_sub(TAIL_LINES)..].join(" | ");
    db.execute(
        "UPDATE recipes SET ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), status = ?2,
           output = ?3, shown = ?4 WHERE id = ?1",
        params![id, status, output, shown],
    )?;
    Ok(())
}

/// After recipe `id` ended: a failed setup or resume blocks its task, while the task still waits
/// for it; a setup or resume that succeeded clears the task's reason.
fn settle(db: &mut Connection, trace: &Trace, id: i64) -> rusqlite::Result<()> {
    let (task, moment): (i64, String) = db.query_row(
        "SELECT task, moment FROM recipes WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let Some(row) = latest(db, task)?.filter(|row| row.id == id && moment != "teardown") else {
        return Ok(());
    };
    let waiting: bool = db.query_row(
        "SELECT state IN ('queued', 'ready') FROM tasks WHERE id = ?1",
        [task],
        |row| row.get(0),
    )?;
    if !waiting {
        return Ok(());
    }
    if row.ok() {
        db.execute(
            "UPDATE tasks SET reason = NULL WHERE id = ?1 AND reason = ?2",
            params![task, format!("runs its {moment}")],
        )?;
    } else {
        state::block(db, trace, task, "failed", &row.failure())?;
    }
    Ok(())
}

/// Starts each teardown the sweep owes, records each recipe that ended, and kills a setup or resume
/// whose task no longer waits for it, as after a cancel. Returns whether a teardown ended, so the
/// sweep looks again.
pub(crate) fn tick(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
    running: &mut Running,
) -> Result<bool, Box<dyn std::error::Error>> {
    let owed: Vec<i64> = db
        .prepare(
            "SELECT id FROM recipes WHERE moment = 'teardown' AND started IS NULL
             AND ended IS NULL ORDER BY id",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in owed {
        let _ = start(db, trace.factory, home, id, running)?;
    }
    let mut torn = false;
    let mut still = Vec::new();
    for (id, mut child) in std::mem::take(&mut running.0) {
        let wanted: bool = db.query_row(
            "SELECT recipes.moment = 'teardown' OR tasks.state IN ('queued', 'ready')
             FROM recipes JOIN tasks ON tasks.id = recipes.task WHERE recipes.id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if !wanted {
            script::kill(child.pid());
        }
        match child.poll() {
            Some((status, output)) => {
                ended(db, trace.factory, id, &status, &output)?;
                settle(db, trace, id)?;
                let moment: String =
                    db.query_row("SELECT moment FROM recipes WHERE id = ?1", [id], |row| {
                        row.get(0)
                    })?;
                torn |= moment == "teardown";
            }
            None => still.push((id, child)),
        }
    }
    running.0 = still;
    Ok(torn)
}

/// Ends each recipe a stopped daemon left running, killing its process group first when it is
/// still that recipe's. A setup or resume it ends blocks its task; a teardown it ends counts as
/// failed.
pub(crate) fn orphans(
    db: &mut Connection,
    trace: &Trace,
    home: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let left: Vec<(i64, Option<u32>, Option<i64>)> = db
        .prepare(
            "SELECT id, pid, pid_started FROM recipes WHERE started IS NOT NULL AND ended IS NULL",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, pid, started) in left {
        if let (Some(pid), Some(started)) = (pid, started)
            && memory::started(pid) == Ok(Some(started))
        {
            script::kill(pid);
        }
        let output = script::collect(&log_of(home, id));
        ended(db, trace.factory, id, INTERRUPTED, &output)?;
        settle(db, trace, id)?;
    }
    Ok(())
}

/// What a task's teardown says about removing its worktree now.
pub(crate) enum Teardown {
    /// No teardown is owed, or it succeeded: the worktree can go.
    Done,
    /// A teardown is owed or runs: the worktree stays until it ends.
    Runs,
    /// The teardown failed: the worktree stays, with this reason.
    Failed(String),
}

/// Where `task`'s teardown stands, from its latest recipe row alone.
pub(crate) fn teardown_of(db: &Connection, task: i64) -> rusqlite::Result<Option<Teardown>> {
    Ok(latest(db, task)?
        .filter(|row| row.moment == "teardown")
        .map(|row| match (row.ended, row.ok()) {
            (false, _) => Teardown::Runs,
            (true, true) => Teardown::Done,
            (true, false) => Teardown::Failed(row.failure()),
        }))
}

/// Whether no teardown of `repo` is left to run before `task`'s worktree goes.
pub(crate) fn torn(
    db: &Connection,
    factory: &Factory,
    task: i64,
    repo: &str,
) -> rusqlite::Result<bool> {
    Ok(factory.recipe(repo, Moment::Teardown).is_none()
        || matches!(teardown_of(db, task)?, Some(Teardown::Done)))
}

/// Before the sweep or a retirement removes `task`'s `worktree` of `repo`: `Done` when it may go,
/// owing the factory's teardown first when its latest one is not done. A failed teardown is owed
/// again only once the factory's teardown changed.
pub(crate) fn teardown(
    db: &Connection,
    factory: &Factory,
    task: i64,
    repo: &str,
    worktree: &Path,
) -> rusqlite::Result<Teardown> {
    let Some(recipe) = factory.recipe(repo, Moment::Teardown) else {
        return Ok(Teardown::Done);
    };
    let latest = latest(db, task)?.filter(|row| row.moment == "teardown");
    let argv = serde_json::to_string(&recipe.exec()).expect("strings serialize");
    match latest {
        Some(row) if !row.ended => Ok(Teardown::Runs),
        Some(row) if row.ok() => Ok(Teardown::Done),
        Some(row) if row.argv == argv => Ok(Teardown::Failed(row.failure())),
        _ => {
            owe(db, factory, task, Moment::Teardown, worktree, recipe)?;
            Ok(Teardown::Runs)
        }
    }
}
