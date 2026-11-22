//! The command an agent's ask carries, which a person's answer runs at most once (R6.8).

use priory_factory::factory::{EVENT_FIELDS, Factory, placeholders};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A command as the `ask` tool stores it: its target resolved to a folder when it was asked, so a
/// later factory change never moves it.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Command {
    pub argv: Vec<String>,
    pub target: String,
    pub dir: PathBuf,
    pub runs: Vec<String>,
    /// How long it may run, in seconds, when its step sets a `timeout`; else `command_timeout`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

/// The `command` field as an agent sends it.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    argv: Vec<String>,
    target: Option<String>,
    runs: Option<Vec<String>>,
}

pub(crate) const WORKTREE: &str = "worktree";

/// The command of an ask with `options` and `recommendation`, from a task with `worktree`, or why
/// the tool refuses it.
pub(crate) fn parse(
    value: &Value,
    options: &[&str],
    recommendation: &str,
    worktree: Option<&str>,
    factory: &Factory,
) -> Result<Command, String> {
    let asked: Asked = serde_json::from_value(value.clone()).map_err(|error| {
        format!("command must be an object with argv, and optionally target and runs: {error}")
    })?;
    if asked.argv.first().is_none_or(String::is_empty) {
        return Err("command argv must start with a program".to_owned());
    }
    let target = asked.target.unwrap_or_else(|| WORKTREE.to_owned());
    let dir = match (target.as_str(), worktree) {
        (WORKTREE, Some(worktree)) => PathBuf::from(worktree),
        (WORKTREE, None) => return Err("command target worktree: this task has no worktree".into()),
        (name, _) => factory.target(name).ok_or_else(|| {
            format!(
                "command target {name:?} is not allowed: use {WORKTREE} or a name in [asks.targets]: {}",
                factory.targets().join(", ")
            )
        })?,
    };
    let runs = match asked.runs {
        Some(runs) => runs,
        None if options.contains(&recommendation) => vec![recommendation.to_owned()],
        None => {
            return Err(format!(
                "command needs runs: the recommendation {recommendation:?} is not one of the options"
            ));
        }
    };
    if let Some(run) = runs.iter().find(|run| !options.contains(&run.as_str())) {
        return Err(format!(
            "command runs {run:?}, which is not one of the options"
        ));
    }
    let dir = std::path::absolute(dir).map_err(|error| format!("command target: {error}"))?;
    Ok(Command {
        argv: asked.argv,
        target,
        dir,
        runs,
        timeout: None,
    })
}

/// The value of each event field that `texts` name, from the task's source event `data`, or why
/// the task cannot take them. A command runs with the person's sign-in, so `{head_sha}` takes only
/// a head the event says is in the event's own repository: never a fork's, nor an unknown one's.
pub(crate) fn event_values(
    texts: &[&str],
    data: Option<&Value>,
) -> Result<BTreeMap<&'static str, String>, String> {
    let mut values = BTreeMap::new();
    let named: Vec<&'static str> = EVENT_FIELDS
        .into_iter()
        .filter(|field| texts.iter().any(|text| placeholders(text).contains(field)))
        .collect();
    if named.is_empty() {
        return Ok(values);
    }
    let Some(data) = data else {
        return Err(format!(
            "the task has no source event, so nothing fills {{{}}}",
            named[0]
        ));
    };
    if named.contains(&"head_sha") {
        let number = data["pull_request"].as_u64().map_or_else(
            || "the pull request".to_owned(),
            |n| format!("pull request {n}"),
        );
        match (
            data.get("head_repo").and_then(Value::as_str),
            data["repo"].as_str(),
        ) {
            (Some(head), Some(repo)) if head == repo => {}
            (Some(head), _) => {
                return Err(format!(
                    "{number}'s head is in {head}, a fork: a command never runs a fork's commit"
                ));
            }
            (None, _) => {
                return Err(format!(
                    "the event names no head repository for {number}, so its head may be a fork's: a command never runs it"
                ));
            }
        }
    }
    for field in named {
        let value = match &data[field] {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => return Err(format!("the task's event names no {field}")),
        };
        if !fits(field, &value) {
            return Err(format!(
                "the task's event has {field} {value:?}, which a command cannot take"
            ));
        }
        values.insert(field, value);
    }
    Ok(values)
}

/// Whether `value` has the shape of the event field `field`, so it can never pass for an option.
fn fits(field: &str, value: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && !part.starts_with(['-', '.'])
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    match field {
        "repo" => value
            .split_once('/')
            .is_some_and(|(owner, name)| word(owner) && word(name)),
        "pull_request" => value.parse::<u64>().is_ok(),
        "head_sha" => {
            [40, 64].contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit())
        }
        _ => {
            !value.starts_with('-')
                && !value.is_empty()
                && value.bytes().all(|b| b.is_ascii_graphic())
        }
    }
}

/// `text` with each `{field}` that `values` holds replaced by its value.
pub(crate) fn filled(text: &str, values: &BTreeMap<&'static str, String>) -> String {
    values.iter().fold(text.to_owned(), |text, (field, value)| {
        text.replace(&format!("{{{field}}}"), value)
    })
}

impl Command {
    /// The argv as a JSON array, which shows every word, space and quote as it is.
    pub fn shown(&self) -> String {
        serde_json::to_string(&self.argv).expect("argv serializes")
    }

    /// The program to exec: a path with a slash resolved against the target folder, a bare name
    /// left for `PATH`.
    pub fn program(&self) -> String {
        let program = &self.argv[0];
        match program.contains('/') {
            true => self.dir.join(program).to_string_lossy().into_owned(),
            false => program.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    const TEXTS: [&str; 2] = [
        "Compare {pull_request} at {head_sha}?",
        "--base={base_branch} --post {repo}#{pull_request}",
    ];

    #[test]
    fn a_head_in_the_events_own_repository_fills_every_field() {
        let head = "4c1e2b7f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d";
        let data = json!({ "repo": "owner/factory", "head_repo": "owner/factory",
            "pull_request": 12, "head_sha": head, "base_branch": "main" });

        let values = event_values(&TEXTS, Some(&data)).unwrap();

        assert_eq!(filled(TEXTS[0], &values), format!("Compare 12 at {head}?"));
        assert_eq!(
            filled(TEXTS[1], &values),
            "--base=main --post owner/factory#12"
        );
    }

    #[test]
    fn a_value_that_could_pass_for_an_option_is_refused() {
        let data = json!({ "repo": "owner/factory", "pull_request": 12,
            "base_branch": "--upload-pack=touch" });

        assert_eq!(
            event_values(&TEXTS[1..], Some(&data)),
            Err("the task's event has base_branch \"--upload-pack=touch\", which a command cannot take".to_owned())
        );
    }

    proptest! {
        #[test]
        fn event_values_never_panic_on_any_event(
            fields in prop::collection::vec(
                (prop::sample::select(vec!["repo", "head_repo", "pull_request", "head_sha", "base_branch"]),
                 prop_oneof![".{0,48}".prop_map(Value::from), any::<i64>().prop_map(Value::from), Just(Value::Null)]),
                0..6),
            text in ".{0,40}",
        ) {
            let data: serde_json::Map<String, Value> =
                fields.into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
            let data = Value::Object(data);
            let texts = [text.as_str(), TEXTS[0], TEXTS[1]];
            if let Ok(values) = event_values(&texts, Some(&data)) {
                for value in values.values() {
                    prop_assert!(!value.starts_with('-'));
                }
            }
        }
    }
}
