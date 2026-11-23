//! Consults: a worker asks an investigator one question and sleeps until the report
//! (docs/daemon/tools.md#consultagent-question).

use super::*;

/// A factory whose implementer may consult sentry, and nobody else; honeycomb is an investigator
/// nobody may consult. `consult` holds the lines `[consult]` gets beside `allow`.
fn factory_toml(consult: &str) -> String {
    format!(
        "name = \"demo\"\n\n[consult]\n{consult}allow = {{ implementer = [\"sentry\"] }}\n\n[mcp.sentry]\nread_only = \"The token has the org:read and event:read scopes only.\"\n\n[mcp.honeycomb]\nread_only = \"The key is a read-only API key.\"\n"
    )
}

/// An investigator of `server`, which needs 20 GB of memory to start.
fn investigator(description: &str, server: &str) -> String {
    format!(
        "+++\ndescription = \"{description}\"\nharness = \"claude-code\"\nmodel = \"claude-fake-5\"\ninvestigator = true\nmemory = \"20GB\"\nmcpServers = [\"{server}\"]\npermissions = {{ defaultMode = \"dontAsk\", allow = [\"mcp__{server}__get_issue_details\", \"Read\"] }}\n+++\nYou read {server} and answer one question.\n"
    )
}

/// The price `prices.toml` gives the fake `claude`'s model, in dollars per million input tokens.
fn prices(input: f64) -> String {
    format!(
        "[\"claude-fake-5\"]\ninput = {input}\noutput = 0.0\ncache_read = 0.0\ncache_write = 0.0\n"
    )
}

