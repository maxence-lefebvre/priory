use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
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
    /// The context size after the last message Claude's model wrote, in tokens: its input, cached
    /// input and output.
    pub context: Option<u64>,
    /// The byte offset past the last whole line read, where the next read starts.
    pub end: u64,
}

/// Sums the usage of the assistant messages in the transcript at `path` from byte `from`. Claude
/// writes one line per content block, each with its message's usage, so a message id counts once.
/// A missing file reads as empty; `None` when the file cannot be read.
pub fn usage(path: &Path, from: u64) -> Option<Usage> {
    let mut bytes = Vec::new();
    match File::open(path) {
        Ok(mut file) => {
            file.seek(SeekFrom::Start(from)).ok()?;
            file.read_to_end(&mut bytes).ok()?;
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    let whole = bytes
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |last| last + 1);
    let mut messages = BTreeMap::new();
    let mut model = None;
    let mut context = None;
    for line in bytes[..whole].split(|&byte| byte == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let message = &entry["message"];
        if entry["type"] != "assistant" || !message["usage"].is_object() {
            continue;
        }
        let usage = &message["usage"];
        // Claude marks the messages it writes itself, such as API errors, as `<synthetic>`.
        if let Some(name) = message["model"]
            .as_str()
            .filter(|name| *name != "<synthetic>")
        {
            model = Some(name.to_owned());
            context = Some(
                [
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                    "output_tokens",
                ]
                .iter()
                .map(|name| usage[name].as_u64().unwrap_or(0))
                .sum(),
            );
        }
        let id = message["id"].as_str().unwrap_or_default().to_owned();
        messages.insert(id, usage.clone());
    }
    let mut usage = Usage {
        end: from + whole as u64,
        messages: messages.len(),
        model,
        context,
        ..Usage::default()
    };
    for tokens in messages.values() {
        let tokens = |name: &str| tokens[name].as_u64().unwrap_or(0);
        usage.input += tokens("input_tokens");
        usage.output += tokens("output_tokens");
        usage.cache_read += tokens("cache_read_input_tokens");
        usage.cache_write += tokens("cache_creation_input_tokens");
    }
    Some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_context_is_the_last_real_message_s_and_a_synthetic_error_leaves_it() {
        let path =
            std::env::temp_dir().join(format!("priory-transcript-{}.jsonl", std::process::id()));
        let line = |id: &str, model: &str, input: u64| {
            format!(
                r#"{{"type":"assistant","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"cache_read_input_tokens":1000,"cache_creation_input_tokens":50,"output_tokens":10}}}}}}"#
            ) + "\n"
        };
        let transcript = [
            line("a", "claude-x", 100),
            line("b", "claude-x", 200),
            line("c", "<synthetic>", 0),
        ];
        std::fs::write(&path, transcript.concat()).unwrap();

        let read = usage(&path, 0).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(read.context, Some(200 + 1000 + 50 + 10));
        assert_eq!(read.messages, 3);
    }
}
