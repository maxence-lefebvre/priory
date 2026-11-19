use candle_core::{D, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokenizers::{Tokenizer, TruncationParams};

/// A sentence-transformers BERT model, such as all-MiniLM-L6-v2, run on the CPU with candle.
pub struct Model {
    bert: BertModel,
    /// Cuts a text at the model's input, for embedding.
    tokenizer: Tokenizer,
    /// Counts a whole text, for the TL;DR limit.
    counter: Tokenizer,
    limit: usize,
}

type Error = Box<dyn std::error::Error + Send + Sync>;

impl Model {
    /// Loads the Hugging Face model `repo` from `dir`, downloading its files there at first use.
    pub fn load(repo: &str, dir: &Path) -> Result<Self, String> {
        Self::open(repo, dir).map_err(|error| format!("model {repo}: {error}"))
    }

    fn open(repo: &str, dir: &Path) -> Result<Self, Error> {
        let dir = dir.join(repo.replace('/', "--"));
        let config: Config = serde_json::from_slice(&fs::read(fetch(repo, &dir, "config.json")?)?)?;
        // sentence-transformers names the input it was trained on, shorter than BERT's positions.
        let limit = fetch(repo, &dir, "sentence_bert_config.json")
            .ok()
            .and_then(|file| fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|config| config["max_seq_length"].as_u64())
            .map_or(config.max_position_embeddings, |limit| limit as usize);
        let mut counter = Tokenizer::from_file(fetch(repo, &dir, "tokenizer.json")?)?;
        // MiniLM's tokenizer pads and truncates to 128 tokens: padding would be pooled, and a cut
        // text never counts over the limit.
        counter.with_padding(None);
        counter.with_truncation(None)?;
        let mut tokenizer = counter.clone();
        tokenizer.with_truncation(Some(TruncationParams {
            max_length: limit,
            ..TruncationParams::default()
        }))?;
        let weights = fs::read(fetch(repo, &dir, "model.safetensors")?)?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DTYPE, &Device::Cpu)?;
        let bert = BertModel::load(vb, &config)?;
        Ok(Self {
            bert,
            tokenizer,
            counter,
            limit,
        })
    }

    /// The mean of the token vectors of `text`, normalized, as sentence-transformers pools them.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        self.forward(text).map_err(|error| error.to_string())
    }

    fn forward(&self, text: &str) -> Result<Vec<f32>, Error> {
        let encoding = self.tokenizer.encode(text, true)?;
        let ids = Tensor::new(encoding.get_ids(), &Device::Cpu)?.unsqueeze(0)?;
        let types = ids.zeros_like()?;
        let mask = Tensor::new(encoding.get_attention_mask(), &Device::Cpu)?.unsqueeze(0)?;
        let tokens = self.bert.forward(&ids, &types, Some(&mask))?;
        let pooled = (tokens.sum(1)? / encoding.get_ids().len() as f64)?;
        let normalized = pooled.broadcast_div(&pooled.sqr()?.sum_keepdim(D::Minus1)?.sqrt()?)?;
        Ok(normalized.squeeze(0)?.to_vec1()?)
    }

    /// How many tokens `text` takes, with the model's special tokens, and how many it reads.
    pub fn tokens(&self, text: &str) -> Option<(usize, usize)> {
        let encoding = self.counter.encode(text, true).ok()?;
        Some((encoding.get_ids().len(), self.limit))
    }
}

/// The path of `file` of `repo` in `dir`, downloaded from Hugging Face with `curl` when missing.
fn fetch(repo: &str, dir: &Path, file: &str) -> Result<PathBuf, Error> {
    let path = dir.join(file);
    if path.exists() {
        return Ok(path);
    }
    fs::create_dir_all(dir)?;
    let partial = dir.join(format!("{file}.partial"));
    let url = format!("https://huggingface.co/{repo}/resolve/main/{file}");
    let output = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location", "--retry", "2"])
        .arg("--output")
        .arg(&partial)
        .arg(&url)
        .output()
        .map_err(|error| format!("curl: {error}"))?;
    if !output.status.success() {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "cannot download {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    fs::rename(&partial, &path)?;
    Ok(path)
}
