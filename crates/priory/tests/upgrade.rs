//! An upgrade with fake binaries, a fake systemd and a fake release server. The fake binaries are
//! scripts around the `priory` this build made, so the daemons that start are real.

#[allow(dead_code)]
mod harness;
#[allow(dead_code)]
mod tmux_dir;

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const REAL: &str = env!("CARGO_BIN_EXE_priory");

/// Runs `$FAKE/exe` for each unit. With `$FAKE/kill-<verb>`, it kills the helper that calls it once.
const SYSTEMCTL: &str = r#"#!/bin/sh
shift
verb=$1
shift
echo "$verb $*" >> "$FAKE/systemctl.log"
alive() {
  state=$(sed 's/.*) //; s/ .*//' "/proc/$1/stat" 2>/dev/null)
  [ -n "$state" ] && [ "$state" != Z ]
}
pid() { cat "$FAKE/$1.pid" 2>/dev/null || echo 0; }
case "$verb" in
  stop)
    for unit in "$@"; do
      running=$(pid "$unit")
      [ "$running" != 0 ] && kill "$running" 2>/dev/null
      while [ "$running" != 0 ] && alive "$running"; do sleep 0.05; done
      [ "$running" != 0 ] && : > "$FAKE/$unit.stopped"
      rm -f "$FAKE/$unit.pid"
    done
    [ -e "$FAKE/on-stop" ] && . "$FAKE/on-stop" ;;
  start)
    for unit in "$@"; do
      alive "$(pid "$unit")" && continue
      verb_of=daemon
      case "$unit" in *intake*) verb_of=intake ;; esac
      ulimit -S -f unlimited 2>/dev/null
      (
        [ -e "$FAKE/env" ] && . "$FAKE/env"
        exec "$FAKE/exe" "$verb_of" "$FAKE/factory"
      ) >> "$FAKE/$unit.log" 2>&1 < /dev/null &
      echo $! > "$FAKE/$unit.pid"
      rm -f "$FAKE/$unit.stopped"
    done ;;
  show)
    for unit in "$@"; do :; done
    running=$(pid "$unit")
    alive "$running" || running=0
    case "$*" in
      *MainPID*) echo "$running" ;;
      *EnvironmentFiles*) [ -e "$FAKE/env" ] && [ ! -e "$FAKE/inline" ] && echo "$FAKE/env (ignore_errors=no)" ;;
      *Environment*) cat "$FAKE/inline" 2>/dev/null ;;
      *Restart*) cat "$FAKE/restart" 2>/dev/null || echo on-failure ;;
      *ActiveState*) [ "$running" = 0 ] && { [ -e "$FAKE/$unit.stopped" ] && echo inactive || { [ -e "$FAKE/$unit.pid" ] && echo failed || echo inactive; }; } || echo active ;;
      *ExecMainCode*) [ -e "$FAKE/$unit.stopped" ] && echo 2 || echo 1 ;;
      *ExecMainStatus*) [ -e "$FAKE/$unit.stopped" ] && echo 15 || echo 1 ;;
      *ExecStart*) echo "{ path=$FAKE/exe ; argv[]=$FAKE/exe daemon ; }" ;;
    esac ;;
esac
if [ -e "$FAKE/kill-$verb" ]; then
  rm "$FAKE/kill-$verb"
  kill -9 $PPID
fi
exit 0
"#;

/// Starts a service's command in the background, with its `--setenv` settings.
const SYSTEMD_RUN: &str = r#"#!/bin/sh
[ "$1" = --version ] && { echo 'systemd 258 (fake)'; exit 0; }
scope=no
wait=no
while [ $# -gt 0 ] && [ "$1" != -- ]; do
  case "$1" in
    --setenv=*=*) export "${1#--setenv=}" ;;
    --wait) wait=yes; kept=$TOKEN; unset TOKEN SECRET ;;
    --setenv=TOKEN) export TOKEN="$kept" ;;
    --property=EnvironmentFile=*) . "${1#--property=EnvironmentFile=}" ;;
    --scope) scope=yes ;;
  esac
  shift
done
shift
[ $scope = yes ] && exec "$@"
[ $wait = yes ] && exec "$@"
echo "$*" >> "$FAKE/systemd-run.log"
[ -e "$FAKE/no-helper" ] && exit 0
"$FAKE/as-helper" >> "$FAKE/helper.log" 2>&1 < /dev/null &
"#;

