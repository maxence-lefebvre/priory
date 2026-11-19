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
    tokenizer: Tokenizer,
}

/// The model's tokenizer alone, which counts a TL;DR against the input the model reads.
pub struct Counter {
    tokenizer: Tokenizer,
    pub limit: usize,
}

type Error = Box<dyn std::error::Error + Send + Sync>;

/// Where the files of the Hugging Face model `repo` live, and whether a missing one is downloaded.
pub struct Files<'a> {
    pub repo: &'a str,
    pub dir: PathBuf,
    pub download: bool,
}

impl<'a> Files<'a> {
    pub fn new(repo: &'a str, models: &Path, download: bool) -> Self {
        Self {
            repo,
            dir: models.join(repo.replace('/', "--")),
            download,
        }
    }

    /// The path of `file`, downloaded from Hugging Face with `curl` when missing and allowed.
    fn get(&self, file: &str) -> Result<PathBuf, Error> {
        let path = self.dir.join(file);
        if path.exists() {
            return Ok(path);
        }
        if !self.download {
            return Err(format!("{file} is not downloaded yet: the daemon fetches it").into());
        }
        fs::create_dir_all(&self.dir)?;
        // Each download has its own file, so two processes never publish a mix of both.
        let partial = self
            .dir
            .join(format!("{file}.{}.partial", std::process::id()));
        let url = format!("https://huggingface.co/{}/resolve/main/{file}", self.repo);
        let output = Command::new("curl")
            .args(["--fail", "--silent", "--show-error", "--location"])
            .args([
                "--retry",
                "2",
                "--connect-timeout",
                "30",
                "--max-time",
                "1800",
            ])
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
}

impl Counter {
    pub fn load(files: &Files) -> Result<Self, String> {
        Self::open(files).map_err(|error| format!("model {}: {error}", files.repo))
    }

    fn open(files: &Files) -> Result<Self, Error> {
        let config: Config = serde_json::from_slice(&fs::read(files.get("config.json")?)?)?;
        // sentence-transformers names the input it was trained on, shorter than BERT's positions.
        let limit = files
            .get("sentence_bert_config.json")
            .ok()
            .and_then(|file| fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|config| config["max_seq_length"].as_u64())
            .map_or(config.max_position_embeddings, |limit| limit as usize);
        let mut tokenizer = Tokenizer::from_file(files.get("tokenizer.json")?)?;
        // MiniLM's tokenizer pads and truncates to 128 tokens: padding would be pooled, and a cut
        // text never counts over the limit.
        tokenizer.with_padding(None);
        tokenizer.with_truncation(None)?;
        Ok(Self { tokenizer, limit })
    }

    /// How many tokens `text` takes, with the model's special tokens.
    pub fn count(&self, text: &str) -> Option<usize> {
        Some(self.tokenizer.encode(text, true).ok()?.get_ids().len())
    }
}

impl Model {
    pub fn load(files: &Files) -> Result<Self, String> {
        Self::open(files).map_err(|error| format!("model {}: {error}", files.repo))
    }

    fn open(files: &Files) -> Result<Self, Error> {
        let config: Config = serde_json::from_slice(&fs::read(files.get("config.json")?)?)?;
        let counter = Counter::open(files)?;
        let mut tokenizer = counter.tokenizer;
        tokenizer.with_truncation(Some(TruncationParams {
            max_length: counter.limit,
            ..TruncationParams::default()
        }))?;
        let weights = fs::read(files.get("model.safetensors")?)?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DTYPE, &Device::Cpu)?;
        let bert = BertModel::load(vb, &config)?;
        Ok(Self { bert, tokenizer })
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
}
