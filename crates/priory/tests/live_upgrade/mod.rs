//! An upgrade with a live run, from the binary `PRIORY_TEST_OLD` to `PRIORY_TEST_NEW`, which adds
//! a migration, then back to this build. `scripts/live-upgrade.sh` builds both, and CI runs it.

use super::*;

const TOKEN: &str = "live-upgrade-token";

fn binary(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} names no binary"))
}

fn schema(setup: &Setup) -> (i64, i64) {
    let db = rusqlite::Connection::open(setup.dir.join("priory.db")).unwrap();
    let read = |field: &str| {
        db.pragma_query_value(None, field, |row| row.get(0))
            .unwrap()
    };
    (read("user_version"), read("application_id"))
}

/// Puts `exe` in the place of the installed binary in one rename, as an install does.
fn install(setup: &Setup, exe: &str) -> PathBuf {
    let installed = setup.dir.join("bin/priory");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    let next = setup.dir.join("bin/priory.next");
    fs::copy(exe, &next).unwrap();
    fs::rename(&next, &installed).unwrap();
    installed
}

/// Stops the daemon as systemd does, and waits for it.
fn stop(setup: &mut Setup) {
    let mut daemon = setup.daemon.take().unwrap();
    send_signal(daemon.id(), "TERM");
    daemon.wait().unwrap();
}

fn start(setup: &mut Setup, installed: &Path) {
    setup.start_with(installed, &[("PRIORY_TEST_TOKEN", TOKEN)]);
}

