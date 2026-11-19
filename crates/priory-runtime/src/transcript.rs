use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// The tokens of the assistant messages in part of a Claude Code session transcript.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub messages: usize,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// The model of the last message.
    pub model: Option<String>,
    /// The byte offset past the last whole line read, where the next read starts.
    pub end: u64,
}

/// Sums the usage of the assistant messages in the transcript at `path` from byte `from`. Claude
/// writes one line per content block, each with its message's usage, so a message id counts once.
/// A missing file reads as empty.
pub fn usage(path: &Path, from: u64) -> Usage {
    let bytes = std::fs::read(path).unwrap_or_default();
    let start = usize::try_from(from).unwrap_or(usize::MAX).min(bytes.len());
    let whole = bytes[start..]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |last| last + 1);
    let mut messages = BTreeMap::new();
    let mut model = None;
    for line in bytes[start..start + whole].split(|&byte| byte == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let message = &entry["message"];
        if entry["type"] != "assistant" || !message["usage"].is_object() {
            continue;
        }
        // Claude marks the messages it writes itself, such as API errors, as `<synthetic>`.
        if let Some(name) = message["model"]
            .as_str()
            .filter(|name| *name != "<synthetic>")
        {
            model = Some(name.to_owned());
        }
        let id = message["id"].as_str().unwrap_or_default().to_owned();
        messages.insert(id, message["usage"].clone());
    }
    let mut usage = Usage {
        end: (start + whole) as u64,
        messages: messages.len(),
        model,
        ..Usage::default()
    };
    for tokens in messages.values() {
        let tokens = |name: &str| tokens[name].as_u64().unwrap_or(0);
        usage.input += tokens("input_tokens");
        usage.output += tokens("output_tokens");
        usage.cache_read += tokens("cache_read_input_tokens");
        usage.cache_write += tokens("cache_creation_input_tokens");
    }
    usage
}
