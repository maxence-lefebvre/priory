use crate::runtime::run;
use std::process::Command;

/// The systemd user slice every run starts in: `PRIORY_SLICE`, else `priory-agents.slice`.
pub fn name() -> String {
    std::env::var("PRIORY_SLICE")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "priory-agents.slice".to_owned())
}

/// Caps the user slice `slice` at `cpu` of this machine's CPUs and `memory` of its memory, below the
/// user's own apps for CPU. It fails on a host with no systemd user manager.
pub fn confine(slice: &str, cpu: f64, memory: f64) -> Result<(), String> {
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    // Runtime properties need no daemon-reload and vanish at reboot; each daemon start sets them again.
    run(Command::new("systemctl").args([
        "--user",
        "set-property",
        "--runtime",
        slice,
        // Apps default to 100, so the agents yield the CPU to them under contention.
        "CPUWeight=20",
        &format!("CPUQuota={}%", (cpus as f64 * cpu * 100.0).round()),
        // Past MemoryHigh the kernel reclaims and throttles; nothing sets MemoryMax, so nothing is killed.
        &format!("MemoryHigh={}%", (memory * 1000.0).round() / 10.0),
    ]))
    .map(drop)
}