/// A setup whose implementer lists the MCP server linear and may consult sentry, with 12 GB free,
/// so an investigator waits for room until the test frees it. The person's own Claude config
/// defines sentry, honeycomb, linear and personal, which no agent lists.
fn consulting(name: &str, consult: &str) -> Setup {
    let setup = Setup::new(name);
    let files = [
        ("factory/factory.toml", factory_toml(consult)),
        (
            "factory/agents/implementer/agent.md",
            AGENT.replacen("+++\n", "+++\nmcpServers = [\"linear\"]\n", 1),
        ),
        (
            "factory/agents/sentry/agent.md",
            investigator(
                "Reads Sentry issues, events and releases, and answers questions about them.",
                "sentry",
            ),
        ),
        (
            "factory/agents/honeycomb/agent.md",
            investigator(
                "Reads traces and answers questions about latency.",
                "honeycomb",
            ),
        ),
        ("factory/prices.toml", prices(1.0)),
    ];
    for (path, text) in files {
        let file = setup.dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    let servers = json!({
        "sentry": {"command": "npx", "args": ["@sentry/mcp-server"]},
        "honeycomb": {"command": "npx", "args": ["@honeycomb/mcp"]},
        "linear": {"type": "http", "url": "https://mcp.linear.app/mcp"},
        "personal": {"command": "personal-mcp"},
    });
    fs::create_dir_all(setup.dir.join("claude")).unwrap();
    fs::write(
        setup.dir.join("claude/.claude.json"),
        json!({ "mcpServers": servers }).to_string(),
    )
    .unwrap();
    setup.memory(12);
    setup
}

const QUESTION: &str = "Issue WEB-41 started on 2026-11-20. Which release introduced it?";

fn consult(agent: &str) -> serde_json::Value {
    json!({ "name": "consult", "arguments": { "agent": agent, "question": QUESTION } })
}

fn reported(summary: &str) -> serde_json::Value {
    json!({ "name": "result", "arguments": { "outcome": "reported", "summary": summary } })
}

/// The `priory task ID --json` of task `id`.
fn task_json(setup: &Setup, id: &str) -> serde_json::Value {
    let output = setup.priory(&["task", id, "--json"]);
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The MCP config the run of harness session `session` was launched with.
fn mcp_config(setup: &Setup, session: &str) -> serde_json::Value {
    let file = setup.dir.join(format!("home/prompts/{session}.mcp.json"));
    serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap()
}

fn server_names(config: &serde_json::Value) -> Vec<&str> {
    config["mcpServers"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

/// Holds the investigator's first turn before its tool call: it waits for room until the hold is
/// set, so it cannot take its call first.
fn hold_investigator(setup: &Setup) {
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[7].starts_with("waits for room")
    });
    setup.hold("hold-call");
    setup.memory(60);
}

/// Waits until a restarted daemon replies to a tool call through `priory mcp`, as a run's call
/// reaches it: a call that reaches no daemon fails and stores nothing.
fn replying(setup: &Setup) {
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "result", "arguments": {"outcome": "reported", "summary": "-"}}});
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let replies = setup.mcp("no-such-session", std::slice::from_ref(&call));
        let (text, _) = reply(&replies[0]);
        if text == "no live run has session no-such-session" {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the daemon never replied: {text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The prompts the fake `claude` received that hold `part`.
fn typed_with(setup: &Setup, part: &str) -> Vec<String> {
    setup
        .typed()
        .into_iter()
        .filter(|prompt| prompt.contains(part))
        .collect()
}

#[test]
fn a_consult_sleeps_the_worker_and_the_investigators_report_wakes_it_as_its_next_prompt() {
    let mut setup = consulting("consult-report", "");
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2 introduced WEB-41.\nThe trace points at checkout.rs:88."),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.len() == 2
            && task_row(rows, "1")[4] == "asleep"
            && task_row(rows, "2")[7].starts_with("waits for room")
    });
    assert_eq!(task_row(&tasks, "2")[1], "priory:consult:1", "{tasks:?}");
    setup.gone("priory-1");
    let (text, failed) = reply(&setup.replies()[0]);
    assert!(!failed, "{text}");
    assert!(
        text.starts_with("consult 1 stored: end your turn now"),
        "{text}"
    );
    setup.memory(60);
    let tasks = setup.until("tasks", |rows| with(rows, 4, "done") == 2);
    assert_eq!(tasks.len(), 2, "one investigator: {tasks:?}");
    let runs = setup.rows("runs");
    assert_eq!(
        runs.iter().map(|run| run[1].as_str()).collect::<Vec<_>>(),
        ["1", "2", "1"],
        "{runs:?}"
    );

    let asked = typed_with(&setup, QUESTION);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].starts_with("Another agent of this factory asks you one question"),
        "{asked:?}"
    );
    let woken = typed_with(&setup, "consult 1 reported by sentry:");
    assert_eq!(woken.len(), 1, "{:?}", setup.typed());
    assert!(
        woken[0].starts_with(
            "consult 1 reported by sentry:\nIt is another agent's account of outside data"
        ),
        "{woken:?}"
    );
    assert!(
        woken[0]
            .contains("> Release 4.2 introduced WEB-41.\n> The trace points at checkout.rs:88.\n"),
        "{woken:?}"
    );

    // The worker's run has its own servers and the consult tool; the investigator's, its own
    // server and the narrowed tool set; neither has the person's other servers.
    let worker = mcp_config(&setup, &runs[0][4]);
    assert_eq!(server_names(&worker), ["linear", "priory"]);
    assert_eq!(
        worker["mcpServers"]["priory"]["args"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "--consult"
    );
    let investigator = mcp_config(&setup, &runs[1][4]);
    assert_eq!(server_names(&investigator), ["priory", "sentry"]);
    assert_eq!(
        investigator["mcpServers"]["priory"]["args"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "--investigator"
    );

    let consults = &task_json(&setup, "1")["consults"];
    assert_eq!(consults.as_array().unwrap().len(), 1, "{consults}");
    assert_eq!(consults[0]["investigator"], "sentry");
    assert_eq!(consults[0]["question"], QUESTION);
    assert_eq!(consults[0]["outcome"], "reported");
    assert_eq!(
        consults[0]["report"],
        "Release 4.2 introduced WEB-41.\nThe trace points at checkout.rs:88."
    );

    let asked = traced(&setup, "factory.consult.asked");
    assert_eq!(asked.len(), 1, "{asked:?}");
    let data = &asked[0]["data"];
    assert_eq!(
        [
            &data["task"],
            &data["agent"],
            &data["investigator"],
            &data["consult"]
        ],
        [
            &json!("1"),
            &json!("implementer"),
            &json!("sentry"),
            &json!(1)
        ]
    );
    assert_eq!(data["question_length"], QUESTION.chars().count());
    let ended: Vec<_> = traced(&setup, "factory.step.ended")
        .into_iter()
        .filter(|record| record["data"]["consult"].is_object())
        .collect();
    assert_eq!(ended.len(), 1, "{ended:?}");
    assert_eq!(ended[0]["data"]["consult"], json!({ "id": 1, "task": "1" }));
    assert_eq!(ended[0]["data"]["outcome"], "reported");
    let reported = traced(&setup, "factory.consult.reported");
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0]["data"]["outcome"], "reported");
    assert_eq!(reported[0]["data"]["task"], "1");
    assert!(reported[0]["data"]["seconds"].as_f64().unwrap() > 0.0);

    let factory = setup.dir.join("factory");
    let costs = setup.priory(&[
        "costs",
        factory.to_str().unwrap(),
        "--by",
        "agent",
        "--json",
    ]);
    let costs: serde_json::Value = serde_json::from_slice(&costs.stdout).unwrap();
    assert!(costs.to_string().contains("\"sentry\""), "{costs}");
}

