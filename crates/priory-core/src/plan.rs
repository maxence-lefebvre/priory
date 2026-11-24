//! The plan an approval ask shows: snapshotted when the ask is raised, and compared when it is
//! answered, so an answer approves only what the person read.

use priory_factory::factory::Step;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The step whose result is a task's brief, and the one whose result is its plan. A factory may
/// name its plan step `plan-<something>`, such as `plan-implementation`. The workflow marks no
/// step as the plan, so the rule goes by the step's name.
pub(crate) const BRIEF_STEP: &str = "groom";
pub(crate) const PLAN_STEP: &str = "plan";
pub(crate) const PLAN_PREFIX: &str = "plan-";

/// The plan documents an agent writes in its worktree.
const FILES: [&str; 2] = [".priory/plan.md", ".priory/brief.md"];
/// The most bytes of a plan document an ask shows.
const SHOWN: usize = 64 * 1024;
/// A longer file is no plan document, so the daemon never reads an endless one.
const LONGEST: u64 = 1024 * 1024;

struct Document {
    /// The file, or `None` for the result's summary.
    path: Option<PathBuf>,
    text: String,
    truncated: bool,
    hash: String,
}

impl Document {
    fn new(path: Option<PathBuf>, bytes: &[u8]) -> Self {
        let mut end = bytes.len().min(SHOWN);
        while end < bytes.len() && end > 0 && bytes[end] & 0xC0 == 0x80 {
            end -= 1;
        }
        Document {
            path,
            text: String::from_utf8_lossy(&bytes[..end]).into_owned(),
            truncated: end < bytes.len(),
            hash: Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        }
    }
}

/// Whether a human step that `step`'s `outcome` leads to approves a plan.
fn is_plan(step: &Step, outcome: &str) -> bool {
    [BRIEF_STEP, PLAN_STEP].contains(&step.id.as_str())
        || step.id.starts_with(PLAN_PREFIX)
        || crate::tools::GRADES.contains(&outcome)
}

/// The step whose plan an ask at `target` approves: that of the task's latest result, when its
/// outcome leads to `target`.
pub(crate) fn approved_at<'a>(
    tx: &Transaction,
    task: i64,
    target: &str,
    step: impl Fn(&str) -> Option<&'a Step>,
) -> rusqlite::Result<Option<String>> {
    let latest = tx
        .query_row(
            "SELECT step, outcome FROM results WHERE task = ?1 ORDER BY run DESC LIMIT 1",
            [task],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    Ok(latest.and_then(|(at, outcome)| {
        let step = step(&at)?;
        let leads = step.next.get(&outcome).is_some_and(|next| next == target);
        (leads && is_plan(step, &outcome)).then_some(at)
    }))
}

/// Stores the plan of `step` as the one ask `ask` shows, in place of an earlier one.
pub(crate) fn snapshot(tx: &Transaction, ask: i64, task: i64, step: &str) -> rusqlite::Result<()> {
    let Some((run, document)) = document(tx, task, step)? else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR REPLACE INTO ask_plans (ask, step, run, path, text, truncated, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            ask,
            step,
            run,
            document
                .path
                .map(|path| path.to_string_lossy().into_owned()),
            document.text,
            document.truncated,
            document.hash
        ],
    )
    .map(drop)
}

/// Takes the plan of ask `ask` again, as when the ask is asked again. An ask with no plan keeps none.
pub(crate) fn again(tx: &Transaction, ask: i64) -> rusqlite::Result<()> {
    match of_ask(tx, ask)? {
        Some((task, step, _)) => snapshot(tx, ask, task, &step),
        None => Ok(()),
    }
}

/// Whether the plan ask `ask` shows is no longer the task's plan. A retired task has no worktree
/// to read, so what it showed stands.
pub(crate) fn changed(tx: &Transaction, ask: i64) -> rusqlite::Result<bool> {
    let Some((task, step, hash)) = of_ask(tx, ask)? else {
        return Ok(false);
    };
    let retired: bool = tx.query_row(
        "SELECT retired IS NOT NULL FROM tasks WHERE id = ?1",
        [task],
        |row| row.get(0),
    )?;
    Ok(!retired && document(tx, task, &step)?.is_none_or(|(_, now)| now.hash != hash))
}

