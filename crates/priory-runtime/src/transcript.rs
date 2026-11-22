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

/// A Bash call in a Claude Code session transcript, with what it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    /// The tool call's id, which its result names.
    pub id: String,
    pub command: String,
    /// What the call printed; `None` when the transcript holds no result for it.
    pub output: Option<Printed>,
}

/// The result of one Bash call, as the transcript holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printed {
    pub text: String,
    /// The call failed, or was interrupted: the text is not the whole of what it printed.
    pub failed: bool,
    /// Claude cut the output down, so the text is not the whole of what it printed.
    pub truncated: bool,
}

/// What Claude writes in a Bash result whose output it cut down.
const TRUNCATED: &[&str] = &["lines truncated] ...", "Output too large"];

/// The Bash calls in a Claude Code session transcript, in the order Claude made them, each with
/// its result when the transcript holds one. Lines that are not JSON, or not of this shape, are
/// skipped: the transcript is outside text.
pub fn shell_calls(bytes: &[u8]) -> Vec<Shell> {
    let mut calls: Vec<Shell> = Vec::new();
    for line in bytes.split(|&byte| byte == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some(content) = entry["message"]["content"].as_array() else {
            continue;
        };
        for block in content {
            match block["type"].as_str() {
                Some("tool_use") if block["name"] == "Bash" => {
                    let (Some(id), Some(command)) =
                        (block["id"].as_str(), block["input"]["command"].as_str())
                    else {
                        continue;
                    };
                    calls.push(Shell {
                        id: id.to_owned(),
                        command: command.to_owned(),
                        output: None,
                    });
                }
                Some("tool_result") => {
                    let Some(call) = calls.iter_mut().find(|call| {
                        call.output.is_none() && block["tool_use_id"] == call.id.as_str()
                    }) else {
                        continue;
                    };
                    let text = match &block["content"] {
                        Value::String(text) => text.clone(),
                        Value::Array(parts) => parts
                            .iter()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => continue,
                    };
                    let failed =
                        block["is_error"] == true || entry["toolUseResult"]["interrupted"] == true;
                    let truncated = TRUNCATED.iter().any(|marker| text.contains(marker));
                    call.output = Some(Printed {
                        text,
                        failed,
                        truncated,
                    });
                }
                _ => {}
            }
        }
    }
    calls
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

    fn bash(id: &str, command: &str) -> String {
        serde_json::json!({"type": "assistant", "message": {"id": format!("msg_{id}"),
            "content": [{"type": "text", "text": "Reading."}, {"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command}}]}})
        .to_string()
    }

    fn printed(id: &str, content: serde_json::Value, error: bool) -> String {
        serde_json::json!({"type": "user", "message": {"role": "user",
            "content": [{"type": "tool_result", "tool_use_id": id, "content": content, "is_error": error}]}})
        .to_string()
    }

    #[test]
    fn shell_calls_pairs_each_bash_call_with_what_it_printed_in_call_order() {
        let transcript = [
            bash("t1", "gh issue view 42"),
            bash("t2", "cargo test"),
            printed(
                "t2",
                serde_json::json!([{"type": "text", "text": "ok"}]),
                false,
            ),
            printed("t1", serde_json::json!("title: A widget"), false),
            bash("t3", "gh pr list"),
            printed(
                "t3",
                serde_json::json!("a\n\n... [120 lines truncated] ...\n\nz"),
                false,
            ),
            bash("t4", "gh api user"),
            printed("t4", serde_json::json!("Exit code 1\nHTTP 502"), true),
            bash("t5", "gh issue list"),
            "{not json".to_owned(),
        ]
        .join("\n");

        let calls = shell_calls(transcript.as_bytes());

        type Seen<'a> = (&'a str, Option<(&'a str, bool, bool)>);
        let seen: Vec<Seen> = calls
            .iter()
            .map(|call| {
                (
                    call.command.as_str(),
                    call.output
                        .as_ref()
                        .map(|out| (out.text.as_str(), out.failed, out.truncated)),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("gh issue view 42", Some(("title: A widget", false, false))),
                ("cargo test", Some(("ok", false, false))),
                (
                    "gh pr list",
                    Some(("a\n\n... [120 lines truncated] ...\n\nz", false, true))
                ),
                ("gh api user", Some(("Exit code 1\nHTTP 502", true, false))),
                ("gh issue list", None),
            ]
        );
    }

    proptest::proptest! {
        #[test]
        fn shell_calls_reads_any_transcript_without_panicking(
            command in "[ -~]{0,40}",
            content in "[ -~]{0,40}",
            cut in 0usize..400,
            junk in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..64),
        ) {
            let mut text = format!("{}\n{}\n", bash("t1", &command), printed("t1", serde_json::json!(content), false)).into_bytes();
            let cut = cut.min(text.len());
            text.truncate(cut);
            text.extend(junk);
            let _ = shell_calls(&text);
            let _ = shell_calls(content.as_bytes());
        }
    }
}