#[test]
fn an_investigators_claude_loads_no_user_or_project_settings_and_has_no_tool_that_writes() {
    let mut setup = consulting("consult-read-only", "");
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));
    setup.start();

    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[7].starts_with("waits for room")
    });
    let worker = setup.claude_args(0);
    setup.hold("hold-call");
    setup.memory(60);
    let investigator = setup.claude_args(1);

    // Claude merges `allow` across the person's, the repository's and the flag's settings, so
    // only the flag's may reach an investigator.
    assert_eq!(
        arg(&investigator, "--setting-sources").as_deref(),
        Some(""),
        "{investigator:?}"
    );
    assert_eq!(
        arg(&investigator, "--disallowedTools").as_deref(),
        Some("Edit,MultiEdit,Write,NotebookEdit,Bash,Agent,Task"),
        "{investigator:?}"
    );
    for flag in ["--setting-sources", "--disallowedTools"] {
        assert!(!worker.contains(&flag.to_owned()), "{worker:?}");
    }
    setup.release("hold-call");
    setup.until("tasks", |rows| with(rows, 4, "done") == 2);
}

#[test]
fn the_index_lists_only_the_investigators_an_agent_may_consult() {
    let mut setup = consulting("consult-index", "");
    setup.calls(&[consult("honeycomb"), consult("nobody")]);
    setup.publish(&requested(42));

    setup.start();

    let replies = setup.replied(2);
    let runs = setup.rows("runs");
    let system = fs::read_to_string(
        setup
            .dir
            .join(format!("home/prompts/{}.system.md", runs[0][4])),
    )
    .unwrap();
    assert!(
        system.contains(
            "Investigators you can consult with the consult tool, one question each:\n- sentry: Reads Sentry issues, events and releases, and answers questions about them."
        ),
        "{system}"
    );
    assert!(!system.contains("honeycomb"), "{system}");
    for reply in &replies {
        let (text, failed) = super::reply(reply);
        assert!(failed, "{text}");
        assert!(
            text.ends_with("; it may consult: sentry"),
            "refused with the names it may use: {text}"
        );
    }
    assert_eq!(setup.rows("tasks").len(), 1);
}

#[test]
fn an_agent_with_no_consult_line_gets_no_consult_tool() {
    let mut setup = consulting("consult-none", "");
    fs::write(
        setup.dir.join("factory/factory.toml"),
        factory_toml("").replace("implementer = [", "reviewer = ["),
    )
    .unwrap();
    setup.calls(&[consult("sentry")]);
    setup.publish(&requested(42));

    setup.start();

    let (text, failed) = reply(&setup.replied(1)[0]);
    assert!(failed, "{text}");
    assert_eq!(text, "unknown tool consult");
    let runs = setup.rows("runs");
    let args = mcp_config(&setup, &runs[0][4])["mcpServers"]["priory"]["args"].clone();
    assert_eq!(
        args.as_array().unwrap().len(),
        3,
        "no tool set flag: {args}"
    );
    let listed = setup.mcp(
        &runs[0][4],
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})],
    );
    let names: Vec<_> = listed[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"consult"), "{names:?}");
}

