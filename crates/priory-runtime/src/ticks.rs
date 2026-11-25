//! The durations of a process's loop ticks, the other durations it times for a histogram, which
//! its metrics exporter takes each interval, and the counts it keeps for a counter.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

static ON: AtomicBool = AtomicBool::new(false);
static TICKS: Mutex<Vec<f64>> = Mutex::new(Vec::new());
static TIMED: Mutex<Vec<Timed>> = Mutex::new(Vec::new());
static COUNTS: Mutex<BTreeMap<Counter, u64>> = Mutex::new(BTreeMap::new());

/// A counter's series: metric `.0` under the attributes `.1`.
pub type Counter = (&'static str, Vec<(&'static str, &'static str)>);

/// One duration a process timed: metric `metric`, under its `attributes`, such as its outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    pub metric: &'static str,
    pub attributes: Vec<(&'static str, &'static str)>,
    pub seconds: f64,
}

/// Starts keeping ticks; a process with no exporter keeps none.
pub fn keep() {
    ON.store(true, Ordering::SeqCst);
}

/// Whether this process keeps its ticks, durations and counts, which only an exporter wants.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Records one tick that took `elapsed`.
pub fn record(elapsed: Duration) {
    if ON.load(Ordering::Relaxed)
        && let Ok(mut ticks) = TICKS.lock()
    {
        ticks.push(elapsed.as_secs_f64());
    }
}

/// The ticks recorded since the last take, in seconds.
pub fn take() -> Vec<f64> {
    TICKS
        .lock()
        .map(|mut ticks| std::mem::take(&mut *ticks))
        .unwrap_or_default()
}

/// Records one duration of `metric`, such as a wait for the log's connection.
pub fn time(metric: &'static str, elapsed: Duration) {
    timed_as(metric, &[], elapsed);
}

/// Records one duration of `metric` under `attributes`, such as how it ended.
pub fn timed_as(
    metric: &'static str,
    attributes: &[(&'static str, &'static str)],
    elapsed: Duration,
) {
    if ON.load(Ordering::Relaxed)
        && let Ok(mut timed) = TIMED.lock()
    {
        timed.push(Timed {
            metric,
            attributes: attributes.to_vec(),
            seconds: elapsed.as_secs_f64(),
        });
    }
}

/// The durations timed since the last take.
pub fn take_timed() -> Vec<Timed> {
    TIMED
        .lock()
        .map(|mut timed| std::mem::take(&mut *timed))
        .unwrap_or_default()
}

/// Adds one to counter `metric` under `attributes`, such as an event the intake published.
pub fn count(metric: &'static str, attributes: &[(&'static str, &'static str)]) {
    if ON.load(Ordering::Relaxed)
        && let Ok(mut counts) = COUNTS.lock()
    {
        *counts.entry((metric, attributes.to_vec())).or_default() += 1;
    }
}

/// Every counter's total since the process started keeping them.
pub fn counts() -> BTreeMap<Counter, u64> {
    COUNTS
        .lock()
        .map(|counts| counts.clone())
        .unwrap_or_default()
}

