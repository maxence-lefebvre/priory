//! Evidence: a step's `record` command, the casts an agent adds, the API and the pull request body
//! (docs/daemon/evidence.md).

use super::*;
use sha2::Digest as _;

/// One model step whose result runs the factory's `record` command.
const RECORDED: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
record = "record"
next = { passed = "done", failed = "stop" }
"#;

/// A factory that serves its state and names two secrets beside its token.
const SERVED: &str = "name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"TEST_TOKEN\"\nredact_env = [\"TEST_SECRET\", \"SHORT_SECRET\"]\nurl = \"https://runner.example:7390\"\n";

const SECRET: &str = "hunter2-evidence-value";
/// A secret under the 8 bytes the screen's redaction skips.
const SHORT: &str = "q7Zx9w";

/// A PNG's signature and a few bytes, which the type check reads as a PNG.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake image";

/// A `record` command that writes one PNG with `caption`, and what it saw to files beside it.
fn png_recorder(caption: &str) -> String {
    format!(
        "#!/bin/sh\nhere=$(dirname \"$0\")\nls -A > \"$here/seen\"\nenv > \"$here/env\"\nls -A \"$HOME\" > \"$here/home\"\n\
         printf '\\211PNG\\r\\n\\032\\nfake image' > \"$PRIORY_EVIDENCE/login.png\"\n\
         printf '%s\\n' {caption:?} > \"$PRIORY_EVIDENCE/login.png.caption\"\n"
    )
}

/// A cast whose output events are `events`, with `header` extra fields.
fn cast(header: &str, events: &[&str]) -> String {
    let mut text = format!("{{\"version\": 2, \"width\": 80, \"height\": 24{header}}}\n");
    for (at, data) in events.iter().enumerate() {
        text.push_str(&format!(
            "[{}.5, \"o\", {}]\n",
            at,
            serde_json::to_string(data).unwrap()
        ));
    }
    text
}