/// Serves `$FAKE/releases/<the url's last part>`, or fails as curl does when it cannot connect.
const CURL: &str = r#"#!/bin/sh
echo "$*" >> "$FAKE/curl.log"
while [ $# -gt 1 ]; do
  [ "$1" = --output ] && to=$2
  shift
done
name=${1##*/}
name=${name%%\?*}
[ -e "$FAKE/releases/$name" ] || { echo 'curl: (7) no server' >&2; exit 7; }
cp "$FAKE/releases/$name" "$to"
"#;

/// What the fake binaries' attestation holds, since no test can sign a real one.
const GENUINE: &[u8] = b"genuine";

/// Not a test: the process that runs the helper, as `priory upgrade resume` does, but with a check
/// the fake binaries pass.
#[test]
#[ignore = "the other tests run it"]
fn helper_process() {
    let verify = |answer: &[u8], _: &str, name: &str, sha256: &str| match answer == GENUINE
        && name == target()
        && sha256.len() == 64
    {
        true => Ok(()),
        false => Err("it is not genuine".to_owned()),
    };
    if let Err(error) = priory_core::swap::resume(&verify) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

struct Setup {
    dir: PathBuf,
    tmux: TmuxDir,
}

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A `priory` that says it is `version` and does `daemon` before it runs as the real daemon.
fn fake(version: &str, check: &str, daemon: &str) -> String {
    format!(
        "#!/bin/sh\ncase \"$1\" in\n--version) echo 'priory {version}'; exit 0 ;;\n\
         check) {check} ;;\nintake) exec sleep 600 ;;\ndaemon) {daemon} ;;\nesac\n\
         exec '{REAL}' \"$@\"\n"
    )
}

fn old() -> String {
    fake("0.0.1", "exit 0", ":")
}

impl Setup {
    fn new(name: &str, settings: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("upgrade")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        for folder in ["factory", "bin", "home/upgrade", "releases"] {
            fs::create_dir_all(dir.join(folder)).unwrap();
        }
        fs::write(
            dir.join("factory/factory.toml"),
            format!("name = \"demo\"\n{settings}"),
        )
        .unwrap();
        script(&dir.join("bin/systemctl"), SYSTEMCTL);
        script(&dir.join("bin/systemd-run"), SYSTEMD_RUN);
        script(&dir.join("bin/curl"), CURL);
        let me = std::env::current_exe().unwrap();
        let me = me.display();
        script(
            &dir.join("as-helper"),
            &format!("#!/bin/sh\nexec '{me}' --exact helper_process --ignored --nocapture\n"),
        );
        for name in ["attestations.json", "own.json"] {
            fs::write(dir.join("home/upgrade").join(name), GENUINE).unwrap();
        }
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
        };
        // The state and the log exist, with one report to find again.
        succeeded(&setup.priory(&["daemon", setup.path("factory").to_str().unwrap(), "--once"]));
        succeeded(&setup.priory(&["post", "marker", "kept"]));
        setup
    }

    fn start(&self, unit: &str) {
        let mut command = self.command("systemctl");
        succeeded(&command.args(["--user", "start", unit]).output().unwrap());
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn command(&self, program: &str) -> Command {
        let mut command = harness::command(program);
        let path = std::env::var("PATH").unwrap();
        command
            .env("PATH", format!("{}:{path}", self.path("bin").display()))
            .env("FAKE", &self.dir)
            .env("PRIORY_HOME", self.path("home"))
            .env("PRIORY_DB", self.path("priory.db"))
            .env("HERDR_BUS_DB", self.path("bus.db"))
            .env("TMUX_TMPDIR", self.tmux.path());
        command
    }

    fn priory(&self, args: &[&str]) -> Output {
        self.command(REAL).args(args).output().unwrap()
    }

    /// The old binary and its services, and the new binary `new` staged with a journal at `phase`.
    fn upgrade(&self, new: &str, edit: impl FnOnce(&mut Value)) {
        script(&self.path("exe"), &old());
        let started = self
            .command("systemctl")
            .args([
                "--user",
                "start",
                "priory-daemon.service",
                "priory-intake.service",
            ])
            .status()
            .unwrap();
        assert!(started.success());
        script(&self.path("home/upgrade/staged"), new);
        script(&self.path("home/upgrade/helper"), &old());
        let mut journal = json!({
            "op": "op", "phase": "stopping", "from": "0.0.1", "to": CURRENT,
            "schema_from": 1, "schema_to": 1, "asset": target(),
            "attestations": self.path("home/upgrade/attestations.json"),
            "own": self.path("home/upgrade/own.json"),
            "exe": self.path("exe"), "staged": self.path("home/upgrade/staged"),
            "sha256": sha256(new.as_bytes()), "kept": self.path("exe.0.0.1"),
            "factory": self.path("factory"),
            "state": self.path("priory.db"), "bus": self.path("bus.db"),
            "state_backup": self.path("priory.db.backup"), "bus_backup": self.path("bus.db.backup"),
            "units": ["priory-daemon.service", "priory-intake.service"],
            "answers_within": 30, "watch": 1,
        });
        edit(&mut journal);
        self.wait("the old daemon answers", || self.answers());
        fs::write(self.path("home/upgrade/journal.json"), journal.to_string()).unwrap();
    }

    /// Runs the helper as its unit does: again while it fails or is killed.
    fn helper(&self) {
        for _ in 0..5 {
            let output = self
                .command(self.path("as-helper").to_str().unwrap())
                .output();
            let output = output.unwrap();
            let log = [&output.stdout[..], &output.stderr[..]].concat();
            let path = self.path("helper.log");
            let before = fs::read(&path).unwrap_or_default();
            fs::write(path, [before, log].concat()).unwrap();
            if output.status.success() {
                return;
            }
        }
        panic!("the helper never ended\n{}", self.logs());
    }

    fn answers(&self) -> bool {
        // A dismissal of no ask needs a daemon to refuse it.
        let output = self.priory(&["dismiss", "999"]);
        !String::from_utf8_lossy(&output.stderr).contains("no priory daemon")
    }

    fn wait(&self, what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(90);
        while !done() {
            assert!(Instant::now() < until, "never: {what}\n{}", self.logs());
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn logs(&self) -> String {
        [
            "helper.log",
            "daemon.log",
            "priory-daemon.service.log",
            "systemctl.log",
        ]
        .map(|name| {
            let text = fs::read_to_string(self.path(name)).unwrap_or_default();
            format!("{name}:\n{text}")
        })
        .join("\n")
    }

    fn installed(&self) -> String {
        fs::read_to_string(self.path("exe")).unwrap()
    }

    /// The trace records of upgrades, once the daemon that runs has stored one.
    fn recorded(&self) -> Value {
        let mut found = Value::Null;
        self.wait("the daemon records the upgrade", || {
            let output = self.priory(&["trace"]);
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            let records: Vec<Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .filter(|record: &Value| {
                    record["type"]
                        .as_str()
                        .unwrap()
                        .starts_with("factory.upgrade.")
                })
                .collect();
            found = records.first().cloned().unwrap_or_default();
            assert!(records.len() <= 1, "{records:?}");
            !records.is_empty()
        });
        found
    }

    /// The old binary serves again with its data, and the release may be tried again later.
    fn assert_put_off(&self, name: &str, why: &str) {
        assert_eq!(self.installed(), old(), "{name}\n{}", self.logs());
        let record = self.recorded();
        assert_eq!(record["type"], "factory.upgrade.put_off", "{name}");
        let reason = record["data"]["reason"].as_str().unwrap();
        assert!(reason.contains(why), "{name}: {reason}");
        assert!(self.desk().contains("off: "), "{name}: {}", self.desk());
        self.wait("the old daemon answers", || self.answers());
        let inbox = self.priory(&["inbox"]);
        let inbox = String::from_utf8_lossy(&inbox.stdout).into_owned();
        assert!(inbox.contains("kept"), "{name}: {inbox}");
        assert!(!self.path("home/upgrade/journal.json").exists(), "{name}");
        let store = self.store();
        assert_eq!(store["refused"], json!({}), "{name}");
        assert!(store["put_off"][CURRENT]["until"].is_string(), "{name}");
        for left in ["exe.next", "exe.back", "home/upgrade/reserve"] {
            assert!(!self.path(left).exists(), "{name}: {left}");
        }
        for left in ["priory.db.backup", "bus.db.backup"] {
            assert!(!self.path(left).exists(), "{name}: {left}");
            let partial = self.path(&format!("{left}.partial"));
            assert!(!partial.exists(), "{name}: {left}.partial");
        }
    }

    fn store(&self) -> Value {
        let store = fs::read_to_string(self.path("home/upgrade/state.json")).unwrap();
        serde_json::from_str(&store).unwrap()
    }

    fn desk(&self) -> String {
        String::from_utf8_lossy(&self.priory(&["desk"]).stdout).into_owned()
    }

    /// The old binary serves again, with its data, and the new version is never tried again.
    fn assert_back(&self, name: &str, new: &str) -> Value {
        assert_eq!(self.installed(), old(), "{name}\n{}", self.logs());
        let record = self.recorded();
        assert_eq!(record["type"], "factory.upgrade.rolled_back", "{name}");
        assert!(
            self.desk().contains("priory went back from"),
            "{name}: {}",
            self.desk()
        );
        assert!(self.answers(), "{name}");
        let inbox = self.priory(&["inbox"]);
        assert!(
            String::from_utf8_lossy(&inbox.stdout).contains("kept"),
            "{name}: {inbox:?}"
        );
        assert!(!self.path("home/upgrade/journal.json").exists(), "{name}");
        let store = self.store();
        assert!(store["refused"][new].is_string(), "{name}: {store}");
        record
    }
}

fn succeeded(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn a_healthy_release_replaces_the_binary_and_keeps_the_old_one_and_backups() {
    let setup = Setup::new("healthy", "");
    let new = fake(CURRENT, "exit 0", ":");
    setup.upgrade(&new, |_| {});

    fs::set_permissions(setup.path("priory.db"), fs::Permissions::from_mode(0o600)).unwrap();
    setup.helper();

    assert_eq!(setup.installed(), new);
    assert_eq!(fs::read_to_string(setup.path("exe.0.0.1")).unwrap(), old());
    let mode = |name: &str| fs::metadata(setup.path(name)).unwrap().permissions().mode();
    assert_eq!(mode("priory.db.backup") & 0o777, 0o600);
    assert!(setup.path("priory.db.backup").exists() && setup.path("bus.db.backup").exists());
    assert!(!setup.path("home/upgrade/journal.json").exists());
    let record = setup.recorded();
    assert_eq!(record["type"], "factory.upgrade.installed");
    assert_eq!(record["data"]["from"], "0.0.1");
    assert_eq!(record["data"]["to"], CURRENT);
    assert!(
        setup.desk().contains("priory upgraded from 0.0.1"),
        "{}",
        setup.desk()
    );
}

#[test]
fn a_release_that_fails_goes_back_to_the_old_binary() {
    for (name, new, reason) in [
        (
            "check",
            fake(CURRENT, "echo broken >&2; exit 1", ":"),
            "broken",
        ),
        ("exits", fake(CURRENT, "exit 0", "exit 1"), "stopped before"),
        (
            "dies",
            fake(CURRENT, "exit 0", "exec timeout 4 \"$0\" serves; exit 0"),
            "stopped after it answered",
        ),
    ] {
        let new = new
            .replace(
                "intake)",
                "serves) exec '__REAL__' daemon \"$FAKE/factory\" ;;\nintake)",
            )
            .replace("__REAL__", REAL);
        let setup = Setup::new(&format!("fails-{name}"), "");
        setup.upgrade(&new, |journal| {
            journal["answers_within"] = json!(3);
            journal["watch"] = json!(8);
        });

        setup.helper();

        let record = setup.assert_back(name, CURRENT);
        let said = record["data"]["reason"].as_str().unwrap();
        assert!(said.contains(reason), "{name}: {said}");
    }
}

#[test]
fn a_daemon_that_answers_as_another_version_goes_back() {
    let setup = Setup::new("answers-as-another", "");
    setup.upgrade(&fake("9.9.9", "exit 0", ":"), |journal| {
        journal["to"] = json!("9.9.9")
    });

    setup.helper();

    let record = setup.assert_back("answers", "9.9.9");
    assert_eq!(
        record["data"]["reason"],
        format!("the daemon answers as {CURRENT}")
    );
}

#[test]
fn a_release_that_is_not_what_was_verified_stops_nothing() {
    type Edit = fn(&Setup);
    let edits: [(&str, Edit, &str); 3] = [
        (
            "changed",
            |setup| script(&setup.path("home/upgrade/staged"), "#!/bin/sh\n"),
            "changed after its check",
        ),
        (
            "forged",
            |setup| fs::write(setup.path("home/upgrade/attestations.json"), "forged").unwrap(),
            "its attestation does not verify",
        ),
        (
            "own-build",
            |setup| fs::write(setup.path("home/upgrade/own.json"), "forged").unwrap(),
            "the installed binary is not a release",
        ),
    ];
    for (name, edit, why) in edits {
        let setup = Setup::new(&format!("unverified-{name}"), "");
        setup.upgrade(&fake(CURRENT, "exit 0", ":"), |_| {});
        edit(&setup);

        setup.helper();

        setup.assert_put_off(name, why);
        let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
        assert!(!log.contains("stop "), "{name}: {log}");
        assert!(!setup.path("home/upgrade/staged").exists(), "{name}");
    }
}

/// The real verb, with the real check, which the fake attestation fails.
#[test]
fn the_real_helper_refuses_a_forged_attestation_and_a_run_s_shell() {
    let setup = Setup::new("real-helper", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |_| {});

    for args in [
        &["upgrade", "resume"][..],
        &["upgrade", "retry", "1.0.0"],
        &["upgrade", "probe", "x"],
    ] {
        let refused = setup
            .command(REAL)
            .args(args)
            .env("PRIORY_SESSION", "run")
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(2), "{args:?}");
    }
    assert!(setup.path("home/upgrade/journal.json").exists());
    succeeded(&setup.priory(&["upgrade", "resume"]));

    setup.assert_put_off("real", "its attestation does not verify");
}

#[test]
fn a_fault_of_this_machine_puts_the_upgrade_off() {
    let half_edited = "[ -e \"$FAKE/edited\" ] && exit 1; : > \"$FAKE/edited\"; exit 0";
    for (name, old_check, new, why) in [
        (
            "factory",
            "echo half-edited >&2; exit 1",
            fake(CURRENT, "exit 1", ":"),
            "the running version's priory check failed",
        ),
        (
            "edited-meanwhile",
            half_edited,
            fake(CURRENT, "exit 1", ":"),
            "the running version's priory check failed",
        ),
        (
            "disk",
            "exit 0",
            fake(CURRENT, "echo 'database or disk is full' >&2; exit 1", ":"),
            "disk is full",
        ),
        (
            "killed-check",
            "exit 0",
            fake(CURRENT, "kill -9 $$", ":"),
            "signal: 9",
        ),
        (
            "hangs",
            "exit 0",
            fake(CURRENT, "exit 0", "exec sleep 600"),
            "did not answer within",
        ),
    ] {
        let setup = Setup::new(&format!("machine-{name}"), "");
        setup.upgrade(&new, |journal| journal["answers_within"] = json!(3));
        let old = fake("0.0.1", old_check, ":");
        fs::remove_file(setup.path("exe")).unwrap();
        script(&setup.path("exe"), &old);

        setup.helper();

        // The comparison below is against the plain old binary.
        assert_eq!(setup.installed(), old, "{name}");
        fs::remove_file(setup.path("exe")).unwrap();
        script(&setup.path("exe"), &fake("0.0.1", "exit 0", ":"));
        setup.assert_put_off(name, why);
    }
}

fn add_table(path: &Path, table: &str) {
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch(&format!(
        "CREATE TABLE {table} (a); INSERT INTO {table} VALUES ('old')"
    ))
    .unwrap();
}

fn tables(path: &Path, name: &str) -> i64 {
    let db = rusqlite::Connection::open(path).unwrap();
    db.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .unwrap()
}

/// A daemon that migrates `state` and, when `bus`, the log, and then dies.
fn migrating(bus: bool) -> String {
    let test = std::env::current_exe().unwrap();
    let test = test.display();
    let bus = if bus { "both" } else { "state" };
    let daemon =
        format!("MIGRATE={bus} '{test}' --exact migrate_process --ignored >/dev/null; exit 1");
    fake(CURRENT, "exit 0", &daemon)
}

/// Not a test: the migration of a fake release.
#[test]
#[ignore = "the other tests run it"]
fn migrate_process() {
    let path = |name: &str| PathBuf::from(std::env::var_os(name).unwrap());
    let state = rusqlite::Connection::open(path("PRIORY_DB")).unwrap();
    if std::env::var("MIGRATE").unwrap() == "additive" {
        // What an additive migration and a minute of the new daemon leave.
        let version: i64 = state
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        state
            .execute_batch(&format!(
                "CREATE TABLE added (a); INSERT INTO added VALUES ('written meanwhile');
                 PRAGMA application_id = {version}; PRAGMA user_version = {};",
                version + 1
            ))
            .unwrap();
        return;
    }
    state
        .execute_batch("CREATE TABLE migrated (a); PRAGMA user_version = 9999")
        .unwrap();
    let bus = rusqlite::Connection::open(path("HERDR_BUS_DB")).unwrap();
    if std::env::var("MIGRATE").unwrap() == "both" {
        bus.execute_batch("CREATE TABLE migrated (a)").unwrap();
    }
    bus.execute_batch("CREATE TABLE webhook (a)").unwrap();
}

#[test]
fn a_migration_that_fails_puts_back_the_databases_it_changes_alone() {
    for (name, bus) in [("state", false), ("both", true)] {
        let setup = Setup::new(&format!("migration-{name}"), "");
        add_table(&setup.path("priory.db"), "marker");
        setup.upgrade(&migrating(bus), |journal| {
            journal["migrates_state"] = json!(true);
            journal["migrates_bus"] = json!(bus);
            journal["answers_within"] = json!(20);
        });

        setup.helper();

        setup.assert_back(name, CURRENT);
        assert_eq!(tables(&setup.path("priory.db"), "migrated"), 0, "{name}");
        assert_eq!(tables(&setup.path("priory.db"), "marker"), 1, "{name}");
        assert_eq!(tables(&setup.path("bus.db"), "migrated"), 0, "{name}");
        // What another process stored in a log the release does not migrate stays.
        let kept = i64::from(!bus);
        assert_eq!(tables(&setup.path("bus.db"), "webhook"), kept, "{name}");
    }
}

#[test]
fn an_additive_migration_rolls_back_to_the_old_binary_and_keeps_what_was_written() {
    let setup = Setup::new("additive", "");
    let version = |db: &rusqlite::Connection| -> i64 {
        db.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    };
    let before = version(&rusqlite::Connection::open(setup.path("priory.db")).unwrap());
    let test = std::env::current_exe().unwrap();
    let daemon = format!(
        "MIGRATE=additive '{}' --exact migrate_process --ignored >/dev/null; exit 1",
        test.display()
    );
    setup.upgrade(&fake(CURRENT, "exit 0", &daemon), |journal| {
        journal["answers_within"] = json!(20);
    });

    setup.helper();

    setup.assert_back("additive", CURRENT);
    let db = rusqlite::Connection::open(setup.path("priory.db")).unwrap();
    assert_eq!(version(&db), before + 1);
    let kept: String = db
        .query_row("SELECT a FROM added", [], |row| row.get(0))
        .unwrap();
    assert_eq!(kept, "written meanwhile");
}

#[test]
fn a_verb_is_refused_while_a_migration_may_be_put_back() {
    let setup = Setup::new("migration-verb", "");
    let verb = "\"$0\" desk > \"$FAKE/refused\" 2>&1; exit 1";
    setup.upgrade(&fake(CURRENT, "exit 0", verb), |journal| {
        journal["migrates_state"] = json!(true);
    });

    setup.helper();

    setup.assert_back("verb", CURRENT);
    let refused = fs::read_to_string(setup.path("refused")).unwrap();
    assert!(
        refused.contains("an upgrade of priory is in progress"),
        "{refused}"
    );
}

/// A kill after one database is back and before the other is: the backup that is gone was used.
#[test]
fn a_rollback_killed_between_two_restores_ends_with_both() {
    let setup = Setup::new("between-restores", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
        journal["phase"] = json!("rolling-back");
        journal["migrates_state"] = json!(true);
        journal["migrates_bus"] = json!(true);
        journal["opened"] = json!(true);
        journal["running"] = json!([true, true]);
        journal["refuse"] = json!(true);
        journal["reason"] = json!("it failed");
    });
    script(&setup.path("exe.0.0.1"), &old());
    let stopped = setup
        .command("systemctl")
        .args(["--user", "stop", "priory-daemon.service"])
        .status();
    assert!(stopped.unwrap().success());
    let bus = rusqlite::Connection::open(setup.path("bus.db")).unwrap();
    let to = setup.path("bus.db.backup");
    bus.execute("VACUUM INTO ?1", [to.to_str().unwrap()])
        .unwrap();
    bus.execute_batch("CREATE TABLE migrated (a)").unwrap();
    drop(bus);
    // The state is back already: what it holds now is the backup's.
    add_table(&setup.path("priory.db"), "marker");

    setup.helper();

    setup.assert_back("between", CURRENT);
    assert_eq!(tables(&setup.path("priory.db"), "marker"), 1);
    assert_eq!(tables(&setup.path("bus.db"), "migrated"), 0);
}

#[test]
fn a_process_that_holds_a_database_a_migration_changes_puts_it_off() {
    for (name, stage) in [("before", false), ("after", true)] {
        let setup = Setup::new(&format!("holder-{name}"), "");
        setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
            journal["migrates_bus"] = json!(true);
        });
        let hold = format!(
            "sh -c 'exec 9< \"{}\"; sleep 30' >/dev/null 2>&1 &\necho $! > \"$FAKE/holder\"; sleep 0.5\n",
            setup.path("bus.db").display()
        );
        if stage {
            fs::write(
                setup.path("on-stop"),
                format!("[ -e \"$FAKE/holder\" ] || {{ {hold} }}"),
            )
            .unwrap();
        } else {
            let status = setup.command("sh").args(["-c", &hold]).status();
            assert!(status.unwrap().success());
        }

        setup.helper();

        setup.assert_put_off(name, "bus.db open");
        let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
        assert_eq!(log.contains("stop "), stage, "{name}: {log}");
        let holder = fs::read_to_string(setup.path("holder")).unwrap();
        let _ = setup.command("kill").arg(holder.trim()).status();
    }
}

#[test]
fn a_migration_s_daemon_does_no_work_until_its_commit() {
    let setup = Setup::new("held", "");
    let new = fake(CURRENT, "exit 0", ":");
    setup.upgrade(&new, |journal| {
        journal["migrates_state"] = json!(true);
        journal["watch"] = json!(6);
    });
    let mut helper = setup
        .command(setup.path("as-helper").to_str().unwrap())
        .spawn()
        .unwrap();
    let mut refused = String::new();
    setup.wait("the held daemon refuses", || {
        let output = setup.priory(&["dismiss", "999"]);
        refused = String::from_utf8_lossy(&output.stderr).into_owned();
        refused.contains("in progress")
    });
    let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
    assert_eq!(
        log.matches("\nstart priory-daemon.service\n").count(),
        1,
        "{log}"
    );
    assert!(helper.wait().unwrap().success());

    assert_eq!(setup.installed(), new);
    assert_eq!(setup.recorded()["type"], "factory.upgrade.installed");
    assert!(setup.answers());
}

#[test]
fn a_backup_that_fails_starts_the_old_binary_again() {
    let setup = Setup::new("no-backup", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
        journal["state_backup"] = json!(setup.path("missing/priory.db"));
    });

    setup.helper();

    setup.assert_put_off("backup", "missing");
}

/// A file size limit fails the copy of a large log part of the way, as a full disk does.
#[test]
fn a_backup_that_runs_out_of_room_leaves_no_partial_and_starts_the_old_binary() {
    let setup = Setup::new("no-room", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |_| {});
    let db = rusqlite::Connection::open(setup.path("bus.db")).unwrap();
    db.execute_batch("CREATE TABLE filler (a); INSERT INTO filler VALUES (randomblob(6000000))")
        .unwrap();
    drop(db);
    let helper = setup.path("as-helper");
    let limited = fs::read_to_string(&helper)
        .unwrap()
        .replace("exec", "trap '' XFSZ\nulimit -S -f 8192\nexec");
    script(&helper, &limited);
    // One an interrupted backup of the same upgrade left.
    fs::write(setup.path("bus.db.backup.partial"), "left").unwrap();

    setup.helper();

    setup.assert_put_off("room", "bus.db.backup");
}

/// Each write of the journal, the store and the outcome fails with ENOSPC from the stop on.
#[test]
fn a_full_disk_starts_the_old_services_without_a_journal() {
    for (name, full) in [
        ("once", "ln -sf /dev/full \"$FAKE/home/upgrade/$file.next\""),
        (
            "for-good",
            "rm -rf \"$FAKE/home/upgrade/$file.next\"; mkdir -p \"$FAKE/home/upgrade/$file.next/x\"",
        ),
    ] {
        let setup = Setup::new(&format!("full-disk-{name}"), "");
        setup.upgrade(&fake(CURRENT, "exit 0", "exit 1"), |journal| {
            journal["state_backup"] = json!(setup.path("missing/priory.db"));
        });
        fs::write(
            setup.path("on-stop"),
            format!("for file in journal.json state.json; do {full}; done\n"),
        )
        .unwrap();

        setup.helper();

        assert_eq!(setup.installed(), old(), "{name}");
        assert!(!setup.path("home/upgrade/journal.json").exists(), "{name}");
        setup.wait("the old daemon answers", || setup.answers());
        for unit in ["priory-daemon.service", "priory-intake.service"] {
            assert!(setup.path(&format!("{unit}.pid")).exists(), "{name} {unit}");
        }
        assert!(!setup.path("missing").exists(), "{name}");
    }
}

/// A rollback with a disk where the journal and the store take nothing more.
#[test]
fn a_full_disk_does_not_stop_a_rollback() {
    let setup = Setup::new("full-rollback", "");
    let fill = "for file in journal.json state.json; do \
                mkdir -p \"$FAKE/home/upgrade/$file.next/x\"; done; exit 1";
    setup.upgrade(&fake(CURRENT, "exit 0", fill), |journal| {
        journal["migrates_state"] = json!(true);
    });

    setup.helper();

    assert_eq!(setup.installed(), old(), "{}", setup.logs());
    assert!(!setup.path("home/upgrade/journal.json").exists());
    setup.wait("the old daemon answers", || setup.answers());
    let outcome = fs::read_dir(setup.path("home/upgrade/outcomes")).unwrap();
    assert_eq!(outcome.count(), 1);
    // Once there is room again, the daemon refuses and records it.
    for file in ["journal.json", "state.json"] {
        fs::remove_dir_all(setup.path(&format!("home/upgrade/{file}.next"))).unwrap();
    }
    setup.assert_back("full", CURRENT);
}

#[test]
fn a_quiet_moment_that_passed_puts_the_upgrade_off_and_stops_nothing() {
    for (name, busy, why) in [
        (
            "grant",
            "INSERT INTO grants (bytes, pid, started, command) VALUES (1, 1, 1, 'build')",
            "a memory grant",
        ),
        (
            "pause",
            "INSERT INTO pause (id, reason, since) VALUES (1, 'x', 'now')",
            "priory is paused",
        ),
    ] {
        let setup = Setup::new(&format!("put-off-{name}"), "");
        setup.upgrade(&fake(CURRENT, "exit 0", ":"), |_| {});
        let db = rusqlite::Connection::open(setup.path("priory.db")).unwrap();
        db.execute(busy, []).unwrap();
        drop(db);

        setup.helper();

        assert!(setup.path("home/upgrade/staged").exists());
        setup.assert_put_off(name, why);
        let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
        assert!(!log.contains("stop "), "{name}: {log}");
    }
}

#[test]
fn a_service_a_person_stopped_stays_stopped() {
    for (name, new, migrates) in [
        ("installed", fake(CURRENT, "exit 0", ":"), false),
        ("migrated", fake(CURRENT, "exit 0", ":"), true),
        ("back", fake(CURRENT, "exit 0", "exit 1"), false),
    ] {
        let setup = Setup::new(&format!("stopped-{name}"), "");
        setup.upgrade(&new, |journal| journal["migrates_state"] = json!(migrates));
        let stop = |unit: &str| {
            let mut stopped = setup.command("systemctl");
            assert!(
                stopped
                    .args(["--user", "stop", unit])
                    .status()
                    .unwrap()
                    .success()
            );
        };
        stop("priory-intake.service");

        setup.helper();

        assert!(!setup.path("priory-intake.service.pid").exists(), "{name}");
        match name {
            "back" => drop(setup.assert_back(name, CURRENT)),
            _ => assert_eq!(setup.installed(), new, "{name}\n{}", setup.logs()),
        }
        assert!(setup.answers(), "{name}");
    }
}

#[test]
fn a_daemon_a_person_stops_during_the_watch_stays_stopped() {
    let setup = Setup::new("stopped-watched", "");
    let new = fake(CURRENT, "exit 0", ":");
    setup.upgrade(&new, |journal| journal["watch"] = json!(20));
    let mut helper = setup
        .command(setup.path("as-helper").to_str().unwrap())
        .spawn()
        .unwrap();
    setup.wait("the new daemon answers", || {
        setup.installed() == new && setup.answers()
    });
    let stopped = setup
        .command("systemctl")
        .args(["--user", "stop", "priory-daemon.service"])
        .status();
    assert!(stopped.unwrap().success());

    assert!(helper.wait().unwrap().success());

    assert_eq!(setup.installed(), old());
    assert!(!setup.path("priory-daemon.service.pid").exists());
    assert!(setup.path("priory-intake.service.pid").exists());
    let store = setup.path("home/upgrade/state.json");
    assert!(!store.exists() || setup.store()["refused"] == json!({}));
}

/// The helper's unit starts it again after a kill. Before the swap it goes on; after it, it goes back.
#[test]
fn a_helper_killed_in_any_phase_ends_on_one_binary() {
    let good = fake(CURRENT, "exit 0", ":");
    let killed_in_check = fake(
        CURRENT,
        "case $0 in *next) exit 0 ;; esac; \
         [ -e \"$FAKE/killed\" ] || { : > \"$FAKE/killed\"; kill -9 $PPID; }; exit 0",
        ":",
    );
    // Checked beside the old binary first, so it fails only in its place.
    let failing = fake(CURRENT, "case $0 in *next) exit 0 ;; esac; exit 1", ":");
    for (name, new, kill, installs, refused) in [
        ("stopping", &good, Some("stop"), true, false),
        ("installing", &killed_in_check, None, false, false),
        ("starting", &good, Some("start"), false, false),
        ("watching", &good, Some("show"), false, false),
        ("rolling-back", &failing, Some("stop-2"), false, true),
        ("restarting", &failing, Some("start"), false, true),
    ] {
        let setup = Setup::new(&format!("killed-{name}"), "");
        setup.upgrade(new, |journal| journal["answers_within"] = json!(10));
        match kill {
            // Only the second stop, the rollback's, kills: the first is already journaled over.
            Some("stop-2") => {
                let path = setup.path("bin/systemctl");
                let text = fs::read_to_string(&path).unwrap().replace(
                    "[ -e \"$FAKE/kill-$verb\" ]",
                    "[ -e \"$FAKE/kill-$verb\" ] && grep -q '\"rolling-back\"' \"$FAKE/home/upgrade/journal.json\"",
                );
                script(&path, &text);
                fs::write(setup.path("kill-stop"), "").unwrap();
            }
            // The helper asks systemd before the stop too, where a kill changes nothing.
            Some("show") => {
                let path = setup.path("bin/systemctl");
                let text = fs::read_to_string(&path).unwrap().replace(
                    "[ -e \"$FAKE/kill-$verb\" ]",
                    "[ -e \"$FAKE/kill-$verb\" ] && grep -q '\"starting\"' \"$FAKE/home/upgrade/journal.json\"",
                );
                script(&path, &text);
                fs::write(setup.path("kill-show"), "").unwrap();
            }
            Some(verb) => fs::write(setup.path(format!("kill-{verb}").as_str()), "").unwrap(),
            None => {}
        }

        setup.helper();

        let killed = setup.path("killed").exists()
            || ["stop", "start", "show"]
                .iter()
                .all(|verb| !setup.path(&format!("kill-{verb}")).exists());
        assert!(killed, "{name}: nothing was killed");
        if installs {
            assert_eq!(setup.installed(), *new, "{name}\n{}", setup.logs());
            assert_eq!(
                setup.recorded()["type"],
                "factory.upgrade.installed",
                "{name}"
            );
        } else if refused {
            setup.assert_back(name, CURRENT);
        } else {
            setup.assert_put_off(name, "the helper of the upgrade stopped");
        }
    }
}

/// The states a kill leaves between two commands of the helper, where no fake can kill it.
#[test]
fn a_helper_resumes_from_each_phase_of_the_journal() {
    for (phase, migrates, installs) in [
        ("stopping", true, true),
        ("installing", true, false),
        ("starting", true, false),
        ("committed", true, true),
        ("rolling-back", true, false),
        ("restarting", true, false),
    ] {
        let setup = Setup::new(&format!("resumes-{phase}"), "");
        let new = fake(CURRENT, "exit 0", ":");
        setup.upgrade(&new, |journal| {
            journal["phase"] = json!(phase);
            journal["migrates_state"] = json!(migrates);
            journal["migrates_bus"] = json!(migrates);
            if phase != "stopping" {
                journal["running"] = json!([true, true]);
            }
            journal["refuse"] = json!(matches!(phase, "rolling-back" | "restarting"));
            journal["reason"] = json!("it failed");
            journal["opened"] = json!(matches!(phase, "starting" | "rolling-back"));
        });
        if phase != "stopping" {
            for name in ["priory.db", "bus.db"] {
                let db = rusqlite::Connection::open(setup.path(name)).unwrap();
                let to = setup.path(&format!("{name}.backup"));
                db.execute("VACUUM INTO ?1", [to.to_str().unwrap()])
                    .unwrap();
            }
            script(&setup.path("exe.0.0.1"), &old());
            if phase != "restarting" {
                script(&setup.path("exe"), &new);
            }
        }
        if phase == "restarting" {
            let store = json!({ "refused": { CURRENT: "it failed" } });
            fs::write(setup.path("home/upgrade/state.json"), store.to_string()).unwrap();
        }

        setup.helper();

        if installs {
            assert_eq!(setup.installed(), new, "{phase}\n{}", setup.logs());
            assert_eq!(
                setup.recorded()["type"],
                "factory.upgrade.installed",
                "{phase}"
            );
        } else if matches!(phase, "installing" | "starting") {
            setup.assert_put_off(phase, "the helper of the upgrade stopped");
        } else {
            setup.assert_back(phase, CURRENT);
        }
    }
}

#[test]
fn a_service_that_starts_inside_an_interrupted_upgrade_starts_its_helper() {
    for verb in ["daemon", "intake"] {
        let setup = Setup::new(&format!("interrupted-{verb}"), "");
        setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
            journal["phase"] = json!("installing");
            journal["running"] = json!([true, true]);
        });
        script(&setup.path("exe.0.0.1"), &old());

        let refused = setup.priory(&[verb, setup.path("factory").to_str().unwrap()]);

        assert!(!refused.status.success(), "{verb}");
        let said = String::from_utf8_lossy(&refused.stderr).into_owned();
        assert!(said.contains("was interrupted"), "{verb}: {said}");
        setup.wait("the helper ends", || {
            !setup.path("home/upgrade/journal.json").exists()
        });
        setup.assert_put_off(verb, "the helper of the upgrade stopped");
    }
}

