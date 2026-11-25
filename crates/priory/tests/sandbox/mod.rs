//! Sandboxes: a workflow's `sandbox` starts each of its runs' agents in a bubblewrap jail
//! (docs/daemon/sandboxes.md). The tests marked `ignore` need the real `bwrap`; run them with
//! `cargo test -p priory --test runtime sandbox:: -- --include-ignored`.

use super::*;

/// The jail every test's jailed workflow names. The fake `claude` writes in its folder and reads
/// it from `FAKE_DIR`, so the sandbox names both.
fn sandboxes(setup: &Setup) -> String {
    format!(
        "name = \"demo\"\n\n[sandboxes.jail]\nkind = \"bwrap\"\nwrite = [\"{}\"]\nenv = [\"FAKE_DIR\", \"KEPT\"]\n",
        setup.dir.join("fake").display()
    )
}

/// A factory whose `implement` workflow is jailed and whose `plain` workflow is not.
fn jailed(setup: &Setup) {
    fs::write(setup.dir.join("factory/factory.toml"), sandboxes(setup)).unwrap();
    fs::write(
        setup.dir.join("factory/workflows/implement.toml"),
        format!("sandbox = \"jail\"\n{WORKFLOW}"),
    )
    .unwrap();
    fs::write(setup.dir.join("factory/workflows/plain.toml"), WORKFLOW).unwrap();
}

/// A task request of the `plain` workflow.
fn plain(issue: u64) -> Message {
    let mut message = requested(issue);
    message.data["workflow"] = json!("plain");
    message
}

/// What the daemon's variables hold beside the test's own: one the sandbox keeps, one it does
/// not, and the server's token.
const VARIABLES: [(&str, &str); 3] = [
    ("KEPT", "kept"),
    ("DROPPED", "dropped"),
    ("PRIORY_TOKEN", "secret"),
];

fn the_step_ended(setup: &Setup) -> serde_json::Value {
    setup
        .trace()
        .into_iter()
        .find(|event| event["type"] == "factory.step.ended")
        .expect("a step ended")["data"]
        .clone()
}

#[test]
fn a_jailed_run_starts_its_agent_in_bubblewrap_with_only_the_variables_it_names() {
    let mut setup = Setup::new("jail-args");
    jailed(&setup);
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));

    setup.start_with(Path::new(env!("CARGO_BIN_EXE_priory")), &VARIABLES);

    setup.until("tasks", |rows| with(rows, 4, "done") == 1);
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    let worktree = fs::canonicalize(worktree).unwrap().display().to_string();
    let jail = fake_log(&setup, "bwrap.log");
    // The daemon's start check, then the run.
    assert_eq!(jail[0], "--ro-bind / / true", "{jail:?}");
    let run = &jail[1];
    for part in [
        "--die-with-parent --new-session --unshare-pid --ro-bind / / --dev-bind /dev /dev --proc /proc --tmpfs /home --tmpfs /root --tmpfs /tmp".to_owned(),
        format!("--bind {worktree} {worktree}"),
        format!("--chdir {worktree}"),
        // Where an upgrade's next binary waits.
        format!(
            "--tmpfs {}",
            fs::canonicalize(setup.dir.join("home")).unwrap().join("upgrade").display()
        ),
    ] {
        assert!(run.contains(&part), "{part}\n{run}");
    }
    let session = &setup.rows("runs")[0][4];
    let news = fs::canonicalize(setup.dir.join("home"))
        .unwrap()
        .join("news")
        .join(session);
    let news = news.display();
    assert!(run.contains(&format!("--ro-bind {news} {news}")), "{run}");
    let names = fake_log(&setup, "env.log");
    for kept in [
        "KEPT",
        "FAKE_DIR",
        "PATH",
        "HOME",
        "PRIORY_HOME",
        "PRIORY_SESSION",
    ] {
        assert!(names.iter().any(|name| name == kept), "{kept}: {names:?}");
    }
    for dropped in ["DROPPED", "PRIORY_TOKEN", "PRIORY_DB", "HERDR_BUS_DB"] {
        assert!(
            !names.iter().any(|name| name == dropped),
            "{dropped}: {names:?}"
        );
    }
    assert_eq!(setup.rows("runs")[0][11], "jail");
    let ended = the_step_ended(&setup);
    assert_eq!(ended["sandbox"], "jail");
    assert!(ended["session_start_ms"].as_u64().is_some(), "{ended}");
}