impl Setup {
    /// A factory that serves its state on `workflow`, whose `record` command is `script`.
    fn recording(name: &str, workflow: &str, script: &str) -> Setup {
        let setup = Setup::with_workflow(name, workflow);
        fs::write(setup.dir.join("factory/factory.toml"), SERVED).unwrap();
        let record = setup.dir.join("factory/record");
        fs::write(&record, script).unwrap();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o755)).unwrap();
        setup
    }

    /// Starts the daemon with the server's token and the two secrets in its environment.
    fn start_recording(&mut self) {
        self.start_with(
            Path::new(env!("CARGO_BIN_EXE_priory")),
            &[
                ("TEST_TOKEN", SERVE_TOKEN),
                ("TEST_SECRET", SECRET),
                ("SHORT_SECRET", SHORT),
            ],
        );
    }

    /// Task `task`'s first result, once its `record` command ended.
    fn recorded(&self, task: i64) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let output = self.priory(&["task", &task.to_string(), "--json"]);
            let shown: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or_default();
            let result = shown["results"][0].clone();
            let state = result["record"]["state"].as_str().unwrap_or("queued");
            if state != "queued" {
                return result;
            }
            assert!(
                Instant::now() < deadline,
                "the record never ended: {result}\ndaemon: {}",
                fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Task `task`'s items, as `priory evidence TASK --json` lists them.
    fn evidence(&self, task: i64) -> Vec<serde_json::Value> {
        let output = self.priory(&["evidence", &task.to_string(), "--json"]);
        assert!(output.status.success(), "{output:?}");
        let listed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        listed["evidence"].as_array().unwrap().clone()
    }

    /// Waits until task `task` has `count` items, and returns them.
    fn until_evidence(&self, task: i64, count: usize) -> Vec<serde_json::Value> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let output = self.priory(&["evidence", &task.to_string(), "--json"]);
            let listed: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or_default();
            let items = listed["evidence"].as_array().cloned().unwrap_or_default();
            if output.status.success() && items.len() == count {
                return items;
            }
            assert!(
                Instant::now() < deadline,
                "task {task} has {items:?}\ndaemon: {}",
                fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Where the daemon's server listens, read from its log.
    fn address(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let log = fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default();
            if let Some(address) = log
                .lines()
                .find_map(|line| line.trim().strip_prefix("priory: serving on http://"))
            {
                return address.to_owned();
            }
            assert!(Instant::now() < deadline, "the daemon never served: {log}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `priory evidence add FILE --caption CAPTION` from task 1's pane, in its worktree.
    fn add(&self, session: &str, file: &str, caption: &str) -> Exit {
        self.command(env!("CARGO_BIN_EXE_priory"))
            .env("PRIORY_SESSION", session)
            .current_dir(self.dir.join("home/worktrees/owner/repo/1"))
            .args(["evidence", "add", file, "--caption", caption])
            .output()
            .unwrap()
    }
}

/// A `GET` of `path` with the token, or without it, and the status, head and body it answers.
fn get(address: &str, path: &str, token: bool) -> (u16, String, Vec<u8>) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(address).unwrap();
    let auth = match token {
        true => format!("Authorization: Bearer {SERVE_TOKEN}\r\n"),
        false => String::new(),
    };
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\n{auth}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(response[..split].to_vec()).unwrap();
    let status = head[9..12].parse().unwrap();
    (status, head, response[split + 4..].to_vec())
}

fn stderr(output: &Exit) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn a_record_command_runs_in_a_checkout_of_the_results_commit_with_a_short_environment() {
    let mut setup = Setup::recording(
        "evidence-checkout",
        RECORDED,
        &png_recorder("The login page"),
    );
    setup.calls(&[result("passed")]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start_recording();
    setup.written("fake/submitted");
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::write(
        worktree.join("uncommitted.env"),
        format!("TOKEN={SECRET}\n"),
    )
    .unwrap();

    setup.release("hold-call");
    let result = setup.recorded(1);

    assert_eq!(result["record"]["state"], "recorded", "{result}");
    let seen = fs::read_to_string(setup.dir.join("factory/seen")).unwrap();
    assert!(seen.lines().any(|name| name == "README.md"), "{seen}");
    assert!(!seen.contains("uncommitted.env"), "{seen}");
    let env = fs::read_to_string(setup.dir.join("factory/env")).unwrap();
    let names: Vec<&str> = env
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name))
        .collect();
    let allowed = [
        "PATH",
        "LANG",
        "TERM",
        "HOME",
        "PRIORY_EVIDENCE",
        "PRIORY_TASK",
        "PRIORY_COMMIT",
        // What sh sets for itself.
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
    ];
    assert!(names.iter().all(|name| allowed.contains(name)), "{names:?}");
    assert!(!env.contains(SECRET) && !env.contains(SERVE_TOKEN), "{env}");
    let home = fs::read_to_string(setup.dir.join("factory/home")).unwrap();
    assert_eq!(home, "", "HOME is a new, empty folder");
    assert_eq!(result["record"]["commit_sha"], json!(setup.head()));
}

#[test]
fn a_record_commands_png_is_the_results_attachment_and_the_endpoint_serves_its_bytes() {
    let caption = "The login page <script>alert(1)</script>";
    let mut setup = Setup::recording("evidence-png", RECORDED, &png_recorder(caption));
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.start_recording();
    let address = setup.address();

    let result = setup.recorded(1);

    assert_eq!(result["record"]["state"], "recorded", "{result}");
    assert_eq!(result["record"]["reason"], json!(null));
    let (status, _, body) = get(&address, "/api/tasks/1", true);
    assert_eq!(status, 200);
    let task: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let attachments = task["results"][0]["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1, "{attachments:?}");
    let item = &attachments[0];
    let sha256: String = sha2::Sha256::digest(PNG)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        [
            &item["kind"],
            &item["caption"],
            &item["commit_sha"],
            &item["bytes"],
            &item["sha256"],
            &item["approved"],
        ],
        [
            &json!("image"),
            &json!(caption),
            &json!(setup.head()),
            &json!(PNG.len()),
            &json!(sha256),
            &json!(false),
        ]
    );
    let id = item["id"].as_i64().unwrap();
    let (status, head, bytes) = get(&address, &format!("/api/evidence/{id}"), true);
    assert_eq!(status, 200);
    assert_eq!(bytes, PNG);
    for header in [
        "Content-Type: image/png",
        "Content-Disposition: inline",
        "X-Content-Type-Options: nosniff",
        "Content-Security-Policy: sandbox",
    ] {
        assert!(head.contains(header), "{head}");
    }
    let (unauthorized, _, _) = get(&address, &format!("/api/evidence/{id}"), false);
    assert_eq!(unauthorized, 401);
    let (missing, _, _) = get(&address, "/api/evidence/99", true);
    assert_eq!(missing, 404);
    let stored = setup.dir.join(format!("home/evidence/1/{sha256}.png"));
    assert_eq!(fs::read(stored).unwrap(), PNG);
}

#[test]
fn a_checkout_whose_tracked_file_holds_a_secret_is_not_recorded() {
    let mut setup = Setup::recording(
        "evidence-tracked",
        RECORDED,
        &png_recorder("The login page"),
    );
    setup.calls(&[result("passed")]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start_recording();
    setup.written("fake/submitted");
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::create_dir_all(worktree.join("tests/fixtures")).unwrap();
    fs::write(
        worktree.join("tests/fixtures/login.json"),
        format!("{{\"token\": \"{SECRET}\"}}\n"),
    )
    .unwrap();
    for args in [
        &["add", "tests/fixtures/login.json"][..],
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(git(&worktree).args(args).status().unwrap().success());
    }

    setup.release("hold-call");
    let result = setup.recorded(1);

    assert_eq!(result["record"]["state"], "refused", "{result}");
    let reason = result["record"]["reason"].as_str().unwrap();
    assert_eq!(
        reason,
        "the checkout's tracked file tests/fixtures/login.json holds the value of the secret variable TEST_SECRET: nothing was recorded"
    );
    assert!(!reason.contains(SECRET));
    assert_eq!(result["attachments"], json!([]));
    assert!(!setup.dir.join("factory/seen").exists(), "the command ran");
}

#[test]
fn a_record_command_that_exits_1_adds_no_item_and_leaves_the_result() {
    let script = format!("{}echo the page never loaded\nexit 1\n", png_recorder("x"));
    let mut setup = Setup::recording("evidence-exit", RECORDED, &script);
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.start_recording();

    let result = setup.recorded(1);

    assert_eq!(
        [&result["record"]["state"], &result["record"]["reason"]],
        [&json!("failed"), &json!("exit 1: the page never loaded")]
    );
    assert_eq!(result["attachments"], json!([]));
    assert_eq!(
        [&result["outcome"], &result["summary"]],
        [&json!("passed"), &json!("passed it")]
    );
    assert_eq!(setup.evidence(1), Vec::<serde_json::Value>::new());
    setup.until("tasks", |rows| rows[0][4] == "done");
}

#[test]
fn a_disguised_html_file_an_svg_an_11_mb_image_and_a_twenty_first_item_are_refused() {
    let script = "#!/bin/sh\ncd \"$PRIORY_EVIDENCE\"\n\
        for n in 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16 17 18 19 20 21; do\n\
          printf '\\211PNG\\r\\n\\032\\n%s' \"$n\" > \"a$n.png\"; echo \"Shot $n\" > \"a$n.png.caption\"\n\
        done\n\
        printf '<html><script>alert(1)</script></html>' > fake.png; echo Fake > fake.png.caption\n\
        printf '<svg xmlns=\"http://www.w3.org/2000/svg\"/>' > logo.svg; echo Logo > logo.svg.caption\n\
        { printf '\\211PNG\\r\\n\\032\\n'; head -c 11534336 /dev/zero; } > big.png; echo Big > big.png.caption\n";
    let mut setup = Setup::recording("evidence-refused", RECORDED, script);
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));
    setup.start_recording();

    let result = setup.recorded(1);

    assert_eq!(result["record"]["state"], "recorded", "{result}");
    assert_eq!(result["attachments"].as_array().unwrap().len(), 20);
    let reason = result["record"]["reason"].as_str().unwrap();
    let reasons: Vec<&str> = reason.split("; ").collect();
    assert_eq!(
        reasons,
        [
            "big.png: the file is 11534344 bytes, over the limit of 10 MB",
            "a21.png: task 1 already has 20 items, the most it may have",
            "fake.png: an SVG or HTML file is refused: it can carry script",
            "logo.svg: an SVG or HTML file is refused: it can carry script",
        ]
    );
}

