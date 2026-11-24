//! Task labels and runner filters: which tasks a runner claims and starts, and when.

use crate::factory::{time_zone, weekdays};
use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::{Deserializer, Error as _};
use std::collections::BTreeMap;
use std::fmt;

/// A task's labels, such as `project = "oss"`.
pub type Labels = BTreeMap<String, String>;

/// Why `key` and `value` cannot be a label, if they cannot. A key is lower case letters, digits,
/// `_`, `.` and `-`; a value is letters, digits, `_`, `.`, `:`, `/` and `-`.
pub fn label(key: &str, value: &str) -> Result<(), String> {
    let key_ok = key.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c));
    if !key_ok {
        return Err(format!(
            "label key {key:?} is not lower case letters, digits, `_`, `.` or `-`"
        ));
    }
    let value_ok = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.:/-".contains(c));
    if !value_ok {
        return Err(format!(
            "label {key} has value {value:?}: use letters, digits, `_`, `.`, `:`, `/` or `-`"
        ));
    }
    Ok(())
}

/// `key=value` as a label, or why it is not one.
pub fn parse_label(text: &str) -> Result<(String, String), String> {
    let (key, value) = text
        .split_once('=')
        .ok_or_else(|| format!("{text:?} is not a label such as project=oss"))?;
    label(key, value)?;
    Ok((key.to_owned(), value.to_owned()))
}

pub(crate) fn labels<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Labels>, D::Error> {
    let labels = Labels::deserialize(input)?;
    for (key, value) in &labels {
        label(key, value).map_err(D::Error::custom)?;
    }
    Ok(Some(labels))
}

/// One filter of a runner file: the tasks it accepts, while its window is open.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    /// `key=value` accepts only tasks with that label; `!key=value` accepts only tasks without it.
    #[serde(rename = "match", deserialize_with = "matching")]
    #[schemars(
        with = "String",
        regex(pattern = r"^!?[a-z0-9][a-z0-9_.-]*=[A-Za-z0-9_.:/-]+$")
    )]
    pub matches: Match,
    /// When the filter applies; always when unset. Outside it, the filter is lifted.
    pub when: Option<Window>,
}

/// A filter's `match`.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub negated: bool,
    pub key: String,
    pub value: String,
}

impl fmt::Display for Match {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let not = if self.negated { "!" } else { "" };
        write!(f, "{not}{}={}", self.key, self.value)
    }
}

fn matching<'de, D: Deserializer<'de>>(input: D) -> Result<Match, D::Error> {
    let text = String::deserialize(input)?;
    let (negated, rest) = match text.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let (key, value) = parse_label(rest).map_err(|_| {
        D::Error::custom(format!(
            "{text:?} is not a match such as \"project=oss\" or \"!project=oss\""
        ))
    })?;
    Ok(Match {
        negated,
        key,
        value,
    })
}

/// The days and hours a filter applies, in `timezone`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Window {
    /// The weekdays it applies on; every day when unset. With `hours` that pass midnight, the day
    /// the hours start on.
    #[serde(default, deserialize_with = "weekdays")]
    #[schemars(
        with = "Vec<String>",
        length(min = 1),
        extend("items" = { "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] })
    )]
    pub days: Option<Vec<Weekday>>,
    /// The hours it applies, `HH:MM-HH:MM`, which may pass midnight; the whole day when unset.
    #[serde(default, deserialize_with = "hours")]
    #[schemars(
        with = "String",
        regex(pattern = r"^([01]?[0-9]|2[0-3]):[0-5][0-9]-([01]?[0-9]|2[0-3]):[0-5][0-9]$")
    )]
    pub hours: Option<(Time, Time)>,
    /// The IANA zone the days and hours are read in, such as `Europe/Paris`; the machine's own
    /// when unset.
    #[serde(default, deserialize_with = "time_zone")]
    #[schemars(with = "String")]
    pub timezone: Option<TimeZone>,
}

fn hours<'de, D: Deserializer<'de>>(input: D) -> Result<Option<(Time, Time)>, D::Error> {
    let text = String::deserialize(input)?;
    let bad = || D::Error::custom(format!("{text:?} is not hours such as \"08:00-18:00\""));
    let (from, to) = text.split_once('-').ok_or_else(bad)?;
    let from = Time::strptime("%H:%M", from).map_err(|_| bad())?;
    let to = Time::strptime("%H:%M", to).map_err(|_| bad())?;
    if from == to {
        return Err(D::Error::custom(format!(
            "{text:?} starts and ends at once: leave hours out for the whole day"
        )));
    }
    Ok(Some((from, to)))
}

impl Window {
    fn zone(&self) -> TimeZone {
        self.timezone.clone().unwrap_or_else(TimeZone::system)
    }