#[test]
fn a_run_of_a_workflow_without_sandbox_starts_as_before() {
    let mut setup = Setup::new("jail-none");
    setup.calls(&[result("passed")]);
    setup.publish(&requested(42));

    setup.start_with(Path::new(env!("CARGO_BIN_EXE_priory")), &VARIABLES);

    setup.until("tasks", |rows| with(rows, 4, "done") == 1);
    assert_eq!(fake_log(&setup, "bwrap.log"), Vec::<String>::new());
    let names = fake_log(&setup, "env.log");
    assert!(names.iter().any(|name| name == "DROPPED"), "{names:?}");
    assert_eq!(setup.rows("runs")[0][11], "-");
    let ended = the_step_ended(&setup);
    assert_eq!(ended.get("sandbox"), None, "{ended}");
    assert!(ended["session_start_ms"].as_u64().is_some(), "{ended}");
}

#[test]
fn a_runner_that_cannot_start_bubblewrap_blocks_its_jailed_task_and_runs_the_others() {
    let mut setup = Setup::new("jail-fails");
    jailed(&setup);
    fs::write(setup.dir.join("fake/bwrap-fail"), "").unwrap();
    setup.calls(&[result("passed")]);
    setup.publish(&requested(1));
    setup.publish(&plain(2));

    setup.start();

    let tasks = setup.until("tasks", |rows| {
        rows.len() == 2 && rows[0][4] == "blocked" && rows[1][4] == "done"
    });
    assert_eq!(
        tasks[0][7],
        "workflow implement needs sandbox jail, and this runner cannot start bubblewrap: bwrap: setting up uid map: Permission denied"
    );
    let runs = setup.rows("runs");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0][1], "2");
}

#[test]
fn a_jailed_task_blocked_on_a_runner_that_cannot_jail_runs_jailed_on_the_runner_given_it() {
    let mut a = runner("jail-give", "a", None);
    let mut b = runner("jail-give", "b", Some(&a));
    jailed(&a);
    jailed(&b);
    fs::write(a.dir.join("fake/bwrap-fail"), "").unwrap();
    b.calls(&[result("passed")]);
    a.publish(&requested(1));
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));

    a.start();
    let tasks = a.until_own_tasks(|rows| rows.first().is_some_and(|row| row[4] == "blocked"));
    assert!(
        tasks[0][7].starts_with("workflow implement needs sandbox jail"),
        "{tasks:?}"
    );
    b.start();
    b.until("inbox", |rows| {
        rows.iter().any(|row| {
            row.contains(&"priory.runner.alive".to_owned()) && row.contains(&"b".to_owned())
        })
    });
    let factory = b.dir.join("factory");
    let given = b
        .command(exe.to_str().unwrap())
        .args([
            "give",
            factory.to_str().unwrap(),
            "github:owner/repo#1",
            "b",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        given.status.success(),
        "{}",
        String::from_utf8_lossy(&given.stderr)
    );

    b.until_own_tasks(|rows| rows.first().is_some_and(|row| row[4] == "done"));
    a.until_own_tasks(|rows| rows[0][4] == "dropped");
    assert_eq!(a.rows("runs").len(), 0);
    assert_eq!(b.rows("runs")[0][11], "jail");
}

/// The real `bwrap`, which these tests need: they fail, and do not skip, without it.
fn real(setup: &Setup) {
    let found = harness::command("bwrap")
        .args(["--ro-bind", "/", "/", "true"])
        .status();
    assert!(
        found.is_ok_and(|status| status.success()),
        "these tests need bubblewrap: install it and allow it user namespaces"
    );
    fs::write(setup.dir.join("fake/bwrap-real"), "").unwrap();
}

