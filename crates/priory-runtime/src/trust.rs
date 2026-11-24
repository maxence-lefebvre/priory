//! The folder trust Priory records for its own worktrees and clones, when the factory turns on
//! `trust_worktrees`, so no run stops on its harness's trust dialog (docs/daemon/sessions.md).

use crate::runtime::{Harness, clone_of, fed, fed_raw, fed_raw_within, git, run};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// The Codex profile Priory owns: `$CODEX_HOME/priory-trust.config.toml`, layered by `-p`.
pub const CODEX_PROFILE: &str = "priory-trust";

/// How many times Priory reads and writes a config that another writer keeps changing.
const TRIES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Priory wrote the grant.
    Granted,
    /// The grant was already there.
    Present,
    /// Priory wrote nothing, for the reason the grant carries.
    Refused,
    /// The harness asked for trust that Priory had recorded.
    Missed,
    /// direnv failed before the harness started, which then started without the `.envrc`.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Present => "present",
            Self::Refused => "refused",
            Self::Missed => "missed",
            Self::Failed => "failed",
        }
    }
}

/// One path a harness trusts, the file that records it, and what Priory did.
#[derive(Debug)]
pub struct Grant {
    pub path: PathBuf,
    pub file: PathBuf,
    pub outcome: Outcome,
    /// Why Priory wrote nothing, for a refused grant.
    pub why: Option<String>,
}

/// Records `harness`'s trust in `worktree` of `repo` and in its clone, both under Priory `home`.
/// A path outside `home/worktrees/<repo>` or `home/repos/<repo>` gets a refused grant only.
pub fn grant(harness: &dyn Harness, home: &Path, repo: &str, worktree: &Path) -> Vec<Grant> {
    match own(home, repo, worktree) {
        Ok((worktree, clone)) => harness.grant_trust(&worktree, &clone),
        Err(why) => vec![Grant {
            path: worktree.to_path_buf(),
            file: PathBuf::new(),
            outcome: Outcome::Refused,
            why: Some(why),
        }],
    }
}

/// Whether `harness`'s config still holds the trust recorded for `worktree` and its clone.
/// An error means Priory could not tell, as with a config it cannot read or parse.
pub fn holds(harness: &dyn Harness, worktree: &Path) -> Result<bool, String> {
    let real = |path: &Path| {
        fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))
    };
    let worktree = real(worktree)?;
    let clone =
        clone_of(&worktree).ok_or_else(|| format!("{} has no clone", worktree.display()))?;
    harness.holds_trust(&worktree, &real(&clone)?)
}

/// direnv's config folder of the run `session` under Priory `home`, which its `DIRENV_CONFIG` names.
pub fn direnv_folder(home: &Path, session: &str) -> PathBuf {
    home.join("direnv").join(session)
}

/// Makes direnv load `worktree` of `repo` through `folder`, a config folder of Priory's own, and
/// only with the `.envrc` bytes checked here: the folder holds a copy of them, and direnv's allow
/// files for them in `data`. No grant when `direnv` is not on PATH or the worktree has no `.envrc`.
/// Refused unless the `.envrc` is the one of commit `base`, or of `repo`'s default branch.
pub fn direnv(
    home: &Path,
    repo: &str,
    worktree: &Path,
    base: Option<&str>,
    folder: &Path,
) -> Vec<Grant> {
    if !crate::jail::on_path("direnv") || !worktree.join(".envrc").is_file() {
        return Vec::new();
    }
    let file = folder.join("direnv.toml");
    let (real, clone) = match own(home, repo, worktree) {
        Ok(paths) => paths,
        Err(why) => return grants(folder, &[worktree], Err(why)),
    };
    let result = vouched(&clone, repo, &real, base).and_then(|content| {
        // direnv hashes the path as `direnv exec` gets it, so a symlinked home needs both.
        let envrcs = [worktree, &real].map(|path| path.join(".envrc"));
        if denied(person_data().as_deref(), &envrcs) {
            return Err("a person denied this .envrc with direnv deny".to_owned());
        }
        let person = direnv_config().ok_or("no DIRENV_CONFIG, XDG_CONFIG_HOME or HOME")?;
        if linked(home) {
            return Err("the direnv folder in Priory home is a symlink".to_owned());
        }
        prepare(folder, &person, &content, &envrcs).map(|()| Outcome::Granted)
    });
    grants(&file, &[&real], result)
}

/// The `direnvrc` of a run's folder. direnv reads an `.envrc` once to hash it and again to run it,
/// so it makes direnv's first `source_env` run the copy that Priory checked. The person's files and
/// the `.envrc` see the person's `XDG_DATA_HOME`, which direnv alone gets replaced.
const DIRENVRC: &str = r#"priory_data=$XDG_DATA_HOME
if [[ -n ${PRIORY_DATA_HOME-} ]]; then export XDG_DATA_HOME=$PRIORY_DATA_HOME; else unset XDG_DATA_HOME; fi
priory_folder=$DIRENV_CONFIG
if [[ -f $priory_folder/direnvrc.person ]]; then
  source "$priory_folder/direnvrc.person"
elif [[ -f $HOME/.direnvrc ]]; then
  source "$HOME/.direnvrc"
fi
priory_source_env=$(declare -f source_env)
source_env() {
  eval "$priory_source_env"
  watch_file "$1"
  pushd "${1%/*}" >/dev/null || return 1
  . "$priory_folder/envrc"
  [[ ${XDG_DATA_HOME-} != "${PRIORY_DATA_HOME-}" ]] || export XDG_DATA_HOME=$priory_data
  popd >/dev/null || return 1
}
"#;