    fn on(&self, date: Date) -> bool {
        self.days
            .as_ref()
            .is_none_or(|days| days.contains(&date.weekday()))
    }

    /// Whether the window is open at `at`.
    pub fn open(&self, at: Timestamp) -> bool {
        let now = at.to_zoned(self.zone());
        let (date, time) = (now.date(), now.time());
        match self.hours {
            None => self.on(date),
            Some((from, to)) if from < to => self.on(date) && from <= time && time < to,
            Some((from, to)) => {
                (time >= from && self.on(date))
                    || (time < to && date.yesterday().is_ok_and(|day| self.on(day)))
            }
        }
    }

    /// The first instant after `at` at which the window, open at `at`, closes; `None` when it never
    /// does, as for every day with no hours.
    fn closes(&self, at: Timestamp) -> Option<Timestamp> {
        let zone = self.zone();
        let now = at.to_zoned(zone.clone());
        let at_time = |date: Date, time: Time| -> Option<Timestamp> {
            Some(
                date.to_datetime(time)
                    .to_zoned(zone.clone())
                    .ok()?
                    .timestamp(),
            )
        };
        match self.hours {
            Some((from, to)) if from > to && now.time() >= from => {
                at_time(now.date().tomorrow().ok()?, to)
            }
            Some((_, to)) => at_time(now.date(), to),
            None => {
                let mut date = now.date();
                for _ in 0..7 {
                    date = date.tomorrow().ok()?;
                    if !self.on(date) {
                        return at_time(date, Time::midnight());
                    }
                }
                None
            }
        }
    }
}

impl Filter {
    fn accepts(&self, labels: &Labels) -> bool {
        let has = labels.get(&self.matches.key) == Some(&self.matches.value);
        has != self.matches.negated
    }

    fn active(&self, at: Timestamp) -> bool {
        self.when.as_ref().is_none_or(|when| when.open(at))
    }
}

/// Why a runner does not take a task now: the first filter that refuses it, and when every filter
/// accepts it again, or `None` when no window lifts them.
#[derive(Debug, PartialEq)]
pub struct Refused<'a> {
    pub filter: &'a Match,
    pub until: Option<Timestamp>,
}

/// Whether `filters` accept a task with `labels` at `at`, or the filter that refuses it.
pub fn accepts<'a>(
    filters: &'a [Filter],
    labels: &Labels,
    at: Timestamp,
) -> Result<(), Refused<'a>> {
    let refusing = |at: Timestamp| {
        filters
            .iter()
            .filter(move |filter| filter.active(at) && !filter.accepts(labels))
    };
    let Some(first) = refusing(at).next() else {
        return Ok(());
    };
    // A filter that lifts can leave another that still refuses, so step until none does.
    let mut until = at;
    for _ in 0..16 {
        let lifts = refusing(until)
            .map(|filter| filter.when.as_ref().and_then(|when| when.closes(until)))
            .collect::<Option<Vec<_>>>();
        match lifts.as_deref() {
            None => break,
            Some([]) => {
                return Err(Refused {
                    filter: &first.matches,
                    until: Some(until),
                });
            }
            Some(lifts) => until = lifts.iter().copied().max().unwrap_or(until),
        }
    }
    Err(Refused {
        filter: &first.matches,
        until: None,
    })
}

/// `until` as a person reads it next to `now`: `18:00` the same day in `zone`, else `Mon 08:00`.
pub fn when(until: Timestamp, now: Timestamp, zone: &TimeZone) -> String {
    let (until, now): (Zoned, Zoned) = (until.to_zoned(zone.clone()), now.to_zoned(zone.clone()));
    if until.date() == now.date() {
        until.strftime("%H:%M").to_string()
    } else {
        until.strftime("%a %H:%M").to_string()
    }
}