/// Runs `priory mcp --session s <flag>` with `lines` and returns its replies.
fn mcp_with(setup: &Setup, flag: &str, lines: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let mut mcp = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["mcp", "--session", "s", flag])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut mcp.stdin.take().unwrap(), input.as_bytes()).unwrap();
    String::from_utf8(mcp.wait_with_output().unwrap().stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn an_investigators_server_lists_result_and_the_read_tools_and_knows_no_other() {
    let setup = Setup::new("consult-tools");
    let mut lines = vec![json!({"jsonrpc": "2.0", "id": 0, "method": "tools/list"})];
    let refused = ["consult", "ask", "wait", "finding", "remember", "add"];
    for (id, tool) in refused.iter().enumerate() {
        lines.push(
            json!({"jsonrpc": "2.0", "id": id + 1, "method": "tools/call",
            "params": {"name": tool, "arguments": {}}}),
        );
    }

    let replies = mcp_with(&setup, "--investigator", &lines);

    let names: Vec<_> = replies[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["result", "memory", "search"]);
    for (tool, reply) in refused.iter().zip(&replies[1..]) {
        assert_eq!(
            super::reply(reply),
            (format!("unknown tool {tool}"), true),
            "{reply}"
        );
    }
    let asker = mcp_with(&setup, "--consult", &lines[..1]);
    let names: Vec<_> = asker[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.last(), Some(&"consult"), "{names:?}");
}

#[test]
fn a_guarded_call_and_an_unserved_tool_in_an_investigators_run_are_denied() {
    let mut setup = consulting("consult-guard", "");
    setup.calls(&[
        consult("sentry"),
        json!([
            bash("gh issue comment 41 --body hi"),
            { "name": "mcp__sentry__update_issue", "arguments": { "issueId": "WEB-41" } },
            reported("Release 4.2."),
        ]),
    ]);
    setup.publish(&requested(42));

    setup.start();
    setup.until("tasks", |rows| rows.len() == 2);
    setup.memory(60);

    let replies = setup.replied(4);
    assert!(
        guard_denied(&replies[1]).is_some_and(|reason| reason.contains("gh-issue-comment")),
        "{}",
        replies[1]
    );
    assert!(
        guard_denied(&replies[2]).is_some_and(|reason| reason.contains("mcp-write")),
        "{}",
        replies[2]
    );
    let denied = traced(&setup, "factory.tool.denied");
    assert_eq!(denied.len(), 2, "{denied:?}");
    assert!(denied.iter().all(|record| record["data"]["task"] == "2"));
}

#[test]
fn a_consult_still_queued_at_its_deadline_never_starts_and_wakes_the_worker() {
    let mut setup = consulting("consult-queued-deadline", "deadline = \"2s\"\n");
    setup.calls(&[consult("sentry")]);
    setup.publish(&requested(42));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[4] == "dropped"
    });
    assert_eq!(
        task_row(&tasks, "2")[7],
        "consult 1 passed its deadline",
        "{tasks:?}"
    );
    let woken = setup.until("outbox", |rows| rows.len() >= 2);
    let typed = loop {
        let typed = typed_with(&setup, "consult 1 got no report");
        if !typed.is_empty() {
            break typed;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        typed[0].starts_with("consult 1 got no report: no report 2s after the question\n"),
        "{typed:?} {woken:?}"
    );
    assert!(setup.rows("runs").iter().all(|run| run[1] == "1"));
    let reported = traced(&setup, "factory.consult.reported");
    assert_eq!(reported[0]["data"]["outcome"], "deadline");
}

#[test]
fn an_investigator_running_past_its_deadline_is_ended_and_its_late_result_refused() {
    let mut setup = consulting("consult-running-deadline", "deadline = \"3s\"\n");
    setup.calls(&[consult("sentry"), reported("Too late.")]);
    setup.publish(&requested(42));

    setup.start();
    hold_investigator(&setup);
    let runs = setup.until("runs", |rows| rows.len() == 2);
    let session = runs[1][4].clone();

    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[4] == "dropped"
    });
    setup.gone("priory-2");
    let late = setup.mcp(
        &session,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "result", "arguments": {"outcome": "reported", "summary": "Too late."}}})],
    );
    let (text, failed) = reply(&late[0]);
    assert!(failed, "{text}");
    let consults = &task_json(&setup, "1")["consults"];
    assert_eq!(consults[0]["outcome"], "deadline", "{consults}");
    assert!(consults[0].get("report").is_none(), "{consults}");
}

