//! The plan an approval ask shows: snapshotted when the ask is raised, and compared when it is
//! answered, so an answer approves only what the person read.

use priory_factory::factory::Step;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

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

/// What an ask that approves a plan says when it has none to show.
const NO_PLAN: &str =
    "This ask shows no plan: no groom or plan step of this generation ended with a result.";

/// The plan an ask shows.
struct Shown {
    /// The file, or `None` for the result's summary.
    path: Option<PathBuf>,
    text: String,
    truncated: bool,
}

impl Shown {
    fn new(path: Option<PathBuf>, bytes: &[u8]) -> Self {
        let mut end = bytes.len().min(SHOWN);
        while end < bytes.len() && end > 0 && bytes[end] & 0xC0 == 0x80 {
            end -= 1;
        }
        Shown {
            path,
            text: String::from_utf8_lossy(&bytes[..end]).into_owned(),
            truncated: end < bytes.len(),
        }
    }
}

/// A plan document, if the worktree holds it.
struct Document {
    written: jiff::Timestamp,
    path: PathBuf,
    bytes: Vec<u8>,
}

/// The content of each of `FILES`, as a hash: `None` for a missing one.
type Contents = [Option<String>; FILES.len()];

/// The plan an ask took when it was raised.
struct Taken {
    task: i64,
    step: String,
    run: i64,
    path: Option<String>,
    contents: Contents,
}

/// Whether `step`'s result is the task's plan. A debate's result is its agents' positions.
fn writes(step: &Step) -> bool {
    step.debate.is_none()
        && ([BRIEF_STEP, PLAN_STEP].contains(&step.id.as_str()) || step.id.starts_with(PLAN_PREFIX))
}

/// The step whose plan an ask at `target` approves: the author of that of the task's latest result,
/// when its outcome leads to `target`. The outer none is an ask that approves no plan.
pub(crate) fn approved_at<'a>(
    tx: &Transaction,
    task: i64,
    target: &str,
    step: impl Fn(&str) -> Option<&'a Step>,
) -> rusqlite::Result<Option<Option<String>>> {
    let latest = tx
        .query_row(
            "SELECT step, outcome FROM results WHERE task = ?1 ORDER BY run DESC LIMIT 1",
            [task],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((at, outcome)) = latest else {
        return Ok(None);
    };
    let approves = step(&at).is_some_and(|step| {
        step.next.get(&outcome).is_some_and(|next| next == target)
            && (writes(step) || crate::tools::GRADES.contains(&outcome.as_str()))
    });
    match approves {
        true => author(tx, task, &at, step).map(Some),
        false => Ok(None),
    }
}

/// The step that wrote the plan `at` presents: `at` itself, or, when `at` only grades it, the
/// latest one of this generation that ended with a result.
pub(crate) fn author<'a>(
    tx: &Transaction,
    task: i64,
    at: &str,
    step: impl Fn(&str) -> Option<&'a Step>,
) -> rusqlite::Result<Option<String>> {
    if step(at).is_some_and(writes) {
        return Ok(Some(at.to_owned()));
    }
    let mut ended = tx.prepare(
        "SELECT step FROM results WHERE task = ?1 AND run > coalesce(
           (SELECT max(after) FROM generations WHERE generations.task = ?1), 0)
         ORDER BY run DESC",
    )?;
    for ended in ended.query_map([task], |row| row.get::<_, String>(0))? {
        let ended = ended?;
        if step(&ended).is_some_and(writes) {
            return Ok(Some(ended));
        }
    }
    Ok(None)
}

/// Stores the plan of `author` as the one ask `ask` shows, or says on the ask that there is none.
pub(crate) fn present(
    tx: &Transaction,
    ask: i64,
    task: i64,
    author: Option<&str>,
) -> rusqlite::Result<()> {
    if let Some(step) = author {
        return snapshot(tx, ask, task, step, None);
    }
    tx.execute(
        "UPDATE asks SET text = CASE WHEN coalesce(text, '') = '' THEN ?2
           ELSE text || char(10, 10) || ?2 END WHERE id = ?1",
        params![ask, NO_PLAN],
    )
    .map(drop)
}