#[test]
fn an_agent_adds_a_cast_from_its_pane_and_nothing_else() {
    let mut setup = Setup::recording("evidence-add", WORKFLOW, "#!/bin/sh\n");
    setup.calls(&[result("passed")]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start_recording();
    setup.written("fake/submitted");
    let session = setup.rows("runs")[0][4].clone();
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    fs::write(
        worktree.join("session.cast"),
        cast(
            "",
            &["cargo run -- migrate\r\n", "applied 2 migrations\r\n"],
        ),
    )
    .unwrap();
    fs::write(worktree.join("shot.png"), PNG).unwrap();
    let outside = setup.dir.join("outside.cast");
    fs::write(&outside, cast("", &["hello\r\n"])).unwrap();
    std::os::unix::fs::symlink(&outside, worktree.join("linked.cast")).unwrap();

    let added = setup.add(&session, "session.cast", "The migration, run twice");
    let image = setup.add(&session, "shot.png", "The login page");
    let away = setup.add(&session, outside.to_str().unwrap(), "Outside");
    let linked = setup.add(&session, "linked.cast", "Linked");
    let paneless = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .env_remove("PRIORY_SESSION")
        .current_dir(&worktree)
        .args(["evidence", "add", "session.cast", "--caption", "No pane"])
        .output()
        .unwrap();

    assert!(added.status.success(), "{}", stderr(&added));
    assert_eq!(
        String::from_utf8_lossy(&added.stdout),
        format!("evidence 1 added to task 1 at {}\n", setup.head())
    );
    assert_eq!(
        (image.status.code(), stderr(&image).as_str()),
        (
            Some(1),
            "priory evidence add: only a step's record command adds an image\n"
        )
    );
    assert_eq!(
        (away.status.code(), stderr(&away)),
        (
            Some(1),
            format!(
                "priory evidence add: {} is not a file inside the run's worktree\n",
                outside.display()
            )
        )
    );
    assert_eq!(
        (linked.status.code(), stderr(&linked)),
        (
            Some(1),
            format!(
                "priory evidence add: {} is not a file inside the run's worktree\n",
                worktree.join("linked.cast").display()
            )
        )
    );
    assert_eq!(
        (paneless.status.code(), stderr(&paneless).as_str()),
        (
            Some(1),
            "priory evidence add: PRIORY_SESSION is unset; run it from a run's pane\n"
        )
    );
    let items = setup.evidence(1);
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(
        [&items[0]["kind"], &items[0]["caption"], &items[0]["run"]],
        [
            &json!("cast"),
            &json!("The migration, run twice"),
            &json!(1)
        ]
    );
    let rows = setup.listed(&["evidence", "1"]);
    assert_eq!(
        [&rows[0][2], &rows[0][3], &rows[0][8]],
        ["cast", "The migration, run twice", "-"]
    );
    let address = setup.address();
    let (status, head, body) = get(&address, "/api/evidence/1", true);
    assert_eq!(status, 200);
    assert!(
        head.contains("Content-Type: text/plain; charset=utf-8"),
        "{head}"
    );
    assert!(!head.contains("Content-Disposition"), "{head}");
    assert_eq!(body, fs::read(worktree.join("session.cast")).unwrap());
    setup.release("hold-call");
    setup.until("tasks", |rows| rows[0][4] == "done");
    assert_eq!(
        setup.task_json("1")["results"][0]["attachments"][0]["caption"],
        "The migration, run twice"
    );
}

#[test]
fn a_cast_or_caption_that_holds_a_secret_is_refused_without_the_value() {
    let mut setup = Setup::recording("evidence-secret", WORKFLOW, "#!/bin/sh\n");
    setup.calls(&[result("passed")]);
    setup.hold("hold-call");
    setup.publish(&requested(42));
    setup.start_recording();
    setup.written("fake/submitted");
    let session = setup.rows("runs")[0][4].clone();
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    let (head, tail) = SECRET.split_at(7);
    let casts = [
        ("printed.cast", cast("", &[&format!("token: {SECRET}\r\n")])),
        (
            "split.cast",
            cast("", &[&format!("token: {head}"), &format!("{tail}\r\n")]),
        ),
        (
            "header.cast",
            cast(
                &format!(", \"env\": {{\"TOKEN\": \"{SECRET}\"}}"),
                &["ok\r\n"],
            ),
        ),
        ("short.cast", cast("", &[&format!("pin {SHORT}\r\n")])),
        ("clean.cast", cast("", &["ok\r\n"])),
    ];
    for (name, text) in &casts {
        fs::write(worktree.join(name), text).unwrap();
    }

    let refused: Vec<Exit> = casts[..4]
        .iter()
        .map(|(name, _)| setup.add(&session, name, "A run"))
        .collect();
    let caption = setup.add(&session, "clean.cast", &format!("Ran with {SECRET}"));

    let wanted = [
        "the cast holds the value of the secret variable TEST_SECRET",
        "the cast holds the value of the secret variable TEST_SECRET",
        "the cast holds the value of the secret variable TEST_SECRET",
        "the cast holds the value of the secret variable SHORT_SECRET",
    ];
    for (output, wanted) in refused.iter().zip(wanted) {
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(stderr(output), format!("priory evidence add: {wanted}\n"));
    }
    assert_eq!(
        stderr(&caption),
        "priory evidence add: the caption holds the value of the secret variable TEST_SECRET\n"
    );
    let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap();
    assert!(!log.contains(SECRET) && !log.contains(SHORT), "{log}");
    assert_eq!(setup.evidence(1), Vec::<serde_json::Value>::new());
}

/// `OPEN_PR` with a `record` command on its implement step.
fn open_pr_recorded() -> String {
    OPEN_PR.replacen(
        "prompt = \"Implement the issue.\"\n",
        "prompt = \"Implement the issue.\"\nrecord = \"record\"\n",
        1,
    )
}

/// The body of the last `gh pr create` the fake `gh` got.
fn body(setup: &Setup) -> String {
    let created = setup.gh().pop().unwrap();
    let at = created.iter().position(|arg| arg == "--body").unwrap();
    created[at + 1].clone()
}

#[test]
fn the_pull_request_body_lists_each_item_with_a_link_to_the_task_page() {
    let script = "#!/bin/sh\ncd \"$PRIORY_EVIDENCE\"\n\
        printf '\\211PNG\\r\\n\\032\\none' > login.png; echo 'The login page' > login.png.caption\n\
        printf '\\211PNG\\r\\n\\032\\ntwo' > cart.png; echo 'The empty cart' > cart.png.caption\n";
    let mut setup = Setup::recording("evidence-pr", &open_pr_recorded(), script);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start_recording();
    setup.until("asks", |rows| rows.len() == 1);
    let at = setup.head();
    setup.until_evidence(1, 2);

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);

    assert!(answered.status.success(), "{answered:?}");
    setup.until("actions", |rows| rows.len() == 1 && rows[0][4] == "opened");
    let short = &at[..7];
    assert_eq!(
        body(&setup),
        format!(
            "Add the widget\nIt draws a box.\n\ncargo test passed\n\n## Evidence\n\n\
             - The empty cart (image at {short}): https://runner.example:7390/tasks/1\n\
             - The login page (image at {short}): https://runner.example:7390/tasks/1"
        )
    );
    let items = setup.evidence(1);
    assert!(
        items.iter().all(|item| item["approved"] == true),
        "{items:?}"
    );
}

