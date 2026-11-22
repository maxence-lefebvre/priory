//! The factory's skills, instructions and memories, and its kit repository's, as each run gets
//! them (R11.1).

use priory_factory::factory::{self, Factory, KitFiles, Memory, Skill};
use priory_runtime::runtime;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

type Error = Box<dyn std::error::Error>;

/// A hex digest of `parts`, each prefixed with its length so no two lists of parts hash alike.
fn digest<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A kit commit, checked out under Priory home and loaded.
pub(crate) struct Fetched {
    pub(crate) commit: String,
    files: KitFiles,
}

/// What a run gets: the kit's skills, instructions and memories, then the factory's own, the
/// factory's winning on a name both hold, then the memory candidates the run sees, in order.
pub(crate) struct Sources<'a> {
    skills: BTreeMap<&'a str, &'a Skill>,
    instructions: String,
    memories: BTreeMap<&'a str, &'a Memory>,
    /// The factory's name, which the plugin's manifest shows.
    name: &'a str,
    /// The kit commit, when the runner has one.
    pub(crate) commit: Option<&'a str>,
}

pub(crate) fn sources<'a>(
    factory: &'a Factory,
    kit: Option<&'a Fetched>,
    candidates: &'a [(String, Option<Memory>)],
) -> Sources<'a> {
    let mut skills = BTreeMap::new();
    let mut memories = BTreeMap::new();
    let mut instructions = "";
    if let Some(kit) = kit {
        skills.extend(kit.files.skills.iter().map(|(n, s)| (n.as_str(), &s.value)));
        memories.extend(
            kit.files
                .memories
                .iter()
                .map(|(n, m)| (n.as_str(), &m.value)),
        );
        instructions = kit.files.instructions.as_deref().unwrap_or("").trim();
    }
    skills.extend(factory.skills.iter().map(|(n, s)| (n.as_str(), &s.value)));
    memories.extend(factory.memories.iter().map(|(n, m)| (n.as_str(), &m.value)));
    for (name, memory) in candidates {
        match memory {
            Some(memory) => memories.insert(name.as_str(), memory),
            None => memories.remove(name.as_str()),
        };
    }
    Sources {
        skills,
        instructions: factory::join_instructions(instructions, factory.instructions()),
        memories,
        name: &factory.name,
        commit: kit.map(|kit| kit.commit.as_str()),
    }
}

impl Sources<'_> {
    /// The memory `name` a run gets, if any.
    pub(crate) fn memory(&self, name: &str) -> Option<&Memory> {
        self.memories.get(name).copied()
    }

    /// The instructions, then the index of the memories in `folder`, as every run gets them.
    pub(crate) fn run_instructions(&self, folder: &str) -> String {
        let index = factory::memory_index(folder, self.memories.iter().map(|(n, m)| (*n, *m)));
        factory::join_instructions(&self.instructions, &index)
    }

    /// The index of the skills, whose folders are in `folder`, as a Codex run gets it.
    pub(crate) fn skill_index(&self, folder: &str) -> String {
        factory::skill_index(folder, self.skills.iter().map(|(n, s)| (*n, *s)))
    }

    /// Each skill's name, then each of its files' paths and bytes, in order.
    fn skill_parts(&self) -> Vec<&[u8]> {
        let mut parts = Vec::new();
        for (name, skill) in &self.skills {
            parts.push(name.as_bytes());
            for (path, bytes) in &skill.files {
                parts.extend([path.as_os_str().as_bytes(), bytes.as_slice()]);
            }
        }
        parts
    }

    /// Each memory's name and bytes, in order.
    fn memory_parts(&self) -> Vec<&[u8]> {
        let mut parts = Vec::new();
        for (name, memory) in &self.memories {
            parts.extend([name.as_bytes(), memory.bytes.as_slice()]);
        }
        parts
    }

    /// A hash of the skills, instructions and memories that a run gets, or none when it gets none
    /// of them.
    pub(crate) fn hash(&self) -> Option<String> {
        let skills = !self.skills.is_empty();
        let memories = !self.memories.is_empty();
        if self.instructions.is_empty() && !skills && !memories {
            return None;
        }
        let mut parts: Vec<&[u8]> = vec![b"instructions", self.instructions.as_bytes()];
        if skills {
            parts.push(b"skills");
            parts.extend(self.skill_parts());
        }
        // Without memories, the hash is the one runs recorded before them.
        if memories {
            parts.push(b"memories");
            parts.extend(self.memory_parts());
        }
        Some(digest(parts))
    }

    /// The folder that holds the memories, `<home>/kit/<hash>/memories`, written once per content,
    /// or none without memories.
    pub(crate) fn memories(&self, home: &Path) -> Result<Option<PathBuf>, String> {
        if self.memories.is_empty() {
            return Ok(None);
        }
        // The tag keeps this folder's hash apart from a plugin folder's.
        let hash = digest([&b"memories"[..]].into_iter().chain(self.memory_parts()));
        let files: Vec<(PathBuf, &[u8])> = self
            .memories
            .iter()
            .map(|(name, memory)| (PathBuf::from(format!("{name}.md")), memory.bytes.as_slice()))
            .collect();
        runtime::write_once(&home.join("kit").join(hash), "memories", &files).map(Some)
    }

    /// The plugin folder that holds the skills, `<home>/kit/<hash>/plugin`, written once per
    /// content, or none without skills.
    pub(crate) fn plugin(&self, home: &Path) -> Result<Option<PathBuf>, String> {
        if self.skills.is_empty() {
            return Ok(None);
        }
        let hash = digest(self.skill_parts());
        let manifest = serde_json::json!({
            "name": "priory",
            "description": format!("The skills of the Priory factory {}", self.name),
        })
        .to_string();
        let mut files: Vec<(PathBuf, &[u8])> = vec![(
            PathBuf::from(".claude-plugin/plugin.json"),
            manifest.as_bytes(),
        )];
        for (name, skill) in &self.skills {
            for (path, bytes) in &skill.files {
                files.push((Path::new("skills").join(name).join(path), bytes));
            }
        }
        runtime::plugin(&home.join("kit").join(hash), &files).map(Some)
    }
}

