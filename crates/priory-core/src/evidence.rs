//! Evidence: screenshots, terminal recordings and screen recordings that show a change working,
//! each kept with its run's result. Text is searched for secrets before it is stored; pixels are
//! admitted only from the factory's own `record` command (docs/daemon/evidence.md).

use crate::daemon::repo_of;
use priory_factory::factory::Factory;
use priory_runtime::{runtime::git, script};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::JoinHandle;
use std::time::Duration;

/// The largest item.
pub const MOST_BYTES: u64 = 10 * 1024 * 1024;
/// The most a task's items may weigh together.
pub const TASK_BYTES: u64 = 50 * 1024 * 1024;
/// The most items a task may have.
pub const TASK_ITEMS: i64 = 20;
/// The longest caption, in characters.
pub const CAPTION: usize = 200;
/// How long a task's items stay after it ended.
pub const KEPT_DAYS: u32 = 30;
/// What a caption file is named after its item: `shot.png` and `shot.png.caption`.
const CAPTION_FILE: &str = ".caption";

/// What an item is, read from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniffed {
    /// `image`, `cast` or `video`.
    pub kind: &'static str,
    /// The content type it is served with.
    pub media: &'static str,
    /// The extension its file is stored with.
    pub ext: &'static str,
}

const PNG: Sniffed = Sniffed {
    kind: "image",
    media: "image/png",
    ext: "png",
};
const JPEG: Sniffed = Sniffed {
    kind: "image",
    media: "image/jpeg",
    ext: "jpg",
};
const WEBP: Sniffed = Sniffed {
    kind: "image",
    media: "image/webp",
    ext: "webp",
};
const MP4: Sniffed = Sniffed {
    kind: "video",
    media: "video/mp4",
    ext: "mp4",
};
const WEBM: Sniffed = Sniffed {
    kind: "video",
    media: "video/webm",
    ext: "webm",
};
const CAST: Sniffed = Sniffed {
    kind: "cast",
    media: "text/plain; charset=utf-8",
    ext: "cast",
};

/// Every type an item may be, by content type, for the file a row names.
const TYPES: [Sniffed; 6] = [PNG, JPEG, WEBP, MP4, WEBM, CAST];

/// What `bytes` is, by its first bytes and never its name, or why it is refused.
pub fn sniff(bytes: &[u8]) -> Result<Sniffed, String> {
    let at = |range: std::ops::Range<usize>| bytes.get(range).unwrap_or_default();
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(PNG);
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Ok(JPEG);
    }
    if at(0..4) == b"RIFF" && at(8..12) == b"WEBP" {
        return Ok(WEBP);
    }
    if at(4..8) == b"ftyp" {
        return Ok(MP4);
    }
    // Matroska shares WebM's EBML magic; its doctype says which, within the header.
    if bytes.starts_with(b"\x1a\x45\xdf\xa3")
        && at(0..64.min(bytes.len())).windows(4).any(|w| w == b"webm")
    {
        return Ok(WEBM);
    }
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    if text.trim_ascii_start().starts_with(b"<") {
        return Err("an SVG or HTML file is refused: it can carry script".to_owned());
    }
    if cast(bytes).is_ok() {
        return Ok(CAST);
    }
    Err("not a PNG, JPEG, WebP, MP4, WebM or asciicast v2 file".to_owned())
}

/// The texts a cast's secret search reads: the whole file, its header's strings, and its output
/// and input events each joined into one stream, so a value split across events is found.
pub fn cast(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "a cast is UTF-8 text".to_owned())?;
    let mut lines = text.lines();
    let header: Value = lines
        .next()
        .and_then(|line| serde_json::from_str(line).ok())
        .ok_or("a cast starts with a JSON header line")?;
    let sized = |name: &str| header[name].as_u64().is_some();
    if header["version"] != 2 || !sized("width") || !sized("height") {
        return Err("a cast's header has version 2, a width and a height".to_owned());
    }
    let mut strings = vec![text.to_owned()];
    fn gather(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::String(text) => into.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| gather(item, into)),
            Value::Object(fields) => fields.iter().for_each(|(key, item)| {
                into.push(key.clone());
                gather(item, into);
            }),
            _ => {}
        }
    }
    gather(&header, &mut strings);
    let mut streams: HashMap<String, String> = HashMap::new();
    for (at, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(line)
            .map_err(|_| format!("line {} of the cast is not JSON", at + 2))?;
        let (Some(_), Some(code), Some(data)) =
            (event[0].as_f64(), event[1].as_str(), event[2].as_str())
        else {
            return Err(format!(
                "line {} of the cast is not an event [time, code, data]",
                at + 2
            ));
        };
        if event.as_array().is_some_and(|event| event.len() != 3) {
            return Err(format!(
                "line {} of the cast is not an event [time, code, data]",
                at + 2
            ));
        }
        streams.entry(code.to_owned()).or_default().push_str(data);
    }
    strings.extend(streams.into_values());
    Ok(strings)
}