/// Writes the run's direnv config folder for the checked `content` of the `.envrc` at `envrcs`,
/// from the config folder `person`.
fn prepare(folder: &Path, person: &Path, content: &[u8], envrcs: &[PathBuf]) -> Result<(), String> {
    let own = ["direnv.toml", "config.toml"]
        .into_iter()
        .find(|name| person.join(name).is_file());
    let text = match own {
        Some(name) => {
            let text = fs::read_to_string(person.join(name))
                .map_err(|error| format!("{name}: {error}"))?;
            unlisted(&text).map_err(|why| format!("{name}: {why}"))?
        }
        None => String::new(),
    };
    let failed = |error: std::io::Error| format!("the run's direnv folder: {error}");
    let private = crate::runtime::write_private;
    let _ = fs::remove_dir_all(folder);
    let allow = folder.join("data/direnv/allow");
    fs::create_dir_all(&allow).map_err(failed)?;
    private(&folder.join("direnv.toml"), text.as_bytes()).map_err(failed)?;
    private(&folder.join("direnvrc"), DIRENVRC.as_bytes()).map_err(failed)?;
    private(&folder.join("envrc"), content).map_err(failed)?;
    for envrc in envrcs {
        let path = line_of(envrc);
        private(&allow.join(sha256(&[&path, content])), &path).map_err(failed)?;
    }
    // A link into the person's home that leads on, say into /nix/store, dangles in a jail.
    for (name, link) in [("direnvrc", PERSONS_DIRENVRC), ("lib", "lib")] {
        if let Ok(target) = fs::canonicalize(person.join(name)) {
            std::os::unix::fs::symlink(&target, folder.join(link)).map_err(failed)?;
        }
    }
    Ok(())
}

/// The link in a run's direnv folder to the person's `direnvrc`.
pub(crate) const PERSONS_DIRENVRC: &str = "direnvrc.person";

fn line_of(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    [path.as_os_str().as_bytes(), b"\n"].concat()
}