/// A fault that fails both daemons alike, such as a missing secret, is never the release's.
#[test]
fn an_old_daemon_that_fails_after_the_rollback_too_clears_the_refusal() {
    let setup = Setup::new("both-fail", "");
    setup.upgrade(&fake(CURRENT, "exit 0", "exit 1"), |_| {});
    // From the next start on, the old one fails too.
    let broken = fake("0.0.1", "exit 0", "[ -e \"$FAKE/broken\" ] && exit 1");
    script(&setup.path("exe"), &broken);
    fs::write(setup.path("broken"), "").unwrap();

    setup.helper();

    assert_eq!(setup.installed(), broken, "{}", setup.logs());
    assert!(!setup.path("home/upgrade/journal.json").exists());
    fs::remove_file(setup.path("broken")).unwrap();
    setup.start("priory-daemon.service");
    let record = setup.recorded();
    assert_eq!(record["type"], "factory.upgrade.put_off");
    let reason = record["data"]["reason"].as_str().unwrap();
    assert!(reason.contains("the daemon does not answer"), "{reason}");
    assert!(setup.desk().contains("off: "), "{}", setup.desk());
    assert_eq!(setup.store()["refused"], json!({}));
    assert!(setup.store()["put_off"][CURRENT]["until"].is_string());
}

