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

/// Whether this process runs inside the slice `slice`, at any depth.
pub fn inside(slice: &str) -> bool {
    std::fs::read_to_string(crate::room::proc().join("self/cgroup")).is_ok_and(|text| {
        text.lines()
            .filter_map(|line| line.strip_prefix("0::"))
            .any(|path| path.trim().split('/').any(|part| part == slice))
    })
}

/// The text of `file` in the cgroup `path`.
pub(crate) fn read(path: &str, file: &str) -> Option<String> {
    std::fs::read_to_string(root().join(path.trim_start_matches('/')).join(file)).ok()
}

/// The memory the cgroup `path` holds that measured free memory does not count as free: its
/// anonymous and shared memory, from its `memory.stat` under `/sys/fs/cgroup`, or under
/// `PRIORY_CGROUP` when set.
pub fn memory(path: &str) -> Option<u64> {
    memory_in(&root(), path)
}

/// `memory`, under `root`. `memory.current` also counts the file cache charged to the cgroup, such
/// as a build's output, which `MemAvailable` already counts as free, so it is not read.
pub fn memory_in(root: &Path, path: &str) -> Option<u64> {
    let file = root.join(path.trim_start_matches('/')).join("memory.stat");
    stat_of(&std::fs::read_to_string(file).ok()?)
}

/// `anon` plus `shmem`, in bytes, from the text of a `memory.stat` file.
pub fn stat_of(text: &str) -> Option<u64> {
    field(text, "anon")?.checked_add(field(text, "shmem")?)
}

/// The value of `key` in the text of a flat keyed cgroup file such as `memory.stat` or `cpu.stat`.
fn field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(' ')?;
        (name == key).then(|| value.trim().parse::<u64>().ok())?
    })
}

/// What a cgroup used up to one moment: its CPU time in microseconds, its memory and the bytes its
/// processes read and wrote. A value the kernel does not expose is None.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sample {
    pub cpu_usec: Option<u64>,
    pub user_usec: Option<u64>,
    pub system_usec: Option<u64>,
    /// `memory.peak`, or `anon + shmem` on a kernel without it.
    pub memory: Option<u64>,
    pub io_read: Option<u64>,
    pub io_write: Option<u64>,
}

/// What the cgroup `path` used so far, under `/sys/fs/cgroup` or `PRIORY_CGROUP`. None once the
/// cgroup is gone, so a caller keeps its last sample.
pub fn sample(path: &str) -> Option<Sample> {
    sample_in(&root(), path)
}

/// `sample`, under `root`.
pub fn sample_in(root: &Path, path: &str) -> Option<Sample> {
    let dir = root.join(path.trim_start_matches('/'));
    let read = |file: &str| std::fs::read_to_string(dir.join(file)).ok();
    let cpu = read("cpu.stat")?;
    let memory = read("memory.peak")
        .and_then(|peak| peak.trim().parse().ok())
        .or_else(|| stat_of(&read("memory.stat")?));
    // `io.stat` exists only when the user manager delegates the io controller.
    let io = read("io.stat").map(|text| io_of(&text));
    Some(Sample {
        cpu_usec: field(&cpu, "usage_usec"),
        user_usec: field(&cpu, "user_usec"),
        system_usec: field(&cpu, "system_usec"),
        memory,
        io_read: io.map(|(read, _)| read),
        io_write: io.map(|(_, write)| write),
    })
}

/// The `rbytes` and `wbytes` of every device in the text of an `io.stat` file, summed.
fn io_of(text: &str) -> (u64, u64) {
    let mut sum = (0u64, 0u64);
    for pair in text.split_whitespace() {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = value.parse::<u64>().unwrap_or(0);
        match key {
            "rbytes" => sum.0 = sum.0.saturating_add(value),
            "wbytes" => sum.1 = sum.1.saturating_add(value),
            _ => {}
        }
    }
    sum
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
    fn memory_counts_anonymous_and_shared_memory_and_leaves_out_the_file_cache() {
        let root = std::env::temp_dir().join(format!("priory-cgroup-{}", std::process::id()));
        let dir = root.join(SCOPE.trim_start_matches('/'));
        std::fs::create_dir_all(&dir).unwrap();
        // A build: 5 GB charged, of which 4 GB is its output's file cache.
        std::fs::write(dir.join("memory.current"), format!("{}\n", 5u64 << 30)).unwrap();
        std::fs::write(
            dir.join("memory.stat"),
            format!(
                "anon {}\nfile {}\nkernel 1048576\nshmem {}\nfile_mapped 0\n",
                (1u64 << 30) - (1 << 20),
                (4u64 << 30) + (1 << 20),
                1u64 << 20
            ),
        )
        .unwrap();

        assert_eq!(memory_in(&root, SCOPE), Some(1 << 30));
        assert_eq!(memory_in(&root, "/user.slice/gone.scope"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn io_stat_sums_the_bytes_of_every_device() {
        let text = "8:0 rbytes=100 wbytes=20 rios=1 wios=1 dbytes=0 dios=0\n\
                    259:0 rbytes=5 wbytes=7 rios=1 wios=1 dbytes=0 dios=0\n";
        assert_eq!(io_of(text), (105, 27));
        assert_eq!(io_of(""), (0, 0));
    }

    #[test]
    fn a_stat_without_anon_or_shmem_measures_nothing() {
        assert_eq!(stat_of("anon 10\nshmem 5\n"), Some(15));
        assert_eq!(stat_of("anon 10\n"), None);
        assert_eq!(stat_of("anon_thp 10\nshmem 5\n"), None);
        assert_eq!(stat_of(""), None);
    }
}