fn sha256(parts: &[&[u8]]) -> String {
    Sha256::digest(parts.concat())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether the person ran `direnv deny` on one of `envrcs`, as their data folder `data` says.
fn denied(data: Option<&Path>, envrcs: &[PathBuf]) -> bool {
    data.is_some_and(|data| {
        envrcs.iter().any(|envrc| {
            data.join("direnv/deny")
                .join(sha256(&[&line_of(envrc)]))
                .exists()
        })
    })
}

/// The content of `worktree`'s `.envrc`, refused unless it is a file with the content of the one at
/// `base`, or on `repo`'s default branch.
fn vouched(
    clone: &Path,
    repo: &str,
    worktree: &Path,
    base: Option<&str>,
) -> Result<Vec<u8>, String> {
    let path = worktree.join(".envrc");
    let failed = |error: std::io::Error| format!(".envrc: {error}");
    if !fs::symlink_metadata(&path).map_err(failed)?.is_file() {
        return Err(".envrc is not a regular file".to_owned());
    }
    let content = crate::runtime::read_regular(&path, ENVRC_CAP + 1).map_err(failed)?;
    if content.len() as u64 > ENVRC_CAP {
        return Err(format!(".envrc is over {ENVRC_CAP} bytes"));
    }
    let blob = hash("blob", content.clone())?;
    if base.is_some_and(|base| envrc_at(clone, base).is_ok_and(|at| at.as_ref() == Some(&blob))) {
        return Ok(content);
    }
    // No commit can vouch for it, so GitHub is not asked.
    if run(git(worktree).args(["ls-files", "--error-unmatch", "--", ".envrc"])).is_err() {
        return Err(
            ".envrc is untracked, so neither the commit the task started from nor the default \
             branch vouches for it"
                .to_owned(),
        );
    }
    match envrc_on_trunk(clone, repo) {
        Ok(Some(at)) if at == blob => Ok(content),
        Ok(_) => Err(
            ".envrc differs from the commit the task started from and from the default branch"
                .to_owned(),
        ),
        Err(why) => {
            eprintln!("priory daemon: the default branch of {repo}: {why}");
            Err(
                ".envrc differs from the commit the task started from, and the default branch \
                 could not be read"
                    .to_owned(),
            )
        }
    }
}

/// The blob id of the `.envrc` on `repo`'s default branch as GitHub has it. An agent can write the
/// clone, so its objects are hashed again.
fn envrc_on_trunk(clone: &Path, repo: &str) -> Result<Option<String>, String> {
    let trunk = crate::runtime::trunk(repo, TRUNK_WITHIN)?;
    envrc_at(clone, &trunk).or_else(|_| {
        fed_raw_within(
            git(clone).args(["fetch", "--quiet", "origin", &trunk]),
            None,
            TRUNK_WITHIN,
        )?;
        envrc_at(clone, &trunk)
    })
}

/// The largest `.envrc` a run can start in.
const ENVRC_CAP: u64 = 1 << 20;

/// How long each of the default branch's two reads may take.
// shortcut: they hold the daemon's thread, move them off when one often nears the limit.
const TRUNK_WITHIN: Duration = Duration::from_secs(10);

/// The id git gives an object of `kind` with `content`.
fn hash(kind: &str, content: Vec<u8>) -> Result<String, String> {
    fed(
        git(Path::new("/")).args(["hash-object", "-t", kind, "--stdin"]),
        Some(content),
    )
    .map(|id| id.trim().to_owned())
}

/// The content of the object `id` of `clone`, hashed again: git does not check a loose object on
/// read, and an agent can write one.
fn object(clone: &Path, kind: &str, id: &str) -> Result<Vec<u8>, String> {
    let content = fed_raw(
        git(clone).args(["--no-replace-objects", "cat-file", kind, id]),
        None,
    )?;
    if hash(kind, content.clone())? != id {
        return Err(format!("{kind} {id} of {} is forged", clone.display()));
    }
    Ok(content)
}

/// The blob id of the regular file `.envrc` at `commit` of `clone`, if it has one.
fn envrc_at(clone: &Path, commit: &str) -> Result<Option<String>, String> {
    let content = object(clone, "commit", commit)?;
    let tree = content
        .strip_prefix(b"tree ")
        .and_then(|rest| rest.split(|byte| *byte == b'\n').next())
        .and_then(|id| std::str::from_utf8(id).ok())
        .ok_or_else(|| format!("commit {commit} names no tree"))?;
    let entries = object(clone, "tree", tree)?;
    Ok(entry(&entries, b".envrc", tree.len() / 2))
}

/// The id of the regular file `name` in `tree`, git's raw tree of ids `width` bytes long.
fn entry(mut tree: &[u8], name: &[u8], width: usize) -> Option<String> {
    while !tree.is_empty() {
        let nul = tree.iter().position(|byte| *byte == 0)?;
        let id = tree.get(nul + 1..nul + 1 + width)?;
        let space = tree[..nul].iter().position(|byte| *byte == b' ')?;
        if &tree[space + 1..nul] == name {
            let file = matches!(&tree[..space], b"100644" | b"100755");
            return file.then(|| id.iter().map(|byte| format!("{byte:02x}")).collect());
        }
        tree = &tree[nul + 1 + width..];
    }
    None
}

/// The direnv folders whose failed removal is already logged.
static KEPT: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Logs `why` the daemon keeps `path`, once.
fn kept(path: PathBuf, why: &dyn std::fmt::Display) {
    if KEPT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(path.clone())
    {
        eprintln!("priory daemon: {}: {why}", path.display());
    }
}

/// Whether the folder under Priory `home` that holds each run's direnv folder is a symlink.
fn linked(home: &Path) -> bool {
    home.join("direnv").is_symlink()
}

/// Removes the direnv config folder under Priory `home` of each session that is not `live`. It logs
/// a folder it cannot remove once.
pub fn forget_direnv(home: &Path, live: &dyn Fn(&str) -> bool) {
    let root = home.join("direnv");
    if linked(home) {
        return kept(root, &"is a symlink, so nothing in it is removed");
    }
    for entry in fs::read_dir(root).into_iter().flatten().flatten() {
        if !entry.file_name().to_str().is_some_and(live)
            && let Err(error) = fs::remove_dir_all(entry.path())
        {
            kept(entry.path(), &error);
        }
    }
}

/// The person's own direnv config folder, as direnv finds it.
fn direnv_config() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    set("DIRENV_CONFIG").map(PathBuf::from).or_else(|| {
        set("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| set("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|config| config.join("direnv"))
    })
}

/// The person's data folder, where direnv keeps what they denied.
fn person_data() -> Option<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    set("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| set("HOME").map(|home| PathBuf::from(home).join(".local/share")))
}

/// `text`, a `direnv.toml`, without its `whitelist`, which skips direnv's check of the content.
fn unlisted(text: &str) -> Result<String, String> {
    let mut table: toml::Table = text
        .parse()
        .map_err(|error| format!("not TOML, at line {}", line(text, &error)))?;
    table.remove("whitelist");
    toml::to_string(&table).map_err(|error| error.to_string())
}

/// The worktree and its clone, each resolved through symlinks and `..`, when both are Priory's own:
/// a symlink below Priory home that leads out of `repos/<repo>` or `worktrees/<repo>` is refused.
fn own(home: &Path, repo: &str, worktree: &Path) -> Result<(PathBuf, PathBuf), String> {
    let real = |path: &Path| {
        fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))
    };
    let home = real(home)?;
    let worktree = real(worktree)?;
    let clone = home.join("repos").join(repo);
    let worktrees = home.join("worktrees").join(repo);
    if real(&clone)? != clone {
        return Err(format!("{} leads out of Priory home", clone.display()));
    }
    if worktree.parent() != Some(worktrees.as_path()) {
        return Err(format!(
            "{} is not a worktree folder under {}",
            worktree.display(),
            worktrees.display()
        ));
    }
    let common = clone_of(&worktree)
        .map(|common| real(&common))
        .transpose()?;
    if common.as_ref() != Some(&clone) {
        return Err(format!(
            "{} is not a worktree of {}",
            worktree.display(),
            clone.display()
        ));
    }
    Ok((worktree, clone))
}

