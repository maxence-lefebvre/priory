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
    assert_eq!(api(&address, "/api/asks?band=now"), cli);
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
                        { "panel": "reports" }
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
            path.extension()
                .is_some_and(|ext| ext != "txt" && ext != "md")
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

    let (status, head, index) = fetch(&address, "/asks/42");
    assert_eq!(status, 200);
    assert_eq!(index, fetch(&address, "/").2);
    assert_eq!(
        header(&head, "content-security-policy"),
        Some(
            "default-src 'none'; script-src 'self'; style-src 'self'; font-src 'self'; \
             img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; \
             frame-ancestors 'none'"
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
                "trigger": null, "why": "first in the queue",
            },
            {
                "at": "2026-11-17T20:00:00Z", "kind": "held", "task": 2, "key": "cli:docs-2",
                "trigger": null, "why": "held until 2026-11-17T20:00:00Z",
            },
            {
                "at": "2026-11-17T23:00:00Z", "kind": "scheduled", "task": null, "key": null,
                "trigger": "nightly", "why": "schedule nightly fires",
            },
        ])
    );
    assert_eq!(api(&address, "/api/queue"), queue);
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
    assert!(prompt.starts_with("left"), "{prompt}");
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
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
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
fn a_stale_revision_gets_409_with_the_current_ask_and_changes_nothing() {
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
    assert_eq!(body["ask"], before);
    assert_eq!(setup.ask(), before);
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
