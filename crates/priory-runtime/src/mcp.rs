use crate::runtime;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

// shortcut: hand-written JSON-RPC over stdio for initialize, ping, tools/list and tools/call; switch to rmcp if Priory needs more of MCP.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// What every agent learns on connecting: how to run a heavy command behind the memory gate.
fn instructions() -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| "priory".into());
    format!(
        "Wrap each heavy command, such as a build or a test suite, in `{} memory acquire --gb N -- <command>`, with N its peak memory in GB, so it waits for room instead of running the machine out of memory.",
        exe.display()
    )
}

/// Serves Priory's agent tools over MCP on stdin and stdout, for the run whose harness session is
/// `session`. `local` answers the tools this process serves itself; every other call goes to the
/// daemon, which applies it and replies.
pub fn serve(session: &str, local: &Local) -> Result<(), String> {
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = respond(session, &line, local) else {
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
fn respond(session: &str, line: &str, local: &Local) -> Option<Value> {
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
                "instructions": instructions(),
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call(session, params, local),
        method => return Some(error(id, -32601, &format!("unknown method {method}"))),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool this process answers without the daemon, by its name and arguments; `None` for another.
pub type Local<'a> = dyn Fn(&str, &Value) -> Option<Result<String, String>> + 'a;

/// Answers the call here or sends it to the daemon. A tool error, or no daemon, is a tool result
/// with `isError`.
fn call(session: &str, params: &Value, local: &Local) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let reply = local(name, &params["arguments"]).unwrap_or_else(|| {
        let line = json!({
            "tool": params["name"],
            "session": session,
            "arguments": params["arguments"],
        });
        runtime::call(&runtime::home(), &line.to_string()).and_then(|reply| {
            serde_json::from_str::<Result<String, String>>(&reply)
                .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
        })
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
                    "commit": text("The full SHA of the commit the result is about, if any, as git rev-parse HEAD prints it."),
                    "reason": text("For a red, yellow or green outcome: why, in one line. Defaults to the summary's first line."),
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
                    "title": text("The question in one line, for the desk."),
                    "text": text("The ask's body in Markdown, one idea per paragraph."),
                    "whatWaits": text("What waits on the answer."),
                    "nextAction": text("What you do once you have the answer."),
                    "command": text("A command for the answer to run. Priory stores it but does not run it yet."),
                },
                "required": ["question", "options", "recommendation", "tradeoff"],
            },
        },
        {
            "name": "wait",
            "description": "Sleep until one condition holds, then end your turn. Priory stops this process and resumes the session with the event that satisfies the condition as your next prompt. Waiting on a pull request makes its events yours.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "on": {
                        "type": "object",
                        "description": "The condition: {type: pull_request|checks|review, repo, pull_request} for a pull request that closes, whose checks all complete, or that gets a review; {type: time, until} for an RFC 3339 instant or a duration such as 30 seconds, 5 minutes or 2 hours; {type: task, task} for another task that ends; {type: linear_comment, issue} for a comment on a Linear issue; {type: linear_state, issue, state} for a Linear issue that moves to a state; {type: slack_reply, channel, thread_ts} for a reply in a Slack thread; {type: disk, gb} for the machine to have gb GB of disk free.",
                        "properties": {
                            "type": { "type": "string", "enum": ["pull_request", "checks", "review", "time", "task", "linear_comment", "linear_state", "slack_reply", "disk"] },
                            "repo": text("owner/name, for a pull request condition."),
                            "pull_request": { "type": "integer", "description": "The pull request number." },
                            "head_sha": text("For checks: the commit whose checks you wait for. Without it, any head of the pull request."),
                            "until": text("For time: an RFC 3339 instant, or a duration such as 30 seconds."),
                            "task": { "type": "integer", "description": "For task: the id of the task to wait for." },
                            "issue": text("For linear_comment and linear_state: the issue identifier, such as ENG-7."),
                            "state": text("For linear_state: the state to wait for, such as Done."),
                            "channel": text("For slack_reply: the channel id, such as C0123."),
                            "thread_ts": text("For slack_reply: the ts of the thread's first message."),
                            "gb": { "type": "number", "description": "For disk: the free disk to wait for, in GB." },
                        },
                        "required": ["type"],
                    },
                },
                "required": ["on"],
            },
        },
        {
            "name": "compact",
            "description": "Compact this conversation when its context runs long, then end your turn. Priory types /compact once you are idle, then a prompt that names your task and step so you carry on.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "focus": text("What the summary must keep, such as the current plan and open failures. Codex ignores it."),
                },
            },
        },
        {
            "name": "tldr",
            "description": "Replace your task's TL;DR, the short summary search reads, when your work drifted from the request. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": text("What the task is about now, in a few sentences of at most 180 words."),
                },
                "required": ["text"],
            },
        },
        {
            "name": "report",
            "description": "Write your task's report for a person: ordered sections of rows, some with a pick. It replaces your task's report and keeps the choices already made. A pick arrives as your next prompt; apply it, then write the report again with its applied filled. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "date": text("The day the report is for, such as 2026-11-20."),
                    "since": text("The RFC 3339 instant the report covers changes from."),
                    "greeting": text("The line the report opens with."),
                    "sections": {
                        "type": "array",
                        "description": "In order. Each: {id, title, count?, rows}. A row: {text, id?, key?, meta?, state?, owner?, time?, pick?, comments: []}. A pick: {id, options, recommended?, applied?, requests?}, where requests maps an option to a task request {workflow, key, text, band?, lane?, repo?, tldr?, hold?} that Priory starts instead of waking you.",
                        "items": { "type": "object" },
                    },
                },
                "required": ["date", "since", "greeting", "sections"],
            },
        },
        {
            "name": "search",
            "description": "Find tasks by what they are about, such as other tasks that touch the code you change, or one that already did your work. Prints task, key, state, step and TL;DR, best match first. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": text("What to look for, in words."),
                    "states": { "type": "array", "items": { "type": "string" }, "description": "Only tasks in these states, such as running or done." },
                    "limit": { "type": "integer", "description": "The most tasks to return; 10 when unset." },
                },
                "required": ["query"],
            },
        },
    ])
}
