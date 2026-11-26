//! What each live run uses now, measured from its systemd scope's cgroup, and what each run used
//! in all, stored in `run_usage` as it runs and when it ends.

use priory_runtime::{cgroup, runtime, slice};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long a run whose pane tmux did not find waits before tmux is asked again, so a dead pane
/// costs one `tmux list-panes` per run every 30 seconds, not one per room check.
const RETRY: Duration = Duration::from_secs(30);

/// What tmux said about a live run's pane.
#[derive(Clone, Copy)]
enum Pane {
    /// The pane's process and its start time, so a pid the kernel reused is never read.
    Found { pid: u32, started: i64 },
    /// No live pane when tmux was asked.
    Missing { at: Instant },
}

/// Each live run's pane, by run id and harness session. The scope execs the agent in place, so the
/// process lasts as long as the run's agent.
static PANES: Mutex<BTreeMap<(i64, String), Pane>> = Mutex::new(BTreeMap::new());

fn panes() -> std::sync::MutexGuard<'static, BTreeMap<(i64, String), Pane>> {
    PANES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The memory run `run` holds now that measured free memory does not count as free, in bytes: its
/// scope's anonymous and shared memory. None when the run has no scope inside the agents slice, as
/// on a host without systemd, or when it cannot be read.
pub(crate) fn memory(run: i64, tmux: &str, session: &str) -> Option<u64> {
    // macOS has no cgroup to read.
    if cfg!(target_os = "macos") {
        return None;
    }
    let key = (run, session.to_owned());
    let cached = panes().get(&key).copied();
    let pid = match cached {
        Some(Pane::Found { pid, started }) if crate::memory::start_time(pid) == Some(started) => {
            pid
        }
        Some(Pane::Missing { at }) if at.elapsed() < RETRY => return None,
        // The lock is not held while tmux runs.
        _ => {
            let found = runtime::pane_pid(tmux, session)
                .and_then(|pid| Some((pid, crate::memory::start_time(pid)?)));
            let pane = match found {
                Some((pid, started)) => Pane::Found { pid, started },
                None => Pane::Missing { at: Instant::now() },
            };
            panes().insert(key, pane);
            found?.0
        }
    };
    let scope = cgroup::scope_of(pid, &slice::name())?;
    ledger().observe(run, cgroup::sample(&scope));
    cgroup::memory(&scope)
}

/// Forgets the pane of every run that is not in `live`, so an ended run's pid is never read again,
/// and stores what each run used.
pub(crate) fn keep(db: &Connection, live: &[(i64, String)]) -> rusqlite::Result<()> {
    panes().retain(|key, _| live.contains(key));
    ledger().keep(db, &live.iter().map(|(run, _)| *run).collect())
}

/// How often a live run's sample is written, so a daemon restart loses at most this much.
const WRITE_EVERY: Duration = Duration::from_secs(15);
/// How long the walk of an ended run's worktree may take before its disk is left unmeasured.
const WALK_LIMIT: Duration = Duration::from_secs(60);

static LEDGER: LazyLock<Mutex<Ledger>> = LazyLock::new(Mutex::default);

fn ledger() -> std::sync::MutexGuard<'static, Ledger> {
    LEDGER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A run's last sample, with its memory the highest seen, and when it was last written.
struct Kept {
    sample: cgroup::Sample,
    written: Option<Instant>,
}

/// What the live runs used so far, and the ended runs whose worktree is being measured.
#[derive(Default)]
pub(crate) struct Ledger {
    kept: BTreeMap<i64, Kept>,
    live: BTreeSet<i64>,
    walked: Arc<Mutex<Vec<(i64, Option<u64>)>>>,
}

impl Ledger {
    /// Takes run `run`'s new sample: the last CPU and IO, and the highest memory. A failed read, as
    /// of a scope already collected, keeps the last sample.
    pub(crate) fn observe(&mut self, run: i64, sample: Option<cgroup::Sample>) {
        let Some(new) = sample else { return };
        let kept = self.kept.entry(run).or_insert(Kept {
            sample: new,
            written: None,
        });
        let old = kept.sample;
        kept.sample = cgroup::Sample {
            cpu_usec: new.cpu_usec.or(old.cpu_usec),
            user_usec: new.user_usec.or(old.user_usec),
            system_usec: new.system_usec.or(old.system_usec),
            memory: new.memory.max(old.memory),
            io_read: new.io_read.or(old.io_read),
            io_write: new.io_write.or(old.io_write),
        };
    }

