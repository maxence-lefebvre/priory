use super::Ended;
use super::slack::{API, TOKEN, post, send, text};
use crate::request::{self, Add, Refused};
use priory_factory::factory::{Factory, SlackShortcut};
use serde_json::{Value, json};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::Duration;
use tungstenite::Message;

/// The `callback_id` of the shortcut in the app manifest, and of the form it opens.
const CALLBACK: &str = "priory_queue";
/// How many payloads wait for the worker; more get a "busy" reply.
const QUEUE: usize = 16;
const BUSY: &str = "Priory is busy with other requests; try again in a minute.";
const REFUSED: &str =
    "You are not on this Priory's `[slack.shortcut] users`, so nothing was queued.";
const NONE: &str =
    "Every workflow of this Priory sets `requestable = false`, so there is nothing to queue.";
const LONGEST_WAIT: Duration = Duration::from_secs(60);
/// How long the socket may stay silent before Priory pings it.
const QUIET: Duration = Duration::from_secs(30);

/// Starts the shortcut: a thread that holds one Socket Mode connection and acknowledges every
/// envelope before anything else, a worker that handles their payloads, and one that sends the
/// "busy" replies when the worker's queue is full. `ended` counts the tasks with a key that ended,
/// so a thread whose task ended takes a new one. Errs when a token is missing.
pub(super) fn start(dir: &Path, shortcut: &SlackShortcut, ended: Ended) -> Result<(), String> {
    let env = |name: &str, what: &str| {
        std::env::var(name)
            .ok()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| format!("{name} is not set: the Slack shortcut needs {what}"))
    };
    let app = (
        shortcut.token_env.clone(),
        env(&shortcut.token_env, "an app-level token")?,
    );
    let bot = env(TOKEN, "a bot token to open its form")?;
    // The threads outlive this call's borrow of the factory, so they own a fresh load of it.
    let (factory, _) = priory_factory::factory::load(dir);
    let users = shortcut.users.clone();
    let (work, payloads) = sync_channel(QUEUE);
    let (busy, overflow) = sync_channel(QUEUE);
    std::thread::spawn(move || {
        for payload in payloads {
            if let Err(error) = handle(&factory, &users, &bot, &payload, ended) {
                eprintln!("Slack shortcut: {error}");
            }
        }
    });
    std::thread::spawn(move || {
        for payload in overflow {
            if let Err(error) = reply(&payload, BUSY) {
                eprintln!("Slack shortcut: {error}");
            }
        }
    });
    std::thread::spawn(move || listen(&app, &work, &busy));
    Ok(())
}

/// Holds one connection for good: reconnects at once after Slack's `disconnect`, and with a backoff
/// of 1 s doubling to a minute after an error, back to 1 s once Slack says `hello`. A rate limit
/// waits Slack's `Retry-After` instead.
fn listen(app: &(String, String), work: &SyncSender<Value>, busy: &SyncSender<Value>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if let Err(error) = connection(app, work, busy, &mut backoff) {
            let wait = backoff.as_secs();
            eprintln!("Slack shortcut: {error}; retrying in {wait}s");
            std::thread::sleep(backoff);
            backoff = (backoff * 2).min(LONGEST_WAIT);
        }
    }
}

