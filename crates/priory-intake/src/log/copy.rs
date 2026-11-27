//! Moves a herdr-bus log into an empty Postgres log, keeping each message's seq and each
//! consumer's cursor, so that the intake and the daemons resume where they stood.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;
use std::time::Duration;

use herdr_bus::{Bus, MessageQuery, Stored};

use super::Result;
use super::postgres::{Cursor, Postgres};

/// How long the copy's one transaction may run.
pub const TIMEOUT: Duration = Duration::from_secs(600);

/// Copies the herdr-bus file at `path` into `target`, or with `dry_run` rolls the copy back, and
/// returns the counts per output and the cursor per consumer.
pub fn from_herdr_bus(path: &Path, target: &Postgres, dry_run: bool) -> Result<String> {
    // `Bus::open` creates a missing file, which would copy an empty log without a word.
    if !path.is_file() {
        return Err(format!("{} is no herdr-bus file", path.display()).into());
    }
    let bus = Bus::open(path)?;
    let messages = all(&bus)?;
    let here = format!("@{}", bus.id());
    let mut cursors = Vec::new();
    let mut skipped = Vec::new();
    for connection in bus.connections()? {
        // Postgres keeps neither taps nor disabled connections.
        if connection.output == herdr_bus::TAP || !connection.enabled {
            skipped.push(format!(
                "{} on {}{}",
                connection.consumer,
                connection.output,
                if connection.enabled { "" } else { ", disabled" }
            ));
            continue;
        }
        // Priory's readers go by their bare name, as `Log::backlog` shows them.
        let consumer = connection
            .consumer
            .strip_suffix(&here)
            .map_or_else(|| connection.consumer.clone(), str::to_owned);
        cursors.push(Cursor {
            consumer,
            output: connection.output,
            seq: connection.cursor.0,
        });
    }
    target.copy_in(&messages, &cursors, dry_run)?;

    let mut outputs = BTreeMap::<&str, usize>::new();
    for stored in &messages {
        *outputs.entry(&stored.output).or_default() += 1;
    }
    let mut report = String::new();
    let verb = if dry_run { "would copy" } else { "copied" };
    writeln!(report, "{verb} {} messages", messages.len())?;
    for (output, count) in outputs {
        writeln!(report, "  {output}: {count}")?;
    }
    writeln!(report, "{verb} {} cursors", cursors.len())?;
    for cursor in &cursors {
        writeln!(
            report,
            "  {} on {}: {}",
            cursor.consumer, cursor.output, cursor.seq
        )?;
    }
    if !skipped.is_empty() {
        writeln!(
            report,
            "skipped {} connections Postgres has no place for",
            skipped.len()
        )?;
        for connection in skipped {
            writeln!(report, "  {connection}")?;
        }
    }
    write!(
        report,
        "left in herdr-bus: its id, its outputs' descriptions and its requests' replies"
    )?;
    if dry_run {
        write!(report, "\ndry run: the Postgres log is unchanged")?;
    }
    Ok(report)
}

/// Every message of `bus`, oldest first.
fn all(bus: &Bus) -> Result<Vec<Stored>> {
    let mut messages = Vec::new();
    let mut before = None;
    loop {
        let page = bus.messages(&MessageQuery {
            before,
            limit: 1000,
            ..MessageQuery::default()
        })?;
        let Some(oldest) = page.last() else {
            break;
        };
        before = Some(oldest.seq);
        messages.extend(page);
    }
    messages.reverse();
    Ok(messages)
}