    /// Writes each live run's sample every `WRITE_EVERY`, and an ended run's last one. Starts the
    /// walk of an ended run's worktree, and stores the walks that finished.
    pub(crate) fn keep(&mut self, db: &Connection, live: &BTreeSet<i64>) -> rusqlite::Result<()> {
        let walked = std::mem::take(&mut *self.walked.lock().unwrap_or_else(|p| p.into_inner()));
        for (run, disk) in walked {
            db.execute(
                &format!(
                    "INSERT INTO run_usage (run, disk, measured) VALUES (?1, ?2, {NOW})
                     ON CONFLICT (run) DO UPDATE SET disk = ?2, measured = {NOW}"
                ),
                params![run, disk.map(|disk| disk as i64)],
            )?;
        }
        for (run, kept) in &mut self.kept {
            let due = kept.written.is_none_or(|at| at.elapsed() >= WRITE_EVERY);
            if !live.contains(run) || due {
                write(db, *run, &kept.sample)?;
                kept.written = Some(Instant::now());
            }
        }
        let ended: Vec<i64> = self.live.difference(live).copied().collect();
        self.kept.retain(|run, _| live.contains(run));
        for run in ended {
            let worktree: Option<String> = db
                .query_row(
                    "SELECT tasks.worktree FROM runs JOIN tasks ON tasks.id = runs.task
                     WHERE runs.id = ?1",
                    [run],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let walked = Arc::clone(&self.walked);
            let worktree = worktree.map(PathBuf::from).filter(|path| path.is_dir());
            std::thread::spawn(move || {
                let disk = worktree.and_then(|path| size(&path, Instant::now() + WALK_LIMIT));
                walked
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push((run, disk));
            });
        }
        self.live.clone_from(live);
        Ok(())
    }
}

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%SZ', 'now')";

fn write(db: &Connection, run: i64, sample: &cgroup::Sample) -> rusqlite::Result<()> {
    let int = |value: Option<u64>| value.and_then(|value| i64::try_from(value).ok());
    db.execute(
        "INSERT INTO run_usage (run, cpu_usec, user_usec, system_usec, memory_peak, io_read, io_write)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (run) DO UPDATE SET cpu_usec = ?2, user_usec = ?3, system_usec = ?4,
           memory_peak = ?5, io_read = ?6, io_write = ?7",
        params![
            run,
            int(sample.cpu_usec),
            int(sample.user_usec),
            int(sample.system_usec),
            int(sample.memory),
            int(sample.io_read),
            int(sample.io_write)
        ],
    )?;
    Ok(())
}

/// The bytes of every file under `root`, as `du -sb` counts them, on `root`'s filesystem, without
/// following links. None past `deadline`, or when a folder cannot be read, as one a teardown
/// removes during the walk.
fn size(root: &Path, deadline: Instant) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let device = std::fs::symlink_metadata(root).ok()?.dev();
    let mut total = 0u64;
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        if Instant::now() > deadline {
            return None;
        }
        for entry in std::fs::read_dir(&folder).ok()? {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            total = total.saturating_add(metadata.len());
            if metadata.is_dir() && metadata.dev() == device {
                folders.push(entry.path());
            }
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str =
        "/user.slice/user@1000.service/priory.slice/priory-agents.slice/run-r1.scope";

    /// A fake cgroup tree and a worktree in a fresh folder, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("priory-usage-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("cgroup").join(SCOPE.trim_start_matches('/')))
                .unwrap();
            std::fs::create_dir_all(root.join("worktree/src")).unwrap();
            Self(root)
        }

        fn scope(&self) -> PathBuf {
            self.0.join("cgroup").join(SCOPE.trim_start_matches('/'))
        }

        /// Writes the scope's files as the kernel shows them at one moment.
        fn at(&self, cpu: u64, peak: u64, read: u64, written: u64) {
            let scope = self.scope();
            std::fs::write(
                scope.join("cpu.stat"),
                format!(
                    "usage_usec {cpu}\nuser_usec {}\nsystem_usec {}\n",
                    cpu - cpu / 4,
                    cpu / 4
                ),
            )
            .unwrap();
            std::fs::write(scope.join("memory.peak"), format!("{peak}\n")).unwrap();
            std::fs::write(
                scope.join("io.stat"),
                format!("8:0 rbytes={read} wbytes={written} rios=1 wios=1\n"),
            )
            .unwrap();
        }