fn snapshot(
    tx: &Transaction,
    ask: i64,
    task: i64,
    step: &str,
    taken: Option<&Taken>,
) -> rusqlite::Result<()> {
    let Some(source) = source(tx, task, step)? else {
        return Ok(());
    };
    let taken = taken.filter(|taken| taken.run == source.run);
    let (documents, contents) = documents(source.worktree.as_deref());
    let shown = choose(source.started, documents, &contents, taken)
        .map(|document| Shown::new(Some(document.path), &document.bytes))
        .unwrap_or_else(|| Shown::new(None, source.summary.as_bytes()));
    tx.execute(
        "INSERT OR REPLACE INTO ask_plans (ask, step, run, path, text, truncated, contents)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            ask,
            step,
            source.run,
            shown.path.map(|path| path.to_string_lossy().into_owned()),
            shown.text,
            shown.truncated,
            serde_json::to_string(&contents).expect("hashes serialize")
        ],
    )
    .map(drop)
}

/// The document to show: one whose content changed since `taken`, or else the one it showed, or
/// else the newest written since `started`. So a rewrite never hides behind an old time.
fn choose(
    started: Option<jiff::Timestamp>,
    documents: [Option<Document>; FILES.len()],
    contents: &Contents,
    taken: Option<&Taken>,
) -> Option<Document> {
    let mut found: Vec<_> = documents
        .into_iter()
        .zip(contents)
        .enumerate()
        .filter_map(|(at, (document, now))| {
            let changed = taken.is_some_and(|taken| taken.contents[at] != *now);
            Some((changed, document?))
        })
        .collect();
    found.sort_by_key(|(_, document)| document.written);
    let mut pick = |wanted: &dyn Fn(bool, &Document) -> bool| {
        let at = found
            .iter()
            .rposition(|(changed, document)| wanted(*changed, document))?;
        Some(found.swap_remove(at).1)
    };
    let shown = taken.and_then(|taken| taken.path.as_deref());
    if let Some(document) = pick(&|changed, _| changed).or_else(|| {
        pick(&|_, document| shown.is_some_and(|shown| document.path.as_os_str() == shown))
    }) {
        return Some(document);
    }
    // `runs.started` holds whole seconds: a file of that second may be an earlier step's.
    let since = started?
        .checked_add(jiff::SignedDuration::from_secs(1))
        .ok()?;
    pick(&|_, document| document.written >= since)
}

/// Takes the plan of ask `ask` again, as when the ask is asked again. An ask with no plan keeps none.
pub(crate) fn again(tx: &Transaction, ask: i64) -> rusqlite::Result<()> {
    match of_ask(tx, ask)? {
        Some(taken) => snapshot(tx, ask, taken.task, &taken.step, Some(&taken)),
        None => Ok(()),
    }
}

/// Whether ask `ask` says that it shows no plan.
pub(crate) fn missing(tx: &Transaction, ask: i64) -> rusqlite::Result<bool> {
    tx.query_row(
        "SELECT coalesce(instr(text, ?2), 0) > 0 FROM asks WHERE id = ?1",
        params![ask, NO_PLAN],
        |row| row.get(0),
    )
}

/// Whether the plan ask `ask` shows is no longer the task's plan: either document's content or the
/// author's result changed. A retired task has no worktree to read, so what it showed stands.
pub(crate) fn changed(tx: &Transaction, ask: i64) -> rusqlite::Result<bool> {
    let Some(taken) = of_ask(tx, ask)? else {
        return Ok(false);
    };
    let retired: bool = tx.query_row(
        "SELECT retired IS NOT NULL FROM tasks WHERE id = ?1",
        [taken.task],
        |row| row.get(0),
    )?;
    if retired {
        return Ok(false);
    }
    let now = source(tx, taken.task, &taken.step)?;
    Ok(now.is_none_or(|now| {
        now.run != taken.run || documents(now.worktree.as_deref()).1 != taken.contents
    }))
}

