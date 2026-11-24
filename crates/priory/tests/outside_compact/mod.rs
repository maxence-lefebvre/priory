//! A person's own session asks for `/compact` through `priory mcp --outside`, and the factory's
//! `[asks] type` command types it (docs/daemon/tools.md#compact-from-the-session).

use super::*;

const SESSION: [(&str, &str); 2] = [
    ("HC_TERMINAL_ID", "t9"),
    ("CLAUDE_CODE_SESSION_ID", "8f3c2a10"),
];
const NAME: &str = "t9 8f3c2a10";
const PREFIX: &str =
    "Priory compacted this session, as it asked. The session's own note, not a person's words: ";
const QUEUED: &str = "Queued /compact keep the plan. End your turn now: Priory types it once this session is idle, then your note. It gives up after 1 hour without the plugin's Stop hook.";

/// A setup whose fake `type` command records its arguments. `check` fails while `fake/gone`
/// exists, and `send` exits 75 while `fake/busy` does and waits while `fake/hold` does.
fn typing(name: &str) -> Setup {
    let mut setup = Setup::new(name);
    fs::write(
        setup.dir.join("factory/factory.toml"),
        "name = \"demo\"\n\n[asks]\ntype = \"bin/type\"\n",
    )
    .unwrap();
    let script = setup.dir.join("factory/bin/type");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(
        &script,
        "#!/bin/sh\n\
         for arg in \"$@\"; do printf '%s\\0' \"$arg\"; done >> \"$FAKE_DIR/type.log\"\n\
         printf '\\036' >> \"$FAKE_DIR/type.log\"\n\
         if [ \"$1\" = check ] && [ -e \"$FAKE_DIR/gone\" ]; then echo 't9 runs no claude' >&2; exit 1; fi\n\
         if [ \"$1\" = send ]; then\n\
           while [ -e \"$FAKE_DIR/hold\" ]; do sleep 0.05; done\n\
           [ ! -e \"$FAKE_DIR/busy\" ] || exit 75\n\
         fi\nexit 0\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    setup.start();
    listening(&setup);
    setup
}

/// The arguments of each call of the fake `type` command but its checks, in order.
fn typed(setup: &Setup) -> Vec<Vec<String>> {
    fs::read_to_string(setup.dir.join("fake/type.log"))
        .unwrap_or_default()
        .split_terminator('\x1e')
        .map(|call| call.split_terminator('\0').map(str::to_owned).collect())
        .filter(|call: &Vec<String>| call[0] != "check")
        .collect()
}

fn until_typed(setup: &Setup, count: usize) -> Vec<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let calls = typed(setup);
        if calls.len() >= count {
            return calls;
        }
        assert!(Instant::now() < deadline, "typed only {calls:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn send(text: &str) -> Vec<String> {
    ["send", NAME, text].map(str::to_owned).to_vec()
}

fn hook(setup: &Setup, event: &str, fields: serde_json::Value) {
    let mut input = json!({ "session_id": "8f3c2a10" });
    for (key, value) in fields.as_object().unwrap() {
        input[key] = value.clone();
    }
    let mut hook = setup
        .command(env!("CARGO_BIN_EXE_priory"))
        .args(["hook", event, "--outside"])
        .env_remove("PRIORY_SESSION")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(
        &mut hook.stdin.take().unwrap(),
        input.to_string().as_bytes(),
    )
    .unwrap();
    let output = hook.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stdout.is_empty() && output.stderr.is_empty(),
        "{output:?}"
    );
}

fn compact(setup: &Setup) -> (String, bool) {
    setup.outside_call(
        &SESSION,
        "compact",
        json!({ "focus": "keep\n the plan", "then": "! priory answer 1 ok" }),
    )
}

/// The reason of the only failed compaction in the trace.
fn failure(setup: &Setup) -> String {
    let event = until_traced(setup, "factory.session.compact_failed");
    event["data"]["reason"].as_str().unwrap().to_owned()
}

/// Gives the daemon time to do what it must not.
fn settle() {
    std::thread::sleep(Duration::from_secs(3));
}

#[test]
fn a_session_that_asks_is_compacted_once_idle_then_gets_its_follow_up() {
    let setup = typing("compact-flow");

    assert_eq!(compact(&setup), (QUEUED.to_owned(), false));
    let (again, refused) = compact(&setup);
    assert!(refused && again.contains("already queued"), "{again}");
    settle();
    assert!(typed(&setup).is_empty(), "typed before the session's Stop");

    hook(&setup, "Stop", json!({}));
    let calls = until_typed(&setup, 1);
    assert_eq!(calls, [send("/compact keep the plan")]);
    until_traced(&setup, "factory.session.compact_typed");
    hook(&setup, "Stop", json!({}));
    hook(&setup, "SessionStart", json!({ "source": "resume" }));
    settle();
    assert_eq!(typed(&setup).len(), 1, "typed before the compaction ended");

    hook(&setup, "SessionStart", json!({ "source": "compact" }));
    let calls = until_typed(&setup, 2);
    let follow_up = format!("{PREFIX}! priory answer 1 ok");
    assert_eq!(calls[1], send(&follow_up));
    until_traced(&setup, "factory.session.compact_typed");
    hook(&setup, "UserPromptSubmit", json!({ "prompt": follow_up }));
    let prompted = until_traced(&setup, "factory.session.compact_prompted");

    assert_eq!(prompted["data"]["session"], NAME);
    assert_eq!(prompted["data"]["focus_chars"], 13);
    assert_eq!(prompted["data"]["then_chars"], 20);
    for kind in ["requested", "typed", "compacted"] {
        let kind = format!("factory.session.compact_{kind}");
        assert_eq!(traced(&setup, &kind).len(), 1, "{kind}");
    }
    // The refused second call.
    assert!(failure(&setup).contains("already queued"));
    settle();
    assert_eq!(typed(&setup).len(), 2, "typed again after the follow-up");
    assert!(!compact(&setup).1, "a finished request blocks the next one");
}