/// A secret Priory knows: the variable that holds it and its value.
pub struct Secret {
    pub name: String,
    pub value: String,
}

/// The values of the factory's secret variables and of `[serve] redact_env`, each whatever its
/// length, and the parts of each the screen's redaction also hides.
pub fn secrets(factory: &Factory) -> Vec<Secret> {
    let mut found = Vec::new();
    for name in factory.secret_envs() {
        let Some(value) = std::env::var(&name).ok().filter(|value| !value.is_empty()) else {
            continue;
        };
        let parts = crate::screen::secrets(std::slice::from_ref(&name));
        for value in std::iter::once(value).chain(parts) {
            if !found
                .iter()
                .any(|known: &Secret| known.value == value && known.name == name)
            {
                found.push(Secret {
                    name: name.clone(),
                    value,
                });
            }
        }
    }
    found
}

/// The variable of the first secret that `texts` holds.
fn holds<'s>(texts: &[&[u8]], secrets: &'s [Secret]) -> Option<&'s str> {
    secrets
        .iter()
        .find(|secret| {
            let needle = secret.value.as_bytes();
            texts
                .iter()
                .any(|text| text.windows(needle.len()).any(|window| window == needle))
        })
        .map(|secret| secret.name.as_str())
}

/// Why `caption` is refused, if it is: empty, over one line or 200 characters, or holding a secret.
fn caption_refused(caption: &str, secrets: &[Secret]) -> Option<String> {
    if caption.trim().is_empty() {
        return Some("a caption is needed".to_owned());
    }
    if caption.chars().any(char::is_control) {
        return Some("a caption is one line".to_owned());
    }
    if caption.chars().count() > CAPTION {
        return Some(format!("a caption is at most {CAPTION} characters"));
    }
    holds(&[caption.as_bytes()], secrets)
        .map(|name| format!("the caption holds the value of the secret variable {name}"))
}

/// Who adds an item: the factory's `record` command, or the agent from its pane.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Door {
    Record,
    Agent,
}

/// What `bytes` is, once it passed the checks that need no state: its type and size, a type its
/// door may add, its caption, and a search for every secret.
pub fn check(
    bytes: &[u8],
    caption: &str,
    secrets: &[Secret],
    door: Door,
) -> Result<Sniffed, String> {
    if bytes.len() as u64 > MOST_BYTES {
        return Err(format!(
            "the file is {} bytes, over the limit of 10 MB",
            bytes.len()
        ));
    }
    let sniffed = sniff(bytes)?;
    if door == Door::Agent && sniffed.kind != "cast" {
        return Err(format!(
            "only a step's record command adds an {}",
            sniffed.kind
        ));
    }
    if let Some(refused) = caption_refused(caption, secrets) {
        return Err(refused);
    }
    if sniffed.kind == "cast" {
        let texts = cast(bytes)?;
        let texts: Vec<&[u8]> = texts.iter().map(|text| text.as_bytes()).collect();
        if let Some(name) = holds(&texts, secrets) {
            return Err(format!(
                "the cast holds the value of the secret variable {name}"
            ));
        }
    }
    Ok(sniffed)
}

/// An item that passed `check`, to store for a run.
pub struct Item<'a> {
    pub task: i64,
    pub run: i64,
    pub commit: &'a str,
    pub caption: &'a str,
    pub bytes: &'a [u8],
    pub sniffed: Sniffed,
}