/// One Socket Mode connection, until Slack asks for a new one (`Ok`) or it fails.
fn connection(
    (name, token): &(String, String),
    work: &SyncSender<Value>,
    busy: &SyncSender<Value>,
    backoff: &mut Duration,
) -> Result<(), String> {
    let open = format!("{API}apps.connections.open");
    let opened = send(Some((name, token)), &open, &json!({})).map_err(|(error, retry)| {
        // A rate limit says when to come back; Slack's own value caps at an hour.
        if let Some(seconds) = retry {
            *backoff = Duration::from_secs(seconds.clamp(1, 3600));
        }
        error
    })?;
    let url = text(&opened["url"]);
    let failed = |error: &dyn std::fmt::Display| format!("Socket Mode connect: {error}");
    let uri: tungstenite::http::Uri = url.parse().map_err(|error| failed(&error))?;
    let port = match uri.scheme_str() {
        Some("ws") => 80,
        _ => 443,
    };
    let address = (
        uri.host().unwrap_or_default(),
        uri.port_u16().unwrap_or(port),
    )
        .to_socket_addrs()
        .map_err(|error| failed(&error))?
        .next()
        .ok_or_else(|| failed(&"Slack's URL names no address"))?;
    // Timeouts before the handshake, so neither it nor a blocked write can hold the thread.
    let stream = TcpStream::connect_timeout(&address, QUIET).map_err(|error| failed(&error))?;
    stream
        .set_read_timeout(Some(QUIET))
        .and_then(|()| stream.set_write_timeout(Some(QUIET)))
        .map_err(|error| failed(&error))?;
    let (mut socket, _) = tungstenite::client_tls(url, stream).map_err(|error| failed(&error))?;
    let mut pinged = false;
    loop {
        let frame = match socket.read() {
            Ok(Message::Text(frame)) => frame,
            Ok(Message::Close(_)) => return Err("Slack closed the socket".to_owned()),
            Ok(_) => {
                pinged = false;
                continue;
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if pinged {
                    return Err("the socket went silent".to_owned());
                }
                pinged = true;
                socket
                    .send(Message::Ping(Default::default()))
                    .map_err(|error| format!("Socket Mode: {error}"))?;
                continue;
            }
            Err(error) => return Err(format!("Socket Mode: {error}")),
        };
        pinged = false;
        let frame: Value = serde_json::from_str(&frame).unwrap_or_default();
        match text(&frame["type"]) {
            "hello" => *backoff = Duration::from_secs(1),
            "disconnect" => {
                let _ = socket.close(None);
                return Ok(());
            }
            _ => {}
        }
        let Some(envelope) = frame["envelope_id"].as_str() else {
            continue;
        };
        socket
            .send(Message::text(
                json!({ "envelope_id": envelope }).to_string(),
            ))
            .map_err(|error| format!("Socket Mode ack: {error}"))?;
        if let Err(TrySendError::Full(payload)) = work.try_send(frame["payload"].clone()) {
            // A full overflow queue too drops the payload unanswered.
            let _ = busy.try_send(payload);
        }
    }
}

/// Opens the form for a shortcut, or queues the task a submitted form asks for. Refuses anyone
/// off the allowlist.
fn handle(
    factory: &Factory,
    users: &[String],
    bot: &str,
    payload: &Value,
    ended: Ended,
) -> Result<(), String> {
    let allowed = users
        .iter()
        .any(|user| user == text(&payload["user"]["id"]));
    match text(&payload["type"]) {
        "message_action" if payload["callback_id"] == CALLBACK => {
            if !allowed {
                return reply(payload, REFUSED);
            }
            // Slack refuses a form whose select has no option.
            if factory.requestable().is_empty() {
                return reply(payload, NONE);
            }
            let opened =
                json!({ "trigger_id": payload["trigger_id"], "view": form(factory, payload) });
            post(Some((TOKEN, bot)), &format!("{API}views.open"), &opened).map(drop)
        }
        "view_submission" if payload["view"]["callback_id"] == CALLBACK => {
            if !allowed {
                return reply(payload, REFUSED);
            }
            reply(payload, &submit(factory, payload, ended))
        }
        _ => Ok(()),
    }
}

/// The form: the requestable workflows, preselected from `[request]`; the repo, empty for `triggers/cli.toml`'s;
/// and the note, prefilled with the message. Its metadata carries the thread and the reply URL.
fn form(factory: &Factory, payload: &Value) -> Value {
    let channel = text(&payload["channel"]["id"]);
    let message = &payload["message"];
    let ts = text(&message["ts"]);
    let thread = message["thread_ts"].as_str().unwrap_or(ts);
    let domain = text(&payload["team"]["domain"]);
    let link = (!domain.is_empty()).then(|| {
        format!(
            "https://{domain}.slack.com/archives/{channel}/p{}",
            ts.replace('.', "")
        )
    });
    let metadata = json!({
        "channel": channel,
        "thread": thread,
        "response_url": payload["response_url"],
        "link": link,
    });
    let option = |workflow: &str| json!({ "text": { "type": "plain_text", "text": workflow }, "value": workflow });
    let options: Vec<Value> = factory
        .requestable()
        .iter()
        .take(100)
        .map(|name| option(name))
        .collect();
    let mut select =
        json!({ "type": "static_select", "action_id": "workflow", "options": options });
    if let Some(defaults) = factory.request() {
        select["initial_option"] = option(&defaults.workflow);
    }
    let note: String = text(&message["text"]).chars().take(3000).collect();
    let label = |label: &str| json!({ "type": "plain_text", "text": label });
    json!({
        "type": "modal",
        "callback_id": CALLBACK,
        "title": label("Queue in Priory"),
        "submit": label("Queue"),
        "private_metadata": metadata.to_string(),
        "blocks": [
            { "type": "input", "block_id": "workflow", "label": label("Workflow"), "element": select },
            {
                "type": "input",
                "block_id": "repo",
                "optional": true,
                "label": label("Repository"),
                "hint": label("owner/name; empty uses triggers/cli.toml's repo"),
                "element": { "type": "plain_text_input", "action_id": "repo" },
            },
            {
                "type": "input",
                "block_id": "note",
                "label": label("Note"),
                "element": {
                    "type": "plain_text_input",
                    "action_id": "note",
                    "multiline": true,
                    "initial_value": note,
                },
            },
        ],
    })
}

