//! The Slack message shortcut: `priory intake` against a fake Slack, a WebSocket server on
//! 127.0.0.1 and a fake `curl` for the Web API calls.

use herdr_bus::{Bus, MessageQuery, Stored};
use serde_json::{Value, json};
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const APP_TOKEN: &str = "xapp-1-secret-app-token";
const BOT_TOKEN: &str = "xoxb-secret-bot-token";
const ALLOWED: &str = "U0ALLOWED";
const CHANNEL: &str = "C0123";
const THREAD: &str = "1763500000.000100";

const WORKFLOW: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\n+++\n";

/// A factory with the shortcut, a fake `curl`, and a fake Slack listening for the intake.
struct Slack {
    dir: PathBuf,
    listener: TcpListener,
    intake: Option<Child>,
}

impl Slack {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("shortcut")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let settings = format!(
            "name = \"demo\"\n\n[request]\nworkflow = \"implement\"\n\n[slack.shortcut]\nusers = [\"{ALLOWED}\"]\n"
        );
        let files = [
            ("factory/factory.toml", settings.as_str()),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            write(&dir.join(path), content);
        }
        fs::create_dir_all(dir.join("calls")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        // Each call's config (stdin) goes to its own file, named by time; `views.open` waits while
        // `hold` exists, for 10 s at most.
        let curl = format!(
            "#!/bin/sh\necho \"$*\" >> '{dir}/args'\ncfg=$(cat)\n\
             call='{dir}/calls/'$(date +%s%N)-$$\nprintf '%s' \"$cfg\" > \"$call.tmp\"; mv \"$call.tmp\" \"$call\"\n\
             case \"$cfg\" in\n\
             *apps.connections.open*) if [ -e '{dir}/fail-open' ]; then rm '{dir}/fail-open'; \
             echo \"> $(printf %s \"$cfg\" | grep Authorization)\" >&2; echo 'curl: (22) 500' >&2; exit 22; fi; echo '{{\"ok\":true,\"url\":\"ws://127.0.0.1:{port}/link\"}}' ;;\n\
             *views.open*) n=0; while [ -e '{dir}/hold' ] && [ $n -lt 200 ]; do sleep 0.05; n=$((n+1)); done; echo '{{\"ok\":true}}' ;;\n\
             *) echo '{{\"ok\":true}}' ;;\nesac\n",
            dir = dir.display()
        );
        let bin = dir.join("bin/curl");
        write(&bin, &curl);
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        Slack {
            dir,
            listener,
            intake: None,
        }
    }

    fn start(&mut self) {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let child = Command::new(env!("CARGO_BIN_EXE_priory"))
            .arg("intake")
            .arg(self.dir.join("factory"))
            .env("PATH", path)
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("SLACK_APP_TOKEN", APP_TOKEN)
            .env("SLACK_BOT_TOKEN", BOT_TOKEN)
            .stdout(fs::File::create(self.dir.join("stdout")).unwrap())
            .stderr(fs::File::create(self.dir.join("stderr")).unwrap())
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        self.intake = Some(child);
    }

    /// The intake's next connection, greeted with `hello`.
    fn accept(&self) -> WebSocket<TcpStream> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(error) => panic!("the intake never connected: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        socket
            .send(Message::text(
                json!({"type": "hello", "num_connections": 1}).to_string(),
            ))
            .unwrap();
        socket
    }