        fn sample(&self) -> Option<cgroup::Sample> {
            cgroup::sample_in(&self.0.join("cgroup"), SCOPE)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn one_run(worktree: &Path) -> Connection {
        let db = crate::state::memory();
        db.execute(
            "INSERT INTO tasks (id, key, workflow, step, state, trigger, source, worktree)
             VALUES (1, 'k', 'build', 'implement', 'running', 't', 'e', ?1)",
            [worktree.to_str().unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO runs (id, task, step, tmux, session) VALUES (7, 1, 'implement', 'm', 'n')",
            [],
        )
        .unwrap();
        db
    }

    type Row = (
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );

    fn stored(db: &Connection) -> Option<Row> {
        db.query_row(
            "SELECT cpu_usec, user_usec, system_usec, memory_peak, io_read, io_write, disk
             FROM run_usage WHERE run = 7",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()
        .unwrap()
    }

    /// Ends run 7 and waits for its worktree's walk to be stored.
    fn end(ledger: &mut Ledger, db: &Connection) {
        db.execute("UPDATE runs SET ended = '2026-11-26T12:00:00Z'", [])
            .unwrap();
        ledger.keep(db, &BTreeSet::new()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while db
            .query_row("SELECT measured FROM run_usage WHERE run = 7", [], |row| {
                row.get::<_, Option<String>>(0)
            })
            .optional()
            .unwrap()
            .flatten()
            .is_none()
        {
            assert!(Instant::now() < deadline, "the walk never finished");
            std::thread::sleep(Duration::from_millis(10));
            ledger.keep(db, &BTreeSet::new()).unwrap();
        }
    }

    #[test]
    fn samples_keep_the_peak_and_the_last_cpu_and_io() {
        let tree = Tree::new("peak");
        std::fs::write(tree.0.join("worktree/src/main.rs"), [0u8; 1000]).unwrap();
        let db = one_run(&tree.0.join("worktree"));
        let mut ledger = Ledger::default();
        let live = BTreeSet::from([7]);

        tree.at(4_000_000, 900 << 20, 10, 20);
        ledger.observe(7, tree.sample());
        ledger.keep(&db, &live).unwrap();
        // A lower memory.peak than seen, as after a reset, keeps the higher one.
        tree.at(8_000_000, 300 << 20, 30, 40);
        ledger.observe(7, tree.sample());
        ledger.keep(&db, &live).unwrap();
        end(&mut ledger, &db);

        let (cpu, user, system, peak, read, written, disk) = stored(&db).unwrap();
        assert_eq!(
            (cpu, user, system, peak, read, written),
            (
                Some(8_000_000),
                Some(6_000_000),
                Some(2_000_000),
                Some(900 << 20),
                Some(30),
                Some(40)
            )
        );
        // The file's bytes and its two folders' entries.
        assert!(disk.is_some_and(|disk| disk >= 1000), "{disk:?}");
    }

    #[test]
    fn a_scope_that_vanished_keeps_the_last_sample() {
        let tree = Tree::new("vanished");
        let db = one_run(&tree.0.join("worktree"));
        let mut ledger = Ledger::default();
        let live = BTreeSet::from([7]);

        tree.at(5_000_000, 1 << 30, 1, 2);
        ledger.observe(7, tree.sample());
        ledger.keep(&db, &live).unwrap();
        // systemd collects the scope as the agent exits.
        std::fs::remove_dir_all(tree.scope()).unwrap();
        ledger.observe(7, tree.sample());
        end(&mut ledger, &db);

        let (cpu, _, _, peak, read, written, _) = stored(&db).unwrap();
        assert_eq!(
            (cpu, peak, read, written),
            (Some(5_000_000), Some(1 << 30), Some(1), Some(2))
        );
    }

    #[test]
    fn a_run_outside_a_scope_stores_nulls() {
        let tree = Tree::new("outside");
        std::fs::remove_dir_all(tree.0.join("worktree")).unwrap();
        let db = one_run(&tree.0.join("worktree"));
        let mut ledger = Ledger::default();

        ledger.observe(7, None);
        ledger.keep(&db, &BTreeSet::from([7])).unwrap();
        end(&mut ledger, &db);

        assert_eq!(
            stored(&db),
            Some((None, None, None, None, None, None, None))
        );
    }

    #[test]
    fn a_scope_without_io_or_memory_peak_falls_back() {
        let tree = Tree::new("fallback");
        let scope = tree.scope();
        std::fs::write(
            scope.join("cpu.stat"),
            "usage_usec 10\nuser_usec 6\nsystem_usec 4\n",
        )
        .unwrap();
        std::fs::write(scope.join("memory.stat"), "anon 100\nfile 900\nshmem 5\n").unwrap();

        let sample = tree.sample().unwrap();

        assert_eq!(
            (sample.memory, sample.io_read, sample.io_write),
            (Some(105), None, None)
        );
    }
}