#[test]
fn an_item_made_after_the_approving_answer_is_not_in_the_list_the_approval_covers() {
    let script = "#!/bin/sh\nwhile [ ! -e \"$(dirname \"$0\")/go\" ]; do sleep 0.05; done\n\
        printf '\\211PNG\\r\\n\\032\\nlate' > \"$PRIORY_EVIDENCE/late.png\"\n\
        echo 'After the answer' > \"$PRIORY_EVIDENCE/late.png.caption\"\n";
    let mut setup = Setup::recording("evidence-late", &open_pr_recorded(), script);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start_recording();
    setup.until("asks", |rows| rows.len() == 1);
    let at = setup.head();
    let approved = setup.commit(1, "widget.txt");

    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.until("actions", |rows| rows.len() == 1 && rows[0][4] == "opened");
    fs::write(setup.dir.join("factory/go"), "").unwrap();

    let items = setup.until_evidence(1, 1);
    assert_eq!(
        [&items[0]["commit_sha"], &items[0]["approved"]],
        [&json!(at), &json!(false)]
    );
    assert_ne!(at, approved);
    assert_eq!(
        body(&setup),
        "Add the widget\nIt draws a box.\n\ncargo test passed"
    );
    let address = setup.address();
    let (_, _, task) = get(&address, "/api/tasks/1", true);
    let task: serde_json::Value = serde_json::from_slice(&task).unwrap();
    assert_eq!(
        task["results"][0]["attachments"][0]["approved"],
        json!(false)
    );
}

