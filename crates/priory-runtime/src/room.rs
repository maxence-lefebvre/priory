use crate::runtime::fed_within;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The machine's room, measured live.
#[derive(Debug)]
pub struct Room {
    /// `MemAvailable`, in bytes.
    pub memory_free: u64,
    /// `MemTotal`, in bytes.
    pub memory_total: u64,
    /// Free disk under `PRIORY_HOME` for an unprivileged user, in bytes.
    pub disk_free: u64,
    pub disk_total: u64,
    /// The 1-minute load average.
    pub load: f64,
    pub cpus: usize,
}

/// Measures memory and load from `/proc`, or from `PRIORY_PROC` when set, and disk with `df` on
/// `home`.
pub fn measure(home: &Path) -> Result<Room, String> {
    let proc = proc();
    let loadavg = read(&proc.join("loadavg"))?;
    let load = loadavg
        .split_whitespace()
        .next()
        .and_then(|load| load.parse::<f64>().ok())
        .filter(|load| load.is_finite() && *load >= 0.0)
        .ok_or_else(|| format!("no load in {}", proc.join("loadavg").display()))?;
    let (disk_total, disk_free) = disk(home)?;
    Ok(Room {
        memory_free: meminfo("MemAvailable")?,
        memory_total: meminfo("MemTotal")?,
        disk_free,
        disk_total,
        load,
        cpus: cpus(),
    })
}

/// The field `key` of `/proc/meminfo`, in bytes.
pub(crate) fn meminfo(key: &str) -> Result<u64, String> {
    let file = proc().join("meminfo");
    read(&file)?
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|value| value.trim().strip_suffix("kB")?.trim().parse::<u64>().ok())
        .and_then(|value| value.checked_mul(1024))
        .ok_or_else(|| format!("no {key} in {}", file.display()))
}

/// The machine's online CPUs, from `/proc/stat`. Rust's `available_parallelism` counts the caller's
/// own cgroup quota, so it only stands in where there is no such file.
pub fn cpus() -> usize {
    let stat = read(&proc().join("stat")).unwrap_or_default();
    match cpus_of(&stat) {
        0 => std::thread::available_parallelism().map_or(1, usize::from),
        online => online,
    }
}

/// The CPUs the text of a `/proc/stat` file lists: its `cpu<n>` lines, not their `cpu` total.
pub fn cpus_of(stat: &str) -> usize {
    stat.lines()
        .filter_map(|line| line.strip_prefix("cpu"))
        .filter(|rest| rest.starts_with(|first: char| first.is_ascii_digit()))
        .count()
}

/// The machine's host name, which names its runner file.
pub fn hostname() -> Option<String> {
    let name = std::fs::read_to_string(proc().join("sys/kernel/hostname")).ok()?;
    Some(name.trim().to_owned()).filter(|name| !name.is_empty())
}

/// This process's own resident memory and CPU time, from `/proc/self`.
#[derive(Debug, PartialEq)]
pub struct Own {
    /// Resident memory, in bytes.
    pub memory: u64,
    /// CPU time in user mode, in seconds.
    pub user: f64,
    /// CPU time in kernel mode, in seconds.
    pub system: f64,
}

/// Measures this process from `/proc/self`, or `PRIORY_PROC`'s `self` when set.
pub fn own() -> Result<Own, String> {
    let dir = proc().join("self");
    let status = read(&dir.join("status"))?;
    let stat = read(&dir.join("stat"))?;
    own_of(&status, &stat).ok_or_else(|| format!("{}: no VmRSS, utime or stime", dir.display()))
}

/// Clock ticks per second in `/proc/<pid>/stat`: Linux's `USER_HZ`, 100 on every architecture.
const USER_HZ: f64 = 100.0;

/// Reads `VmRSS` from the text of `status`, and `utime` and `stime` from the text of `stat`.
pub fn own_of(status: &str, stat: &str) -> Option<Own> {
    let memory = status_bytes(status, "VmRSS")?;
    // The command name in parentheses may hold spaces and parentheses, so fields count from the last.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks = |index: usize| Some(fields.get(index)?.parse::<u64>().ok()? as f64 / USER_HZ);
    Some(Own {
        memory,
        user: ticks(11)?,
        system: ticks(12)?,
    })
}

/// The field `key` of the text of a `/proc/<pid>/status` file, which counts in kB, in bytes.
fn status_bytes(status: &str, key: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix(':')?.trim();
        let kilobytes = value.strip_suffix("kB")?.trim();
        kilobytes.parse::<u64>().ok()?.checked_mul(1024)
    })
}

/// The anonymous memory of process `pid` and its descendants, in bytes, without those in `seen`,
/// which it adds them to. A process that cannot be read counts as none.
pub fn tree(pid: u32, seen: &mut BTreeSet<u32>) -> u64 {
    tree_in(&proc(), pid, seen)
}

/// `tree`, under `proc`.
fn tree_in(proc: &Path, pid: u32, seen: &mut BTreeSet<u32>) -> u64 {
    let (mut waiting, mut bytes) = (vec![pid], 0u64);
    while let Some(pid) = waiting.pop() {
        if !seen.insert(pid) {
            continue;
        }
        let dir = proc.join(pid.to_string());
        let status = read(&dir.join("status")).unwrap_or_default();
        bytes = bytes.saturating_add(status_bytes(&status, "RssAnon").unwrap_or(0));
        // Each thread lists only the children it forked.
        for thread in std::fs::read_dir(dir.join("task"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let children = read(&thread.path().join("children")).unwrap_or_default();
            waiting.extend(
                children
                    .split_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok()),
            );
        }
    }
    bytes
}

