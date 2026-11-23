use crate::runtime::run;
use std::path::{Path, PathBuf};
use std::process::Command;

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
/// `home`. On macOS, which has no `/proc`, memory and load come from `sysctl` and `vm_stat`.
pub fn measure(home: &Path) -> Result<Room, String> {
    let (memory_free, memory_total, load) = if macos() {
        let total = run(Command::new("sysctl").args(["-n", "hw.memsize"]))?;
        let load = run(Command::new("sysctl").args(["-n", "vm.loadavg"]))?;
        let pages = run(&mut Command::new("vm_stat"))?;
        (
            vm_stat_free(&pages).ok_or("vm_stat: no free pages")?,
            total
                .trim()
                .parse()
                .map_err(|_| format!("sysctl hw.memsize printed {total:?}"))?,
            load_of(load.trim_start_matches(['{', ' ']))
                .ok_or_else(|| format!("sysctl vm.loadavg printed {load:?}"))?,
        )
    } else {
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
        let load = load_of(&read(&proc.join("loadavg"))?)
            .ok_or_else(|| format!("no load in {}", proc.join("loadavg").display()))?;
        (kilobytes("MemAvailable")?, kilobytes("MemTotal")?, load)
    };
    let (disk_total, disk_free) = disk(home)?;
    Ok(Room {
        memory_free,
        memory_total,
        disk_free,
        disk_total,
        load,
        cpus: std::thread::available_parallelism().map_or(1, usize::from),
    })
}

/// The 1-minute load average, the first number of `text`.
fn load_of(text: &str) -> Option<f64> {
    let load = text.split_whitespace().next()?.parse::<f64>().ok()?;
    (load.is_finite() && load >= 0.0).then_some(load)
}

/// The bytes macOS can hand out without swapping, from the text of `vm_stat`: its free,
/// speculative and inactive pages.
pub fn vm_stat_free(text: &str) -> Option<u64> {
    let (_, size) = text.lines().next()?.split_once("page size of ")?;
    let size: u64 = size.split_whitespace().next()?.parse().ok()?;
    let pages = |key: &str| {
        let value = text.lines().find_map(|line| line.strip_prefix(key))?;
        value.trim().trim_end_matches('.').parse::<u64>().ok()
    };
    pages("Pages free:")?
        .checked_add(pages("Pages speculative:")?)?
        .checked_add(pages("Pages inactive:")?)?
        .checked_mul(size)
}

/// The machine's host name, which names its runner file.
pub fn hostname() -> Option<String> {
    let name = if macos() {
        run(Command::new("hostname").arg("-s")).ok()?
    } else {
        std::fs::read_to_string(proc().join("sys/kernel/hostname")).ok()?
    };
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

/// Measures this process from `/proc/self`, or `PRIORY_PROC`'s `self` when set. On macOS, `ps`
/// measures it.
pub fn own() -> Result<Own, String> {
    if macos() {
        let pid = std::process::id().to_string();
        let line = run(Command::new("ps").args(["-o", "rss=,utime=,time=", "-p", &pid]))?;
        return own_of_ps(&line).ok_or_else(|| format!("ps printed {line:?}"));
    }
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

/// Reads resident kilobytes, user CPU time and total CPU time from a line of macOS `ps`.
pub fn own_of_ps(line: &str) -> Option<Own> {
    // A CPU time reads `[days-][hours:]minutes:seconds`.
    let seconds = |time: &str| {
        let (days, clock) = time.split_once('-').unwrap_or(("0", time));
        let parts: Vec<&str> = clock.rsplit(':').collect();
        (parts.len() <= 3).then_some(())?;
        let mut total = days.parse::<u32>().ok()? as f64 * 86400.0;
        for (part, unit) in parts.iter().zip([1.0, 60.0, 3600.0]) {
            let part = part.parse::<f64>().ok()?;
            (part.is_finite() && part >= 0.0).then_some(())?;
            total += part * unit;
        }
        Some(total)
    };
    let fields: Vec<&str> = line.split_whitespace().collect();
    let [memory, user, total] = fields[..] else {
        return None;
    };
    let user = seconds(user)?;
    Some(Own {
        memory: memory.parse::<u64>().ok()?.checked_mul(1024)?,
        user,
        system: (seconds(total)? - user).max(0.0),
    })
}

/// Whether to measure with macOS's tools: on macOS, unless `PRIORY_PROC` names a stand-in `/proc`.
fn macos() -> bool {
    cfg!(target_os = "macos") && std::env::var_os("PRIORY_PROC").is_none_or(|proc| proc.is_empty())
}

fn proc() -> PathBuf {
    std::env::var_os("PRIORY_PROC")
        .filter(|proc| !proc.is_empty())
        .map_or_else(|| PathBuf::from("/proc"), PathBuf::from)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// Total and available bytes of the file system holding `path`, from POSIX `df -Pk`.
fn disk(path: &Path) -> Result<(u64, u64), String> {
    let output = Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .map_err(|error| format!("df: {error}"))?;
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

    #[test]
    fn this_machine_is_measured_with_its_own_tools() {
        let room = measure(Path::new(".")).unwrap();
        assert!(0 < room.memory_free && room.memory_free <= room.memory_total);
        assert!(0 < room.disk_free && room.disk_free <= room.disk_total);
        assert!(hostname().is_some());
        assert!(own().unwrap().memory > 0);
    }

    #[test]
    fn vm_stat_free_adds_the_free_speculative_and_inactive_pages() {
        let text = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                    Pages free:                                8172.\n\
                    Pages active:                            266998.\n\
                    Pages inactive:                          265148.\n\
                    Pages speculative:                         1268.\n";
        assert_eq!(vm_stat_free(text), Some((8172 + 265148 + 1268) * 16384));
        assert_eq!(vm_stat_free("Pages free: 1.\n"), None);
    }

    #[test]
    fn own_of_ps_reads_resident_memory_and_splits_cpu_time() {
        assert_eq!(
            own_of_ps("  4048   1:02.50   1-00:01:03.00\n"),
            Some(Own {
                memory: 4048 * 1024,
                user: 62.5,
                system: 86400.0 + 63.0 - 62.5,
            })
        );
        assert_eq!(own_of_ps("4048 0:00.01"), None);
    }

    proptest! {
        #[test]
        fn the_macos_parsers_return_or_refuse_any_text_without_panicking(
            text in "[0-9 .:\\-\\na-zP()]{0,200}",
        ) {
            let _ = vm_stat_free(&text);
            let _ = vm_stat_free(&format!("(page size of {text}\nPages free: {text}"));
            let _ = own_of_ps(&text);
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
