//! A benchmark case: a past task captured into a folder by `priory bench capture`, which a replay
//! plays again. [scorers-and-benchmarks.md](../../../docs/design/scorers-and-benchmarks.md) explains it.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
    /// What each model step before `ends_at` spent, in order, for a replay's estimate.
    #[serde(default)]
    pub spent: Vec<Spent>,
}

/// What one model step of the task spent before the first irreversible step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Spent {
    /// The model step.
    pub step: String,
    /// The model the run reported, such as `claude-sonnet-5-5`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How long the step took, in whole minutes rounded up, at least 1.
    #[schemars(range(min = 1))]
    pub minutes: u64,
    /// The step's tokens. Absent when Priory could not read them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
}

/// Tokens of each kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
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

/// A case folder as a replay reads it: `case.toml`, its `gh/` answers in file order, and the
/// answers of `answers.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub case: Case,
    pub gh: Vec<GhAnswer>,
    pub answers: Vec<Answer>,
}

/// Reads the case folder `dir`. Each error names the file.
pub fn load(dir: &Path) -> Result<Loaded, String> {
    fn read<T: serde::de::DeserializeOwned>(file: &Path) -> Result<T, String> {
        let text =
            fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
        toml::from_str(&text).map_err(|error| format!("{}: {error}", file.display()))
    }
    let case = read(&dir.join("case.toml"))?;
    let answers = dir.join("answers.toml");
    let answers = if answers.exists() {
        read::<Answers>(&answers)?.answers
    } else {
        Vec::new()
    };
    let mut files = match fs::read_dir(dir.join("gh")) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("{}: {error}", dir.join("gh").display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("{}: {error}", dir.join("gh").display())),
    };
    files.retain(|file| {
        file.extension()
            .is_some_and(|extension| extension == "toml")
    });
    files.sort();
    let gh = files
        .iter()
        .map(|file| read(file))
        .collect::<Result<_, _>>()?;
    Ok(Loaded { case, gh, answers })
}

/// The keys of `factory.toml` a replay leaves out: each reaches past the scratch folder, starts a
/// model call the budget does not count, or runs a script.
pub const LEFT_OUT: [&str; 10] = [
    "log",
    "telemetry",
    "notify",
    "serve",
    "webhooks",
    "slack",
    "asks",
    "handler",
    "search",
    "sweep",
];

/// The text of a replay's `factory.toml`, from the factory folder `dir`: its `factory.toml` or
/// `factory.yaml` without the keys of `LEFT_OUT`, and with a kit repository given as a relative
/// path made absolute, since the replay's copy of the factory lives elsewhere.
pub fn replay_settings(dir: &Path) -> Result<String, String> {
    let found = ["factory.toml", "factory.yaml", "factory.yml"]
        .map(|name| dir.join(name))
        .into_iter()
        .find(|file| file.is_file())
        .ok_or_else(|| format!("{}: no factory.toml or factory.yaml", dir.display()))?;
    let text =
        fs::read_to_string(&found).map_err(|error| format!("{}: {error}", found.display()))?;
    let at = |error: String| format!("{}: {error}", found.display());
    let mut table: toml::Table = if found.extension().is_some_and(|ext| ext == "toml") {
        toml::from_str(&text).map_err(|error| at(error.to_string()))?
    } else {
        serde_saphyr::from_str(&text).map_err(|error| at(error.to_string()))?
    };
    for key in LEFT_OUT {
        table.remove(key);
    }
    if let Some(toml::Value::String(repo)) = table
        .get_mut("kit")
        .and_then(|kit| kit.as_table_mut())
        .and_then(|kit| kit.get_mut("repo"))
    {
        let trimmed = repo.trim();
        if !trimmed.contains(':') && Path::new(trimmed).is_relative() {
            let absolute =
                std::path::absolute(dir.join(trimmed)).map_err(|error| at(error.to_string()))?;
            *repo = absolute.display().to_string();
        }
    }
    toml::to_string(&table).map_err(|error| at(error.to_string()))
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
            spent: vec![Spent {
                step: "groom".to_owned(),
                model: Some("claude-sonnet-5-5".to_owned()),
                minutes: 3,
                tokens: Some(Tokens {
                    input: 1,
                    output: 2,
                    cache_read: 3,
                    cache_write: 4,
                }),
            }],
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
            std::slice::from_ref(&answer)
        );
        assert_eq!(toml::from_str::<GhAnswer>(&files[2].1).unwrap(), gh);

        let dir = std::env::temp_dir().join(format!("priory-case-{}", std::process::id()));
        for (path, text) in &files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, text).unwrap();
        }
        let loaded = load(&dir);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            loaded.unwrap(),
            Loaded {
                case,
                gh: vec![gh],
                answers: vec![answer]
            }
        );
    }

    #[test]
    fn a_replay_s_settings_leave_out_what_reaches_past_the_scratch_folder() {
        let dir = std::env::temp_dir().join(format!("priory-settings-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("factory.yaml"),
            "name: demo\nhandler: reviewer\ngate: cargo test\nlog:\n  backend: postgres\nserve:\n  listen: 127.0.0.1:7340\nkit:\n  repo: ../kit\n",
        )
        .unwrap();

        let settings = replay_settings(&dir);

        let absolute = std::path::absolute(dir.join("../kit")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let settings: toml::Table = toml::from_str(&settings.unwrap()).unwrap();
        let keys: Vec<&str> = settings.keys().map(String::as_str).collect();
        assert_eq!(keys, ["name", "gate", "kit"]);
        assert_eq!(
            settings["kit"]["repo"].as_str(),
            Some(absolute.display().to_string().as_str())
        );
    }
}