/// A port nothing listens on now.
fn port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn served(port: u16) -> String {
    format!("[serve]\nlisten = \"127.0.0.1:{port}\"\ntoken_env = \"TOKEN\"\n")
}

#[test]
fn a_daemon_that_could_not_start_again_puts_the_upgrade_off_and_stops_nothing() {
    let github = "[webhooks.github]\nsecret_env = \"SECRET\"\n";
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let other = taken.local_addr().unwrap().port();
    let own = port();
    let same = served(own);
    for (name, factory, env, why) in [
        ("token", same.clone(), "", "TOKEN is unset"),
        (
            "secret",
            format!("{same}{github}"),
            "export TOKEN=0123456789abcdef0123456789abcdef",
            "SECRET is unset",
        ),
        (
            "bind",
            served(other),
            "export TOKEN=0123456789abcdef0123456789abcdef",
            "listen on 127.0.0.1",
        ),
    ] {
        let setup = Setup::new(&format!("startable-{name}"), "");
        let write = |text: &str| {
            fs::write(
                setup.path("factory/factory.toml"),
                format!("name = \"demo\"\n{text}"),
            )
            .unwrap();
        };
        setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
            journal["bound"] = json!(format!("127.0.0.1:{own}"));
        });
        write(&factory);
        fs::write(setup.path("env"), format!("{env}\n")).unwrap();

        setup.helper();

        // The machine's fix of it, and an unchanged factory.
        write("");
        setup.assert_put_off(name, why);
        let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
        assert!(!log.contains("stop "), "{name}: {log}");
    }
}