/// How a runner keeps the kit that `[kit]` names: the commit its runs get, and its next fetch.
pub(crate) struct Fetcher {
    settings: factory::Kit,
    /// The repository as `git fetch` takes it, a relative path made absolute.
    repo: String,
    mirror: PathBuf,
    home: PathBuf,
    current: Option<Arc<Fetched>>,
    tried: Instant,
    fetching: Option<JoinHandle<Result<Fetched, String>>>,
    /// The names of the factory's own skills and memories, which hide the kit's.
    own: Vec<String>,
}

impl Fetcher {
    /// The fetcher of the factory's `[kit]`, or none without one. Without one, it forgets any
    /// earlier kit's banner.
    pub(crate) fn new(
        db: &Connection,
        factory: &Factory,
        home: &Path,
    ) -> Result<Option<Fetcher>, Error> {
        let Some(settings) = factory.settings.as_ref().and_then(|s| s.value.kit.clone()) else {
            db.execute("DELETE FROM kit", [])?;
            return Ok(None);
        };
        let (repo, mirror) = place(&settings, factory, home)?;
        let own = factory
            .skills
            .keys()
            .map(|name| format!("skill {name}"))
            .chain(factory.memories.keys().map(|name| format!("memory {name}")))
            .collect();
        Ok(Some(Fetcher {
            settings,
            repo,
            mirror,
            home: home.to_path_buf(),
            current: None,
            tried: Instant::now(),
            fetching: None,
            own,
        }))
    }

    /// The commit runs get now, if any.
    pub(crate) fn current(&self) -> Option<&Fetched> {
        self.current.as_deref()
    }

    /// Fetches the ref before any run starts. A failed fetch keeps the last good commit, which the
    /// state file holds, and says so on the desk.
    pub(crate) fn start(&mut self, db: &Connection) -> Result<(), Error> {
        self.tried = Instant::now();
        let fetched = fetch(&self.settings, &self.repo, &self.mirror, &self.home);
        if fetched.is_err() {
            let last: Option<String> = db
                .query_row("SELECT kit_commit FROM kit", [], |row| row.get(0))
                .optional()?
                .flatten();
            if let Some(last) = last {
                match load(&self.settings, &self.mirror, &self.home, last) {
                    Ok(kit) => self.current = Some(Arc::new(kit)),
                    Err(error) => eprintln!("priory daemon: the last kit commit: {error}"),
                }
            }
        }
        self.record(db, fetched)
    }