/// Every live process whose environment holds `PRIORY_SESSION=<session>`.
fn processes_of(session: &str) -> Vec<String> {
    let marker = format!("PRIORY_SESSION={session}");
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().parse::<u32>().is_ok())
        .filter(|entry| {
            fs::read(entry.path().join("environ")).is_ok_and(|environ| {
                environ
                    .split(|byte| *byte == 0)
                    .any(|variable| variable == marker.as_bytes())
            })
        })
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
#[ignore = "needs bubblewrap"]
fn a_real_jail_hides_the_home_keeps_the_worktree_and_reaches_the_daemon() {
    let mut setup = Setup::new("jail-real");
    jailed(&setup);
    real(&setup);
    let home = setup.dir.join("user");
    fs::create_dir_all(home.join(".ssh")).unwrap();
    fs::write(home.join(".ssh/id"), "private key\n").unwrap();
    fs::write(
        setup.dir.join("fake/probe"),
        "cat \"$HOME/.ssh/id\" && echo read the key\n\
         echo x > jailed && echo wrote the worktree\n\
         git add jailed && git -c user.name=t -c user.email=t@t commit -qm jailed && echo committed\n\
         echo x > ../beside\n",
    )
    .unwrap();
    setup.calls(&[json!([bash("git push origin main")]), result("passed")]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    let exe = Path::new(env!("CARGO_BIN_EXE_priory"));
    let mut variables = VARIABLES.to_vec();
    let home_text = home.display().to_string();
    variables.push(("HOME", &home_text));

    setup.start_with(exe, &variables);

    // The guard asks the daemon over the socket bound into the jail.
    let replies = setup.replied(1);
    let reason = guard_denied(&replies[0]).expect("the push is denied");
    assert!(reason.contains("git-push-default"), "{reason}");
    assert!(
        setup
            .trace()
            .iter()
            .any(|event| event["type"] == "factory.tool.denied"),
        "no denial recorded"
    );
    let probe = fs::read_to_string(setup.dir.join("fake/probe.log")).unwrap();
    assert!(!probe.contains("read the key"), "{probe}");
    assert!(probe.contains("wrote the worktree\ncommitted\n"), "{probe}");
    let worktree = setup.dir.join("home/worktrees/owner/repo/1");
    // Next to the worktree is the jail's own empty folder, which no write reaches the host from.
    assert!(!worktree.with_file_name("beside").exists());
    assert_eq!(
        setup.git(&worktree, &["log", "-1", "--format=%s"]),
        "jailed\n"
    );
    let names = fake_log(&setup, "env.log");
    assert!(names.iter().any(|name| name == "KEPT"), "{names:?}");
    assert!(
        !names
            .iter()
            .any(|name| name == "DROPPED" || name == "PRIORY_TOKEN"),
        "{names:?}"
    );
    let screen = setup.priory(&["screen", "1"]);
    assert!(screen.status.success(), "{screen:?}");
    // The sweep reads a jailed process's folder in /proc as the worktree.
    let found = priory_runtime::worktrees::found(&setup.dir.join("home"));
    let found = found
        .iter()
        .find(|found| found.path.ends_with("repo/1"))
        .unwrap();
    assert_eq!(
        priory_runtime::worktrees::retire_blocker(&setup.dir.join("home"), found).as_deref(),
        Some("a process runs in it")
    );

    // With the daemon stopped, the jailed run's Stop hook spools, and the next daemon applies it.
    setup.crash();
    setup.release("hold-stop");
    let spools = setup.dir.join("home/spools");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !fs::read_dir(&spools).is_ok_and(|dirs| {
        dirs.flatten().any(|dir| {
            fs::metadata(dir.path().join("signals.spool")).is_ok_and(|file| file.len() > 0)
        })
    }) {
        assert!(Instant::now() < deadline, "nothing spooled");
        std::thread::sleep(Duration::from_millis(50));
    }
    setup.start_with(exe, &variables);

    setup.until("tasks", |rows| with(rows, 4, "done") == 1);
    let ended = the_step_ended(&setup);
    assert_eq!(ended["sandbox"], "jail");
    assert!(ended["tokens"]["input"].as_u64().unwrap() > 0, "{ended}");
    let session = setup.rows("runs")[0][4].clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !processes_of(&session).is_empty() {
        assert!(
            Instant::now() < deadline,
            "the jail outlived its tmux session: {:?}",
            processes_of(&session)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A probe for the fake `claude` that plants a `pre-push` hook in the clone's hooks folder, and in
/// a folder of its own that it names in `core.hooksPath`. Each hook leaves a file in the fake's
/// folder when it runs.
fn hooks(setup: &Setup) -> String {
    let fake = setup.dir.join("fake").display().to_string();
    format!(
        "common=$(git rev-parse --path-format=absolute --git-common-dir)\n\
         mkdir -p \"$common/hooks\" {fake}/hooks\n\
         printf '#!/bin/sh\\ntouch {fake}/clone-hook-ran\\n' > \"$common/hooks/pre-push\" && echo planted the clone hook\n\
         printf '#!/bin/sh\\ntouch {fake}/path-hook-ran\\n' > {fake}/hooks/pre-push\n\
         chmod +x \"$common/hooks/pre-push\" {fake}/hooks/pre-push\n\
         git config core.hooksPath {fake}/hooks && echo set hooksPath\n"
    )
}

/// Starts a jailed task on `OPEN_PR` whose agent runs `probe`, and opens its pull request.
fn opened_after(setup: &mut Setup, probe: &str, variables: &[(&str, &str)]) -> String {
    fs::write(setup.dir.join("factory/factory.toml"), sandboxes(setup)).unwrap();
    fs::write(
        setup.dir.join("factory/workflows/implement.toml"),
        format!("sandbox = \"jail\"\n{OPEN_PR}"),
    )
    .unwrap();
    fs::write(setup.dir.join("fake/probe"), probe).unwrap();
    setup.calls(&[passed_with_evidence()]);
    setup.publish(&requested(42));
    setup.start_with(Path::new(env!("CARGO_BIN_EXE_priory")), variables);
    setup.until("asks", |rows| rows.len() == 1);
    setup.gone("priory-1");
    let commit = setup.commit(1, "widget.txt");
    let answered = setup.priory(&["answer", "1", "open", "--revision", "1"]);
    assert!(answered.status.success(), "{answered:?}");
    let actions = setup.until("actions", |rows| rows.len() == 1);
    assert_eq!(actions[0][4], "opened", "{actions:?}");
    commit
}

#[test]
fn a_hook_a_jailed_agent_plants_never_runs_in_the_daemons_push() {
    let mut setup = Setup::new("jail-hooks");

    let probe = hooks(&setup);

    let commit = opened_after(&mut setup, &probe, &VARIABLES);

    let probe = fs::read_to_string(setup.dir.join("fake/probe.log")).unwrap();
    assert!(
        probe.contains("planted the clone hook\nset hooksPath"),
        "{probe}"
    );
    assert_eq!(setup.pushed("priory/1"), commit);
    for ran in ["clone-hook-ran", "path-hook-ran"] {
        assert!(!setup.dir.join("fake").join(ran).exists(), "{ran}");
    }
    let git = fs::canonicalize(setup.dir.join("home/repos/owner/repo/.git")).unwrap();
    let run = &fake_log(&setup, "bwrap.log")[1];
    for read in [
        "config",
        "hooks",
        "commondir",
        "worktrees",
        "worktrees/1/commondir",
    ] {
        let path = git.join(read).display().to_string();
        assert!(
            run.contains(&format!("--ro-bind {path} {path}")),
            "{read}\n{run}"
        );
    }
}

#[test]
#[ignore = "needs bubblewrap"]
fn a_real_jail_leaves_nothing_the_host_runs_and_hides_the_state_file() {
    let mut setup = Setup::new("jail-real-hooks");
    real(&setup);
    let fake = setup.dir.join("fake").display().to_string();
    let claude = setup.dir.join("claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("settings.json"), "{}\n").unwrap();
    let claude = claude.display().to_string();
    let state = setup.dir.join("priory.db").display().to_string();
    let probe = format!(
        "{}printf '#!/bin/sh\\ntouch {fake}/ssh-ran\\n' > {fake}/ssh && chmod +x {fake}/ssh\n\
         git config core.sshCommand {fake}/ssh || echo sshCommand refused\n\
         git config remote.origin.pushurl ssh://git@example.invalid/x || echo pushurl refused\n\
         echo '{{\"hooks\":{{}}}}' > {claude}/settings.json || echo settings refused\n\
         echo /x > \"$common/commondir\" || echo commondir refused\n\
         echo t > {claude}/projects/probe.jsonl && echo wrote a transcript\n\
         ls {state} && echo saw the state file\n",
        hooks(&setup)
    );
    let home = setup.dir.join("user").display().to_string();
    let mut variables = VARIABLES.to_vec();
    variables.push(("HOME", &home));

    let commit = opened_after(&mut setup, &probe, &variables);

    let probe = fs::read_to_string(setup.dir.join("fake/probe.log")).unwrap();
    for refused in [
        "sshCommand refused",
        "pushurl refused",
        "commondir refused",
        "settings refused",
    ] {
        assert!(probe.contains(refused), "{refused}\n{probe}");
    }
    assert!(probe.contains("wrote a transcript"), "{probe}");
    for done in [
        "planted the clone hook",
        "set hooksPath",
        "saw the state file",
    ] {
        assert!(!probe.contains(done), "{done}\n{probe}");
    }
    assert_eq!(setup.pushed("priory/1"), commit);
    for ran in ["clone-hook-ran", "path-hook-ran", "ssh-ran"] {
        assert!(!setup.dir.join("fake").join(ran).exists(), "{ran}");
    }
    let clone = setup.dir.join("home/repos/owner/repo");
    let config = fs::read_to_string(clone.join(".git/config")).unwrap();
    assert!(
        !config.contains("example.invalid") && !config.contains(&fake),
        "{config}"
    );
    assert_eq!(
        fs::read_to_string(setup.dir.join("claude/settings.json")).unwrap(),
        "{}\n"
    );
}

#[test]
fn a_jailed_run_keeps_the_clones_own_worktree_config() {
    let mut setup = Setup::new("jail-keeps-config");
    jailed(&setup);
    setup.calls(&[result("passed"), result("passed")]);
    setup.publish(&requested(1));
    setup.start();
    setup.until("tasks", |rows| with(rows, 4, "done") == 1);
    let git = setup.dir.join("home/repos/owner/repo/.git");
    let kept = "[core]\n\tsparseCheckout = true\n";
    fs::write(git.join("config.worktree"), kept).unwrap();

    setup.publish(&requested(2));

    setup.until("tasks", |rows| with(rows, 4, "done") == 2);
    assert_eq!(
        fs::read_to_string(git.join("config.worktree")).unwrap(),
        kept
    );
}

#[test]
fn a_jailed_tasks_lead_answers_its_ask_in_the_tasks_jail() {
    let mut setup = hands_off("jail-lead", &format!("sandbox = \"jail\"\n{WORKFLOW}"));
    fs::write(setup.dir.join("factory/factory.toml"), sandboxes(&setup)).unwrap();
    setup.calls(&[ask(), answered("sqlite"), result("passed")]);
    setup.publish(&requested(42));

    setup.start();

    setup.until("tasks", |rows| {
        rows.len() == 2 && rows.iter().all(|row| row[4] == "done")
    });
    let runs = setup.rows("runs");
    let lead = runs.iter().find(|run| run[1] == "2").expect("the lead ran");
    assert_eq!(lead[11], "jail", "{runs:?}");
    // The daemon's start check, then one launch per run, the lead's among them.
    assert_eq!(fake_log(&setup, "bwrap.log").len(), 1 + runs.len());
}

/// A jailed hands-off task whose lead stays queued for room, then the daemon stopped and `breaks`
/// run on the state file, then the daemon started with room: returns the lead task's row.
fn lead_after(name: &str, breaks: &str) -> (Setup, Vec<String>) {
    let mut setup = hands_off(name, &format!("sandbox = \"jail\"\n{WORKFLOW}"));
    fs::write(setup.dir.join("factory/factory.toml"), sandboxes(&setup)).unwrap();
    let lead = LEAD.replace(
        "session = \"shared\"\n",
        "session = \"shared\"\nmemory = \"8GB\"\n",
    );
    fs::write(setup.dir.join("factory/agents/lead/agent.md"), lead).unwrap();
    setup.memory(12);
    setup.calls(&[ask()]);
    setup.publish(&requested(42));
    setup.start();
    setup.until("tasks", |rows| {
        rows.len() == 2 && rows.iter().any(|row| row[0] == "1" && row[4] == "asleep")
    });
    setup.crash();
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    db.execute_batch(breaks).unwrap();
    drop(db);
    setup.memory(60);

    setup.start();

    // A lead that cannot start is dropped with its reason, and its ask goes back to the person.
    let tasks = setup.until("tasks", |rows| {
        rows.iter().any(|row| row[0] == "2" && row[4] == "dropped")
    });
    let lead = tasks.into_iter().find(|row| row[0] == "2").unwrap();
    assert!(
        !setup.rows("runs").iter().any(|run| run[1] == "2"),
        "the lead ran"
    );
    // The two daemons' start checks and the worker's one launch: no jail for the lead.
    assert_eq!(fake_log(&setup, "bwrap.log").len(), 3);
    (setup, lead)
}

#[test]
fn a_lead_whose_served_tasks_workflow_left_the_factory_is_dropped_and_never_starts() {
    let (_setup, lead) = lead_after(
        "jail-lead-gone",
        "UPDATE tasks SET workflow = 'gone' WHERE id = 1;",
    );
    assert_eq!(
        lead[7],
        "task 2 serves a task of workflow gone, which the factory no longer has"
    );
}

#[test]
fn a_lead_whose_routed_ask_names_no_task_is_dropped_and_never_starts() {
    let (_setup, lead) = lead_after(
        "jail-lead-malformed",
        "UPDATE inbox SET data = json_set(data, '$.task', 'one') WHERE type = 'priory.ask.routed';",
    );
    assert_eq!(
        lead[7],
        "task 2 serves a task its routed ask does not name, so its sandbox is unknown"
    );
}