/// The daemon's own port is no fault, and neither is a token the unit itself sets.
#[test]
fn a_daemon_that_serves_is_upgraded() {
    for inline in [false, true] {
        let port = port();
        let setup = Setup::new(&format!("serves-{inline}"), &served(port));
        let token = "0123456789abcdef0123456789abcdef";
        fs::write(setup.path("env"), format!("export TOKEN={token}\n")).unwrap();
        let new = fake(CURRENT, "exit 0", ":");
        setup.upgrade(&new, |journal| {
            journal["bound"] = json!(format!("127.0.0.1:{port}"));
        });
        if inline {
            fs::write(setup.path("inline"), format!("\"A=b c\" TOKEN={token}\n")).unwrap();
        }

        setup.helper();

        assert_eq!(setup.installed(), new, "{}", setup.logs());
        assert_eq!(setup.recorded()["type"], "factory.upgrade.installed");
    }
}

#[test]
fn an_intake_that_ends_during_the_watch_puts_the_upgrade_off() {
    let setup = Setup::new("intake-ends", "");
    let new = fake(CURRENT, "exit 0", ":").replace("exec sleep 600", "sleep 3; exit 1");
    setup.upgrade(&new, |journal| journal["watch"] = json!(6));
    std::thread::sleep(Duration::from_secs(7));

    setup.helper();

    setup.assert_put_off("intake", "the intake did not stay up");
}

