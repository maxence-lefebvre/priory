use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

/// `lanes.toml`: how a task without a lane of its own gets one. Without the file, every task gets
/// the `{team}/misc` fallback.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "lanes")]
pub struct Lanes {
    /// A lane, in which `{team}` stands for the task's team, or `none` without one.
    pub fallback: Option<String>,
    /// The rules, tried in order: the first that matches gives the lane.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Each lane's settings, by its `<team>/<theme>` name.
    #[serde(default)]
    pub lane: BTreeMap<String, Lane>,
}

/// `[lane."<team>/<theme>"]`: how the queue treats one lane.
#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lane {
    /// The most of the lane's tasks that may have a live run at once; new starts past it wait.
    /// With `days` or `hours`, only inside that window.
    #[serde(default, deserialize_with = "crate::factory::max_running")]
    #[schemars(range(min = 1))]
    pub max_running: Option<u32>,
    /// The weekdays `max_running` holds on; every day when unset.
    #[serde(default, deserialize_with = "crate::factory::weekdays")]
    #[schemars(
        with = "Vec<String>",
        length(min = 1),
        extend("items" = { "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] })
    )]
    pub days: Option<Vec<jiff::civil::Weekday>>,
    /// The local hours `max_running` holds in, `HH:MM-HH:MM`; all day when unset.
    #[serde(default, deserialize_with = "hours")]
    #[schemars(
        with = "String",
        regex(pattern = r"^([01]?[0-9]|2[0-3]):[0-5][0-9]-([01]?[0-9]|2[0-3]):[0-5][0-9]$")
    )]
    pub hours: Option<(jiff::civil::Time, jiff::civil::Time)>,
    /// The most that may run at once outside `days` and `hours`; no cap when unset.
    #[serde(default, deserialize_with = "crate::factory::max_running")]
    #[schemars(range(min = 1))]
    pub max_running_outside: Option<u32>,
}

impl Lane {
    /// Whether the table sets a window or an outside cap with no `max_running` to frame them.
    pub fn is_unframed(&self) -> bool {
        self.max_running.is_none()
            && (self.days.is_some() || self.hours.is_some() || self.max_running_outside.is_some())
    }

    /// The cap in force at `now`, a time in the runner's own zone.
    pub fn cap(&self, now: &jiff::Zoned) -> Option<u32> {
        let day = self
            .days
            .as_ref()
            .is_none_or(|days| days.contains(&now.weekday()));
        let hour = self
            .hours
            .is_none_or(|(start, end)| (start..end).contains(&now.time()));
        if day && hour {
            self.max_running
        } else {
            self.max_running_outside
        }
    }

    /// The window as `mon-fri 08:00-18:00`, or none when the cap holds at every hour.
    pub fn window(&self) -> Option<String> {
        const NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
        let days = self.days.as_ref().map(|days| {
            let mut indexes: Vec<_> = days
                .iter()
                .map(|day| day.to_monday_zero_offset() as usize)
                .collect();
            indexes.sort_unstable();
            indexes.dedup();
            let (first, last) = (indexes[0], indexes[indexes.len() - 1]);
            if indexes.len() > 2 && last - first + 1 == indexes.len() {
                format!("{}-{}", NAMES[first], NAMES[last])
            } else {
                let names: Vec<_> = indexes.iter().map(|index| NAMES[*index]).collect();
                names.join(",")
            }
        });
        let hours = self
            .hours
            .map(|(start, end)| format!("{}-{}", start.strftime("%H:%M"), end.strftime("%H:%M")));
        match (days, hours) {
            (None, None) => None,
            (days, hours) => Some(
                [days, hours]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        }
    }
}

fn hours<'de, D: serde::Deserializer<'de>>(
    input: D,
) -> Result<Option<(jiff::civil::Time, jiff::civil::Time)>, D::Error> {
    use serde::de::Error;
    let text = String::deserialize(input)?;
    let parse = |part: &str| jiff::civil::Time::strptime("%H:%M", part).ok();
    let Some((start, end)) = text
        .split_once('-')
        .and_then(|(start, end)| Some((parse(start)?, parse(end)?)))
    else {
        return Err(D::Error::custom(format!(
            "{text:?} is not a window of hours such as \"08:00-18:00\""
        )));
    };
    if end <= start {
        return Err(D::Error::custom(format!(
            "{text:?} ends before it starts: a window ends the day it starts"
        )));
    }
    Ok(Some((start, end)))
}

/// A rule sets at least one of `key`, `team`, `project` and `branch`, and matches a task that fits
/// each one it sets.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// The task's key, exactly.
    pub key: Option<String>,
    /// The team of the task's source event, exactly.
    pub team: Option<String>,
    /// The project of the task's source event, exactly.
    pub project: Option<String>,
    /// A pattern for the task's branch, in which `*` matches any run of characters.
    pub branch: Option<String>,
    /// The lane the rule gives, `<team>/<theme>`.
    pub lane: String,
}

