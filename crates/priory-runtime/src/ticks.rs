//! The durations of a process's loop ticks, the other durations it times for a histogram, which
//! its metrics exporter takes each interval, and the counts it keeps for a counter.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