fn of_ask(tx: &Transaction, ask: i64) -> rusqlite::Result<Option<Taken>> {
    tx.query_row(
        "SELECT asks.task, ask_plans.step, ask_plans.run, ask_plans.path, ask_plans.contents
         FROM ask_plans JOIN asks ON asks.id = ask_plans.ask WHERE asks.id = ?1",
        [ask],
        |row| {
            Ok(Taken {
                task: row.get(0)?,
                step: row.get(1)?,
                run: row.get(2)?,
                path: row.get(3)?,
                contents: serde_json::from_str(&row.get::<_, String>(4)?)
                    .expect("hashes are stored as JSON"),
            })
        },
    )
    .optional()
}

/// Where a step's plan comes from: its latest result, and its worktree's documents written since
/// the visit that ended with that result.
struct Source {
    run: i64,
    summary: String,
    worktree: Option<String>,
    started: Option<jiff::Timestamp>,
}

/// The source of the plan of `step`. A visit is the runs of the step since another step's run, an
/// earlier result or the task's generation began.
fn source(db: &Connection, task: i64, step: &str) -> rusqlite::Result<Option<Source>> {
    db.query_row(
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
            Ok(Source {
                run: row.get(0)?,
                summary: row.get(1)?,
                worktree: row.get(2)?,
                started: row
                    .get::<_, Option<String>>(3)?
                    .and_then(|started| started.parse().ok()),
            })
        },
    )
    .optional()
}

