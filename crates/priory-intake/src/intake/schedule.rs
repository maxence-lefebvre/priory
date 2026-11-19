use super::{id, publish, task_requested};
use herdr_bus::{Bus, Message, Timestamp};
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, ToSpan};
use priory_factory::factory::{Trigger, Until};
use serde_json::json;

pub(super) const FIRED: &str = "schedule.fired";

/// Publishes the latest firing of each schedule at or before `now`. Its id names the firing, so
/// herdr-bus drops a firing already published, and a restart fires only the latest missed one.
pub(super) fn fire(bus: &Bus, triggers: &[(&str, &Trigger)], now: Timestamp) -> Result<(), String> {
    for (name, trigger) in triggers {
        let Some(slot) = latest(trigger, now) else {
            continue;
        };
        let firing = format!("{name}@{slot}");
        let key = match &trigger.key {
            Some(key) => format!("schedule:{key}"),
            None => format!("schedule:{firing}"),
        };
        let data = json!({ "trigger": name, "scheduled": slot.to_string() });
        let source = Message::new(FIRED, data)
            .id(id(&format!("{FIRED}:{firing}")))
            .subject(format!("schedule:{firing}"))
            .time(slot);
        let mut task = task_requested(name, trigger, &source, key);
        task.data["scheduled"] = json!(slot.to_string());
        task.data["prompt"] = json!(trigger.prompt);
        publish(bus, &source)?;
        publish(bus, &task)?;
    }
    Ok(())
}

/// The latest time at or before `now` that the trigger fires.
fn latest(trigger: &Trigger, now: Timestamp) -> Option<Timestamp> {
    let at = trigger.at?;
    let zone = trigger.timezone.clone().unwrap_or_else(TimeZone::system);
    let instant = |day: Date, time: Time| {
        let zoned = day.to_datetime(time).to_zoned(zone.clone()).ok()?;
        Some(zoned.timestamp())
    };
    let today = now.to_zoned(zone.clone()).date();
    (0..=7_i64)
        .filter_map(|back| today.checked_sub(back.days()).ok())
        .filter(|day| {
            trigger
                .days
                .as_ref()
                .is_none_or(|days| days.contains(&day.weekday()))
        })
        .find_map(|day| {
            let first = instant(day, at)?;
            let end = match trigger.until {
                Some(Until::At(until)) => instant(day, until)?,
                Some(Until::After(after)) => first.checked_add(after).ok()?,
                None => instant(day, Time::MAX)?,
            };
            let last = end.min(now);
            if last < first {
                return None;
            }
            let Some(every) = trigger.every else {
                return Some(first);
            };
            let every = every.as_secs() as i64;
            let steps = (last.as_second() - first.as_second()) / every;
            first
                .checked_add(SignedDuration::from_secs(steps * every))
                .ok()
        })
}
