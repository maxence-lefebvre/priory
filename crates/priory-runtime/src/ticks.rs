//! The durations of a process's loop ticks, and the other durations it times for a histogram,
//! which its metrics exporter takes each interval.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ON: AtomicBool = AtomicBool::new(false);
static TICKS: Mutex<Vec<f64>> = Mutex::new(Vec::new());
static TIMED: Mutex<Vec<Timed>> = Mutex::new(Vec::new());

/// One duration a process timed: metric `metric`, under `outcome` when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Timed {
    pub metric: &'static str,
    pub outcome: Option<&'static str>,
    pub seconds: f64,
}

/// Starts keeping ticks; a process with no exporter keeps none.
pub fn keep() {
    ON.store(true, Ordering::SeqCst);
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
    timed_as(metric, None, elapsed);
}

/// Records one duration of `metric` that ended as `outcome`.
pub fn timed_as(metric: &'static str, outcome: Option<&'static str>, elapsed: Duration) {
    if ON.load(Ordering::Relaxed)
        && let Ok(mut timed) = TIMED.lock()
    {
        timed.push(Timed {
            metric,
            outcome,
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