/// What a lane is derived from. The branch is unknown until the task's first run.
#[derive(Debug, Default)]
pub struct Subject<'a> {
    pub key: &'a str,
    pub team: Option<&'a str>,
    pub project: Option<&'a str>,
    pub branch: Option<&'a str>,
}

impl Lanes {
    /// The lane of the first matching rule, else the fallback, else `{team}/misc`.
    pub fn lane(&self, subject: &Subject) -> String {
        self.rules
            .iter()
            .find(|rule| rule.matches(subject))
            .map_or_else(
                || {
                    let fallback = self.fallback.as_deref().unwrap_or("{team}/misc");
                    fallback.replace("{team}", subject.team.unwrap_or("none"))
                },
                |rule| rule.lane.clone(),
            )
    }
}

impl Rule {
    pub fn is_bare(&self) -> bool {
        [&self.key, &self.team, &self.project, &self.branch]
            .iter()
            .all(|field| field.is_none())
    }

    /// Every field the rule sets must match, and a rule that sets none matches nothing.
    fn matches(&self, subject: &Subject) -> bool {
        let same = |rule: &Option<String>, task: Option<&str>| {
            rule.as_deref().is_none_or(|rule| task == Some(rule))
        };
        !self.is_bare()
            && same(&self.key, Some(subject.key))
            && same(&self.team, subject.team)
            && same(&self.project, subject.project)
            && self
                .branch
                .as_deref()
                .is_none_or(|pattern| subject.branch.is_some_and(|branch| glob(pattern, branch)))
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut parts = parts.peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(part);
        }
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    rest.is_empty()
}

/// Whether `lane` reads `<team>/<theme>`, once `{team}` is filled in.
pub fn well_formed(lane: &str) -> bool {
    let lane = lane.replace("{team}", "team");
    lane.split_once('/').is_some_and(|(team, theme)| {
        [team, theme].iter().all(|part| {
            !part.is_empty()
                && !part.contains(|c: char| c == '/' || c.is_whitespace() || c.is_control())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{Lane, glob};

    #[test]
    fn a_window_reads_as_a_day_range_or_list_and_hours() {
        let lane = |toml: &str| -> Lane { toml::from_str(toml).unwrap() };
        let weekdays = lane(
            "max_running = 2\ndays = [\"fri\", \"mon\", \"tue\", \"wed\", \"thu\"]\nhours = \"08:00-18:00\"",
        );
        assert_eq!(weekdays.window().as_deref(), Some("mon-fri 08:00-18:00"));
        assert_eq!(
            lane("max_running = 2\ndays = [\"mon\", \"sat\"]")
                .window()
                .as_deref(),
            Some("mon,sat")
        );
        assert_eq!(lane("max_running = 2").window(), None);

        let at = |time: &str| -> jiff::Zoned { format!("{time}[UTC]").parse().unwrap() };
        let mut lane = weekdays;
        assert_eq!(lane.cap(&at("2026-11-23T08:00:00")), Some(2));
        assert_eq!(lane.cap(&at("2026-11-23T18:00:00")), None);
        assert_eq!(lane.cap(&at("2026-11-28T10:00:00")), None);
        lane.max_running_outside = Some(6);
        assert_eq!(lane.cap(&at("2026-11-23T07:59:00")), Some(6));
    }

    #[test]
    fn a_star_matches_any_run() {
        assert!(glob("*desktop*", "priory/desktop-7"));
        assert!(glob("priory/*", "priory/1"));
        assert!(glob("a*b*c", "aXbYc"));
        assert!(glob("exact", "exact"));
        assert!(!glob("*desktop*", "priory/1"));
        assert!(!glob("a*b", "ab-"));
        assert!(!glob("exact", "exactly"));
        assert!(!glob("a*a", "a"));
    }
}
