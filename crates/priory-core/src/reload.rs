//! Reloads the factory's files while the daemon or the server runs. A poll walks the files
//! `factory::load` reads, and a change that held for one more poll loads and validates as
//! `priory check` does.

use crate::trace::{self, Trace};
use priory_factory::factory::Factory;
use priory_factory::rules;
use rusqlite::{Connection, TransactionBehavior};
use serde_json::json;
use sha2::{Digest as _, Sha256};
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

/// A hash of the bytes of every file a fingerprint names.
pub type Digest = [u8; 32];

/// The factory's files and folders `factory::load` reads, besides the `writing` file.
const FILES: &[&str] = &[
    "factory.toml",
    "factory.yaml",
    "factory.yml",
    "lanes.toml",
    "lanes.yaml",
    "lanes.yml",
    "prices.toml",
    "prices.yaml",
    "prices.yml",
    priory_factory::factory::INSTRUCTIONS,
];
/// The folders whose top-level TOML and YAML files `factory::load` reads.
const CONFIG_FOLDERS: &[&str] = &[
    "triggers",
    "workflows",
    "views",
    "scorers",
    "benchmarks",
    "runners",
];
/// The folders `factory::load` reads whole: an agent's, a skill's and a memory's files.
const TREES: &[&str] = &["agents", "skills", "memories"];

/// Watches one factory's folder for edits.
pub struct Watch {
    dir: PathBuf,
    /// The guide `writing` names, read from the live factory.
    writing: Option<PathBuf>,
    seen: Fingerprint,
    pending: Option<Fingerprint>,
    /// The bytes of the files the live factory was loaded from.
    live: Digest,
    polled: Instant,
    /// The git reflog of the factory's `HEAD`, and when it last changed.
    head: Option<(PathBuf, Option<u128>)>,
    committed: bool,
}

/// What one poll found.
pub enum Polled {
    /// No edit, or one that has not yet held for a poll.
    Same,
    /// An edit that left the files as the live factory read them, such as a touch or an undo.
    Current,
    /// The edited factory, which passed every rule, and the digest of its files.
    Loaded(Box<Factory>, Digest),
    /// The edited factory's problems, each as `file:line: message`.
    Refused(Vec<String>),
}

impl Watch {
    /// A watch of the files `factory` was loaded from, as they are now.
    pub fn new(factory: &Factory) -> Watch {
        let dir = factory.dir.clone();
        let writing = writing(factory);
        let seen = fingerprint(&dir, writing.as_deref());
        let head = reflog(&dir).map(|path| {
            let modified = modified(&path);
            (path, modified)
        });
        Watch {
            live: digest(&seen),
            dir,
            writing,
            seen,
            pending: None,
            polled: Instant::now(),
            head,
            committed: false,
        }
    }

    /// Walks the factory's files once `EVERY` passed. An edit loads once it held for one poll, so
    /// a file an editor is still writing does not load half written. A refused edit is reported
    /// once, and again only after the next edit.
    pub fn poll(&mut self) -> Polled {
        if self.polled.elapsed() < EVERY {
            return Polled::Same;
        }
        self.polled = Instant::now();
        if let Some((path, seen)) = &mut self.head {
            let now = modified(path);
            if now != *seen {
                *seen = now;
                self.committed = true;
            }
        }
        let now = fingerprint(&self.dir, self.writing.as_deref());
        if now == self.seen {
            self.pending = None;
            return Polled::Same;
        }
        if self.pending.as_ref() != Some(&now) {
            self.pending = Some(now);
            return Polled::Same;
        }
        self.pending = None;
        let files = digest(&now);
        self.seen = now;
        if files == self.live {
            return Polled::Current;
        }
        match rules::validated(&self.dir) {
            Ok(factory) => Polled::Loaded(Box::new(factory), files),
            Err(problems) => Polled::Refused(problems.iter().map(ToString::to_string).collect()),
        }
    }

    /// Records that `factory`, whose files hash to `files`, is now live.
    pub fn applied(&mut self, factory: &Factory, files: Digest) {
        self.live = files;
        let writing = writing(factory);
        if writing != self.writing {
            self.writing = writing;
            self.seen = fingerprint(&self.dir, self.writing.as_deref());
        }
    }

    /// Whether the factory's git `HEAD` moved since the last call, as a commit or a checkout does.
    pub fn committed(&mut self) -> bool {
        std::mem::take(&mut self.committed)
    }
}

/// The guide `writing` names, under the factory's folder.
fn writing(factory: &Factory) -> Option<PathBuf> {
    let settings = factory.settings.as_ref()?;
    Some(factory.dir.join(settings.value.writing.as_ref()?))
}

