use priory_factory::factory::Embedder as Spec;
use priory_runtime::runtime;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// How long one `command` embedding may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Turns a text into a vector: a person's command, or a model Priory runs in process.
pub trait Embedder {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>, String>;
}

/// Counts a text's tokens and gives the most the model reads.
pub type Counter = Box<dyn Fn(&str) -> Option<(usize, usize)> + Send>;

/// The name a vector is stored with, which changes when the factory names another model or command.
pub fn name(spec: &Spec) -> String {
    match spec {
        Spec::Model(model) => format!("model {model}"),
        Spec::Command(command) => format!("command {command}"),
    }
}

/// An embedder `command`, run once per text.
struct Script(String);

impl Embedder for Script {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        run(&self.0, text)
    }
}

#[cfg(feature = "model")]
impl Embedder for crate::model::Model {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        crate::model::Model::embed(self, text)
    }
}

/// The embedder a stored `name` stands for. A model's files come into `home` at first use only when
/// `download`, which the daemon alone sets.
fn open(name: &str, home: &Path, download: bool) -> Result<Box<dyn Embedder>, String> {
    if let Some(command) = name.strip_prefix("command ") {
        return Ok(Box::new(Script(command.to_owned())));
    }
    let model = model_of(name)?;
    load(model, home, download)
}

fn model_of(name: &str) -> Result<&str, String> {
    name.strip_prefix("model ")
        .ok_or_else(|| format!("unknown embedder {name:?}"))
}

#[cfg(feature = "model")]
fn load(model: &str, home: &Path, download: bool) -> Result<Box<dyn Embedder>, String> {
    let files = crate::model::Files::new(model, &home.join("models"), download);
    Ok(Box::new(crate::model::Model::load(&files)?))
}

#[cfg(not(feature = "model"))]
fn load(model: &str, _home: &Path, _download: bool) -> Result<Box<dyn Embedder>, String> {
    Err(unbuilt(model))
}

#[cfg(not(feature = "model"))]
fn unbuilt(model: &str) -> String {
    format!(
        "this priory was built without the `model` feature, so it cannot run {model}: \
         rebuild with `--features model`, or name a `command` in [search]"
    )
}

/// The token counter of the embedder `name`: the model's tokenizer, without its weights. A command
/// has none, so its TL;DRs count words.
pub fn counter(name: &str, home: &Path, download: bool) -> Result<Option<Counter>, String> {
    if name.starts_with("command ") {
        return Ok(None);
    }
    counter_of(model_of(name)?, home, download).map(Some)
}

#[cfg(feature = "model")]
fn counter_of(model: &str, home: &Path, download: bool) -> Result<Counter, String> {
    let files = crate::model::Files::new(model, &home.join("models"), download);
    let counter = crate::model::Counter::load(&files)?;
    Ok(Box::new(move |text| {
        Some((counter.count(text)?, counter.limit))
    }))
}

#[cfg(not(feature = "model"))]
fn counter_of(model: &str, _home: &Path, _download: bool) -> Result<Counter, String> {
    Err(unbuilt(model))
}

/// The embedder of a stored name, opened at first use and again when the name changes.
pub struct Lazy {
    home: PathBuf,
    download: bool,
    open: Option<(String, Box<dyn Embedder>)>,
}

impl Lazy {
    pub fn new(home: &Path, download: bool) -> Self {
        Self {
            home: home.to_owned(),
            download,
            open: None,
        }
    }

    pub fn embed(&mut self, name: &str, text: &str) -> Result<Vec<f32>, String> {
        if self.open.as_ref().is_none_or(|(open, _)| open != name) {
            self.open = Some((name.to_owned(), open(name, &self.home, self.download)?));
        }
        self.open.as_mut().expect("just opened").1.embed(text)
    }
}

/// Runs the embedder `command` with `text` on its stdin and reads the JSON array it prints.
fn run(command: &str, text: &str) -> Result<Vec<f32>, String> {
    let input = Some(text.as_bytes().to_vec());
    let output = runtime::fed_within(Command::new("sh").args(["-c", command]), input, TIMEOUT)
        .map_err(|error| format!("`{command}`: {error}"))?
        .ok_or_else(|| format!("`{command}` timed out after {}s", TIMEOUT.as_secs()))?;
    if !output.status.success() {
        return Err(format!(
            "`{command}` failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let vector: Vec<f32> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("`{command}` printed no JSON array of numbers: {error}"))?;
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(format!("`{command}` printed an empty or non-finite vector"));
    }
    Ok(vector)
}