pub(crate) fn proc() -> PathBuf {
    std::env::var_os("PRIORY_PROC")
        .filter(|proc| !proc.is_empty())
        .map_or_else(|| PathBuf::from("/proc"), PathBuf::from)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// How long `df` may take, since a hung mount would hold the daemon's thread.
const DF: Duration = Duration::from_secs(5);

/// Free disk on the filesystem of `path` for an unprivileged user, in bytes.
pub fn free(path: &Path) -> Result<u64, String> {
    disk(path).map(|(_, free)| free)
}

/// Total and available bytes of the file system holding `path`, from POSIX `df -Pk`.
fn disk(path: &Path) -> Result<(u64, u64), String> {
    let output = fed_within(Command::new("df").arg("-Pk").arg(path), None, DF)
        .map_err(|error| format!("df: {error}"))?
        .ok_or_else(|| {
            let seconds = DF.as_secs();
            format!("df -Pk {}: timed out after {seconds}s", path.display())
        })?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Columns: file system, 1024-blocks, used, available, capacity, mount point.
    let fields: Vec<&str> = text
        .lines()
        .nth(1)
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let kilobytes = |column: usize| {
        fields
            .get(column)?
            .parse::<u64>()
            .ok()
            .map(|value| value << 10)
    };
    match (output.status.success(), kilobytes(1), kilobytes(3)) {
        (true, Some(total), Some(free)) => Ok((total, free)),
        _ => Err(format!(
            "df -Pk {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn own_reads_resident_memory_and_cpu_time_past_a_command_name_with_parentheses() {
        let status = "Name:\tpriory\nVmRSS:\t   20480 kB\n";
        let stat = "42 (a) b (c) S 1 42 42 0 -1 0 0 0 0 0 250 50 0 0 20 0 3 0 100\n";
        assert_eq!(
            own_of(status, stat),
            Some(Own {
                memory: 20480 * 1024,
                user: 2.5,
                system: 0.5,
            })
        );
        assert_eq!(own_of("VmRSS: 1 kB\n", "42 (a) S 1"), None);
    }

    /// A process of a fake `/proc` that holds `kb` kB, and whose one thread forked `children`.
    fn process(proc: &Path, pid: u32, kb: u64, children: &str) {
        let dir = proc.join(pid.to_string());
        let thread = dir.join("task").join(pid.to_string());
        std::fs::create_dir_all(&thread).unwrap();
        std::fs::write(
            dir.join("status"),
            format!("VmRSS:\t 999 kB\nRssAnon:\t {kb} kB\nRssShmem:\t 50 kB\n"),
        )
        .unwrap();
        std::fs::write(thread.join("children"), children).unwrap();
    }

    #[test]
    fn a_tree_counts_each_process_once_across_calls() {
        let proc = std::env::temp_dir().join(format!("priory-tree-{}", std::process::id()));
        process(&proc, 10, 1, "11 12 ");
        process(&proc, 11, 2, "");
        process(&proc, 12, 4, "13 99 ");
        process(&proc, 13, 8, "12 ");
        process(&proc, 20, 16, "");
        let mut seen = BTreeSet::new();

        assert_eq!(tree_in(&proc, 12, &mut seen), 12 << 10);
        assert_eq!(tree_in(&proc, 10, &mut seen), 3 << 10);
        assert_eq!(tree_in(&proc, 11, &mut seen), 0);
        assert_eq!(tree_in(&proc, 10, &mut BTreeSet::new()), 15 << 10);
        std::fs::remove_dir_all(&proc).unwrap();
    }

    #[test]
    fn a_tree_counts_a_real_childs_memory() {
        // A shell that forks a process holding 64 MB of anonymous memory, until its input closes.
        let mut child = Command::new("sh")
            .args([
                "-c",
                r#"sh -c 'v=$(head -c 67108864 /dev/zero | tr "\0" x); echo ${#v}; read _'; :"#,
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.as_mut().unwrap()),
            &mut line,
        )
        .unwrap();

        let bytes = tree_in(Path::new("/proc"), child.id(), &mut BTreeSet::new());

        drop(child.stdin.take());
        child.wait().unwrap();
        assert_eq!(line, "67108864\n");
        assert!(bytes >= 64 << 20, "{bytes}");
    }

    proptest! {
        #[test]
        fn status_bytes_of_any_text_returns_or_refuses_without_panicking(
            value in "\\PC{0,40}",
            kb in any::<u64>(),
        ) {
            let _ = status_bytes(&format!("RssAnon:{value}"), "RssAnon");
            prop_assert_eq!(
                status_bytes(&format!("RssAnon:\t{kb} kB\n"), "RssAnon"),
                kb.checked_mul(1024)
            );
        }

        #[test]
        fn own_of_any_text_returns_or_refuses_without_panicking(
            status in "\\PC{0,200}",
            stat in "[0-9 ()a-zS-]{0,200}",
        ) {
            let _ = own_of(&status, &stat);
        }
    }
}
