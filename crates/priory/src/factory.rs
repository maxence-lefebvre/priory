use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
    pub memory: Option<String>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
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
    pub on: String,
    pub view: Option<String>,
    pub filter: Option<BTreeMap<String, String>>,
    pub workflow: String,
    pub key: Option<String>,
    pub band: Option<String>,
    pub lane: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub start: String,
    pub steps: Vec<Step>,
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
    pub cpu: Option<String>,
    pub memory: Option<String>,
}

pub struct Loaded<T> {
    pub file: PathBuf,
    pub value: T,
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
        if let Some(value) = file
            .is_file()
            .then(|| found.parse::<Settings>(&file))
            .flatten()
        {
            found.define(&mut settings, "factory".into(), &file, value);
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
        if let Some(value) = found.parse::<Agent>(&file) {
            found.define(&mut factory.agents, file_name(&path), &file, value);
        }
    }
    for file in found.config_files("triggers") {
        if let Some(value) = found.parse::<Trigger>(&file) {
            found.define(&mut factory.triggers, stem(&file), &file, value);
        }
    }
    for file in found.config_files("workflows") {
        if let Some(value) = found.parse::<Workflow>(&file) {
            found.define(&mut factory.workflows, stem(&file), &file, value);
        }
    }
    for file in found.config_files("runners") {
        if let Some(value) = found.parse::<Runner>(&file) {
            found.define(&mut factory.runners, stem(&file), &file, value);
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
        map.insert(name, Loaded { file, value });
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

    fn parse<T: DeserializeOwned>(&mut self, file: &Path) -> Option<T> {
        let relative = self.relative(file);
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) => {
                self.problems.push(Problem {
                    file: relative,
                    at: None,
                    message: error.to_string(),
                });
                return None;
            }
        };
        let parsed = match file.extension().and_then(|ext| ext.to_str()) {
            Some("md") => frontmatter(&source),
            Some("toml") => Ok(Document {
                format: Format::Toml,
                text: &source,
                first_line: 1,
            }),
            _ => Ok(Document {
                format: Format::Yaml,
                text: &source,
                first_line: 1,
            }),
        }
        .and_then(Document::deserialize);
        match parsed {
            Ok(value) => Some(value),
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

enum Format {
    Toml,
    Yaml,
}

struct Document<'a> {
    format: Format,
    text: &'a str,
    first_line: usize,
}

type ParseError = (Option<String>, String);

impl Document<'_> {
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

fn line_column(text: &str, offset: usize) -> (usize, usize) {
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
