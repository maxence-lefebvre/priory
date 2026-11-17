use crate::lines::{self, Lines};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug)]
pub struct Problem {
    pub file: PathBuf,
    pub at: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.at {
            Some(at) => write!(f, "{}:{at}: {}", self.file.display(), self.message),
            None => write!(f, "{}: {}", self.file.display(), self.message),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Agent {
    pub description: String,
    pub harness: Harness,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub runner: Option<String>,
    #[serde(default)]
    pub session: Session,
    /// What a runner must have free to admit a run of this agent, in bytes.
    #[serde(default, deserialize_with = "size")]
    pub memory: Option<u64>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub permissions: Permissions,
    /// The Markdown after the frontmatter: the agent's instructions.
    #[serde(skip)]
    pub body: String,
}

/// Claude Code's own `permissions` settings, so a person can copy rules across.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Permissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<PermissionMode>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    Auto,
    DontAsk,
    BypassPermissions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Session {
    #[default]
    PerTask,
    Shared,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub source: String,
    pub repo: Option<String>,
    pub on: String,
    pub view: Option<String>,
    pub filter: Option<BTreeMap<String, String>>,
    pub workflow: String,
    pub key: Option<String>,
    pub band: Option<String>,
    pub lane: Option<String>,
    pub backfill: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub start: String,
    pub steps: Vec<Step>,
}

impl Workflow {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// Whether a path from the step `from` reaches the step `to`, `from` itself included.
    pub fn reaches(&self, from: &str, to: &str) -> bool {
        let steps = self
            .steps
            .iter()
            .map(|step| (step.id.as_str(), step))
            .collect();
        crate::rules::reachable(&steps, from, |_| false).contains(to)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub decides: Decider,
    pub effect: Effect,
    pub agent: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    pub action: Option<String>,
    #[serde(default)]
    pub approves: Vec<String>,
    pub next: BTreeMap<String, String>,
    pub autonomous: Option<String>,
    /// How long one turn of a model step may run before Priory ends it.
    #[serde(default, deserialize_with = "duration")]
    pub timeout: Option<Duration>,
}

/// A duration such as `90s`, `15m` or `2h`.
fn duration<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<Duration>, D::Error> {
    let text = String::deserialize(input)?;
    [("s", 1), ("m", 60), ("h", 3600)]
        .into_iter()
        .find_map(|(unit, scale)| {
            text.strip_suffix(unit)?
                .parse::<u64>()
                .ok()?
                .checked_mul(scale)
        })
        // The daemon compares turns in SQLite's signed seconds.
        .filter(|seconds| (1..=i64::MAX as u64).contains(seconds))
        .map(|seconds| Some(Duration::from_secs(seconds)))
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not a duration such as 90s, 15m or 2h"))
        })
}

/// A size such as `512MB`, `6GB` or `1.5TB`, in bytes. Units are binary, as in `/proc/meminfo`.
fn size<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<u64>, D::Error> {
    let text = String::deserialize(input)?;
    [("KB", 10), ("MB", 20), ("GB", 30), ("TB", 40)]
        .into_iter()
        .find_map(|(unit, shift)| {
            let number = text.strip_suffix(unit)?.parse::<f64>().ok()?;
            let bytes = number * (1u64 << shift) as f64;
            (bytes >= 1.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
        })
        .map(Some)
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not a size such as 512MB or 6GB"))
        })
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Decider {
    Code,
    Model,
    Human,
}

impl Decider {
    pub fn name(&self) -> &'static str {
        match self {
            Decider::Code => "code",
            Decider::Model => "model",
            Decider::Human => "human",
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Effect {
    None,
    Read,
    Reversible,
    Irreversible,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runner {
    pub platform: Platform,
    pub agents: Option<Share>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    pub os: Os,
    pub arch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    /// The fraction of the machine's CPUs the runner's agents may load together.
    #[serde(default, deserialize_with = "percent")]
    pub cpu: Option<f64>,
    /// The fraction of the machine's memory the runner's agents may take together.
    #[serde(default, deserialize_with = "percent")]
    pub memory: Option<f64>,
}

/// A share such as `60%`, as a fraction above 0 and at most 1.
fn percent<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<f64>, D::Error> {
    let text = String::deserialize(input)?;
    text.strip_suffix('%')
        .and_then(|number| number.parse::<f64>().ok())
        .filter(|number| *number > 0.0 && *number <= 100.0)
        .map(|number| Some(number / 100.0))
        .ok_or_else(|| {
            serde::de::Error::custom(format!("{text:?} is not a share such as 60% or 90%"))
        })
}

pub struct Loaded<T> {
    pub file: PathBuf,
    pub value: T,
    pub lines: Lines,
}

#[derive(Default)]
pub struct Factory {
    pub agents: BTreeMap<String, Loaded<Agent>>,
    pub triggers: BTreeMap<String, Loaded<Trigger>>,
    pub workflows: BTreeMap<String, Loaded<Workflow>>,
    pub runners: BTreeMap<String, Loaded<Runner>>,
}

pub fn load(dir: &Path) -> (Factory, Vec<Problem>) {
    let mut factory = Factory::default();
    let mut problems = Vec::new();
    let mut found = Loader {
        dir,
        problems: &mut problems,
    };

    let mut settings = BTreeMap::new();
    for file in ["factory.toml", "factory.yaml", "factory.yml"].map(|name| dir.join(name)) {
        if let Some((value, lines)) = file
            .is_file()
            .then(|| found.parse::<Settings>(&file))
            .flatten()
        {
            found.define(&mut settings, "factory".into(), &file, value, lines);
        }
    }
    if settings.is_empty() && found.problems.is_empty() {
        found.problems.push(Problem {
            file: PathBuf::from("factory.toml"),
            at: None,
            message: "missing: a factory needs factory.toml or factory.yaml".into(),
        });
    }

    for path in found
        .entries("agents")
        .into_iter()
        .filter(|path| path.is_dir())
    {
        let file = path.join("agent.md");
        let Some(source) = found.read(&file) else {
            continue;
        };
        if let Some((mut value, lines)) = found.parse_source::<Agent>(&file, &source) {
            value.body = body(&source);
            found.define(&mut factory.agents, file_name(&path), &file, value, lines);
        }
    }
    for file in found.config_files("triggers") {
        if let Some((value, lines)) = found.parse::<Trigger>(&file) {
            found.define(&mut factory.triggers, stem(&file), &file, value, lines);
        }
    }
    for file in found.config_files("workflows") {
        if let Some((value, lines)) = found.parse::<Workflow>(&file) {
            found.define(&mut factory.workflows, stem(&file), &file, value, lines);
        }
    }
    for file in found.config_files("runners") {
        if let Some((value, lines)) = found.parse::<Runner>(&file) {
            found.define(&mut factory.runners, stem(&file), &file, value, lines);
        }
    }
    (factory, problems)
}

struct Loader<'a> {
    dir: &'a Path,
    problems: &'a mut Vec<Problem>,
}

impl Loader<'_> {
    fn relative(&self, file: &Path) -> PathBuf {
        file.strip_prefix(self.dir).unwrap_or(file).to_path_buf()
    }

    fn define<T>(
        &mut self,
        map: &mut BTreeMap<String, Loaded<T>>,
        name: String,
        file: &Path,
        value: T,
        lines: Lines,
    ) {
        let file = self.relative(file);
        if let Some(existing) = map.get(&name) {
            let message = format!("`{name}` is also defined in {}", existing.file.display());
            self.problems.push(Problem {
                file,
                at: None,
                message,
            });
            return;
        }
        map.insert(name, Loaded { file, value, lines });
    }

    /// The entries of a factory folder; a missing folder has none.
    fn entries(&mut self, folder: &str) -> Vec<PathBuf> {
        let read = match fs::read_dir(self.dir.join(folder)) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                self.problems.push(Problem {
                    file: folder.into(),
                    at: None,
                    message: error.to_string(),
                });
                return Vec::new();
            }
        };
        let mut paths = Vec::new();
        for entry in read {
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(error) => self.problems.push(Problem {
                    file: folder.into(),
                    at: None,
                    message: error.to_string(),
                }),
            }
        }
        paths.sort();
        paths
    }

    fn config_files(&mut self, folder: &str) -> Vec<PathBuf> {
        self.entries(folder)
            .into_iter()
            .filter(|path| {
                matches!(
                    path.extension().and_then(|ext| ext.to_str()),
                    Some("toml" | "yaml" | "yml")
                )
            })
            .collect()
    }

    fn read(&mut self, file: &Path) -> Option<String> {
        match fs::read_to_string(file) {
            Ok(source) => Some(source),
            Err(error) => {
                self.problems.push(Problem {
                    file: self.relative(file),
                    at: None,
                    message: error.to_string(),
                });
                None
            }
        }
    }

    fn parse<T: DeserializeOwned>(&mut self, file: &Path) -> Option<(T, Lines)> {
        let source = self.read(file)?;
        self.parse_source(file, &source)
    }

    fn parse_source<T: DeserializeOwned>(
        &mut self,
        file: &Path,
        source: &str,
    ) -> Option<(T, Lines)> {
        let relative = self.relative(file);
        let parsed = match file.extension().and_then(|ext| ext.to_str()) {
            Some("md") => frontmatter(source),
            Some("toml") => Ok(Document {
                format: Format::Toml,
                text: source,
                first_line: 1,
            }),
            _ => Ok(Document {
                format: Format::Yaml,
                text: source,
                first_line: 1,
            }),
        }
        .and_then(|document| {
            let value = document.deserialize()?;
            Ok((value, document.lines()))
        });
        match parsed {
            Ok(parsed) => Some(parsed),
            Err((at, message)) => {
                self.problems.push(Problem {
                    file: relative,
                    at,
                    message,
                });
                None
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Format {
    Toml,
    Yaml,
}

#[derive(Clone, Copy)]
struct Document<'a> {
    format: Format,
    text: &'a str,
    first_line: usize,
}

type ParseError = (Option<String>, String);

impl Document<'_> {
    fn lines(self) -> Lines {
        match self.format {
            Format::Toml => lines::toml(self.text, self.first_line),
            Format::Yaml => lines::yaml(self.text, self.first_line),
        }
        .unwrap_or_default()
    }

    fn deserialize<T: DeserializeOwned>(self) -> Result<T, ParseError> {
        let offset = self.first_line - 1;
        match self.format {
            Format::Toml => toml::from_str(self.text).map_err(|error| {
                let at = error.span().map(|span| {
                    let (line, column) = line_column(self.text, span.start);
                    format!("{}:{column}", line + offset)
                });
                (at, error.message().trim().to_string())
            }),
            Format::Yaml => serde_saphyr::from_str(self.text).map_err(|error| {
                let at = error.location().map(|location| {
                    format!(
                        "{}:{}",
                        location.line() as usize + offset,
                        location.column()
                    )
                });
                let message = error.without_snippet().to_string();
                // saphyr always suffixes its plain message with the location we already print.
                let message = message
                    .rsplit_once(" at line ")
                    .map_or(message.as_str(), |(text, _)| text);
                (at, message.to_string())
            }),
        }
    }
}

fn frontmatter(source: &str) -> Result<Document<'_>, ParseError> {
    let (first, body) = source.split_once('\n').unwrap_or((source, ""));
    let (format, fence) = match first.trim_end() {
        "+++" => (Format::Toml, "+++"),
        "---" => (Format::Yaml, "---"),
        _ => {
            return Err((
                Some("1:1".into()),
                "expected frontmatter fenced by +++ (TOML) or --- (YAML)".into(),
            ));
        }
    };
    let mut end = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end() == fence {
            return Ok(Document {
                format,
                text: &body[..end],
                first_line: 2,
            });
        }
        end += line.len();
    }
    Err((
        Some("1:1".into()),
        format!("frontmatter is not closed by {fence}"),
    ))
}

/// The text after a frontmatter that `frontmatter` accepted.
fn body(source: &str) -> String {
    let (fence, rest) = source.split_once('\n').unwrap_or((source, ""));
    rest.split_inclusive('\n')
        .skip_while(|line| line.trim_end() != fence.trim_end())
        .skip(1)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(crate) fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    (line, column)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}