/// Stores `item` under `home`, unless it takes its task past 20 items or 50 MB, and returns its id.
pub(crate) fn store(
    tx: &Transaction,
    home: &Path,
    item: &Item,
) -> rusqlite::Result<Result<i64, String>> {
    let (count, weight): (i64, i64) = tx.query_row(
        "SELECT count(*), coalesce(sum(bytes), 0) FROM evidence WHERE task = ?1",
        [item.task],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if count >= TASK_ITEMS {
        return Ok(Err(format!(
            "task {} already has {TASK_ITEMS} items, the most it may have",
            item.task
        )));
    }
    if weight as u64 + item.bytes.len() as u64 > TASK_BYTES {
        return Ok(Err(format!(
            "it takes task {} past 50 MB of items",
            item.task
        )));
    }
    let sha256 = hex(&Sha256::digest(item.bytes));
    let file = file_of(home, item.task, &sha256, item.sniffed.ext);
    let written = (|| {
        std::fs::create_dir_all(file.parent().expect("an item's file has a folder"))?;
        let partial = file.with_extension("partial");
        std::fs::write(&partial, item.bytes)?;
        std::fs::rename(&partial, &file)
    })();
    if let Err(why) = written {
        return Ok(Err(format!("cannot store it: {why}")));
    }
    tx.execute(
        "INSERT INTO evidence (task, run, kind, media, caption, commit_sha, bytes, sha256)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            item.task,
            item.run,
            item.sniffed.kind,
            item.sniffed.media,
            item.caption,
            item.commit,
            item.bytes.len() as i64,
            sha256
        ],
    )?;
    Ok(Ok(tx.last_insert_rowid()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where an item lives: `PRIORY_HOME/evidence/<task>/<sha256>.<ext>`, outside every worktree.
fn file_of(home: &Path, task: i64, sha256: &str, ext: &str) -> PathBuf {
    home.join("evidence")
        .join(task.to_string())
        .join(format!("{sha256}.{ext}"))
}

/// Item `id`'s bytes and the content type it is served with, or none when it does not exist.
pub fn served(
    db: &Connection,
    home: &Path,
    id: i64,
) -> rusqlite::Result<Option<(Sniffed, Vec<u8>)>> {
    let row = db
        .query_row(
            "SELECT task, media, sha256 FROM evidence WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(task, media, sha256)| {
        let sniffed = TYPES.into_iter().find(|known| known.media == media)?;
        let bytes = std::fs::read(file_of(home, task, &sha256, sniffed.ext)).ok()?;
        Some((sniffed, bytes))
    }))
}

/// `priory evidence add`'s line: `{"evidence": {...}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddLine {
    pub evidence: Add,
}

/// A cast an agent adds from its run's pane.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Add {
    pub session: String,
    /// The file's absolute path, as the agent named it.
    pub file: String,
    pub caption: String,
}

/// Admits the cast `add` names for the live run of its session: a file inside the run's worktree,
/// checked out at the head of the task's branch.
pub(crate) fn add(
    db: &mut Connection,
    add: &Add,
    factory: &Factory,
    home: &Path,
    head: &dyn Fn(&Path, &str) -> Option<String>,
    checked_out: &dyn Fn(&Path) -> Option<String>,
) -> rusqlite::Result<Result<String, String>> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some((run, task, worktree, branch)) = tx
        .query_row(
            "SELECT runs.id, runs.task, tasks.worktree, tasks.branch
             FROM runs JOIN tasks ON tasks.id = runs.task
             WHERE runs.ended IS NULL AND runs.session = ?1",
            [&add.session],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(Err(format!("no live run has session {}", add.session)));
    };
    let (Some(worktree), Some(branch)) = (worktree, branch) else {
        return Ok(Err(format!("task {task} has no worktree")));
    };
    let shown = &add.file;
    let inside = Path::new(&worktree)
        .canonicalize()
        .ok()
        .zip(Path::new(&add.file).canonicalize().ok())
        .filter(|(tree, file)| file.starts_with(tree));
    let Some((_, file)) = inside else {
        return Ok(Err(format!(
            "{shown} is not a file inside the run's worktree"
        )));
    };
    let size = std::fs::metadata(&file).map(|meta| (meta.is_file(), meta.len()));
    match size {
        Ok((true, size)) if size > MOST_BYTES => {
            return Ok(Err(format!(
                "the file is {size} bytes, over the limit of 10 MB"
            )));
        }
        Ok((true, _)) => {}
        _ => {
            return Ok(Err(format!(
                "{shown} is not a file inside the run's worktree"
            )));
        }
    }
    let bytes = match std::fs::read(&file) {
        Ok(bytes) => bytes,
        Err(why) => return Ok(Err(format!("cannot read {shown}: {why}"))),
    };
    let Some(commit) = head(Path::new(&worktree), &branch) else {
        return Ok(Err(format!("branch {branch} of task {task} has no commit")));
    };
    let at = checked_out(Path::new(&worktree)).unwrap_or_default();
    if at != commit {
        return Ok(Err(format!(
            "the worktree is at {at}, not at the head of its branch, {commit}: check out the branch, then add it again"
        )));
    }
    let sniffed = match check(&bytes, &add.caption, &secrets(factory), Door::Agent) {
        Ok(sniffed) => sniffed,
        Err(refused) => return Ok(Err(refused)),
    };
    let item = Item {
        task,
        run,
        commit: &commit,
        caption: &add.caption,
        bytes: &bytes,
        sniffed,
    };
    let id = match store(&tx, home, &item)? {
        Ok(id) => id,
        Err(refused) => return Ok(Err(refused)),
    };
    tx.commit()?;
    Ok(Ok(format!(
        "evidence {id} added to task {task} at {commit}"
    )))
}

