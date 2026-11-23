use std::path::{Path, PathBuf};

/// The cgroup v2 path of process `pid`, from `/proc/<pid>/cgroup` or `PRIORY_PROC`'s, when it is a
/// systemd scope inside the slice `slice`. A process outside such a scope has none.
pub fn scope_of(pid: u32, slice: &str) -> Option<String> {
    let text =
        std::fs::read_to_string(crate::room::proc().join(pid.to_string()).join("cgroup")).ok()?;
    scope_in(&text, slice)
}

/// The scope path in the text of a `/proc/<pid>/cgroup` file, when its unified line names a scope
/// inside the slice `slice`.
pub fn scope_in(text: &str, slice: &str) -> Option<String> {
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();
    let mut parts = path.rsplit('/');
    let scope = parts.next()?;
    // The slice holds the scope, so a nested slice such as `priory-agents.slice` is its parent.
    (scope.ends_with(".scope") && parts.next() == Some(slice)).then(|| path.to_owned())
}

/// The memory the cgroup `path` uses now, from its `memory.current` under `/sys/fs/cgroup`, or
/// under `PRIORY_CGROUP` when set.
pub fn memory(path: &str) -> Option<u64> {
    memory_in(&root(), path)
}

/// The memory the cgroup `path` uses now, from its `memory.current` under `root`.
pub fn memory_in(root: &Path, path: &str) -> Option<u64> {
    let file = root
        .join(path.trim_start_matches('/'))
        .join("memory.current");
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

fn root() -> PathBuf {
    std::env::var_os("PRIORY_CGROUP")
        .filter(|root| !root.is_empty())
        .map_or_else(|| PathBuf::from("/sys/fs/cgroup"), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "/user.slice/user-1000.slice/user@1000.service/priory.slice/priory-agents.slice/run-r1.scope";

    #[test]
    fn a_scope_inside_the_slice_has_a_path() {
        let text = format!("0::{SCOPE}\n");
        assert_eq!(
            scope_in(&text, "priory-agents.slice").as_deref(),
            Some(SCOPE)
        );
    }

    #[test]
    fn a_process_outside_a_scope_of_the_slice_has_none() {
        let service =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/priory-daemon.service\n";
        assert_eq!(scope_in(service, "priory-agents.slice"), None);
        let other = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/run-r2.scope\n";
        assert_eq!(scope_in(other, "priory-agents.slice"), None);
        // cgroup v1 has no unified line.
        assert_eq!(
            scope_in("4:memory:/user.slice\n", "priory-agents.slice"),
            None
        );
        assert_eq!(scope_in("", "priory-agents.slice"), None);
    }

    #[test]
    fn memory_reads_memory_current_under_the_root() {
        let root = std::env::temp_dir().join(format!("priory-cgroup-{}", std::process::id()));
        let dir = root.join(SCOPE.trim_start_matches('/'));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("memory.current"), "4294967296\n").unwrap();

        assert_eq!(memory_in(&root, SCOPE), Some(4 << 30));
        assert_eq!(memory_in(&root, "/user.slice/gone.scope"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