fn of_ask(tx: &Transaction, ask: i64) -> rusqlite::Result<Option<(i64, String, String)>> {
    tx.query_row(
        "SELECT asks.task, ask_plans.step, ask_plans.hash FROM ask_plans
         JOIN asks ON asks.id = ask_plans.ask WHERE asks.id = ?1",
        [ask],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
}

/// The newest plan of the latest visit to `step` that ended with a result, with that result's run:
/// the newest file written since the visit began, or else the result's summary. A visit is the runs
/// of the step since another step's run, an earlier result or the task's generation began.
fn document(db: &Connection, task: i64, step: &str) -> rusqlite::Result<Option<(i64, Document)>> {
    let found = db
        .query_row(
            "SELECT latest.run, latest.summary, tasks.worktree,
               (SELECT min(visit.started) FROM runs AS visit
                WHERE visit.task = ?1 AND visit.step = ?2 AND visit.id <= latest.run
                AND visit.id > max(
                  coalesce((SELECT max(before.id) FROM runs AS before
                    WHERE before.task = ?1 AND before.id < latest.run AND (before.step != ?2
                      OR EXISTS (SELECT 1 FROM results WHERE results.run = before.id))), 0),
                  coalesce((SELECT max(after) FROM generations
                    WHERE generations.task = ?1 AND after < latest.run), 0)))
             FROM results AS latest JOIN tasks ON tasks.id = latest.task
             WHERE latest.task = ?1 AND latest.step = ?2 ORDER BY latest.run DESC LIMIT 1",
            params![task, step],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((run, summary, worktree, started)) = found else {
        return Ok(None);
    };
    let file = worktree
        .zip(started.and_then(|started| started.parse::<jiff::Timestamp>().ok()))
        .and_then(|(worktree, started)| {
            let worktree = Path::new(&worktree).canonicalize().ok()?;
            FILES
                .iter()
                .filter_map(|file| read(&worktree, file))
                .filter(|(modified, ..)| *modified >= SystemTime::from(started))
                .max_by_key(|(modified, ..)| *modified)
        })
        .map(|(_, path, bytes)| Document::new(Some(path), &bytes));
    let document = file.unwrap_or_else(|| Document::new(None, summary.as_bytes()));
    Ok(Some((run, document)))
}

/// `file` of `worktree`, with when it was written, if it is a regular file the worktree holds.
fn read(worktree: &Path, file: &str) -> Option<(SystemTime, PathBuf, Vec<u8>)> {
    let path = worktree.join(file);
    let opened = open(&path)?;
    let found = opened.metadata().ok()?;
    if !found.is_file() || found.len() > LONGEST || !beneath(worktree, &path, &found) {
        return None;
    }
    let mut bytes = Vec::new();
    opened.take(LONGEST + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= LONGEST).then_some((found.modified().ok()?, path, bytes))
}

/// Opens `path` without following a symlink at its end or waiting on a FIFO's writer.
#[cfg(unix)]
fn open(path: &Path) -> Option<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(FLAGS?)
        .open(path)
        .ok()
}

#[cfg(not(unix))]
fn open(_: &Path) -> Option<File> {
    None
}

/// `O_NOFOLLOW | O_NONBLOCK`, which std does not name, or none on a system not listed.
#[cfg(unix)]
const FLAGS: Option<i32> = if cfg!(target_os = "macos") {
    Some(0x100 | 0x4)
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some(0x2_0000 | 0x800)
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some(0x8000 | 0x800)
} else {
    None
};

/// Whether the opened file is one the worktree holds. A folder on the way may have been a symlink
/// when it opened, so the file now at `path` must be the same inode.
#[cfg(unix)]
fn beneath(worktree: &Path, path: &Path, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.canonicalize().is_ok_and(|real| {
        real.starts_with(worktree)
            && std::fs::symlink_metadata(&real)
                .is_ok_and(|now| (now.dev(), now.ino()) == (opened.dev(), opened.ino()))
    })
}

#[cfg(not(unix))]
fn beneath(_: &Path, _: &Path, _: &std::fs::Metadata) -> bool {
    false
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::tools::{self, Answer};
    use crate::trace::Trace;
    use priory_factory::factory::Factory;
    use serde_json::{Value, json};
    use std::fs;
    use std::time::Duration;

    /// A groom task whose groom run ended red a minute ago, with a worktree, ready at `approve-plan`.
    struct Setup {
        home: PathBuf,
        worktree: PathBuf,
        factory: Factory,
        db: Connection,
    }

    impl Drop for Setup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    impl Setup {
        fn new(name: &str) -> Self {
            let home =
                std::env::temp_dir().join(format!("priory-plan-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&home);
            let worktree = home.join("7");
            fs::create_dir_all(worktree.join(".priory")).unwrap();
            let worktree = worktree.canonicalize().unwrap();
            let groom = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/groom");
            let db = crate::state::memory();
            db.execute(
                "INSERT INTO tasks (key, workflow, step, state, band, lane, lane_derived, trigger,
                   source, entered, repo, branch, worktree)
                 VALUES ('cli:7', 'groom', 'approve-plan', 'ready', 'now', 'cli', 0, 'cli', '7',
                   strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'acme/shop', 'priory/7', ?1)",
                [worktree.to_str().unwrap()],
            )
            .unwrap();
            let setup = Setup {
                home,
                worktree,
                factory: priory_factory::factory::load(&groom).0,
                db,
            };
            setup.result("groom", "red", "the groom summary");
            setup
        }

        /// A run of `step` that started a minute ago and ended with `outcome`.
        fn result(&self, step: &str, outcome: &str, summary: &str) {
            self.db
                .execute(
                    "INSERT INTO runs (task, step, tmux, session, started, ended)
                     VALUES (1, ?1, 't', 's' || (SELECT count(*) FROM runs),
                       strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 minute'), 'x')",
                    [step],
                )
                .unwrap();
            self.db
                .execute(
                    "INSERT INTO results (run, task, step, outcome, summary)
                     VALUES (last_insert_rowid(), 1, ?1, ?2, ?3)",
                    [step, outcome, summary],
                )
                .unwrap();
        }

        fn write(&self, file: &str, text: &str, seconds_ago: u64) {
            let path = self.worktree.join(".priory").join(file);
            fs::write(&path, text).unwrap();
            let at = SystemTime::now() - Duration::from_secs(seconds_ago);
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(at)
                .unwrap();
        }

        /// Raises the ask of the task's human step.
        fn raise(&mut self) {
            tools::enter_queued(&mut self.db, &self.factory).unwrap();
        }

        fn shown(&self) -> Value {
            crate::listing::task(&self.db, 1).unwrap().unwrap()["plan_file"].clone()
        }

        /// The text the raised ask shows.
        fn raised(&mut self) -> Value {
            self.raise();
            self.shown()["text"].clone()
        }

        fn answer(&mut self, revision: i64) -> Result<String, String> {
            let trace = Trace {
                factory: &self.factory,
                revision: None,
                machine: None,
                runner: None,
                usage: priory_runtime::transcript::usage,
            };
            let answer = Answer {
                ask: 1,
                text: "approved".to_owned(),
                revision,
                run: false,
                by: "cli".to_owned(),
            };
            tools::answer(&mut self.db, &answer, &trace, &|_, _| None).unwrap()
        }

        fn step(&self) -> String {
            self.db
                .query_row("SELECT step FROM tasks", [], |row| row.get(0))
                .unwrap()
        }
    }

    #[test]
    fn plan_edited_after_it_was_shown_refuses_the_answer_and_shows_the_edit_as_the_next_revision() {
        let mut setup = Setup::new("edited");
        setup.write("brief.md", "as read", 30);
        assert_eq!(setup.raised(), "as read");
        setup.write("brief.md", "edited", 0);
        assert_eq!(setup.shown()["text"], "as read");

        let refused = setup.answer(1).unwrap_err();

        assert!(refused.contains("changed since revision 1"), "{refused}");
        assert_eq!(setup.step(), "approve-plan");
        let shown = setup.shown();
        assert_eq!(
            (&shown["text"], &shown["revision"]),
            (&json!("edited"), &json!(2))
        );
        assert!(setup.answer(1).is_err(), "the old revision stays refused");
        setup.answer(2).unwrap();
        assert_eq!(setup.step(), "implement");
    }

    #[test]
    fn plan_of_an_ask_raised_again_is_the_one_its_revision_shows() {
        let mut setup = Setup::new("raised-again");
        setup.write("brief.md", "first", 30);
        setup.raise();
        let tx = setup.db.transaction().unwrap();
        assert!(tools::tell_ask(&tx, 1, "a comment").unwrap());
        tx.commit().unwrap();
        let shown = setup.shown();
        assert_eq!(
            (&shown["text"], &shown["revision"]),
            (&json!("first"), &json!(2))
        );
        setup.answer(2).unwrap();
    }

    #[test]
    fn plan_removed_or_past_the_cap_edited_refuses_the_answer() {
        let mut setup = Setup::new("tail");
        // A 3-byte character straddles the cap.
        let text = format!("{}€tail", "a".repeat(SHOWN - 1));
        setup.write("plan.md", &text, 30);
        assert_eq!(setup.raised(), "a".repeat(SHOWN - 1));
        assert_eq!(setup.shown()["truncated"], true);
        setup.write("plan.md", &format!("{text}!"), 0);
        assert!(setup.answer(1).is_err(), "an edit nobody was shown");
        fs::remove_file(setup.worktree.join(".priory/plan.md")).unwrap();
        assert!(setup.answer(2).is_err());
        assert_eq!(setup.shown()["text"], "the groom summary");
    }

    #[test]
    fn plan_of_a_retired_task_is_answered_without_its_worktree() {
        let mut setup = Setup::new("retired");
        setup.write("brief.md", "as read", 30);
        setup.raise();
        fs::remove_dir_all(&setup.worktree).unwrap();
        setup
            .db
            .execute("UPDATE tasks SET retired = 'x'", [])
            .unwrap();
        setup.answer(1).unwrap();
    }

    #[test]
    fn plan_left_by_an_earlier_visit_loses_to_the_newer_result() {
        let mut setup = Setup::new("leftover");
        setup.write("plan.md", "left over", 3600);
        assert_eq!(setup.raised(), "the groom summary");
        assert_eq!(setup.shown()["path"], Value::Null);
    }

    #[test]
    fn plan_left_over_loses_to_a_fresh_brief() {
        let mut setup = Setup::new("fresh-brief");
        setup.write("plan.md", "left over", 3600);
        setup.write("brief.md", "fresh", 30);
        assert_eq!(setup.raised(), "fresh");
        let path = setup.shown()["path"].as_str().unwrap().to_owned();
        assert!(path.ends_with("/7/.priory/brief.md"), "{path}");
    }

    #[test]
    fn plan_of_an_earlier_generation_loses_to_the_new_generations_result() {
        let mut setup = Setup::new("generation");
        setup
            .db
            .execute_batch(
                "UPDATE runs SET started = '2026-01-01T00:00:00Z';
                 INSERT INTO runs (task, step, tmux, session, started, ended)
                 VALUES (1, 'groom', 't', 'lost', '2026-01-02T00:00:00Z', 'x');
                 INSERT INTO generations VALUES (1, 2);",
            )
            .unwrap();
        setup.write("plan.md", "of the lost run", 3600);
        setup.result("groom", "red", "the second generation");
        assert_eq!(setup.raised(), "the second generation");
    }

    #[test]
    fn plan_leads_no_ask_after_another_step() {
        let mut setup = Setup::new("merge");
        setup.write("plan.md", "the plan", 30);
        setup.result("risk", "low", "low risk");
        setup.raise();
        let asks = crate::listing::asks(&setup.db, &[], &[]).unwrap();
        assert_eq!(asks["asks"][0]["state"], "open");
        assert_eq!(asks["asks"][0]["approves_plan"], false);
        assert_eq!(setup.shown(), Value::Null);
    }

    #[test]
    fn plan_of_a_task_another_runner_holds_is_not_shown() {
        let mut setup = Setup::new("runner");
        setup.raise();
        let asks = crate::listing::asks(&setup.db, &[], &[]).unwrap();
        assert_eq!(asks["asks"][0]["approves_plan"], true);
        setup
            .db
            .execute_batch(
                "INSERT INTO runner VALUES (1, 'server');
                 INSERT INTO claims (key, runner, request, won, hold)
                 VALUES ('cli:7', 'server', '7', 1, 'queued');",
            )
            .unwrap();
        assert_eq!(setup.shown()["text"], "the groom summary");
        setup
            .db
            .execute("UPDATE claims SET runner = 'desktop'", [])
            .unwrap();
        assert_eq!(setup.shown(), Value::Null);
    }

    #[test]
    fn plan_skips_a_symlink_a_symlinked_folder_a_fifo_and_an_endless_file() {
        let setup = Setup::new("symlink");
        let (home, folder) = (&setup.home, setup.worktree.join(".priory"));
        fs::write(home.join("plan.md"), "outside").unwrap();
        std::os::unix::fs::symlink(home.join("plan.md"), folder.join("plan.md")).unwrap();
        assert!(read(&setup.worktree, FILES[0]).is_none());
        fs::write(folder.join("brief.md"), "inside").unwrap();
        assert!(read(&setup.worktree, FILES[1]).is_some());
        // A symlink inside the worktree is not followed either.
        fs::remove_file(folder.join("plan.md")).unwrap();
        std::os::unix::fs::symlink("brief.md", folder.join("plan.md")).unwrap();
        assert!(read(&setup.worktree, FILES[0]).is_none());
        fs::remove_file(folder.join("plan.md")).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(folder.join("plan.md"))
            .status()
            .unwrap();
        assert!(made.success());
        assert!(read(&setup.worktree, FILES[0]).is_none());
        fs::remove_file(folder.join("plan.md")).unwrap();
        File::create(folder.join("plan.md"))
            .unwrap()
            .set_len(LONGEST + 1)
            .unwrap();
        assert!(read(&setup.worktree, FILES[0]).is_none());
        fs::remove_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(home, &folder).unwrap();
        assert!(read(&setup.worktree, FILES[0]).is_none());
    }
}
