//! Journals: a trigger's runs keep state on the log, which a run of that trigger on any runner reads.

use super::*;

/// A tool call of `name` with `arguments`.
fn journal(name: &str, arguments: serde_json::Value) -> serde_json::Value {
    json!({ "name": name, "arguments": arguments })
}

/// A request of issue `issue` that trigger `trigger` made.
pub(crate) fn requested_by(issue: u64, trigger: &str) -> Message {
    let mut request = requested(issue);
    request.data["trigger"] = json!(trigger);
    request
}

impl Setup {
    /// The reply to `params`, called by the live run `run`, and whether it failed.
    pub(crate) fn tool(&self, run: usize, params: serde_json::Value) -> (String, bool) {
        let session = self.rows("runs")[run][4].clone();
        reply(&self.mcp(&session, &[mcp_call(params)])[0])
    }

    /// The JSON reply to `params`, called by the live run `run`, which must succeed.
    pub(crate) fn read_journal(&self, run: usize, params: serde_json::Value) -> serde_json::Value {
        let (text, failed) = self.tool(run, params);
        assert!(!failed, "{text}");
        serde_json::from_str(&text).unwrap()
    }

    /// Waits until run `run` is asleep on its ask.
    fn asleep(&self, run: usize) {
        self.until("runs", |rows| {
            rows.get(run).is_some_and(|row| row[7] == "asleep")
        });
    }

    /// `priory journal ARGS` from the pane of run `run`, with `stdin`.
    fn journal_cli(&self, run: usize, args: &[&str], stdin: &str) -> Exit {
        let session = self.rows("runs")[run][4].clone();
        let mut child = self
            .command(env!("CARGO_BIN_EXE_priory"))
            .env("PRIORY_SESSION", session)
            .arg("journal")
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }
}

