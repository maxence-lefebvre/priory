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
/// user's own apps for CPU, and returns the command that starts a run in its own scope inside it.
/// It fails on a host with no systemd user manager.
pub fn scope(slice: &str, cpu: f64, memory: f64) -> Result<Vec<String>, String> {
    // A dash nests a slice, and CPU weights split the CPU between siblings, so the weight goes on the
    // top-level slice, beside the user's app.slice.
    let top = slice
        .split_once('-')
        .map_or_else(|| slice.to_owned(), |(root, _)| format!("{root}.slice"));
    set(&top, &["CPUWeight=20".to_owned()])?;
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    set(
        slice,
        &[
            format!("CPUQuota={}%", (cpus as f64 * cpu * 100.0).round()),
            // A throttling threshold: past it the kernel reclaims and slows the agents down.
            format!("MemoryHigh={}%", (memory * 1000.0).round() / 10.0),
        ],
    )?;
    let version = run(Command::new("systemd-run").arg("--version"))?;
    let version = version
        .split_whitespace()
        .nth(1)
        .and_then(|number| number.parse::<u32>().ok())
        .ok_or_else(|| format!("systemd-run --version printed {version:?}"))?;
    let mut scope: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect"]
        .map(str::to_owned)
        .into();
    // systemd 254 brought the option, and a later release expands ${NAME} in a scope's command by default.
    if version >= 254 {
        scope.push("--expand-environment=no".to_owned());
    }
    scope.extend([format!("--slice={slice}"), "--".to_owned()]);
    Ok(scope)
}

/// Sets `properties` on the user unit `unit` until reboot, with no daemon-reload.
fn set(unit: &str, properties: &[String]) -> Result<(), String> {
    run(Command::new("systemctl")
        .args(["--user", "set-property", "--runtime", unit])
        .args(properties))
    .map(drop)
}
