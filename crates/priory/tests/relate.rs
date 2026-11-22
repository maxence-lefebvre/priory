mod tmux_dir;

use herdr_bus::{Bus, Output};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output as Exit};
use std::time::{Duration, Instant};
use tmux_dir::TmuxDir;

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

/// A factory, a bus file and a Priory state file under the test's own directory. Its tasks come
/// from held CLI requests, so no run starts.
struct Setup {
    dir: PathBuf,
    daemon: Option<Child>,
    tmux: TmuxDir,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("relate")
            .join(name);
        priory_runtime::runtime::remove_sealed(&dir);
        let setup = Setup {
            tmux: TmuxDir::new(&dir),
            dir,
            daemon: None,
        };
        setup.write("factory/factory.toml", "name = \"demo\"\n");
        setup.write("factory/agents/implementer/agent.md", AGENT);
        setup.write("factory/workflows/implement.toml", WORKFLOW);
        Bus::open(&setup.dir.join("bus.db"))
            .unwrap()
            .declare(&Output::new("factory.intake"))
            .unwrap();
        succeeded(&setup.priory(&["tasks"]));
        setup
    }

    fn write(&self, path: &str, content: &str) {
        let file = self.dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_priory"));
        command
            .args(args)
            .env_remove("TMUX")
            .env("USER", "ana")
            .env("TMUX_TMPDIR", self.tmux.path())
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"));
        command
    }

    fn priory(&self, args: &[&str]) -> Exit {
        self.command(args).output().unwrap()
    }

    fn start(&mut self) {
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        let factory = self.factory();
        self.daemon = Some(
            self.command(&["daemon", &factory])
                .stderr(log)
                .spawn()
                .unwrap(),
        );
    }

    /// Waits until `priory tasks --json` lists `count` tasks.
    fn until_tasks(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let tasks: Value = serde_json::from_str(&self.stdout(&["tasks", "--json"])).unwrap();
            if tasks["tasks"].as_array().unwrap().len() == count {
                return;
            }
            assert!(Instant::now() < deadline, "timed out on {count} tasks");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Requests a held task with key `key` and request `text`, plus any `extra` flags.
    fn request(&self, key: &str, text: &str, extra: &[&str]) {
        let factory = self.factory();
        let mut args = vec![
            "request",
            &factory,
            "--workflow",
            "implement",
            "--key",
            key,
            "--hold",
            "related only",
            "--repo",
            "owner/repo",
        ];
        args.extend(extra);
        args.push(text);
        succeeded(&self.priory(&args));
    }

    /// Routes the pending requests to tasks, as `priory daemon --once` does.
    fn once(&self) {
        succeeded(&self.priory(&["daemon", &self.factory(), "--once"]));
    }

    fn stdout(&self, args: &[&str]) -> String {
        let output = self.priory(args);
        succeeded(&output);
        String::from_utf8(output.stdout).unwrap()
    }

    /// Task `task`'s relations, as `priory task TASK --json` lists them, without their times.
    fn relations(&self, task: &str) -> Value {
        let task: Value = serde_json::from_str(&self.stdout(&["task", task, "--json"])).unwrap();
        let mut relations = task["relations"].clone();
        for relation in relations.as_array_mut().expect("relations is a list") {
            relation.as_object_mut().unwrap().remove("created");
        }
        relations
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
    }
}

fn succeeded(output: &Exit) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn priory_relate_records_a_relation_on_both_tasks_and_remove_removes_it() {
    let setup = Setup::new("relate");
    setup.request("login", "Fix the login page", &[]);
    setup.request("login-ui", "Restyle the login page", &[]);
    setup.once();

    assert_eq!(
        setup.stdout(&["relate", "2", "builds-on", "1"]),
        "task 2 builds-on task 1\n"
    );

    let relation = json!([{ "task": 2, "kind": "builds-on", "other": 1, "by": "ana" }]);
    assert_eq!(setup.relations("2"), relation);
    assert_eq!(setup.relations("1"), relation);

    assert_eq!(
        setup.stdout(&["relate", "2", "builds-on", "1", "--remove"]),
        "task 2 no longer builds-on task 1\n"
    );
    assert_eq!(setup.relations("2"), json!([]));
    assert_eq!(setup.relations("1"), json!([]));
}

