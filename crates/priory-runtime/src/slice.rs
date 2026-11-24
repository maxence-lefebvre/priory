use crate::runtime::run;
use crate::{cgroup, room};
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

/// The systemd user slice every run starts in: `PRIORY_SLICE`, else `priory-agents.slice`.
pub fn name() -> String {
    std::env::var("PRIORY_SLICE")
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "priory-agents.slice".to_owned())
}

/// Takes the slice `slice` for the daemon of `home`, or says why that daemon must leave its limits
/// alone: it runs inside the slice itself, or another home took the slice first.
pub fn claim(slice: &str, home: &Path) -> Result<(), String> {
    if cgroup::inside(slice) {
        return Err("this daemon runs inside it".to_owned());
    }
    let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|run| !run.is_empty()) else {
        return Err("XDG_RUNTIME_DIR, which holds its owner, is not set".to_owned());
    };
    let file = Path::new(&run)
        .join("priory")
        .join(format!("{slice}.owner"));
    let at = |error: std::io::Error| format!("{}: {error}", file.display());
    let home = fs::canonicalize(home).map_err(|error| format!("{}: {error}", home.display()))?;
    match fs::read(&file) {
        Ok(owner) => {
            let owner = Path::new(OsStr::from_bytes(&owner));
            if owner == home {
                return Ok(());
            }
            if owner.is_dir() {
                return Err(format!(
                    "the daemon of {} set them; name another slice in PRIORY_SLICE, or remove {} to take this one",
                    owner.display(),
                    file.display()
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(at(error)),
    }
    // A rename, so no daemon reads half a path. Of two that start at once, the last one owns the
    // slice, and the other's next sweep sees it.
    let new = file.with_extension(format!("owner.{}", std::process::id()));
    fs::write(&new, home.as_os_str().as_bytes())
        .and_then(|()| fs::rename(&new, &file))
        .map_err(at)
}

/// Caps the user slice `slice` at `cpu` of this machine's CPUs and `memory` of its memory, below the
/// user's own apps for CPU.
pub fn cap(slice: &str, cpu: f64, memory: f64) -> Result<(), String> {
    // A dash nests a slice, and CPU weights split the CPU between siblings, so the weight goes on the
    // top-level slice, beside the user's app.slice.
    let top = slice
        .split_once('-')
        .map_or_else(|| slice.to_owned(), |(root, _)| format!("{root}.slice"));
    set(&top, &["CPUWeight=20".to_owned()])?;
    set(
        slice,
        &[
            format!("CPUQuota={}%", quota(cpu)),
            // A throttling threshold: past it the kernel reclaims and slows the agents down.
            format!("MemoryHigh={}%", (memory * 1000.0).round() / 10.0),
        ],
    )
}

/// Caps the slice again when its cgroup's limits differ from `cap`'s, and returns those it had. A
/// slice with no cgroup yet is capped again and returns none.
pub fn restore(slice: &str, cpu: f64, memory: f64) -> Result<Option<String>, String> {
    let path = control_group(slice)?;
    if path.is_empty() {
        return cap(slice, cpu, memory).map(|()| None);
    }
    let (Some(max), Some(high)) = (
        cgroup::read(&path, "cpu.max").and_then(|text| quota_of(&text)),
        cgroup::read(&path, "memory.high").and_then(|text| bytes_of(&text)),
    ) else {
        return Ok(None);
    };
    let total = room::meminfo("MemTotal")? as f64;
    // The kernel rounds the limit down to a page, and systemd may read a slightly smaller total.
    if (max - quota(cpu)).abs() < 1.0 && (high / total - memory).abs() < 0.01 {
        return Ok(None);
    }
    cap(slice, cpu, memory)?;
    Ok(Some(format!(
        "CPUQuota={} MemoryHigh={}",
        percent(max),
        percent(high / total * 100.0)
    )))
}

/// The share of the last minute, in percent, in which every runnable process of the slice waited
/// for CPU, with the slice's CPU quota: what its quota throttles. None below `THROTTLED`, or where
/// the kernel does not measure pressure.
pub fn throttled(slice: &str) -> Option<String> {
    let path = control_group(slice).ok().filter(|path| !path.is_empty())?;
    let waited = full_of(&cgroup::read(&path, "cpu.pressure")?)?;
    if waited < THROTTLED {
        return None;
    }
    let quota = cgroup::read(&path, "cpu.max").and_then(|text| quota_of(&text))?;
    Some(format!(
        "{slice} was throttled: its agents had no CPU for {waited:.0}% of the last minute, under CPUQuota={}",
        percent(quota)
    ))
}

/// Above what a busy machine shows: a slice with its whole quota in use measured 6 %.
const THROTTLED: f64 = 10.0;

fn percent(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.0}%")
    } else {
        "infinity".to_owned()
    }
}

/// `cpu` of this machine's CPUs, in percent of one CPU.
fn quota(cpu: f64) -> f64 {
    (room::cpus() as f64 * cpu * 100.0).round()
}

/// The quota in the text of a `cpu.max` file, in percent of one CPU.
fn quota_of(text: &str) -> Option<f64> {
    let mut parts = text.split_whitespace();
    let quota = bytes_of(parts.next()?)?;
    let period: f64 = parts.next()?.parse().ok()?;
    (period > 0.0).then_some(quota / period * 100.0)
}

/// A number, or `max`, as a cgroup file writes its limit.
fn bytes_of(text: &str) -> Option<f64> {
    match text.trim() {
        "max" => Some(f64::INFINITY),
        number => number.parse::<u64>().ok().map(|number| number as f64),
    }
}

/// `avg60` of the `full` line in the text of a `cpu.pressure` file.
fn full_of(text: &str) -> Option<f64> {
    text.lines()
        .find_map(|line| line.strip_prefix("full "))?
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg60="))?
        .parse()
        .ok()
}

/// The slice's cgroup path, empty while nothing runs in it. It fails with no systemd user manager.
pub fn control_group(slice: &str) -> Result<String, String> {
    let path = run(Command::new("systemctl").args([
        "--user",
        "show",
        slice,
        "--property=ControlGroup",
        "--value",
    ]))?;
    Ok(path.trim().to_owned())
}

/// The command that starts a run in its own scope inside the user slice `slice`.
pub fn scope(slice: &str) -> Result<Vec<String>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_kernels_files_give_the_quota_the_pressure_and_the_online_cpus() {
        assert_eq!(quota_of("1980000 100000\n"), Some(1980.0));
        assert_eq!(quota_of("max 100000\n"), Some(f64::INFINITY));
        assert_eq!(quota_of("1 0"), None);
        let pressure = "some avg10=1.01 avg60=8.40 avg300=34.40 total=1\nfull avg10=0.18 avg60=5.81 avg300=32.35 total=1\n";
        assert_eq!(full_of(pressure), Some(5.81));
        assert_eq!(room::cpus_of("cpu  1 2\ncpu0 1 2\ncpu21 1 2\nintr 5\n"), 2);
    }

    proptest! {
        #[test]
        fn any_text_is_read_or_refused_without_panicking(
            text in "(max|full|some|avg60=|cpu|[0-9.]|[ \n])*|\\PC{0,200}",
        ) {
            let _ = (quota_of(&text), full_of(&text), room::cpus_of(&text));
        }
    }
}