fn get(address: &str, path: &str) -> u16 {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(mut stream) = std::net::TcpStream::connect(address) {
            use std::io::{Read, Write};
            let head = format!(
                "GET {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n"
            );
            let mut response = String::new();
            if stream.write_all(head.as_bytes()).is_ok()
                && stream.read_to_string(&mut response).is_ok()
                && let Some(status) = response.split_whitespace().nth(1)
            {
                return status.parse().unwrap();
            }
        }
        assert!(Instant::now() < deadline, "{path} never answered");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn address(setup: &Setup) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let log = fs::read_to_string(setup.dir.join("daemon.log")).unwrap_or_default();
        let found = log
            .lines()
            .filter_map(|line| line.trim().strip_prefix("priory: serving on http://"))
            .next_back();
        if let Some(address) = found {
            return address.to_owned();
        }
        assert!(Instant::now() < deadline, "the daemon never served: {log}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The rows `<exe> <verb>` prints, once `done` holds: the state is the old binary's until the swap.
fn until_on(
    setup: &Setup,
    exe: &str,
    verb: &str,
    done: impl Fn(&[Vec<String>]) -> bool,
) -> Vec<Vec<String>> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let output = setup.command(exe).arg(verb).output().unwrap();
        let rows: Vec<Vec<String>> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect();
        if done(&rows) {
            return rows;
        }
        assert!(Instant::now() < deadline, "{verb}: {output:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "scripts/live-upgrade.sh builds its binaries and runs it"]
fn processes_that_outlive_an_upgrade_and_its_rollback_keep_working() {
    let (old, new) = (binary("PRIORY_TEST_OLD"), binary("PRIORY_TEST_NEW"));
    let mut setup = Setup::new("live-upgrade");
    // The factory serves the web view on a port its log names.
    let file = setup.dir.join("factory/factory.toml");
    let toml = fs::read_to_string(&file).unwrap();
    let serve = "\n[serve]\nlisten = \"127.0.0.1:0\"\ntoken_env = \"PRIORY_TEST_TOKEN\"\n";
    fs::write(&file, toml + serve).unwrap();
    // The old binary makes its own state, at its own schema.
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(setup.dir.join(format!("priory.db{suffix}")));
    }
    let installed = install(&setup, &old);
    let path = installed.to_str().unwrap().to_owned();
    setup.calls(&[]);
    setup.hold("hold-stop");
    setup.publish(&requested(42));
    start(&mut setup, &installed);
    let runs = until_on(&setup, &old, "runs", |rows| {
        rows.iter().any(|row| row[1] == "1")
    });
    let session = runs[0][4].clone();
    setup.written("fake/typed.log");
    let before = schema(&setup);

    // What runs on the old binary across the swap.
    let mut kept = Kept::on(&setup, &session, &old);
    assert!(!kept.call("tldr", json!({ "text": "Before." })).1);
    let fake = setup.dir.join("fake");
    let gated = |name: &str, gb: &str| {
        fs::write(fake.join(format!("hold-{name}")), "").unwrap();
        setup
            .command(&old)
            .args(["memory", "acquire", "--gb", gb, "--", "sh", "-c"])
            .arg(format!(
                "while [ -e hold-{name} ]; do sleep 0.05; done; echo {name} >> gated"
            ))
            .current_dir(&fake)
            .spawn()
            .unwrap()
    };
    let mut first = gated("first", "40");
    until_on(&setup, &old, "memory", |rows| {
        rows.iter().any(|row| row[2] == "held")
    });
    let mut second = gated("second", "40");
    until_on(&setup, &old, "memory", |rows| rows.len() == 2);
    let revision = setup
        .command(&old)
        .args(["wait", "--since", "-1", "--timeout", "1s"])
        .output()
        .unwrap();
    let revision = String::from_utf8(revision.stdout).unwrap();
    let waiting = setup
        .command(&old)
        .args(["wait", "--since", revision.trim(), "--timeout", "180s"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut intake = setup
        .command(&old)
        .args(["intake", setup.dir.join("factory").to_str().unwrap()])
        .stderr(fs::File::create(setup.dir.join("intake.log")).unwrap())
        .spawn()
        .unwrap();
    let web = address(&setup);
    assert_eq!(get(&web, "/api/tasks"), 200);

    // The swap, with a tool call and a hook in its gap.
    stop(&mut setup);
    let installed = install(&setup, &new);
    let call = std::thread::spawn(move || {
        let reply = kept.call("progress", json!({ "latestChange": "In the gap." }));
        (reply, kept)
    });
    let hook = setup
        .command(&old)
        .args(["hook", "Notification"])
        .env("PRIORY_SESSION", &session)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = hook.stdin.as_ref().unwrap();
    std::io::Write::write_all(
        &mut stdin,
        format!(r#"{{"session_id":"{session}","message":"in the gap"}}"#).as_bytes(),
    )
    .unwrap();
    assert!(hook.wait_with_output().unwrap().status.success());
    std::thread::sleep(Duration::from_secs(1));
    start(&mut setup, &installed);

    let ((text, error), mut kept) = call.join().unwrap();
    assert!(!error, "the call in the gap: {text}");
    let after = schema(&setup);
    assert_eq!(after.0, before.0 + 1, "the new daemon migrated");
    assert!(
        after.1 > 0 && after.1 <= before.0,
        "its floor {after:?} lets {before:?} in"
    );
    assert!(
        !setup.dir.join("home/signals.spool").exists(),
        "the spooled hook was applied"
    );
    assert!(!kept.call("tldr", json!({ "text": "After." })).1);
    assert_eq!(get(&address(&setup), "/api/tasks"), 200);
    let posted = setup
        .command(&path)
        .args(["post", "live", "after"])
        .output()
        .unwrap();
    assert!(posted.status.success(), "{posted:?}");
    let waited = waiting.wait_with_output().unwrap();
    assert!(waited.status.success(), "{waited:?}");
    assert_ne!(
        String::from_utf8(waited.stdout).unwrap().trim(),
        revision.trim()
    );
    fs::remove_file(fake.join("hold-first")).unwrap();
    assert!(first.wait().unwrap().success());
    fs::remove_file(fake.join("hold-second")).unwrap();
    assert!(second.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(fake.join("gated")).unwrap(),
        "first\nsecond\n"
    );
    // A restart of the intake, as the upgrade's helper does, runs on the new binary.
    intake.kill().unwrap();
    intake.wait().unwrap();

    // Back to this build, which reads the newer state by its floor and keeps what came after.
    stop(&mut setup);
    let installed = install(&setup, env!("CARGO_BIN_EXE_priory"));
    start(&mut setup, &installed);
    assert!(
        !kept
            .call("tldr", json!({ "text": "After the rollback." }))
            .1
    );
    assert_eq!(schema(&setup), after);
    assert_eq!(get(&address(&setup), "/api/tasks"), 200);
    let inbox = setup.priory(&["inbox"]);
    assert!(
        String::from_utf8_lossy(&inbox.stdout).contains("after"),
        "{inbox:?}"
    );
    let runs = setup.rows("runs");
    assert!(
        runs.iter().all(|run| run[6] == "-"),
        "the run stays live: {runs:?}"
    );
    setup.release("hold-stop");
}
