//! Reloads the factory's files while the daemon or the server runs. A poll walks the factory's
//! folder, and a change that held for one more poll loads and validates as `priory check` does.

use crate::trace::{self, Trace};
use priory_factory::factory::Factory;
use priory_factory::rules;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// How often a watch walks the factory's folder.
pub const EVERY: Duration = Duration::from_secs(1);

/// The trace record of a reload the daemon applied.
pub const RELOADED: &str = "factory.config.reloaded";
/// The trace record of a reload the daemon refused, keeping the last good factory.
pub const REFUSED: &str = "factory.config.refused";

/// Each file's size and modification time, by its path under the factory's folder.
type Fingerprint = BTreeMap<PathBuf, (u64, u128)>;

/// Watches one factory's folder for edits.
pub struct Watch {
    dir: PathBuf,
    seen: Fingerprint,
    pending: Option<Fingerprint>,
    polled: Instant,
}

/// What one poll found.
pub enum Polled {
    /// No edit, or one that has not yet held for a poll.
    Same,
    /// The edited factory, which passed every rule.
    Loaded(Box<Factory>),
    /// The edited factory's problems, each as `file:line: message`.
    Refused(Vec<String>),
}

impl Watch {
    /// A watch of `dir` as it is now, which the running factory was loaded from.
    pub fn new(dir: &Path) -> Watch {
        Watch {
            dir: dir.to_path_buf(),
            seen: fingerprint(dir),
            pending: None,
            polled: Instant::now(),
        }
    }

    /// Walks the folder once `EVERY` passed. An edit loads once it held for one poll, so a file an
    /// editor is still writing does not load half written. A refused edit is reported once, and
    /// again only after the next edit.
    pub fn poll(&mut self) -> Polled {
        if self.polled.elapsed() < EVERY {
            return Polled::Same;
        }
        self.polled = Instant::now();
        let now = fingerprint(&self.dir);
        if now == self.seen {
            self.pending = None;
            return Polled::Same;
        }
        if self.pending.as_ref() != Some(&now) {
            self.pending = Some(now);
            return Polled::Same;
        }
        self.pending = None;
        self.seen = now;
        match rules::validated(&self.dir) {
            Ok(factory) => Polled::Loaded(Box::new(factory)),
            Err(problems) => Polled::Refused(problems.iter().map(ToString::to_string).collect()),
        }
    }
}

/// Every file under `dir`, skipping hidden entries such as `.git` and not following a linked
/// folder. A file that vanishes mid-walk is left out.
fn fingerprint(dir: &Path) -> Fingerprint {
    let mut files = Fingerprint::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                folders.push(path);
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = metadata
                .modified()
                .ok()
                .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |at| at.as_nanos());
            files.insert(path, (metadata.len(), modified));
        }
    }
    files
}

/// The keys the daemon reads only when it starts, which `new` changes from `old`.
pub fn restart_only(old: &Factory, new: &Factory) -> Vec<&'static str> {
    let settings = |factory: &Factory, key: &str| -> String {
        let Some(settings) = factory.settings.as_ref().map(|settings| &settings.value) else {
            return String::new();
        };
        match key {
            "[serve]" => format!("{:?}", settings.serve),
            "[webhooks]" => format!("{:?}", settings.webhooks),
            "[log]" => format!("{:?}", settings.log),
            "[telemetry]" => format!("{:?}", settings.telemetry),
            "[notify]" => format!("{:?}", settings.notify),
            "[search]" => format!("{:?}", settings.search),
            "[kit]" => format!("{:?}", settings.kit),
            "[memories]" => format!("{:?}", settings.memories),
            "command_timeout" => format!("{:?}", settings.command_timeout),
            "[sweep] every" => format!("{:?}", settings.sweep.every),
            _ => unreachable!("a key of the list below"),
        }
    };
    let runners = |factory: &Factory| -> String {
        let runners: Vec<_> = factory
            .runners
            .iter()
            .map(|(name, runner)| (name, &runner.value))
            .collect();
        format!("{runners:?}")
    };
    let mut changed: Vec<&'static str> = [
        "[serve]",
        "[webhooks]",
        "[log]",
        "[telemetry]",
        "[notify]",
        "[search]",
        "[kit]",
        "[memories]",
        "command_timeout",
        "[sweep] every",
    ]
    .into_iter()
    .filter(|key| settings(old, key) != settings(new, key))
    .collect();
    if runners(old) != runners(new) {
        changed.push("runners/");
    }
    changed
}

