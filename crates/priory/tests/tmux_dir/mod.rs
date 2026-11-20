use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// A test's `TMUX_TMPDIR`, under `/tmp` so the socket path stays within the 108 bytes of a Unix
/// socket wherever the worktree lives. Dropping it kills its tmux server and removes it, so a
/// server that starts later has no directory to start in.
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

    /// Kills every process started with this directory, the tmux server among them, then the
    /// server. A child the daemon left in its own process group can start a server after any
    /// `kill-server`, so the processes are found by the `TMUX_TMPDIR` they inherit.
    pub fn kill(&self) {
        let mut marker = format!("TMUX_TMPDIR={}", self.0.display()).into_bytes();
        marker.push(0);
        // Bounded, so a process that will not die cannot hang the suite in `Drop`.
        for _ in 0..100 {
            let Ok(processes) = fs::read_dir("/proc") else {
                break;
            };
            let found: Vec<String> = processes
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|pid| pid.bytes().all(|byte| byte.is_ascii_digit()))
                .filter(|pid| {
                    fs::read(format!("/proc/{pid}/environ")).is_ok_and(|environ| {
                        environ
                            .split_inclusive(|byte| *byte == 0)
                            .any(|variable| variable == marker)
                    })
                })
                .collect();
            if found.is_empty() {
                break;
            }
            let _ = Command::new("kill").arg("-9").args(&found).output();
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = Command::new("tmux")
            .env_remove("TMUX")
            .env("TMUX_TMPDIR", &self.0)
            .arg("kill-server")
            .output();
    }
}

impl Drop for TmuxDir {
    fn drop(&mut self) {
        self.kill();
        let _ = fs::remove_dir_all(&self.0);
    }
}
