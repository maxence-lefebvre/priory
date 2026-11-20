use herdr_bus::{Bus, Output as BusOutput};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

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

/// A factory, a bus, a state file and Priory home under the test's own directory. Dropping it kills
/// the server it started and the tmux server of its daemon.
struct Setup {
    dir: PathBuf,
    server: Option<Child>,
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
        fs::create_dir_all(dir.join("tmux")).unwrap();
        fs::create_dir_all(dir.join("fake")).unwrap();
        let setup = Setup { dir, server: None };
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
            .env("TMUX_TMPDIR", self.dir.join("tmux"))
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("FAKE_DIR", self.dir.join("fake"))
            .env("TEST_TOKEN", TOKEN);
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
        let _ = Command::new("tmux")
            .env("TMUX_TMPDIR", self.dir.join("tmux"))
            .arg("kill-server")
            .output();
    }
}

fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
        tasks
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
    let ask = &asks[0];
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
        "id\ttask\trevision\tquestion\toptions\tsnoozed_until\tback_early\n1\t1\t2\tWhich way?\\nPick one.\t[\"left\",\"right\"]\t-\t-\n"
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
    assert_eq!(inbox.as_array().unwrap().len(), text.lines().count() - 1);
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
    assert_eq!(setup.shown(&[]), expected);
    assert_eq!(setup.shown(&["1"]), expected);
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
        (stray, r#"pick "2026-11-20_sentry_fnd-2409": "Maybe" is not one of its options"#),
        (commented, r#"section "backlog": comments must be empty"#),
    ];

    for (report, why) in cases {
        let put = setup.put("1", &report);

        assert!(!put.status.success());
        let stderr = String::from_utf8_lossy(&put.stderr);
        assert!(
            stderr.starts_with("priory report put: the report is refused: ") && stderr.contains(why),
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
    succeeded(&setup.put("1", &first));
    let address = setup.start(&["serve", &setup.factory()]);

    assert_eq!(api(&address, "/api/reports/latest"), setup.shown(&[]));
    assert_eq!(api(&address, "/api/reports/latest")["task"], 1);
    assert_eq!(api(&address, "/api/reports/2"), setup.shown(&["2"]));
    assert_eq!(get(&address, "/api/reports/3", Some(TOKEN)).0, 404);
}