/// Sets `projects["<path>"].hasTrustDialogAccepted` for each of `paths` in Claude Code's `config`.
/// A config that is a symlink, as a dotfiles checkout makes it, gets the grant in the file it leads
/// to, so the link stays.
pub fn claude(config: &Path, paths: &[&Path]) -> Vec<Grant> {
    let real = match fs::canonicalize(config) {
        Ok(real) => real,
        Err(error) => return grants(config, paths, Err(format!("{}: {error}", config.display()))),
    };
    let result = edit(&real, false, |text| {
        let mut value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
        let projects = value
            .as_object_mut()
            .ok_or("not a JSON object")?
            .entry("projects")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or("its projects is not an object")?;
        let mut changed = false;
        for path in paths {
            let project = projects
                .entry(path.display().to_string())
                .or_insert_with(|| serde_json::json!({}));
            if project["hasTrustDialogAccepted"] == true {
                continue;
            }
            project
                .as_object_mut()
                .ok_or("a project is not an object")?
                .insert("hasTrustDialogAccepted".into(), true.into());
            changed = true;
        }
        Ok(changed
            .then(|| serde_json::to_string_pretty(&value).expect("a JSON value serializes") + "\n"))
    });
    grants(config, paths, result)
}

/// Sets `projects."<clone>".trust_level = "trusted"` in Priory's Codex profile under `codex_home`,
/// unless the person's own `config.toml` marks the clone untrusted.
pub fn codex(codex_home: &Path, clone: &Path) -> Vec<Grant> {
    let key = clone.display().to_string();
    let profile = codex_home.join(format!("{CODEX_PROFILE}.config.toml"));
    let own = codex_home.join("config.toml");
    // A jailed run binds the profile read-only only when it exists, so it always does.
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&profile)
    {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            let why = format!("{}: {error}", profile.display());
            return grants(&profile, &[clone], Err(why));
        }
    }
    let table = match fs::read_to_string(&own) {
        Ok(text) => match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(error) => {
                let why = format!(
                    "{} is not TOML, at line {}",
                    own.display(),
                    line(&text, &error)
                );
                return grants(&profile, &[clone], Err(why));
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => toml::Table::new(),
        Err(error) => {
            let why = format!("{}: {error}", own.display());
            return grants(&profile, &[clone], Err(why));
        }
    };
    let level = (|| {
        table
            .get("projects")?
            .get(&key)?
            .get("trust_level")?
            .as_str()
    })();
    if level == Some("untrusted") {
        let why = format!("{} marks {key} untrusted", own.display());
        return grants(&profile, &[clone], Err(why));
    }
    // `-p` would replace the person's default profile, which then would not apply to the run.
    if let Some(name) = table.get("profile").and_then(toml::Value::as_str) {
        let why = format!(
            "{} sets profile {name}, which -p {CODEX_PROFILE} would replace",
            own.display()
        );
        return grants(&profile, &[clone], Err(why));
    }
    let result = edit(&profile, true, |text| {
        let mut table: toml::Table = text
            .parse()
            .map_err(|error| format!("not TOML, at line {}", line(text, &error)))?;
        let projects = table
            .entry("projects")
            .or_insert_with(|| toml::Table::new().into())
            .as_table_mut()
            .ok_or("its projects is not a table")?;
        let project = projects
            .entry(key.clone())
            .or_insert_with(|| toml::Table::new().into())
            .as_table_mut()
            .ok_or("a project is not a table")?;
        if project.get("trust_level").and_then(toml::Value::as_str) == Some("trusted") {
            return Ok(None);
        }
        project.insert("trust_level".into(), "trusted".into());
        Ok(Some(
            toml::to_string(&table).map_err(|error| error.to_string())?,
        ))
    });
    grants(&profile, &[clone], result)
}

/// The line a TOML error points at. The parser's own message quotes that line, which can hold a
/// token, so only its number reaches the trace.
fn line(text: &str, error: &toml::de::Error) -> usize {
    let start = error.span().map_or(0, |span| span.start.min(text.len()));
    text.as_bytes()[..start]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        + 1
}

/// Whether `config` still holds Claude Code's trust for each of `paths`.
pub fn claude_holds(config: &Path, paths: &[&Path]) -> Result<bool, String> {
    let text =
        fs::read_to_string(config).map_err(|error| format!("{}: {error}", config.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{}: not JSON: {error}", config.display()))?;
    Ok(paths.iter().all(|path| {
        value["projects"][path.display().to_string()]["hasTrustDialogAccepted"] == true
    }))
}

/// Whether Priory's Codex profile under `codex_home` still trusts `clone`.
pub fn codex_holds(codex_home: &Path, clone: &Path) -> Result<bool, String> {
    let profile = codex_home.join(format!("{CODEX_PROFILE}.config.toml"));
    let Some((text, _)) = load(&profile, true)? else {
        return Err(format!(
            "{} changed while Priory read it",
            profile.display()
        ));
    };
    let table: toml::Table = text.parse().map_err(|error| {
        format!(
            "{}: not TOML, at line {}",
            profile.display(),
            line(&text, &error)
        )
    })?;
    let level = (|| {
        table
            .get("projects")?
            .get(clone.display().to_string())?
            .get("trust_level")?
            .as_str()
    })();
    Ok(level == Some("trusted"))
}

fn grants(file: &Path, paths: &[&Path], result: Result<Outcome, String>) -> Vec<Grant> {
    paths
        .iter()
        .map(|path| Grant {
            path: path.to_path_buf(),
            file: file.to_path_buf(),
            outcome: *result.as_ref().unwrap_or(&Outcome::Refused),
            why: result.as_ref().err().cloned(),
        })
        .collect()
}

type Stamp = (u64, u64, i64, i64);

fn stamp_of(meta: &fs::Metadata) -> Stamp {
    (meta.ino(), meta.size(), meta.mtime(), meta.mtime_nsec())
}

/// What tells one version of `file` from another: its inode, size and modification time. A
/// symlink is refused: a run in a jail can write the folder, and a link there would make the
/// daemon rewrite a file outside the jail.
fn stamp(file: &Path) -> Result<Option<Stamp>, String> {
    match fs::symlink_metadata(file) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{} is a symlink, which Priory does not follow",
            file.display()
        )),
        // A FIFO or a device would block or never end the read.
        Ok(meta) if !meta.is_file() => Err(format!("{} is not a regular file", file.display())),
        Ok(meta) => Ok(Some(stamp_of(&meta))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", file.display())),
    }
}