/// The reflog git appends to each time the factory's `HEAD` moves; none outside a repository.
fn reflog(dir: &Path) -> Option<PathBuf> {
    let output = priory_runtime::runtime::git(dir)
        .args(["rev-parse", "--git-path", "logs/HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let path = String::from_utf8(output.stdout).ok()?;
    Some(dir.join(path.trim()))
}

fn modified(path: &Path) -> Option<u128> {
    let at = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(at.duration_since(UNIX_EPOCH).ok()?.as_nanos())
}

/// Whether a walk skips `name`: hidden entries such as `.git`, editor backups and the folders
/// tools fill, which no factory file is.
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || name.ends_with('~')
        || (name.starts_with('#') && name.ends_with('#'))
        || matches!(name, "__pycache__" | "node_modules")
}

/// The files `factory::load` reads under `dir`, with `writing`. It does not follow a linked folder,
/// and leaves out a file that vanishes mid-walk.
fn fingerprint(dir: &Path, writing: Option<&Path>) -> Fingerprint {
    let mut files = Fingerprint::new();
    let mut note = |path: PathBuf| {
        if let Ok(metadata) = std::fs::metadata(&path)
            && metadata.is_file()
        {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |at| at.as_nanos());
            files.insert(path, (metadata.len(), modified));
        }
    };
    for name in FILES {
        note(dir.join(name));
    }
    if let Some(writing) = writing {
        note(writing.to_path_buf());
    }
    for folder in CONFIG_FOLDERS {
        let Ok(entries) = std::fs::read_dir(dir.join(folder)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let config = matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("toml" | "yaml" | "yml")
            );
            if config && !skipped(&entry.file_name().to_string_lossy()) {
                note(path);
            }
        }
    }
    let mut folders: Vec<PathBuf> = TREES.iter().map(|tree| dir.join(tree)).collect();
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            if skipped(&entry.file_name().to_string_lossy()) {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(entry.path()),
                Ok(_) => note(entry.path()),
                Err(_) => {}
            }
        }
    }
    files
}

/// A hash of each file's path and bytes, in path order. A file that cannot be read hashes as empty.
fn digest(files: &Fingerprint) -> Digest {
    let mut hash = Sha256::new();
    for path in files.keys() {
        hash.update(path.as_os_str().as_encoded_bytes());
        hash.update([0]);
        let bytes = std::fs::read(path).unwrap_or_default();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    hash.finalize().into()
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
    if old.name != new.name {
        changed.insert(0, "name");
    }
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

    fn factory(dir: &Path) -> Factory {
        Factory {
            dir: dir.to_path_buf(),
            ..Factory::default()
        }
    }

    /// Polls as if `EVERY` passed since the last poll.
    fn poll(watch: &mut Watch) -> Polled {
        watch.polled -= EVERY;
        watch.poll()
    }

    #[test]
    fn an_edit_loads_once_it_held_for_a_poll() {
        let dir = Scratch::new("held");
        std::fs::write(dir.path().join("factory.toml"), "name = \"a\"\n").unwrap();
        let mut watch = Watch::new(&factory(dir.path()));
        std::fs::write(dir.path().join("factory.toml"), "name = \"bb\"\n").unwrap();
        assert!(matches!(poll(&mut watch), Polled::Same));
        assert!(matches!(poll(&mut watch), Polled::Loaded(..)));
        assert!(matches!(poll(&mut watch), Polled::Same));
    }

    #[test]
    fn a_rewrite_that_keeps_the_live_bytes_loads_nothing() {
        let dir = Scratch::new("current");
        let file = dir.path().join("factory.toml");
        std::fs::write(&file, "name = \"a\"\n").unwrap();
        let mut watch = Watch::new(&factory(dir.path()));
        std::fs::write(&file, "name = \"b\"\n").unwrap();
        std::fs::write(&file, "name = \"a\"\n").unwrap();
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(matches!(poll(&mut watch), Polled::Same));
        assert!(matches!(poll(&mut watch), Polled::Current));
    }

    #[test]
    fn only_the_files_the_loader_reads_are_watched() {
        let dir = Scratch::new("read");
        let files = [
            "factory.toml",
            "README.md",
            ".git/HEAD",
            "notes/plan.md",
            "workflows/a.toml",
            "workflows/a.toml~",
            "workflows/#a.toml#",
            "workflows/a.md",
            "agents/x/agent.md",
            "agents/x/__pycache__/y.pyc",
            "skills/s/node_modules/m.js",
            "skills/s/SKILL.md",
            "guide.md",
        ];
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        let watched = fingerprint(dir.path(), Some(&dir.path().join("guide.md")));
        let mut watched: Vec<_> = watched
            .keys()
            .map(|path| path.strip_prefix(dir.path()).unwrap().to_owned())
            .collect();
        watched.sort();
        let expected: Vec<PathBuf> = [
            "agents/x/agent.md",
            "factory.toml",
            "guide.md",
            "skills/s/SKILL.md",
            "workflows/a.toml",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(watched, expected);
    }

    #[test]
    fn a_new_name_waits_for_a_restart() {
        let old = Factory {
            name: "a".into(),
            ..Factory::default()
        };
        let new = Factory {
            name: "b".into(),
            ..Factory::default()
        };
        assert_eq!(restart_only(&old, &new), ["name"]);
        assert!(restart_only(&old, &old).is_empty());
    }
}
