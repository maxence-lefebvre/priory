use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A test's `TMUX_TMPDIR`, under `/tmp` so the socket path stays within the 108 bytes of a Unix
/// socket wherever the worktree lives. Dropping it kills every process that inherited it, its tmux
/// server among them, and removes it. tmux falls back to `/tmp` when `TMUX_TMPDIR` is gone, so a
/// process that escaped the kill would start its server on the shared default socket.
pub struct TmuxDir(PathBuf);

impl TmuxDir {
    /// The directory for the test whose files live in `data`, emptied of what a previous run left.
    pub fn new(data: &Path) -> Self {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        let dir = TmuxDir(PathBuf::from(format!(
            "/tmp/priory-test-{:016x}",
            hasher.finish()
        )));
        dir.kill();
        let _ = fs::remove_dir_all(&dir.0);
        fs::DirBuilder::new().mode(0o700).create(&dir.0).unwrap();
        dir
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The default socket of a tmux server started in this directory, while the directory exists.
    pub fn socket(&self) -> Option<PathBuf> {
        let uid = fs::metadata(&self.0).ok()?.uid();
        Some(self.0.join(format!("tmux-{uid}")).join("default"))
    }

    /// Kills every process started with this directory, the tmux server among them, then the
    /// server. A child the daemon left in its own process group can start a server after any
    /// `kill-server`, so the processes are found by the `TMUX_TMPDIR` they inherit.
    pub fn kill(&self) {
        // Bounded, so a process that will not die cannot hang the suite in `Drop`.
        for _ in 0..100 {
            let Some(found) = inheriting(&self.0) else {
                break;
            };
            if found.is_empty() {
                break;
            }
            let _ = Command::new("kill").arg("-9").args(&found).output();
            std::thread::sleep(Duration::from_millis(20));
        }
        // Through the directory's own socket: a bare kill-server can reach the person's server.
        if let Some(socket) = self.socket() {
            let _ = Command::new("tmux")
                .env_remove("TMUX")
                .arg("-S")
                .arg(socket)
                .arg("kill-server")
                .output();
        }
    }
}

impl Drop for TmuxDir {
    fn drop(&mut self) {
        self.kill();
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The live processes that inherited `TMUX_TMPDIR` set to `dir`, or `None` without `/proc`. A
/// process inside `execve` reads an empty environment until its new one is in place, so a scan
/// that meets one waits and scans again rather than miss it.
pub fn inheriting(dir: &Path) -> Option<Vec<String>> {
    let mut marker = format!("TMUX_TMPDIR={}", dir.display()).into_bytes();
    marker.push(0);
    let mut found = Vec::new();
    // Bounded, so a process that clears its own command line cannot hang the suite.
    for _ in 0..500 {
        found.clear();
        let mut starting = false;
        for entry in fs::read_dir("/proc").ok()? {
            let Some(pid) = entry
                .ok()
                .and_then(|entry| entry.file_name().into_string().ok())
            else {
                continue;
            };
            if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            match inherits(&pid, &marker) {
                Some(true) => found.push(pid),
                Some(false) => {}
                None => starting = true,
            }
        }
        if !starting {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Some(found)
}

/// Whether process `pid` carries `marker` in its environment, or `None` while it is inside
/// `execve`. A zombie or kernel thread has no executable, so it never reads as inside `execve`.
fn inherits(pid: &str, marker: &[u8]) -> Option<bool> {
    let carries = |environ: &[u8]| {
        environ
            .split_inclusive(|byte| *byte == 0)
            .any(|variable| variable == marker)
    };
    // Another user's process, or one already gone.
    let Ok(environ) = fs::read(format!("/proc/{pid}/environ")) else {
        return Some(false);
    };
    if !environ.is_empty() {
        return Some(carries(&environ));
    }
    // Read before the environment again: the kernel fills both in the same step of `execve`.
    let arguments = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let program = fs::read_link(format!("/proc/{pid}/exe")).is_ok();
    match fs::read(format!("/proc/{pid}/environ")) {
        Ok(environ) if !environ.is_empty() => Some(carries(&environ)),
        Ok(_) if arguments.is_empty() && program => None,
        _ => Some(false),
    }
}