/// The bodies of the entries a `journal_read` reply holds.
pub(crate) fn bodies(read: &serde_json::Value) -> Vec<serde_json::Value> {
    read["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["body"].clone())
        .collect()
}

/// A second trigger, `nightly`, beside the setup's `agent-label`, whose runs may read its journal.
fn nightly(setup: &Setup) {
    let trigger = format!("{TRIGGER}[journal]\nreads = [\"agent-label\"]\n");
    fs::write(setup.dir.join("factory/triggers/nightly.toml"), trigger).unwrap();
}

#[test]
fn a_triggers_journal_written_on_one_runner_reaches_its_next_run_on_another_and_no_other_trigger() {
    let mut a = runner("journal", "a", None);
    let mut b = runner("journal", "b", Some(&a));
    nightly(&b);
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    a.calls(&[ask()]);
    a.publish(&requested(1));
    a.start_as(exe);
    a.asleep(0);
    let (text, failed) = a.tool(
        0,
        journal(
            "journal_append",
            json!({ "kind": "run", "body": { "items": 14 } }),
        ),
    );
    assert!(
        !failed && text.ends_with("to the journal of agent-label"),
        "{text}"
    );
    let put = journal(
        "journal_put",
        json!({ "name": "history", "body": { "runs": [1] } }),
    );
    let (text, failed) = a.tool(0, put);
    assert!(!failed, "{text}");
    let task = json!({ "trigger": "agent-label", "task": "github:owner/repo#1", "runner": "a" });
    logged(&a, "priory.journal.appended", task.clone());
    logged(&a, "priory.journal.put", task);
    a.crash();

    b.calls(&[ask(), ask()]);
    b.publish(&requested(2));
    b.start_as(exe);
    b.asleep(0);
    let read = b.read_journal(0, journal("journal_read", json!({})));
    assert_eq!(bodies(&read), [json!({ "items": 14 })]);
    assert_eq!(read["entries"][0]["kind"], "run");
    assert_eq!(read["entries"][0]["runner"], "a");
    let history = b.read_journal(0, journal("journal_get", json!({ "name": "history" })));
    assert_eq!(history["body"], json!({ "runs": [1] }));
    let listed = b.read_journal(0, journal("journal_get", json!({})));
    assert_eq!(listed["documents"][0]["name"], "history");

    b.publish(&requested_by(3, "nightly"));
    b.asleep(1);
    let other = b.read_journal(1, journal("journal_read", json!({})));
    assert_eq!(other["trigger"], "nightly");
    assert_eq!(bodies(&other), Vec::<serde_json::Value>::new());
    let (text, failed) = b.tool(1, journal("journal_get", json!({ "name": "history" })));
    assert!(
        failed && text.contains("holds no document history"),
        "{text}"
    );
    // nightly lists agent-label in its reads, and agent-label lists nothing.
    let listed = b.journal_cli(1, &["read", "--from", "agent-label", "--json"], "");
    assert!(listed.status.success(), "{listed:?}");
    let listed: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(bodies(&listed), [json!({ "items": 14 })]);
    let unlisted = b.journal_cli(0, &["get", "--from", "nightly"], "");
    assert!(
        !unlisted.status.success()
            && String::from_utf8_lossy(&unlisted.stderr)
                .contains("trigger agent-label does not list nightly in [journal] reads"),
        "{unlisted:?}"
    );
    let note = journal(
        "journal_append",
        json!({ "kind": "note", "body": "nightly" }),
    );
    let (text, failed) = b.tool(1, note);
    assert!(!failed, "{text}");
    let read = b.read_journal(0, journal("journal_read", json!({})));
    assert_eq!(
        bodies(&read),
        [json!({ "items": 14 })],
        "nightly wrote its own"
    );

    // Task 2 came from agent-label, so a message that files it under nightly is forged.
    let entry = |task: &str, body: &str| {
        Message::new(
            "priory.journal.appended",
            json!({ "trigger": "nightly", "kind": "note", "body": body, "task": task, "runner": "a" }),
        )
    };
    b.publish(&entry("github:owner/repo#2", "forged"));
    b.publish(&entry("github:owner/repo#3", "marker"));
    let deadline = Instant::now() + Duration::from_secs(60);
    let read = loop {
        let read = b.read_journal(1, journal("journal_read", json!({})));
        if bodies(&read).contains(&json!("marker")) || Instant::now() > deadline {
            break read;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(bodies(&read), [json!("nightly"), json!("marker")]);
}

#[test]
fn a_journal_write_over_a_limit_fails_a_task_without_a_trigger_has_none_and_the_cli_reaches_it() {
    let mut setup = Setup::new("journal-limits");
    setup.calls(&[ask(), ask()]);
    let factory = setup.dir.join("factory");
    let request = [
        "request",
        factory.to_str().unwrap(),
        "--workflow",
        "implement",
        "--key",
        "by-hand",
        "--repo",
        "owner/repo",
        "Do it.",
    ];
    let by_hand = setup.priory(&request);
    assert!(by_hand.status.success(), "{by_hand:?}");
    setup.start();
    setup.asleep(0);
    for (tool, arguments) in [
        ("journal_append", json!({ "kind": "run", "body": 1 })),
        ("journal_read", json!({})),
        ("journal_put", json!({ "name": "history", "body": 1 })),
        ("journal_get", json!({})),
    ] {
        let (text, failed) = setup.tool(0, journal(tool, arguments));
        assert!(
            failed && text.ends_with("has no trigger (it came from cli), so it has no journal: only the runs of a factory trigger keep one"),
            "{tool}: {text}"
        );
    }

    setup.publish(&requested(1));
    setup.asleep(1);
    let appended = setup.journal_cli(1, &["append", "--kind", "run", r#"{"items":3}"#], "");
    assert!(appended.status.success(), "{appended:?}");
    let put = setup.journal_cli(1, &["put", "notes"], "# Notes\n");
    assert!(put.status.success(), "{put:?}");
    let read = setup.journal_cli(1, &["read", "--json"], "");
    let read: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(bodies(&read), [json!({ "items": 3 })]);
    let got = setup.journal_cli(1, &["get", "notes"], "");
    assert_eq!(String::from_utf8_lossy(&got.stdout), "# Notes\n");
    let person = setup.priory(&["journal", "read", "--trigger", "agent-label"]);
    let lines = String::from_utf8_lossy(&person.stdout).into_owned();
    assert!(lines.ends_with("\trun\t{\"items\":3}\n"), "{lines}");
    let run = setup.journal_cli(1, &["read", "--trigger", "nightly"], "");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        !run.status.success() && stderr.contains("only a person runs it"),
        "{stderr}"
    );

    // A JSON string's two quotes count: these bodies are one byte over each limit.
    let over = |kb: usize| "x".repeat(kb * 1024 - 1);
    let (text, failed) = setup.tool(
        1,
        journal("journal_append", json!({ "kind": "run", "body": over(16) })),
    );
    assert!(
        failed && text.contains("an entry holds at most 16 KB"),
        "{text}"
    );
    let (text, failed) = setup.tool(
        1,
        journal("journal_put", json!({ "name": "big", "body": over(256) })),
    );
    assert!(
        failed && text.contains("a document holds at most 256 KB"),
        "{text}"
    );
    // 31 documents of 256 KB and what is there already leave less than 256 KB of the 8 MB.
    let session = setup.rows("runs")[1][4].clone();
    let full = "x".repeat(256 * 1024 - 2);
    let puts: Vec<serde_json::Value> = (0..31)
        .map(|n| {
            mcp_call(journal(
                "journal_put",
                json!({ "name": format!("doc-{n}"), "body": full }),
            ))
        })
        .collect();
    for put in setup.mcp(&session, &puts) {
        let (text, failed) = reply(&put);
        assert!(!failed, "{text}");
    }
    let (text, failed) = setup.tool(
        1,
        journal("journal_put", json!({ "name": "doc-31", "body": full })),
    );
    assert!(
        failed && text.contains("the journal of agent-label holds at most 8 MB"),
        "{text}"
    );
    let replaced = journal("journal_put", json!({ "name": "doc-0", "body": full }));
    let (text, failed) = setup.tool(1, replaced);
    assert!(
        !failed,
        "a put that replaces a document counts it once: {text}"
    );
    let listed = setup.read_journal(1, journal("journal_get", json!({})));
    assert_eq!(listed["documents"].as_array().unwrap().len(), 32);
    let read = setup.read_journal(1, journal("journal_read", json!({})));
    assert_eq!(
        bodies(&read),
        [json!({ "items": 3 })],
        "a refused write wrote nothing"
    );
}

#[test]
fn an_import_appends_a_files_lines_in_order_and_a_second_one_needs_again() {
    let mut setup = Setup::new("journal-import");
    setup.start();
    let factory = setup.dir.join("factory");
    let factory = factory.to_str().unwrap();
    let runs = setup.dir.join("runs.jsonl");
    fs::write(&runs, "{\"run\":1}\n\n{\"run\":2}\n{\"run\":3}\n").unwrap();
    let history = setup.dir.join("history.json");
    fs::write(&history, r#"{"seen":[1,2,3]}"#).unwrap();
    let runs = runs.to_str().unwrap();
    let import = |extra: &[&str]| {
        let mut args = vec!["journal", "import", factory, "agent-label"];
        args.extend(extra);
        setup.priory(&args)
    };
    let entries = |want: usize| {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let read = setup.priory(&["journal", "read", "--trigger", "agent-label", "--json"]);
            let read: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap_or_default();
            if read["entries"].as_array().is_some_and(|e| e.len() >= want)
                || Instant::now() > deadline
            {
                break read;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };

    let first = import(&["--entries", runs, "--kind", "run"]);
    assert!(first.status.success(), "{first:?}");
    let read = entries(3);
    assert_eq!(
        bodies(&read),
        [
            json!({ "run": 1 }),
            json!({ "run": 2 }),
            json!({ "run": 3 })
        ]
    );
    assert_eq!(read["entries"][0]["kind"], "run");

    let again = import(&["--entries", runs, "--kind", "run"]);
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert!(
        again.status.code() == Some(1) && stderr.contains("pass --again"),
        "{again:?}"
    );
    // A run's shell, even with PRIORY_SESSION emptied, cannot import.
    let in_run = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .env("PRIORY_SESSION", "")
        .args([
            "journal",
            "import",
            factory,
            "agent-label",
            "--entries",
            runs,
            "--kind",
            "run",
            "--again",
        ])
        .output()
        .unwrap();
    assert!(
        in_run.status.code() == Some(2)
            && String::from_utf8_lossy(&in_run.stderr).contains("only a person runs it"),
        "{in_run:?}"
    );
    // --skip 1 publishes the file's last two lines, after the refused imports.
    let appended = import(&["--entries", runs, "--kind", "run", "--again", "--skip", "1"]);
    assert!(appended.status.success(), "{appended:?}");
    assert_eq!(
        bodies(&entries(5)),
        [1, 2, 3, 2, 3].map(|run| json!({ "run": run })),
        "the refused imports wrote nothing"
    );

    let document = ["--document", "history", history.to_str().unwrap()];
    let put = import(&document);
    assert!(put.status.success(), "{put:?}");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !setup
        .priory(&["journal", "get", "history", "--trigger", "agent-label"])
        .status
        .success()
    {
        assert!(Instant::now() < deadline, "the document never applied");
        std::thread::sleep(Duration::from_millis(50));
    }
    let twice = import(&document);
    assert!(
        twice.status.code() == Some(1)
            && String::from_utf8_lossy(&twice.stderr).contains("already holds document history"),
        "{twice:?}"
    );
}
