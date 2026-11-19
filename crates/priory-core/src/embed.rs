use priory_factory::factory::Embedder as Spec;
use priory_runtime::runtime;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, PoisonError, TryLockError};
use std::time::Duration;

/// How long one `command` embedding may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// What turns a text into a vector: a person's command, or a model Priory runs in process.
pub enum Embedder {
    Command(String),
    #[cfg(feature = "model")]
    Model(Box<crate::model::Model>),
}

/// The name a vector is stored with, which changes when the factory names another model or command.
pub fn name(spec: &Spec) -> String {
    match spec {
        Spec::Model(model) => format!("model {model}"),
        Spec::Command(command) => format!("command {command}"),
    }
}

/// The embedder a stored `name` stands for. A model's weights come into `home` at first use.
pub fn open(name: &str, home: &Path) -> Result<Embedder, String> {
    if let Some(command) = name.strip_prefix("command ") {
        return Ok(Embedder::Command(command.to_owned()));
    }
    let Some(model) = name.strip_prefix("model ") else {
        return Err(format!("unknown embedder {name:?}"));
    };
    model_at(model, home)
}

#[cfg(feature = "model")]
fn model_at(model: &str, home: &Path) -> Result<Embedder, String> {
    crate::model::Model::load(model, &home.join("models"))
        .map(|model| Embedder::Model(Box::new(model)))
}

#[cfg(not(feature = "model"))]
fn model_at(model: &str, _home: &Path) -> Result<Embedder, String> {
    Err(format!(
        "this priory was built without the `model` feature, so it cannot run {model}: \
         rebuild with `--features model`, or name a `command` in [search]"
    ))
}

impl Embedder {
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        match self {
            Embedder::Command(command) => run(command, text),
            #[cfg(feature = "model")]
            Embedder::Model(model) => model.embed(text),
        }
    }

    /// How many tokens `text` takes and how many the model reads, when the embedder has a tokenizer.
    pub fn tokens(&self, _text: &str) -> Option<(usize, usize)> {
        match self {
            Embedder::Command(_) => None,
            #[cfg(feature = "model")]
            Embedder::Model(model) => model.tokens(_text),
        }
    }
}

/// The embedder of a stored name, opened at first use and again when the name changes.
pub struct Lazy {
    home: std::path::PathBuf,
    open: Option<(String, Embedder)>,
}

impl Lazy {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_owned(),
            open: None,
        }
    }

    fn get(&mut self, name: &str) -> Result<&mut Embedder, String> {
        if self.open.as_ref().is_none_or(|(open, _)| open != name) {
            self.open = Some((name.to_owned(), open(name, &self.home)?));
        }
        Ok(&mut self.open.as_mut().expect("just opened").1)
    }

    pub fn embed(&mut self, name: &str, text: &str) -> Result<Vec<f32>, String> {
        self.get(name)?.embed(text)
    }

    /// The token count and limit of `text` for the embedder `name`, when it has a tokenizer that
    /// opens.
    pub fn tokens(&mut self, name: &str, text: &str) -> Option<(usize, usize)> {
        self.get(name).ok()?.tokens(text)
    }
}

/// One embedder shared by the daemon's threads.
pub type Shared = Mutex<Lazy>;

/// Embeds `text` with the embedder `name`. A command runs outside the lock. A model waits for the
/// lock when `wait`; otherwise it fails while another thread holds it, as when it loads.
pub fn embed_shared(
    shared: &Shared,
    name: &str,
    text: &str,
    wait: bool,
) -> Result<Vec<f32>, String> {
    if let Some(command) = name.strip_prefix("command ") {
        return run(command, text);
    }
    let mut lazy = match shared.try_lock() {
        Ok(lazy) => lazy,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) if wait => {
            shared.lock().unwrap_or_else(PoisonError::into_inner)
        }
        Err(TryLockError::WouldBlock) => {
            return Err("the embedding model is busy loading; try again shortly".to_owned());
        }
    };
    lazy.embed(name, text)
}

/// The token count and limit of `text`, when the model `name` is free and has a tokenizer.
pub fn tokens_shared(shared: &Shared, name: &str, text: &str) -> Option<(usize, usize)> {
    if name.starts_with("command ") {
        return None;
    }
    let mut lazy = match shared.try_lock() {
        Ok(lazy) => lazy,
        Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(TryLockError::WouldBlock) => return None,
    };
    lazy.tokens(name, text)
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