/// Queues the task a submitted form asks for, under its thread's key, from the person who sent it,
/// and says what happened.
fn submit(factory: &Factory, payload: &Value, ended: Ended) -> String {
    let metadata = metadata(payload);
    let values = &payload["view"]["state"]["values"];
    let value = |block: &str| {
        let element = &values[block][block];
        element["selected_option"]["value"]
            .as_str()
            .or(element["value"].as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let (channel, thread) = (text(&metadata["channel"]), text(&metadata["thread"]));
    if channel.is_empty() || thread.is_empty() {
        return "Not queued: the form lost its thread; use the shortcut again.".to_owned();
    }
    let note = value("note").unwrap_or_default();
    let text = match metadata["link"].as_str() {
        Some(link) => format!("{note}\n\n{link}"),
        None => note,
    };
    let add = Add {
        text,
        workflow: value("workflow"),
        band: None,
        lane: None,
        repo: value("repo"),
        relates: Vec::new(),
        intervention: None,
        session: None,
        auto_answer: None,
        labels: Vec::new(),
    };
    let key = format!("slack:{channel}:{thread}");
    let ended = match ended(&key) {
        Ok(ended) => ended,
        Err(error) => return format!("Not queued: {error}"),
    };
    match request::add_as(factory, &add, &key, ended, Some(&person(payload))) {
        Ok(key) => format!("Queued as `{key}`."),
        Err(Refused::Repeated(key)) => {
            format!("This thread is already queued as `{key}`, so nothing was added.")
        }
        Err(Refused::Invalid(reason)) => format!("Not queued: {reason}"),
    }
}

/// Who sent the form, as `<name> (<user id>)`, or the user id alone when Slack names nobody.
fn person(payload: &Value) -> String {
    let user = &payload["user"];
    let id = text(&user["id"]);
    let name: String = user["username"]
        .as_str()
        .or(user["name"].as_str())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    match name.trim() {
        "" => id.to_owned(),
        name => format!("{name} ({id})"),
    }
}

/// The form's metadata, or nothing for a payload that is not a submission.
fn metadata(payload: &Value) -> Value {
    let metadata = text(&payload["view"]["private_metadata"]);
    serde_json::from_str(metadata).unwrap_or_default()
}

/// Posts `message` as an ephemeral reply to the person who used the shortcut or sent the form.
fn reply(payload: &Value, message: &str) -> Result<(), String> {
    let metadata = metadata(payload);
    let url = payload["response_url"]
        .as_str()
        .or(metadata["response_url"].as_str())
        .unwrap_or_default();
    // The URL comes from Slack; anything else would send the reply elsewhere.
    if !url.starts_with("https://hooks.slack.com/") {
        return Err(format!("no Slack response_url to reply `{message}`"));
    }
    let body = json!({ "response_type": "ephemeral", "text": message });
    post(None, url, &body).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::fuzz::json_value;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn a_shortcut_payload_of_any_shape_gets_a_form_without_panicking(payload in json_value()) {
            form(&Factory::default(), &payload);
            super::metadata(&payload);
            person(&payload);
        }

        #[test]
        fn private_metadata_of_any_text_is_read_without_panicking(text in "\\PC{0,300}") {
            let payload = json!({ "view": { "private_metadata": text } });
            super::metadata(&payload);
        }
    }
}