/// The `record` jobs running now, by run.
#[derive(Default)]
pub struct Records {
    running: HashMap<i64, JoinHandle<Recorded>>,
}

/// How a `record` command ended.
enum Recorded {
    /// It exited 0: its files, each with its name, bytes and caption, and why others were left out.
    Files(Vec<(String, Vec<u8>, String)>, Vec<String>),
    /// Nothing was recorded, for this reason.
    Ended(&'static str, String),
}

/// Queues the step's `record` command `record` for `run`'s result at `commit`.
pub(crate) fn owe(
    tx: &Transaction,
    run: i64,
    task: i64,
    record: &Path,
    commit: Option<&str>,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO evidence_records (run, task, command, commit_sha) VALUES (?1, ?2, ?3, ?4)",
        params![run, task, record.display().to_string(), commit],
    )
    .map(drop)
}

/// A `record` command owed: its run, task, command, commit, and the task's trigger and repo.
type Owed = (i64, i64, String, Option<String>, String, Option<String>);

/// Starts each `record` command owed and not running, and stores what each finished one made.
pub(crate) fn tick(
    db: &mut Connection,
    factory: &Factory,
    home: &Path,
    records: &mut Records,
) -> rusqlite::Result<()> {
    let owed: Vec<Owed> = db
        .prepare(
            "SELECT evidence_records.run, evidence_records.task, evidence_records.command,
               evidence_records.commit_sha, tasks.trigger, tasks.repo
             FROM evidence_records JOIN tasks ON tasks.id = evidence_records.task
             WHERE evidence_records.state = 'queued' ORDER BY evidence_records.run",
        )?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (run, task, command, commit, trigger, repo) in owed {
        if records.running.contains_key(&run) {
            continue;
        }
        let program = priory_factory::rules::recorder(&factory.dir, Path::new(&command));
        let job = match (program, commit, repo_of(factory, &trigger, repo.as_deref())) {
            (Err(why), ..) => Err(format!("`record`: {why}")),
            (_, None, _) => Err("the result has no commit to record".to_owned()),
            (_, _, None) => Err(format!("task {task} names no repo")),
            (Ok(program), Some(commit), Some(repo)) => Ok(Job {
                task,
                program,
                clone: home.join("repos").join(repo),
                scratch: home.join("records").join(uuid::Uuid::new_v4().to_string()),
                commit,
                timeout: factory.command_timeout(),
                secrets: secrets(factory),
            }),
        };
        match job {
            Ok(job) => {
                records
                    .running
                    .insert(run, std::thread::spawn(move || job.record()));
            }
            Err(why) => ended(db, run, "failed", Some(&why))?,
        }
    }
    let finished: Vec<i64> = records
        .running
        .iter()
        .filter(|(_, job)| job.is_finished())
        .map(|(run, _)| *run)
        .collect();
    for run in finished {
        let job = records
            .running
            .remove(&run)
            .expect("a finished job is running");
        let recorded = job
            .join()
            .unwrap_or_else(|_| Recorded::Ended("failed", "the record job panicked".to_owned()));
        match recorded {
            Recorded::Ended(state, why) => ended(db, run, state, Some(&why))?,
            Recorded::Files(files, mut left) => {
                let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let (task, commit): (i64, String) = tx.query_row(
                    "SELECT task, commit_sha FROM evidence_records WHERE run = ?1",
                    [run],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                let secrets = secrets(factory);
                for (name, bytes, caption) in files {
                    let stored = match check(&bytes, &caption, &secrets, Door::Record) {
                        Ok(sniffed) => store(
                            &tx,
                            home,
                            &Item {
                                task,
                                run,
                                commit: &commit,
                                caption: &caption,
                                bytes: &bytes,
                                sniffed,
                            },
                        )?
                        .map(drop),
                        Err(refused) => Err(refused),
                    };
                    if let Err(refused) = stored {
                        left.push(format!("{name}: {refused}"));
                    }
                }
                tx.commit()?;
                let why = (!left.is_empty()).then(|| left.join("; "));
                ended(db, run, "recorded", why.as_deref())?;
            }
        }
    }
    Ok(())
}

fn ended(db: &Connection, run: i64, state: &str, why: Option<&str>) -> rusqlite::Result<()> {
    if let Some(why) = why {
        eprintln!("priory daemon: the record command of run {run}: {why}");
    }
    db.execute(
        "UPDATE evidence_records SET state = ?2, reason = ?3,
           ended = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE run = ?1",
        params![run, state, why],
    )
    .map(drop)
}

/// One `record` command to run in a checkout of `commit`.
struct Job {
    task: i64,
    program: PathBuf,
    clone: PathBuf,
    scratch: PathBuf,
    commit: String,
    timeout: Duration,
    secrets: Vec<Secret>,
}

impl Job {
    /// Checks out the commit, searches its tracked files, runs the command and reads its files.
    fn record(self) -> Recorded {
        let added = git(&self.clone)
            .args(["worktree", "add", "--quiet", "--detach"])
            .arg(&self.scratch)
            .arg(&self.commit)
            .output();
        let recorded = match added {
            Ok(output) if output.status.success() => self.in_checkout(),
            Ok(output) => Recorded::Ended(
                "failed",
                format!(
                    "cannot check out {}: {}",
                    self.commit,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ),
            Err(why) => Recorded::Ended("failed", format!("cannot run git: {why}")),
        };
        let _ = git(&self.clone)
            .args(["worktree", "remove", "--force"])
            .arg(&self.scratch)
            .output();
        for folder in [
            self.scratch.with_extension("out"),
            self.scratch.with_extension("home"),
        ] {
            let _ = std::fs::remove_dir_all(folder);
        }
        recorded
    }

    fn in_checkout(&self) -> Recorded {
        if let Some(refused) = tracked_secret(&self.scratch, &self.secrets) {
            return Recorded::Ended("refused", refused);
        }
        let (out, home, log) = (
            self.scratch.with_extension("out"),
            self.scratch.with_extension("home"),
            self.scratch.with_extension("log"),
        );
        for folder in [&out, &home] {
            if let Err(why) = std::fs::create_dir_all(folder) {
                return Recorded::Ended("failed", format!("{}: {why}", folder.display()));
            }
        }
        let mut command = Command::new(&self.program);
        command.env_clear();
        for name in ["PATH", "LANG", "TERM"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("HOME", &home)
            .env("PRIORY_EVIDENCE", &out)
            .env("PRIORY_TASK", self.task.to_string())
            .env("PRIORY_COMMIT", &self.commit);
        let mut running = match script::spawn_command(command, &self.scratch, &log, self.timeout) {
            Ok(running) => running,
            Err(why) => return Recorded::Ended("failed", format!("cannot start: {why}")),
        };
        let (status, output) = loop {
            if let Some(ended) = running.poll() {
                break ended;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = std::fs::remove_file(&log);
        if status != "exit 0" {
            let last = output.lines().last().unwrap_or_default().trim();
            let why = match (last.is_empty(), holds(&[last.as_bytes()], &self.secrets)) {
                (true, _) => status,
                (false, Some(name)) => {
                    format!("{status}: its last line holds the value of the secret variable {name}")
                }
                (false, None) => format!("{status}: {last}"),
            };
            return Recorded::Ended("failed", why);
        }
        made(&out)
    }
}

/// The files the command wrote to `out`, each with its caption, and why others were left out.
fn made(out: &Path) -> Recorded {
    let mut names: Vec<String> = match std::fs::read_dir(out) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.ends_with(CAPTION_FILE))
            .collect(),
        Err(why) => return Recorded::Ended("failed", format!("{}: {why}", out.display())),
    };
    names.sort();
    let mut files = Vec::new();
    let mut left = Vec::new();
    for name in names {
        let path = out.join(&name);
        let caption = out.join(format!("{name}{CAPTION_FILE}"));
        let read = match std::fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() => Err("not a file".to_owned()),
            Ok(meta) if meta.len() > MOST_BYTES => Err(format!(
                "the file is {} bytes, over the limit of 10 MB",
                meta.len()
            )),
            Ok(_) => match std::fs::read_to_string(&caption) {
                Ok(text) => std::fs::read(&path)
                    .map(|bytes| (bytes, text.trim_end_matches(['\n', '\r']).to_owned()))
                    .map_err(|why| why.to_string()),
                Err(_) => Err(format!("no caption in {name}{CAPTION_FILE}")),
            },
            Err(why) => Err(why.to_string()),
        };
        match read {
            Ok((bytes, caption)) => files.push((name, bytes, caption)),
            Err(why) => left.push(format!("{name}: {why}")),
        }
    }
    Recorded::Files(files, left)
}

/// Why nothing is recorded in `checkout`: a tracked file holds a secret's value, named by its path
/// and the secret's variable, never the value.
fn tracked_secret(checkout: &Path, secrets: &[Secret]) -> Option<String> {
    if secrets.is_empty() {
        return None;
    }
    let listed = git(checkout).args(["ls-files", "-z"]).output();
    let listed = match listed {
        Ok(output) if output.status.success() => output.stdout,
        Ok(output) => {
            return Some(format!(
                "cannot list the checkout's files: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Err(why) => return Some(format!("cannot run git: {why}")),
    };
    for name in listed
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = String::from_utf8_lossy(name);
        let path = checkout.join(name.as_ref());
        let bytes = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_symlink() => std::fs::read_link(&path)
                .map(|target| target.into_os_string().into_encoded_bytes())
                .unwrap_or_default(),
            Ok(meta) if meta.is_file() => std::fs::read(&path).unwrap_or_default(),
            _ => continue,
        };
        if let Some(secret) = holds(&[&bytes], secrets) {
            return Some(format!(
                "the checkout's tracked file {name} holds the value of the secret variable {secret}: nothing was recorded"
            ));
        }
    }
    None
}

/// The highest item the task's latest approval covers: none when no approval exists, and
/// `Some(None)` when one exists that covers nothing.
fn covered(db: &Connection, task: i64) -> rusqlite::Result<Option<Option<i64>>> {
    db.query_row(
        "SELECT approval_evidence.evidence FROM approvals
         LEFT JOIN approval_evidence ON approval_evidence.approval = approvals.id
         WHERE approvals.task = ?1 ORDER BY approvals.id DESC LIMIT 1",
        [task],
        |row| row.get(0),
    )
    .optional()
}

/// Records that `approval` covers every item its task has now.
pub(crate) fn approved(tx: &Transaction, task: i64, approval: i64) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO approval_evidence (approval, evidence)
         SELECT ?2, max(id) FROM evidence WHERE task = ?1 HAVING max(id) IS NOT NULL",
        params![task, approval],
    )
    .map(drop)
}

/// The columns of an item, as the API and `priory evidence` give them.
const COLUMNS: &str = "id, run, kind, caption, commit_sha, bytes, sha256, created";

/// The task's items, oldest first, each with whether the task's latest approval covers it.
pub fn list(db: &Connection, task: i64) -> rusqlite::Result<Value> {
    items(db, task, "task = ?1", task)
}

fn items(db: &Connection, task: i64, filter: &str, key: i64) -> rusqlite::Result<Value> {
    let covered = covered(db, task)?.flatten();
    let mut rows = crate::listing::rows(
        db,
        &format!("SELECT {COLUMNS} FROM evidence WHERE {filter} ORDER BY id"),
        [key],
    )?;
    for row in rows.as_array_mut().expect("rows are an array") {
        let id = row["id"].as_i64();
        row["approved"] = Value::Bool(id.zip(covered).is_some_and(|(id, top)| id <= top));
    }
    Ok(rows)
}

/// Adds `attachments` and `record` to each of the task's results.
pub(crate) fn attach(db: &Connection, task: i64, results: &mut Value) -> rusqlite::Result<()> {
    for result in results.as_array_mut().expect("rows are an array") {
        let Some(run) = result["run"].as_i64() else {
            continue;
        };
        result["attachments"] = items(db, task, "run = ?1", run)?;
        let record = crate::listing::rows(
            db,
            "SELECT state, reason, commit_sha, created, ended FROM evidence_records WHERE run = ?1",
            [run],
        )?;
        result["record"] = record
            .as_array()
            .and_then(|rows| rows.first())
            .cloned()
            .unwrap_or(Value::Null);
    }
    Ok(())
}

/// The Evidence section of a task's pull request body: one line per item its approval covers, or
/// per item when no approval exists, each linking to the task page at `url` when the factory names
/// it. None when there is no item.
pub(crate) fn section(
    db: &Connection,
    task: i64,
    url: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let covered = covered(db, task)?;
    let lines: Vec<String> = db
        .prepare("SELECT id, caption, kind, commit_sha FROM evidence WHERE task = ?1 ORDER BY id")?
        .query_map([task], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|(id, ..)| match covered {
            None => true,
            Some(top) => top.is_some_and(|top| *id <= top),
        })
        .map(|(_, caption, kind, commit)| {
            let short = commit.get(..7).unwrap_or(&commit);
            let line = format!("- {} ({kind} at {short})", markdown_text(&caption));
            match url {
                Some(url) => format!(
                    "{line}: {}{}",
                    url.trim_end_matches('/'),
                    crate::listing::task_page(task)
                ),
                None => line,
            }
        })
        .collect();
    Ok((!lines.is_empty()).then(|| format!("## Evidence\n\n{}", lines.join("\n"))))
}

/// `text` with each ASCII punctuation character escaped, so an agent's caption shows as text.
fn markdown_text(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c.is_ascii_punctuation() {
            true => vec!['\\', c],
            false => vec![c],
        })
        .collect()
}

