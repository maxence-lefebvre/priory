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
/// A Codex session file counts each `token_count` event's last usage instead, and takes its model
/// from the latest `turn_context`. A missing file reads as empty; `None` when the file cannot be
/// read.
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
    let mut codex = Codex::default();
    for line in bytes[..whole].split(|&byte| byte == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if codex.read(&entry, &mut model, &mut context) {
            continue;
        }
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
        messages: messages.len() + codex.events,
        model,
        context,
        input: codex.input,
        output: codex.output,
        cache_read: codex.cache_read,
        cache_write: codex.cache_write,
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

/// The tokens of a Codex session file's `token_count` events read so far.
#[derive(Default)]
struct Codex {
    events: usize,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    /// The last cumulative usage counted, so an event that repeats it counts once.
    total: Option<Value>,
}

impl Codex {
    /// Counts `entry` when it is a Codex `token_count` event, or takes its model when it is a
    /// `turn_context`. Codex writes an event either under `payload` or flat. Says whether `entry`
    /// was one of the two.
    fn read(
        &mut self,
        entry: &Value,
        model: &mut Option<String>,
        context: &mut Option<u64>,
    ) -> bool {
        let payload = match &entry["payload"] {
            Value::Object(_) => &entry["payload"],
            _ => entry,
        };
        if entry["type"] == "turn_context" {
            if let Some(name) = payload["model"].as_str() {
                *model = Some(name.to_owned());
            }
            return true;
        }
        if payload["type"] != "token_count" {
            return false;
        }
        let info = &payload["info"];
        let last = &info["last_token_usage"];
        if !last.is_object()
            || (info["total_token_usage"].is_object()
                && self.total.as_ref() == Some(&info["total_token_usage"]))
        {
            return true;
        }
        self.total = Some(info["total_token_usage"].clone());
        let tokens = |name: &str| last[name].as_u64().unwrap_or(0);
        // Codex's input counts its cached input too.
        let cached = tokens("cached_input_tokens").min(tokens("input_tokens"));
        self.events += 1;
        self.input += tokens("input_tokens") - cached;
        self.cache_read += cached;
        self.cache_write += tokens("cache_write_input_tokens");
        self.output += tokens("output_tokens");
        *context = Some(tokens("input_tokens") + tokens("output_tokens"));
        true
    }
}

/// Whether the agent of a Claude Code transcript or Codex session file is inside a tool call: one it
/// made in its latest message holds no result yet. `None` when the file holds no line of either, or
/// its last line is cut short.
pub fn in_tool_call(bytes: &[u8]) -> Option<bool> {
    let mut open: Vec<String> = Vec::new();
    let (mut known, mut message) = (false, None);
    let last = bytes
        .trim_ascii_end()
        .split(|&byte| byte == b'\n')
        .next_back()?;
    serde_json::from_slice::<Value>(last).ok()?;
    for line in bytes.split(|&byte| byte == b'\n') {
        let Ok(entry) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let payload = &entry["payload"];
        if let Some(id) = payload["call_id"].as_str() {
            known = true;
            match payload["type"].as_str() {
                Some("function_call" | "custom_tool_call" | "local_shell_call") => {
                    open.push(id.to_owned());
                }
                Some("function_call_output" | "custom_tool_call_output") => {
                    open.retain(|call| call != id);
                }
                _ => {}
            }
            continue;
        }
        if entry["type"] == "event_msg" {
            known = true;
            if matches!(
                payload["type"].as_str(),
                Some("task_complete" | "turn_aborted")
            ) {
                open.clear();
            }
        }
        let assistant = entry["type"] == "assistant";
        if !assistant && entry["type"] != "user" {
            continue;
        }
        known = true;
        // A later message ends the calls of a session that was killed inside one.
        if assistant && message.as_ref() != entry["message"].get("id") {
            message = entry["message"].get("id").cloned();
            open.clear();
        }
        for block in entry["message"]["content"].as_array().into_iter().flatten() {
            match (block["type"].as_str(), &block["id"], &block["tool_use_id"]) {
                (Some("tool_use"), Value::String(id), _) => open.push(id.clone()),
                (Some("tool_result"), _, Value::String(id)) => open.retain(|call| call != id),
                _ => {}
            }
        }
    }
    known.then_some(!open.is_empty())
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
    fn a_tool_call_is_open_until_its_result_or_a_later_message() {
        let call = r#"{"type":"assistant","message":{"id":"a","content":[{"type":"tool_use","id":"t1"},{"type":"tool_use","id":"t2"}]}}"#;
        let result = |id: &str| {
            format!(
                r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"{id}"}}]}}}}"#
            )
        };
        let later = r#"{"type":"assistant","message":{"id":"b","content":[{"type":"text"}]}}"#;
        let read = |lines: &[&str]| in_tool_call(lines.join("\n").as_bytes());
        assert_eq!(read(&[call]), Some(true));
        assert_eq!(read(&[call, &result("t1")]), Some(true));
        assert_eq!(read(&[call, &result("t1"), &result("t2")]), Some(false));
        assert_eq!(read(&[call, later]), Some(false));
        assert_eq!(read(&["", "not json", "{}"]), None);
        assert_eq!(read(&[call, later, &call[..40]]), None);
    }

    #[test]
    fn a_codex_call_is_open_until_its_output_or_the_end_of_its_turn() {
        let call = r#"{"type":"response_item","payload":{"type":"function_call","call_id":"c"}}"#;
        let output =
            r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c"}}"#;
        let ended = r#"{"type":"event_msg","payload":{"type":"turn_aborted"}}"#;
        let read = |lines: &[&str]| in_tool_call(lines.join("\n").as_bytes());
        assert_eq!(read(&[call]), Some(true));
        assert_eq!(read(&[call, output]), Some(false));
        assert_eq!(read(&[call, ended]), Some(false));
    }

    proptest::proptest! {
        #[test]
        fn any_transcript_reads_without_panicking(
            lines in proptest::collection::vec("[^\n]{0,60}", 0..8),
            kind in "(assistant|user|event_msg)",
            content in "[\\[\\]{}\":,a-z0-9]{0,40}",
        ) {
            let built = format!(r#"{{"type":"{kind}","message":{{"content":{content}}}}}"#);
            let _ = in_tool_call(lines.join("\n").as_bytes());
            let _ = in_tool_call(built.as_bytes());
        }
    }

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

    #[test]
    fn a_codex_session_file_counts_each_token_count_once_with_cached_input_apart() {
        let path = std::env::temp_dir().join(format!("priory-codex-{}.jsonl", std::process::id()));
        let count = |input: u64, cached: u64, output: u64, total: u64| {
            serde_json::json!({"timestamp": "2026-11-22T10:00:00.000Z", "type": "event_msg",
                "payload": {"type": "token_count", "info": {
                    "total_token_usage": {"input_tokens": total, "cached_input_tokens": 0, "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": total},
                    "last_token_usage": {"input_tokens": input, "cached_input_tokens": cached, "output_tokens": output, "reasoning_output_tokens": 0, "total_tokens": input + output},
                    "model_context_window": 200000}}})
            .to_string()
        };
        let lines = [
            serde_json::json!({"type": "turn_context", "payload": {"model": "gpt-fake-5", "cwd": "/w"}}).to_string(),
            count(1000, 800, 50, 1050),
            // A repeated event, as when only the rate limits changed, counts once.
            count(1000, 800, 50, 1050),
            serde_json::json!({"type": "event_msg", "payload": {"type": "token_count", "info": null}}).to_string(),
            count(2000, 1500, 70, 3120),
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();

        let read = usage(&path, 0).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(read.messages, 2);
        assert_eq!(read.input, 200 + 500);
        assert_eq!(read.cache_read, 800 + 1500);
        assert_eq!(read.output, 50 + 70);
        assert_eq!(read.model.as_deref(), Some("gpt-fake-5"));
        assert_eq!(read.context, Some(2070));
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