#[test]
fn priory_relate_refuses_an_unknown_kind_a_missing_task_and_the_task_itself() {
    let setup = Setup::new("relate-refused");
    setup.request("login", "Fix the login page", &[]);
    setup.once();

    let refused = |args: &[&str]| {
        let output = setup.priory(args);
        assert!(!output.status.success());
        String::from_utf8(output.stderr).unwrap()
    };
    assert!(
        refused(&["relate", "1", "blocks", "2"]).contains("builds-on, fixes, takes-up, relates")
    );
    assert_eq!(
        refused(&["relate", "1", "fixes", "9"]),
        "priory relate: no task 9\n"
    );
    assert_eq!(
        refused(&["relate", "1", "fixes", "1"]),
        "priory relate: a task does not relate to itself\n"
    );
}

#[test]
fn a_request_with_relates_records_the_new_tasks_relations() {
    let setup = Setup::new("request-relates");
    setup.request("login", "Fix the login page", &[]);
    setup.request("docs", "Document the login page", &[]);
    setup.once();

    setup.request(
        "login-ui",
        "Restyle the login page",
        &["--relates", "builds-on:1", "--relates", "relates:2"],
    );
    setup.once();

    assert_eq!(
        setup.relations("3"),
        json!([
            { "task": 3, "kind": "builds-on", "other": 1, "by": "request" },
            { "task": 3, "kind": "relates", "other": 2, "by": "request" },
        ])
    );
}

#[test]
fn a_request_or_add_that_relates_to_a_missing_task_is_refused_and_publishes_nothing() {
    let setup = Setup::new("request-relates-missing");
    setup.request("login", "Fix the login page", &[]);
    setup.once();
    let factory = setup.factory();

    let requested = setup.priory(&[
        "request",
        &factory,
        "--workflow",
        "implement",
        "--key",
        "login-ui",
        "--relates",
        "builds-on:9",
        "Restyle the login page",
    ]);
    let added = setup.priory(&["add", &factory, "--relates", "fixes:9", "Fix it"]);
    setup.once();

    assert!(!requested.status.success());
    assert_eq!(
        String::from_utf8(requested.stderr).unwrap(),
        "priory request: no task 9\n"
    );
    assert!(!added.status.success());
    assert_eq!(
        String::from_utf8(added.stderr).unwrap(),
        "priory add: no task 9\n"
    );
    let tasks: Value = serde_json::from_str(&setup.stdout(&["tasks", "--json"])).unwrap();
    assert_eq!(tasks["tasks"].as_array().unwrap().len(), 1, "{tasks}");
}

#[test]
fn a_new_task_on_the_key_of_a_dropped_one_takes_it_up() {
    let mut setup = Setup::new("takes-up");
    setup.request("login", "Fix the login page", &[]);
    setup.start();
    setup.until_tasks(1);
    succeeded(&setup.priory(&["cancel", "1"]));

    setup.request("login", "Fix the login page, take two", &[]);
    setup.until_tasks(2);

    assert_eq!(
        setup.relations("2"),
        json!([{ "task": 2, "kind": "takes-up", "other": 1, "by": "priory" }])
    );
}

/// Tasks 1 to 5: 2 builds on 1, 3 fixes 2, 4 relates to 1, and 5 relates to none.
fn graph(name: &str) -> Setup {
    let setup = Setup::new(name);
    for key in ["login", "login-ui", "login-ui-fix", "docs", "build"] {
        setup.request(key, &format!("Work on {key}"), &[]);
    }
    setup.once();
    for relation in [
        ["2", "builds-on", "1"],
        ["3", "fixes", "2"],
        ["4", "relates", "1"],
    ] {
        succeeded(&setup.priory(&[&["relate"][..], &relation].concat()));
    }
    setup
}

#[test]
fn tasks_graph_prints_the_related_tasks_as_an_indented_tree() {
    let setup = graph("graph");

    let tree = "\
1 cli:login held
  2 cli:login-ui held (2 builds-on 1)
    3 cli:login-ui-fix held (3 fixes 2)
  4 cli:docs held (4 relates 1)
";
    assert_eq!(setup.stdout(&["tasks", "--graph", "1"]), tree);
    assert_eq!(
        setup.stdout(&["tasks", "--graph", "3"]).lines().next(),
        Some("3 cli:login-ui-fix held")
    );
    assert_eq!(setup.stdout(&["tasks", "--graph"]), tree);
}

#[test]
fn tasks_graph_mermaid_draws_each_relation_as_a_link() {
    let setup = graph("graph-mermaid");

    assert_eq!(
        setup.stdout(&["tasks", "--graph", "1", "--mermaid"]),
        "\
flowchart TD
    t1[\"1 cli:login held\"]
    t2[\"2 cli:login-ui held\"]
    t3[\"3 cli:login-ui-fix held\"]
    t4[\"4 cli:docs held\"]
    t2 -->|\"builds-on\"| t1
    t3 -->|\"fixes\"| t2
    t4 -->|\"relates\"| t1
"
    );
}