#[test]
fn an_investigator_whose_transcript_passes_the_cap_is_ended_and_no_more_consults_are_made() {
    let mut setup = consulting("consult-cap", "cap = 0.5\n");
    // About 100 input tokens at $10,000 per million cost about $1, past the $0.50 cap.
    fs::write(setup.dir.join("factory/prices.toml"), prices(10_000.0)).unwrap();
    // The investigator is ended before it takes a call, so the worker's next turn takes the second.
    setup.calls(&[consult("sentry"), consult("sentry")]);
    setup.publish(&requested(42));

    setup.start();
    hold_investigator(&setup);
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[4] == "dropped"
    });
    setup.gone("priory-2");
    fs::remove_file(setup.dir.join("fake/hold-call")).unwrap();

    let woken = loop {
        let woken = typed_with(&setup, "consult 1 got no report");
        if !woken.is_empty() {
            break woken;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(
        woken[0].starts_with("consult 1 got no report: sentry passed its cap of $0.50\n"),
        "{woken:?}"
    );
    let reported = traced(&setup, "factory.consult.reported");
    assert_eq!(reported[0]["data"]["outcome"], "cap");
    assert!(reported[0]["data"]["tokens"]["input"].as_u64().unwrap() > 0);
    assert_eq!(reported[0]["data"]["model"], "claude-fake-5");
    let (text, failed) = reply(&setup.replied(2)[1]);
    assert!(failed, "{text}");
    assert!(
        text.starts_with("consult refused: consult 1 of task 1 cost $1.0")
            && text.ends_with(", past its cap of $0.50, so this task makes no more consults"),
        "{text}"
    );
}

#[test]
fn a_consult_past_per_task_is_refused_with_the_count() {
    let mut setup = consulting("consult-per-task", "per_task = 1\n");
    setup.memory(60);
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        consult("sentry"),
    ]);
    setup.publish(&requested(42));

    setup.start();

    let replies = setup.replied(3);
    let (text, failed) = reply(&replies[2]);
    assert!(failed, "{text}");
    assert_eq!(
        text,
        "consult refused: task 1 already made 1 consults, the most [consult] per_task allows"
    );
    assert_eq!(setup.rows("tasks").len(), 2);
}

#[test]
fn a_result_or_a_second_consult_while_a_consult_is_open_is_refused() {
    let mut setup = consulting("consult-open", "");
    setup.calls(&[json!([
        consult("sentry"),
        consult("sentry"),
        result("passed")
    ])]);
    setup.publish(&requested(42));

    setup.start();

    let replies = setup.replied(3);
    for reply in &replies[1..] {
        let (text, failed) = super::reply(reply);
        assert!(failed, "{text}");
        assert!(text.contains("already waits on consult 1"), "{text}");
    }
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "1")[4] == "asleep"
    });
}

