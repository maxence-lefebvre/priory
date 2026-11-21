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
            .env("XDG_RUNTIME_DIR", self.tmux.path())
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
                let mut log = fs::File::create(self.dir.join("server.log")).unwrap();
                log.write_all(said.as_bytes()).unwrap();
                std::thread::spawn(move || std::io::copy(&mut stderr, &mut log));
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
        .set_read_timeout(Some(Duration::from_secs(60)))
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
    // `serve` reads the factory, and the queue lists its workflows only when given it too.
    let factory = setup.factory();
    let output = match verb {
        "queue" => setup.priory(&[verb, &factory, "--json"]),
        _ => setup.priory(&[verb, "--json"]),
    };
    succeeded(&output);
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Whether process `pid` holds a listening TCP socket, or `None` when this host's `/proc` shows
/// neither its descriptors nor a TCP table to read them against.
fn listening(pid: u32) -> Option<bool> {
    let sockets: Vec<String> = fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()?
        .filter_map(|fd| fs::read_link(fd.ok()?.path()).ok())
        .filter_map(|link| {
            let link = link.to_string_lossy().into_owned();
            Some(link.strip_prefix("socket:[")?.strip_suffix(']')?.to_owned())
        })
        .collect();
    // A kernel built without IPv6 has no tcp6 table, and so no IPv6 socket to listen on.
    let tables: Vec<String> = ["tcp", "tcp6"]
        .iter()
        .filter_map(|table| fs::read_to_string(format!("/proc/{pid}/net/{table}")).ok())
        .collect();
    if tables.is_empty() {
        return None;
    }
    Some(tables.iter().any(|table| {
        table
            .lines()
            .skip(1)
            .map(|line| line.split_whitespace().collect::<Vec<_>>())
            // State 0A is LISTEN; column 9 is the socket's inode.
            .any(|cells| cells[3] == "0A" && sockets.contains(&cells[9].to_owned()))
    }))
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
        let mut served = api(&address, &format!("/api/{verb}"));
        // The asks and tasks endpoints add the count of rows they left out.
        served.as_object_mut().unwrap().remove("older");
        assert_eq!(served, json_of(&setup, verb), "{verb}");
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
        "id\ttask\trevision\tquestion\toptions\tsnoozed_until\tback_early\ttitle\ttext\twhat_waits\tnext_action\tcommand\tcoordinator\tkey\trun_id\tsession\tmerge_risk\trisk_estimate\n1\t1\t2\tWhich way?\\nPick one.\t[\"left\",\"right\"]\t-\t-\t-\t-\t-\t-\t-\t-\tcli:docs-1\t-\t-\t-\t-\n"
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
        json!({ "asleep": 0, "snoozed": 0, "unrouted": 0, "outsider": 0 })
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

impl Setup {
    /// `seed`'s asks on band `later`, an ask on a third task in band `now`, and an outside ask.
    fn seed_bands(&self) {
        self.seed();
        let factory = self.factory();
        succeeded(&self.priory(&[
            "request",
            &factory,
            "--repo",
            "owner/repo",
            "--workflow",
            "implement",
            "--key",
            "docs-3",
            "--band",
            "now",
            "Fix the index",
        ]));
        succeeded(&self.priory(&["daemon", &factory, "--once"]));
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        db.execute(
            "INSERT INTO asks (task, step, kind, revision, question, options)
             VALUES (3, 'implement', 'agent', 1, 'Now?', '[\"yes\",\"no\"]')",
            [],
        )
        .unwrap();
        let outside = json!({ "title": "Ship it?", "options": ["ship", "wait"], "key": "REL-7" });
        succeeded(&self.priory(&["ask", "review-7", &outside.to_string()]));
    }
}

fn ids(asks: &Value) -> Vec<Value> {
    asks["asks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ask| ask["id"].clone())
        .collect()
}

#[test]
fn asks_band_lists_only_the_asks_of_tasks_in_that_band() {
    let setup = Setup::new("asks-band", "name = \"demo\"\n");
    setup.seed_bands();

    let output = setup.priory(&["asks", "--band", "now", "--json"]);
    succeeded(&output);
    let now: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(ids(&now), vec![json!(2)]);
    assert_eq!(now["asks"][0]["band"], "now");

    let both = setup.priory(&["asks", "--band", "now,later", "--json"]);
    assert_eq!(
        ids(&serde_json::from_slice(&both.stdout).unwrap()),
        vec![json!(1), json!(2)]
    );
    let repeated = setup.priory(&["asks", "--band", "now", "--band", "later", "--json"]);
    assert_eq!(repeated.stdout, both.stdout);
    let text = String::from_utf8(setup.priory(&["asks", "--band", "now"]).stdout).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(
        text.lines().nth(1).unwrap().starts_with("2\t3\t1\tNow?"),
        "{text}"
    );
}

#[test]
fn asks_refuses_an_unknown_band_with_the_factorys_bands() {
    let mut setup = Setup::new("asks-unknown-band", SERVE);
    setup.seed_bands();
    let address = setup.start(&["serve", &setup.factory()]);

    let output = setup.priory(&["asks", "--band", "now,someday"]);
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory asks: band `someday` is not one of blocker, now, today, later\n"
    );
    let (status, body) = get(&address, "/api/asks?band=someday", Some(TOKEN));
    assert_eq!(status, 400);
    assert!(
        body.contains("band `someday` is not one of blocker, now, today, later"),
        "{body}"
    );
}

#[test]
fn the_asks_endpoint_filters_by_band_as_the_cli_does_and_an_outside_ask_has_none() {
    let mut setup = Setup::new("asks-band-api", SERVE);
    setup.seed_bands();
    let address = setup.start(&["serve", &setup.factory()]);

    let cli = setup.priory(&["asks", "--band", "now", "--json"]);
    succeeded(&cli);
    let cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let mut served = api(&address, "/api/asks?band=now&all=1");
    assert_eq!(served["older"], json!({ "answered": 0, "withdrawn": 0 }));
    served.as_object_mut().unwrap().remove("older");
    assert_eq!(served, cli);
    assert_eq!(
        api(&address, "/api/asks?band=now%2Clater"),
        api(&address, "/api/asks?band=now&band=later")
    );

    let all = api(&address, "/api/asks");
    assert_eq!(all["bands"], json!(["blocker", "now", "today", "later"]));
    let outside = &all["asks"][2];
    assert_eq!(
        [&outside["id"], &outside["band"]],
        [&json!("review-7"), &Value::Null]
    );
    assert_eq!(all["asks"][0]["band"], "later");
}

/// Answering `approve-pr` lets `open-pr` run; answering `approve-plan` waits for `approve-pr`.
const GATED: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "approve-plan", failed = "stop" }

[[steps]]
id = "approve-plan"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "approve-pr", rejected = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
"#;

impl Setup {
    /// Tasks 1 to 3 graded red, green and none; asks 1 to 4 and the outside ask `review-7`.
    fn seed_risks(&self) {
        fs::write(self.dir.join("factory/workflows/implement.toml"), GATED).unwrap();
        let factory = self.factory();
        for key in ["docs-1", "docs-2", "docs-3"] {
            succeeded(&self.priory(&[
                "request",
                &factory,
                "--repo",
                "owner/repo",
                "--workflow",
                "implement",
                "--key",
                key,
                "Fix the docs",
            ]));
        }
        succeeded(&self.priory(&["daemon", &factory, "--once"]));
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        db.execute_batch(
            r#"UPDATE tasks SET grade = 'red' WHERE id = 1;
            UPDATE tasks SET grade = 'green' WHERE id = 2;
            INSERT INTO asks (task, step, kind, question, options)
              VALUES (1, 'approve-pr', 'step', 'Open the PR?', '["open","stop"]');
            INSERT INTO asks (task, step, kind, question, options)
              VALUES (2, 'approve-plan', 'step', 'Plan ok?', '["approved","rejected"]');
            INSERT INTO asks (task, step, kind, question, options, command)
              VALUES (3, 'implement', 'agent', 'Deploy?', '["deploy","wait"]',
                '{"argv":["make","deploy"],"target":"worktree","dir":"/tmp","runs":["deploy"]}');
            INSERT INTO asks (task, step, kind, question, options)
              VALUES (3, 'implement', 'agent', 'Which way?', '["left","right"]');"#,
        )
        .unwrap();
        let outside =
            json!({ "title": "Ship it?", "options": ["ship", "wait"], "command": "make ship" });
        succeeded(&self.priory(&["ask", "review-7", &outside.to_string()]));
    }
}

fn risks(asks: &Value) -> Vec<(Value, Value)> {
    asks["asks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ask| (ask["id"].clone(), ask["risk"].clone()))
        .collect()
}

#[test]
fn an_ask_lists_irreversible_when_its_answer_lets_an_irreversible_step_run_and_its_tasks_grade() {
    let setup = Setup::new("asks-risk", "name = \"demo\"\n");
    setup.seed_risks();

    let asks = json_of(&setup, "asks");
    assert_eq!(
        risks(&asks)[..2],
        [
            (json!(1), json!(["irreversible", "red"])),
            (json!(2), json!(["green"]))
        ]
    );
}

#[test]
fn a_command_ask_is_irreversible_and_an_outside_ask_has_no_risk() {
    let setup = Setup::new("asks-risk-command", "name = \"demo\"\n");
    setup.seed_risks();

    let asks = json_of(&setup, "asks");
    assert_eq!(
        risks(&asks)[2..],
        [
            (json!(3), json!(["irreversible"])),
            (json!(4), json!([])),
            (json!("review-7"), json!([]))
        ]
    );
}

#[test]
fn asks_risk_lists_only_the_asks_with_that_risk_and_none_lists_those_with_neither() {
    let setup = Setup::new("asks-risk-filter", "name = \"demo\"\n");
    setup.seed_risks();
    let listed = |args: &[&str]| {
        let output = setup.priory(&[&["asks"], args, &["--json"]].concat());
        succeeded(&output);
        ids(&serde_json::from_slice(&output.stdout).unwrap())
    };

    assert_eq!(listed(&["--risk", "none"]), [json!(4), json!("review-7")]);
    assert_eq!(listed(&["--risk", "irreversible"]), [json!(1), json!(3)]);
    assert_eq!(
        listed(&["--risk", "green,none"]),
        [json!(2), json!(4), json!("review-7")]
    );
    assert_eq!(
        listed(&["--risk", "green", "--risk", "none"]),
        listed(&["--risk", "green,none"])
    );
    let output = setup.priory(&["asks", "--risk", "grey"]);
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "priory asks: risk `grey` is not one of irreversible, high, medium, low, red, yellow, green, none\n"
    );
}

#[test]
fn the_asks_endpoint_filters_by_risk_as_the_cli_does() {
    let mut setup = Setup::new("asks-risk-api", SERVE);
    setup.seed_risks();
    let address = setup.start(&["serve", &setup.factory()]);

    let cli = setup.priory(&["asks", "--risk", "red", "--json"]);
    succeeded(&cli);
    let cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(ids(&cli), [json!(1)]);
    let mut served = api(&address, "/api/asks?risk=red&all=1");
    served.as_object_mut().unwrap().remove("older");
    assert_eq!(served, cli);
    assert_eq!(
        api(&address, "/api/asks?risk=green%2Cnone"),
        api(&address, "/api/asks?risk=green&risk=none")
    );
    let (status, body) = get(&address, "/api/asks?risk=grey", Some(TOKEN));
    assert_eq!(status, 400);
    assert!(body.contains("risk `grey` is not one of"), "{body}");

    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "UPDATE asks SET answer = 'open', answered = '2026-01-01T00:00:00Z' WHERE id = 1",
        [],
    )
    .unwrap();
    let red = api(&address, "/api/asks?risk=red");
    assert_eq!((ids(&red), &red["older"]["answered"]), (vec![], &json!(1)));
    assert_eq!(
        api(&address, "/api/asks?risk=green")["older"]["answered"],
        0
    );
}

impl Setup {
    /// `seed_risks`, with each ask asked at its own minute and task 2 blocked last.
    fn seed_ranks(&self) {
        self.seed_risks();
        let db = rusqlite::Connection::open(self.dir.join("priory.db")).unwrap();
        db.execute_batch(
            "UPDATE asks SET created = CASE id WHEN 1 THEN '2026-11-17T10:05:00Z'
               WHEN 2 THEN '2026-11-17T10:01:00Z' WHEN 3 THEN '2026-11-17T10:04:00Z'
               WHEN 4 THEN '2026-11-17T10:00:00Z' ELSE '2026-11-17T10:02:00Z' END;
             UPDATE tasks SET state = 'blocked', reason = 'tests fail',
               changed = '2026-11-17T11:00:00Z' WHERE id = 2;",
        )
        .unwrap();
    }
}

fn ranked(desk: &Value) -> Vec<(Value, Value)> {
    desk["ranked"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["kind"].clone(), item["id"].clone()))
        .collect()
}

#[test]
fn the_desk_ranks_alarms_then_irreversible_asks_then_plans_then_the_rest_each_oldest_first() {
    let setup = Setup::new("desk-rank", "name = \"demo\"\n");
    setup.seed_ranks();

    let desk = json_of(&setup, "desk");

    assert_eq!(
        ranked(&desk),
        [
            (json!("blocked"), json!(2)),
            (json!("ask"), json!(3)),
            (json!("ask"), json!(1)),
            (json!("ask"), json!(2)),
            (json!("ask"), json!(4)),
            (json!("ask"), json!("review-7")),
        ]
    );
    assert_eq!(desk["next"]["kind"], "blocked");
    assert_eq!(
        desk["next"]["why"],
        "an alarm: work stopped and waits on a person"
    );
    assert_eq!(desk["ranked"][2]["why"], "it lets step `open-pr` run");
    assert_eq!(desk["ranked"][3]["why"], "it approves a plan");
}

#[test]
fn a_command_ask_says_what_it_runs_and_where() {
    let setup = Setup::new("desk-why-command", "name = \"demo\"\n");
    setup.seed_ranks();

    let desk = json_of(&setup, "desk");

    assert_eq!(desk["ranked"][1]["id"], 3);
    assert_eq!(desk["ranked"][1]["why"], "it runs `make` in worktree");
}

#[test]
fn an_approval_recorded_before_the_step_was_kept_keeps_the_general_why() {
    let setup = Setup::new("desk-why-upgraded", "name = \"demo\"\n");
    setup.seed_ranks();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute("UPDATE irreversible_asks SET lets = ''", [])
        .unwrap();

    let desk = json_of(&setup, "desk");

    assert_eq!(desk["ranked"][2]["id"], 1);
    assert_eq!(
        desk["ranked"][2]["why"],
        "its answer lets an irreversible step run"
    );
}

#[test]
fn an_ask_that_reviews_a_pull_request_ranks_with_the_irreversible_ones() {
    let setup = Setup::new("desk-rank-review", "name = \"demo\"\n");
    setup.seed_ranks();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "UPDATE tasks SET key = 'github:owner/repo:pulls/43:review:octocat' WHERE id = 3",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE asks SET created = '2026-11-17T10:03:00Z' WHERE id = 4",
        [],
    )
    .unwrap();

    let desk = json_of(&setup, "desk");

    assert_eq!(
        ranked(&desk)[1..4],
        [
            (json!("ask"), json!(4)),
            (json!("ask"), json!(3)),
            (json!("ask"), json!(1)),
        ]
    );
    assert_eq!(desk["ranked"][1]["why"], "it reviews a pull request");
}

#[test]
fn a_snoozed_item_never_ranks_even_when_the_desk_shows_it() {
    let setup = Setup::new("desk-rank-snoozed", "name = \"demo\"\n");
    setup.seed_ranks();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "INSERT INTO snoozes (kind, item, task, snoozed, until)
           VALUES ('task', 2, 2, '2026-11-17T11:00:00Z', '2999-01-01T00:00:00Z'),
                  ('ask', 3, 3, '2026-11-17T11:00:00Z', '2999-01-01T00:00:00Z');",
    )
    .unwrap();

    let output = setup.priory(&["desk", "--json", "--all"]);
    succeeded(&output);
    let desk: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(
        ranked(&desk)[..2],
        [(json!("ask"), json!(1)), (json!("ask"), json!(2)),]
    );
    assert_eq!(desk["next"]["id"], 1);
    assert_eq!(desk["ranked"].as_array().unwrap().len(), 4);
}

#[test]
fn a_failing_intake_ranks_as_an_alarm_whose_title_names_the_source_not_the_error() {
    let setup = Setup::new("desk-rank-intake", "name = \"demo\"\n");
    setup.seed_ranks();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "INSERT INTO inbox (seq, id, time, type, subject, data, links, outcome)
         VALUES (900, 'r900', '2026-11-17T09:00:00Z', 'priory.report', 'intake',
           '{\"line\":\"github: HTTP 401: token abc\"}', '[]', '')",
        [],
    )
    .unwrap();

    let desk = json_of(&setup, "desk");

    assert_eq!(
        (
            &desk["next"]["kind"],
            &desk["next"]["title"],
            &desk["next"]["why"]
        ),
        (
            &json!("report"),
            &json!("the intake fails to poll github"),
            &json!("an alarm: work stopped and waits on a person")
        )
    );
}

#[test]
fn the_desk_endpoints_next_is_the_clis() {
    let mut setup = Setup::new("desk-rank-api", SERVE);
    setup.seed_ranks();
    let address = setup.start(&["serve", &setup.factory()]);

    let served = api(&address, "/api/desk");

    assert_eq!(served["next"], json_of(&setup, "desk")["next"]);
    assert_eq!(
        (&served["next"]["kind"], &served["next"]["title"]),
        (&json!("blocked"), &json!("task cli:docs-2 blocked"))
    );
}

#[test]
fn the_desk_endpoint_lists_since_you_looked_for_a_person_as_the_cli_does() {
    let mut setup = Setup::new("looked", SERVE);
    setup.seed();
    let before = json_of(&setup, "desk")["revision"].clone();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "INSERT INTO trace (id, type, time, event) VALUES ('e1', 'factory.step.ended',
           '2999-01-01T00:00:00.000Z',
           '{\"data\":{\"task\":\"1\",\"step\":\"implement\",\"outcome\":\"passed\"}}')",
        [],
    )
    .unwrap();
    assert_ne!(json_of(&setup, "desk")["revision"], before);
    succeeded(&setup.priory(&["seen", "--person", "Ana Li", "--at", "2998-01-01T00:00:00Z"]));
    let address = setup.start(&["serve", &setup.factory()]);

    let mine = api(&address, "/api/desk");
    let ana = api(&address, "/api/desk?person=Ana%20Li");

    assert_eq!(mine, json_of(&setup, "desk"));
    assert!(mine["since_you_looked"].as_array().unwrap().len() > 1);
    let cli: Value = serde_json::from_slice(
        &setup
            .priory(&["desk", "--json", "--person", "Ana Li"])
            .stdout,
    )
    .unwrap();
    assert_eq!(ana, cli);
    assert_eq!(
        ana["since_you_looked"],
        json!([{ "time": "2999-01-01T00:00:00.000Z", "task": 1, "key": "cli:docs-1",
                 "what": "factory.step.ended implement: passed" }])
    );
    for path in [
        "/api/desk?person=%zz",
        "/api/desk?person=%2",
        "/api/desk?who=me",
        "/api/desk?person=Ana%20Li&who=me",
        "/api/desk?person=a&person=b",
    ] {
        assert_eq!(get(&address, path, Some(TOKEN)).0, 400, "{path}");
    }
}