/// The tick in progress, for the watchdog.
struct Tick {
    started: Instant,
    phase: (&'static str, Instant),
    /// The time of each phase so far, which a hold over several passes enters again.
    phases: BTreeMap<&'static str, Duration>,
    command: Option<(String, Instant)>,
    /// The command that took longest so far.
    slowest: Option<(String, Duration)>,
}

static TICK: Mutex<Option<Tick>> = Mutex::new(None);

thread_local! {
    /// Whether this thread runs the loop, whose commands hold the tick.
    static LOOP: Cell<bool> = const { Cell::new(false) };
}

fn tick() -> MutexGuard<'static, Option<Tick>> {
    TICK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Starts a tick of the loop, on the calling thread.
pub fn begin() {
    LOOP.set(true);
    let now = Instant::now();
    *tick() = Some(Tick {
        started: now,
        phase: ("start", now),
        phases: BTreeMap::new(),
        command: None,
        slowest: None,
    });
}

/// Says what the tick does from now on.
pub fn phase(name: &'static str) {
    if let Some(tick) = tick().as_mut() {
        tick.close_phase();
        tick.phase = (name, Instant::now());
    }
}

/// Ends the tick. When it took over `over`, returns what took longest.
pub fn end(over: Duration) -> Option<String> {
    let mut tick = tick().take()?;
    tick.close_phase();
    let took = tick.started.elapsed();
    (took >= over).then(|| {
        let longest = tick.phases.into_iter().max_by_key(|(_, time)| *time);
        let (phase, time) = longest.expect("a closed phase is counted");
        let command = tick.slowest.map_or_else(String::new, |(command, time)| {
            format!(", {command} for {}s", time.as_secs())
        });
        format!(
            "a tick took {}s: {phase} for {}s{command}",
            took.as_secs(),
            time.as_secs()
        )
    })
}

/// When the tick in progress started, and what it runs, once it runs for over `over`.
pub fn stuck(over: Duration) -> Option<(Instant, String)> {
    let tick = tick();
    let tick = tick.as_ref()?;
    (tick.started.elapsed() >= over).then(|| {
        let command = tick
            .command
            .as_ref()
            .map_or_else(String::new, |(command, _)| format!(", {command}"));
        let what = format!(
            "a tick runs for over {}s: {}{command}",
            over.as_secs(),
            tick.phase.0
        );
        (tick.started, what)
    })
}

impl Tick {
    fn close_phase(&mut self) {
        let (name, since) = self.phase;
        *self.phases.entry(name).or_default() += since.elapsed();
    }
}

/// Marks `command` as what the tick runs until it drops, on the loop's thread.
pub(crate) struct Running(bool);

pub(crate) fn running(command: &Command) -> Running {
    if !LOOP.get() {
        return Running(false);
    }
    if let Some(tick) = tick().as_mut() {
        tick.command = Some((label(command), Instant::now()));
    }
    Running(true)
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.0 {
            return;
        }
        if let Some(tick) = tick().as_mut()
            && let Some((command, since)) = tick.command.take()
        {
            let took = since.elapsed();
            if tick.slowest.as_ref().is_none_or(|(_, most)| took > *most) {
                tick.slowest = Some((command, took));
            }
        }
    }
}

/// The program and its subcommand, such as `git fetch`. Other arguments can hold private text.
fn label(command: &Command) -> String {
    let program = std::path::Path::new(command.get_program())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut arguments = command
        .get_args()
        .map(|argument| argument.to_string_lossy());
    while let Some(argument) = arguments.next() {
        if matches!(&*argument, "-C" | "-c" | "-L" | "-S" | "-f") {
            arguments.next();
        } else if !argument.starts_with('-') {
            let word = argument.len() <= 24
                && argument
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'-');
            return match word {
                true => format!("{program} {argument}"),
                false => program.into_owned(),
            };
        }
    }
    program.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_tick_over_the_limit_names_its_phase_and_its_command_but_no_argument() {
        begin();
        assert_eq!(end(Duration::from_secs(10)), None, "a short tick is silent");
        assert_eq!(stuck(Duration::ZERO), None, "no tick runs");

        begin();
        phase("sweep");
        let mut git = crate::runtime::git(Path::new("/private"));
        git.args(["fetch", "secret"]);
        let running = running(&git);
        let (_, what) = stuck(Duration::ZERO).unwrap();
        assert_eq!(what, "a tick runs for over 0s: sweep, git fetch");
        std::thread::sleep(Duration::from_millis(20));
        drop(running);
        phase("runs");
        let said = end(Duration::ZERO).unwrap();
        assert_eq!(said, "a tick took 0s: sweep for 0s, git fetch for 0s");

        begin();
        for (name, millis) in [("messages", 200), ("signals", 250), ("messages", 200)] {
            phase(name);
            std::thread::sleep(Duration::from_millis(millis));
        }
        let said = end(Duration::ZERO).unwrap();
        assert_eq!(said, "a tick took 0s: messages for 0s", "a phase sums up");
    }
}
