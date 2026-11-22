//! The durations of a process's loop ticks, which its metrics exporter takes each interval.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static ON: AtomicBool = AtomicBool::new(false);
static TICKS: Mutex<Vec<f64>> = Mutex::new(Vec::new());

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