#[test]
fn seen_through_its_endpoint_records_the_visit_as_the_cli_verb_does() {
    let mut setup = Setup::new("web-seen", SERVE);
    setup.seed();
    let address = setup.start(&["daemon", &setup.factory()]);
    let body = r#"{"at": "2998-01-01T00:00:00.250+01:00"}"#;

    let foreign = post(
        &address,
        "/api/seen",
        "Origin: http://evil.example\r\n",
        body,
    );
    let bad = post(&address, "/api/seen", "", r#"{"at": "yesterday"}"#);
    let extra = post(
        &address,
        "/api/seen",
        "",
        r#"{"at": "2998-01-01T00:00:00Z", "person": "Ana"}"#,
    );

    assert_eq!(foreign.0, 403, "{}", foreign.1);
    assert_eq!(bad.0, 400, "{}", bad.1);
    assert_eq!(extra.0, 400, "{}", extra.1);
    assert!(json_of(&setup, "desk")["last_visit"].is_null());
    let seen = post(&address, "/api/seen", "", body);
    assert_eq!(seen.0, 200, "{}", seen.1);
    assert_eq!(
        json_of(&setup, "desk")["last_visit"],
        "2997-12-31T23:00:00Z"
    );
}

#[test]
fn the_asks_endpoint_leaves_out_asks_closed_over_a_week_ago_unless_asked_for_all() {
    let mut setup = Setup::new("asks-week", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "INSERT INTO asks (task, step, kind, question, options, answer, answered, created)
           VALUES (1, 'implement', 'agent', 'Old?', '[]', 'yes', '2026-11-01T09:00:00Z',
             '2026-11-01T08:00:00Z'),
           (1, 'implement', 'agent', 'Recent?', '[]', 'no', '2026-11-15T09:00:00Z',
             '2026-11-01T08:00:00Z');
         INSERT INTO asks (task, step, kind, question, options, withdrawn, withdrawn_reason, created)
           VALUES (1, 'implement', 'agent', 'Gone?', '[]', 1, 'dismissed', '2026-11-02T08:00:00Z');
         INSERT INTO asks (task, step, kind, question, options, created)
           VALUES (1, 'implement', 'agent', 'Lately gone?', '[]', '2026-11-02T08:00:00Z');
         UPDATE asks SET withdrawn = 1, withdrawn_reason = 'dismissed' WHERE id = 5;",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);
    let ids = |asks: &Value| {
        asks["asks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|ask| ask["id"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };

    let recent = api(&address, "/api/asks");
    let all = api(&address, "/api/asks?all=1");

    // An ask withdrawn now ages from its withdrawal, however old it is.
    assert_eq!(ids(&recent), [1, 3, 5]);
    assert!(recent["asks"][2]["withdrawn_at"].is_string(), "{recent}");
    assert_eq!(recent["older"], json!({ "answered": 1, "withdrawn": 1 }));
    assert_eq!(ids(&all), [1, 2, 3, 4, 5]);
    assert_eq!(all["older"], json!({ "answered": 0, "withdrawn": 0 }));
    assert_eq!(all["asks"], json_of(&setup, "asks")["asks"]);
    assert_eq!(get(&address, "/api/asks?all=yes", Some(TOKEN)).0, 400);
}

#[test]
fn the_tasks_endpoint_leaves_out_tasks_ended_over_a_week_ago_unless_asked_for_all() {
    let mut setup = Setup::new("tasks-week", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "UPDATE tasks SET state = 'done' WHERE id = 1;
         UPDATE tasks SET changed = '2026-11-01T09:00:00Z' WHERE id = 1;",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);
    let ids = |tasks: &Value| {
        tasks["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| task["id"].as_i64().unwrap())
            .collect::<Vec<_>>()
    };

    let recent = api(&address, "/api/tasks");
    let all = api(&address, "/api/tasks?all=1");

    assert_eq!(ids(&recent), [2]);
    assert_eq!(recent["older"], 1);
    assert_eq!(ids(&all), [1, 2]);
    assert_eq!(all["older"], 0);
    assert_eq!(all["tasks"], json_of(&setup, "tasks")["tasks"]);
    assert_eq!(get(&address, "/api/tasks?lane=x", Some(TOKEN)).0, 400);
}

#[test]
fn the_tasks_endpoint_gives_each_task_its_title_and_batch() {
    let mut setup = Setup::new("tasks-title", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute("UPDATE tasks SET batch = 4 WHERE id = 2", [])
        .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let tasks = api(&address, "/api/tasks");
    let shown = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| (task["title"].clone(), task["batch"].clone()))
        .collect::<Vec<_>>();

    assert_eq!(
        shown,
        [
            (json!("Fix the docs"), Value::Null),
            (json!("Fix the"), json!(4))
        ]
    );
}

#[test]
fn a_task_endpoint_names_the_batch_of_a_batch_entry() {
    let mut setup = Setup::new("task-batch", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute("UPDATE tasks SET batch = 4 WHERE id = 2", [])
        .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(api(&address, "/api/tasks/2")["batch"], 4);
    assert_eq!(api(&address, "/api/tasks/1")["batch"], Value::Null);
}

#[test]
fn the_runs_endpoint_lists_the_runs_of_the_last_week_newest_first_with_outcome_model_and_tokens() {
    let mut setup = Setup::new("runs", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "DELETE FROM runs WHERE NOT EXISTS (SELECT 1 FROM results WHERE results.run = runs.id);
         INSERT INTO runs (id, task, step, tmux, session, agent, started, ended, ended_as) VALUES
           (91, 1, 'implement', 't', 's91', 'implementer', '2026-11-01T09:00:00Z', '2026-11-01T09:30:00Z', 'moved'),
           (92, 1, 'implement', 't', 's92', 'implementer', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-2 hours'),
             strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hours'), 'done'),
           (93, 2, 'implement', 't', 's93', 'implementer', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-10 minutes'), NULL, NULL);
         INSERT INTO results (run, task, step, outcome, summary) VALUES (92, 1, 'implement', 'passed', 'Fixed');
         INSERT INTO trace (id, type, time, event) VALUES ('e92', 'factory.step.ended', strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 hours'),
           json_object('data', json_object('run', '92', 'model', 'claude-opus-5-5',
             'tokens', json_object('input', 1200, 'output', 300))));",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let runs = api(&address, "/api/runs");
    let shown = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            json!([
                run["run"],
                run["key"],
                run["agent"],
                run["end"],
                run["outcome"],
                run["model"],
                run["tokens"]
            ])
        })
        .collect::<Vec<_>>();

    assert_eq!(
        shown,
        [
            json!([93, "cli:docs-2", "implementer", null, null, null, null]),
            json!([92, "cli:docs-1", "implementer", "done", "passed", "claude-opus-5-5", { "input": 1200, "output": 300 }]),
        ]
    );
    assert_eq!(runs["older"], 1);
    assert!(runs["revision"].is_i64());
    let all = api(&address, "/api/runs?all=1");
    assert_eq!(all["runs"].as_array().unwrap().len(), 3);
    let output = setup.priory(&["runs", "--json"]);
    succeeded(&output);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        all
    );
}

#[test]
fn the_tasks_endpoint_finds_each_active_task_holding_a_pull_request_and_how_it_holds_it() {
    let mut setup = Setup::new("tasks-pr", SERVE);
    let db = setup.db();
    let task = |id: i64, key: &str, state: &str| {
        db.execute(
            "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source)
             VALUES (?1, ?2, 'implement', 'implement', ?3, 'none/misc', 'cli', 'cli-' || ?1)",
            rusqlite::params![id, key, state],
        )
        .unwrap();
    };
    task(1, "github:owner/repo#7", "asleep");
    task(9, "cli:docs-9", "running");
    task(3, "cli:docs-3", "asleep");
    task(4, "cli:docs-4", "ready");
    task(5, "github:owner/repo:pulls/7:review:ana", "asleep");
    task(6, "cli:docs-6", "done");
    task(7, "github:owner/repo#70", "asleep");
    task(8, "github:owner/repo:pulls/70:review:ana", "asleep");
    task(10, "cli:docs-10", "asleep");
    db.execute_batch(
        "INSERT INTO owns (task, resource) VALUES (1, 'github:owner/repo#7'),
           (9, 'github:owner/repo#7'), (3, 'github:owner/repo#7'), (7, 'github:owner/repo#70'),
           (10, 'github:owner/repo#7');
         INSERT INTO pull_requests (task, resource, branch, base)
           VALUES (9, 'github:owner/repo#7', 'priory/9', 'main');
         INSERT INTO runs (task, step, tmux, session) VALUES (3, 'implement', 'priory-3', 's3');
         INSERT INTO waits (task, run, kind, resource) VALUES
           (3, (SELECT max(id) FROM runs), 'review', 'github:owner/repo#7');
         INSERT INTO subscriptions (task, resource) VALUES (1, 'github:owner/repo#7'),
           (4, 'github:owner/repo#7'), (6, 'github:owner/repo#7'), (7, 'github:owner/repo#70');",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let found = api(&address, "/api/tasks?pr=github%3Aowner%2Frepo%237");

    let holds: Vec<(i64, Value, Value)> = found["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            (
                task["id"].as_i64().unwrap(),
                task["state"].clone(),
                task["holds"].clone(),
            )
        })
        .collect();
    assert_eq!(
        holds,
        [
            (1, json!("asleep"), json!(["key", "subscription"])),
            (9, json!("running"), json!(["stack"])),
            (3, json!("asleep"), json!(["wait"])),
            (4, json!("ready"), json!(["subscription"])),
            (5, json!("asleep"), json!(["review"])),
            (10, json!("asleep"), json!(["owns"])),
        ]
    );
    assert!(found["revision"].is_i64(), "{found}");
    assert_eq!(
        api(&address, "/api/tasks?pr=github%3Aowner%2Frepo%23999")["tasks"],
        json!([])
    );
    for bad in [
        "linear:ENG-7",
        "github%3Aowner%237",
        "github%3Aowner%2Frepo%23x",
    ] {
        let (status, body) = get(&address, &format!("/api/tasks?pr={bad}"), Some(TOKEN));
        assert_eq!(status, 400, "{bad}: {body}");
    }
}

#[test]
fn the_tasks_endpoint_finds_the_holders_of_a_pull_request_in_any_case_of_its_repository() {
    let mut setup = Setup::new("tasks-pr-case", SERVE);
    let db = setup.db();
    db.execute_batch(
        "INSERT INTO tasks (id, key, workflow, step, state, lane, trigger, source) VALUES
           (1, 'github:owner/repo#7', 'implement', 'implement', 'asleep', 'none/misc', 'cli', 'cli-1'),
           (2, 'cli:docs-2', 'implement', 'implement', 'asleep', 'none/misc', 'cli', 'cli-2'),
           (3, 'github:owner/repo:pulls/7:review:ana', 'implement', 'implement', 'asleep', 'none/misc', 'cli', 'cli-3'),
           (4, 'cli:docs-4', 'implement', 'implement', 'asleep', 'none/misc', 'cli', 'cli-4');
         INSERT INTO pull_requests (task, resource, branch, base)
           VALUES (2, 'github:owner/repo#7', 'priory/2', 'main');
         INSERT INTO owns (task, resource) VALUES (4, 'github:owner/repo#7');",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let found = api(&address, "/api/tasks?pr=github%3AOwner%2FRepo%237");

    let holds: Vec<(Value, Value)> = found["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| (task["id"].clone(), task["holds"].clone()))
        .collect();
    assert_eq!(
        holds,
        [
            (json!(1), json!(["key"])),
            (json!(2), json!(["stack"])),
            (json!(3), json!(["review"])),
            (json!(4), json!(["owns"])),
        ]
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
fn task_json_prints_the_task_endpoints_body() {
    let mut setup = Setup::new("task-json", SERVE);
    setup.seed();
    let address = setup.start(&["serve", &setup.factory()]);

    let output = setup.priory(&["task", "1", "--json"]);
    succeeded(&output);
    let printed: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(printed, api(&address, "/api/tasks/1"));
    assert_eq!(printed["id"], 1);
    let missing = setup.priory(&["task", "9", "--json"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no task 9"));
}

#[test]
fn a_task_endpoint_shows_its_brief_plan_pull_request_and_each_runs_tokens() {
    let mut setup = Setup::new("task-detail", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    let tokens = r#"{"input":120,"output":30,"cache_read":0,"cache_write":0}"#;
    for (run, step, summary) in [
        (1, "groom", "Brief: fix the docs."),
        (2, "plan", "1. Old plan."),
        (3, "plan", "1. Edit the README.\n2. Test."),
        (4, "implement", "Implemented."),
    ] {
        db.execute(
            "INSERT INTO runs (id, task, step, tmux, session, started, ended, ended_as)
             VALUES (?1, 1, ?2, 'tmux', 's', '2026-11-17T10:00:00Z', '2026-11-17T10:30:00Z', 'moved')",
            rusqlite::params![run, step],
        )
        .unwrap();
        db.execute(
            "INSERT INTO results (run, task, step, outcome, summary) VALUES (?1, 1, ?2, 'passed', ?3)",
            rusqlite::params![run, step, summary],
        )
        .unwrap();
    }
    let ended = json!({ "type": "factory.step.ended",
        "data": { "task": "1", "run": "3", "step": "plan", "tokens": serde_json::from_str::<Value>(tokens).unwrap() } });
    db.execute(
        "INSERT INTO trace (id, type, time, event) VALUES ('e1', 'factory.step.ended', '2026-11-17T10:30:00Z', ?1)",
        [ended.to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO owns (task, resource) VALUES (1, 'github:owner/repo#43')",
        [],
    )
    .unwrap();
    let pr = |name: &str, conclusion: &str| {
        json!({ "repo": "owner/repo", "pull_request": 43, "head_sha": "abc", "head_branch": "docs-1",
            "base_branch": "main", "url": "https://github.com/owner/repo/pull/43",
            "title": "Fix the docs", "name": name, "conclusion": conclusion, "state": "approved" })
    };
    for (seq, kind, data) in [
        (1, "github.pull_request.opened", pr("", "")),
        (2, "github.check_run.completed", pr("test", "failure")),
        (3, "github.check_run.completed", pr("test", "success")),
        (4, "github.check_run.completed", pr("lint", "success")),
        // A review the intake read without the pull request's branches and head.
        (5, "github.pull_request_review.submitted", {
            let mut review = pr("", "");
            review["head_branch"] = Value::Null;
            review["base_branch"] = Value::Null;
            review["head_sha"] = Value::Null;
            review
        }),
        // A check run on the commit before the head, completing late.
        (6, "github.check_run.completed", {
            let mut late = pr("test", "failure");
            late["head_sha"] = json!("old");
            late
        }),
    ] {
        db.execute(
            "INSERT INTO inbox (id, type, subject, time, data, links, outcome)
             VALUES (?1, ?2, 'owner/repo#43', ?3, ?4, '[]', 'ignored')",
            rusqlite::params![
                format!("pr-{seq}"),
                kind,
                format!("2026-11-17T11:0{seq}:00Z"),
                data.to_string()
            ],
        )
        .unwrap();
    }
    let address = setup.start(&["serve", &setup.factory()]);

    let task = api(&address, "/api/tasks/1");

    assert_eq!(task["brief"]["request"], "Fix the docs");
    assert_eq!(task["brief"]["result"]["summary"], "Brief: fix the docs.");
    assert_eq!(task["brief"]["result"]["run"], 1);
    assert_eq!(task["plan"]["summary"], "1. Edit the README.\n2. Test.");
    assert_eq!(task["plan"]["run"], 3);
    let pull = &task["pull_request"];
    assert_eq!(pull["number"], 43);
    assert_eq!(pull["repo"], "owner/repo");
    assert_eq!(pull["title"], "Fix the docs");
    assert_eq!(pull["state"], "open");
    assert_eq!(pull["head_sha"], "abc");
    assert_eq!(pull["branch"], "docs-1");
    assert_eq!(pull["base"], "main");
    assert_eq!(pull["review"], "approved");
    assert_eq!(
        pull["checks"],
        json!([
            { "name": "lint", "conclusion": "success", "at": "2026-11-17T11:04:00Z" },
            { "name": "test", "conclusion": "success", "at": "2026-11-17T11:03:00Z" }
        ])
    );
    let run = &task["runs"][2];
    assert_eq!(run["step"], "plan");
    assert_eq!(run["outcome"], "passed");
    assert_eq!(
        run["tokens"],
        serde_json::from_str::<Value>(tokens).unwrap()
    );
    assert_eq!(task["runs"][0]["tokens"], Value::Null);
}

#[test]
fn a_task_without_a_groom_or_plan_step_or_pull_request_shows_none() {
    let mut setup = Setup::new("task-detail-empty", SERVE);
    setup.seed();
    let address = setup.start(&["serve", &setup.factory()]);

    let task = api(&address, "/api/tasks/2");

    assert_eq!(task["brief"]["request"], "Fix the\nreadme");
    assert_eq!(task["brief"]["result"], Value::Null);
    assert_eq!(task["plan"], Value::Null);
    assert_eq!(task["pull_request"], Value::Null);
}

#[test]
fn a_task_endpoint_shows_the_output_its_run_step_kept() {
    let mut setup = Setup::new("task-run-step", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "INSERT INTO actions (task, step, action, status, error, stdout, stderr)
         VALUES (1, 'poll', 'run', 'failed', 'exit 2', 'checking', 'no network')",
        [],
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let step = &api(&address, "/api/tasks/1")["steps"][0];

    assert_eq!(step["action"], "run");
    assert_eq!(step["error"], "exit 2");
    assert_eq!(step["stdout"], "checking");
    assert_eq!(step["stderr"], "no network");
}

#[test]
fn the_task_endpoint_and_the_tasks_listing_carry_the_tasks_relations() {
    let mut setup = Setup::new("relations", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["relate", "2", "builds-on", "1"]));
    let address = setup.start(&["serve", &setup.factory()]);

    let relations = api(&address, "/api/tasks/1")["relations"].clone();

    assert_eq!(relations.as_array().map(Vec::len), Some(1), "{relations}");
    assert_eq!(relations[0]["task"], 2);
    assert_eq!(relations[0]["kind"], "builds-on");
    assert_eq!(relations[0]["other"], 1);
    assert_eq!(relations, json_of(&setup, "tasks")["tasks"][0]["relations"]);
    assert_eq!(relations, api(&address, "/api/tasks/2")["relations"]);
}

#[test]
fn progress_stores_the_four_fields_that_the_task_endpoint_and_the_tasks_listing_show() {
    let mut setup = Setup::new("progress", SERVE);
    setup.seed();
    let address = setup.start(&["daemon", &setup.factory()]);
    let progress = json!({
        "goal": "Fix the docs",
        "acceptanceCriteria": [
            { "id": "PF1", "description": "cargo test passes", "required": true },
            { "id": "PF2", "description": "a screenshot", "required": false }
        ],
        "latestChange": "Rewrote the intro",
        "nextAction": "Open the PR"
    });

    succeeded(&setup.priory(&["progress", "1", &progress.to_string()]));

    assert_eq!(api(&address, "/api/tasks/1")["progress"], progress);
    assert_eq!(
        api(&address, "/api/tasks")["tasks"][0]["progress"],
        progress
    );
    assert_eq!(json_of(&setup, "tasks")["tasks"][0]["progress"], progress);
}

#[test]
fn a_second_progress_call_with_one_field_keeps_the_others_and_the_trace_records_each_change() {
    let mut setup = Setup::new("progress-merge", SERVE);
    setup.seed();
    let address = setup.start(&["daemon", &setup.factory()]);
    let first = json!({
        "goal": "Fix the docs",
        "acceptanceCriteria": [{ "id": "PF1", "description": "tests pass", "required": true }],
        "latestChange": "Rewrote the intro",
        "nextAction": "Open the PR"
    });
    succeeded(&setup.priory(&["progress", "1", &first.to_string()]));

    succeeded(&setup.priory(&["progress", "1", r#"{"nextAction": "Wait for CI"}"#]));

    let mut merged = first.clone();
    merged["nextAction"] = json!("Wait for CI");
    assert_eq!(api(&address, "/api/tasks/1")["progress"], merged);
    let trace = String::from_utf8(setup.priory(&["trace"]).stdout).unwrap();
    let recorded: Vec<Value> = trace
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|event| event["type"] == "factory.task.progressed")
        .map(|event| {
            assert_eq!(event["data"]["task"], "1");
            event["data"]["progress"].clone()
        })
        .collect();
    assert_eq!(recorded, [first, merged]);
}

#[test]
fn progress_refuses_a_bad_acceptance_criteria_shape_and_stores_nothing() {
    let mut setup = Setup::new("progress-shape", SERVE);
    setup.seed();
    let address = setup.start(&["daemon", &setup.factory()]);

    let refused = setup.priory(&[
        "progress",
        "1",
        r#"{"goal": "Fix the docs", "acceptanceCriteria": [{"id": "PF1", "description": "tests pass"}]}"#,
    ]);

    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(error.contains("missing field `required`"), "{error}");
    for positional in [
        r#"{"acceptanceCriteria": [["PF1", "tests pass", true]]}"#,
        r#"["Fix the docs"]"#,
    ] {
        let refused = setup.priory(&["progress", "1", positional]);
        assert!(!refused.status.success(), "{positional}: {refused:?}");
    }
    assert_eq!(api(&address, "/api/tasks/1")["progress"], Value::Null);
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
fn a_target_without_its_leading_slash_gets_400_and_leaves_every_worker_serving() {
    let mut setup = Setup::new("target", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);

    for target in ["é", "x", "é", "x", "é", "x", "é", "x", "é"] {
        let head = format!("GET {target} HTTP/1.1\r\nHost: {address}\r\n\r\n");
        assert_eq!(raw(&address, &head).0, 400, "{target}");
    }

    assert_eq!(get(&address, "/api/tasks", Some(TOKEN)).0, 200);
    assert_eq!(get(&address, "/app.css", None).0, 200);
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
fn eight_silent_clients_delay_a_request_by_about_a_second_not_the_whole_read_timeout() {
    let mut setup = Setup::new("silent-eight", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let _silent: Vec<_> = (0..8)
        .map(|_| TcpStream::connect(&address).unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    let started = Instant::now();

    assert_eq!(get(&address, "/app.css", None).0, 200);

    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn eight_clients_that_send_one_byte_and_stall_do_not_delay_a_request() {
    let mut setup = Setup::new("stalled-eight", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let _stalled: Vec<_> = (0..8)
        .map(|_| {
            let mut stream = TcpStream::connect(&address).unwrap();
            stream.write_all(b"G").unwrap();
            stream
        })
        .collect();
    std::thread::sleep(Duration::from_millis(1100));
    let started = Instant::now();

    assert_eq!(get(&address, "/api/tasks", Some(TOKEN)).0, 200);

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_head_whose_first_byte_comes_at_once_has_the_whole_read_timeout_to_finish() {
    let mut setup = Setup::new("slow-head", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let mut stream = TcpStream::connect(&address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();

    stream.write_all(b"G").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    stream
        .write_all(format!("ET /app.css HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
        .unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

#[test]
fn the_daemon_serves_its_state_when_the_factory_has_a_serve_table() {
    let mut setup = Setup::new("daemon", SERVE);
    setup.seed();

    let address = setup.start(&["daemon", &setup.factory()]);

    // The running daemon may write a task's reason between the two reads: compare until they agree.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let served = api(&address, "/api/tasks")["tasks"].clone();
        let listed = json_of(&setup, "tasks")["tasks"].clone();
        if served == listed {
            break;
        }
        assert!(Instant::now() < deadline, "{served} != {listed}");
        std::thread::sleep(Duration::from_millis(100));
    }
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
    let deadline = Instant::now() + Duration::from_secs(10);
    // The daemon's hook socket shows it is past its start.
    let socket =
        priory_runtime::runtime::socket_in(Some(setup.tmux.path()), &setup.dir.join("home"));
    while !socket.exists() {
        assert!(Instant::now() < deadline, "the daemon did not start");
        std::thread::sleep(Duration::from_millis(50));
    }

    match listening(pid) {
        Some(listens) => assert!(!listens),
        None => eprintln!(
            "skipped: this host's /proc shows no descriptors or TCP table for process {pid}"
        ),
    }
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
fn the_queue_and_tasks_list_each_condition_of_a_hold_and_whether_it_is_met() {
    let mut setup = Setup::new("held", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "UPDATE tasks SET state = 'held', reason = 'held: b, until tomorrow and until task 2 ends'
           WHERE id = 1;
         INSERT INTO holds (task, reason, kind, until, target, resume, met, met_as) VALUES
           (1, 'b', 'time', '2099-01-01T09:00:00Z', NULL, 'queued', NULL, NULL),
           (1, 'b', 'task', NULL, 2, 'queued', '2026-11-20T10:00:00Z', 'task 2 is done');",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);
    let conditions = json!([
        { "kind": "time", "until": "2099-01-01T09:00:00Z", "task": null, "resource": null,
          "met": null, "met_as": null },
        { "kind": "task", "until": null, "task": 2, "resource": null,
          "met": "2026-11-20T10:00:00Z", "met_as": "task 2 is done" },
    ]);

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
    assert_eq!(waiting[1]["conditions"], conditions);
    assert_eq!(waiting[0]["conditions"], Value::Null);
    assert_eq!(api(&address, "/api/tasks/1")["conditions"], conditions);
    let tasks = json_of(&setup, "tasks");
    let held = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["id"] == 1);
    assert_eq!(held.unwrap()["conditions"], conditions);
    assert_eq!(api(&address, "/api/tasks/2")["position"], 1);
}

#[test]
fn the_queue_lists_each_asleep_task_with_what_it_waits_on_and_since_when() {
    let mut setup = Setup::new("queue-asleep", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "UPDATE tasks SET state = 'asleep' WHERE id = 1;
         UPDATE tasks SET changed = '2026-11-17T10:00:00Z' WHERE id = 1;
         INSERT INTO runs (task, step, tmux, session, ended, ended_as)
           VALUES (1, 'implement', 'priory-1', 's1', '2026-11-17T10:00:00Z', 'asleep');
         INSERT INTO waits (task, run, kind, resource, until, created) VALUES
           (1, (SELECT max(id) FROM runs WHERE task = 1), 'review', 'github:owner/repo#7', NULL,
            '2026-11-17T10:00:00Z'),
           (1, (SELECT max(id) FROM runs WHERE task = 1), 'time', NULL, '2026-11-18T09:00:00Z',
            '2026-11-17T10:01:00Z');",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let asleep = api(&address, "/api/queue")["asleep"].clone();

    assert_eq!(asleep, json_of(&setup, "queue")["asleep"]);
    assert_eq!(asleep.as_array().unwrap().len(), 1, "{asleep}");
    let task = &asleep[0];
    assert_eq!(
        [&task["id"], &task["key"], &task["title"], &task["since"]],
        [
            &json!(1),
            &json!("cli:docs-1"),
            &json!("Fix the docs"),
            &json!("2026-11-17T10:00:00Z")
        ]
    );
    assert_eq!(
        task["waits"],
        json!([
            { "kind": "review", "on": "github:owner/repo#7", "since": "2026-11-17T10:00:00Z" },
            { "kind": "time", "on": "2026-11-18T09:00:00Z", "since": "2026-11-17T10:01:00Z" },
        ])
    );
    let text = setup.priory(&["queue"]);
    succeeded(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.ends_with(
            "\nasleep\tkey\tsince\twaits on\n1\tcli:docs-1\t2026-11-17T10:00:00Z\treview github:owner/repo#7 since 2026-11-17T10:00:00Z; time 2026-11-18T09:00:00Z since 2026-11-17T10:01:00Z\n"
        ),
        "{text}"
    );
}

#[test]
fn each_queue_row_carries_its_tasks_tldr_or_else_the_first_line_of_its_request() {
    let mut setup = Setup::new("queue-titles", SERVE);
    setup.seed();
    let long = "x".repeat(130);
    succeeded(&setup.priory(&[
        "request",
        &setup.factory(),
        "--repo",
        "owner/repo",
        "--workflow",
        "implement",
        "--key",
        "docs-3",
        &long,
    ]));
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    succeeded(&setup.priory(&["tldr", "1", "Docs fixed"]));
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "INSERT INTO runs (task, step, tmux, session) VALUES (1, 'implement', 'priory-1', 's1');
         UPDATE tasks SET state = 'done', changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
           WHERE id = 3;",
    )
    .unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let queue = api(&address, "/api/queue");

    let titles = |list: &str, id: &str| -> Vec<(Value, Value)> {
        queue[list]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row[id].clone(), row["title"].clone()))
            .collect()
    };
    assert_eq!(
        titles("waiting", "id"),
        [
            (json!(1), json!("Docs fixed")),
            (json!(2), json!("Fix the"))
        ]
    );
    assert_eq!(titles("running", "task"), [(json!(1), json!("Docs fixed"))]);
    assert_eq!(titles("ended", "id"), [(json!(3), json!("x".repeat(120)))]);
    assert_eq!(
        titles("up_next", "task"),
        [
            (json!(1), json!("Docs fixed")),
            (json!(2), json!("Fix the"))
        ]
    );
    assert_eq!(json_of(&setup, "queue"), queue);
}

#[test]
fn the_queue_lists_the_factorys_bands_in_order_even_an_empty_one() {
    let settings = SERVE.replacen("\n", "\nbands = [\"urgent\", \"normal\", \"someday\"]\n", 1);
    let mut setup = Setup::new("queue-bands", &settings);
    setup.seed();
    let address = setup.start(&["serve", &setup.factory()]);

    let queue = api(&address, "/api/queue");

    assert_eq!(queue["bands"], json!(["urgent", "normal", "someday"]));
    assert_eq!(json_of(&setup, "queue")["bands"], queue["bands"]);
}

#[test]
fn the_queue_lists_the_factorys_workflows_so_a_view_can_offer_them() {
    let mut setup = Setup::new("queue-workflows", SERVE);
    fs::write(setup.dir.join("factory/workflows/review.toml"), WORKFLOW).unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let queue = api(&address, "/api/queue");

    assert_eq!(queue["workflows"], json!(["implement", "review"]));
    assert_eq!(json_of(&setup, "queue")["workflows"], queue["workflows"]);
    let output = setup.priory(&["queue", "--json"]);
    succeeded(&output);
    let without_factory: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(without_factory["workflows"], json!([]));
}

#[test]
fn the_views_endpoint_returns_the_default_view_priory_ships() {
    let mut setup = Setup::new("views", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);

    let views = api(&address, "/api/views");

    assert_eq!(views["factory"], "demo");
    assert_eq!(
        views["views"],
        json!({
            "default": {
                "layout": {
                    "tabs": [
                        { "panel": "desk" },
                        { "panel": "asks" },
                        { "panel": "tasks" },
                        { "panel": "runs" },
                        { "panel": "inbox" },
                        { "panel": "reports" },
                        { "panel": "workflow" },
                        { "panel": "costs" }
                    ]
                }
            }
        })
    );
}

#[test]
fn a_factory_view_replaces_the_shipped_view_of_its_name() {
    let mut setup = Setup::new("view-override", SERVE);
    let views = setup.dir.join("factory/views");
    fs::create_dir_all(&views).unwrap();
    let row = "[layout]\nrow = [{ panel = \"desk\" }, { column = [{ panel = \"queue\" }, { panel = \"runs\" }] }]\n";
    fs::write(views.join("default.toml"), row).unwrap();
    fs::write(views.join("night.yaml"), "layout:\n  panel: terminal\n").unwrap();
    let address = setup.start(&["serve", &setup.factory()]);

    let views = api(&address, "/api/views");

    assert_eq!(
        views["views"],
        json!({
            "default": {
                "layout": {
                    "row": [
                        { "panel": "desk" },
                        { "column": [{ "panel": "queue" }, { "panel": "runs" }] }
                    ]
                }
            },
            "night": { "layout": { "panel": "terminal" } }
        })
    );
}

/// Sends a GET without the token and returns the status, the headers in lowercase and the body.
fn fetch(address: &str, path: &str) -> (u16, String, Vec<u8>) {
    let mut stream = TcpStream::connect(address).unwrap();
    let head = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let end = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&response[..end]).to_lowercase();
    let status = head[9..12].parse().unwrap();
    (status, head, response[end + 4..].to_vec())
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: ")))
}

#[test]
fn the_web_view_serves_every_file_with_its_type_and_a_strict_policy_without_the_token() {
    let mut setup = Setup::new("web-files", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files: Vec<String> = ["crates/priory-core/web", "design/fonts"]
        .iter()
        .flat_map(|dir| fs::read_dir(root.join(dir)).unwrap())
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            // The web view's Node tests and their package.json are not served.
            path.extension()
                .is_some_and(|ext| ext != "txt" && ext != "md" && ext != "json")
                && !name.ends_with(".test.js")
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            match path.extension().unwrap().to_str().unwrap() {
                "woff2" => format!("/fonts/{name}"),
                _ => format!("/{name}"),
            }
        })
        .collect();
    files.extend(
        [
            "/tokens/structure.css",
            "/tokens/themes.css",
            "/favicon.ico",
            "/favicon.svg",
            "/apple-touch-icon.png",
            "/icon-192.png",
            "/icon-512.png",
            "/icon-maskable-192.png",
            "/icon-maskable-512.png",
        ]
        .map(str::to_owned),
    );
    assert!(files.len() > 12, "{files:?}");

    for path in files {
        let (status, head, body) = fetch(&address, &path);
        let expected = match path.rsplit('.').next().unwrap() {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "woff2" => "font/woff2",
            "ico" => "image/x-icon",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "webmanifest" => "application/manifest+json",
            other => panic!("no type expected for .{other}"),
        };
        assert_eq!(status, 200, "{path}: {}", String::from_utf8_lossy(&body));
        assert_eq!(header(&head, "content-type"), Some(expected), "{path}");
        assert_eq!(
            header(&head, "x-content-type-options"),
            Some("nosniff"),
            "{path}"
        );
        assert!(!body.is_empty(), "{path}");
    }

    // The worker's install fails whole when one file it caches is missing.
    let worker =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../priory-core/web/sw.js"))
            .unwrap();
    let files = worker
        .lines()
        .find_map(|line| line.strip_prefix("const FILES = ["))
        .unwrap();
    for path in files.split('"').filter(|part| part.starts_with('/')) {
        assert_eq!(fetch(&address, path).0, 200, "{path}");
    }

    let (status, head, index) = fetch(&address, "/asks/42");
    assert_eq!(status, 200);
    assert_eq!(index, fetch(&address, "/").2);
    assert_eq!(
        header(&head, "content-security-policy"),
        Some(
            "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; \
             img-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src 'self'; \
             base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
        )
    );
    assert_eq!(fetch(&address, "/asks/cache.ship").2, index);
    assert_eq!(fetch(&address, "/nothing.js").0, 404);
    assert_eq!(fetch(&address, "/fonts/nothing.woff2").0, 404);
    assert_eq!(fetch(&address, "/api/views").0, 401);
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
        self.poll_answering(&[(EVENTS_PATH, events)]);
    }

    /// Polls once with a fake `gh` that answers each `gh api <path>...` from `answers`.
    fn poll_answering(&self, answers: &[(&str, Value)]) {
        let mut script = String::from("#!/bin/sh\ncase \"$*\" in\n");
        for (at, (path, answer)) in answers.iter().enumerate() {
            let file = self.dir.join(format!("answer-{at}.json"));
            fs::write(&file, answer.to_string()).unwrap();
            script.push_str(&format!("  \"api {path}\"*) cat '{}' ;;\n", file.display()));
        }
        script.push_str("  *) echo '[]' ;;\nesac\n");
        let gh = self.dir.join("bin/gh");
        fs::create_dir_all(gh.parent().unwrap()).unwrap();
        fs::write(&gh, script).unwrap();
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
        "merge_commit_sha": "def456",
        "body": "Reverts owner/repo#7",
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
                    "body": "Looks good.",
                    "html_url": "https://github.com/owner/repo/pull/43#pullrequestreview-2561579718",
                    "user": { "login": "ana" },
                    "commit_id": "fff000",
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
    assert_eq!(
        (
            &published[3].data["merge_commit"],
            &published[3].data["body"]
        ),
        (&json!("def456"), &json!("Reverts owner/repo#7"))
    );
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
            "body": "Looks good.",
            "commit": "fff000",
            "actor": "ana",
            "open": false,
            "time": "2026-11-17T10:30:00Z",
            "github_id": 2561579718u64,
            "delivery": "d-2",
            "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_comment_a_poll_published_publishes_nothing_more_from_its_webhook() {
    let mut setup = Setup::hooks("hook-comments");
    let address = setup.start(&["serve", &setup.factory()]);
    let comment = json!({
        "id": 701,
        "body": "Can you split this?",
        "html_url": "https://github.com/owner/repo/pull/43#issuecomment-701",
        "created_at": "2026-11-17T12:00:00Z",
        "user": { "login": "ana", "type": "User" }
    });
    let thread = json!({
        "id": 802,
        "body": "Rename this.",
        "html_url": "https://github.com/owner/repo/pull/43#discussion_r802",
        "created_at": "2026-11-17T12:10:00Z",
        "path": "src/lib.rs",
        "line": 12,
        "in_reply_to_id": 801,
        "pull_request_url": "https://api.github.com/repos/owner/repo/pulls/43",
        "user": { "login": "ana", "type": "User" }
    });
    let open = json!([{ "data": { "repository": { "pullRequests": { "nodes": [{
        "number": 43,
        "headRefName": "priory/3",
        "headRefOid": "abc123",
        "baseRefName": "main",
        "merged": false,
        "state": "OPEN",
        "updatedAt": "2999-01-01T00:00:00Z",
        "reviews": { "nodes": [] },
        "comments": { "nodes": [{
            "databaseId": 701,
            "body": "Can you split this?",
            "url": "https://github.com/owner/repo/pull/43#issuecomment-701",
            "createdAt": "2026-11-17T12:00:00Z",
            "author": { "login": "ana", "__typename": "User" }
        }] },
        "commits": { "nodes": [] }
    }] } } } }]);
    setup.poll_answering(&[
        ("graphql --paginate", open),
        (
            "repos/owner/repo/pulls/comments?sort=created&direction=desc&per_page=100",
            json!([thread]),
        ),
    ]);
    let polled = setup.published();
    let repository = json!({ "full_name": "owner/repo" });
    let mut issue: Value = serde_json::from_str(&issue_labeled("agent")).unwrap();
    issue["issue"]["number"] = json!(43);
    issue["issue"]["pull_request"] =
        json!({ "url": "https://api.github.com/repos/owner/repo/pulls/43" });
    let deliveries = [
        (
            "issue_comment",
            json!({ "action": "created", "issue": issue["issue"], "comment": comment, "repository": repository }),
        ),
        (
            "pull_request_review_comment",
            json!({ "action": "created", "pull_request": pull(), "comment": thread, "repository": repository }),
        ),
    ];

    for (event, body) in deliveries {
        let body = body.to_string();
        let signature = sign(&setup.secret, body.as_bytes());
        let (status, reply) = deliver(&address, event, "d-9", Some(&signature), &body);
        assert_eq!(status, 202, "{event}: {reply}");
        assert_eq!(reply, r#"{"outcome":"published"}"#, "{event}");
    }

    let kinds: Vec<_> = polled.iter().map(|m| m.kind.as_str()).collect();
    assert!(kinds.contains(&"github.pull_request.comment"), "{kinds:?}");
    assert!(
        kinds.contains(&"github.pull_request_review_comment"),
        "{kinds:?}"
    );
    assert_eq!(setup.published(), polled);
}

#[test]
fn a_failed_gh_api_user_is_asked_again_on_the_next_delivery() {
    let mut setup = Setup::hooks("hook-viewer");
    let address = setup.start(&["serve", &setup.factory()]);
    let review = |id: u64| {
        json!({
            "action": "submitted",
            "review": {
                "id": id,
                "state": "commented",
                "body": "One thing.",
                "html_url": format!("https://github.com/owner/repo/pull/43#pullrequestreview-{id}"),
                "user": { "login": "ana" },
                "commit_id": "fff000",
                "submitted_at": "2026-11-17T10:30:00Z"
            },
            "pull_request": pull(),
            "repository": { "full_name": "owner/repo" },
            "sender": { "login": "ana" }
        })
        .to_string()
    };
    let send = |body: String| {
        let signature = sign(&setup.secret, body.as_bytes());
        let (status, reply) = deliver(
            &address,
            "pull_request_review",
            "d-3",
            Some(&signature),
            &body,
        );
        assert_eq!(status, 202, "{reply}");
    };

    send(review(1));
    fs::write(setup.dir.join("fake/gh-user"), "max").unwrap();
    send(review(2));

    let owns: Vec<_> = setup
        .published()
        .iter()
        .map(|m| m.data["own"].clone())
        .collect();
    assert_eq!(owns, [Value::Null, json!(false)]);
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
        .args(["wait", "--since", &revision.to_string(), "--timeout", "60s"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    succeeded(&setup.priory(&["pause", "lunch"]));
    let changed = Instant::now();
    let output = waiter.wait_with_output().unwrap();

    succeeded(&output);
    // Far under the 60 s timeout, so a wake on the change shows; load can stretch the wake past 1 s.
    assert!(changed.elapsed() < Duration::from_secs(30));
    let after: i64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .unwrap();
    assert!(after > revision);
    assert!(!setup.priory(&["wait"]).status.success());
}

#[test]
fn a_task_leaving_the_queues_ended_list_after_a_day_bumps_the_revision() {
    let mut setup = Setup::new("queue-ages", SERVE);
    succeeded(&setup.priory(&[
        "request",
        &setup.factory(),
        "--repo",
        "owner/repo",
        "--workflow",
        "implement",
        "--key",
        "docs-1",
        "Fix the docs",
    ]));
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    // A trigger stamps `changed` on a state change, so the time goes in a second update.
    db.execute_batch(
        "UPDATE tasks SET state = 'done';
         UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-1 day', '+8 seconds');",
    )
    .unwrap();
    setup.start(&["daemon", &setup.factory()]);
    let queue = json_of(&setup, "queue");
    assert_eq!(queue["ended"].as_array().unwrap().len(), 1, "{queue}");
    let revision = queue["revision"].as_i64().unwrap().to_string();
    let idle = setup.priory(&["wait", "--since", &revision, "--timeout", "1s"]);
    assert_eq!(
        String::from_utf8_lossy(&idle.stdout),
        format!("{revision}\n")
    );

    let aged = setup.priory(&["wait", "--since", &revision, "--timeout", "30s"]);

    assert_ne!(
        String::from_utf8_lossy(&aged.stdout),
        format!("{revision}\n")
    );
    assert_eq!(json_of(&setup, "queue")["ended"], json!([]));
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
fn report_list_names_every_report_newest_first_as_its_endpoint_returns_them() {
    let mut setup = Setup::new("report-list", SERVE);
    setup.seed();
    let listed = setup.priory(&["report", "list", "--json"]);
    succeeded(&listed);
    let empty: Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(empty["reports"], json!([]));
    succeeded(&setup.put("2", &report()));
    let mut later = report();
    later["date"] = json!("2026-11-21");
    later["greeting"] = json!("Later");
    later["sections"][0]["rows"][0]["pick"]["id"] = json!("2026-11-21_sentry_fnd-2409");
    succeeded(&setup.put("1", &later));
    let address = setup.start(&["serve", &setup.factory()]);

    let listed = setup.priory(&["report", "list", "--json"]);

    succeeded(&listed);
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    let heads: Vec<_> = listed["reports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|head| {
            (
                head["task"].clone(),
                head["date"].clone(),
                head["greeting"].clone(),
            )
        })
        .collect();
    assert_eq!(
        heads,
        [
            (json!(1), json!("2026-11-21"), json!("Later")),
            (json!(2), json!("2026-11-20"), json!("Bonjour, Max")),
        ]
    );
    assert!(listed["revision"].is_i64());
    assert_eq!(api(&address, "/api/reports"), listed);
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

impl Setup {
    fn schedule(&self, name: &str, trigger: &str) {
        fs::create_dir_all(self.dir.join("factory/triggers")).unwrap();
        fs::write(
            self.dir.join(format!("factory/triggers/{name}.toml")),
            trigger,
        )
        .unwrap();
    }

    fn json_at(&self, now: &str, args: &[&str]) -> Value {
        let output = self
            .command()
            .env("PRIORY_NOW", now)
            .args(args)
            .output()
            .unwrap();
        succeeded(&output);
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

const WEEKDAYS: &str = r#"
source = "schedule"
repo = "owner/repo"
at = "09:00"
days = ["mon", "tue", "wed", "thu", "fri"]
timezone = "Europe/Paris"
workflow = "implement"
"#;

#[test]
fn a_weekday_schedule_seen_on_friday_evening_fires_on_monday_morning() {
    let setup = Setup::new("schedules-weekday", SERVE);
    setup.schedule("standup", WEEKDAYS);

    let schedules = setup.json_at(
        "2026-11-20T19:00:00Z",
        &["schedules", &setup.factory(), "--json"],
    );

    assert_eq!(
        schedules["schedules"],
        json!([{
            "trigger": "standup",
            "workflow": "implement",
            "next": "2026-11-23T08:00:00Z",
            "key": null,
            "task": null,
            "task_state": null,
        }])
    );
}

const SUPPORT_DAY: &str = r#"
source = "schedule"
repo = "owner/repo"
at = "09:00"
every = "1h"
until = "18:00"
timezone = "Europe/Paris"
key = "support-day"
workflow = "implement"
"#;

#[test]
fn a_schedule_past_its_until_fires_next_at_the_start_of_its_next_day_and_names_its_task() {
    let setup = Setup::new("schedules-until", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "UPDATE tasks SET key = 'schedule:support-day', state = 'asleep' WHERE id = 2",
        [],
    )
    .unwrap();
    setup.schedule("support", SUPPORT_DAY);

    let factory = setup.factory();
    let args = ["schedules", factory.as_str(), "--json"];
    let during = setup.json_at("2026-11-19T10:30:00Z", &args);
    let after = setup.json_at("2026-11-19T17:30:00Z", &args);
    let queue = setup.json_at("2026-11-19T17:30:00Z", &["queue", &factory, "--json"]);

    assert_eq!(during["schedules"][0]["next"], "2026-11-19T11:00:00Z");
    assert_eq!(
        after["schedules"],
        json!([{
            "trigger": "support",
            "workflow": "implement",
            "next": "2026-11-20T08:00:00Z",
            "key": "schedule:support-day",
            "task": 2,
            "task_state": "asleep",
        }])
    );
    assert_eq!(
        [&queue["up_next"][1]["task"], &queue["up_next"][1]["title"]],
        [&json!(2), &json!("Fix the")]
    );
}

#[test]
fn up_next_lists_the_next_three_runs_queued_held_or_scheduled_in_time_order() {
    let mut setup = Setup::new("up-next", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(
        "UPDATE tasks SET state = 'held', reason = 'until tonight' WHERE id = 2;
         INSERT INTO holds (task, reason, kind, until, resume)
         VALUES (2, 'until tonight', 'time', '2026-11-17T20:00:00Z', 'queued');",
    )
    .unwrap();
    setup.schedule(
        "friday",
        &WEEKDAYS.replace(r#"["mon", "tue", "wed", "thu", "fri"]"#, r#"["fri"]"#),
    );
    setup.schedule(
        "nightly",
        "source = \"schedule\"\nrepo = \"owner/repo\"\nat = \"23:00\"\ntimezone = \"UTC\"\nworkflow = \"implement\"\n",
    );
    let factory = setup.factory();
    let address = setup.start(&["serve", &factory]);

    let queue = setup.json_at("2026-11-17T12:00:00Z", &["queue", &factory, "--json"]);

    assert_eq!(
        queue["up_next"],
        json!([
            {
                "at": "2026-11-17T12:00:00Z", "kind": "queued", "task": 1, "key": "cli:docs-1",
                "title": "Fix the docs", "trigger": null, "why": "first in the queue",
            },
            {
                "at": "2026-11-17T20:00:00Z", "kind": "held", "task": 2, "key": "cli:docs-2",
                "title": "Fix the", "trigger": null, "why": "held until 2026-11-17T20:00:00Z",
            },
            {
                "at": "2026-11-17T23:00:00Z", "kind": "scheduled", "task": null, "key": null,
                "title": null, "trigger": "nightly", "why": "schedule nightly fires",
            },
        ])
    );
    assert_eq!(api(&address, "/api/queue"), queue);
}

#[test]
fn up_next_says_a_queued_task_waits_for_its_own_reason_before_its_place() {
    let setup = Setup::new("up-next-reason", SERVE);
    setup.seed();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute(
        "UPDATE tasks SET reason = 'waits for its claim to reach the log' WHERE id = 1",
        [],
    )
    .unwrap();
    let factory = setup.factory();

    let queue = setup.json_at("2026-11-17T12:00:00Z", &["queue", &factory, "--json"]);

    let whys: Vec<&Value> = queue["up_next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["why"])
        .collect();
    assert_eq!(
        whys,
        [
            &json!("waits for its claim to reach the log"),
            &json!("queue position 2")
        ]
    );
}

#[test]
fn up_next_lists_each_firing_of_a_keyless_schedule_and_none_of_a_held_one() {
    let setup = Setup::new("up-next-firings", SERVE);
    setup.schedule(
        "hourly",
        "source = \"schedule\"\nrepo = \"owner/repo\"\nat = \"13:00\"\nevery = \"1h\"\ntimezone = \"UTC\"\nworkflow = \"implement\"\n",
    );
    setup.schedule(
        "held",
        "source = \"schedule\"\nrepo = \"owner/repo\"\nat = \"12:30\"\ntimezone = \"UTC\"\nworkflow = \"implement\"\nhold = { reason = \"a person starts it\" }\n",
    );
    let factory = setup.factory();

    let queue = setup.json_at("2026-11-17T12:00:00Z", &["queue", &factory, "--json"]);

    let at: Vec<&Value> = queue["up_next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["at"])
        .collect();
    assert_eq!(
        at,
        [
            "2026-11-17T13:00:00Z",
            "2026-11-17T14:00:00Z",
            "2026-11-17T15:00:00Z"
        ]
    );
}

/// Posts `body` as JSON with the token and `extra` header lines, and returns the status and body.
fn post(address: &str, path: &str, extra: &str, body: &str) -> (u16, String) {
    raw(
        address,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\n\
             Content-Type: application/json\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
}

impl Setup {
    /// `seed`, with task 1 asleep on its open ask, as an agent's ask leaves it.
    fn asleep_on_ask(&self) {
        self.seed();
        self.db()
            .execute("UPDATE tasks SET state = 'asleep' WHERE id = 1", [])
            .unwrap();
    }

    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.dir.join("priory.db")).unwrap()
    }

    /// Ask 1 as `priory asks --json` lists it.
    fn ask(&self) -> Value {
        json_of(self, "asks")["asks"][0].clone()
    }
}

#[test]
fn an_answer_through_the_endpoint_wakes_the_task_as_priory_answer_does() {
    let mut setup = Setup::new("web-answer", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);

    let (status, body) = post(
        &address,
        "/api/asks/1/answer",
        "",
        r#"{"option": "left", "revision": 2}"#,
    );

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({ "ok": "ask 1 answered" })
    );
    let ask = setup.ask();
    assert_eq!([&ask["state"], &ask["answer"]], ["answered", "left"]);
    let prompt: String = setup
        .db()
        .query_row(
            "SELECT text FROM outbox WHERE task = 1 ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        prompt.starts_with("Answer to ask 1 (revision 2): left"),
        "{prompt}"
    );
}

#[test]
fn an_answer_that_runs_a_command_needs_run_true_and_the_ask_then_shows_what_it_ran() {
    let mut setup = Setup::new("web-command", SERVE);
    setup.asleep_on_ask();
    let command = json!({
        "argv": ["sh", "-c", "echo ran >> ran.log; echo shipped"],
        "target": "worktree",
        "dir": setup.dir,
        "runs": ["left"],
    });
    setup
        .db()
        .execute(
            "UPDATE asks SET command = ?1 WHERE id = 1",
            [command.to_string()],
        )
        .unwrap();
    let address = setup.start(&["daemon", &setup.factory()]);
    let answer = |body: &str| post(&address, "/api/asks/1/answer", "", body);

    let (unconfirmed, refusal) = answer(r#"{"option": "left", "revision": 2}"#);
    let (status, body) = answer(r#"{"option": "left", "revision": 2, "run": true}"#);

    assert_eq!(unconfirmed, 422, "{refusal}");
    let shown = r#"["sh","-c","echo ran >> ran.log; echo shipped"]"#;
    let dir = setup.dir.display();
    assert_eq!(
        serde_json::from_str::<Value>(&refusal).unwrap()["error"],
        format!(
            "ask 1 runs {shown} in {dir} (target worktree), bound to revision 2, when answered \"left\": answer again with --run to run it"
        )
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["ok"],
        format!("ask 1 answered: running {shown} in {dir}")
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while setup.ask()["ran"]["status"] == "running" {
        assert!(std::time::Instant::now() < deadline, "{}", setup.ask());
        std::thread::sleep(Duration::from_millis(50));
    }
    let ask = setup.ask();
    assert_eq!(
        ask["command"]["argv"][2],
        "echo ran >> ran.log; echo shipped"
    );
    assert_eq!(
        ask["ran"],
        json!({ "status": "done", "revision": 2, "error": null, "stdout": "shipped", "stderr": "" })
    );
    assert_eq!(
        fs::read_to_string(setup.dir.join("ran.log")).unwrap(),
        "ran\n"
    );
}

#[test]
fn a_stale_revision_gets_409_with_the_current_ask_and_changes_only_its_refused_answers() {
    let mut setup = Setup::new("web-stale", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);
    let before = setup.ask();

    let (status, body) = post(
        &address,
        "/api/asks/1/answer",
        "",
        r#"{"option": "left", "revision": 1}"#,
    );

    assert_eq!(status, 409, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["error"], "ask 1 is at revision 2, not 1");
    let mut after = setup.ask();
    assert_eq!(body["ask"], after);
    let refused = after["refused"].take();
    assert_eq!(refused[0]["answer"], "left");
    assert_eq!(refused[0]["by"], "serve");
    assert_eq!(refused.as_array().unwrap().len(), 1);
    after["refused"] = before["refused"].clone();
    assert_eq!(after, before);
}

#[test]
fn a_wrong_token_gets_401_and_a_foreign_origin_403_and_neither_changes_anything() {
    let mut setup = Setup::new("web-refused", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);
    let before = setup.ask();
    let body = r#"{"option": "left", "revision": 2}"#;

    let wrong = raw(
        &address,
        &format!(
            "POST /api/asks/1/answer HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer nope\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    let foreign = post(
        &address,
        "/api/asks/1/answer",
        "Origin: http://evil.example\r\n",
        body,
    );
    let null = post(&address, "/api/asks/1/dismiss", "Origin: null\r\n", body);

    assert_eq!(wrong.0, 401, "{}", wrong.1);
    assert_eq!(foreign.0, 403, "{}", foreign.1);
    assert_eq!(null.0, 403, "{}", null.1);
    assert_eq!(setup.ask(), before);
    let own = post(
        &address,
        "/api/asks/1/answer",
        &format!("Origin: http://{address}\r\n"),
        body,
    );
    assert_eq!(own.0, 200, "{}", own.1);
}

const EXTENSION: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";

#[test]
fn an_extension_origin_writes_only_when_serve_lists_it_and_still_needs_the_token() {
    let listed = format!("{SERVE}extension_origins = [\"{EXTENSION}\"]\n");
    let mut setup = Setup::new("web-extension", &listed);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);
    let before = setup.ask();
    let body = r#"{"option": "left", "revision": 2}"#;
    let origin = format!("Origin: {EXTENSION}\r\n");

    let other = post(
        &address,
        "/api/asks/1/answer",
        "Origin: chrome-extension://ponmlkjihgfedcbaponmlkjihgfedcba\r\n",
        body,
    );
    let tokenless = raw(
        &address,
        &format!(
            "POST /api/asks/1/answer HTTP/1.1\r\nHost: {address}\r\n{origin}\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );

    assert_eq!(other.0, 403, "{}", other.1);
    assert_eq!(tokenless.0, 401, "{}", tokenless.1);
    assert_eq!(setup.ask(), before);
    let listed = post(&address, "/api/asks/1/answer", &origin, body);
    assert_eq!(listed.0, 200, "{}", listed.1);
}

#[test]
fn an_extension_origin_cannot_write_when_serve_lists_none() {
    let mut setup = Setup::new("web-extension-unlisted", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);
    let before = setup.ask();

    let (status, body) = post(
        &address,
        "/api/asks/1/answer",
        &format!("Origin: {EXTENSION}\r\n"),
        r#"{"option": "left", "revision": 2}"#,
    );

    assert_eq!(status, 403, "{body}");
    assert_eq!(setup.ask(), before);
}

#[test]
fn check_refuses_an_extension_origin_that_is_not_an_exact_extension_origin() {
    for bad in ["https://github.com", "chrome-extension://abc/", "*"] {
        let setup = Setup::new(
            "extension-check",
            "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n",
        );
        fs::write(
            setup.dir.join("factory/factory.toml"),
            format!("name = \"demo\"\n[serve]\nextension_origins = [\"{bad}\"]\n"),
        )
        .unwrap();
        let output = setup.priory(&["check", &setup.factory()]);
        assert!(!output.status.success(), "{bad} passed check");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(&format!(
                "`[serve] extension_origins` {bad} is not a Chromium extension's origin"
            )),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn serve_without_a_daemon_gets_503_on_writes_and_200_on_reads() {
    let mut setup = Setup::new("web-no-daemon", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["serve", &setup.factory()]);

    let (status, body) = post(
        &address,
        "/api/asks/1/answer",
        "",
        r#"{"option": "left", "revision": 2}"#,
    );

    assert_eq!(status, 503, "{body}");
    assert!(body.contains("Start `priory daemon`"), "{body}");
    assert_eq!(get(&address, "/api/asks", Some(TOKEN)).0, 200);
    assert_eq!(setup.ask()["state"], "open");
}

#[test]
fn a_daemon_that_takes_a_write_but_does_not_reply_gets_504_and_the_state_stays_unread() {
    let mut setup = Setup::new("web-504", SERVE);
    setup.asleep_on_ask();
    let socket =
        priory_runtime::runtime::socket_in(Some(setup.tmux.path()), &setup.dir.join("home"));
    fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let silent = std::thread::spawn(move || {
        // Reads the line, then drops the connection with no reply.
        let (stream, _) = listener.accept().unwrap();
        BufReader::new(stream)
            .read_line(&mut String::new())
            .unwrap();
    });
    let address = setup.start(&["serve", &setup.factory()]);

    let (status, body) = post(
        &address,
        "/api/snooze",
        "",
        r#"{"kind": "ask", "id": 1, "for": "1h"}"#,
    );

    silent.join().unwrap();
    assert_eq!(status, 504, "{body}");
    assert!(body.contains("The daemon may have applied it"), "{body}");
    assert!(setup.ask()["snoozed_until"].is_null());
}

#[test]
fn a_write_body_over_16_kib_gets_413_and_one_not_json_415() {
    let mut setup = Setup::new("web-limits", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);
    let long = format!(r#"{{"text": "{}", "revision": 2}}"#, "a".repeat(16 * 1024));

    let (status, body) = post(&address, "/api/asks/1/answer", "", &long);
    let form = raw(
        &address,
        &format!(
            "POST /api/asks/1/answer HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\n\
             Content-Type: application/x-www-form-urlencoded\r\nContent-Length: 6\r\n\r\noption"
        ),
    );

    assert_eq!(status, 413, "{body}");
    assert_eq!(form.0, 415, "{}", form.1);
    assert_eq!(setup.ask()["state"], "open");
}

#[test]
fn dismiss_and_snooze_through_their_endpoints_do_what_their_cli_verbs_do() {
    let mut setup = Setup::new("web-dismiss", SERVE);
    setup.asleep_on_ask();
    let address = setup.start(&["daemon", &setup.factory()]);

    let snoozed = post(
        &address,
        "/api/snooze",
        "",
        r#"{"kind": "ask", "id": 1, "for": "1h"}"#,
    );
    assert_eq!(snoozed.0, 200, "{}", snoozed.1);
    assert!(setup.ask()["snoozed_until"].is_string());
    let cleared = post(
        &address,
        "/api/snooze",
        "",
        r#"{"kind": "ask", "id": 1, "clear": true}"#,
    );
    assert_eq!(cleared.0, 200, "{}", cleared.1);
    assert!(setup.ask()["snoozed_until"].is_null());
    let both = post(
        &address,
        "/api/snooze",
        "",
        r#"{"kind": "ask", "id": 1, "for": "1h", "clear": true}"#,
    );
    assert_eq!(both.0, 400, "{}", both.1);
    let unknown = post(
        &address,
        "/api/snooze",
        "",
        r#"{"kind": "run", "id": 1, "for": "1h"}"#,
    );
    assert_eq!(unknown.0, 422, "{}", unknown.1);

    let dismissed = post(&address, "/api/asks/1/dismiss", "", r#"{"revision": 2}"#);

    assert_eq!(dismissed.0, 200, "{}", dismissed.1);
    let ask = setup.ask();
    assert_eq!(
        [&ask["state"], &ask["withdrawn_why"]],
        ["withdrawn", "dismissed"]
    );
    let again = post(&address, "/api/asks/1/dismiss", "", r#"{"revision": 2}"#);
    assert_eq!(again.0, 409, "{}", again.1);
    assert_eq!(get(&address, "/api/asks/1/answer", Some(TOKEN)).0, 405);
    assert_eq!(
        post(
            &address,
            "/api/asks/9/answer",
            "",
            r#"{"text": "x", "revision": 1}"#
        )
        .0,
        404
    );
}

#[test]
fn todos_added_and_done_through_their_endpoints_do_what_their_cli_verbs_do() {
    let mut setup = Setup::new("web-todos", SERVE);
    setup.seed();
    let address = setup.start(&["daemon", &setup.factory()]);
    let desk_todos = || -> Vec<Value> {
        let desk = json_of(&setup, "desk");
        desk["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["kind"] == "todo")
            .cloned()
            .collect()
    };

    let added = post(
        &address,
        "/api/todos",
        "",
        r#"{"text": "Renew the TLS cert", "band": "today", "due": "2026-12-01T09:00:00Z"}"#,
    );
    let todos = desk_todos();
    let bad_band = post(
        &address,
        "/api/todos",
        "",
        r#"{"text": "x", "band": "someday"}"#,
    );
    let foreign = post(
        &address,
        "/api/todos",
        "Origin: http://evil.example\r\n",
        r#"{"text": "x"}"#,
    );
    let done = post(&address, "/api/todos/1/done", "", "{}");
    let again = post(&address, "/api/todos/1/done", "", "{}");

    assert_eq!(added, (200, r#"{"ok":"1"}"#.to_owned()));
    assert_eq!(todos.len(), 1);
    assert_eq!(
        todos[0]["what"],
        "Renew the TLS cert, due 2026-12-01T09:00:00Z"
    );
    assert_eq!(bad_band.0, 422, "{}", bad_band.1);
    assert_eq!(foreign.0, 403, "{}", foreign.1);
    assert_eq!(done, (200, r#"{"ok":"to-do 1 is done"}"#.to_owned()));
    assert_eq!(again.0, 422, "{}", again.1);
    assert!(desk_todos().is_empty());
    assert_eq!(post(&address, "/api/todos/x/done", "", "{}").0, 405);
}

#[test]
fn a_report_and_a_held_task_leave_the_desk_through_the_read_and_release_endpoints() {
    let mut setup = Setup::new("web-clear", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    let posted = setup.priory(&["post", "ci", "the nightly build broke"]);
    succeeded(&posted);
    let seq = String::from_utf8(posted.stdout).unwrap().trim().to_owned();
    let address = setup.start(&["daemon", &setup.factory()]);
    succeeded(&setup.priory(&["hold", "1", "--reason", "after the freeze"]));
    let kinds = || -> Vec<Value> {
        json_of(&setup, "desk")["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["kind"].clone())
            .collect()
    };
    assert!(kinds().contains(&json!("report")), "{:?}", kinds());
    assert!(kinds().contains(&json!("held")), "{:?}", kinds());

    let read = post(&address, &format!("/api/inbox/{seq}/read"), "", "{}");
    let released = post(&address, "/api/tasks/1/release", "", "{}");

    assert_eq!(read.0, 200, "{}", read.1);
    assert_eq!(released.0, 200, "{}", released.1);
    assert!(!kinds().contains(&json!("report")), "{:?}", kinds());
    assert!(!kinds().contains(&json!("held")), "{:?}", kinds());
    assert_eq!(post(&address, "/api/inbox/1/read", "", "{}").0, 422);
}

#[test]
fn move_through_its_endpoint_does_what_priory_move_does_at_the_revision_the_page_drew() {
    let mut setup = Setup::new("web-move", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    // The daemon's first pass records a summary per task after it listens, which bumps the revision.
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while db
        .query_row(
            "SELECT count(*) FROM tasks
             WHERE id NOT IN (SELECT task FROM summaries WHERE answered IS NOT NULL)",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
        > 0
    {
        assert!(
            Instant::now() < deadline,
            "the daemon never summarized the tasks"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let queue = || json_of(&setup, "queue");
    let order = || -> Vec<(Value, Value)> {
        queue()["waiting"]
            .as_array()
            .unwrap()
            .iter()
            .map(|task| (task["id"].clone(), task["band"].clone()))
            .collect()
    };
    let at = |revision: i64, rest: &str| format!(r#"{{"revision": {revision}{rest}}}"#);
    let revision = queue()["revision"].as_i64().unwrap();

    let topped = post(
        &address,
        "/api/tasks/2/move",
        "",
        &at(revision, r#", "top": true"#),
    );
    let topped_order = order();
    let stale = post(
        &address,
        "/api/tasks/1/move",
        "",
        &at(revision, r#", "top": true"#),
    );
    let banded = post(
        &address,
        "/api/tasks/1/move",
        "",
        &at(queue()["revision"].as_i64().unwrap(), r#", "band": "now""#),
    );

    assert_eq!(topped.0, 200, "{}", topped.1);
    assert_eq!(
        topped_order,
        [(json!(2), json!("later")), (json!(1), json!("later"))]
    );
    assert_eq!(stale.0, 409, "{}", stale.1);
    assert_eq!(banded.0, 200, "{}", banded.1);
    assert_eq!(
        order(),
        [(json!(1), json!("now")), (json!(2), json!("later"))]
    );
    let before = post(
        &address,
        "/api/tasks/2/move",
        "",
        &at(queue()["revision"].as_i64().unwrap(), r#", "before": 1"#),
    );
    assert_eq!(before.0, 200, "{}", before.1);
    assert_eq!(
        order(),
        [(json!(2), json!("now")), (json!(1), json!("now"))]
    );
    let after = post(
        &address,
        "/api/tasks/2/move",
        "",
        &at(queue()["revision"].as_i64().unwrap(), r#", "after": 1"#),
    );
    assert_eq!(after.0, 200, "{}", after.1);
    assert_eq!(
        order(),
        [(json!(1), json!("now")), (json!(2), json!("now"))]
    );
    let revision = queue()["revision"].as_i64().unwrap();
    let both = post(
        &address,
        "/api/tasks/2/move",
        "",
        &at(revision, r#", "top": true, "after": 1"#),
    );
    let neither = post(&address, "/api/tasks/2/move", "", &at(revision, ""));
    let no_band = post(
        &address,
        "/api/tasks/2/move",
        "",
        &at(revision, r#", "band": "someday""#),
    );
    assert_eq!(both.0, 400, "{}", both.1);
    assert_eq!(neither.0, 400, "{}", neither.1);
    assert_eq!(no_band.0, 422, "{}", no_band.1);
}

#[test]
fn resume_say_cancel_and_revive_through_their_endpoints_do_what_their_cli_verbs_do() {
    let mut setup = Setup::new("web-act", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    setup
        .db()
        .execute_batch(
            "UPDATE tasks SET state = 'blocked', reason = 'needs a key' WHERE id = 1;
             UPDATE tasks SET state = 'asleep' WHERE id = 2;
             INSERT INTO runs (task, step, tmux, session, ended, ended_as)
               VALUES (2, 'implement', 'priory-2', 's2', '2026-11-17T10:00:00Z', 'asleep');",
        )
        .unwrap();
    let address = setup.start(&["daemon", &setup.factory()]);
    let state = |task: usize| {
        json_of(&setup, "tasks")["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == task)
            .unwrap()["state"]
            .clone()
    };
    let revision = || json_of(&setup, "tasks")["revision"].as_i64().unwrap();

    let resumed = post(&address, "/api/tasks/1/resume", "", "{}");
    assert_eq!(resumed.0, 200, "{}", resumed.1);
    assert_eq!(
        serde_json::from_str::<Value>(&resumed.1).unwrap(),
        json!({ "ok": "task 1 is queued" })
    );
    let said = post(
        &address,
        "/api/tasks/2/say",
        "",
        r#"{"text": "also fix the link"}"#,
    );
    assert_eq!(said.0, 200, "{}", said.1);
    let stale = post(&address, "/api/tasks/2/cancel", "", r#"{"revision": 1}"#);
    assert_eq!(stale.0, 409, "{}", stale.1);
    assert_ne!(state(2), "dropped");
    // The daemon may move the state between the read and the cancel, as it may for any client.
    let cancel = || {
        let body = format!(r#"{{"revision": {}}}"#, revision());
        post(&address, "/api/tasks/2/cancel", "", &body)
    };
    let cancelled = (0..20)
        .map(|_| cancel())
        .find(|(status, _)| *status != 409)
        .unwrap();
    assert_eq!(cancelled.0, 200, "{}", cancelled.1);
    assert_eq!(state(2), "dropped");
    let revived = post(
        &address,
        "/api/tasks/2/revive",
        "",
        r#"{"step": "implement"}"#,
    );
    assert_eq!(revived.0, 200, "{}", revived.1);
    assert_ne!(state(2), "dropped");
    let foreign = post(
        &address,
        "/api/tasks/2/cancel",
        "Origin: http://evil.example\r\n",
        "{}",
    );
    assert_eq!(foreign.0, 403, "{}", foreign.1);
    assert_eq!(get(&address, "/api/tasks/2/cancel", Some(TOKEN)).0, 405);
    assert_eq!(post(&address, "/api/tasks/9/resume", "", "{}").0, 404);
    assert_eq!(post(&address, "/api/tasks/2/say", "", "{}").0, 400);
}

#[test]
fn the_links_endpoint_lists_the_factorys_links_in_file_order_then_the_built_in_ones_it_keeps() {
    let plain = Setup::new("links-built-in", SERVE);
    let built_in = plain.priory(&["links", &plain.factory(), "--json"]);
    succeeded(&built_in);
    let github = json!({ "pattern": r"\B#(\d+)\b", "url": "https://github.com/{repo}/issues/$1" });
    assert_eq!(
        serde_json::from_slice::<Value>(&built_in.stdout).unwrap(),
        json!({ "links": [github] })
    );

    let settings = format!(
        "{SERVE}\n[links]\n'\\b([A-Z]{{2,5}}-\\d+)\\b' = \"https://linear.app/acme/issue/$1\"\n'\\B#(\\d+)\\b' = \"https://github.com/acme/app/issues/$1\"\n"
    );
    let mut setup = Setup::new("links", &settings);
    let address = setup.start(&["serve", &setup.factory()]);

    let links = api(&address, "/api/links");

    assert_eq!(
        links,
        json!({ "links": [
            { "pattern": r"\b([A-Z]{2,5}-\d+)\b", "url": "https://linear.app/acme/issue/$1" },
            { "pattern": r"\B#(\d+)\b", "url": "https://github.com/acme/app/issues/$1" },
        ] })
    );
    let cli = setup.priory(&["links", &setup.factory(), "--json"]);
    succeeded(&cli);
    assert_eq!(serde_json::from_slice::<Value>(&cli.stdout).unwrap(), links);
}

const DEFAULTS: &str = "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n\n[request]\nworkflow = \"implement\"\nband = \"today\"\n";

#[test]
fn a_request_through_the_endpoint_queues_the_task_priory_add_does_and_answers_its_key_and_id() {
    let mut setup = Setup::new("web-request", DEFAULTS);
    // No run starts, so a run's git and tmux commands never hold up the daemon's reply.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);

    let (status, body) = post(
        &address,
        "/api/request",
        "",
        r#"{"text": "Fix https://github.com/cli/cli/issues/42", "lane": "docs/site"}"#,
    );
    let (bad, refusal) = post(
        &address,
        "/api/request",
        "",
        r#"{"text": "Fix the docs", "repo": "../escape"}"#,
    );
    let (wrong, _) = raw(
        &address,
        &format!(
            "POST /api/request HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer nope\r\n\
             Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        ),
    );
    let (foreign, _) = post(
        &address,
        "/api/request",
        "Origin: http://evil.example\r\n",
        r#"{"text": "Fix the docs", "repo": "owner/repo"}"#,
    );

    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({ "key": "github:cli/cli#42", "id": 1 })
    );
    let task = json_of(&setup, "tasks")["tasks"][0].clone();
    assert_eq!(
        [
            &task["key"],
            &task["workflow"],
            &task["band"],
            &task["lane"]
        ],
        ["github:cli/cli#42", "implement", "today", "docs/site"]
    );
    assert_eq!(bad, 422, "{refusal}");
    assert!(refusal.contains("owner/name"), "{refusal}");
    assert_eq!((wrong, foreign), (401, 403));
    assert_eq!(
        json_of(&setup, "tasks")["tasks"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn a_repeated_add_of_the_same_linked_text_is_refused_with_the_task_it_reached() {
    let mut setup = Setup::new("web-request-again", DEFAULTS);
    // No run starts, so the daemon answers each request at once.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    let text = r#"{"text": "Review https://github.com/cli/cli/pull/42"}"#;
    let (first, _) = post(&address, "/api/request", "", text);

    let (status, body) = post(&address, "/api/request", "", text);
    let cli = setup.priory(&[
        "add",
        &setup.factory(),
        "Review https://github.com/cli/cli/pull/42",
    ]);

    assert_eq!((first, status), (200, 409), "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        [&body["key"], &body["id"]],
        [&json!("github:cli/cli#42"), &json!(1)]
    );
    assert!(
        body["error"].as_str().unwrap().contains("already"),
        "{body}"
    );
    assert_eq!(cli.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&cli.stderr);
    assert!(
        stderr.starts_with("priory add: github:cli/cli#42 already got this exact request"),
        "{stderr}"
    );
    succeeded(&setup.priory(&["cancel", "1"]));
    let (ended, body) = post(&address, "/api/request", "", text);
    assert_eq!(ended, 409, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        [body.get("key"), body.get("id")],
        [Some(&json!("github:cli/cli#42")), Some(&Value::Null)],
        "{body}"
    );
    assert_eq!(
        json_of(&setup, "tasks")["tasks"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn a_request_without_a_daemon_gets_503() {
    let mut setup = Setup::new("web-request-no-daemon", DEFAULTS);
    let address = setup.start(&["serve", &setup.factory()]);

    let (status, body) = post(&address, "/api/request", "", r#"{"text": "Fix the docs"}"#);

    assert_eq!(status, 503, "{body}");
}

#[test]
fn the_request_endpoint_reads_the_defaults_and_choices_the_add_control_offers() {
    let mut setup = Setup::new("web-request-defaults", DEFAULTS);
    let address = setup.start(&["serve", &setup.factory()]);
    let mut bare = Setup::new("web-request-no-defaults", SERVE);
    let bare_address = bare.start(&["serve", &bare.factory()]);

    assert_eq!(
        api(&address, "/api/request"),
        json!({
            "workflow": "implement",
            "band": "today",
            "workflows": ["implement"],
            "bands": ["blocker", "now", "today", "later"],
        })
    );
    assert_eq!(api(&bare_address, "/api/request")["workflow"], Value::Null);
    assert_eq!(get(&address, "/api/request", None).0, 401);
}

#[test]
fn the_comment_endpoints_do_what_their_cli_verbs_do() {
    let mut setup = Setup::new("web-comments", SERVE);
    setup.asleep_on_ask();
    setup
        .db()
        .execute(
            "INSERT INTO runs (task, step, tmux, session, ended) VALUES (1, 'implement', 'gone', 'gone', '2026-11-20T08:00:00Z')",
            [],
        )
        .unwrap();
    succeeded(&setup.put("1", &report()));
    let address = setup.start(&["daemon", &setup.factory()]);
    let ok = |path: &str, body: &str| {
        let (status, reply) = post(&address, path, "", body);
        assert_eq!(status, 200, "{reply}");
        serde_json::from_str::<Value>(&reply).unwrap()["ok"].clone()
    };

    assert_eq!(
        ok(
            "/api/reports/comment",
            r#"{"row": "fnd-2409", "text": "Why?", "revision": 1}"#
        ),
        "comment 1 on row fnd-2409 at revision 2"
    );
    assert_eq!(
        ok(
            "/api/reports/reply",
            r#"{"comment": 1, "text": "And when?"}"#
        ),
        "reply 2 in thread 1 at revision 3"
    );
    assert_eq!(
        ok("/api/reports/resolve", r#"{"comment": 1}"#),
        "thread 1 is resolved at revision 4"
    );
    assert_eq!(
        ok("/api/reports/reopen", r#"{"comment": 1}"#),
        "thread 1 is open at revision 5"
    );
    let stale = post(
        &address,
        "/api/reports/comment",
        "",
        r#"{"row": "fnd-2409", "text": "Late", "revision": 1}"#,
    );
    assert_eq!(stale.0, 422, "{}", stale.1);
    assert!(
        stale.1.contains("task 1's report is at revision 5, not 1"),
        "{}",
        stale.1
    );
    for (path, body) in [
        ("/api/reports/resolve", r#"{"comment": 1, "text": "Fixed"}"#),
        ("/api/reports/reply", r#"{"comment": 1}"#),
        (
            "/api/reports/comment",
            r#"{"row": "fnd-2409", "revision": 5}"#,
        ),
        (
            "/api/reports/comment",
            r#"{"comment": 1, "text": "Hi", "revision": 5}"#,
        ),
    ] {
        let (status, reply) = post(&address, path, "", body);
        assert_eq!(status, 400, "{path} {body}: {reply}");
    }
    let thread =
        api(&address, "/api/reports/latest")["report"]["sections"][0]["rows"][0]["comments"][0]
            .clone();
    assert_eq!(
        [&thread["text"], &thread["author"], &thread["resolved"]],
        [&json!("Why?"), &json!("person"), &json!(false)]
    );
    assert_eq!(thread["replies"][0]["text"], "And when?");
    assert_eq!(
        api(&address, "/api/reports/latest")["report"],
        setup.shown(&[])["report"]
    );
}

#[test]
fn the_pick_endpoint_records_a_choice_and_tells_a_stale_one_by_409() {
    let mut setup = Setup::new("web-pick", SERVE);
    setup.asleep_on_ask();
    setup
        .db()
        .execute(
            "INSERT INTO runs (task, step, tmux, session, ended) VALUES (1, 'implement', 'gone', 'gone', '2026-11-20T08:00:00Z')",
            [],
        )
        .unwrap();
    succeeded(&setup.put("1", &report()));
    let address = setup.start(&["daemon", &setup.factory()]);
    let pick = |option: &str, revision: i64| {
        post(
            &address,
            "/api/reports/pick",
            "",
            &json!({ "pick": "2026-11-20_sentry_fnd-2409", "option": option, "revision": revision })
                .to_string(),
        )
    };

    let (stale, reply) = pick("Take", 0);
    assert_eq!(stale, 409, "{reply}");
    let (status, reply) = pick("Later", 1);
    assert_eq!(status, 200, "{reply}");
    assert_eq!(
        serde_json::from_str::<Value>(&reply).unwrap()["ok"],
        "pick 2026-11-20_sentry_fnd-2409 is Later at revision 2"
    );
    let (same, reply) = pick("Later", 1);
    assert_eq!(same, 409, "{reply}");
    let (again, reply) = pick("Later", 2);
    assert_eq!(again, 409, "{reply}");
    assert!(reply.contains("already Later"), "{reply}");
    let (changed, reply) = pick("Take", 2);
    assert_eq!(changed, 200, "{reply}");
    let (unknown, reply) = pick("Maybe", 3);
    assert_eq!(unknown, 422, "{reply}");
    assert_eq!(
        api(&address, "/api/reports/latest")["report"]["sections"][0]["rows"][0]["pick"]["chosen"],
        "Take"
    );
    let mut applied = report();
    applied["sections"][0]["rows"][0]["pick"]["applied"] = json!("took it");
    succeeded(&setup.put("1", &applied));
    let (stands, reply) = pick("Later", 4);
    assert_eq!(stands, 409, "{reply}");
    assert!(reply.contains("already Take"), "{reply}");
    // A write that leaves `applied` empty keeps the applied choice.
    succeeded(&setup.put("1", &report()));
    let (still, reply) = pick("Later", 5);
    assert_eq!(still, 409, "{reply}");
    assert_eq!(
        api(&address, "/api/reports/latest")["report"]["sections"][0]["rows"][0]["pick"]["applied"],
        "took it"
    );
    assert_eq!(
        post(
            &address,
            "/api/reports/pick",
            "",
            r#"{"pick": "gone", "option": "Take", "revision": 2}"#
        )
        .0,
        404
    );
    assert_eq!(
        post(&address, "/api/reports/pick", "", r#"{"pick": "x"}"#).0,
        400
    );
}

/// A browser's `PushSubscription.toJSON()` for a fresh key pair, at `endpoint`.
fn subscription(endpoint: &str) -> (Value, p256::SecretKey) {
    use base64ct::{Base64UrlUnpadded, Encoding};
    use p256::elliptic_curve::Generate;
    let key = p256::SecretKey::generate();
    let public = key.public_key().to_sec1_bytes();
    let auth: [u8; 16] = *uuid::Uuid::new_v4().as_bytes();
    let json = json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": {
            "p256dh": Base64UrlUnpadded::encode_string(&public),
            "auth": Base64UrlUnpadded::encode_string(&auth),
        },
    });
    (json, key)
}

#[test]
fn the_push_key_endpoint_serves_the_public_key_once_a_daemon_made_the_pair() {
    let mut setup = Setup::new("push-key", SERVE);
    let alone = setup.start(&["serve", &setup.factory()]);
    let before = get(&alone, "/api/push/key", Some(TOKEN));
    let mut server = setup.server.take().unwrap();
    server.kill().unwrap();
    server.wait().unwrap();

    let address = setup.start(&["daemon", &setup.factory()]);
    let (status, body) = get(&address, "/api/push/key", Some(TOKEN));

    assert_eq!(before.0, 503, "{}", before.1);
    assert_eq!(get(&address, "/api/push/key", None).0, 401);
    assert_eq!(status, 200, "{body}");
    let reply: Value = serde_json::from_str(&body).unwrap();
    let key = reply["key"].as_str().unwrap();
    use base64ct::{Base64UrlUnpadded, Encoding};
    let point = Base64UrlUnpadded::decode_vec(key).unwrap();
    assert!(p256::PublicKey::from_sec1_bytes(&point).is_ok(), "{key}");
    assert_eq!(reply.as_object().unwrap().len(), 1, "{body}");
    let private = setup.dir.join("home/push/vapid.key");
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(&private).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let secret = fs::read_to_string(&private).unwrap();
    assert!(!body.contains(secret.trim()));
}

#[test]
fn subscribe_and_unsubscribe_store_and_drop_a_subscription_behind_the_token_and_origin() {
    let mut setup = Setup::new("push-subscribe", SERVE);
    let address = setup.start(&["daemon", &setup.factory()]);
    let (browser, _) = subscription("https://web.push.apple.com/send/abc");
    let body = browser.to_string();
    let stored = |setup: &Setup| -> Vec<String> {
        let db = setup.db();
        let mut rows = db
            .prepare("SELECT endpoint FROM push_subscriptions")
            .unwrap();
        rows.query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };

    let wrong = raw(
        &address,
        &format!(
            "POST /api/push/subscribe HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer nope\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    let foreign = post(
        &address,
        "/api/push/subscribe",
        "Origin: http://evil.example\r\n",
        &body,
    );
    let long = post(
        &address,
        "/api/push/subscribe",
        "",
        &json!({ "endpoint": "a".repeat(16 * 1024) }).to_string(),
    );
    let extra = post(
        &address,
        "/api/push/subscribe",
        "",
        &json!({ "endpoint": "https://web.push.apple.com/x", "keys": browser["keys"], "brief": "x" })
            .to_string(),
    );
    let (plain, _) = subscription("http://web.push.apple.com/send/abc");
    let (local, _) = subscription("https://127.0.0.1/send/abc");
    let insecure = post(&address, "/api/push/subscribe", "", &plain.to_string());
    let loopback = post(&address, "/api/push/subscribe", "", &local.to_string());
    let others: Vec<(u16, String)> = [
        "https://127.1/send/abc",
        "https://intranet.corp.example/send/abc",
        "https://fcm.googleapis.com.evil.example/send/abc",
        "https://user@fcm.googleapis.com/send/abc",
    ]
    .iter()
    .map(|endpoint| {
        post(
            &address,
            "/api/push/subscribe",
            "",
            &subscription(endpoint).0.to_string(),
        )
    })
    .collect();

    assert_eq!(wrong.0, 401, "{}", wrong.1);
    assert_eq!(foreign.0, 403, "{}", foreign.1);
    assert_eq!(long.0, 413, "{}", long.1);
    assert_eq!(extra.0, 400, "{}", extra.1);
    assert_eq!(insecure.0, 422, "{}", insecure.1);
    assert_eq!(loopback.0, 422, "{}", loopback.1);
    for (status, body) in &others {
        assert_eq!(*status, 422, "{body}");
    }
    assert!(stored(&setup).is_empty());

    let own = post(
        &address,
        "/api/push/subscribe",
        &format!("Origin: http://{address}\r\n"),
        &body,
    );
    assert_eq!(own.0, 200, "{}", own.1);
    assert_eq!(stored(&setup), ["https://web.push.apple.com/send/abc"]);

    let gone = post(
        &address,
        "/api/push/unsubscribe",
        "",
        &json!({ "endpoint": "https://web.push.apple.com/send/abc" }).to_string(),
    );
    assert_eq!(gone.0, 200, "{}", gone.1);
    assert!(stored(&setup).is_empty());
}

/// The plaintext of an RFC 8291 `body` sent to the subscription whose private key is `key`.
fn decrypt(body: &[u8], key: &p256::SecretKey, auth: &[u8]) -> Vec<u8> {
    use aes_gcm::aead::{Aead, KeyInit};
    let expand = |salt: &[u8], secret: &[u8], info: &[u8], out: &mut [u8]| {
        hkdf::Hkdf::<sha2::Sha256>::new(Some(salt), secret)
            .expand(info, out)
            .unwrap();
    };
    let (salt, rest) = body.split_at(16);
    let length = usize::from(rest[4]);
    let (from, sealed) = rest[5..].split_at(length);
    let server = p256::PublicKey::from_sec1_bytes(from).unwrap();
    let shared = p256::ecdh::diffie_hellman(key.to_nonzero_scalar(), server.as_affine());
    let to = key.public_key().to_sec1_bytes();
    let info = [b"WebPush: info\0".as_slice(), &to, from].concat();
    let mut ikm = [0; 32];
    expand(auth, shared.raw_secret_bytes(), &info, &mut ikm);
    let (mut cek, mut nonce) = ([0; 16], [0; 12]);
    expand(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek);
    expand(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce);
    let mut plain = aes_gcm::Aes128Gcm::new(&cek.into())
        .decrypt(&nonce.into(), sealed)
        .unwrap();
    assert_eq!(
        plain.pop(),
        Some(2),
        "one record, ending in the last-record delimiter"
    );
    plain
}

impl Setup {
    /// Subscribes a browser at `endpoint` through the API, and returns its private key and auth.
    fn subscribe(&self, address: &str, endpoint: &str) -> (p256::SecretKey, Vec<u8>) {
        use base64ct::{Base64UrlUnpadded, Encoding};
        let (browser, key) = subscription(endpoint);
        let (status, body) = post(address, "/api/push/subscribe", "", &browser.to_string());
        assert_eq!(status, 200, "{body}");
        let auth = Base64UrlUnpadded::decode_vec(browser["keys"]["auth"].as_str().unwrap());
        (key, auth.unwrap())
    }

    /// The Web Push requests the fake `curl` received, oldest first.
    fn pushes(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.join("fake/push.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Waits until the fake `curl` received `count` pushes, and returns them.
    fn until_pushes(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let pushes = self.pushes();
            if pushes.len() >= count {
                return pushes;
            }
            assert!(
                Instant::now() < deadline,
                "{} pushes: {pushes:?}\n{}",
                pushes.len(),
                fs::read_to_string(self.dir.join("server.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Adds an open ask to task 1, with a brief and a command that must never reach a phone.
    fn ask_more(&self, db: &rusqlite::Connection, title: &str) -> i64 {
        db.execute(
            "INSERT INTO asks (task, step, kind, question, options, title, text, command)
             VALUES (1, 'implement', 'agent', 'Merge it?', '[\"yes\",\"no\"]', ?1,
                     'the brief: private plan', 'rm -rf /tmp/x')",
            [title],
        )
        .unwrap();
        db.last_insert_rowid()
    }
}

fn base64(text: &Value) -> Vec<u8> {
    use base64ct::{Base64, Encoding};
    Base64::decode_vec(text.as_str().unwrap()).unwrap()
}

/// A notification's payload without its `id`, once the id is the time the notification was queued
/// and its queue row, which names nothing a lock screen shows.
fn without_id(mut payload: Value) -> Value {
    let id = payload
        .as_object_mut()
        .and_then(|fields| fields.remove("id"))
        .expect("each notification carries an id");
    let (queued, row) = id.as_str().unwrap().split_once('/').unwrap();
    assert!(queued.parse::<jiff::Timestamp>().is_ok(), "{id}");
    assert!(row.parse::<u64>().is_ok(), "{id}");
    payload
}

#[test]
fn a_new_ask_pushes_one_notification_per_subscription_with_only_its_title_and_url() {
    let mut setup = Setup::new("push-ask", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    let phone = setup.subscribe(&address, "https://web.push.apple.com/phone");
    let laptop = setup.subscribe(&address, "https://fcm.googleapis.com/laptop");

    let ask = setup.ask_more(&setup.db(), "Merge the docs fix?");

    let pushes = setup.until_pushes(2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.pushes().len(), 2, "{pushes:?}");
    let mut urls: Vec<&str> = pushes
        .iter()
        .map(|push| push["url"].as_str().unwrap())
        .collect();
    urls.sort();
    assert_eq!(
        urls,
        [
            "https://fcm.googleapis.com/laptop",
            "https://web.push.apple.com/phone"
        ]
    );
    for push in &pushes {
        let (key, auth) = if push["url"] == "https://web.push.apple.com/phone" {
            &phone
        } else {
            &laptop
        };
        let plain = decrypt(&base64(&push["body"]), key, auth);
        let payload = without_id(serde_json::from_slice(&plain).unwrap());
        assert_eq!(
            payload,
            json!({ "title": "Merge the docs fix?", "url": format!("/asks/{ask}") })
        );
        let headers = push["headers"].as_str().unwrap();
        assert!(headers.contains("Content-Encoding: aes128gcm"), "{headers}");
        assert!(headers.contains("Authorization: vapid t="), "{headers}");
        assert!(
            push["args"].as_str().unwrap().contains("--proto =https"),
            "{push}"
        );
    }
    let secret = fs::read_to_string(setup.dir.join("home/push/vapid.key")).unwrap();
    let secret = secret.trim();
    assert!(!secret.is_empty());
    for file in ["fake/push.jsonl", "fake/curl.log", "server.log"] {
        let text = fs::read_to_string(setup.dir.join(file)).unwrap_or_default();
        assert!(!text.contains(secret), "{file} holds the private key");
    }
    assert!(
        !get(&address, "/api/push/key", Some(TOKEN))
            .1
            .contains(secret)
    );
}

#[test]
fn a_snoozed_ask_pushes_nothing() {
    let mut setup = Setup::new("push-snoozed", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    setup.subscribe(&address, "https://web.push.apple.com/phone");

    let mut db = setup.db();
    let tx = db.transaction().unwrap();
    let snoozed = setup.ask_more(&tx, "Snoozed one");
    tx.execute(
        "INSERT INTO snoozes (kind, item, task, snoozed, until)
         VALUES ('ask', ?1, 1, '2026-11-17T12:00:00Z', '2999-01-01T00:00:00Z')",
        [snoozed],
    )
    .unwrap();
    tx.commit().unwrap();
    setup.ask_more(&db, "Awake one");

    let pushes = setup.until_pushes(1);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.pushes().len(), 1, "{pushes:?}");
}

#[test]
fn a_subscription_the_push_service_calls_gone_is_removed() {
    let mut setup = Setup::new("push-gone", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    setup.subscribe(&address, "https://web.push.apple.com/phone");
    fs::write(setup.dir.join("fake/push-status"), "410").unwrap();

    setup.ask_more(&setup.db(), "Anyone there?");

    setup.until_pushes(1);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let left: i64 = setup
            .db()
            .query_row("SELECT count(*) FROM push_subscriptions", [], |row| {
                row.get(0)
            })
            .unwrap();
        if left == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "the gone subscription stayed");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn an_ask_without_a_title_pushes_its_task_and_step_not_its_question() {
    let mut setup = Setup::new("push-untitled", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    let phone = setup.subscribe(&address, "https://web.push.apple.com/phone");

    setup
        .db()
        .execute(
            "INSERT INTO asks (task, step, kind, question, options)
             VALUES (1, 'implement', 'step', 'implement: ran rm -rf /srv with the deploy key', '[]')",
            [],
        )
        .unwrap();

    let pushes = setup.until_pushes(1);
    let plain = decrypt(&base64(&pushes[0]["body"]), &phone.0, &phone.1);
    let payload: Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(payload["title"], "task cli:docs-1 asks at implement");
}

#[test]
fn an_ask_asked_again_pushes_again() {
    let mut setup = Setup::new("push-again", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    setup.subscribe(&address, "https://web.push.apple.com/phone");
    let db = setup.db();
    let ask = setup.ask_more(&db, "Approve the push?");
    setup.until_pushes(1);
    db.execute(
        "UPDATE asks SET answer = 'yes', answered = '2026-11-17T12:00:00Z' WHERE id = ?1",
        [ask],
    )
    .unwrap();

    db.execute(
        "UPDATE asks SET revision = revision + 1, answer = NULL, answered = NULL WHERE id = ?1",
        [ask],
    )
    .unwrap();

    setup.until_pushes(2);
}

#[test]
fn a_failing_intake_and_an_uncertain_prompt_push_an_alarm_with_only_a_title_and_url() {
    let mut setup = Setup::new("push-alarms", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    let phone = setup.subscribe(&address, "https://web.push.apple.com/phone");
    let db = setup.db();

    db.execute(
        "INSERT INTO inbox (id, time, type, subject, data, links, outcome)
         VALUES ('r900', '2026-11-17T09:00:00Z', 'priory.report', 'intake',
           '{\"line\":\"github: HTTP 401: token abc\"}', '[]', 'report')",
        [],
    )
    .unwrap();
    setup.until_pushes(1);
    db.execute(
        "INSERT INTO outbox (task, text, state) VALUES (1, 'the secret plan', 'attempted')",
        [],
    )
    .unwrap();
    db.execute("UPDATE outbox SET state = 'uncertain'", [])
        .unwrap();

    let pushes = setup.until_pushes(2);
    let payloads: Vec<Value> = pushes
        .iter()
        .map(|push| {
            without_id(
                serde_json::from_slice(&decrypt(&base64(&push["body"]), &phone.0, &phone.1))
                    .unwrap(),
            )
        })
        .collect();
    assert_eq!(
        payloads,
        [
            json!({ "title": "the intake fails to poll github", "url": "/" }),
            json!({ "title": "task cli:docs-1 prompt", "url": "/tasks/1" }),
        ]
    );
}

#[test]
fn an_ask_asked_again_before_its_first_push_went_out_pushes_once() {
    let mut setup = Setup::new("push-superseded", SERVE);
    setup.seed();
    // No task starts, so none turns blocked and sends an alarm of its own.
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    setup.subscribe(&address, "https://web.push.apple.com/phone");

    let mut db = setup.db();
    let tx = db.transaction().unwrap();
    let ask = setup.ask_more(&tx, "Approve the push?");
    tx.execute(
        "UPDATE asks SET revision = revision + 1 WHERE id = ?1",
        [ask],
    )
    .unwrap();
    tx.commit().unwrap();
    setup.ask_more(&db, "Later one");

    let pushes = setup.until_pushes(2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(setup.pushes().len(), 2, "{pushes:?}");
}

impl Setup {
    /// The argument lists the fake `notify-send` received, oldest first.
    fn notices(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.join("fake/notify.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Waits until the fake `notify-send` ran `count` times, then a while for any extra call.
    fn until_notices(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.notices().len() < count {
            assert!(
                Instant::now() < deadline,
                "{:?}\n{}",
                self.notices(),
                fs::read_to_string(self.dir.join("server.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(1500));
        self.notices()
    }
}

#[test]
fn desktop_notifications_run_notify_send_once_per_new_top_item_with_its_title_only() {
    let mut setup = Setup::new("notify", &format!("{SERVE}\n[notify]\ndesktop = true\n"));
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    setup.start(&["daemon", &setup.factory()]);
    assert_eq!(
        setup.until_notices(1),
        [json!(["--", "task cli:docs-1 asks at implement"])]
    );

    let db = setup.db();
    db.execute_batch(
        "BEGIN;
         UPDATE tasks SET state = 'blocked', reason = 'tests fail' WHERE id = 2;
         INSERT INTO snoozes (kind, item, task, snoozed, until)
           VALUES ('task', 2, 2, '2026-11-17T11:00:00Z', '2999-01-01T00:00:00Z');
         COMMIT;",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    db.execute(
        "INSERT INTO asks (task, step, kind, question, options, title, text, command)
         VALUES (1, 'implement', 'agent', 'Deploy?', '[\"deploy\",\"wait\"]', 'Deploy the docs?',
           'the brief: private plan', '{\"argv\":[\"make\",\"deploy\"],\"runs\":[\"deploy\"]}')",
        [],
    )
    .unwrap();

    assert_eq!(
        setup.until_notices(2),
        [
            json!(["--", "task cli:docs-1 asks at implement"]),
            json!(["--", "Deploy the docs?"])
        ]
    );
}

#[test]
fn a_restarted_daemon_does_not_notify_the_item_it_already_told() {
    let mut setup = Setup::new(
        "notify-restart",
        &format!("{SERVE}\n[notify]\ndesktop = true\n"),
    );
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    setup.start(&["daemon", &setup.factory()]);
    setup.until_notices(1);
    let mut server = setup.server.take().unwrap();
    server.kill().unwrap();
    server.wait().unwrap();

    setup.start(&["daemon", &setup.factory()]);
    std::thread::sleep(Duration::from_millis(2500));

    assert_eq!(setup.notices().len(), 1, "{:?}", setup.notices());
}

#[test]
fn an_ask_asked_again_and_a_task_blocked_again_notify_again() {
    let mut setup = Setup::new(
        "notify-again",
        &format!("{SERVE}\n[notify]\ndesktop = true\n"),
    );
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    setup.start(&["daemon", &setup.factory()]);
    setup.until_notices(1);

    let db = setup.db();
    db.execute("UPDATE asks SET revision = revision + 1 WHERE task = 1", [])
        .unwrap();
    assert_eq!(setup.until_notices(2).len(), 2);

    db.execute(
        "UPDATE tasks SET state = 'blocked', reason = 'tests fail',
           changed = '2026-11-17T11:00:00Z' WHERE id = 2",
        [],
    )
    .unwrap();
    assert_eq!(setup.until_notices(3).len(), 3);
    db.execute_batch(
        "UPDATE tasks SET state = 'running' WHERE id = 2;
         UPDATE tasks SET state = 'blocked', changed = '2026-11-17T12:00:00Z' WHERE id = 2;",
    )
    .unwrap();
    assert_eq!(setup.until_notices(4).len(), 4, "{:?}", setup.notices());
}

#[test]
fn without_notify_desktop_the_daemon_runs_no_notify_send() {
    let mut setup = Setup::new("notify-off", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    setup.start(&["daemon", &setup.factory()]);

    std::thread::sleep(Duration::from_millis(2500));

    assert_eq!(setup.notices(), Vec::<Value>::new());
}

/// Copies the factory in `from` into `setup`, serving it with the test's token on any port.
fn use_factory(setup: &Setup, from: &Path) {
    let to = setup.dir.join("factory");
    fs::remove_dir_all(&to).unwrap();
    let mut pending = vec![from.to_owned()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let target = to.join(path.strip_prefix(from).unwrap());
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(path, target).unwrap();
        }
    }
    let file = to.join("factory.toml");
    let mut text = fs::read_to_string(&file)
        .unwrap()
        .replace("PRIORY_TOKEN", "TEST_TOKEN")
        .replace(
            "[webhooks.github]\nsecret_env = \"PRIORY_GITHUB_WEBHOOK_SECRET\"",
            "",
        )
        .replace(
            "[webhooks.linear]\nsecret_env = \"PRIORY_LINEAR_WEBHOOK_SECRET\"",
            "",
        );
    if !text.contains("[serve]") {
        text.push_str("\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n");
    }
    fs::write(file, text).unwrap();
    // The reference example names a `deliver` script that its pages do not show.
    let deliver = to.join("bin/deliver");
    if !deliver.exists() {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(deliver.parent().unwrap()).unwrap();
        fs::write(&deliver, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&deliver, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn priory_views_prints_what_the_views_endpoint_serves() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let factories = [
        root.join("examples/groom"),
        root.join("docs/reference/examples/factory"),
    ];
    for (index, from) in factories.iter().enumerate() {
        let mut setup = Setup::new(&format!("views-{index}"), SERVE);
        use_factory(&setup, from);
        let address = setup.start(&["serve", &setup.factory(), "--listen", "127.0.0.1:0"]);

        let views = api(&address, "/api/views");

        let twin = setup.priory(&["views", &setup.factory(), "--json"]);
        succeeded(&twin);
        let twin: Value = serde_json::from_slice(&twin.stdout).unwrap();
        assert_eq!(twin, views, "{from:?}: priory views --json");
    }
}

#[test]
fn the_flows_endpoint_serves_check_explains_json_and_each_graph_as_priory_flows_prints_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let factories = [
        root.join("examples/groom"),
        root.join("docs/reference/examples/factory"),
    ];
    for (index, from) in factories.iter().enumerate() {
        let mut setup = Setup::new(&format!("flows-{index}"), SERVE);
        use_factory(&setup, from);
        let address = setup.start(&["serve", &setup.factory(), "--listen", "127.0.0.1:0"]);

        let flows = api(&address, "/api/flows");

        let twin = setup.priory(&["flows", &setup.factory(), "--json"]);
        succeeded(&twin);
        let twin: Value = serde_json::from_slice(&twin.stdout).unwrap();
        assert_eq!(twin, flows, "{from:?}: priory flows --json");
        let explain = setup.priory(&["check", &setup.factory(), "--explain", "--json"]);
        succeeded(&explain);
        let explain: Value = serde_json::from_slice(&explain.stdout).unwrap();
        for part in ["irreversible", "paths", "tools"] {
            assert_eq!(flows[part], explain[part], "{from:?}: {part}");
        }
        let workflows = flows["workflows"].as_array().unwrap();
        assert!(!workflows.is_empty(), "{from:?}");
        let step_ids = |workflow: &Value| -> Vec<Value> {
            workflow["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|step| step["id"].clone())
                .collect()
        };
        for workflow in workflows {
            assert!(
                step_ids(workflow).contains(&workflow["start"]),
                "{workflow}"
            );
        }
    }

    let mut setup = Setup::new("flows-graph", SERVE);
    let address = setup.start(&["serve", &setup.factory()]);
    let flows = api(&address, "/api/flows");
    let implement = &flows["workflows"][0];
    assert_eq!(implement["name"], "implement");
    assert_eq!(
        implement["steps"][0],
        json!({
            "id": "implement", "decides": "model", "effect": "reversible", "agent": "implementer",
            "kind": "reversible", "edges": [
                { "outcome": "failed", "to": "stop", "by": "model" },
                { "outcome": "passed", "to": "done", "by": "model" },
            ],
        })
    );
}

const LINEAR_HOOKS: &str = "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\n\n[webhooks.linear]\nsecret_env = \"TEST_SECRET\"\n";

/// Linear's webhook timestamp for `PRIORY_NOW`, 2026-11-17T12:00:00Z, in milliseconds.
const NOW_MS: u64 = 1_794_916_800_000;

fn linear_trigger(on: &str) -> String {
    format!(
        "source = \"linear\"\non = \"{on}\"\nfilter = {{ team = \"ENG\" }}\nworkflow = \"implement\"\nkey = \"issue.identifier\"\n"
    )
}

/// A Linear `Issue` delivery for ENG-7. `action` is `create` or `update`, `team` its team key.
fn linear_issue(action: &str, team: &str) -> Value {
    json!({
        "action": action,
        "type": "Issue",
        "actor": { "id": "u1", "type": "user", "name": "max" },
        "data": {
            "id": "issue-7",
            "identifier": format!("{team}-7"),
            "title": "Fix the thing",
            "createdAt": "2026-11-17T09:00:00.000Z",
            "updatedAt": "2026-11-17T10:00:00.000Z",
            "teamId": "team-1",
            "projectId": null,
            "state": { "id": "s2", "name": "In Progress", "type": "started" },
            "team": { "id": "team-1", "key": team, "name": "Engineering" },
            "labels": [{ "id": "l1", "name": "agent" }]
        },
        "url": format!("https://linear.app/acme/issue/{team}-7/fix-the-thing"),
        "createdAt": "2026-11-17T10:00:00.000Z",
        "organizationId": "org-1",
        "webhookTimestamp": NOW_MS,
        "webhookId": "hook-1"
    })
}

/// POSTs `body` to `/hooks/linear` as Linear would, signed with `signature` when given: the bare
/// hex HMAC-SHA256 of the body.
fn deliver_linear(address: &str, signature: Option<&str>, body: &str) -> (u16, String) {
    let signature = signature.map_or(String::new(), |signature| {
        format!("Linear-Signature: {signature}\r\n")
    });
    raw(
        address,
        &format!(
            "POST /hooks/linear HTTP/1.1\r\nHost: {address}\r\nUser-Agent: Linear-Webhook\r\n\
             Content-Type: application/json; charset=utf-8\r\nLinear-Delivery: 234d1a4e-b617-4388-90fe-adc3633d6b72\r\n\
             Linear-Event: Issue\r\n{signature}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

fn linear_signature(secret: &str, body: &str) -> String {
    sign(secret, body.as_bytes()).replace("sha256=", "")
}

impl Setup {
    /// A factory with `[serve]`, the Linear webhook and `linear` triggers on team ENG.
    fn linear_hooks(name: &str) -> Self {
        Self::linear_hooks_on(
            name,
            &["issue.created", "issue.state_changed", "comment.created"],
        )
    }

    fn linear_hooks_on(name: &str, ons: &[&str]) -> Self {
        let setup = Setup::new(name, LINEAR_HOOKS);
        fs::create_dir_all(setup.dir.join("factory/triggers")).unwrap();
        for on in ons {
            fs::write(
                setup.dir.join(format!("factory/triggers/{on}.toml")),
                linear_trigger(on),
            )
            .unwrap();
        }
        setup
    }

    fn send_linear(&self, address: &str, body: &Value) -> (u16, String) {
        let body = body.to_string();
        deliver_linear(address, Some(&linear_signature(&self.secret, &body)), &body)
    }
}

#[test]
fn a_signed_linear_issue_created_delivery_publishes_the_event_a_poll_would() {
    let mut setup = Setup::linear_hooks("linear-created");
    let address = setup.start(&["serve", &setup.factory()]);

    let (status, reply) = setup.send_linear(&address, &linear_issue("create", "ENG"));

    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply, r#"{"outcome":"published"}"#);
    let published = setup.published();
    let kinds: Vec<_> = published.iter().map(|m| m.kind.as_str()).collect();
    assert_eq!(kinds, ["linear.issue.created", "factory.task.requested"]);
    assert_eq!(published[0].subject.as_deref(), Some("ENG-7"));
    assert_eq!(
        published[0].data,
        json!({
            "issue": "ENG-7",
            "linear_id": "issue-7",
            "title": "Fix the thing",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing",
            "team": "ENG",
            "project": null,
            "project_id": null,
            "state": "In Progress",
            "labels": ["agent"],
            "actor": "max",
            "blockers": null,
            "time": "2026-11-17T09:00:00.000Z",
            "delivery": "234d1a4e-b617-4388-90fe-adc3633d6b72",
            "baseline": false,
            "triggers": ["issue.created"]
        })
    );
    assert_eq!(published[1].subject.as_deref(), Some("linear:ENG-7"));
}

fn linear_state_change() -> Value {
    let mut payload = linear_issue("update", "ENG");
    payload["updatedFrom"] = json!({ "stateId": "s1", "updatedAt": "2026-11-17T09:00:00.000Z" });
    payload["createdAt"] = json!("2026-11-17T10:00:00.000Z");
    payload
}

fn linear_comment() -> Value {
    json!({
        "action": "create",
        "type": "Comment",
        "actor": { "id": "u1", "type": "user", "name": "max" },
        "data": {
            "id": "comment-1",
            "createdAt": "2026-11-17T10:30:00.000Z",
            "body": "Looks good",
            "issueId": "issue-7",
            "userId": "u1"
        },
        "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-comment-1",
        "createdAt": "2026-11-17T10:30:00.000Z",
        "webhookTimestamp": NOW_MS
    })
}

impl Setup {
    /// Runs `priory intake --once` with a fake Linear that holds `issues` and `comments`.
    fn poll_linear(&self, issues: Value, comments: Value) {
        let page = |nodes: Value| {
            json!({ "data": { "issues": { "nodes": nodes, "pageInfo": { "hasNextPage": false } },
                "comments": { "nodes": [], "pageInfo": { "hasNextPage": false } } } })
        };
        let answer = |name: &str, body: Value| {
            let file = self.dir.join(name);
            fs::write(&file, body.to_string()).unwrap();
            file.display().to_string()
        };
        let issues = answer("linear-issues.json", page(issues));
        let mut comments_answer = page(json!([]));
        comments_answer["data"]["comments"]["nodes"] = comments;
        let comments = answer("linear-comments.json", comments_answer);
        let empty = answer("linear-blocked.json", page(json!([])));
        let script = format!(
            "#!/bin/sh\ncat >/dev/null\ncase \"$*\" in\n  *'query Issues'*) cat '{issues}' ;;\n  \
             *'query Comments'*) cat '{comments}' ;;\n  *) cat '{empty}' ;;\nesac\n"
        );
        let curl = self.dir.join("bin/curl");
        fs::create_dir_all(curl.parent().unwrap()).unwrap();
        fs::write(&curl, script).unwrap();
        fs::set_permissions(&curl, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            curl.parent().unwrap().display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = self.command();
        command
            .args(["intake", "--once", &self.factory()])
            .env("LINEAR_API_KEY", "lin_api_test")
            .env("PATH", path);
        succeeded(&command.output().unwrap());
    }
}

/// What Linear's API answers for ENG-7 after `linear_issue` was created and moved to In Progress.
fn polled_issue() -> Value {
    json!({
        "id": "issue-7", "identifier": "ENG-7", "title": "Fix the thing",
        "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing",
        "createdAt": "2026-11-17T09:00:00.000Z", "updatedAt": "2026-11-17T10:00:00.000Z",
        "team": { "key": "ENG" }, "project": null, "state": { "name": "In Progress" },
        "labels": { "nodes": [{ "name": "agent" }] }, "creator": { "displayName": "max" },
        "history": { "nodes": [{
            "id": "history-1", "createdAt": "2026-11-17T10:00:03.000Z",
            "fromState": { "name": "Todo" }, "toState": { "name": "In Progress" },
            "actor": { "displayName": "max" }
        }] },
        "inverseRelations": { "nodes": [], "pageInfo": { "hasNextPage": false } },
        "relations": { "nodes": [] }
    })
}

#[test]
fn a_signed_linear_state_change_and_comment_publish_their_poll_types() {
    let mut setup = Setup::linear_hooks_on(
        "linear-state-comment",
        &["issue.created", "issue.state_changed"],
    );
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(setup.send_linear(&address, &linear_state_change()).0, 200);
    assert_eq!(setup.send_linear(&address, &linear_comment()).0, 200);

    let published = setup.published();
    let kinds: Vec<_> = published.iter().map(|m| m.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "linear.issue.state_changed",
            "factory.task.requested",
            "linear.comment.created"
        ]
    );
    let mut state = published[0].data.clone();
    assert_eq!(state["delivery"], "234d1a4e-b617-4388-90fe-adc3633d6b72");
    for field in ["delivery", "triggers"] {
        state.as_object_mut().unwrap().remove(field);
    }
    assert_eq!(
        state,
        json!({
            "issue": "ENG-7", "linear_id": "issue-7", "title": "Fix the thing",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing", "team": "ENG",
            "project": null, "project_id": null, "state": "In Progress", "labels": ["agent"],
            "from": null, "to": "In Progress", "actor": "max", "blockers": null,
            "time": "2026-11-17T10:00:00.000Z", "baseline": false
        })
    );
    assert_eq!(
        published[2].data,
        json!({
            "issue": "ENG-7", "linear_id": "comment-1", "title": "", "body": "Looks good",
            "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-comment-1",
            "team": "ENG", "project": null, "project_id": null, "state": "", "labels": [],
            "actor": "max", "time": "2026-11-17T10:30:00.000Z",
            "delivery": "234d1a4e-b617-4388-90fe-adc3633d6b72", "baseline": false,
            "triggers": []
        })
    );
}

#[test]
fn a_linear_delivery_with_a_wrong_signature_or_a_stale_timestamp_gets_401_before_it_is_read() {
    let mut setup = Setup::linear_hooks("linear-refused");
    let address = setup.start(&["serve", &setup.factory()]);
    let body = linear_issue("create", "ENG").to_string();
    let signatures = [
        Some(linear_signature("not-the-secret", &body)),
        None,
        Some("zz".to_owned()),
        Some(sign(&setup.secret, body.as_bytes())),
    ];
    for signature in &signatures {
        let (status, reply) = deliver_linear(&address, signature.as_deref(), &body);

        assert_eq!(status, 401, "{signature:?}");
        assert_eq!(
            reply,
            r#"{"error":"Linear-Signature is missing or does not match the body"}"#
        );
    }
    let mut stale = linear_issue("create", "ENG");
    stale["webhookTimestamp"] = json!(NOW_MS - 61_000);
    let (status, reply) = setup.send_linear(&address, &stale);
    assert_eq!(status, 401);
    assert_eq!(
        reply,
        r#"{"error":"`webhookTimestamp` is over a minute from now"}"#
    );
    let mut early = linear_issue("create", "ENG");
    early["webhookTimestamp"] = json!(NOW_MS + 61_000);
    assert_eq!(setup.send_linear(&address, &early).0, 401);
    for extreme in [i64::MIN, i64::MAX] {
        let mut body = linear_issue("create", "ENG");
        body["webhookTimestamp"] = json!(extreme);
        assert_eq!(setup.send_linear(&address, &body).0, 401, "{extreme}");
    }
    let mut edge = linear_issue("create", "ENG");
    edge["webhookTimestamp"] = json!(NOW_MS - 60_000);
    assert_eq!(setup.send_linear(&address, &edge).0, 200);
    assert_eq!(setup.published().len(), 2);
}

#[test]
fn a_linear_body_over_1_mib_gets_413_before_it_is_read() {
    let mut setup = Setup::linear_hooks("linear-oversize");
    let address = setup.start(&["serve", &setup.factory()]);

    let (status, reply) = raw(
        &address,
        &format!(
            "POST /hooks/linear HTTP/1.1\r\nHost: {address}\r\nLinear-Signature: 00\r\n\
             Content-Length: {}\r\n\r\n",
            1024 * 1024 + 1
        ),
    );

    assert_eq!(status, 413);
    assert_eq!(reply, r#"{"error":"the body is over 1 MiB"}"#);
    assert!(setup.published().is_empty());
}

#[test]
fn a_linear_delivery_for_a_team_no_trigger_names_or_an_unmapped_event_gets_200_and_publishes_nothing()
 {
    let mut setup = Setup::linear_hooks("linear-dropped");
    let address = setup.start(&["serve", &setup.factory()]);
    let mut removed = linear_issue("remove", "ENG");
    removed["action"] = json!("remove");
    let mut renamed = linear_issue("update", "ENG");
    renamed["updatedFrom"] = json!({ "title": "Old" });
    let mut other_type = linear_issue("create", "ENG");
    other_type["type"] = json!("Project");
    let deliveries = [linear_issue("create", "OPS"), removed, renamed, other_type];

    for body in deliveries {
        let (status, reply) = setup.send_linear(&address, &body);

        assert_eq!(status, 200, "{reply}");
        assert_eq!(reply, r#"{"outcome":"dropped"}"#);
    }
    assert!(setup.published().is_empty());
}

#[test]
fn a_linear_webhook_then_a_poll_of_the_same_changes_make_one_message_each() {
    let mut setup = Setup::linear_hooks("linear-hook-then-poll");
    setup.poll_linear(json!([]), json!([]));
    let address = setup.start(&["serve", &setup.factory()]);
    for body in [
        linear_issue("create", "ENG"),
        linear_state_change(),
        linear_comment(),
    ] {
        assert_eq!(setup.send_linear(&address, &body).0, 200);
    }
    let before = setup.published();

    let mut comment = json!({ "id": "comment-1", "body": "Looks good", "createdAt": "2026-11-17T10:30:00.000Z",
        "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-comment-1",
        "user": { "displayName": "max" }, "issue": polled_issue() });
    comment["issue"]["history"] = json!({ "nodes": [] });
    setup.poll_linear(json!([polled_issue()]), json!([comment]));

    let kinds = |messages: &[herdr_bus::Message]| -> Vec<String> {
        messages
            .iter()
            .filter(|m| m.kind.starts_with("linear.") && !m.kind.contains("baseline"))
            .map(|m| m.kind.clone())
            .collect()
    };
    let after = setup.published();
    // The comment would request a task, so the poll publishes it.
    let mut expected = kinds(&before);
    expected.push("linear.comment.created".into());
    assert_eq!(kinds(&after), expected);
    assert_eq!(
        after
            .iter()
            .filter(|m| m.kind == "factory.task.requested")
            .count(),
        3
    );
}

#[test]
fn a_linear_poll_then_a_webhook_of_the_same_changes_make_one_message_each() {
    let mut setup = Setup::linear_hooks("linear-poll-then-hook");
    setup.poll_linear(json!([]), json!([]));
    let mut comment = json!({ "id": "comment-1", "body": "Looks good", "createdAt": "2026-11-17T10:30:00.000Z",
        "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-comment-1",
        "user": { "displayName": "max" }, "issue": polled_issue() });
    comment["issue"]["history"] = json!({ "nodes": [] });
    setup.poll_linear(json!([polled_issue()]), json!([comment]));
    let before = setup.published();
    assert_eq!(
        before
            .iter()
            .filter(|m| m.kind.starts_with("linear.") && !m.kind.contains("baseline"))
            .count(),
        3
    );
    let address = setup.start(&["serve", &setup.factory()]);

    for body in [
        linear_issue("create", "ENG"),
        linear_state_change(),
        linear_comment(),
    ] {
        let (status, reply) = setup.send_linear(&address, &body);

        // A change that shares the poll's id publishes again, and herdr-bus drops it.
        assert_eq!(status, 200, "{reply}");
    }
    assert_eq!(setup.published(), before);
}

#[test]
fn a_redelivered_linear_state_change_with_a_new_timestamp_makes_one_message() {
    let mut setup = Setup::linear_hooks("linear-redelivery");
    let address = setup.start(&["serve", &setup.factory()]);
    assert_eq!(setup.send_linear(&address, &linear_state_change()).0, 200);
    let first = setup.published();
    let mut again = linear_state_change();
    again["webhookTimestamp"] = json!(NOW_MS + 5_000);

    assert_eq!(setup.send_linear(&address, &again).0, 200);

    assert_eq!(setup.published(), first);
}

#[test]
fn a_linear_poll_consumes_one_webhook_state_change_per_history_entry() {
    let mut setup = Setup::linear_hooks("linear-repeated");
    setup.poll_linear(json!([]), json!([]));
    let address = setup.start(&["serve", &setup.factory()]);
    assert_eq!(setup.send_linear(&address, &linear_state_change()).0, 200);
    let mut issue = polled_issue();
    let entry = issue["history"]["nodes"][0].clone();
    let mut second = entry.clone();
    second["id"] = json!("history-2");
    second["createdAt"] = json!("2026-11-17T10:00:20.000Z");
    issue["history"]["nodes"] = json!([entry, second]);

    setup.poll_linear(json!([issue]), json!([]));

    let changes = setup
        .published()
        .iter()
        .filter(|m| m.kind == "linear.issue.state_changed")
        .count();
    assert_eq!(changes, 2);
}

#[test]
fn a_linear_comment_publishes_only_when_it_would_request_no_task() {
    let mut setup = Setup::linear_hooks_on("linear-comment-only", &["issue.created"]);
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(setup.send_linear(&address, &linear_comment()).0, 200);
    assert_eq!(setup.send_linear(&address, &linear_state_change()).0, 200);

    let kinds: Vec<_> = setup.published().into_iter().map(|m| m.kind).collect();
    assert_eq!(
        kinds,
        ["linear.comment.created", "linear.issue.state_changed"]
    );
    let mut setup = Setup::linear_hooks("linear-comment-triggered");
    let address = setup.start(&["serve", &setup.factory()]);

    let (status, reply) = setup.send_linear(&address, &linear_comment());

    assert_eq!(status, 200);
    assert_eq!(reply, r#"{"outcome":"dropped"}"#);
    assert!(setup.published().is_empty());
}

#[test]
fn a_linear_issue_whose_project_the_payload_names_only_by_id_waits_for_the_poll() {
    let mut setup = Setup::linear_hooks("linear-project-id");
    let address = setup.start(&["serve", &setup.factory()]);
    let mut body = linear_issue("create", "ENG");
    body["data"]["projectId"] = json!("project-1");

    let (status, reply) = setup.send_linear(&address, &body);

    assert_eq!(status, 200);
    assert_eq!(reply, r#"{"outcome":"dropped"}"#);
    assert!(setup.published().is_empty());
}

/// Publishes a comment on pull request `pull_request` that no active task owns, sent at `time`.
fn unrouted_comment(setup: &Setup, pull_request: u64, time: &str) {
    let message = herdr_bus::Message::new(
        "github.pull_request.comment",
        json!({ "repo": "owner/repo", "pull_request": pull_request, "actor": "ana", "body": "Still broken on mobile.\nSee the log." }),
    )
    .subject(format!("owner/repo#{pull_request}"))
    .time(time.parse().unwrap());
    setup.bus().publish("factory.intake", &message).unwrap();
}

impl Setup {
    /// A task in `state` that owned `resources`, added after the events routed, as a task that ended.
    fn owner(&self, state: &str, resources: &[&str]) -> i64 {
        let db = self.db();
        db.execute(
            "INSERT INTO tasks (key, workflow, step, state, trigger, source) VALUES (?1, 'implement', 'implement', ?2, 'cli', '-')",
            [resources[0], state],
        )
        .unwrap();
        let task = db.last_insert_rowid();
        for resource in resources {
            db.execute(
                "INSERT INTO owns (task, resource) VALUES (?1, ?2)",
                rusqlite::params![task, resource],
            )
            .unwrap();
        }
        task
    }
}

#[test]
fn unrouted_events_split_into_ended_owners_with_their_follow_up_and_a_count_per_source() {
    let mut setup = Setup::new("unrouted", SERVE);
    // An issue id no shell should run, whose owner is still active, so its follow-up names it.
    let hostile = herdr_bus::Message::new(
        "linear.comment.created",
        json!({ "issue": "ENG-7$(id)", "body": "Hm." }),
    )
    .subject("ENG-7")
    .time("2026-11-17T10:00:00Z".parse().unwrap());
    setup.bus().publish("factory.intake", &hostile).unwrap();
    unrouted_comment(&setup, 7, "2026-11-17T11:00:00Z");
    unrouted_comment(&setup, 8, "2026-11-17T11:30:00Z");
    unrouted_comment(&setup, 9, "2026-11-17T11:40:00Z");
    let linear = herdr_bus::Message::new(
        "linear.comment.created",
        json!({ "issue": "ENG-9", "body": "Hm." }),
    )
    .subject("ENG-9")
    .time("2026-11-17T11:50:00Z".parse().unwrap());
    setup.bus().publish("factory.intake", &linear).unwrap();
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    let done = setup.owner("done", &["github:owner/repo#7"]);
    let dropped = setup.owner("dropped", &["github:owner/repo#8"]);
    let active = setup.owner("asleep", &["linear:ENG-7$(id)"]);
    let address = setup.start(&["serve", &setup.factory()]);

    let served = api(&address, "/api/inbox?unrouted=1");
    assert_eq!(
        served["unrouted"]["owned"][2]["follow_up"],
        format!("priory subscribe {active} 'linear:ENG-7$(id)'")
    );

    let output = setup.priory(&["inbox", "--unrouted", "--json"]);
    succeeded(&output);
    assert_eq!(
        served,
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    );
    let owned: Vec<_> = served["unrouted"]["owned"]
        .as_array()
        .unwrap()
        .iter()
        .take(2)
        .map(|group| {
            (
                group["resource"].clone(),
                group["task"]["id"].clone(),
                group["task"]["state"].clone(),
                group["events"][0]["line"].clone(),
                group["follow_up"].clone(),
            )
        })
        .collect();
    assert_eq!(
        owned,
        [
            (
                json!("github:owner/repo#8"),
                json!(dropped),
                json!("dropped"),
                json!("Still broken on mobile."),
                json!(format!("priory revive {dropped}"))
            ),
            (
                json!("github:owner/repo#7"),
                json!(done),
                json!("done"),
                json!("Still broken on mobile."),
                json!(format!(
                    "priory add --workflow implement 'Follow up on https://github.com/owner/repo/issues/7 after event {}'",
                    served["unrouted"]["owned"][1]["events"][0]["seq"]
                ))
            ),
        ]
    );
    let text = served["unrouted"]["owned"][1]["request_text"]
        .as_str()
        .unwrap();

    // The follow-up's task owns the resource, so the next comment on #7 reaches it.
    succeeded(&setup.priory(&["add", &setup.factory(), "--workflow", "implement", text]));
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    unrouted_comment(&setup, 7, "2026-11-17T11:58:00Z");
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    let inbox = json_of(&setup, "inbox");
    let last = inbox["inbox"].as_array().unwrap().last().unwrap().clone();
    let task: Value = serde_json::from_slice(
        &setup
            .priory(&["task", &last["task"].to_string(), "--json"])
            .stdout,
    )
    .unwrap();
    assert_eq!(
        [&last["type"], &task["key"]],
        [
            &json!("github.pull_request.comment"),
            &json!("github:owner/repo#7")
        ],
        "{last}"
    );
    assert_eq!(
        served["unrouted"]["other"],
        json!([{ "source": "github", "events": 1 }, { "source": "linear", "events": 1 }])
    );
    let text = String::from_utf8(setup.priory(&["inbox", "--unrouted"]).stdout).unwrap();
    assert!(!text.contains("TASK"), "{text}");
    assert!(
        text.contains(&format!(
            "github:owner/repo#8: 1 event, task {dropped} dropped\n"
        )),
        "{text}"
    );
    assert!(
        text.ends_with("No task ever owned: 1 github event, 1 linear event.\n"),
        "{text}"
    );
    let (status, _) = get(&address, "/api/inbox?unrouted=2", Some(TOKEN));
    assert_eq!(status, 400);
}

#[test]
fn an_outsider_comment_is_counted_on_the_desk_and_listed_as_a_group_of_its_own() {
    let setup = Setup::new("desk-outsider", SERVE);
    let message = herdr_bus::Message::new(
        "github.pull_request.comment",
        json!({
            "repo": "owner/repo", "pull_request": 7, "actor": "stranger",
            "association": "NONE", "body": "Looks wrong.",
        }),
    )
    .subject("owner/repo#7")
    .time("2026-11-17T11:00:00Z".parse().unwrap());
    setup.bus().publish("factory.intake", &message).unwrap();
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));

    let desk = json_of(&setup, "desk");
    let output = setup.priory(&["inbox", "--unrouted", "--json"]);
    succeeded(&output);
    let unrouted: Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = setup.priory(&["desk"]);
    succeeded(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    let listed = setup.priory(&["inbox", "--unrouted"]);
    succeeded(&listed);
    let listed = String::from_utf8(listed.stdout).unwrap();

    assert_eq!(desk["counts"]["outsider"], 1);
    assert_eq!(desk["counts"]["unrouted"], 0);
    assert_eq!(unrouted["unrouted"], json!({ "owned": [], "other": [] }));
    assert_eq!(unrouted["outsider"][0]["subject"], "owner/repo#7");
    assert_eq!(unrouted["outsider"][0]["actor"], "stranger");
    assert!(
        text.starts_with(
            "1 comment from outside the repository came in the last 24 hours. See it with `priory inbox --unrouted`.\n"
        ),
        "{text}"
    );
    assert!(
        listed.starts_with("from outside the repository: 1 event\n"),
        "{listed}"
    );
}

#[test]
fn the_desk_counts_the_last_24_hours_unrouted_events_of_ended_tasks_in_a_calm_line() {
    let mut setup = Setup::new("desk-unrouted", SERVE);
    unrouted_comment(&setup, 7, "2026-11-16T11:59:59Z");
    unrouted_comment(&setup, 8, "2026-11-16T12:00:00Z");
    unrouted_comment(&setup, 9, "2026-11-17T11:00:00Z");
    unrouted_comment(&setup, 10, "2026-11-17T12:00:01Z");
    unrouted_comment(&setup, 11, "2026-11-17T11:00:00Z");
    succeeded(&setup.priory(&["daemon", &setup.factory(), "--once"]));
    setup.owner(
        "done",
        &[
            "github:owner/repo#7",
            "github:owner/repo#8",
            "github:owner/repo#9",
            "github:owner/repo#10",
        ],
    );
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(api(&address, "/api/desk")["counts"]["unrouted"], 2);
    assert_eq!(json_of(&setup, "desk")["counts"]["unrouted"], 2);
    let output = setup.priory(&["desk"]);
    succeeded(&output);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with(
            "2 events reached no task in the last 24 hours. See them with `priory inbox --unrouted`.\n"
        ),
        "{text}"
    );
}

#[test]
fn the_subscribe_endpoint_subscribes_a_task_as_priory_subscribe_does() {
    let mut setup = Setup::new("web-subscribe", SERVE);
    setup.seed();
    succeeded(&setup.priory(&["pause"]));
    let address = setup.start(&["daemon", &setup.factory()]);
    let subscribe =
        |extra: &str, body: Value| post(&address, "/api/subscribe", extra, &body.to_string());
    let pull = "github:owner/repo#7";

    let foreign = subscribe(
        "Origin: http://evil.example\r\n",
        json!({ "task": 1, "resource": pull }),
    );
    let malformed = subscribe("", json!({ "task": 1, "resource": "owner/repo#7" }));
    let stale = subscribe("", json!({ "task": 1, "resource": pull, "revision": 0 }));
    let missing = subscribe("", json!({ "task": 9, "resource": pull }));
    let revision = json_of(&setup, "desk")["revision"].clone();
    let done = subscribe(
        "",
        json!({ "task": 1, "resource": pull, "revision": revision }),
    );
    let taken = subscribe("", json!({ "task": 2, "resource": pull }));

    assert_eq!(foreign.0, 403, "{}", foreign.1);
    assert_eq!(malformed.0, 400, "{}", malformed.1);
    assert_eq!(stale.0, 409, "{}", stale.1);
    assert_eq!(missing.0, 404, "{}", missing.1);
    assert_eq!(
        done,
        (
            200,
            json!({ "ok": "task 1 subscribes to github:owner/repo#7" }).to_string()
        )
    );
    assert_eq!(taken.0, 422, "{}", taken.1);
    assert!(taken.1.contains("task 1"), "{}", taken.1);
    let output = setup.priory(&["task", "1", "--json"]);
    succeeded(&output);
    let task: Value = serde_json::from_slice(&output.stdout).unwrap();
    let resources: Vec<_> = task["subscriptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["resource"].clone())
        .collect();
    assert!(resources.contains(&json!(pull)), "{resources:?}");
}