/// The text of `file` and its stamp, or `None` when the file it opened is not the one it stamped,
/// as when another writer swapped it in between.
fn load(file: &Path, missing: bool) -> Result<Option<(String, Option<Stamp>)>, String> {
    let failed = |error: std::io::Error| format!("{}: {error}", file.display());
    let Some(before) = stamp(file)? else {
        return if missing {
            Ok(Some((String::new(), None)))
        } else {
            Err(format!("{}: no such file", file.display()))
        };
    };
    let mut opened = match fs::File::open(file) {
        Ok(opened) => opened,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    let mut text = String::new();
    std::io::Read::read_to_string(&mut opened, &mut text).map_err(failed)?;
    let meta = opened.metadata().map_err(failed)?;
    Ok((stamp_of(&meta).0 == before.0).then_some((text, Some(before))))
}

/// Rewrites `file` with `change`, which gives `None` when the text already holds the grant. Another
/// program, such as a running Claude Code, may write the file too, so Priory renames its copy only
/// when the file is still the version it read, and reads it back after. It starts again when the
/// file changed, up to `TRIES` times. A missing file is empty when `missing` allows it. It never
/// follows a symlink at `file`.
fn edit(
    file: &Path,
    missing: bool,
    change: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<Outcome, String> {
    let failed = |error: std::io::Error| format!("{}: {error}", file.display());
    let name = file.file_name().ok_or("the config has no file name")?;
    let copy = file.with_file_name(format!(
        ".{}.priory-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut wrote = false;
    // One more read than writes, so the last write is read back too.
    for attempt in 0..=TRIES {
        let Some((text, read)) = load(file, missing)? else {
            continue;
        };
        let Some(text) = change(&text).map_err(|why| format!("{}: {why}", file.display()))? else {
            return Ok(if wrote {
                Outcome::Granted
            } else {
                Outcome::Present
            });
        };
        if attempt == TRIES {
            break;
        }
        let mode =
            fs::symlink_metadata(file).map_or(0o600, |meta| meta.permissions().mode() & 0o7777);
        let _ = fs::remove_file(&copy);
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&copy)
            .and_then(|mut out| out.write_all(text.as_bytes()).and_then(|()| out.sync_all()))
            .map_err(failed)?;
        if stamp(file)? != read {
            let _ = fs::remove_file(&copy);
            continue;
        }
        fs::rename(&copy, file).map_err(failed)?;
        wrote = true;
    }
    let _ = fs::remove_file(&copy);
    Err(format!(
        "{} changed each time Priory wrote it, {TRIES} times",
        file.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A fresh folder of the test's own, removed when it drops.
    struct Temp(PathBuf);

    impl Temp {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp() -> Temp {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("priory-trust-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn sh_git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .env("HOME", dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .arg("-C")
            .arg(dir)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn only_a_worktree_of_priorys_own_clone_under_its_home_is_trusted() {
        let dir = temp();
        let home = dir.path().join("home");
        let clone = home.join("repos/o/r");
        fs::create_dir_all(&clone).unwrap();
        sh_git(&clone, &["init", "-q"]);
        sh_git(
            &clone,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "i",
            ],
        );
        fs::create_dir_all(home.join("worktrees/o/r")).unwrap();
        let worktree = home.join("worktrees/o/r/1");
        sh_git(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "w",
                worktree.to_str().unwrap(),
            ],
        );
        let outside = dir.path().join("elsewhere");
        sh_git(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "x",
                outside.to_str().unwrap(),
            ],
        );
        let link = home.join("worktrees/o/r/2");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let (real, real_clone) = own(&home, "o/r", &worktree).unwrap();
        assert_eq!(real, fs::canonicalize(&worktree).unwrap());
        assert_eq!(real_clone, fs::canonicalize(&clone).unwrap());
        assert!(
            own(&home, "o/r", &link)
                .unwrap_err()
                .contains("not a worktree folder")
        );
        assert!(own(&home, "o/r", &clone).is_err());
        assert!(own(&home, "o/s", &worktree).is_err());

        let away = dir.path().join("away");
        fs::create_dir_all(home.join("repos/p")).unwrap();
        std::os::unix::fs::symlink(&clone, home.join("repos/p/r")).unwrap();
        std::os::unix::fs::symlink(&away, home.join("worktrees/p")).unwrap();
        fs::create_dir_all(&away).unwrap();
        let elsewhere = away.join("r/1");
        fs::create_dir_all(away.join("r")).unwrap();
        sh_git(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "y",
                elsewhere.to_str().unwrap(),
            ],
        );
        let through = home.join("worktrees/p/r/1");
        assert!(
            own(&home, "p/r", &through)
                .unwrap_err()
                .contains("leads out")
        );
    }

    #[test]
    fn a_write_after_two_changes_is_granted_once_read_back() {
        let dir = temp();
        let config = dir.path().join("c.json");
        fs::write(&config, "{}").unwrap();
        let calls = Cell::new(0);
        let outcome = edit(&config, false, |text| {
            calls.set(calls.get() + 1);
            if text.contains("mine") {
                return Ok(None);
            }
            if calls.get() <= 2 {
                let other = dir.path().join("other");
                fs::write(&other, format!("{{\"n\":{}}}", calls.get())).unwrap();
                fs::rename(&other, &config).unwrap();
            }
            Ok(Some("{\"mine\":true}".into()))
        })
        .unwrap();
        assert_eq!(outcome, Outcome::Granted);
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn claude_writes_only_its_keys_and_keeps_the_rest() {
        let dir = temp();
        let config = dir.path().join(".claude.json");
        fs::write(
            &config,
            r#"{"oauthAccount":{"a":1},"projects":{"/w/1":{"allowedTools":["x"]}}}"#,
        )
        .unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let grants = claude(&config, &[Path::new("/w/1"), Path::new("/r")]);
        assert!(grants.iter().all(|grant| grant.outcome == Outcome::Granted));
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(value["oauthAccount"]["a"], 1);
        assert_eq!(value["projects"]["/w/1"]["allowedTools"][0], "x");
        assert_eq!(value["projects"]["/w/1"]["hasTrustDialogAccepted"], true);
        assert_eq!(value["projects"]["/r"]["hasTrustDialogAccepted"], true);
        let mode = fs::metadata(&config).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let again = claude(&config, &[Path::new("/w/1"), Path::new("/r")]);
        assert!(again.iter().all(|grant| grant.outcome == Outcome::Present));
    }

    #[test]
    fn a_symlinked_config_stays_a_symlink_and_its_target_gets_the_grant() {
        let dir = temp();
        let target = dir.path().join("dotfiles/claude.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, r#"{"theme":"dark"}"#).unwrap();
        let config = dir.path().join(".claude.json");
        std::os::unix::fs::symlink(&target, &config).unwrap();

        let grants = claude(&config, &[Path::new("/w/1")]);

        assert_eq!(grants[0].outcome, Outcome::Granted);
        assert!(
            fs::symlink_metadata(&config)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["projects"]["/w/1"]["hasTrustDialogAccepted"], true);
    }

    #[test]
    fn a_symlinked_codex_profile_is_refused_and_its_target_is_left_alone() {
        let dir = temp();
        let target = dir.path().join("Cargo.toml");
        fs::write(&target, "[package]\nname = \"x\"\n").unwrap();
        let codex_home = dir.path().join("codex");
        fs::create_dir_all(&codex_home).unwrap();
        let profile = codex_home.join("priory-trust.config.toml");
        std::os::unix::fs::symlink(&target, &profile).unwrap();

        let grants = codex(&codex_home, Path::new("/home/a/repos/o/r"));

        assert_eq!(grants[0].outcome, Outcome::Refused);
        assert!(grants[0].why.as_deref().unwrap().contains("is a symlink"));
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "[package]\nname = \"x\"\n"
        );
        assert!(
            fs::symlink_metadata(&profile)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_broken_profile_reports_its_line_and_not_its_text() {
        let dir = temp();
        fs::write(
            dir.path().join("priory-trust.config.toml"),
            "[projects]\ntoken = \"sk-secret-9\n",
        )
        .unwrap();
        let grants = codex(dir.path(), Path::new("/home/a/repos/o/r"));
        let why = grants[0].why.as_deref().unwrap();
        assert!(why.ends_with("not TOML, at line 2"), "{why}");
        assert!(!why.contains("sk-secret"));
    }

    #[test]
    fn a_direnv_toml_loses_its_whitelist_alone() {
        let text = "[global]\nwarn_timeout = \"1m\"\n[whitelist]\nprefix = [\"/w\"]\n";
        let table: toml::Table = unlisted(text).unwrap().parse().unwrap();
        assert_eq!(table["global"]["warn_timeout"].as_str(), Some("1m"));
        assert!(!table.contains_key("whitelist"));
    }

    #[test]
    fn a_symlinked_direnv_folder_loses_nothing() {
        let dir = temp();
        let [home, other] = ["home", "other"].map(|name| dir.path().join(name));
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(other.join("s")).unwrap();
        std::os::unix::fs::symlink(&other, home.join("direnv")).unwrap();
        forget_direnv(&home, &|_| false);
        assert!(other.join("s").exists());
    }

    /// Runs real direnv on the files `prepare` writes, as `launch` does.
    #[test]
    #[ignore = "needs direnv 2.38 or later on PATH"]
    fn real_direnv_loads_the_checked_bytes_alone() {
        let dir = temp();
        let root = fs::canonicalize(dir.path()).unwrap();
        let [real, link, folder, person, spool] =
            ["real", "link", "folder", "person", "spool"].map(|name| root.join(name));
        for made in [&real, &person, &spool] {
            fs::create_dir_all(made).unwrap();
        }
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // A whitelist over the worktree would load any content.
        fs::write(
            person.join("direnv.toml"),
            format!("[whitelist]\nprefix = [\"{}\"]\n", root.display()),
        )
        .unwrap();
        // The person's direnvrc runs after direnv's hash, as a writer that waits for it would.
        let late =
            "[[ ! -e $PRIORY_TEST_LATE ]] || echo 'export V=bad' > \"$PRIORY_TEST_WORK/.envrc\"\n";
        fs::write(person.join("direnvrc"), late).unwrap();
        let good = "export V=good D=${XDG_DATA_HOME-unset} P=$PWD\n";
        let run = |worktree: &Path, content: &str, late: bool, data: Option<&str>| {
            let envrcs = [&link, &real].map(|path| path.join(".envrc"));
            prepare(&folder, &person, good.as_bytes(), &envrcs).unwrap();
            fs::write(real.join(".envrc"), content).unwrap();
            let marker = root.join("late");
            let _ = fs::remove_file(&marker);
            let _ = fs::remove_file(spool.join("direnv.started"));
            if late {
                fs::write(&marker, "").unwrap();
            }
            let mut command = std::process::Command::new("sh");
            command
                .args(["-c", crate::runtime::DIRENV_EXEC])
                .args([&spool, worktree])
                .args(["sh", "-c"])
                .arg("echo ${V-none} ${D-none} ${P-none} ${XDG_DATA_HOME-unset} ${PRIORY_DATA_HOME-gone}")
                .env_remove("XDG_DATA_HOME")
                .env_remove("DIRENV_DIFF")
                .env("DIRENV_CONFIG", &folder)
                .env("PRIORY_TEST_LATE", &marker)
                .env("PRIORY_TEST_WORK", &real)
                .current_dir("/");
            if let Some(data) = data {
                command.env("XDG_DATA_HOME", data);
            }
            let out = command.output().unwrap();
            String::from_utf8(out.stdout).unwrap().trim().to_owned()
        };
        for worktree in [&real, &link] {
            let at = real.display();
            let loaded = run(worktree, good, false, Some("/d"));
            assert!(loaded.starts_with("good /d "), "{loaded}");
            assert!(loaded.ends_with(" /d gone"), "{loaded}");
            let unset = run(worktree, good, false, None);
            assert!(unset.starts_with("good unset "), "{unset}");
            assert!(unset.ends_with(" unset gone"), "{unset}");
            assert_eq!(
                run(worktree, "export V=bad\n", false, Some("/d")),
                "none none none /d gone",
                "{at}"
            );
            for _ in 0..20 {
                let late = run(worktree, good, true, Some("/d"));
                assert!(late.starts_with("good /d "), "{late}");
            }
            assert_eq!(
                fs::read_to_string(real.join(".envrc")).unwrap(),
                "export V=bad\n"
            );
        }
    }

    #[test]
    fn a_tree_names_the_regular_file_of_that_name_alone() {
        let tree = [
            b"100644 a\0AB".as_slice(),
            b"120000 .envrc\0CD",
            b"100755 z\0\x01\xff",
        ];
        let tree = tree.concat();
        assert_eq!(entry(&tree, b"a", 2).as_deref(), Some("4142"));
        assert_eq!(entry(&tree, b"z", 2).as_deref(), Some("01ff"));
        assert_eq!(entry(&tree, b".envrc", 2), None);
        assert_eq!(entry(&tree, b"b", 2), None);
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_as_a_tree_are_read_without_a_panic(
            tree in proptest::collection::vec(0u8..128, 0..80), cut in 0usize..40,
        ) {
            let id = entry(&tree, b".envrc", 20);
            proptest::prop_assert!(id.is_none_or(|id| id.len() == 40));
            let whole = [b"100644 .envrc\0".as_slice(), &[7; 20]].concat();
            let part = &whole[..cut.min(whole.len())];
            proptest::prop_assert_eq!(entry(part, b".envrc", 20).is_some(), cut >= whole.len());
        }

        #[test]
        fn a_direnv_toml_loses_its_whitelist_or_is_refused_without_panicking(
            text in "(?s).{0,200}|\\[whitelist\\]\nexact = \\[(\"[a-z/]{0,8}\",? ?){0,3}\\]\n",
        ) {
            if let Ok(kept) = unlisted(&text) {
                let table: toml::Table = kept.parse().unwrap();
                proptest::prop_assert!(!table.contains_key("whitelist"));
            }
        }

        #[test]
        fn a_toml_error_line_lies_within_the_text(text in "(?s).{0,200}") {
            if let Err(error) = text.parse::<toml::Table>() {
                let n = line(&text, &error);
                proptest::prop_assert!(n >= 1);
                proptest::prop_assert!(n <= text.matches('\n').count() + 1);
            }
        }
    }

    #[test]
    fn an_error_at_the_start_of_a_line_names_that_line() {
        let text = "a = 1\n= 2\n";
        let error = text.parse::<toml::Table>().unwrap_err();
        assert_eq!(line(text, &error), 2);
    }

    #[test]
    fn a_grant_check_tells_a_missing_key_from_a_config_it_cannot_read() {
        let dir = temp();
        let config = dir.path().join(".claude.json");
        fs::write(
            &config,
            r#"{"projects":{"/w":{"hasTrustDialogAccepted":true}}}"#,
        )
        .unwrap();
        assert_eq!(claude_holds(&config, &[Path::new("/w")]), Ok(true));
        assert_eq!(
            claude_holds(&config, &[Path::new("/w"), Path::new("/r")]),
            Ok(false)
        );
        fs::write(&config, "{broken").unwrap();
        assert!(claude_holds(&config, &[Path::new("/w")]).is_err());
        assert!(claude_holds(&dir.path().join("gone.json"), &[Path::new("/w")]).is_err());

        let clone = Path::new("/home/a/repos/o/r");
        assert_eq!(codex(dir.path(), clone)[0].outcome, Outcome::Granted);
        assert_eq!(codex_holds(dir.path(), clone), Ok(true));
        assert_eq!(codex_holds(dir.path(), Path::new("/other")), Ok(false));
        fs::write(dir.path().join("priory-trust.config.toml"), "[broken").unwrap();
        assert!(codex_holds(dir.path(), clone).is_err());
    }

    #[test]
    fn a_fifo_in_place_of_the_profile_is_refused_without_blocking() {
        let dir = temp();
        let profile = dir.path().join("priory-trust.config.toml");
        let made = std::process::Command::new("mkfifo")
            .arg(&profile)
            .status()
            .unwrap();
        assert!(made.success());
        let grants = codex(dir.path(), Path::new("/home/a/repos/o/r"));
        assert_eq!(grants[0].outcome, Outcome::Refused);
        assert!(
            grants[0]
                .why
                .as_deref()
                .unwrap()
                .contains("not a regular file")
        );
    }

    #[test]
    fn codex_refuses_a_config_toml_it_cannot_parse() {
        let dir = temp();
        fs::write(
            dir.path().join("config.toml"),
            "model = \"x\"\nheaders = { Authorization = \"Bearer sk-secret-123 }\n",
        )
        .unwrap();
        let grants = codex(dir.path(), Path::new("/home/a/repos/o/r"));
        assert_eq!(grants[0].outcome, Outcome::Refused);
        let why = grants[0].why.as_deref().unwrap();
        assert!(why.ends_with("is not TOML, at line 2"), "{why}");
        assert!(!why.contains("sk-secret"));
        assert_eq!(
            fs::read_to_string(dir.path().join("priory-trust.config.toml")).unwrap(),
            ""
        );
    }

    #[test]
    fn claude_refuses_a_missing_or_broken_config() {
        let dir = temp();
        let config = dir.path().join(".claude.json");
        let grants = claude(&config, &[Path::new("/w/1")]);
        assert_eq!(grants[0].outcome, Outcome::Refused);
        fs::write(&config, "[1]").unwrap();
        let grants = claude(&config, &[Path::new("/w/1")]);
        assert_eq!(grants[0].outcome, Outcome::Refused);
        assert_eq!(fs::read_to_string(&config).unwrap(), "[1]");
    }

    #[test]
    fn a_write_between_the_read_and_the_rename_survives() {
        let dir = temp();
        let config = dir.path().join(".claude.json");
        fs::write(&config, r#"{"projects":{}}"#).unwrap();
        let calls = Cell::new(0);
        let outcome = edit(&config, false, |text| {
            calls.set(calls.get() + 1);
            let mut value: serde_json::Value = serde_json::from_str(text).unwrap();
            if value["projects"]["/w"] == true {
                return Ok(None);
            }
            if calls.get() == 1 {
                // Another writer replaces the file after Priory read it, as Claude Code does.
                let other = dir.path().join("other");
                fs::write(&other, r#"{"projects":{},"refreshed":"token"}"#).unwrap();
                fs::rename(&other, &config).unwrap();
            }
            value["projects"]["/w"] = true.into();
            Ok(Some(value.to_string()))
        })
        .unwrap();
        assert_eq!(outcome, Outcome::Granted);
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(value["refreshed"], "token");
        assert_eq!(value["projects"]["/w"], true);
        assert_eq!(
            calls.get(),
            3,
            "read, read again after the change, read back"
        );
    }

    #[test]
    fn a_file_that_keeps_changing_is_refused() {
        let dir = temp();
        let config = dir.path().join("c.json");
        fs::write(&config, "{}").unwrap();
        let calls = Cell::new(0);
        let result = edit(&config, false, |_| {
            calls.set(calls.get() + 1);
            fs::write(&config, format!("{{\"n\":{}}}", calls.get())).unwrap();
            Ok(Some("{\"mine\":true}".into()))
        });
        assert!(result.unwrap_err().contains("3 times"));
        assert!(!fs::read_to_string(&config).unwrap().contains("mine"));
    }

    #[test]
    fn codex_writes_its_profile_and_respects_an_untrusted_clone() {
        let dir = temp();
        let clone = Path::new("/home/a/repos/o/r");
        let grants = codex(dir.path(), clone);
        assert_eq!(grants[0].outcome, Outcome::Granted);
        let profile = dir.path().join("priory-trust.config.toml");
        let table: toml::Table = fs::read_to_string(&profile).unwrap().parse().unwrap();
        assert_eq!(
            table["projects"]["/home/a/repos/o/r"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert_eq!(codex(dir.path(), clone)[0].outcome, Outcome::Present);
        let other = Path::new("/home/a/repos/o/s");
        fs::write(
            dir.path().join("config.toml"),
            "[projects.\"/home/a/repos/o/s\"]\ntrust_level = \"untrusted\"\n",
        )
        .unwrap();
        let grants = codex(dir.path(), other);
        assert_eq!(grants[0].outcome, Outcome::Refused);
        assert!(!fs::read_to_string(&profile).unwrap().contains("o/s"));
        fs::write(dir.path().join("config.toml"), "profile = \"work\"\n").unwrap();
        let grants = codex(dir.path(), clone);
        assert_eq!(grants[0].outcome, Outcome::Refused);
        assert!(grants[0].why.as_deref().unwrap().contains("profile work"));
    }
}
