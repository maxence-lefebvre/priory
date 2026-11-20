use super::{id, publish, task_requested};
use herdr_bus::{Bus, Message, Timestamp};
use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, ToSpan};
use priory_factory::factory::{Trigger, Until};
use serde_json::json;

pub(super) const FIRED: &str = "schedule.fired";

/// Publishes each schedule's latest firing, under an id that names it so herdr-bus drops a repeat.
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
    days(trigger, now, -7..=0).rev().find_map(|(first, end)| {
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

/// The first time after `now` that the trigger fires; none when it never does.
pub fn next(trigger: &Trigger, now: Timestamp) -> Option<Timestamp> {
    days(trigger, now, 0..=7).find_map(|(first, end)| {
        let slot = match trigger.every {
            _ if first > now => first,
            None => return None,
            Some(every) => {
                let every = every.as_secs() as i64;
                let steps = (now.as_second() - first.as_second()) / every + 1;
                first
                    .checked_add(SignedDuration::from_secs(steps * every))
                    .ok()?
            }
        };
        (slot <= end).then_some(slot)
    })
}

/// The first and last instants of each firing day from `offsets` days after today, in order.
fn days(
    trigger: &Trigger,
    now: Timestamp,
    offsets: std::ops::RangeInclusive<i64>,
) -> impl DoubleEndedIterator<Item = (Timestamp, Timestamp)> + '_ {
    let zone = trigger.timezone.clone().unwrap_or_else(TimeZone::system);
    let today = now.to_zoned(zone.clone()).date();
    let instant = move |day: Date, time: Time| {
        let zoned = day.to_datetime(time).to_zoned(zone.clone()).ok()?;
        Some(zoned.timestamp())
    };
    offsets
        .filter_map(move |offset| today.checked_add(offset.days()).ok())
        .filter(|day| {
            trigger
                .days
                .as_ref()
                .is_none_or(|days| days.contains(&day.weekday()))
        })
        .filter_map(move |day| {
            let first = instant(day, trigger.at?)?;
            let midnight = instant(day, Time::MAX)?;
            let end = match trigger.until {
                Some(Until::At(until)) => instant(day, until)?,
                Some(Until::After(after)) => first.checked_add(after).ok()?.min(midnight),
                None => midnight,
            };
            Some((first, end))
        })
}
