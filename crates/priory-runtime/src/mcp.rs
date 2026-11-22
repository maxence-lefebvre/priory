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
pub fn serve(session: &str, local: &Local, debate: bool) -> Result<(), String> {
    let mut tools = tools();
    if debate && let Value::Array(listed) = &mut tools {
        listed.push(position());
    }
    serve_with(&tools, &|params| call(session, params, local))
}

/// The tool that ends a debate turn, served only to a debate turn's run.
fn position() -> Value {
    json!({
        "name": "position",
        "description": "End this debate turn with your position: a verdict, which must be one of the step's outcomes, and your argument. Priory ends the debate when both agents name the same verdict, or asks a person.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "verdict": { "type": "string", "description": "One of the step's outcomes." },
                "argument": { "type": "string", "description": "Why, in at most 4000 characters." },
                "concedes": { "type": "string", "description": "What the other side got right, if anything." },
            },
            "required": ["verdict", "argument"],
        },
    })
}

/// Serves `tools` over MCP on stdin and stdout, answering each `tools/call` with `call`.
pub(crate) fn serve_with(tools: &Value, call: &dyn Fn(&Value) -> Value) -> Result<(), String> {
    let stdout = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = respond(&line, tools, call) else {
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
fn respond(line: &str, tools: &Value, call: &dyn Fn(&Value) -> Value) -> Option<Value> {
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
        "tools/list" => json!({ "tools": tools }),
        "tools/call" => call(params),
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
        runtime::call(&runtime::home(), &line.to_string(), runtime::REPLY).and_then(|reply| {
            serde_json::from_str::<Result<String, String>>(&reply)
                .map_err(|error| format!("unreadable reply {reply:?}: {error}"))?
        })
    });
    content(reply)
}

