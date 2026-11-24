use crate::runtime::fed_within;
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
    let meminfo = read(&proc.join("meminfo"))?;
    let kilobytes = |key: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
            .and_then(|value| value.trim().strip_suffix("kB")?.trim().parse::<u64>().ok())
            .map(|value| value << 10)
            .ok_or_else(|| format!("no {key} in {}", proc.join("meminfo").display()))
    };
    let loadavg = read(&proc.join("loadavg"))?;
    let load = loadavg
        .split_whitespace()
        .next()
        .and_then(|load| load.parse::<f64>().ok())
        .filter(|load| load.is_finite() && *load >= 0.0)
        .ok_or_else(|| format!("no load in {}", proc.join("loadavg").display()))?;
    let (disk_total, disk_free) = disk(home)?;
    Ok(Room {
        memory_free: kilobytes("MemAvailable")?,
        memory_total: kilobytes("MemTotal")?,
        disk_free,
        disk_total,
        load,
        cpus: std::thread::available_parallelism().map_or(1, usize::from),
    })
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
    let memory = status.lines().find_map(|line| {
        let kilobytes = line.strip_prefix("VmRSS:")?.trim().strip_suffix("kB")?;
        kilobytes.trim().parse::<u64>().ok()?.checked_mul(1024)
    })?;
    // The command name in parentheses may hold spaces and parentheses, so fields count from the last.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks = |index: usize| Some(fields.get(index)?.parse::<u64>().ok()? as f64 / USER_HZ);
    Some(Own {
        memory,
        user: ticks(11)?,
        system: ticks(12)?,
    })
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

    proptest! {
        #[test]
        fn own_of_any_text_returns_or_refuses_without_panicking(
            status in "\\PC{0,200}",
            stat in "[0-9 ()a-zS-]{0,200}",
        ) {
            let _ = own_of(&status, &stat);
        }
    }
}
