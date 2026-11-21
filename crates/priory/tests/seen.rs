use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

struct Setup {
    dir: PathBuf,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("seen")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Setup { dir }
    }

    fn priory(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .output()
            .unwrap()
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = self.priory(args);
        assert!(
            output.status.success(),
            "priory {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn post(&self, line: &str) {
        self.stdout(&["post", "systemd", line]);
    }

    /// What `since_you_looked` lists for `args` added to `desk --json`.
    fn looked(&self, args: &[&str]) -> Vec<String> {
        let desk: Value =
            serde_json::from_str(&self.stdout(&[&["desk", "--json"], args].concat())).unwrap();
        desk["since_you_looked"]
            .as_array()
            .expect("the desk lists since_you_looked")
            .iter()
            .map(|event| event["what"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// Inbox times have whole seconds, so a later event must come in a later second.
fn next_second() {
    std::thread::sleep(Duration::from_millis(1100));
}

#[test]
fn without_a_visit_since_you_looked_lists_the_latest_50_events_newest_first() {
    let setup = Setup::new("cap");
    for n in 1..=55 {
        setup.post(&format!("line {n}"));
    }

    let looked = setup.looked(&[]);

    assert_eq!(looked.len(), 50);
    assert_eq!(looked[0], "systemd: line 55");
    assert_eq!(looked[49], "systemd: line 6");
}

#[test]
fn after_seen_since_you_looked_lists_only_later_events() {
    let setup = Setup::new("later");
    setup.post("before");
    next_second();
    setup.stdout(&["seen"]);
    setup.post("after");

    assert_eq!(setup.looked(&[]), ["systemd: after"]);
}

#[test]
fn an_event_in_the_second_of_the_visit_still_shows() {
    let setup = Setup::new("boundary");
    let now = jiff::Timestamp::now();
    let at = format!("{}.999Z", now.strftime("%Y-%m-%dT%H:%M:%S"));
    setup.stdout(&["seen", "--at", &at]);
    setup.post("same second");

    assert!(
        setup
            .looked(&[])
            .contains(&"systemd: same second".to_owned())
    );
}

#[test]
fn two_persons_keep_separate_visits() {
    let setup = Setup::new("persons");
    setup.post("old");
    next_second();
    let middle = jiff::Timestamp::now().to_string();
    next_second();
    setup.post("new");
    next_second();
    setup.stdout(&["seen", "--person", "Ana Li"]);
    setup.stdout(&["seen", "--person", "bob", "--at", &middle]);

    assert_eq!(setup.looked(&["--person", "Ana Li"]), Vec::<String>::new());
    assert_eq!(setup.looked(&["--person", "bob"]), ["systemd: new"]);
    assert_eq!(setup.looked(&[]), ["systemd: new", "systemd: old"]);
}

#[test]
fn the_text_desk_lists_since_you_looked_after_its_items() {
    let setup = Setup::new("text");
    setup.post("before");
    next_second();
    setup.stdout(&["seen"]);
    setup.post("after");

    let text = setup.stdout(&["desk"]);

    let (_, since) = text
        .split_once("since you looked")
        .unwrap_or_else(|| panic!("no since you looked section:\n{text}"));
    assert!(since.contains("systemd: after"), "{text}");
    assert!(!since.contains("systemd: before"), "{text}");
}
