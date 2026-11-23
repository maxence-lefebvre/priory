//! The folder trust Priory records for its own worktrees and clones, when the factory turns on
//! `trust_worktrees`, so no run stops on its harness's trust dialog (docs/daemon/sessions.md).

use crate::runtime::{Harness, clone_of};
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

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
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Present => "present",
            Self::Refused => "refused",
            Self::Missed => "missed",
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

/// Makes direnv trust `worktree` of `repo` and its clone through `folder`, a config folder of
/// Priory's own: the person's `direnv.toml` with both paths in `[whitelist] exact`, and links to
/// their `direnvrc` and `lib`. No grant when `direnv` is not on PATH or the worktree has no `.envrc`.
pub fn direnv(home: &Path, repo: &str, worktree: &Path, folder: &Path) -> Vec<Grant> {
    if !crate::jail::on_path("direnv") || !worktree.join(".envrc").is_file() {
        return Vec::new();
    }
    let (worktree, clone) = match own(home, repo, worktree) {
        Ok(paths) => paths,
        Err(why) => return grants(folder, &[worktree], Err(why)),
    };
    let file = folder.join("direnv.toml");
    let person = direnv_config();
    let result = person
        .as_deref()
        .ok_or_else(|| "no DIRENV_CONFIG, XDG_CONFIG_HOME or HOME".to_owned())
        .and_then(|person| {
            let own = ["direnv.toml", "config.toml"]
                .map(|name| person.join(name))
                .into_iter()
                .find(|file| file.is_file());
            let text = match &own {
                Some(own) => fs::read_to_string(own)
                    .map_err(|error| format!("{}: {error}", own.display()))?,
                None => String::new(),
            };
            let text = whitelisted(&text, &[&worktree, &clone]).map_err(|why| match &own {
                Some(own) => format!("{}: {why}", own.display()),
                None => why,
            })?;
            let failed = |error: std::io::Error| format!("{}: {error}", folder.display());
            let _ = fs::remove_dir_all(folder);
            fs::create_dir_all(folder).map_err(failed)?;
            fs::write(&file, text).map_err(failed)?;
            for name in ["direnvrc", "lib"] {
                let target = person.join(name);
                if target.exists() {
                    std::os::unix::fs::symlink(&target, folder.join(name)).map_err(failed)?;
                }
            }
            Ok(Outcome::Granted)
        });
    grants(&file, &[&worktree, &clone], result)
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

/// `text`, a `direnv.toml`, with each of `paths` in `[whitelist] exact`.
fn whitelisted(text: &str, paths: &[&Path]) -> Result<String, String> {
    let mut table: toml::Table = text
        .parse()
        .map_err(|error| format!("not TOML, at line {}", line(text, &error)))?;
    let exact = table
        .entry("whitelist")
        .or_insert_with(|| toml::Table::new().into())
        .as_table_mut()
        .ok_or("its whitelist is not a table")?
        .entry("exact")
        .or_insert_with(|| toml::value::Array::new().into())
        .as_array_mut()
        .ok_or("its whitelist.exact is not an array")?;
    for path in paths {
        let path: toml::Value = path.display().to_string().into();
        if !exact.contains(&path) {
            exact.push(path);
        }
    }
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

    fn git(dir: &Path, args: &[&str]) {
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
        git(&clone, &["init", "-q"]);
        git(
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
        git(
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
        git(
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
        git(
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
    fn a_direnv_whitelist_keeps_the_persons_keys_and_adds_each_path_once() {
        let text =
            "[global]\nwarn_timeout = \"1m\"\n[whitelist]\nprefix = [\"/w\"]\nexact = [\"/a\"]\n";
        let merged = whitelisted(text, &[Path::new("/a"), Path::new("/b")]).unwrap();
        let table: toml::Table = merged.parse().unwrap();
        assert_eq!(table["global"]["warn_timeout"].as_str(), Some("1m"));
        assert_eq!(table["whitelist"]["prefix"], toml::Value::from(vec!["/w"]));
        assert_eq!(
            table["whitelist"]["exact"],
            toml::Value::from(vec!["/a", "/b"])
        );
        assert_eq!(
            whitelisted("whitelist = 1\n", &[Path::new("/a")]),
            Err("its whitelist is not a table".into())
        );
    }

    proptest::proptest! {
        #[test]
        fn a_direnv_whitelist_merge_returns_or_refuses_without_panicking(
            text in "(?s).{0,200}|\\[whitelist\\]\nexact = \\[(\"[a-z/]{0,8}\",? ?){0,3}\\]\n",
            path in "/[a-z/]{0,12}",
        ) {
            if let Ok(merged) = whitelisted(&text, &[Path::new(&path)]) {
                let table: toml::Table = merged.parse().unwrap();
                let exact = table["whitelist"]["exact"].as_array().unwrap();
                proptest::prop_assert!(exact.contains(&toml::Value::from(path.as_str())));
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
