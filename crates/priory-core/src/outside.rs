//! The tool calls of `priory mcp --outside`, from a session Priory did not start. They read the
//! desk, the asks and a task, store an ask, or add a task, and never answer an ask.

use crate::{listing, tools};
use priory_factory::factory::Factory;
use rusqlite::Connection;
use serde_json::{Value, json};

/// The socket's line: `{"outside": {"tool": ..., "session": ..., "arguments": ...}}`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    pub outside: Call,
}

/// One call. `session` is the asking session's hc terminal, then its Claude session, when it has
/// one: the server's environment names it, never the tool's arguments.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub tool: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

/// What a task added by a session with no hc terminal names as the session that added it.
pub const ANONYMOUS: &str = "outside";

/// The `add` tool's arguments.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AddArguments {
    text: String,
    #[serde(default)]
    workflow: Option<String>,
    #[serde(default)]
    band: Option<String>,
    #[serde(default)]
    lane: Option<String>,
    #[serde(default)]
    repo: Option<String>,
}

/// The quick request the `add` tool asks for, naming the session that added it.
pub fn request(call: &Call) -> Result<priory_intake::request::Add, String> {
    let arguments = match &call.arguments {
        Value::Null => json!({}),
        arguments => arguments.clone(),
    };
    let added: AddArguments = serde_json::from_value(arguments).map_err(|error| {
        format!(
            "add takes text, and optionally workflow, band, lane and repo, all strings: {error}"
        )
    })?;
    Ok(priory_intake::request::Add {
        text: added.text,
        workflow: added.workflow,
        band: added.band,
        lane: added.lane,
        repo: added.repo,
        relates: Vec::new(),
        intervention: None,
        session: Some(call.session.clone().unwrap_or_else(|| ANONYMOUS.to_owned())),
    })
}

/// Answers a call of `desk`, `asks`, `task` or `ask`.
pub fn apply(
    db: &mut Connection,
    call: &Call,
    factory: &Factory,
    home: &std::path::Path,
) -> rusqlite::Result<Result<String, String>> {
    let arguments = &call.arguments;
    match call.tool.as_str() {
        "desk" => {
            let now = jiff::Timestamp::now();
            Ok(Ok(
                listing::desk(db, home, false, listing::ME, now)?.to_string()
            ))
        }
        "asks" => {
            let all = match &arguments["all"] {
                Value::Null => false,
                Value::Bool(all) => *all,
                _ => return Ok(Err("all must be true or false".to_owned())),
            };
            let mut listed = listing::asks(db, &[], &[])?;
            if let (false, Some(asks)) = (all, listed["asks"].as_array_mut()) {
                asks.retain(|ask| ask["state"] == "open");
            }
            Ok(Ok(listed.to_string()))
        }
        "task" => {
            let Some(id) = arguments["id"].as_i64() else {
                return Ok(Err("id must be a task's number".to_owned()));
            };
            Ok(match listing::task(db, id)? {
                Some(task) => Ok(task.to_string()),
                None => Err(format!("no task {id}")),
            })
        }
        "ask" => ask(db, call, factory),
        tool => Ok(Err(format!("unknown tool {tool}"))),
    }
}

/// Stores the ask as `priory ask` does, with the session the server named, and says whether its
/// answer will reach the session.
fn ask(
    db: &mut Connection,
    call: &Call,
    factory: &Factory,
) -> rusqlite::Result<Result<String, String>> {
    let Some(fields) = call.arguments.as_object() else {
        return Ok(Err("ask takes an object of fields".to_owned()));
    };
    let mut fields = fields.clone();
    let Some(Value::String(id)) = fields.remove("id") else {
        return Ok(Err("id must be a string, such as cache.ship".to_owned()));
    };
    fields.remove("session");
    if let Some(session) = &call.session {
        fields.insert("session".to_owned(), json!(session));
    }
    let ignored = match tools::ask_outside(db, &id, &Value::Object(fields))? {
        Ok(ignored) => ignored,
        Err(refused) => return Ok(Err(refused)),
    };
    let reach = match (factory.deliver(), &call.session) {
        (Some(_), Some(session)) if session.contains(' ') => {
            "the answer will arrive as a message".to_owned()
        }
        (Some(_), Some(_)) => {
            "the answer will be typed into this terminal while it stays open; once it closes, read the answer with asks".to_owned()
        }
        _ => "no delivery is set up for this session, so read the answer with asks".to_owned(),
    };
    let ignored = ignored
        .iter()
        .map(|field| format!("; ignored unknown field {field}"))
        .collect::<String>();
    Ok(Ok(format!("stored as ask {id}; {reach}{ignored}")))
}
