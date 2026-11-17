use crate::runtime;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

// shortcut: hand-written JSON-RPC over stdio for initialize, ping, tools/list and tools/call; switch to rmcp if Priory needs more of MCP.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Serves Priory's agent tools over MCP on stdin and stdout, for the run whose harness session is
/// `session`. Each tool call goes to the daemon, which applies it and replies.
pub fn serve(session: &str) -> Result<(), String> {
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = respond(session, &line) else {
            continue;
        };
        let mut out = stdout.lock();
        writeln!(out, "{reply}")
            .and_then(|()| out.flush())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The reply to one JSON-RPC message, or `None` for a notification.
fn respond(session: &str, line: &str) -> Option<Value> {
    let Ok(request) = serde_json::from_str::<Value>(line) else {
        return Some(error(Value::Null, -32700, "parse error"));
    };
    let id = request.get("id")?.clone();
    let params = &request["params"];
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = VERSIONS
                .into_iter()
                .find(|version| *version == asked)
                .unwrap_or(VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "priory", "version": env!("CARGO_PKG_VERSION") },
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(session, params),
        method => return Some(error(id, -32601, &format!("unknown method {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Sends the call to the daemon. A tool error, or no daemon, is a tool result with `isError`.
fn call(session: &str, params: &Value) -> Value {
    let line = json!({
        "tool": params["name"],
        "session": session,
        "arguments": params["arguments"],
    });
    let reply = runtime::call(&runtime::home(), &line.to_string()).and_then(|reply| {
        serde_json::from_str::<Result<String, String>>(&reply)
            .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
    });
    let (text, failed) = match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
}

fn tools() -> Value {
    let text = |description: &str| json!({ "type": "string", "description": description });
    json!([
        {
            "name": "result",
            "description": "Finish the current workflow step. Call it once, then end your turn: Priory moves the task to the step its outcome leads to.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "outcome": text("One of the outcomes the step's prompt names."),
                    "summary": text("What you did and why, in a few sentences."),
                    "evidence": text("How you checked it: commands, exits, links."),
                    "commit": text("The commit the result is about, if any."),
                },
                "required": ["outcome", "summary"],
            },
        },
        {
            "name": "ask",
            "description": "Ask a person a question, then end your turn. Priory puts you to sleep; the answer arrives as your next prompt.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": text("The question, with the context a person needs to answer it."),
                    "options": { "type": "array", "items": { "type": "string" }, "description": "The answers you expect." },
                    "recommendation": text("The option you recommend, and why."),
                    "tradeoff": text("What each option costs."),
                },
                "required": ["question", "options", "recommendation", "tradeoff"],
            },
        },
    ])
}