    /// The Web API calls so far, each as the curl config the intake wrote on stdin.
    fn calls(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(self.dir.join("calls"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        names
            .iter()
            .map(|name| fs::read_to_string(name).unwrap())
            .collect()
    }

    /// Waits for a call whose config contains `what`.
    fn call(&self, what: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(call) = self.calls().into_iter().find(|call| call.contains(what)) {
                return call;
            }
            assert!(
                Instant::now() < deadline,
                "no call to {what}: {:?}",
                self.calls()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn published(&self) -> Vec<Stored> {
        let Ok(bus) = Bus::open(&self.dir.join("bus.db")) else {
            return Vec::new();
        };
        let mut messages = bus
            .messages(&MessageQuery {
                output: Some("factory.intake".into()),
                limit: 1000,
                ..MessageQuery::default()
            })
            .unwrap_or_default();
        messages.reverse();
        messages
    }
}

impl Drop for Slack {
    fn drop(&mut self) {
        if let Some(mut child) = self.intake.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn write(file: &Path, content: &str) {
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
}

fn envelope(id: &str, payload: Value) -> Message {
    Message::text(
        json!({
            "envelope_id": id,
            "type": "interactive",
            "accepts_response_payload": false,
            "payload": payload,
        })
        .to_string(),
    )
}

fn shortcut(user: &str) -> Value {
    json!({
        "type": "message_action",
        "callback_id": "priory_queue",
        "trigger_id": "13345224609.8534564800.6f8ab1f53e13d0cd15f96106292d5536",
        "response_url": "https://hooks.slack.com/app-actions/T0MJR11A4/21974584944/secret",
        "user": {"id": user},
        "channel": {"id": CHANNEL},
        "team": {"id": "T0MJR11A4", "domain": "acme"},
        "message": {"ts": "1763500000.000200", "thread_ts": THREAD, "text": "Login fails on Safari"},
    })
}

/// The next text frame, skipping pings.
fn read(socket: &mut WebSocket<TcpStream>) -> Value {
    loop {
        match socket.read().expect("a frame before the timeout") {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected frame {other:?}"),
        }
    }
}

#[test]
fn the_intake_acknowledges_a_shortcut_before_it_opens_the_form() {
    let mut slack = Slack::new("ack-first");
    write(&slack.dir.join("hold"), "");
    slack.start();
    let mut socket = slack.accept();

    // While `hold` exists `views.open` cannot answer, so an ack read now did not wait for it.
    socket.send(envelope("e1", shortcut(ALLOWED))).unwrap();
    let ack = read(&mut socket);
    fs::remove_file(slack.dir.join("hold")).unwrap();
    let opened = slack.call("views.open");

    assert_eq!(ack, json!({"envelope_id": "e1"}));
    assert!(opened.contains("13345224609.8534564800"), "{opened}");
}

/// The JSON body of a call, from the `data = "..."` line of its curl config.
fn body(call: &str) -> Value {
    let line = call
        .lines()
        .find_map(|line| line.strip_prefix("data = \""))
        .expect("a data line");
    let quoted = line.strip_suffix('"').unwrap();
    serde_json::from_str(&quoted.replace("\\\"", "\"").replace("\\\\", "\\")).unwrap()
}

/// The submission of the form `views.open` opened, by `user`, with `note`.
fn submission(opened: &Value, user: &str, note: &str) -> Value {
    let view = &opened["view"];
    json!({
        "type": "view_submission",
        "user": {"id": user},
        "view": {
            "callback_id": view["callback_id"],
            "private_metadata": view["private_metadata"],
            "state": {"values": {
                "workflow": {"workflow": {"type": "static_select", "selected_option": {"value": "implement"}}},
                "repo": {"repo": {"type": "plain_text_input", "value": "owner/repo"}},
                "note": {"note": {"type": "plain_text_input", "value": note}},
            }},
        },
    })
}

/// The ephemeral replies posted so far.
fn replies(slack: &Slack) -> Vec<String> {
    slack
        .calls()
        .iter()
        .filter(|call| call.contains("url = \"https://hooks.slack.com/"))
        .map(|call| body(call)["text"].as_str().unwrap().to_owned())
        .collect()
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn requests(slack: &Slack) -> Vec<Value> {
    slack
        .published()
        .into_iter()
        .filter(|stored| stored.message.kind == "factory.task.requested")
        .map(|stored| stored.message.data)
        .collect()
}

#[test]
fn an_allowed_submission_queues_the_thread_once_and_says_so() {
    let mut slack = Slack::new("queue-once");
    slack.start();
    let mut socket = slack.accept();
    socket.send(envelope("e1", shortcut(ALLOWED))).unwrap();
    read(&mut socket);
    let opened = body(&slack.call("views.open"));

    socket
        .send(envelope(
            "e2",
            submission(&opened, ALLOWED, "Fix the Safari login"),
        ))
        .unwrap();
    read(&mut socket);
    wait_for("the first reply", || replies(&slack).len() == 1);
    socket
        .send(envelope("e3", submission(&opened, ALLOWED, "Another note")))
        .unwrap();
    read(&mut socket);
    wait_for("the second reply", || replies(&slack).len() == 2);

    let key = format!("slack:{CHANNEL}:{THREAD}");
    let requests = requests(&slack);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["key"], key);
    assert_eq!(requests[0]["workflow"], "implement");
    assert_eq!(requests[0]["repo"], "owner/repo");
    assert_eq!(
        replies(&slack),
        [
            format!("Queued as `{key}`."),
            format!("This thread is already queued as `{key}`, so nothing was added."),
        ]
    );
}

#[test]
fn a_person_off_the_allowlist_is_refused_and_nothing_is_queued() {
    let mut slack = Slack::new("refused");
    slack.start();
    let mut socket = slack.accept();
    socket.send(envelope("e1", shortcut("U0STRANGER"))).unwrap();
    read(&mut socket);
    wait_for("the refusal", || replies(&slack).len() == 1);
    let opened = json!({"view": {"callback_id": "priory_queue", "private_metadata": json!({
        "channel": CHANNEL, "thread": THREAD,
        "response_url": "https://hooks.slack.com/app-actions/T0MJR11A4/1/secret",
    }).to_string()}});
    socket
        .send(envelope("e2", submission(&opened, "U0STRANGER", "sneak")))
        .unwrap();
    read(&mut socket);
    wait_for("the second refusal", || replies(&slack).len() == 2);

    let refusal = "You are not on this Priory's `[slack.shortcut] users`, so nothing was queued.";
    assert_eq!(replies(&slack), [refusal, refusal]);
    assert!(!slack.calls().iter().any(|call| call.contains("views.open")));
    assert!(requests(&slack).is_empty());
}

#[test]
fn the_intake_reconnects_after_a_disconnect_and_a_dropped_socket() {
    let mut slack = Slack::new("reconnect");
    slack.start();
    let mut socket = slack.accept();
    socket
        .send(Message::text(
            json!({"type": "disconnect", "reason": "refresh_requested"}).to_string(),
        ))
        .unwrap();
    let mut socket_two = slack.accept();
    socket_two
        .send(envelope("e2", json!({"type": "block_actions"})))
        .unwrap();
    let ack_two = read(&mut socket_two);
    drop(socket_two);
    drop(socket);
    let mut socket_three = slack.accept();
    socket_three
        .send(envelope("e3", json!({"type": "block_actions"})))
        .unwrap();
    let ack_three = read(&mut socket_three);

    assert_eq!(ack_two, json!({"envelope_id": "e2"}));
    assert_eq!(ack_three, json!({"envelope_id": "e3"}));
    let opens = slack
        .calls()
        .iter()
        .filter(|call| call.contains("apps.connections.open"))
        .count();
    assert_eq!(opens, 3);
}

#[test]
fn the_tokens_never_reach_the_logs_or_the_command_line() {
    let mut slack = Slack::new("secret");
    write(&slack.dir.join("fail-open"), "");
    slack.start();
    let mut socket = slack.accept();
    socket.send(envelope("e1", shortcut(ALLOWED))).unwrap();
    read(&mut socket);
    slack.call("views.open");
    drop(
        slack
            .intake
            .take()
            .map(|mut child| (child.kill(), child.wait())),
    );

    let stderr = fs::read_to_string(slack.dir.join("stderr")).unwrap();
    let logs = [
        stderr.clone(),
        fs::read_to_string(slack.dir.join("stdout")).unwrap(),
        fs::read_to_string(slack.dir.join("args")).unwrap(),
    ];
    for log in logs {
        assert!(
            !log.contains(APP_TOKEN) && !log.contains(BOT_TOKEN),
            "{log}"
        );
        assert!(!log.contains("hooks.slack.com"), "{log}");
    }
    assert!(stderr.contains("Bearer <SLACK_APP_TOKEN>"), "{stderr}");
}

#[test]
fn a_full_queue_answers_busy() {
    let mut slack = Slack::new("busy");
    write(&slack.dir.join("hold"), "");
    slack.start();
    let mut socket = slack.accept();
    for n in 0..18 {
        socket
            .send(envelope(&format!("e{n}"), shortcut(ALLOWED)))
            .unwrap();
        read(&mut socket);
    }
    wait_for("a busy reply", || !replies(&slack).is_empty());
    fs::remove_file(slack.dir.join("hold")).unwrap();

    let busy = "Priory is busy with other requests; try again in a minute.";
    assert!(
        replies(&slack).iter().all(|reply| reply == busy),
        "{:?}",
        replies(&slack)
    );
}

#[test]
fn a_submission_completes_a_thread_whose_source_was_published_without_its_task() {
    let mut slack = Slack::new("orphan");
    let key = format!("slack:{CHANNEL}:{THREAD}");
    // The source an earlier submission published before it stopped, with the id the intake gives it.
    let namespace = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        b"https://github.com/maxence-lefebvre/priory/intake",
    );
    let name = format!("priory.task.requested.cli:add:{key}");
    let orphan = herdr_bus::Message::new("priory.task.requested.cli", json!({"text": "first"}))
        .id(uuid::Uuid::new_v5(&namespace, name.as_bytes()))
        .subject(key.clone());
    let bus = Bus::open(&slack.dir.join("bus.db")).unwrap();
    bus.declare(&herdr_bus::Output::new("factory.intake"))
        .unwrap();
    bus.publish("factory.intake", &orphan).unwrap();
    slack.start();
    let mut socket = slack.accept();
    socket.send(envelope("e1", shortcut(ALLOWED))).unwrap();
    read(&mut socket);
    let opened = body(&slack.call("views.open"));

    socket
        .send(envelope("e2", submission(&opened, ALLOWED, "Fix it")))
        .unwrap();
    read(&mut socket);
    wait_for("the reply", || replies(&slack).len() == 1);

    let requests = requests(&slack);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["source"], orphan.id.to_string());
    assert_eq!(replies(&slack), [format!("Queued as `{key}`.")]);
}

#[test]
fn a_reply_url_with_a_newline_cannot_add_a_curl_option() {
    let mut slack = Slack::new("newline");
    slack.start();
    let mut socket = slack.accept();
    let mut payload = shortcut("U0STRANGER");
    payload["response_url"] =
        json!("https://hooks.slack.com/x\nurl = \"https://elsewhere.example/\"");
    socket.send(envelope("e1", payload)).unwrap();
    read(&mut socket);
    socket.send(envelope("e2", shortcut("U0STRANGER"))).unwrap();
    read(&mut socket);
    wait_for("the second refusal", || replies(&slack).len() == 1);

    assert!(!slack.calls().iter().any(|call| call.contains("elsewhere")));
    let stderr = fs::read_to_string(slack.dir.join("stderr")).unwrap();
    assert!(stderr.contains("a control character"), "{stderr}");
}