#[test]
fn an_intake_in_a_crash_loop_before_the_stop_is_not_watched() {
    let setup = Setup::new("intake-loops", "");
    let new = fake(CURRENT, "exit 0", ":").replace("exec sleep 600", "sleep 3; exit 1");
    setup.upgrade(&new, |journal| journal["watch"] = json!(8));
    // As systemd does in a loop: the process is a moment old.
    let pid = fs::read_to_string(setup.path("priory-intake.service.pid")).unwrap();
    succeeded(&setup.command("kill").arg(pid.trim()).output().unwrap());
    setup.start("priory-intake.service");

    setup.helper();

    assert_eq!(setup.installed(), new, "{}", setup.logs());
    assert_eq!(setup.recorded()["type"], "factory.upgrade.installed");
}

/// A quota leaves room on the disk that this user cannot take.
#[test]
fn a_quota_is_no_fault_of_the_release() {
    for (name, new, tool) in [
        (
            "said",
            fake(
                CURRENT,
                "echo 'Disk quota exceeded (os error 122)' >&2; exit 1",
                ":",
            ),
            "exit 0",
        ),
        (
            "measured",
            fake(CURRENT, "exit 0", "exit 1"),
            "echo 'fallocate: fallocate failed: Disk quota exceeded' >&2; exit 1",
        ),
    ] {
        let setup = Setup::new(&format!("quota-{name}"), "");
        script(
            &setup.path("bin/fallocate"),
            &format!("#!/bin/sh\n{tool}\n"),
        );
        setup.upgrade(&new, |_| {});

        setup.helper();

        setup.assert_put_off(
            name,
            if name == "said" {
                "quota"
            } else {
                "almost full"
            },
        );
    }
}