/// The zone a runner's filters read their windows in: the first window's, else the machine's.
pub fn zone(filters: &[Filter]) -> TimeZone {
    filters
        .iter()
        .find_map(|filter| filter.when.as_ref()?.timezone.clone())
        .unwrap_or_else(TimeZone::system)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filters(toml: &str) -> Vec<Filter> {
        #[derive(Deserialize)]
        struct File {
            filters: Vec<Filter>,
        }
        toml::from_str::<File>(toml).unwrap().filters
    }

    fn oss() -> Labels {
        Labels::from([("project".to_owned(), "oss".to_owned())])
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    const WORK_HOURS: &str = r#"
        [[filters]]
        match = "!project=oss"
        when = { days = ["mon", "tue", "wed", "thu", "fri"], hours = "08:00-18:00", timezone = "Europe/Paris" }
    "#;

    #[test]
    fn a_negative_filter_refuses_the_label_in_its_window_and_lifts_at_its_end() {
        let filters = filters(WORK_HOURS);
        // Tuesday 10:00 in Paris, UTC+1 in November.
        let refused = accepts(&filters, &oss(), at("2026-11-24T09:00:00Z")).unwrap_err();
        assert_eq!(refused.filter.to_string(), "!project=oss");
        assert_eq!(refused.until, Some(at("2026-11-24T17:00:00Z")));
        assert!(accepts(&filters, &Labels::new(), at("2026-11-24T09:00:00Z")).is_ok());
        assert!(accepts(&filters, &oss(), at("2026-11-24T17:00:00Z")).is_ok());
        // Saturday: outside its days.
        assert!(accepts(&filters, &oss(), at("2026-11-28T09:00:00Z")).is_ok());
    }

    #[test]
    fn a_plain_filter_with_no_window_refuses_every_other_task_for_good() {
        let filters = filters("[[filters]]\nmatch = \"project=oss\"\n");
        assert!(accepts(&filters, &oss(), at("2026-11-24T09:00:00Z")).is_ok());
        let refused = accepts(&filters, &Labels::new(), at("2026-11-24T09:00:00Z")).unwrap_err();
        assert_eq!(refused.until, None);
    }

    #[test]
    fn hours_past_midnight_belong_to_the_day_they_start() {
        let filters = filters(
            r#"
            [[filters]]
            match = "!project=oss"
            when = { days = ["fri"], hours = "22:00-06:00", timezone = "UTC" }
            "#,
        );
        // Saturday 03:00 is in Friday's night.
        let refused = accepts(&filters, &oss(), at("2026-11-28T03:00:00Z")).unwrap_err();
        assert_eq!(refused.until, Some(at("2026-11-28T06:00:00Z")));
        // Friday 23:00 lifts the next morning.
        let refused = accepts(&filters, &oss(), at("2026-11-27T23:00:00Z")).unwrap_err();
        assert_eq!(refused.until, Some(at("2026-11-28T06:00:00Z")));
        // Friday 03:00 is in Thursday's night, which has no window.
        assert!(accepts(&filters, &oss(), at("2026-11-27T03:00:00Z")).is_ok());
    }

    #[test]
    fn days_without_hours_lift_at_the_first_midnight_off_them() {
        let filters = filters(
            r#"
            [[filters]]
            match = "!project=oss"
            when = { days = ["sat", "sun"], timezone = "UTC" }
            "#,
        );
        let refused = accepts(&filters, &oss(), at("2026-11-28T12:00:00Z")).unwrap_err();
        assert_eq!(refused.until, Some(at("2026-11-30T00:00:00Z")));
    }

    #[test]
    fn a_second_filter_that_still_refuses_moves_the_lift_on() {
        let filters = filters(
            r#"
            [[filters]]
            match = "!project=oss"
            when = { hours = "08:00-12:00", timezone = "UTC" }
            [[filters]]
            match = "!project=oss"
            when = { hours = "11:00-18:00", timezone = "UTC" }
            "#,
        );
        let refused = accepts(&filters, &oss(), at("2026-11-24T09:00:00Z")).unwrap_err();
        assert_eq!(refused.until, Some(at("2026-11-24T18:00:00Z")));
    }

    #[test]
    fn the_lift_reads_as_a_time_that_day_or_a_weekday_and_time() {
        let paris = TimeZone::get("Europe/Paris").unwrap();
        let now = at("2026-11-24T09:00:00Z");
        assert_eq!(when(at("2026-11-24T17:00:00Z"), now, &paris), "18:00");
        assert_eq!(when(at("2026-11-30T07:00:00Z"), now, &paris), "Mon 08:00");
    }

    #[test]
    fn bad_matches_hours_and_labels_are_refused_with_the_value() {
        let error = |toml: &str| {
            #[derive(Debug, Deserialize)]
            #[allow(dead_code)]
            struct File {
                filters: Vec<Filter>,
            }
            toml::from_str::<File>(toml).unwrap_err().to_string()
        };
        assert!(
            error("[[filters]]\nmatch = \"project\"\n")
                .contains("\"project\" is not a match such as \"project=oss\"")
        );
        assert!(
            error("[[filters]]\nmatch = \"!a=b\"\nwhen = { hours = \"8-18\" }\n")
                .contains("\"8-18\" is not hours such as \"08:00-18:00\"")
        );
        assert!(
            error("[[filters]]\nmatch = \"!a=b\"\nwhen = { hours = \"08:00-08:00\" }\n")
                .contains("starts and ends at once")
        );
        assert!(
            parse_label("Project=oss")
                .unwrap_err()
                .contains("label key \"Project\"")
        );
        assert!(
            parse_label("project=o s")
                .unwrap_err()
                .contains("label project has value \"o s\"")
        );
        assert_eq!(
            parse_label("project=oss").unwrap(),
            ("project".into(), "oss".into())
        );
    }
}