#[test]
fn a_session_priory_cannot_reach_is_refused_at_call_time_and_nothing_is_typed() {
    let setup = typing("compact-unreachable");
    fs::write(setup.dir.join("fake/gone"), "").unwrap();

    let (gone, refused) = compact(&setup);
    let arguments = json!({ "then": "go on" });
    let anonymous = setup.outside_call(&[], "compact", arguments.clone());
    let terminal = setup.outside_call(&SESSION[..1], "compact", arguments);

    assert!(refused);
    assert_eq!(
        gone,
        "Priory cannot type into this session: exit 1: t9 runs no claude"
    );
    assert_eq!(
        anonymous,
        (
            "Priory cannot type into this session: it runs in no hc terminal".to_owned(),
            true
        )
    );
    assert_eq!(
        terminal,
        (
            "Priory cannot type into this session: it names no Claude session".to_owned(),
            true
        )
    );
    hook(&setup, "Stop", json!({}));
    settle();
    assert!(typed(&setup).is_empty());
    assert!(traced(&setup, "factory.session.compact_requested").is_empty());
    assert_eq!(failure(&setup), gone);
}

#[test]
fn a_factory_with_no_type_command_refuses() {
    let mut setup = Setup::new("compact-no-type");
    setup.start();
    listening(&setup);

    assert_eq!(
        compact(&setup),
        (
            "Priory cannot type into this session: the factory names no [asks] type command"
                .to_owned(),
            true
        )
    );
}

#[test]
fn a_text_that_is_long_secret_or_holds_control_characters_is_refused() {
    let setup = typing("compact-texts");
    let key = format!("ghp_{}", "a".repeat(30));
    for (arguments, why) in [
        (json!({}), "then must be a line of text"),
        (json!({ "then": " " }), "then must be a line of text"),
        (json!({ "then": 4 }), "then must be a line of text"),
        (json!({ "then": "x", "focus": 4 }), "focus must be a string"),
        (json!({ "then": "x", "to": "t1" }), "unknown field to"),
        (json!({ "then": "x".repeat(4001) }), "longer than 4000"),
        (
            json!({ "then": "x", "focus": "x".repeat(1001) }),
            "longer than 1000",
        ),
        (json!({ "then": key }), "holds a secret"),
        (json!({ "then": "x", "focus": key }), "holds a secret"),
        (json!({ "then": "x\u{1b}[201~" }), "control character"),
        (
            json!({ "then": "x", "focus": "x\u{1b}" }),
            "control character",
        ),
    ] {
        let (reply, refused) = setup.outside_call(&SESSION, "compact", arguments.clone());
        assert!(refused && reply.contains(why), "{arguments}: {reply}");
    }
    assert!(traced(&setup, "factory.session.compact_requested").is_empty());
}

#[test]
fn a_prompt_after_the_stop_defers_the_compaction_to_the_next_stop() {
    let setup = typing("compact-defer");
    fs::write(setup.dir.join("fake/busy"), "").unwrap();
    compact(&setup);
    hook(&setup, "Stop", json!({}));
    until_typed(&setup, 2);

    // The busy tries typed nothing, so a person's prompt sends the request back to waiting.
    hook(
        &setup,
        "UserPromptSubmit",
        json!({ "prompt": "one more thing" }),
    );
    settle();
    fs::remove_file(setup.dir.join("fake/busy")).unwrap();
    let tries = typed(&setup).len();
    settle();
    assert_eq!(typed(&setup).len(), tries);
    assert!(traced(&setup, "factory.session.compact_typed").is_empty());

    hook(&setup, "Stop", json!({}));
    until_traced(&setup, "factory.session.compact_typed");
}

#[test]
fn a_persons_prompt_after_the_compact_drops_the_follow_up() {
    let setup = typing("compact-person");
    compact(&setup);
    hook(&setup, "Stop", json!({}));
    until_traced(&setup, "factory.session.compact_typed");

    hook(&setup, "UserPromptSubmit", json!({ "prompt": "stop that" }));
    hook(&setup, "SessionStart", json!({ "source": "compact" }));

    assert_eq!(failure(&setup), "a person typed first");
    settle();
    assert_eq!(typed(&setup).len(), 1);
}

#[test]
fn a_follow_up_with_no_prompt_hook_gets_one_more_enter_then_fails() {
    let setup = typing("compact-enter");
    compact(&setup);
    hook(&setup, "Stop", json!({}));
    until_traced(&setup, "factory.session.compact_typed");
    hook(&setup, "SessionStart", json!({ "source": "compact" }));

    let calls = until_typed(&setup, 3);

    assert_eq!(calls[2], ["enter", NAME]);
    assert!(failure(&setup).contains("was not confirmed"));
    assert_eq!(typed(&setup).len(), 3);
}

#[test]
fn a_restarted_daemon_never_types_a_text_again() {
    let mut setup = typing("compact-restart");
    fs::write(setup.dir.join("fake/hold"), "").unwrap();
    compact(&setup);
    hook(&setup, "Stop", json!({}));
    until_typed(&setup, 1);

    setup.crash();
    setup.start();
    listening(&setup);
    fs::remove_file(setup.dir.join("fake/hold")).unwrap();

    assert!(failure(&setup).contains("restart"));
    settle();
    assert_eq!(typed(&setup).len(), 1);
}
