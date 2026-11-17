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
