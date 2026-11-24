use crate::runtime::{self, run};
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
/// alone: it runs inside the slice itself, the daemon of another home holds it, or the owner cannot be told.
pub fn claim(slice: &str, home: &Path) -> Result<(), String> {
    if cgroup::inside(slice) {
        return Err("this daemon runs inside it".to_owned());
    }
    let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|run| !run.is_empty()) else {
        return Err("XDG_RUNTIME_DIR, which holds its owner, is not set".to_owned());
    };
    let run = Path::new(&run);
    let bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    if foreign(run, &bus.to_string_lossy(), runtime::uid()) {
        return Err(format!(
            "DBUS_SESSION_BUS_ADDRESS is not the bus of XDG_RUNTIME_DIR {}, which holds its owner",
            run.display()
        ));
    }
    let dir = run.join("priory");
    let file = dir.join(format!("{slice}.owner"));
    let at = |error: std::io::Error| format!("{}: {error}", file.display());
    let home = fs::canonicalize(home).map_err(|error| format!("{}: {error}", home.display()))?;
    match fs::read(&file) {
        Ok(owner) => {
            let owner = Path::new(OsStr::from_bytes(&owner));
            if owner == home {
                return Ok(());
            }
            if alive(owner) {
                return Err(format!(
                    "the daemon of {} set them; name another slice in PRIORY_SLICE, or stop that daemon",
                    owner.display()
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(at(error)),
    }
    runtime::private_dir(&dir, runtime::uid(), None)?;
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
    let Some(max) = cgroup::read(&path, "cpu.max").and_then(|text| quota_of(&text)) else {
        return Ok(None);
    };
    let total = room::meminfo("MemTotal")? as f64;
    let high = cgroup::read(&path, "memory.high")
        .and_then(|text| bytes_of(&text))
        .map(|high| high / total);
    // The kernel rounds the limit down to a page, and systemd may read a slightly smaller total.
    if (max - quota(cpu)).abs() < 1.0 && high.is_none_or(|high| (high - memory).abs() < 0.01) {
        return Ok(None);
    }
    cap(slice, cpu, memory)?;
    let high = high.map_or_else(String::new, |high| {
        format!(" MemoryHigh={}", percent(high * 100.0))
    });
    Ok(Some(format!("CPUQuota={}{high}", percent(max))))
}

/// The slice's CPU counters since its cgroup began, from `cpu.stat`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cpu {
    usage: u64,
    times: u64,
    throttled: u64,
}

/// The slice's CPU counters now: zero with no cgroup yet, none where they cannot be read.
pub fn cpu(slice: &str) -> Option<Cpu> {
    let path = control_group(slice).ok()?;
    if path.is_empty() {
        return Some(Cpu::default());
    }
    cpu_of(&cgroup::read(&path, "cpu.stat")?)
}

/// What the slice's CPU quota kept back since `since`, or none below `THROTTLED` or with no quota.
pub fn throttled(slice: &str, since: Cpu) -> Option<String> {
    let path = control_group(slice).ok().filter(|path| !path.is_empty())?;
    let quota = cgroup::read(&path, "cpu.max")
        .and_then(|text| quota_of(&text))
        .filter(|quota| quota.is_finite())?;
    let kept = kept_back(since, cpu_of(&cgroup::read(&path, "cpu.stat")?)?)?;
    (kept >= THROTTLED).then(|| {
        format!(
            "{slice} was throttled: its quota kept back {kept:.0}% of the CPU its agents wanted since the run started, under CPUQuota={}",
            percent(quota)
        )
    })
}

/// In percent, what the default 90 % share gives up by design when every CPU is busy.
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

/// The counters in the text of a `cpu.stat` file, when it has them all.
fn cpu_of(text: &str) -> Option<Cpu> {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix(' '))
            .and_then(|value| value.trim().parse::<u64>().ok())
    };
    Some(Cpu {
        usage: field("usage_usec")?,
        times: field("nr_throttled")?,
        throttled: field("throttled_usec")?,
    })
}

/// Between two readings, the CPU time kept back, in percent of it and the time used. None when no
/// period was throttled, or the counters restarted with a new cgroup.
fn kept_back(since: Cpu, now: Cpu) -> Option<f64> {
    let used = now.usage.checked_sub(since.usage)? as f64;
    let kept = now.throttled.checked_sub(since.throttled)? as f64;
    let times = now.times.checked_sub(since.times)?;
    (times > 0 && kept > 0.0).then(|| kept / (kept + used) * 100.0)
}

/// Whether `systemctl --user` can reach a manager other than that of the runtime directory `run`: it
/// falls back to the bus address, so a scratch directory would hide who owns the slice.
fn foreign(run: &Path, bus: &str, uid: u32) -> bool {
    let own = |path: &str| Path::new(path) == run.join("bus");
    !(run == Path::new(&format!("/run/user/{uid}"))
        || bus.is_empty()
        || !bus.contains(';')
            && bus
                .split(',')
                .next()
                .and_then(|address| address.strip_prefix("unix:path="))
                .is_some_and(own))
}

/// Whether a daemon runs on `home`: it holds the lock `listen` takes.
fn alive(home: &Path) -> bool {
    match fs::File::open(home.join("daemon.lock")) {
        Ok(lock) => lock.try_lock_shared().is_err(),
        Err(error) => !matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory),
    }
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
    fn the_kernels_files_give_the_quota_the_counters_and_the_online_cpus() {
        assert_eq!(quota_of("1980000 100000\n"), Some(1980.0));
        assert_eq!(quota_of("max 100000\n"), Some(f64::INFINITY));
        assert_eq!(quota_of("1 0"), None);
        let stat = "usage_usec 10\nuser_usec 9\nnr_periods 4\nnr_throttled 2\nthrottled_usec 30\n";
        let now = Cpu {
            usage: 10,
            times: 2,
            throttled: 30,
        };
        assert_eq!(cpu_of(stat), Some(now));
        assert_eq!(cpu_of("usage_usec 10\n"), None);
        assert_eq!(kept_back(Cpu::default(), now), Some(75.0));
        assert_eq!(kept_back(now, now), None);
        // A new cgroup's counters start again.
        assert_eq!(kept_back(now, Cpu { usage: 1, ..now }), None);
        assert_eq!(room::cpus_of("cpu  1 2\ncpu0 1 2\ncpu21 1 2\nintr 5\n"), 2);
    }

    #[test]
    fn only_a_bus_of_the_runtime_directory_is_its_own() {
        let scratch = Path::new("/tmp/scratch");
        let real = "unix:path=/run/user/1000/bus";
        assert!(foreign(scratch, real, 1000));
        assert!(foreign(scratch, "unix:abstract=/tmp/scratch/bus", 1000));
        assert!(foreign(
            scratch,
            &format!("unix:path=/tmp/scratch/bus,guid=1;{real}"),
            1000
        ));
        assert!(!foreign(scratch, "", 1000));
        assert!(!foreign(scratch, "unix:path=/tmp/scratch/bus,guid=1", 1000));
        assert!(!foreign(Path::new("/run/user/1000/"), real, 1000));
    }

    proptest! {
        #[test]
        fn any_text_is_read_or_refused_without_panicking(
            text in "(max|usage_usec|nr_throttled|throttled_usec|unix:path=|cpu|[0-9.,;]|[ \n])*|\\PC{0,200}",
        ) {
            let _ = (quota_of(&text), cpu_of(&text), room::cpus_of(&text));
            let _ = foreign(Path::new("/tmp/scratch"), &text, 1000);
        }
    }
}