/// Why `new` would strand a task not yet ended: it removes the task's workflow, or the step the
/// task is at. One line per task, naming the workflow's file.
pub fn stranded(db: &Connection, new: &Factory) -> rusqlite::Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT id, workflow, step FROM tasks
         WHERE state NOT IN ('done', 'failed', 'dropped') ORDER BY id",
    )?;
    let tasks = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut lines = Vec::new();
    for task in tasks {
        let (id, workflow, step) = task?;
        let file = format!("workflows/{workflow}.toml");
        match new.workflows.get(&workflow) {
            None => lines.push(format!(
                "{file}: task {id} is at step {step} of workflow {workflow}, which this edit \
                 removes: end or move task {id}, or keep the workflow"
            )),
            Some(loaded) if !loaded.value.steps.iter().any(|known| known.id == step) => {
                lines.push(format!(
                    "{file}: task {id} is at step {step}, which this edit removes: end or move \
                     task {id}, or keep the step"
                ))
            }
            Some(_) => {}
        }
    }
    Ok(lines)
}

/// Records a reload in the trace: with `Ok`, the keys that wait for a restart; with `Err`, the
/// problems that kept the last good factory. `trace` carries the revision now live.
pub fn record(
    db: &mut Connection,
    trace: &Trace,
    outcome: Result<&[&str], &[String]>,
) -> rusqlite::Result<()> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut data = json!({
        "factory": trace.factory.name,
        "factory_revision": trace.revision,
        "machine": trace.machine,
    });
    let kind = match outcome {
        Ok(restart) => {
            data["restart_only"] = json!(restart);
            RELOADED
        }
        Err(problems) => {
            data["problems"] = json!(problems);
            REFUSED
        }
    };
    if let serde_json::Value::Object(fields) = &mut data {
        fields.retain(|_, value| !value.is_null());
    }
    trace::record(&tx, trace, kind, data)?;
    tx.commit()
}

/// What the daemon prints for a poll's outcome: one line per problem, or the reload and the keys
/// that wait for a restart.
pub fn report(verb: &str, outcome: Result<&[&str], &[String]>) {
    match outcome {
        Ok(restart) => {
            eprintln!("priory {verb}: reloaded the factory");
            for key in restart {
                eprintln!("priory {verb}: {key} changed: restart priory {verb} to apply it");
            }
        }
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            eprintln!("priory {verb}: kept the last good factory");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let dir =
                std::env::temp_dir().join(format!("priory-reload-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_edit_loads_once_it_held_for_a_poll() {
        let dir = Scratch::new("held");
        std::fs::write(dir.path().join("factory.toml"), "name = \"a\"\n").unwrap();
        let mut watch = Watch::new(dir.path());
        std::fs::write(dir.path().join("factory.toml"), "name = \"bb\"\n").unwrap();
        watch.polled -= EVERY;
        assert!(matches!(watch.poll(), Polled::Same));
        watch.polled -= EVERY;
        assert!(!matches!(watch.poll(), Polled::Same));
        watch.polled -= EVERY;
        assert!(matches!(watch.poll(), Polled::Same));
    }

    #[test]
    fn hidden_entries_are_not_watched() {
        let dir = Scratch::new("hidden");
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "x").unwrap();
        std::fs::write(dir.path().join("factory.toml"), "").unwrap();
        let files = fingerprint(dir.path());
        assert_eq!(files.len(), 1);
    }
}
