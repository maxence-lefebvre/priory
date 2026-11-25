use crate::filters::Window;
use jiff::Timestamp;
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
    /// With `when`, only inside that window.
    #[serde(default, deserialize_with = "crate::factory::max_running")]
    #[schemars(range(min = 1))]
    pub max_running: Option<u32>,
    /// When `max_running` holds; always when unset.
    pub when: Option<Window>,
    /// The most that may run at once outside `when`; no cap when unset.
    #[serde(default, deserialize_with = "crate::factory::max_running")]
    #[schemars(range(min = 1))]
    pub max_running_outside: Option<u32>,
}

impl Lane {
    /// Whether the table sets `when` with no cap to hold in it, or `max_running_outside` with no
    /// window to be outside of.
    pub fn is_incomplete(&self) -> bool {
        self.when.is_some() && self.max_running.is_none()
            || self.max_running_outside.is_some() && self.when.is_none()
    }

    /// The cap in force at `at`.
    pub fn cap(&self, at: Timestamp) -> Option<u32> {
        match &self.when {
            Some(when) if !when.open(at) => self.max_running_outside,
            _ => self.max_running,
        }
    }
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
    use super::glob;

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