#[test]
fn a_report_holding_a_secret_is_refused_without_the_value_until_the_investigator_answers_again() {
    const SECRET: &str = "s3cret-serve-token-41";
    let mut setup = consulting("consult-secret", "");
    setup.memory(60);
    setup.calls(&[
        consult("sentry"),
        reported(&format!("The token is {SECRET}.")),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start_with(
        Path::new(env!("CARGO_BIN_EXE_priory")),
        &[("PRIORY_TOKEN", SECRET)],
    );

    let replies = setup.replied(3);
    let (text, failed) = reply(&replies[1]);
    assert!(failed, "{text}");
    assert_eq!(
        text,
        "the report holds the value of a secret Priory knows: answer again without it"
    );
    setup.until("tasks", |rows| with(rows, 4, "done") == 2);
    let typed = setup.typed().join("");
    assert!(!typed.contains(SECRET), "{typed}");
    assert_eq!(typed_with(&setup, "consult 1 reported by sentry:").len(), 1);
}

#[test]
fn a_report_holding_a_secret_split_by_escape_codes_is_refused_without_the_value() {
    const SECRET: &str = "s3cret-serve-token-41";
    let split = "s3cret-se\u{1b}[0mrve-token-41";
    let shaped = "ghp_abcdefghij\u{1b}]8;;https://x\u{7}klmnopqrstu\u{0}vwxyz";
    let mut setup = consulting("consult-escapes", "");
    setup.memory(60);
    setup.calls(&[
        consult("sentry"),
        json!([
            reported(&format!("The token is {split}.")),
            reported(&format!("The key is {shaped}.")),
        ]),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start_with(
        Path::new(env!("CARGO_BIN_EXE_priory")),
        &[("PRIORY_TOKEN", SECRET)],
    );

    let replies = setup.replied(4);
    for reply in &replies[1..3] {
        let (text, failed) = super::reply(reply);
        assert!(failed, "{text}");
        assert_eq!(
            text,
            "the report holds the value of a secret Priory knows: answer again without it"
        );
    }
    setup.until("tasks", |rows| with(rows, 4, "done") == 2);
    let typed = setup.typed().join("");
    for value in ["s3cret-se", "abcdefghij"] {
        assert!(!typed.contains(value), "{typed}");
    }
    assert_eq!(typed_with(&setup, "consult 1 reported by sentry:").len(), 1);
}

#[test]
fn cancelling_the_asking_task_drops_its_consults_queued_task() {
    let mut setup = consulting("consult-cancel", "");
    setup.calls(&[consult("sentry")]);
    setup.publish(&requested(42));

    setup.start();
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "1")[4] == "asleep"
    });
    assert!(setup.priory(&["cancel", "1"]).status.success());

    let tasks = setup.until("tasks", |rows| task_row(rows, "2")[4] == "dropped");
    assert_eq!(
        task_row(&tasks, "2")[7],
        "its consult was withdrawn: task 1 is dropped",
        "{tasks:?}"
    );
    setup.memory(60);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(setup.rows("runs").iter().all(|run| run[1] == "1"));
    assert_eq!(
        task_json(&setup, "1")["consults"][0]["outcome"],
        "withdrawn"
    );
}

#[test]
fn a_revived_asking_task_hears_nothing_of_its_earlier_generations_consult() {
    let mut setup = consulting("consult-revive", "");
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start();
    hold_investigator(&setup);
    let runs = setup.until("runs", |rows| rows.len() == 2);
    assert!(setup.priory(&["cancel", "1"]).status.success());
    setup.until("tasks", |rows| task_row(rows, "2")[4] == "dropped");
    setup.gone("priory-2");
    assert!(setup.priory(&["revive", "1"]).status.success());
    setup.release("hold-call");
    let late = setup.mcp(
        &runs[1][4],
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "result", "arguments": {"outcome": "reported", "summary": "Release 4.2."}}})],
    );
    assert!(reply(&late[0]).1, "{late:?}");

    setup.until("tasks", |rows| {
        task_row(rows, "1")[4] == "running" && task_row(rows, "1")[9] == "2"
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        typed_with(&setup, "consult 1").is_empty(),
        "{:?}",
        setup.typed()
    );
}

/// The row of task `id` among the rows of `priory tasks`.
fn task_row<'r>(rows: &'r [Vec<String>], id: &str) -> &'r [String] {
    rows.iter().find(|row| row[0] == id).unwrap()
}

/// Adds `[queue] review_limit = 1` to the factory, and a pull request of task 1 that waits for a
/// person once task 1 sleeps on its consult.
fn at_review_limit(setup: &Setup) {
    let file = setup.dir.join("factory/factory.toml");
    let toml = fs::read_to_string(&file).unwrap() + "\n[queue]\nreview_limit = 1\n";
    fs::write(file, toml).unwrap();
}

fn write_state(setup: &Setup, statement: &str) {
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.busy_timeout(Duration::from_secs(10)).unwrap();
    db.execute_batch(statement).unwrap();
}