#[test]
fn a_helper_that_is_late_stops_nothing() {
    let setup = Setup::new("late", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
        journal["ready_by"] = json!(1);
    });

    setup.helper();

    setup.assert_put_off("late", "took too long");
    let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
    assert!(!log.contains("stop "), "{log}");
}

const UPGRADE: &str = "[upgrade]\nchannel = \"patch\"\n";

/// The next patch of the running version, which the fake release server offers.
fn next() -> String {
    let (line, patch) = CURRENT.rsplit_once('.').unwrap();
    format!("{line}.{}", patch.parse::<u64>().unwrap() + 1)
}

fn target() -> String {
    format!("priory-{}-unknown-linux-musl", std::env::consts::ARCH)
}

impl Setup {
    /// Starts the installed daemon as its unit does.
    fn serve(&self) {
        // A link, so the daemon runs from the path its unit names.
        fs::hard_link(REAL, self.path("exe"))
            .or_else(|_| fs::copy(REAL, self.path("exe")).map(drop))
            .unwrap();
        let started = self
            .command("systemctl")
            .args(["--user", "start", "priory-daemon.service"])
            .status()
            .unwrap();
        assert!(started.success());
    }

    fn releases(&self, binary: &str, sum: &str, attestations: &str) {
        let list =
            json!([{ "tag_name": format!("v{}", next()), "draft": false, "prerelease": false }]);
        for (name, text) in [
            ("releases".to_owned(), list.to_string()),
            (target(), binary.to_owned()),
            (
                format!("{}.sha256", target()),
                format!("{sum}  {}\n", target()),
            ),
            (
                format!("sha256:{}", sha256(binary.as_bytes())),
                attestations.to_owned(),
            ),
        ] {
            fs::write(self.path("releases").join(name), text).unwrap();
        }
    }
}

#[test]
fn without_the_key_nothing_is_looked_up() {
    let setup = Setup::new("off", "");
    setup.serve();
    setup.wait("the daemon answers", || setup.answers());
    std::thread::sleep(Duration::from_secs(3));

    assert!(!setup.path("curl.log").exists());
}