/// Removes the items of every task that ended over 30 days ago, files first. An active task's
/// items stay, however old.
pub(crate) fn sweep(db: &Connection, home: &Path) -> rusqlite::Result<()> {
    let ended: Vec<i64> = db
        .prepare(&format!(
            "SELECT DISTINCT evidence.task FROM evidence JOIN tasks ON tasks.id = evidence.task
             WHERE tasks.state IN ('done', 'failed', 'dropped')
               AND tasks.changed < strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-{KEPT_DAYS} days')"
        ))?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for task in ended {
        let folder = home.join("evidence").join(task.to_string());
        match std::fs::remove_dir_all(&folder) {
            Ok(()) => {}
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
            Err(why) => {
                eprintln!("priory daemon: removing {}: {why}", folder.display());
                continue;
            }
        }
        db.execute("DELETE FROM evidence WHERE task = ?1", [task])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn the_type_check_and_the_cast_reader_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = sniff(&bytes);
            let _ = cast(&bytes);
        }

        #[test]
        fn a_mutated_cast_is_read_or_refused_without_a_panic(
            at in 0usize..200,
            byte in any::<u8>(),
            cut in 0usize..200,
        ) {
            let mut bytes = b"{\"version\": 2, \"width\": 80, \"height\": 24}\n[0.1, \"o\", \"hello \"]\n[0.2, \"o\", \"world\\r\\n\"]\n".to_vec();
            let at = at % bytes.len();
            bytes[at] = byte;
            bytes.truncate(cut.max(1).min(bytes.len()));
            let _ = sniff(&bytes);
            let _ = cast(&bytes);
            let _ = check(&bytes, "a caption", &[], Door::Agent);
        }
    }
}