/// A tool result that carries `reply`'s text, with `isError` for an error.
pub(crate) fn content(reply: Result<String, String>) -> Value {
    let (text, failed) = match reply {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": failed })
}

pub(crate) fn tools() -> Value {
    let text = |description: &str| json!({ "type": "string", "description": description });
    let condition = json!({
        "type": "object",
        "description": "A condition: {type: pull_request|checks|review, repo, pull_request} for a pull request that closes, whose checks all complete, or that gets a review; {type: time, until} for an RFC 3339 instant or a duration such as 30 seconds, 5 minutes or 2 hours; {type: task, task} for another task that ends; {type: linear_comment, issue} for a comment on a Linear issue; {type: linear_state, issue, state} for a Linear issue that moves to a state; {type: slack_reply, channel, thread_ts} for a reply in a Slack thread; {type: disk, gb} for the machine to have gb GB of disk free.",
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
    });
    let batch = json!({
        "type": "object",
        "description": "For a batch's plan step: the plan. Every issue of the batch is in one entry or one close. entries run in this order; each entry starts after the entries in its after merge. Leave out an issue another task already holds.",
        "properties": {
            "title": text("The batch's title."),
            "entries": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": text("A short id, unique in the plan, such as e1."),
                        "issue": text("The issue, such as ENG-40 or owner/name#42."),
                        "title": text("The task's title."),
                        "brief": text("The task's request text, in Markdown."),
                        "files": { "type": "array", "items": { "type": "string" }, "description": "The files or folders you expect it to change." },
                        "after": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "entry": text("The id of the entry it starts after."),
                                    "moment": { "enum": ["merged"], "description": "merged: once every pull request of that entry merged." },
                                    "why": text("Why it waits, in one line."),
                                },
                                "required": ["entry", "moment"],
                            },
                        },
                    },
                    "required": ["id", "issue", "title", "brief"],
                },
            },
            "close": {
                "type": "array",
                "description": "Issues already done, which the close step closes.",
                "items": {
                    "type": "object",
                    "properties": {
                        "issue": text("The issue."),
                        "why": text("Why it is done, in one line."),
                        "evidence": text("A link that proves it, such as the merged pull request."),
                    },
                    "required": ["issue", "why"],
                },
            },
        },
        "required": ["entries"],
    });
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
                    "goal": text("For a plan step: what the change must achieve. Shown to the person approving the plan."),
                    "testing": text("For a plan step: the testing method, the person's or the repository's. Shown to the person approving the plan."),
                    "brief": text("For a plan step: what to change and why. Shown to the person approving the plan."),
                    "verification": text("For a plan step: how the change will be proved, the commands or checks. The next step's prompt names it."),
                    "reason": text("For a red, yellow or green outcome: why, in one line. Defaults to the summary's first line."),
                    "risk": {
                        "type": "object",
                        "description": "For a low, medium or high outcome, the merge risk verdict: one line per dimension, each starting with its level, such as \"low: one revert undoes it\". The outcome must be the highest level.",
                        "properties": {
                            "ask": text("Does the change do what was asked, and nothing else?"),
                            "blast_radius": text("How much breaks if the change is wrong?"),
                            "verification": text("How well do the tests and checks prove it works?"),
                            "rollback": text("How costly is it to undo once merged?"),
                            "detection": text("Would a failure be noticed soon?"),
                        },
                        "required": ["ask", "blast_radius", "verification", "rollback", "detection"],
                    },
                    "feedback": {
                        "type": "object",
                        "description": "When the prompt carried review feedback: each item's URL mapped to its class. trivial: one obvious answer inside this change's scope, fix it and reply. nothing-to-do: an approval or thanks. persons-call: a behaviour, design or scope change, a disagreement or anything unclear, ask the person.",
                        "additionalProperties": { "enum": ["trivial", "nothing-to-do", "persons-call"] },
                    },
                    "branches": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "For a stack of pull requests: its branches, bottom first, ending with your task's branch. Name each lower one after it: priory/7-api. open-pr opens one pull request per branch, each based on the one below.",
                    },
                    "batch": batch,
                },
                "required": ["outcome", "summary"],
            },
        },
        {
            "name": "ask",
            "description": "Ask a person a question, then end your turn. Priory puts you to sleep; the answer arrives as your next prompt. You may also call wait in the same turn: the first answer or event wins and withdraws the other.",
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
                    "command": {
                        "type": "object",
                        "description": "A command the person's answer runs once, after they confirm it. Priory execs argv without a shell.",
                        "properties": {
                            "argv": { "type": "array", "items": { "type": "string" }, "minItems": 1, "description": "The program and its arguments." },
                            "target": text("Where it runs: worktree, the default, or a name in the factory's [asks.targets]."),
                            "runs": { "type": "array", "items": { "type": "string" }, "description": "The options whose answer runs it. Default: the recommended option." },
                        },
                        "required": ["argv"],
                        "additionalProperties": false,
                    },
                },
                "required": ["question", "options", "recommendation", "tradeoff"],
            },
        },
        {
            "name": "wait",
            "description": "Sleep until a condition holds, then end your turn. Give one condition in on, or a list in any to wake on the first that holds. Priory stops this process and resumes the session with the event that satisfies the condition as your next prompt. A Linear state or pull request that already holds wakes you too. You may also call ask in the same turn: the first answer or event wins and withdraws the other. Waiting on a pull request makes its events yours.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "on": condition,
                    "any": {
                        "type": "array",
                        "items": condition,
                        "minItems": 1,
                        "description": "Instead of on: a list of conditions. You wake on the first that holds, and the others are withdrawn.",
                    },
                },
            },
        },
        {
            "name": "subscribe",
            "description": "Own a pull request, Linear issue or Slack thread until it closes, your task retires, or you unsubscribe. Its events reach you: as a prompt while you run, a wake while you sleep, and a reopen once your task is done. Takes the same condition object as wait. Refused while another active task owns it. It does not end your turn.",
            "inputSchema": { "type": "object", "properties": { "on": condition.clone() }, "required": ["on"] },
        },
        {
            "name": "unsubscribe",
            "description": "End a subscription made with subscribe, so its events stop reaching you. It does not end your turn.",
            "inputSchema": { "type": "object", "properties": { "on": condition.clone() }, "required": ["on"] },
        },
        {
            "name": "relate",
            "description": "Record how your task relates to another: it builds-on it, fixes what it merged, takes-up it after it was dropped or failed, or only relates to it. Find the other task with search. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["builds-on", "fixes", "takes-up", "relates"], "description": "How your task relates to the other." },
                    "task": { "type": "integer", "description": "The other task's id." },
                },
                "required": ["kind", "task"],
                "additionalProperties": false,
            },
        },
        {
            "name": "finding",
            "description": "Hand one problem you found to a fix task. The same key in a later run opens no second task: use one stable key per problem, such as disk-full. A key you skipped for a run and report again opens a new task. A finding reported several runs in a row moves its fix task up a band. Your next prompt lists your earlier findings. Call it once per problem. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": text("A stable key for this problem, the same every run."),
                    "title": text("The problem in one line; the fix task's title."),
                    "details": text("Evidence and links the fix task needs."),
                },
                "required": ["key", "title"],
                "additionalProperties": false,
            },
        },
        {
            "name": "remember",
            "description": "Save one fact for later runs, as a memory: a lesson about this factory's code or process that a later task would need. Your task's later runs see it at once; other tasks' runs see it once a person accepts it. Saving a name again writes a new version, which a person accepts in turn. At most 4096 bytes, and never a secret. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": text("The memory's name: lowercase letters, digits and hyphens, such as release-day."),
                    "description": text("When an agent should read it, in one line. The memory index shows it."),
                    "text": text("The fact itself, in Markdown."),
                },
                "required": ["name", "description", "text"],
                "additionalProperties": false,
            },
        },
        {
            "name": "forget",
            "description": "Remove a memory that is wrong or out of date, by its name. Your task's later runs stop seeing it at once; other tasks' runs once a person accepts the removal. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": text("The memory's name, as the memory index lists it, without .md.") },
                "required": ["name"],
                "additionalProperties": false,
            },
        },
        {
            "name": "memory",
            "description": "Read a memory's file by its name, as your task sees it now. Your instructions' memory index lists each name and when to read it. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": text("The memory's name, as the memory index lists it, without .md.") },
                "required": ["name"],
                "additionalProperties": false,
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
        {
            "name": "progress",
            "description": "Publish where your task stands, for a person who reads the desk: its goal, acceptance criteria, latest change and next action. A field you leave out keeps its value. It does not end your turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "goal": text("What the task must achieve, in one or two sentences."),
                    "acceptanceCriteria": {
                        "type": "array",
                        "description": "Every criterion the task must meet. It replaces the stored list.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": text("A short id, such as AC1."),
                                "description": text("What meets the criterion."),
                                "required": { "type": "boolean", "description": "Whether the task cannot finish without it." },
                            },
                            "required": ["id", "description", "required"],
                        },
                    },
                    "latestChange": text("What changed last, in one sentence."),
                    "nextAction": text("What you do next, in one sentence."),
                },
            },
        },
    ])
}