#[test]
fn a_consult_starts_ahead_of_its_bands_backlog_and_past_a_full_review_limit() {
    let mut setup = consulting("consult-backlog", "");
    at_review_limit(&setup);
    setup.calls(&[consult("sentry"), reported("Release 4.2.")]);
    setup.publish(&requested(42));

    setup.start();
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[7].starts_with("waits for room")
    });
    // Task 1 sleeps on its consult with an open pull request, so its lane is at the limit.
    write_state(
        &setup,
        "INSERT INTO pull_requests (task, resource, branch, base)
         VALUES (1, 'github:owner/repo#7', 'priory/1', 'main')",
    );
    setup.publish(&requested(43));
    setup.publish(&requested(44));
    setup.until("tasks", |rows| rows.len() == 4);
    // A person ordered the band's backlog: the consult still starts first.
    assert!(setup.priory(&["move", "4", "--top"]).status.success());
    let queue = setup.rows("queue");
    assert_eq!(
        queue
            .iter()
            .take(3)
            .map(|row| row[2].as_str())
            .collect::<Vec<_>>(),
        ["2", "4", "3"],
        "{queue:?}"
    );

    setup.memory(60);

    setup.until("tasks", |rows| task_row(rows, "2")[4] == "done");
    let runs = setup.rows("runs");
    assert_eq!(runs[1][1], "2", "the consult ran second: {runs:?}");
    let woken = loop {
        let woken = typed_with(&setup, "consult 1 reported by sentry:");
        if !woken.is_empty() {
            break woken;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(woken[0].contains("> Release 4.2."), "{woken:?}");
}

#[test]
fn a_consult_starts_while_priory_is_paused() {
    let mut setup = consulting("consult-paused", "");
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start();
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[7].starts_with("waits for room")
    });
    assert!(setup.priory(&["pause"]).status.success());
    setup.publish(&requested(43));
    setup.until("tasks", |rows| {
        rows.len() == 3 && task_row(rows, "3")[7] == "waits: priory is paused"
    });

    setup.memory(60);

    let tasks = setup.until("tasks", |rows| task_row(rows, "1")[4] == "done");
    assert_eq!(task_row(&tasks, "2")[4], "done", "{tasks:?}");
    assert_eq!(task_row(&tasks, "3")[4], "queued", "{tasks:?}");
}

#[test]
fn a_revived_task_may_consult_again_up_to_per_task() {
    let mut setup = consulting("consult-revive-count", "per_task = 1\n");
    setup.memory(60);
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        serde_json::Value::Null,
    ]);
    setup.publish(&requested(42));

    setup.start();
    setup.calls_left(0);
    setup.until("tasks", |rows| {
        rows.len() == 2 && task_row(rows, "2")[4] == "done"
    });
    assert!(setup.priory(&["cancel", "1"]).status.success());
    setup.until("tasks", |rows| task_row(rows, "1")[4] == "dropped");
    setup.calls(&[consult("sentry")]);
    assert!(setup.priory(&["revive", "1"]).status.success());

    let replies = setup.replied(3);
    let (text, failed) = reply(&replies[2]);
    assert!(!failed, "{text}");
    assert!(text.starts_with("consult 2 stored"), "{text}");
}

#[test]
fn a_restart_between_the_ask_and_the_report_starts_one_investigator_and_reports_once() {
    let mut setup = consulting("consult-restart", "");
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));

    setup.start();
    hold_investigator(&setup);
    setup.until("outbox", |rows| {
        rows.len() == 2 && rows[1][3] == "confirmed"
    });
    setup.crash();
    setup.start();
    replying(&setup);
    setup.release("hold-call");

    setup.until("tasks", |rows| with(rows, 4, "done") == 2);
    assert_eq!(setup.rows("tasks").len(), 2);
    assert_eq!(
        setup
            .rows("runs")
            .iter()
            .filter(|run| run[1] == "2")
            .count(),
        1
    );
    assert_eq!(typed_with(&setup, "consult 1 reported by sentry:").len(), 1);
    assert_eq!(traced(&setup, "factory.consult.reported").len(), 1);
}