/// The documents `worktree` holds, and the content of each.
fn documents(worktree: Option<&str>) -> ([Option<Document>; FILES.len()], Contents) {
    let worktree = worktree.and_then(|worktree| Path::new(worktree).canonicalize().ok());
    let Some(worktree) = worktree else {
        return Default::default();
    };
    let found = FILES.map(|file| read(&worktree, file));
    let contents = std::array::from_fn(|at| match &found[at] {
        Some(document) => Some(hash(&document.bytes)),
        // A document Priory refuses to read still is not a missing one.
        None => std::fs::symlink_metadata(worktree.join(FILES[at]))
            .is_ok()
            .then(|| "unreadable".to_owned()),
    });
    (found, contents)
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `file` of `worktree`, if it is a regular file the worktree holds.
fn read(worktree: &Path, file: &str) -> Option<Document> {
    let path = worktree.join(file);
    let opened = open(&path)?;
    let found = opened.metadata().ok()?;
    if !found.is_file() || found.len() > LONGEST || !beneath(worktree, &path, &found) {
        return None;
    }
    let written = written(&found)?;
    let mut bytes = Vec::new();
    opened.take(LONGEST + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= LONGEST).then_some(Document {
        written,
        path,
        bytes,
    })
}

/// When a file last changed, which an agent cannot set as it can the modification time.
#[cfg(unix)]
fn written(found: &std::fs::Metadata) -> Option<jiff::Timestamp> {
    use std::os::unix::fs::MetadataExt;
    jiff::Timestamp::new(found.ctime(), found.ctime_nsec().try_into().ok()?).ok()
}

#[cfg(not(unix))]
fn written(_: &std::fs::Metadata) -> Option<jiff::Timestamp> {
    None
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
    use std::time::{Duration, SystemTime};

    /// A groom task whose groom run ended red a minute ago, with a worktree, ready at `approve-plan`.
    struct Setup {
        home: PathBuf,
        worktree: PathBuf,
        factory: Factory,
        db: Connection,
    }

    fn trace(factory: &Factory) -> Trace<'_> {
        Trace {
            factory,
            revision: None,
            machine: None,
            runner: None,
            usage: priory_runtime::transcript::usage,
        }
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

        /// Sets the start of every run so far to the second of `file`'s last change, plus `seconds`.
        fn started_at(&self, file: &str, seconds: i64) {
            let found = fs::metadata(self.worktree.join(".priory").join(file)).unwrap();
            let at = written(&found).unwrap().as_second() + seconds;
            self.db
                .execute(
                    "UPDATE runs SET started = strftime('%Y-%m-%dT%H:%M:%SZ', ?1, 'unixepoch')",
                    [at],
                )
                .unwrap();
        }

        /// Adds a copy of the groom step named `id`, as a step that only grades the plan.
        fn grader(&mut self, id: &str, debate: bool) {
            let flow = &mut self.factory.workflows.get_mut("groom").unwrap().value;
            let mut step = json!({
                "id": id, "decides": "model", "effect": "read", "prompt": "Grade the plan.",
                "next": { "red": "approve-plan", "yellow": "implement", "green": "implement" },
            });
            if debate {
                step["debate"] = json!({ "agents": ["a", "b"], "rounds": 1, "cap": "$1.00" });
            }
            let step = serde_json::from_value(step).unwrap();
            flow.steps.push(step);
        }

        /// A lead's live run, called for the open ask, with the session `lead`.
        fn lead(&self) {
            self.db
                .execute_batch(
                    r#"UPDATE asks SET routed = 'lead';
                     INSERT INTO tasks (id, key, workflow, step, state, band, lane, lane_derived,
                       trigger, source, entered)
                     VALUES (2, 'priory:ask:1:1', 'groom', 'groom', 'running', 'now', 'cli', 0,
                       'priory/lead', 'called', 'x');
                     INSERT INTO inbox (id, type, time, data, links, task, outcome)
                     VALUES ('called', 'priory.ask.routed', 'x', '{"ask":1,"revision":1}', '[]', 2,
                       'created');
                     INSERT INTO runs (task, step, agent, tmux, session)
                     VALUES (2, 'groom', 'lead', 't', 'lead');"#,
                )
                .unwrap();
        }

        fn refusals(&self) -> i64 {
            self.db
                .query_row("SELECT count(*) FROM ask_refusals", [], |row| row.get(0))
                .unwrap()
        }

        /// Asserts that the ask refused its first answer and shows `text` as revision 2.
        fn refused(&self, text: &str) {
            let shown = self.shown();
            assert_eq!(
                (&shown["text"], &shown["revision"]),
                (&json!(text), &json!(2))
            );
            assert_eq!((self.step().as_str(), self.refusals()), ("approve-plan", 1));
        }

        fn text(&self) -> String {
            self.db
                .query_row("SELECT coalesce(text, '') FROM asks", [], |row| row.get(0))
                .unwrap()
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
            let trace = trace(&self.factory);
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
        setup.write("plan.md", "left over", 0);
        setup.started_at("plan.md", 3600);
        assert_eq!(setup.raised(), "the groom summary");
        assert_eq!(setup.shown()["path"], Value::Null);
    }

    #[test]
    fn plan_left_over_loses_to_a_fresh_brief() {
        let mut setup = Setup::new("fresh-brief");
        setup.write("plan.md", "left over", 0);
        setup.started_at("plan.md", 0);
        std::thread::sleep(Duration::from_millis(1100));
        setup.write("brief.md", "fresh", 0);
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
        setup.write("plan.md", "of the lost run", 0);
        setup.result("groom", "red", "the second generation");
        setup
            .db
            .execute(
                "UPDATE runs SET started = '2999-01-01T00:00:00Z' WHERE id = 3",
                [],
            )
            .unwrap();
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

    #[test]
    fn plan_graded_by_another_step_is_the_author_steps_file_and_binds_its_content() {
        for debate in [false, true] {
            let mut setup = Setup::new(&format!("grade-{debate}"));
            setup.grader("grade", debate);
            setup.write("brief.md", "as read", 300);
            setup.result("grade", "red", "the grade summary");
            assert_eq!(setup.raised(), "as read");

            setup.write("brief.md", "rewritten", 3600);

            setup.answer(1).unwrap_err();
            setup.refused("rewritten");
            setup.answer(2).unwrap();
            assert_eq!(setup.step(), "implement");
        }
    }

    #[test]
    fn plan_of_a_debate_named_as_a_plan_step_is_the_author_steps() {
        let mut setup = Setup::new("plan-debate");
        setup.grader("plan-debate", true);
        setup.write("brief.md", "as read", 0);
        setup.result("plan-debate", "red", "both agents chose red");
        assert_eq!(setup.raised(), "as read");
    }

    #[test]
    fn plan_a_split_debate_grades_shows_on_the_persons_pick() {
        let mut setup = Setup::new("split");
        setup.grader("grade", true);
        setup.write("brief.md", "as read", 0);
        setup
            .db
            .execute_batch(
                "UPDATE tasks SET step = 'grade', state = 'running';
                 INSERT INTO debates (task, step) VALUES (1, 'grade');
                 INSERT INTO runs (task, step, agent, tmux, session, debate)
                 VALUES (1, 'grade', 'a', 't', 'split', 1);",
            )
            .unwrap();
        let trace = trace(&setup.factory);
        let tx = setup.db.transaction().unwrap();
        let run = tools::live_run(&tx, "split").unwrap().unwrap();
        crate::debate::stop(&tx, &trace, &run, 1, "the rounds ran out").unwrap();
        tx.commit().unwrap();
        assert_eq!(setup.shown()["text"], "as read");
        setup.write("brief.md", "rewritten", 3600);
        let answer = Answer {
            ask: 1,
            text: "red".to_owned(),
            revision: 1,
            run: false,
            by: "cli".to_owned(),
        };
        let trace = trace(&setup.factory);
        tools::answer(&mut setup.db, &answer, &trace, &|_, _| None)
            .unwrap()
            .unwrap_err();
        assert_eq!(setup.shown()["text"], "rewritten");
    }

    #[test]
    fn plan_ask_with_no_author_in_this_generation_says_that_it_shows_none() {
        let mut setup = Setup::new("no-author");
        setup.grader("grade", false);
        setup.write("brief.md", "of an earlier generation", 0);
        setup
            .db
            .execute("INSERT INTO generations VALUES (1, 1)", [])
            .unwrap();
        setup.result("grade", "red", "the grade summary");
        setup.raise();
        assert_eq!(setup.shown(), Value::Null);
        assert!(setup.text().ends_with(NO_PLAN), "{}", setup.text());
        setup.answer(1).unwrap();
    }

    #[test]
    fn plan_document_that_appears_after_a_summary_was_shown_refuses_the_answer() {
        let mut setup = Setup::new("appears");
        assert_eq!(setup.raised(), "the groom summary");
        setup.write("plan.md", "unseen", 3600);
        setup.answer(1).unwrap_err();
        setup.refused("unseen");
    }

    #[test]
    fn plan_not_shown_that_is_rewritten_refuses_the_answer_and_is_shown() {
        let mut setup = Setup::new("other");
        setup.write("plan.md", "other", 0);
        std::thread::sleep(Duration::from_millis(20));
        setup.write("brief.md", "as read", 0);
        assert_eq!(setup.raised(), "as read");
        setup.write("plan.md", "unseen", 3600);
        setup.answer(1).unwrap_err();
        setup.refused("unseen");
    }

    #[test]
    fn plan_touched_but_not_edited_is_answered() {
        let mut setup = Setup::new("touched");
        setup.write("brief.md", "as read", 30);
        setup.raise();
        setup.write("brief.md", "as read", 0);
        setup.answer(1).unwrap();
    }

    #[test]
    fn plan_written_in_the_second_its_run_started_is_an_earlier_steps() {
        let mut setup = Setup::new("second");
        setup.write("plan.md", "the earlier step's", 0);
        setup.started_at("plan.md", 0);
        assert_eq!(setup.raised(), "the groom summary");
        let mut setup = Setup::new("next-second");
        setup.write("plan.md", "this step's", 0);
        setup.started_at("plan.md", -1);
        assert_eq!(setup.raised(), "this step's");
    }

    #[test]
    fn plan_of_an_open_ask_shows_before_that_of_a_later_answered_one() {
        let mut setup = Setup::new("open-first");
        setup.write("brief.md", "open", 0);
        setup.raise();
        setup
            .db
            .execute_batch(
                "INSERT INTO asks (id, task, step, kind, question, options, answer)
                 VALUES (9, 1, 'approve-plan', 'step', 'q', '[]', 'approved');
                 INSERT INTO ask_plans SELECT 9, step, run, path, 'answered', truncated, contents
                 FROM ask_plans;",
            )
            .unwrap();
        assert_eq!(setup.shown()["text"], "open");
    }

    #[test]
    fn plan_changed_before_a_leads_answer_returns_the_ask_to_the_person_once() {
        let mut setup = Setup::new("lead");
        setup.write("brief.md", "as read", 30);
        setup.raise();
        setup.lead();
        setup.write("brief.md", "rewritten", 3600);
        let trace = trace(&setup.factory);
        for refusals in 1..=2 {
            let call = tools::Call {
                session: "lead".to_owned(),
                tool: "result".to_owned(),
                arguments: json!({ "outcome": "answered", "summary": "approved" }),
            };
            let kit = crate::kit::Current {
                kit: None,
                memories: None,
            };
            let (stored, _) = tools::call(&mut setup.db, &call, &trace, kit, &|_, _| None).unwrap();
            let refused = stored.unwrap_err();
            assert!(refused.contains("end with skipped"), "{refused}");
            if refusals == 1 {
                setup.refused("rewritten");
            }
            assert_eq!(setup.shown()["revision"], 2);
            assert_eq!(setup.refusals(), refusals);
        }
        let routed: Option<String> = setup
            .db
            .query_row("SELECT routed FROM asks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(routed, None);
    }

    #[test]
    fn plan_changed_before_an_outside_sessions_answer_is_the_next_revision() {
        let mut setup = Setup::new("outside");
        setup.write("brief.md", "as read", 30);
        setup.raise();
        setup.write("brief.md", "rewritten", 3600);
        let trace = trace(&setup.factory);
        let call = crate::outside::Call {
            tool: "answer".to_owned(),
            session: Some("desk".to_owned()),
            arguments: json!({ "ask": 1, "revision": 1, "answer": "approved" }),
        };
        let refused = crate::outside::decide(&mut setup.db, &call, &trace, &|_, _| None)
            .unwrap()
            .unwrap_err();
        assert!(refused.contains("changed since revision 1"), "{refused}");
        setup.refused("rewritten");
    }

    #[test]
    fn plan_changed_before_another_runners_answer_is_the_next_revision() {
        let mut setup = Setup::new("remote");
        setup.write("brief.md", "as read", 30);
        setup.raise();
        setup.write("brief.md", "rewritten", 3600);
        let trace = trace(&setup.factory);
        let tx = setup.db.transaction().unwrap();
        let sent = herdr_bus::Message::new(
            priory_intake::intake::ASK_ANSWERED,
            json!({ "runner": "server", "ask": 1, "text": "approved", "revision": 1, "run": false,
              "by": "desk" }),
        );
        let outcome = crate::runners::apply(&tx, &sent, "server", &trace).unwrap();
        assert_eq!(outcome, "refused");
        tx.commit().unwrap();
        setup.refused("rewritten");
    }

    #[test]
    fn plan_of_a_routed_ask_is_in_its_leads_event() {
        let mut setup = Setup::new("routed");
        setup.write("brief.md", "as read", 0);
        setup.raise();
        let tx = setup.db.transaction().unwrap();
        tools::call_lead(&tx, 1, "lead").unwrap();
        let plan: Value = tx
            .query_row(
                "SELECT json_extract(data, '$.plan') FROM inbox WHERE type = 'priory.ask.routed'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map(|plan| serde_json::from_str(&plan).unwrap())
            .unwrap();
        assert_eq!(
            (&plan["text"], &plan["truncated"]),
            (&json!("as read"), &json!(false))
        );
        assert!(plan["path"].as_str().unwrap().ends_with("/brief.md"));
    }
}
