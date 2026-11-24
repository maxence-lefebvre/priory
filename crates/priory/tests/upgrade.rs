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
      rm -f "$FAKE/$unit.pid"
    done ;;
  start)
    for unit in "$@"; do
      alive "$(pid "$unit")" && continue
      verb_of=daemon
      case "$unit" in *intake*) verb_of=intake ;; esac
      "$FAKE/exe" "$verb_of" "$FAKE/factory" >> "$FAKE/$unit.log" 2>&1 < /dev/null &
      echo $! > "$FAKE/$unit.pid"
    done ;;
  show)
    for unit in "$@"; do :; done
    running=$(pid "$unit")
    alive "$running" || running=0
    case "$*" in
      *MainPID*) echo "$running" ;;
      *ActiveState*) [ "$running" = 0 ] && echo failed || echo active ;;
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
while [ $# -gt 0 ] && [ "$1" != -- ]; do
  case "$1" in
    --setenv=*) export "${1#--setenv=}" ;;
    --scope) scope=yes ;;
  esac
  shift
done
shift
[ $scope = yes ] && exec "$@"
echo "$*" >> "$FAKE/systemd-run.log"
"$@" >> "$FAKE/helper.log" 2>&1 < /dev/null &
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
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
        };
        // The state and the log exist, with one to-do to find again.
        succeeded(&setup.priory(&["daemon", setup.path("factory").to_str().unwrap(), "--once"]));
        succeeded(&setup.priory(&["todo", "add", "kept"]));
        setup
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
            "schema_from": 1, "schema_to": 1, "migrates": false,
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
            if self.priory(&["upgrade", "resume"]).status.success() {
                return;
            }
        }
        panic!("the helper never ended");
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
        let todos = self.priory(&["todo", "list"]);
        assert!(
            String::from_utf8_lossy(&todos.stdout).contains("kept"),
            "{name}: {todos:?}"
        );
        assert!(!self.path("home/upgrade/journal.json").exists(), "{name}");
        let store = fs::read_to_string(self.path("home/upgrade/state.json")).unwrap();
        let store: Value = serde_json::from_str(&store).unwrap();
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

    setup.helper();

    assert_eq!(setup.installed(), new);
    assert_eq!(fs::read_to_string(setup.path("exe.0.0.1")).unwrap(), old());
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
        ("exits", fake(CURRENT, "exit 0", "exit 1"), "did not answer"),
        (
            "hangs",
            fake(CURRENT, "exit 0", "exec sleep 600"),
            "did not answer",
        ),
        ("wrong", fake("9.9.9", "exit 0", ":"), "it says it is"),
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
fn a_changed_download_is_never_installed() {
    let setup = Setup::new("changed", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |journal| {
        journal["sha256"] = json!("0")
    });

    setup.helper();

    setup.assert_back("changed", CURRENT);
}

#[test]
fn a_migration_that_fails_puts_both_databases_back() {
    let setup = Setup::new("migration", "");
    let spoil = "\"$0\" todo list > \"$FAKE/refused\" 2>&1; \
                 echo spoiled > \"$PRIORY_DB\"; echo spoiled > \"$HERDR_BUS_DB\"; exit 1";
    setup.upgrade(&fake(CURRENT, "exit 0", spoil), |journal| {
        journal["migrates"] = json!(true);
        journal["answers_within"] = json!(2);
    });

    setup.helper();

    setup.assert_back("migration", CURRENT);
    let refused = fs::read_to_string(setup.path("refused")).unwrap();
    assert!(
        refused.contains("an upgrade of priory is in progress"),
        "{refused}"
    );
}

#[test]
fn a_migration_s_daemon_does_no_work_until_its_commit() {
    let setup = Setup::new("held", "");
    let new = fake(CURRENT, "exit 0", ":");
    setup.upgrade(&new, |journal| {
        journal["migrates"] = json!(true);
        journal["watch"] = json!(6);
    });
    let mut helper = setup
        .command(REAL)
        .args(["upgrade", "resume"])
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
fn a_grant_that_waits_puts_the_upgrade_off() {
    let setup = Setup::new("put-off", "");
    setup.upgrade(&fake(CURRENT, "exit 0", ":"), |_| {});
    let db = rusqlite::Connection::open(setup.path("priory.db")).unwrap();
    db.execute(
        "INSERT INTO grants (bytes, pid, started, command) VALUES (1, 1, 1, 'build')",
        [],
    )
    .unwrap();
    drop(db);

    setup.helper();

    assert_eq!(setup.installed(), old());
    assert!(setup.path("home/upgrade/staged").exists());
    assert!(!setup.path("priory.db.backup").exists());
    setup.wait("the old daemon answers", || setup.answers());
    assert!(!setup.desk().contains("upgrade"), "{}", setup.desk());
}

/// The helper's unit starts it again after a kill. Before the swap it goes on; after it, it goes back.
#[test]
fn a_helper_killed_in_any_phase_ends_on_one_binary() {
    let good = fake(CURRENT, "exit 0", ":");
    let killed_in_check = fake(
        CURRENT,
        "[ -e \"$FAKE/killed\" ] || { : > \"$FAKE/killed\"; kill -9 $PPID; }; exit 0",
        ":",
    );
    let failing = fake(CURRENT, "exit 1", ":");
    for (name, new, kill, installs) in [
        ("stopping", &good, Some("stop"), true),
        ("installing", &killed_in_check, None, false),
        ("starting", &good, Some("start"), false),
        ("watching", &good, Some("show"), false),
        ("rolling-back", &failing, Some("stop-2"), false),
        ("restarting", &failing, Some("start"), false),
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
        } else {
            setup.assert_back(name, CURRENT);
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
            journal["migrates"] = json!(migrates);
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
            journal["phase"] = json!("installing")
        });
        script(&setup.path("exe.0.0.1"), &old());

        let refused = setup.priory(&[verb, setup.path("factory").to_str().unwrap()]);

        assert!(!refused.status.success(), "{verb}");
        let said = String::from_utf8_lossy(&refused.stderr).into_owned();
        assert!(said.contains("was interrupted"), "{verb}: {said}");
        setup.wait("the helper ends", || {
            !setup.path("home/upgrade/journal.json").exists()
        });
        setup.assert_back(verb, CURRENT);
    }
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
fn a_staged_release_waits_inside_its_window() {
    let window = "[upgrade.window]\nfrom = \"00:00\"\nuntil = \"00:00\"\n";
    let setup = Setup::new("window", &format!("{UPGRADE}{window}"));
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
            "[upgrade]\nchannel = \"patch\"\nwindow = { from = \"09:00\" }\n",
            "missing field `until`",
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