/// Runs the real `claude -p` in `project`, with a throwaway `home`, on the prompt to run
/// `touch marker`, with the flags an investigator's launch adds when `investigator` says so.
fn touch_with_real_claude(home: &Path, project: &Path, investigator: bool) -> bool {
    let servers = priory_runtime::servers::Servers::default();
    let launch = priory_runtime::runtime::Launch {
        tmux: "real",
        dir: project,
        home,
        session: "real",
        resume: None,
        model: None,
        effort: None,
        body: "",
        instructions: "",
        plugin: None,
        memories: None,
        prompt: None,
        permissions: json!({ "defaultMode": "dontAsk", "allow": ["Read"] }),
        permission_mode: "dontAsk",
        scope: &[],
        env: &[],
        debate: false,
        servers: &servers,
        tools: priory_runtime::mcp::Tools::Investigator,
        jail: None,
    };
    let args = priory_runtime::runtime::Harness::args(
        &priory_runtime::runtime::ClaudeCode,
        Path::new("/bin/true"),
        &launch,
    )
    .unwrap();
    let mut command = harness::command("claude");
    command
        .current_dir(project)
        .env("HOME", home)
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CLAUDECODE")
        .args(["-p", "--permission-mode", "dontAsk", "--settings"])
        .arg(json!({ "permissions": { "allow": ["Read"] } }).to_string());
    if investigator {
        for flag in ["--setting-sources", "--disallowedTools"] {
            command.args([flag.to_owned(), arg(&args, flag).unwrap()]);
        }
    }
    let output = command
        .args([
            "--",
            "Use the Bash tool to run exactly this command: touch marker. Then say DONE or DENIED.",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let touched = project.join("marker").exists();
    let _ = fs::remove_file(project.join("marker"));
    touched
}

#[test]
#[ignore = "runs the real claude, which needs a sign-in and spends tokens: run it with --ignored"]
fn the_real_claude_denies_an_investigator_a_bash_rule_of_the_persons_or_the_repositorys_settings() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("real-claude-investigator");
    let _ = fs::remove_dir_all(&dir);
    let (home, project) = (dir.join("home"), dir.join("project"));
    let allow = json!({ "permissions": { "allow": ["Bash(touch:*)"] } }).to_string();
    for settings in [home.join(".claude"), project.join(".claude")] {
        fs::create_dir_all(&settings).unwrap();
    }
    // Claude finds the repository's settings at its git root, which would be Priory's own.
    let init = harness::command("git")
        .arg("init")
        .arg("-q")
        .arg(&project)
        .status();
    assert!(init.unwrap().success());
    // The person accepted the trust dialog once, so Claude reads the repository's settings.
    let trusted = json!({ "projects": { project.display().to_string(): { "hasTrustDialogAccepted": true } } });
    fs::write(home.join(".claude.json"), trusted.to_string()).unwrap();

    for file in [
        home.join(".claude/settings.json"),
        project.join(".claude/settings.json"),
    ] {
        fs::write(&file, &allow).unwrap();
        assert!(
            touch_with_real_claude(&home, &project, false),
            "without the flags, {} lets the touch run",
            file.display()
        );
        assert!(
            !touch_with_real_claude(&home, &project, true),
            "with them, {} reaches the investigator",
            file.display()
        );
        fs::remove_file(&file).unwrap();
    }
}

/// Turns the sandbox `jail` on for the implementer's workflow. The fake `claude` reads `FAKE_DIR`,
/// so the sandbox keeps it.
fn jail_the_worker(setup: &Setup) {
    let factory = setup.dir.join("factory/factory.toml");
    let toml = fs::read_to_string(&factory).unwrap();
    fs::write(
        &factory,
        format!("{toml}\n[sandboxes.jail]\nkind = \"bwrap\"\nenv = [\"FAKE_DIR\"]\n"),
    )
    .unwrap();
    fs::write(
        setup.dir.join("factory/workflows/implement.toml"),
        format!("sandbox = \"jail\"\n{WORKFLOW}"),
    )
    .unwrap();
}

/// Runs a worker that consults sentry once, to the end, and returns the investigator's run row.
fn investigated(setup: &mut Setup) -> Vec<String> {
    setup.memory(60);
    setup.calls(&[
        consult("sentry"),
        reported("Release 4.2."),
        result("passed"),
    ]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| with(rows, 4, "done") == 2);
    let runs = setup.rows("runs");
    runs.into_iter()
        .find(|run| run[1] == "2")
        .expect("the investigator ran")
}

#[test]
fn a_jailed_workers_investigator_starts_in_the_workers_jail() {
    let mut setup = consulting("consult-jailed", "");
    jail_the_worker(&setup);

    let investigator = investigated(&mut setup);

    assert_eq!(investigator[11], "jail", "{investigator:?}");
    // The daemon's start check, then one launch per run, the investigator's among them.
    let runs = setup.rows("runs").len();
    assert_eq!(fake_log(&setup, "bwrap.log").len(), 1 + runs);
}

#[test]
fn an_unjailed_workers_investigator_starts_as_before() {
    let mut setup = consulting("consult-unjailed", "");

    let investigator = investigated(&mut setup);

    assert_eq!(investigator[11], "-", "{investigator:?}");
    assert_eq!(fake_log(&setup, "bwrap.log"), Vec::<String>::new());
}