#[test]
fn a_release_that_fails_its_checks_is_refused_on_the_desk_and_changes_nothing() {
    let binary = fake(&next(), "exit 0", ":");
    let sum = sha256(binary.as_bytes());
    for (name, sum, attestations, said) in [
        ("sum", "0".repeat(64), "{}", "its checksum differs"),
        (
            "none",
            sum.clone(),
            "{\"attestations\":[]}",
            "GitHub holds no attestation",
        ),
        ("parse", sum, "not json", "do not parse"),
    ] {
        let setup = Setup::new(&format!("refused-{name}"), UPGRADE);
        setup.releases(&binary, &sum, attestations);
        setup.serve();

        setup.wait("the desk says so", || setup.desk().contains(said));

        assert_eq!(
            setup.recorded()["type"],
            "factory.upgrade.refused",
            "{name}"
        );
        assert!(!setup.path("home/upgrade/staged").exists(), "{name}");
        assert!(!setup.path("systemd-run.log").exists(), "{name}");
        assert_eq!(
            fs::read(setup.path("exe")).unwrap(),
            fs::read(REAL).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn a_server_that_does_not_answer_stages_nothing() {
    let setup = Setup::new("no-server", UPGRADE);
    setup.serve();
    setup.wait("the daemon looked", || setup.path("curl.log").exists());
    setup.wait("it says why", || setup.logs().contains("cannot download"));

    let log = fs::read_to_string(setup.path("curl.log")).unwrap();
    assert!(log.contains("--proto =https --proto-redir =https"), "{log}");
    assert!(!setup.path("home/upgrade/staged").exists());
    assert!(setup.answers());
}

#[test]
fn a_factory_the_machine_s_runner_file_opts_out_is_never_upgraded() {
    let setup = Setup::new("opted-out", UPGRADE);
    let host = priory_runtime::room::hostname().unwrap();
    fs::create_dir_all(setup.path("factory/runners")).unwrap();
    fs::write(
        setup.path(&format!("factory/runners/{host}.toml")),
        "platform = { os = \"linux\" }\nupgrade = false\n",
    )
    .unwrap();
    setup.serve();
    setup.wait("the daemon answers", || setup.answers());
    std::thread::sleep(Duration::from_secs(3));

    assert!(!setup.path("curl.log").exists());
}

impl Setup {
    fn stage(&self, binary: &str) {
        script(&self.path("home/upgrade/staged"), binary);
        let staged = json!({
            "version": next(), "sha256": sha256(binary.as_bytes()), "since": "2026-01-01T00:00:00Z",
        });
        fs::write(self.path("home/upgrade/staged.json"), staged.to_string()).unwrap();
    }
}

/// Neither a `systemd-run` that starts nothing nor a helper that does not run stops the daemon.
#[test]
fn a_helper_that_never_starts_leaves_the_daemon_at_work() {
    let setup = Setup::new("no-helper", UPGRADE);
    fs::write(setup.path("no-helper"), "").unwrap();
    setup.stage(&fake(&next(), "exit 0", ":"));
    setup.serve();

    let record = setup.recorded();

    assert_eq!(record["type"], "factory.upgrade.put_off");
    let reason = record["data"]["reason"].as_str().unwrap();
    assert!(reason.contains("did not start within"), "{reason}");
    assert!(!setup.path("home/upgrade/journal.json").exists());
    assert!(setup.answers());
    let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
    assert!(!log.contains("stop priory-daemon"), "{log}");
    let run = fs::read_to_string(setup.path("systemd-run.log")).unwrap();
    assert!(run.contains("upgrade resume"), "{run}");
    let until = setup.store()["put_off"][next()]["until"].clone();
    let until: jiff::Timestamp = until.as_str().unwrap().parse().unwrap();
    let wait = until.duration_since(jiff::Timestamp::now());
    assert!(wait.as_mins() > 50 && wait.as_mins() <= 60, "{wait:?}");

    // A second time waits twice as long, and `retry` waits no more.
    let mut store = setup.store();
    store["put_off"][next()]["until"] = json!("2026-01-01T00:00:00Z");
    store["put_off"][next()]["op"] = json!("earlier");
    fs::write(setup.path("home/upgrade/state.json"), store.to_string()).unwrap();
    setup.wait("it is put off again", || {
        setup.store()["put_off"][next()]["times"] == 2
    });
    succeeded(&setup.priory(&["upgrade", "retry", &next()]));
    assert_eq!(setup.store()["put_off"], json!({}));
    let unknown = setup.priory(&["upgrade", "retry", "9.9.9"]);
    assert!(!unknown.status.success());
}

/// The helper's check never ends in time, and the daemon does not wait for it.
#[test]
fn a_daemon_works_on_while_a_late_helper_checks() {
    let setup = Setup::new("late-daemon", UPGRADE);
    let helper = setup.path("as-helper");
    let slow = fs::read_to_string(&helper).unwrap();
    let slow = slow.replace(
        "exec ",
        "exec 9> \"$FAKE/home/upgrade/lock\"; flock 9; : > \"$FAKE/slow\"; sleep 70; exec 9>&-; exec ",
    );
    script(&helper, &slow);
    setup.stage(&fake(&next(), "exit 0", ":"));
    setup.serve();
    setup.wait("the helper checks", || setup.path("slow").exists());
    let since = Instant::now();

    setup.wait("the daemon answers", || setup.answers());

    assert!(
        since.elapsed() < Duration::from_secs(60),
        "{:?}",
        since.elapsed()
    );
    assert!(setup.path("home/upgrade/journal.json").exists());
    let record = setup.recorded();
    assert_eq!(record["type"], "factory.upgrade.put_off");
    let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
    assert!(!log.contains("stop "), "{log}");
}

#[test]
fn an_edit_that_waits_for_a_restart_puts_the_upgrade_off() {
    let setup = Setup::new("restart-only", UPGRADE);
    setup.serve();
    setup.wait("the daemon answers", || setup.answers());
    fs::write(
        setup.path("factory/factory.toml"),
        format!("name = \"demo\"\n{UPGRADE}{}", served(port())),
    )
    .unwrap();
    setup.wait("it reloads", || {
        let trace = setup.priory(&["trace"]);
        String::from_utf8_lossy(&trace.stdout).contains("factory.config.reloaded")
    });

    setup.stage(&fake(&next(), "exit 0", ":"));

    let record = setup.recorded();
    assert_eq!(record["type"], "factory.upgrade.put_off");
    let reason = record["data"]["reason"].as_str().unwrap();
    assert!(reason.contains("[serve] changed"), "{reason}");
    assert!(!setup.path("systemd-run.log").exists());
    assert!(setup.answers());
}

#[test]
fn a_daemon_unit_that_never_restarts_is_not_upgraded() {
    let setup = Setup::new("no-restart", UPGRADE);
    fs::write(setup.path("restart"), "no\n").unwrap();
    setup.serve();

    setup.wait("the desk says so", || {
        setup.desk().contains("needs Restart=on-failure")
    });

    assert!(!setup.path("curl.log").exists());
}

#[test]
fn a_service_goes_on_when_the_helper_of_an_unstarted_upgrade_does_not_run() {
    let setup = Setup::new("broken-helper", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |_| {});
    let stopped = setup
        .command("systemctl")
        .args(["--user", "stop", "priory-daemon.service"])
        .status();
    assert!(stopped.unwrap().success());
    script(&setup.path("home/upgrade/helper"), "#!/bin/sh\nexit 1\n");

    let started = setup
        .command("systemctl")
        .args(["--user", "start", "priory-daemon.service"])
        .status();
    assert!(started.unwrap().success());

    setup.wait("the old daemon answers", || setup.answers());
    assert!(!setup.path("home/upgrade/journal.json").exists());
    assert!(!setup.path("systemd-run.log").exists());
}

#[test]
fn a_refused_version_is_tried_again_once_a_person_says_so() {
    let setup = Setup::new("retry", "");
    setup.upgrade(&fake(CURRENT, "exit 0", "exit 1"), |_| {});
    setup.helper();
    setup.assert_back("retry", CURRENT);

    succeeded(&setup.priory(&["upgrade", "retry", CURRENT]));

    assert_eq!(setup.store()["refused"], json!({}));
}

/// The whole way from a verified release to a rollback: the fake answers as the running version.
#[test]
fn a_staged_release_is_rehearsed_swapped_in_and_rolled_back() {
    let setup = Setup::new("staged", UPGRADE);
    setup.stage(&fake(&next(), "exit 0", ":"));
    setup.serve();

    setup.wait("the helper ends", || {
        fs::read_to_string(setup.path("home/upgrade/state.json"))
            .is_ok_and(|store| store.contains(&next()))
            && !setup.path("home/upgrade/journal.json").exists()
    });

    let record = setup.recorded();
    assert_eq!(
        record["type"],
        "factory.upgrade.rolled_back",
        "{}",
        setup.logs()
    );
    assert_eq!(
        record["data"]["reason"],
        format!("the daemon answers as {CURRENT}")
    );
    assert_eq!(
        fs::read(setup.path("exe")).unwrap(),
        fs::read(REAL).unwrap()
    );
    assert!(setup.path(&format!("exe.{CURRENT}")).exists());
    assert!(!setup.path("home/upgrade/rehearsal").exists());
}

#[test]
fn a_release_the_factory_does_not_pass_is_refused_before_anything_stops() {
    let setup = Setup::new("rehearsal", UPGRADE);
    setup.stage(&fake(&next(), "echo broken >&2; exit 1", ":"));
    setup.serve();

    let record = setup.recorded();

    assert_eq!(record["type"], "factory.upgrade.refused");
    assert!(
        record["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("broken"),
        "{record}"
    );
    let log = fs::read_to_string(setup.path("systemctl.log")).unwrap();
    assert!(!log.contains("stop "), "{log}");
}

#[test]
fn a_staged_release_waits_while_it_must_not_upgrade() {
    let hours = "[upgrade.not_during]\nfrom = \"00:00\"\nuntil = \"00:00\"\n";
    let setup = Setup::new("not-during", &format!("{UPGRADE}{hours}"));
    setup.stage(&fake(&next(), "exit 0", ":"));
    setup.serve();
    setup.wait("the daemon answers", || setup.answers());
    std::thread::sleep(Duration::from_secs(12));

    assert!(!setup.path("systemd-run.log").exists());
    assert!(setup.path("home/upgrade/staged.json").exists());
}

#[test]
fn check_refuses_an_upgrade_key_that_cannot_work() {
    for (settings, said) in [
        ("[upgrade]\n", "missing field `channel`"),
        (
            "[upgrade]\nchannel = \"major\"\n",
            "unknown variant `major`",
        ),
        (
            "[upgrade]\nchannel = \"patch\"\nevery = \"5m\"\n",
            "under 10 minutes",
        ),
        (
            "[upgrade]\nchannel = \"patch\"\nnot_during = { from = \"09:00\" }\n",
            "missing field `until`",
        ),
        (
            "[upgrade]\nchannel = \"patch\"\nunits = [\"a.service\", \"a.service\"]\n",
            "names one unit twice",
        ),
        (
            "[upgrade]\nchannel = \"patch\"\n[log]\nbackend = \"postgres\"\n",
            "needs the herdr-bus log",
        ),
    ] {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("upgrade-check");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("factory.toml"),
            format!("name = \"demo\"\n{settings}"),
        )
        .unwrap();
        let output = harness::priory().arg("check").arg(&dir).output().unwrap();
        let text = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            !output.status.success() && text.contains(said),
            "{settings}: {text}"
        );
    }
}
