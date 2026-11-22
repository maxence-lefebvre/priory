//! A benchmark case: a past task captured into a folder by `priory bench capture`, which a replay
//! plays again. [scorers-and-benchmarks.md](../../../docs/design/scorers-and-benchmarks.md) explains it.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const SCHEMAS: &str =
    "https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/";

/// `case.toml`: the task a case was captured from, what it was asked, where its replay starts and
/// ends, and what really happened to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "case")]
pub struct Case {
    /// The workflow the task ran.
    pub workflow: String,
    /// What the task worked on, prefixed by its source, such as `github:owner/repo#42`.
    pub key: String,
    /// The repository the task's worktree was cloned from, as `owner/name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The commit the task's worktree was created from. A replay clones the repository at it.
    pub start: String,
    /// The first irreversible step the task reached. A replay ends there and never runs it.
    pub ends_at: String,
    /// Whether the capture left a gap: a `gh` call it could not keep, or a transcript it could not
    /// read. A person fills each gap by hand or leaves it.
    pub partial: bool,
    /// Why the case is partial, one line per gap.
    pub gaps: Vec<String>,
    /// The message that asked for the task, as the inbox recorded it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Message>,
    /// What really happened to the task, for a reader and for scorers.
    pub reference: Reference,
}

/// A message the inbox recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// The message type, such as `factory.task.requested`.
    #[serde(rename = "type")]
    pub kind: String,
    /// What the message is about, such as `github:owner/repo#42`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The message's data, its null fields left out.
    #[schemars(schema_with = "table_schema")]
    pub data: BTreeMap<String, toml::Value>,
}

/// The task's real history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// The steps the task finished, in order, each as `step:outcome`.
    pub path: Vec<String>,
    /// The outcome labels the task got, such as `merged-clean`.
    pub labels: Vec<String>,
}

/// `gh/<n>.toml`: one `gh` command the task's agent ran before the first irreversible step, with
/// what it printed then. A replay's fake `gh` answers the same command with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "case gh answer")]
pub struct GhAnswer {
    /// The run whose agent ran the command.
    pub run: i64,
    /// The step that run was for.
    pub step: String,
    /// The command's arguments after `gh`.
    pub args: Vec<String>,
    /// What the command printed, as the harness transcript holds it.
    pub output: String,
}

/// `answers.toml`: the answer a person gave at each human step before the first irreversible
/// step, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "case answers")]
pub struct Answers {
    pub answers: Vec<Answer>,
}

/// A person's answer at one human step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    /// The human step.
    pub step: String,
    /// The option or text the person answered.
    pub answer: String,
}

/// `value`, such as a message's JSON data with no null in it, as a TOML table.
pub fn table<T: Serialize>(value: &T) -> Result<BTreeMap<String, toml::Value>, String> {
    match toml::Value::try_from(value).map_err(|error| error.to_string())? {
        toml::Value::Table(table) => Ok(table.into_iter().collect()),
        _ => Err("not a table".to_owned()),
    }
}

fn table_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object" })
}

/// The files of a case folder, by their path in it, with their text: `case.toml`, `answers.toml`
/// when a person answered, and one `gh/<n>.toml` per kept `gh` call, numbered from 001.
pub fn files(
    task: i64,
    case: &Case,
    gh: &[GhAnswer],
    answers: &[Answer],
) -> Result<Vec<(String, String)>, String> {
    let error = |error: toml::ser::Error| error.to_string();
    let mut files = vec![(
        "case.toml".to_owned(),
        format!(
            "#:schema {SCHEMAS}case.json\n# Captured from task {task} by priory bench capture. Read it before you share it.\n{}",
            toml::to_string(case).map_err(error)?
        ),
    )];
    if !answers.is_empty() {
        let answers = Answers {
            answers: answers.to_vec(),
        };
        files.push((
            "answers.toml".to_owned(),
            format!(
                "#:schema {SCHEMAS}case-answers.json\n{}",
                toml::to_string(&answers).map_err(error)?
            ),
        ));
    }
    for (index, answer) in gh.iter().enumerate() {
        files.push((
            format!("gh/{:03}.toml", index + 1),
            format!(
                "#:schema {SCHEMAS}case-gh.json\n{}",
                toml::to_string(answer).map_err(error)?
            ),
        ));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_file_reads_back_as_what_was_written() {
        let case = Case {
            workflow: "groom".to_owned(),
            key: "github:owner/repo#42".to_owned(),
            repo: Some("owner/repo".to_owned()),
            start: "abc".to_owned(),
            ends_at: "open-pr".to_owned(),
            partial: true,
            gaps: vec!["run 1: its transcript is gone".to_owned()],
            request: Some(Message {
                kind: "factory.task.requested".to_owned(),
                subject: Some("github:owner/repo#42".to_owned()),
                data: BTreeMap::from([("band".to_owned(), toml::Value::from("today"))]),
            }),
            reference: Reference {
                path: vec!["groom:red".to_owned()],
                labels: vec![],
            },
        };
        let gh = GhAnswer {
            run: 1,
            step: "groom".to_owned(),
            args: vec!["issue".to_owned(), "view".to_owned(), "42".to_owned()],
            output: "line '''\n\"quoted\"\n".to_owned(),
        };
        let answer = Answer {
            step: "approve-plan".to_owned(),
            answer: "approved".to_owned(),
        };

        let files = files(
            1,
            &case,
            std::slice::from_ref(&gh),
            std::slice::from_ref(&answer),
        )
        .unwrap();

        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["case.toml", "answers.toml", "gh/001.toml"]);
        assert_eq!(toml::from_str::<Case>(&files[0].1).unwrap(), case);
        assert_eq!(
            toml::from_str::<Answers>(&files[1].1).unwrap().answers,
            [answer]
        );
        assert_eq!(toml::from_str::<GhAnswer>(&files[2].1).unwrap(), gh);
    }
}
