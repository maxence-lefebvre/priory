//! The bubblewrap jail a sandboxed workflow's run starts its harness in. Every path is bound at the
//! same path inside as outside, so a path a hook sends, a path a person trusted and a path the sweep
//! reads in `/proc` mean the same file on both sides.

use crate::runtime::{Harness, Launch, Own, fed_within, git, one_line, run, socket_path};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A sandbox of `factory.toml`, with its paths as written.
#[derive(Debug, Clone, Default)]
pub struct Jail {
    /// Paths bound read-only, each absolute or under `~/`.
    pub read: Vec<String>,
    /// Paths bound read-write, each absolute or under `~/`.
    pub write: Vec<String>,
    /// The daemon's variables the run keeps, by name.
    pub env: Vec<String>,
}

/// The variables every jailed run keeps, beside the ones its sandbox names and the ones Priory
/// gives the run.
const KEPT: [&str; 7] = [
    "PATH",
    "LANG",
    "TERM",
    "HOME",
    "XDG_RUNTIME_DIR",
    "PRIORY_HOME",
    "PRIORY_SESSION",
];

/// Folders the jail replaces with empty ones: the person's homes and the shared temporary folders.
const HIDDEN: [&str; 4] = ["/home", "/root", "/tmp", "/run/user"];

/// Whether this host can start a jail: `bwrap --ro-bind / / true` exits 0. The error is the
/// reason a jailed task blocks with.
pub fn probe() -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("bubblewrap needs Linux".to_owned());
    }
    let mut command = Command::new("bwrap");
    command.args(["--ro-bind", "/", "/", "true"]);
    let output = fed_within(&mut command, None, Duration::from_secs(30))
        .map_err(|error| format!("bwrap could not run: {error}"))?
        .ok_or_else(|| "bwrap --ro-bind / / true timed out after 30s".to_owned())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = one_line(&String::from_utf8_lossy(&output.stderr));
    Err(if stderr.is_empty() {
        format!("bwrap --ro-bind / / true exited with {}", output.status)
    } else {
        stderr
    })
}

/// Whether `program` is an executable file in a folder of `PATH`.
pub fn on_path(program: &str) -> bool {
    find(program).is_some()
}

fn find(program: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|file| {
            file.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

/// `~/x` under `home`, or the path as written.
fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => home.map(|home| home.join(rest)),
        None => Some(PathBuf::from(path)),
    }
}