    /// Fetches again every `every`, off the daemon's thread, and applies a fetch that finished.
    pub(crate) fn tick(&mut self, db: &Connection) -> Result<(), Error> {
        if self.fetching.as_ref().is_some_and(JoinHandle::is_finished) {
            let fetched = self
                .fetching
                .take()
                .expect("a fetch runs")
                .join()
                .unwrap_or_else(|_| Err("the kit's fetch panicked".into()));
            return self.record(db, fetched);
        }
        if self.fetching.is_none() && self.tried.elapsed() >= self.settings.every() {
            self.tried = Instant::now();
            let (settings, repo) = (self.settings.clone(), self.repo.clone());
            let (mirror, home) = (self.mirror.clone(), self.home.clone());
            self.fetching = Some(std::thread::spawn(move || {
                fetch(&settings, &repo, &mirror, &home)
            }));
        }
        Ok(())
    }

    /// Applies a fetch: a new commit applies to runs that start after it, and a failure keeps the
    /// commit runs had.
    fn record(&mut self, db: &Connection, fetched: Result<Fetched, String>) -> Result<(), Error> {
        let error = match fetched {
            Ok(kit) => {
                if self
                    .current()
                    .is_none_or(|current| current.commit != kit.commit)
                {
                    eprintln!("priory daemon: runs now get kit commit {}", kit.commit);
                    for hidden in self.own.iter().filter(|own| match own.split_once(' ') {
                        Some(("skill", name)) => kit.files.skills.contains_key(name),
                        Some((_, name)) => kit.files.memories.contains_key(name),
                        None => false,
                    }) {
                        eprintln!("priory daemon: the factory's {hidden} hides the kit's");
                    }
                    self.current = Some(Arc::new(kit));
                }
                None
            }
            Err(error) => {
                eprintln!("priory daemon: the kit: {error}");
                Some(error)
            }
        };
        let commit = self.current().map(|kit| kit.commit.as_str());
        db.execute(
            "INSERT INTO kit (id, kit_commit, failing_since, error)
             VALUES (1, ?1, CASE WHEN ?2 IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END, ?2)
             ON CONFLICT (id) DO UPDATE SET kit_commit = excluded.kit_commit,
               failing_since = CASE WHEN excluded.error IS NULL THEN NULL
                 ELSE coalesce(kit.failing_since, excluded.failing_since) END,
               error = excluded.error",
            params![commit, error],
        )?;
        Ok(())
    }
}

/// The kit's repository as `git fetch` takes it, a relative path made absolute, and its bare mirror
/// under Priory home.
pub(crate) fn place(
    settings: &factory::Kit,
    factory: &Factory,
    home: &Path,
) -> std::io::Result<(String, PathBuf)> {
    let repo = settings.repo.trim();
    let local = !repo.contains("://") && !repo.contains(':');
    let repo = if local && Path::new(repo).is_relative() {
        std::path::absolute(factory.dir.join(repo))?
            .display()
            .to_string()
    } else {
        repo.to_owned()
    };
    let mirror = home
        .join("kit/repos")
        .join(format!("{}.git", digest([repo.as_bytes()])));
    Ok((repo, mirror))
}

/// Fetches the ref, then checks out and loads the commit it names.
fn fetch(
    settings: &factory::Kit,
    repo: &str,
    mirror: &Path,
    home: &Path,
) -> Result<Fetched, String> {
    let commit = runtime::kit_fetch(mirror, repo, settings.reference())?;
    load(settings, mirror, home, commit)
}

/// Checks `commit` out into `<home>/kit/<commit>`, once, and loads it. A commit whose files
/// `priory check` would refuse is refused whole.
fn load(
    settings: &factory::Kit,
    mirror: &Path,
    home: &Path,
    commit: String,
) -> Result<Fetched, String> {
    let dir = runtime::kit_checkout(mirror, &home.join("kit"), &commit)?;
    let (files, problems) = factory::load_kit(&dir, settings);
    if let Some(first) = problems.first() {
        let at = first
            .at
            .as_deref()
            .map(|at| format!(":{at}"))
            .unwrap_or_default();
        return Err(format!(
            "commit {} has {} problem(s), the first in {}{at}: {}",
            &commit[..commit.len().min(12)],
            problems.len(),
            first.file.display(),
            first.message
        ));
    }
    Ok(Fetched { commit, files })
}

/// `the kit fails to fetch since <time>: <error>; runs ...` while the kit's last fetch failed.
pub fn banner(db: &Connection) -> rusqlite::Result<Option<String>> {
    db.query_row(
        "SELECT 'the kit fails to fetch since ' || failing_since || ': ' || error || '; '
           || CASE WHEN kit_commit IS NULL THEN 'runs start without the kit'
              ELSE 'runs keep commit ' || substr(kit_commit, 1, 12) END
         FROM kit WHERE error IS NOT NULL",
        [],
        |row| row.get(0),
    )
    .optional()
}