#[test]
fn the_sweep_removes_the_items_of_a_task_ended_31_days_ago_and_keeps_newer_and_asleep_ones() {
    let mut setup = Setup::recording("evidence-sweep", RECORDED, &png_recorder("The login page"));
    setup.start_recording();
    for (task, issue) in [(1, 42), (2, 43), (3, 44)] {
        setup.calls(&[result("passed")]);
        setup.publish(&requested(issue));
        setup.until_evidence(task, 1);
        setup.until("tasks", |rows| {
            rows.len() == task as usize && rows.iter().all(|row| row[4] == "done")
        });
    }
    setup.crash();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    for (task, state, days) in [(1, "done", 31), (2, "done", 29), (3, "asleep", 31)] {
        db.execute(
            "UPDATE tasks SET state = ?2 WHERE id = ?1",
            rusqlite::params![task, state],
        )
        .unwrap();
        db.execute(
            "UPDATE tasks SET changed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?2) WHERE id = ?1",
            rusqlite::params![task, format!("-{days} days")],
        )
        .unwrap();
    }
    drop(db);

    setup.start_recording();

    setup.until_evidence(1, 0);
    assert!(!setup.dir.join("home/evidence/1").exists());
    for task in [2, 3] {
        assert_eq!(setup.evidence(task).len(), 1, "task {task}");
        let kept = fs::read_dir(setup.dir.join(format!("home/evidence/{task}")))
            .unwrap()
            .count();
        assert_eq!(kept, 1, "task {task}");
    }
}