/// The path with its links resolved, or as given when it does not exist.
fn real(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// Whether `path` lies in a folder the jail replaces with an empty one.
fn hidden(path: &Path, home: Option<&Path>) -> bool {
    HIDDEN.iter().any(|dir| path.starts_with(dir))
        || home.is_some_and(|home| path.starts_with(home))
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Read,
    Write,
    /// A new empty folder in memory, gone when the run ends.
    Empty,
}

struct Binds(Vec<(PathBuf, Mode, bool)>);

impl Binds {
    /// Binds `path` at its own path, and at its resolved path when a link leads elsewhere.
    fn add(&mut self, path: &Path, mode: Mode, required: bool) {
        let resolved = real(path);
        if resolved != path {
            self.0.push((resolved, mode, required));
        }
        self.0.push((path.to_owned(), mode, required));
    }

    /// The arguments, parents first, so a folder's bind never hides one inside it. An empty folder
    /// goes after a bind of the same path, so nothing binds the state file's folder back.
    fn args(mut self) -> Vec<String> {
        self.0
            .sort_by_key(|(path, mode, _)| (path.components().count(), *mode == Mode::Empty));
        let mut args = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (path, mode, required) in self.0 {
            let flag = match (mode, required) {
                (Mode::Read, true) => "--ro-bind",
                (Mode::Read, false) => "--ro-bind-try",
                (Mode::Write, true) => "--bind",
                (Mode::Write, false) => "--bind-try",
                (Mode::Empty, _) => "--tmpfs",
            };
            if !seen.insert((path.clone(), flag)) {
                continue;
            }
            let shown = path.display().to_string();
            args.extend([flag.to_owned(), shown.clone()]);
            if mode != Mode::Empty {
                args.push(shown);
            }
        }
        args
    }
}

/// The run's own spool, which its hooks append to while no daemon listens.
pub fn spool_dir(home: &Path, session: &str) -> PathBuf {
    home.join("spools").join(session)
}

/// The folder that holds the marker of `session`'s news. Only the daemon writes it: a jail binds it
/// read-only.
pub fn news_dir(home: &Path, session: &str) -> PathBuf {
    home.join("news").join(session)
}

impl Jail {
    /// The command that starts `launch`'s harness in the jail: `bwrap`, its binds and a shell that
    /// clears every variable the run does not keep, before the harness's own command. `files` are
    /// the files the harness reads at start.
    pub(crate) fn command(
        &self,
        harness: &dyn Harness,
        exe: &Path,
        launch: &Launch,
        files: &[PathBuf],
    ) -> Result<Vec<String>, String> {
        let user = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from);
        let user = user.as_deref();
        let home = real(launch.home);
        let mut args: Vec<String> = [
            "bwrap",
            "--die-with-parent",
            "--new-session",
            "--unshare-pid",
            "--ro-bind",
            "/",
            "/",
            "--dev-bind",
            "/dev",
            "/dev",
            "--proc",
            "/proc",
        ]
        .map(str::to_owned)
        .into();
        for dir in HIDDEN.iter().filter(|dir| Path::new(dir).is_dir()) {
            args.extend(["--tmpfs".to_owned(), (*dir).to_owned()]);
        }
        if let Some(user) = user.filter(|user| !HIDDEN.iter().any(|dir| user.starts_with(dir))) {
            args.extend(["--tmpfs".to_owned(), user.display().to_string()]);
        }
        let mut binds = Binds(Vec::new());
        binds.add(launch.dir, Mode::Write, true);
        git_folders(&mut binds, launch.dir)?;
        for (path, own) in harness.own_home(user) {
            if own != Own::File {
                std::fs::create_dir_all(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
            }
            let mode = match own {
                Own::Read | Own::File => Mode::Read,
                Own::Write => Mode::Write,
                Own::Scratch => Mode::Empty,
            };
            binds.add(&path, mode, false);
        }
        if let Some(folder) = state_folder()? {
            binds.add(&folder, Mode::Empty, true);
        }
        for file in files {
            binds.add(file, Mode::Read, true);
        }
        binds.add(exe, Mode::Read, true);
        for dir in launch.plugin.iter().chain(&launch.memories) {
            binds.add(dir, Mode::Read, true);
        }
        for variable in launch.env {
            if let Some(file) = variable.strip_prefix("PRIORY_PATHS=") {
                binds.add(Path::new(file), Mode::Read, true);
            }
        }
        if let Some(folder) = launch.direnv {
            binds.add(folder, Mode::Read, true);
            // Its links lead to the person's, which the jail empties with their home.
            for name in [crate::trust::PERSONS_DIRENVRC, "lib"] {
                if let Ok(target) = std::fs::canonicalize(folder.join(name)) {
                    binds.add(&target, Mode::Read, true);
                }
            }
        }
        let programs = std::iter::once(harness.program())
            .chain(launch.direnv.map(|_| "direnv"))
            .filter_map(find);
        // A program, when it lives in a folder the jail empties, such as ~/.local/bin.
        let mut link = Vec::new();
        for program in programs {
            let resolved = real(&program);
            if hidden(&resolved, user) {
                let dir = resolved.parent().expect("a program has a folder");
                binds.add(dir, Mode::Read, true);
            }
            if resolved != program && hidden(&program, user) {
                let folder = program.parent().expect("a program has a folder");
                link.extend([
                    "--dir".to_owned(),
                    folder.display().to_string(),
                    "--symlink".to_owned(),
                    resolved.display().to_string(),
                    program.display().to_string(),
                ]);
            }
        }
        let socket = socket_path(launch.home);
        binds.add(
            socket.parent().expect("the socket has a folder"),
            Mode::Write,
            true,
        );
        // A file in it becomes the daemon's binary.
        crate::upgrade::private(&home)?;
        binds.add(&crate::upgrade::dir(&home), Mode::Empty, true);
        let spool = spool_dir(&home, launch.session);
        std::fs::create_dir_all(&spool).map_err(|error| format!("{}: {error}", spool.display()))?;
        binds.add(&spool, Mode::Write, true);
        let news = news_dir(&home, launch.session);
        std::fs::create_dir_all(&news).map_err(|error| format!("{}: {error}", news.display()))?;
        binds.add(&news, Mode::Read, true);
        for (paths, mode) in [(&self.read, Mode::Read), (&self.write, Mode::Write)] {
            for path in paths.iter().filter_map(|path| expand(path, user)) {
                binds.add(&path, mode, false);
            }
        }
        args.extend(binds.args());
        args.extend(link);
        args.extend(["--chdir".to_owned(), launch.dir.display().to_string()]);
        // The shell runs in the jail, so the variables it keeps are the pane's own.
        let mut kept: Vec<&str> = KEPT.to_vec();
        kept.extend(harness.home_env());
        if launch.direnv.is_some() {
            kept.push("DIRENV_CONFIG");
        }
        kept.extend(
            launch
                .env
                .iter()
                .filter_map(|variable| variable.split_once('=').map(|(name, _)| name)),
        );
        kept.extend(self.env.iter().map(String::as_str));
        let script: String = kept
            .iter()
            .map(|name| format!(" ${{{name}+\"{name}=${name}\"}}"))
            .collect();
        args.extend([
            "--".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            format!("exec env -i{script} \"$@\""),
            "sh".to_owned(),
            format!("PRIORY_HOME={}", home.display()),
            format!("PRIORY_SPOOL={}", spool.join("signals.spool").display()),
        ]);
        Ok(args)
    }
}

/// Binds the git folders of the worktree `dir`. The run commits, so the clone's common folder is
/// writable. Its config, its hooks, the other worktrees' folders and the links that lead git from
/// the worktree to them are read-only, since the daemon's git on the host reads them.
fn git_folders(binds: &mut Binds, dir: &Path) -> Result<(), String> {
    let found = run(git(dir).args([
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
        "--absolute-git-dir",
    ]))?;
    let mut lines = found.lines().map(PathBuf::from);
    let (Some(common), Some(own)) = (lines.next(), lines.next()) else {
        return Err(format!(
            "git rev-parse in {} printed {found:?}",
            dir.display()
        ));
    };
    let hooks = common.join("hooks");
    std::fs::create_dir_all(&hooks).map_err(|error| format!("{}: {error}", hooks.display()))?;
    // Git reads a `commondir` in any git folder as the folder that holds its config, so the run
    // must not write one. A bind needs the file, and git refuses an empty one, so it names itself.
    pin(&common.join("commondir"), ".\n")?;
    pin(&common.join("config.worktree"), "")?;
    binds.add(&common, Mode::Write, true);
    for name in ["config", "hooks", "commondir", "config.worktree"] {
        binds.add(&common.join(name), Mode::Read, true);
    }
    if own != common {
        pin(&own.join("config.worktree"), "")?;
        binds.add(&common.join("worktrees"), Mode::Read, true);
        binds.add(&own, Mode::Write, true);
        for name in ["commondir", "gitdir", "config.worktree"] {
            binds.add(&own.join(name), Mode::Read, true);
        }
        binds.add(&dir.join(".git"), Mode::Read, true);
    }
    Ok(())
}

/// Writes `text` to `file` when it is missing, so the jail can bind it read-only. A file that
/// exists keeps its content: the bind keeps every jail from writing it.
fn pin(file: &Path, text: &str) -> Result<(), String> {
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
    {
        Ok(mut created) => std::io::Write::write_all(&mut created, text.as_bytes()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
    .map_err(|error| format!("{}: {error}", file.display()))
}

/// The folder of the state file `PRIORY_DB` names, which the jail empties: the run's MCP
/// configuration names the file, and its folder also holds its `-wal` file.
fn state_folder() -> Result<Option<PathBuf>, String> {
    let Some(db) = std::env::var_os("PRIORY_DB").filter(|db| {
        let name = db.to_string_lossy();
        !name.is_empty() && name != ":memory:" && !name.starts_with("file:")
    }) else {
        return Ok(None);
    };
    let db = std::path::absolute(&db).map_err(|error| format!("PRIORY_DB: {error}"))?;
    let db = std::fs::canonicalize(&db).unwrap_or(db);
    match db.parent() {
        Some(folder) if folder.parent().is_some() => Ok(Some(folder.to_owned())),
        _ => Err(format!(
            "the state file {} sits in /, which a jail cannot hide: move it to a folder of its own",
            db.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_home_path_expands_under_the_home_and_an_absolute_one_stays() {
        let home = Path::new("/home/a");
        assert_eq!(
            expand("~/.cargo", Some(home)),
            Some(PathBuf::from("/home/a/.cargo"))
        );
        assert_eq!(expand("/opt/x", Some(home)), Some(PathBuf::from("/opt/x")));
        assert_eq!(expand("~/.cargo", None), None);
    }

    #[test]
    fn a_codex_run_reads_priorys_trust_profile_but_cannot_replace_it() {
        let mut binds = Binds(Vec::new());
        // CODEX_HOME may move the folder, so the test reads it back from `own_home`.
        let home = Path::new("/nonexistent/h");
        let own = crate::runtime::Codex.own_home(Some(home));
        let dir = own[0].0.clone();
        assert_eq!(own[1], (dir.join("priory-trust.config.toml"), Own::File));
        for (path, own) in own {
            let mode = if own == Own::Write {
                Mode::Write
            } else {
                Mode::Read
            };
            binds.add(&path, mode, false);
        }
        let shown = |path: &Path| path.display().to_string();
        let profile = shown(&dir.join("priory-trust.config.toml"));
        let dir = shown(&dir);
        assert_eq!(
            binds.args(),
            [
                "--bind-try",
                &dir,
                &dir,
                "--ro-bind-try",
                &profile,
                &profile
            ]
        );
    }

    #[test]
    fn binds_go_parents_first_so_none_hides_another() {
        let mut binds = Binds(Vec::new());
        binds.add(Path::new("/nonexistent/a/b/worktree"), Mode::Write, true);
        binds.add(Path::new("/nonexistent/a"), Mode::Read, false);
        assert_eq!(
            binds.args(),
            [
                "--ro-bind-try",
                "/nonexistent/a",
                "/nonexistent/a",
                "--bind",
                "/nonexistent/a/b/worktree",
                "/nonexistent/a/b/worktree"
            ]
        );
    }
}
