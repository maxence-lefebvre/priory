mod conform;
mod tmux_dir;

use herdr_bus::{Bus, Output as BusOutput};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

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

const SERVE: &str =
    "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n";

const TOKEN: &str = "s3cret-token";

const HOOKS: &str = "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n\n[webhooks.github]\nsecret_env = \"TEST_SECRET\"\n";

/// A factory, a bus, a state file and Priory home under the test's own directory. Dropping it kills
/// the server it started and the tmux server of its daemon.
struct Setup {
    dir: PathBuf,
    server: Option<Child>,
    /// The GitHub webhook secret, generated for this test.
    secret: String,
    tmux: TmuxDir,
}

impl Setup {
    fn new(name: &str, settings: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("serve")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", settings),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        fs::create_dir_all(dir.join("fake")).unwrap();
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
            server: None,
            secret: uuid::Uuid::new_v4().to_string(),
        };
        setup
            .bus()
            .declare(&BusOutput::new("factory.intake"))
            .unwrap();
        succeeded(&setup.priory(&["tasks"]));
        setup
    }

    fn bus(&self) -> Bus {
        Bus::open(&self.dir.join("bus.db")).unwrap()
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn command(&self) -> Command {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake");
        let path = std::env::join_paths(
            std::iter::once(fake).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .env_remove("TMUX")
            .env("PATH", path)
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("FAKE_DIR", self.dir.join("fake"))
            .env("TEST_TOKEN", TOKEN)
            .env("TEST_SECRET", &self.secret)
            .env("PRIORY_NOW", "2026-11-17T12:00:00Z");
        command
    }

    fn priory(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    /// Two tasks, the second queued, and an open ask on the first.
    fn seed(&self) {
        for (key, text) in [("docs-1", "Fix the docs"), ("docs-2", "Fix the\nreadme")] {
            let factory = self.factory();
            let output = self.priory(&[
                "request",
                &factory,
                "--repo",
                "owner/repo",
                "--workflow",
                "implement",
                "--key",
                key,
                text,
            ]);
            succeeded(&output);
        }
        succeeded(&self.priory(&["daemon", &self.factory(), "--once"]));
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        db.execute(
            "INSERT INTO asks (task, step, kind, revision, question, options)
             VALUES (1, 'implement', 'agent', 2, 'Which way?\nPick one.', '[\"left\",\"right\"]')",
            [],
        )
        .unwrap();
    }

    /// Starts `priory <args>` and returns the address it prints once it listens.
    fn start(&mut self, args: &[&str]) -> String {
        let mut child = self
            .command()
            .args(args)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        self.server = Some(child);
        let mut said = String::new();
        loop {
            let mut line = String::new();
            assert!(
                stderr.read_line(&mut line).unwrap() > 0,
                "priory {args:?} exited before it listened: {said}"
            );
            if let Some(address) = line.trim().strip_prefix("priory: serving on http://") {
                std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
                return address.to_owned();
            }
            said.push_str(&line);
        }
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(mut server) = self.server.take() {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs `command`, killed if it still runs after `limit`, so a server that starts fails the test.
fn within(mut command: Command, limit: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + limit;
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    child.wait_with_output().unwrap()
}

/// Sends `head` as a whole request head and returns the status code and body.
fn raw(address: &str, head: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(head.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, body.to_owned())
}

fn get(address: &str, path: &str, token: Option<&str>) -> (u16, String) {
    let auth = token.map_or(String::new(), |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    raw(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n{auth}Connection: close\r\n\r\n"),
    )
}

fn api(address: &str, path: &str) -> Value {
    let (status, body) = get(address, path, Some(TOKEN));
    assert_eq!(status, 200, "{path}: {body}");
    serde_json::from_str(&body).unwrap()
}

fn json_of(setup: &Setup, verb: &str) -> Value {
    let output = setup.priory(&[verb, "--json"]);
    succeeded(&output);
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Whether process `pid` holds a listening TCP socket.
fn listening(pid: u32) -> bool {
    let sockets: Vec<String> = fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|fd| fs::read_link(fd.ok()?.path()).ok())
        .filter_map(|link| {
            let link = link.to_string_lossy().into_owned();
            Some(link.strip_prefix("socket:[")?.strip_suffix(']')?.to_owned())
        })
        .collect();
    ["tcp", "tcp6"].iter().any(|table| {
        fs::read_to_string(format!("/proc/{pid}/net/{table}"))
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split_whitespace().collect::<Vec<_>>())
            // State 0A is LISTEN; column 9 is the socket's inode.
            .any(|cells| cells[3] == "0A" && sockets.contains(&cells[9].to_owned()))
    })
}

#[test]
fn check_refuses_a_listen_address_beyond_loopback_without_public() {
    let setup = Setup::new(
        "public",
        "name = \"demo\"\n[serve]\nlisten = \"0.0.0.0:7340\"\n",
    );

    let output = setup.priory(&["check", &setup.factory()]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "factory.toml:2: `[serve] listen` 0.0.0.0:7340 is beyond this machine: set `public = true` to serve it\n"
    );
    let public = "name = \"demo\"\n[serve]\nlisten = \"0.0.0.0:7340\"\npublic = true\n";
    fs::write(setup.dir.join("factory/factory.toml"), public).unwrap();
    succeeded(&setup.priory(&["check", &setup.factory()]));
}

#[test]
fn check_refuses_webhooks_without_a_serve_table() {
    let setup = Setup::new(
        "webhooks-no-serve",
        "name = \"demo\"\n\n[webhooks.github]\nsecret_env = \"TEST_SECRET\"\n",
    );

    let output = setup.priory(&["check", &setup.factory()]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "factory.toml:3: `[webhooks]` needs a `[serve]` table to listen on\n"
    );
}

#[test]
fn serve_refuses_a_listen_flag_beyond_loopback_without_public() {
    let setup = Setup::new("flag", SERVE);

    let output = setup.priory(&["serve", &setup.factory(), "--listen", "0.0.0.0:7340"]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory serve: `[serve] listen` 0.0.0.0:7340 is beyond this machine: set `public = true` to serve it\n"
    );
}

#[test]
fn serve_refuses_to_start_without_its_token() {
    let setup = Setup::new("token", SERVE);

    for value in [None, Some("")] {
        let mut command = setup.command();
        command.args(["serve", &setup.factory()]);
        match value {
            None => command.env_remove("TEST_TOKEN"),
            Some(value) => command.env("TEST_TOKEN", value),
        };
        let output = command.output().unwrap();

        assert!(!output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "priory serve: TEST_TOKEN is unset or empty: set it to the token clients send\n"
        );
    }
}

#[test]
fn serve_and_the_daemon_refuse_to_start_without_the_webhook_secret() {
    let setup = Setup::new("secret", HOOKS);

    for verb in ["serve", "daemon"] {
        for value in [None, Some("")] {
            let mut command = setup.command();
            command.args([verb, &setup.factory()]);
            match value {
                None => command.env_remove("TEST_SECRET"),
                Some(value) => command.env("TEST_SECRET", value),
            };
            let output = within(command, Duration::from_secs(10));

            assert!(!output.status.success());
            assert_eq!(
                String::from_utf8_lossy(&output.stderr),
                format!(
                    "priory {verb}: TEST_SECRET is unset or empty: set it to the GitHub webhook's secret\n"
                )
            );
        }
    }
}

#[test]
fn serve_refuses_a_factory_without_a_serve_table() {
    let setup = Setup::new("none", "name = \"demo\"\n");

    let output = setup.priory(&["serve", &setup.factory()]);

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory serve: factory.toml has no [serve] table, so nothing listens\n"
    );
}

#[test]
fn a_request_without_the_token_gets_401() {
    let mut setup = Setup::new("auth", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);

    for token in [
        None,
        Some("wrong"),
        Some("s3cret-tokenX"),
        Some("s3cret-toke"),
    ] {
        let (status, _) = get(&address, "/api/tasks", token);
        assert_eq!(status, 401, "token {token:?}");
    }
    let (status, _) = get(&address, "/api/tasks", Some(TOKEN));
    assert_eq!(status, 200);
}

#[test]
fn each_endpoint_returns_what_the_cli_shows_and_its_json() {
    let mut setup = Setup::new("endpoints", SERVE);
    setup.seed();
    let address = setup.start(&["serve", &setup.factory()]);

    for verb in ["desk", "inbox", "asks", "tasks", "queue"] {
        assert_eq!(
            api(&address, &format!("/api/{verb}")),
            json_of(&setup, verb),
            "{verb}"
        );
    }
    let tasks = api(&address, "/api/tasks");
    assert_eq!(
        tasks["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| (
                task["id"].clone(),
                task["key"].clone(),
                task["state"].clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            (json!(1), json!("cli:docs-1"), json!("queued")),
            (json!(2), json!("cli:docs-2"), json!("queued")),
        ]
    );
    let text = String::from_utf8(setup.priory(&["tasks"]).stdout).unwrap();
    assert!(text.starts_with(
        "id\tkey\tworkflow\tstep\tstate\tband\tlane\treason\tposition\tgeneration\tretired\tgrade\tgrade_reason\n"
    ));
    let asks = api(&address, "/api/asks");
    let ask = &asks["asks"][0];
    assert_eq!(
        [
            &ask["id"],
            &ask["task"],
            &ask["key"],
            &ask["state"],
            &ask["revision"],
            &ask["question"],
            &ask["options"]
        ],
        [
            &json!(1),
            &json!(1),
            &json!("cli:docs-1"),
            &json!("open"),
            &json!(2),
            &json!("Which way?\nPick one."),
            &json!(["left", "right"])
        ]
    );
    let asks = String::from_utf8(setup.priory(&["asks"]).stdout).unwrap();
    assert_eq!(
        asks,
        "id\ttask\trevision\tquestion\toptions\tsnoozed_until\tback_early\ttitle\ttext\twhat_waits\tnext_action\tcommand\tcoordinator\tkey\trun_id\tsession\n1\t1\t2\tWhich way?\\nPick one.\t[\"left\",\"right\"]\t-\t-\t-\t-\t-\t-\t-\t-\tcli:docs-1\t-\t-\n"
    );
    let queue = api(&address, "/api/queue");
    let waiting = &queue["waiting"];
    assert_eq!(
        [
            &waiting[0]["id"],
            &waiting[0]["position"],
            &waiting[1]["id"],
            &waiting[1]["band_position"]
        ],
        [&json!(1), &json!(1), &json!(2), &json!(2)]
    );
    assert_eq!(queue["running"], json!([]));
    assert_eq!(
        api(&address, "/api/desk")["counts"],
        json!({ "asleep": 0, "snoozed": 0 })
    );
    let desk = api(&address, "/api/desk");
    assert_eq!(desk["banner"], Value::Null);
    assert_eq!(desk["items"][0]["kind"], "ask");
    let inbox = api(&address, "/api/inbox");
    let text = String::from_utf8(setup.priory(&["inbox"]).stdout).unwrap();
    assert_eq!(
        inbox["inbox"].as_array().unwrap().len(),
        text.lines().count() - 1
    );
}

#[test]
fn a_task_endpoint_returns_its_runs_steps_asks_waits_results_and_tldr() {
    let mut setup = Setup::new("task", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["tldr", "1", "Fixes the docs."]));
    let address = setup.start(&["serve", &setup.factory()]);

    let task = api(&address, "/api/tasks/1");

    assert_eq!(task["id"], 1);
    assert_eq!(task["key"], "cli:docs-1");
    assert_eq!(task["tldr"], "Fixes the docs.");
    assert_eq!(task["asks"][0]["revision"], 2);
    for list in ["runs", "steps", "waits", "results"] {
        assert_eq!(task[list], json!([]), "{list}");
    }
    assert_eq!(get(&address, "/api/tasks/9", Some(TOKEN)).0, 404);
    assert_eq!(get(&address, "/api/tasks/x", Some(TOKEN)).0, 404);
}

#[test]
fn unknown_paths_404_other_methods_405_and_oversize_heads_431() {
    let mut setup = Setup::new("limits", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(get(&address, "/api/nothing", Some(TOKEN)).0, 404);
    let post = format!(
        "POST /api/tasks HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 0\r\n\r\n"
    );
    assert_eq!(raw(&address, &post).0, 405);
    let body = format!(
        "GET /api/tasks HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 5\r\n\r\nhello"
    );
    assert_eq!(raw(&address, &body).0, 413);
    let twice = format!(
        "GET /api/tasks HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 0\r\nContent-Length: 5\r\n\r\nhello"
    );
    assert_eq!(raw(&address, &twice).0, 413);
    let huge = format!(
        "GET /api/tasks HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\nX-Pad: {}\r\n\r\n",
        "a".repeat(16 * 1024)
    );
    assert_eq!(raw(&address, &huge).0, 431);
    let (status, body) = get(&address, "/", Some(TOKEN));
    assert_eq!(status, 200);
    assert!(body.contains("<html"), "{body}");
}

#[test]
fn web_false_serves_only_the_api() {
    let mut setup = Setup::new("web", &format!("{SERVE}web = false\n"));
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(get(&address, "/", Some(TOKEN)).0, 404);
    assert_eq!(get(&address, "/api/tasks", Some(TOKEN)).0, 200);
}

#[test]
fn a_silent_client_is_dropped_after_the_read_timeout() {
    let mut setup = Setup::new("timeout", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let mut stream = TcpStream::connect(&address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let started = Instant::now();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn the_daemon_serves_its_state_when_the_factory_has_a_serve_table() {
    let mut setup = Setup::new("daemon", SERVE);
    setup.seed();

    let address = setup.start(&["daemon", &setup.factory()]);

    assert_eq!(api(&address, "/api/tasks"), json_of(&setup, "tasks"));
}

#[test]
fn the_daemon_listens_nowhere_without_a_serve_table() {
    let mut setup = Setup::new("quiet", "name = \"demo\"\n");
    let child = setup
        .command()
        .args(["daemon", &setup.factory()])
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    setup.server = Some(child);
    let socket = setup.dir.join("home");
    let deadline = Instant::now() + Duration::from_secs(10);
    // The daemon's hook socket under PRIORY_HOME shows it is past its start.
    while !fs::read_dir(&socket).is_ok_and(|mut dir| {
        dir.any(|entry| {
            entry.is_ok_and(|entry| {
                entry.file_type().is_ok_and(|kind| {
                    use std::os::unix::fs::FileTypeExt;
                    kind.is_socket()
                })
            })
        })
    }) {
        assert!(Instant::now() < deadline, "the daemon did not start");
        std::thread::sleep(Duration::from_millis(50));
    }

    assert!(!listening(pid));
}

#[test]
fn a_client_that_trickles_its_head_is_dropped_after_the_read_timeout() {
    let mut setup = Setup::new("trickle", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let mut stream = TcpStream::connect(&address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let head = "GET /api/tasks HTTP/1.1\r\nX-Slow: aaaaaaaaaa\r\n\r\n";
    std::thread::spawn(move || {
        for byte in head.bytes() {
            if writer.write_all(&[byte]).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
}

#[test]
fn serve_reads_the_state_without_creating_it() {
    let setup = Setup::new("fresh", SERVE);
    fs::remove_file(setup.dir.join("priory.db")).unwrap();

    let output = setup.priory(&["serve", &setup.factory()]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unable to open database file"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!setup.dir.join("priory.db").exists());
}

#[test]
fn the_queue_lists_held_tasks_with_their_hold_after_the_waiting_ones() {
    let mut setup = Setup::new("held", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "UPDATE tasks SET state = 'held', reason = 'until tomorrow' WHERE id = 1;
         INSERT INTO holds (task, reason, kind, until, resume)
         VALUES (1, 'until tomorrow', 'time', '2099-01-01T09:00:00Z', 'queued');",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let waiting = &api(&address, "/api/queue")["waiting"];

    assert_eq!(
        [
            &waiting[0]["id"],
            &waiting[0]["position"],
            &waiting[1]["id"],
            &waiting[1]["position"]
        ],
        [&json!(2), &json!(1), &json!(1), &Value::Null]
    );
    assert_eq!(
        [&waiting[1]["hold"], &waiting[1]["hold_until"]],
        [&json!("time"), &json!("2099-01-01T09:00:00Z")]
    );
    assert_eq!(api(&address, "/api/tasks/2")["position"], 1);
}

const TRIGGER: &str = r#"
source = "github"
repo = "owner/repo"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
band = "today"
"#;

/// A GitHub `issues` delivery: issue 42 of `owner/repo` labeled `label`.
fn issue_labeled(label: &str) -> String {
    json!({
        "action": "labeled",
        "issue": {
            "id": 9042,
            "number": 42,
            "title": "Fix the thing",
            "html_url": "https://github.com/owner/repo/issues/42",
            "user": { "login": "max" },
            "created_at": "2026-11-17T09:00:00Z",
            "updated_at": "2026-11-17T10:00:00Z"
        },
        "label": { "id": 1, "name": label, "color": "ededed" },
        "repository": { "id": 1, "full_name": "owner/repo" },
        "sender": { "login": "max" }
    })
    .to_string()
}

/// `sha256=` and the hex HMAC-SHA256 of `body` under `secret`, as GitHub signs a delivery.
fn sign(secret: &str, body: &[u8]) -> String {
    use hmac::{KeyInit, Mac};
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256={hex}")
}

/// POSTs `body` to `/hooks/github` as GitHub would, with `signature` when given.
fn deliver(
    address: &str,
    event: &str,
    delivery: &str,
    signature: Option<&str>,
    body: &str,
) -> (u16, String) {
    let signature = signature.map_or(String::new(), |signature| {
        format!("X-Hub-Signature-256: {signature}\r\n")
    });
    raw(
        address,
        &format!(
            "POST /hooks/github HTTP/1.1\r\nHost: {address}\r\nUser-Agent: GitHub-Hookshot/044aadd\r\n\
             Content-Type: application/json\r\nX-GitHub-Event: {event}\r\nX-GitHub-Delivery: {delivery}\r\n\
             {signature}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

impl Setup {
    /// A factory with `[serve]`, the GitHub webhook and a trigger on `agent` labels.
    fn hooks(name: &str) -> Self {
        let setup = Setup::new(name, HOOKS);
        fs::create_dir_all(setup.dir.join("factory/triggers")).unwrap();
        fs::write(setup.dir.join("factory/triggers/agent-label.toml"), TRIGGER).unwrap();
        setup
    }

    /// Everything on `factory.intake`, oldest first, each checked against its schema.
    fn published(&self) -> Vec<herdr_bus::Message> {
        let mut messages = self
            .bus()
            .messages(&herdr_bus::MessageQuery {
                output: Some("factory.intake".into()),
                limit: 1000,
                ..herdr_bus::MessageQuery::default()
            })
            .unwrap();
        messages.reverse();
        for stored in &messages {
            conform::conforms(&serde_json::to_value(&stored.message).unwrap());
        }
        messages.into_iter().map(|stored| stored.message).collect()
    }
}

#[test]
fn a_signed_issues_labeled_delivery_creates_the_task_a_poll_would() {
    let mut setup = Setup::hooks("hook-labeled");
    let address = setup.start(&["serve", &setup.factory()]);
    let body = issue_labeled("agent");

    let (status, reply) = deliver(
        &address,
        "issues",
        "72d3162e-cc78-11e3-81ab-4c9367dc0958",
        Some(&sign(&setup.secret, body.as_bytes())),
        &body,
    );

    assert_eq!(status, 202, "{reply}");
    let published = setup.published();
    let kinds: Vec<_> = published.iter().map(|m| m.kind.as_str()).collect();
    assert_eq!(kinds, ["github.issues.labeled", "factory.task.requested"]);
    assert_eq!(published[0].subject.as_deref(), Some("owner/repo#42"));
    assert_eq!(
        published[0].data,
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "title": "Fix the thing",
            "url": "https://github.com/owner/repo/issues/42",
            "label": "agent",
            "actor": "max",
            "time": "2026-11-17T10:00:00Z",
            "github_id": null,
            "delivery": "72d3162e-cc78-11e3-81ab-4c9367dc0958",
            "baseline": false,
            "triggers": ["agent-label"]
        })
    );
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    let tasks = &json_of(&setup, "tasks")["tasks"];
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tasks[0]["key"], "github:owner/repo#42");
    assert_eq!(tasks[0]["workflow"], "implement");
    assert_eq!(tasks[0]["band"], "today");
}

#[test]
fn a_wrong_missing_or_other_body_signature_gets_401_and_publishes_nothing() {
    let mut setup = Setup::hooks("hook-unsigned");
    let address = setup.start(&["serve", &setup.factory()]);
    let body = issue_labeled("agent");
    let other = issue_labeled("bug");
    let signatures = [
        Some(sign("not-the-secret", body.as_bytes())),
        None,
        Some(sign(&setup.secret, other.as_bytes())),
        Some("sha256=zz".to_owned()),
        Some(sign(&setup.secret, body.as_bytes()).replace("sha256=", "sha1=")),
    ];

    for signature in &signatures {
        let (status, reply) = deliver(&address, "issues", "d-1", signature.as_deref(), &body);

        assert_eq!(status, 401, "{signature:?}");
        assert_eq!(
            reply,
            r#"{"error":"X-Hub-Signature-256 is missing or does not match the body"}"#
        );
    }
    assert!(setup.published().is_empty());
}

#[test]
fn a_body_over_1_mib_gets_413_before_it_is_read() {
    let mut setup = Setup::hooks("hook-oversize");
    let address = setup.start(&["serve", &setup.factory()]);
    let length = 1024 * 1024 + 1;

    // Only the head goes out: the reply must come without the body.
    let (status, reply) = raw(
        &address,
        &format!(
            "POST /hooks/github HTTP/1.1\r\nHost: {address}\r\nX-GitHub-Event: issues\r\n\
             X-GitHub-Delivery: d-1\r\nX-Hub-Signature-256: sha256=00\r\nContent-Length: {length}\r\n\r\n"
        ),
    );

    assert_eq!(status, 413);
    assert_eq!(reply, r#"{"error":"the body is over 1 MiB"}"#);
    let (status, _) = raw(
        &address,
        &format!(
            "POST /hooks/github HTTP/1.1\r\nHost: {address}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"
        ),
    );
    assert_eq!(status, 411);
    assert!(setup.published().is_empty());
}

#[test]
fn a_replayed_delivery_publishes_nothing_even_under_a_new_delivery_id() {
    let mut setup = Setup::hooks("hook-replay");
    let address = setup.start(&["serve", &setup.factory()]);
    let body = issue_labeled("agent");
    let signature = sign(&setup.secret, body.as_bytes());
    let (status, _) = deliver(&address, "issues", "d-1", Some(&signature), &body);
    assert_eq!(status, 202);
    let first = setup.published();

    for delivery in ["d-1", "d-forged"] {
        let (status, reply) = deliver(&address, "issues", delivery, Some(&signature), &body);

        assert_eq!(status, 202, "{reply}");
    }
    assert_eq!(setup.published(), first);
}

const EVENTS_PATH: &str = "repos/owner/repo/issues/events?per_page=100&page=1";

/// GitHub's issue event `id`: issue 42 labeled `agent` at the time `issue_labeled` reports.
fn polled_label(id: u64, number: u64, label: &str) -> Value {
    json!({
        "id": id,
        "event": "labeled",
        "created_at": "2026-11-17T10:00:00Z",
        "actor": { "login": "max" },
        "label": { "name": label, "color": "ededed" },
        "issue": {
            "id": 9000 + number,
            "number": number,
            "title": "Fix the thing",
            "html_url": format!("https://github.com/owner/repo/issues/{number}"),
            "user": { "login": "max" },
            "created_at": "2026-11-17T09:00:00Z"
        }
    })
}

impl Setup {
    /// Runs `priory intake --once` with `gh api <path>` answering `events` for the issue events
    /// and `[]` for anything else.
    fn poll(&self, events: Value) {
        let answer = self.dir.join("events.json");
        fs::write(&answer, events.to_string()).unwrap();
        let gh = self.dir.join("bin/gh");
        fs::create_dir_all(gh.parent().unwrap()).unwrap();
        fs::write(
            &gh,
            format!(
                "#!/bin/sh\ncase \"$*\" in\n  \"api {EVENTS_PATH}\"*) cat '{}' ;;\n  *) echo '[]' ;;\nesac\n",
                answer.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            gh.parent().unwrap().display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = self.command();
        command
            .args(["intake", "--once", &self.factory()])
            .env("PATH", path);
        succeeded(&command.output().unwrap());
    }

    fn kinds(&self) -> Vec<String> {
        self.published().into_iter().map(|m| m.kind).collect()
    }
}

#[test]
fn a_webhook_then_a_poll_of_the_same_label_create_one_task() {
    let mut setup = Setup::hooks("hook-then-poll");
    setup.poll(json!([polled_label(100, 41, "bug")]));
    let mut expected = setup.kinds();
    let address = setup.start(&["serve", &setup.factory()]);
    let body = issue_labeled("agent");
    let signature = sign(&setup.secret, body.as_bytes());
    assert_eq!(
        deliver(&address, "issues", "d-1", Some(&signature), &body).0,
        202
    );
    expected.extend([
        "github.issues.labeled".into(),
        "factory.task.requested".into(),
    ]);

    setup.poll(json!([
        polled_label(101, 42, "agent"),
        polled_label(100, 41, "bug")
    ]));

    assert_eq!(setup.kinds(), expected);
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    assert_eq!(
        json_of(&setup, "tasks")["tasks"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn a_poll_then_a_webhook_of_the_same_label_create_one_task() {
    let mut setup = Setup::hooks("poll-then-hook");
    setup.poll(json!([polled_label(100, 41, "bug")]));
    setup.poll(json!([
        polled_label(101, 42, "agent"),
        polled_label(100, 41, "bug")
    ]));
    let before = setup.published();
    let address = setup.start(&["serve", &setup.factory()]);
    let body = issue_labeled("agent");
    let signature = sign(&setup.secret, body.as_bytes());

    let (status, reply) = deliver(&address, "issues", "d-1", Some(&signature), &body);

    assert_eq!(status, 202);
    assert_eq!(reply, r#"{"outcome":"dropped"}"#);
    assert_eq!(setup.published(), before);
}

#[test]
fn unknown_events_check_runs_and_other_repositories_get_202_and_publish_nothing() {
    let mut setup = Setup::hooks("hook-dropped");
    let address = setup.start(&["serve", &setup.factory()]);
    let mut other_repo: Value = serde_json::from_str(&issue_labeled("agent")).unwrap();
    other_repo["repository"]["full_name"] = json!("someone/else");
    let mut unlabeled: Value = serde_json::from_str(&issue_labeled("agent")).unwrap();
    unlabeled["action"] = json!("unlabeled");
    let check_run = json!({
        "action": "completed",
        "check_run": { "id": 7, "name": "test", "conclusion": "success", "pull_requests": [] },
        "repository": { "full_name": "owner/repo" }
    });
    let ping = json!({ "zen": "Keep it logically awesome.", "hook_id": 1 });
    let deliveries = [
        ("ping", ping),
        ("check_run", check_run),
        ("issues", unlabeled),
        ("issues", other_repo),
    ];

    for (event, body) in deliveries {
        let body = body.to_string();
        let signature = sign(&setup.secret, body.as_bytes());
        let (status, reply) = deliver(&address, event, "d-1", Some(&signature), &body);

        assert_eq!(status, 202, "{event}: {reply}");
        assert_eq!(reply, r#"{"outcome":"dropped"}"#, "{event}");
    }
    assert!(setup.published().is_empty());
}

/// A GitHub `pull_request` payload's pull request 43 of `owner/repo`.
fn pull() -> Value {
    json!({
        "id": 555,
        "number": 43,
        "title": "Add the thing",
        "html_url": "https://github.com/owner/repo/pull/43",
        "user": { "login": "max" },
        "created_at": "2026-11-17T09:00:00Z",
        "updated_at": "2026-11-17T10:00:00Z",
        "closed_at": "2026-11-17T11:00:00Z",
        "merged": true,
        "head": { "ref": "priory/3", "sha": "abc123" },
        "base": { "ref": "main" }
    })
}

#[test]
fn issue_pull_request_and_review_deliveries_publish_their_poll_types() {
    let mut setup = Setup::hooks("hook-types");
    let address = setup.start(&["serve", &setup.factory()]);
    let repository = json!({ "full_name": "owner/repo" });
    let sender = json!({ "login": "ana" });
    let mut opened: Value = serde_json::from_str(&issue_labeled("agent")).unwrap();
    opened["action"] = json!("opened");
    let deliveries = [
        ("issues", opened),
        (
            "pull_request",
            json!({ "action": "opened", "pull_request": pull(), "repository": repository, "sender": sender }),
        ),
        (
            "pull_request",
            json!({ "action": "labeled", "label": { "name": "agent" }, "pull_request": pull(), "repository": repository, "sender": sender }),
        ),
        (
            "pull_request",
            json!({ "action": "closed", "pull_request": pull(), "repository": repository, "sender": sender }),
        ),
        (
            "pull_request_review",
            json!({
                "action": "submitted",
                "review": {
                    "id": 2561579718u64,
                    "state": "approved",
                    "html_url": "https://github.com/owner/repo/pull/43#pullrequestreview-2561579718",
                    "user": { "login": "ana" },
                    "submitted_at": "2026-11-17T10:30:00Z"
                },
                "pull_request": pull(),
                "repository": repository,
                "sender": sender
            }),
        ),
    ];

    for (event, body) in deliveries {
        let body = body.to_string();
        let signature = sign(&setup.secret, body.as_bytes());
        let (status, reply) = deliver(&address, event, "d-2", Some(&signature), &body);
        assert_eq!(status, 202, "{event}: {reply}");
    }

    let published = setup.published();
    assert_eq!(
        setup.kinds(),
        [
            "github.issues.opened",
            "github.pull_request.opened",
            "github.pull_request.labeled",
            "github.pull_request.closed",
            "github.pull_request_review.submitted"
        ]
    );
    assert_eq!(published[3].data["merged"], true);
    assert_eq!(published[3].time.to_string(), "2026-11-17T11:00:00Z");
    let review = &published[4];
    assert_eq!(review.subject.as_deref(), Some("owner/repo#43"));
    assert_eq!(
        review.data,
        json!({
            "repo": "owner/repo",
            "pull_request": 43,
            "url": "https://github.com/owner/repo/pull/43#pullrequestreview-2561579718",
            "head_branch": "priory/3",
            "head_sha": "abc123",
            "base_branch": "main",
            "state": "approved",
            "actor": "ana",
            "time": "2026-11-17T10:30:00Z",
            "github_id": 2561579718u64,
            "delivery": "d-2",
            "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_poll_requests_the_task_a_crash_left_out_of_a_webhook() {
    let setup = Setup::hooks("hook-crash");
    setup.poll(json!([polled_label(100, 41, "bug")]));
    // A delivery published its event, then the server died before the task request.
    let source = herdr_bus::Message::new(
        "github.issues.labeled",
        json!({
            "repo": "owner/repo",
            "issue": 42,
            "title": "Fix the thing",
            "url": "https://github.com/owner/repo/issues/42",
            "label": "agent",
            "actor": "max",
            "time": "2026-11-17T10:00:00Z",
            "github_id": null,
            "delivery": "d-1",
            "baseline": false,
            "triggers": ["agent-label"]
        }),
    )
    .subject("owner/repo#42")
    .time("2026-11-17T10:00:00Z".parse().unwrap())
    .link("issue", "https://github.com/owner/repo/issues/42");
    setup.bus().publish("factory.intake", &source).unwrap();
    let mut expected = setup.kinds();

    setup.poll(json!([
        polled_label(101, 42, "agent"),
        polled_label(100, 41, "bug")
    ]));

    expected.push("factory.task.requested".into());
    assert_eq!(setup.kinds(), expected);
    let request = setup.published().pop().unwrap();
    assert_eq!(request.data["source"], source.id.to_string());
}

#[test]
fn malformed_or_stale_signed_deliveries_publish_nothing() {
    let mut setup = Setup::hooks("hook-malformed");
    let address = setup.start(&["serve", &setup.factory()]);
    let signed = |body: &str| sign(&setup.secret, body.as_bytes());

    let not_json = "{not json";
    let (status, _) = deliver(
        &address,
        "issues",
        "d-1",
        Some(&sign("wrong", not_json.as_bytes())),
        not_json,
    );
    assert_eq!(
        status, 401,
        "the signature is checked before the body is parsed"
    );

    let mut stale: Value = serde_json::from_str(&issue_labeled("agent")).unwrap();
    stale["issue"]["updated_at"] = json!("2026-11-09T10:00:00Z");
    let stale = stale.to_string();
    let (status, reply) = deliver(&address, "issues", "d-2", Some(&signed(&stale)), &stale);
    assert_eq!((status, reply.as_str()), (202, r#"{"outcome":"dropped"}"#));

    let mut no_id: Value = serde_json::from_str(&issue_labeled("agent")).unwrap();
    no_id["action"] = json!("opened");
    no_id["issue"].as_object_mut().unwrap().remove("id");
    let no_id = no_id.to_string();
    let (status, reply) = deliver(&address, "issues", "d-3", Some(&signed(&no_id)), &no_id);
    assert_eq!(
        (status, reply.as_str()),
        (400, r#"{"error":"the payload lacks the event's id"}"#)
    );

    let body = issue_labeled("agent");
    let (status, _) = raw(
        &address,
        &format!(
            "POST /hooks/github HTTP/1.1\r\nHost: {address}\r\nX-GitHub-Event: issues\r\nX-GitHub-Delivery: d-4\r\n\
             X-Hub-Signature-256: {}\r\nContent-Length: {}\r\nContent-Length: 1048577\r\n\r\n{body}",
            signed(&body),
            body.len()
        ),
    );
    assert_eq!(status, 400);
    assert!(setup.published().is_empty());
}

#[test]
fn a_change_bumps_the_revision_that_each_listing_carries() {
    let mut setup = Setup::new("revision", SERVE);
    let before = json_of(&setup, "tasks");
    assert_eq!(before["tasks"], json!([]));

    setup.seed();

    let after = json_of(&setup, "tasks");
    assert!(
        after["revision"].as_i64().unwrap() > before["revision"].as_i64().unwrap(),
        "{before} then {after}"
    );
    assert_eq!(after["tasks"].as_array().unwrap().len(), 2);
    let address = setup.start(&["serve", &setup.factory()]);
    for path in ["desk", "inbox", "asks", "tasks", "queue", "tasks/1"] {
        assert_eq!(
            api(&address, &format!("/api/{path}"))["revision"],
            after["revision"],
            "{path}"
        );
    }
}

#[test]
fn events_answers_at_once_when_behind_and_within_a_second_of_a_change() {
    let mut setup = Setup::new("events", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let revision = api(&address, "/api/tasks")["revision"].as_i64().unwrap();

    let started = Instant::now();
    let behind = api(&address, &format!("/api/events?since={}", revision - 1));
    assert_eq!(behind, json!({ "revision": revision }));
    assert!(started.elapsed() < Duration::from_secs(1));

    let waiter = {
        let address = address.clone();
        std::thread::spawn(move || {
            let reply = api(&address, &format!("/api/events?since={revision}"));
            (reply, Instant::now())
        })
    };
    std::thread::sleep(Duration::from_millis(500));
    succeeded(&setup.priory(&["pause", "lunch"]));
    let changed = Instant::now();
    let (reply, answered) = waiter.join().unwrap();

    assert!(reply["revision"].as_i64().unwrap() > revision, "{reply}");
    assert!(answered.duration_since(changed) < Duration::from_secs(1));
}

#[test]
fn events_answers_with_the_same_revision_after_its_timeout() {
    let mut setup = Setup::new("events-timeout", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let revision = api(&address, "/api/tasks")["revision"].as_i64().unwrap();

    let started = Instant::now();
    let reply = api(&address, &format!("/api/events?since={revision}&timeout=1"));

    assert_eq!(reply, json!({ "revision": revision }));
    assert!(started.elapsed() >= Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(get(&address, "/api/events", Some(TOKEN)).0, 400);
    assert_eq!(get(&address, "/api/events?since=x", Some(TOKEN)).0, 400);
    assert_eq!(
        get(
            &address,
            &format!("/api/events?since={revision}&timeout=26"),
            Some(TOKEN)
        )
        .0,
        400
    );
}

#[test]
fn waiters_beyond_the_cap_get_503_while_listings_still_answer() {
    let mut setup = Setup::new("events-cap", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let revision = api(&address, "/api/tasks")["revision"].as_i64().unwrap();
    let path = format!("/api/events?since={revision}&timeout=3");

    let waiters: Vec<_> = (0..4)
        .map(|_| {
            let (address, path) = (address.clone(), path.clone());
            std::thread::spawn(move || get(&address, &path, Some(TOKEN)).0)
        })
        .collect();
    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(get(&address, &path, Some(TOKEN)).0, 503);
    assert_eq!(get(&address, "/api/tasks", Some(TOKEN)).0, 200);
    let behind = format!("/api/events?since={}", revision - 1);
    assert_eq!(get(&address, &behind, Some(TOKEN)).0, 200);
    for waiter in waiters {
        assert_eq!(waiter.join().unwrap(), 200);
    }
}

#[test]
fn wait_prints_the_new_revision_on_a_change_or_the_same_one_after_its_timeout() {
    let setup = Setup::new("wait", SERVE);
    let revision = json_of(&setup, "tasks")["revision"].as_i64().unwrap();

    let timed_out = setup.priory(&["wait", "--since", &revision.to_string(), "--timeout", "1s"]);
    succeeded(&timed_out);
    assert_eq!(
        String::from_utf8_lossy(&timed_out.stdout),
        format!("{revision}\n")
    );

    let waiter = setup
        .command()
        .args(["wait", "--since", &revision.to_string()])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    succeeded(&setup.priory(&["pause", "lunch"]));
    let changed = Instant::now();
    let output = waiter.wait_with_output().unwrap();

    succeeded(&output);
    assert!(changed.elapsed() < Duration::from_secs(1));
    let after: i64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap();
    assert!(after > revision);
    assert!(!setup.priory(&["wait"]).status.success());
}

/// A report with its sections out of alphabetical order, so a read in any other order shows.
fn report() -> Value {
    json!({
        "date": "2026-11-20",
        "since": "2026-11-19T18:00:00+01:00",
        "greeting": "Bonjour, Max",
        "sections": [
            {
                "id": "sentry",
                "title": "Sentry",
                "count": 1,
                "rows": [{
                    "id": "fnd-2409",
                    "key": "FND-2409",
                    "text": "Release build pauses on a DWARF breakpoint",
                    "meta": "41 events",
                    "state": null,
                    "owner": null,
                    "time": null,
                    "pick": {
                        "id": "2026-11-20_sentry_fnd-2409",
                        "options": ["Take", "Later", "Drop"],
                        "recommended": "Take",
                        "chosen": null,
                        "applied": null
                    },
                    "comments": []
                }]
            },
            {
                "id": "backlog",
                "title": "Backlog",
                "count": null,
                "rows": [
                    { "id": null, "key": null, "text": "Second", "meta": null, "state": null,
                      "owner": null, "time": null, "pick": null, "comments": [] },
                    { "id": null, "key": null, "text": "First", "meta": null, "state": null,
                      "owner": "on Max", "time": "09:00", "pick": null, "comments": [] }
                ]
            }
        ]
    })
}

impl Setup {
    /// Runs `priory report put TASK -` with `report` on stdin.
    fn put(&self, task: &str, report: &Value) -> Output {
        let mut child = self
            .command()
            .args(["report", "put", task, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(report.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn shown(&self, args: &[&str]) -> Value {
        let mut all = vec!["report", "show"];
        all.extend(args);
        all.push("--json");
        let output = self.priory(&all);
        succeeded(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn a_report_put_from_a_file_reads_back_in_its_order_with_its_task_and_revision() {
    let setup = Setup::new("report-put", SERVE);
    setup.seed();
    let file = setup.dir.join("report.json");
    fs::write(&file, report().to_string()).unwrap();

    let put = setup.priory(&["report", "put", "1", file.to_str().unwrap()]);

    succeeded(&put);
    assert_eq!(
        String::from_utf8_lossy(&put.stdout),
        "task 1's report is at revision 1\n"
    );
    let mut expected = report();
    expected["task"] = json!(1);
    expected["revision"] = json!(1);
    assert_eq!(setup.shown(&[])["report"], expected);
    assert_eq!(setup.shown(&["1"])["report"], expected);
}

#[test]
fn a_report_of_a_bad_shape_is_refused_with_why_and_nothing_stored() {
    let setup = Setup::new("report-bad", SERVE);
    setup.seed();
    let mut untitled = report();
    untitled["sections"][1]
        .as_object_mut()
        .unwrap()
        .remove("title");
    let mut stray = report();
    stray["sections"][0]["rows"][0]["pick"]["recommended"] = json!("Maybe");
    let mut commented = report();
    commented["sections"][1]["rows"][0]["comments"] = json!([{ "text": "hi" }]);
    let cases = [
        (untitled, "missing field `title`"),
        (
            stray,
            r#"pick "2026-11-20_sentry_fnd-2409": "Maybe" is not one of its options"#,
        ),
        (commented, r#"section "backlog": comments must be empty"#),
    ];

    for (report, why) in cases {
        let put = setup.put("1", &report);

        assert!(!put.status.success());
        let stderr = String::from_utf8_lossy(&put.stderr);
        assert!(
            stderr.starts_with("priory report put: the report is refused: ")
                && stderr.contains(why),
            "{stderr}"
        );
    }
    let shown = setup.priory(&["report", "show", "--json"]);
    assert!(!shown.status.success());
    assert_eq!(
        String::from_utf8_lossy(&shown.stderr),
        "priory report show: no task has a report\n"
    );
}

#[test]
fn the_report_endpoints_return_what_report_show_json_prints() {
    let mut setup = Setup::new("report-api", SERVE);
    setup.seed();
    succeeded(&setup.put("2", &report()));
    let mut first = report();
    first["greeting"] = json!("Earlier");
    first["sections"][0]["rows"][0]["pick"]["id"] = json!("2026-11-20_sentry_other");
    succeeded(&setup.put("1", &first));
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(api(&address, "/api/reports/latest"), setup.shown(&[]));
    assert_eq!(api(&address, "/api/reports/latest")["report"]["task"], 1);
    assert_eq!(api(&address, "/api/reports/2"), setup.shown(&["2"]));
    assert_eq!(get(&address, "/api/reports/3", Some(TOKEN)).0, 404);
}

#[test]
fn a_report_whose_pick_another_tasks_report_has_is_refused() {
    let setup = Setup::new("report-shared-pick", SERVE);
    setup.seed();
    succeeded(&setup.put("1", &report()));

    let put = setup.put("2", &report());

    assert!(!put.status.success());
    assert_eq!(
        String::from_utf8_lossy(&put.stderr),
        "priory report put: the report is refused: task 1's report already has the pick \"2026-11-20_sentry_fnd-2409\"\n"
    );
    let missing = setup.priory(&["report", "show", "2", "--json"]);
    assert!(!missing.status.success());
}