#[test]
fn an_approval_a_written_answer_voids_keeps_its_items_listed() {
    let workflow = open_pr_recorded().replace("opened = \"done\"", "opened = \"confirm\"")
        + r#"
[[steps]]
id = "confirm"
decides = "human"
effect = "none"
options = ["fine", "undo"]
next = { fine = "done", undo = "stop" }
"#;
    let script = "#!/bin/sh\ncd \"$PRIORY_EVIDENCE\"\n\
        printf '\\211PNG\\r\\n\\032\\none' > login.png; echo 'The login page' > login.png.caption\n";
    let mut setup = Setup::recording("evidence-voided", &workflow, script);
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start_recording();
    setup.until("asks", |rows| rows.len() == 1);
    setup.until_evidence(1, 1);
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    setup.until("asks", |rows| rows.iter().any(|row| row[0] == "2"));

    let written = setup.priory(&["answer", "2", "fix the typo first", "--revision", "1"]);

    assert!(written.status.success(), "{written:?}");
    assert_eq!(setup.evidence(1)[0]["approved"], true);
    let (_, _, desk) = get(&setup.address(), "/api/tasks/1", true);
    let task: serde_json::Value = serde_json::from_slice(&desk).unwrap();
    assert_eq!(task["results"][0]["attachments"][0]["approved"], true);
}
