use herdr_bus::{Bus, Output};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output as Exit};
use std::time::{Duration, Instant};

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

const SUMMARIZER: &str = "+++\ndescription = \"Summarizes requests.\"\nharness = \"claude-code\"\nmodel = \"haiku\"\n+++\n";

/// A factory, a bus file, a Priory state file and home, and the fake `claude`, under the test's own
/// directory. Its tasks come from held CLI requests, so no run starts. Dropping it kills the daemon.
struct Setup {
    dir: PathBuf,
    daemon: Option<Child>,
}

impl Setup {
    fn new(name: &str) -> Self {
        Self::with(name, "name = \"demo\"\n")
    }

    /// A setup whose `factory.toml` is `settings`.
    fn with(name: &str, settings: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("search")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let setup = Setup { dir, daemon: None };
        let files = [
            ("factory/factory.toml", settings),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/agents/summarizer/agent.md", SUMMARIZER),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            setup.write(path, content);
        }
        fs::create_dir_all(setup.dir.join("fake")).unwrap();
        fs::create_dir_all(setup.dir.join("tmux")).unwrap();
        Bus::open(&setup.dir.join("bus.db"))
            .unwrap()
            .declare(&Output::new("factory.intake"))
            .unwrap();
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

    fn priory(&self, args: &[&str]) -> Exit {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake");
        let path = std::env::join_paths(
            std::iter::once(fake).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env_remove("TMUX")
            .env("PATH", path)
            .env("TMUX_TMPDIR", self.dir.join("tmux"))
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("FAKE_DIR", self.dir.join("fake"))
            .output()
            .unwrap()
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
            "searched only",
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

    fn start(&mut self) {
        let log = fs::File::create(self.dir.join("daemon.log")).unwrap();
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake");
        let path = std::env::join_paths(
            std::iter::once(fake).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        self.daemon = Some(
            Command::new(env!("CARGO_BIN_EXE_priory"))
                .args(["daemon", &self.factory()])
                .env_remove("TMUX")
                .env("PATH", path)
                .env("TMUX_TMPDIR", self.dir.join("tmux"))
                .env("HERDR_BUS_DB", self.dir.join("bus.db"))
                .env("PRIORY_DB", self.dir.join("priory.db"))
                .env("PRIORY_HOME", self.dir.join("home"))
                .env("FAKE_DIR", self.dir.join("fake"))
                .stderr(log)
                .spawn()
                .unwrap(),
        );
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default()
    }

    /// The rows `priory search` prints for `args`, each split on tabs.
    fn found(&self, args: &[&str]) -> Vec<Vec<String>> {
        let output = self.priory(&[&["search"], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split('\t').map(str::to_owned).collect())
            .collect()
    }

    /// The trace records of type `kind`.
    fn records(&self, kind: &str) -> Vec<serde_json::Value> {
        let output = self.priory(&["trace"]);
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .filter(|record| record["type"] == kind)
            .collect()
    }

    fn tldr(&self, task: &str, text: &str) -> Exit {
        self.priory(&["tldr", task, text])
    }

    /// The task ids `priory search` prints for `args`, best first.
    fn search(&self, args: &[&str]) -> Vec<String> {
        let output = self.priory(&[&["search"], args].concat());
        succeeded(&output);
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .skip(1)
            .map(|line| line.split('\t').next().unwrap().to_owned())
            .collect()
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

fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(Instant::now() < deadline, "timed out on {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn keyword_search_finds_a_task_by_a_word_in_its_tldr_and_ranks_the_better_match_first() {
    let setup = Setup::new("keywords");
    setup.request("login", "Fix the login page", &[]);
    setup.request("build", "Speed up the build", &[]);
    setup.request("docs", "Tidy the docs", &[]);
    setup.once();

    succeeded(&setup.tldr(
        "1",
        "Make the login page load faster, and touch the done gate once.",
    ));
    succeeded(&setup.tldr(
        "2",
        "Rework the done gate: the gate waits for checks, and the gate ends on a close.",
    ));

    assert_eq!(setup.search(&["done gate"]), ["2", "1"]);
}

#[test]
fn a_tldr_over_the_word_limit_is_refused_with_its_count() {
    let setup = Setup::new("tldr-limit");
    setup.request("login", "Fix the login page", &[]);
    setup.once();

    let long = vec!["word"; 181].join(" ");
    let refused = setup.tldr("1", &long);

    assert!(!refused.status.success());
    let error = String::from_utf8_lossy(&refused.stderr);
    assert!(
        error.contains("181 words") && error.contains("180"),
        "{error}"
    );
    assert!(setup.search(&["word"]).is_empty());
    succeeded(&setup.tldr("1", &vec!["word"; 180].join(" ")));
    assert_eq!(setup.search(&["word"]), ["1"]);
}

const SUMMARIZED: &str = "name = \"demo\"\n[search]\ntldr = \"summarizer\"\n";

#[test]
fn a_new_task_gets_its_tldr_from_the_summarizer_agent_and_its_cost_is_reported() {
    let mut setup = Setup::with("summarized", SUMMARIZED);
    setup.write(
        "fake/handler",
        "Speeds up the nightly build by caching crates.\n",
    );
    setup.request(
        "build",
        "The nightly build takes an hour. Make it faster.",
        &[],
    );

    setup.start();

    until("the summarizer's TL;DR", || {
        setup.found(&["caching"]).first().map(|row| row[4].as_str())
            == Some("Speeds up the nightly build by caching crates.")
    });
    let asked = fs::read_to_string(setup.dir.join("fake/handler.log")).unwrap();
    assert!(
        asked.contains("The nightly build takes an hour. Make it faster."),
        "{asked}"
    );
    let args = fs::read_to_string(setup.dir.join("fake/handler-args.log")).unwrap();
    assert!(args.contains("haiku"), "{args}");
    until("the summarizer's record", || {
        !setup.records("factory.summarizer.called").is_empty()
    });
    let data = &setup.records("factory.summarizer.called")[0]["data"];
    assert_eq!([&data["task"], &data["model"]], ["1", "claude-fake-haiku"]);
    setup.write(
        "factory/prices.toml",
        "[claude-fake-haiku]\ninput = 1.0\noutput = 5.0\ncache_read = 0.1\ncache_write = 1.25\n",
    );
    let report = setup.priory(&[
        "report",
        &setup.factory(),
        "--workflow",
        "implement",
        "--cost",
    ]);
    let report = String::from_utf8(report.stdout).unwrap();
    assert!(
        report.contains("1\tsummarizer\tclaude-fake-haiku\t300\t20\t0\t40\t$0.000450"),
        "{report}"
    );
}

#[test]
fn a_failing_summarizer_leaves_the_requests_title_and_opening_text() {
    let mut setup = Setup::with("summarizer-fails", SUMMARIZED);
    setup.write("fake/handler-fail", "");
    setup.request("build", "The nightly build takes an hour.", &[]);

    setup.start();

    until("the summarizer's record", || {
        !setup.records("factory.summarizer.called").is_empty()
    });
    let found = setup.found(&["nightly"]);
    assert_eq!(found.len(), 1, "{}", setup.log());
    assert_eq!(found[0][4], "The nightly build takes an hour.");
}

#[test]
fn a_request_that_carries_a_tldr_skips_the_summarizer() {
    let mut setup = Setup::with("request-tldr", SUMMARIZED);
    setup.request(
        "build",
        "The nightly build takes an hour.",
        &["--tldr", "Cache crates in CI."],
    );
    setup.request("docs", "Tidy the docs.", &[]);

    setup.start();

    until("the second task's summary", || {
        fs::read_to_string(setup.dir.join("fake/handler.log")).is_ok_and(|log| log.contains("Tidy"))
    });
    std::thread::sleep(Duration::from_millis(500));
    let asked = fs::read_to_string(setup.dir.join("fake/handler.log")).unwrap();
    assert!(!asked.contains("nightly"), "{asked}");
    assert_eq!(setup.found(&["crates"])[0][4], "Cache crates in CI.");
}

/// A fake embedder: one dimension per topic, cats, builds and gates, set when the text names it.
const EMBEDDER: &str = r#"#!/bin/sh
text=$(tr 'A-Z' 'a-z')
printf '%s\n' "$text" >> "$0.log"
has() { if printf '%s' "$text" | grep -Eq "$1"; then echo 1; else echo 0; fi; }
printf '[%s, %s, %s, 0.01]\n' "$(has 'cat|kitten|feline')" "$(has 'build|compile|cach')" "$(has 'gate|check')"
"#;

impl Setup {
    /// A setup whose factory embeds with the fake embedder at `fake/<name>`.
    fn embedding(test: &str, name: &str) -> Self {
        let setup = Self::new(test);
        setup.embed_with(name, EMBEDDER);
        setup
    }

    /// Writes the embedder script `fake/<name>` and makes the factory use it.
    fn embed_with(&self, name: &str, script: &str) {
        let path = self.dir.join("fake").join(name);
        self.write(&format!("fake/{name}"), script);
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        self.write(
            "factory/factory.toml",
            &format!(
                "name = \"demo\"\n[search]\nembeddings = true\ncommand = \"{}\"\n",
                path.display()
            ),
        );
    }

    /// The texts the fake embedder `fake/<name>` read, one per line.
    fn embedded(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join(format!("fake/{name}.log"))).unwrap_or_default()
    }

    fn stop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            daemon.kill().unwrap();
            daemon.wait().unwrap();
        }
    }
}

#[test]
fn a_query_that_shares_no_word_with_a_task_finds_it_by_its_vector() {
    let mut setup = Setup::embedding("vector", "embed");
    setup.request(
        "kitten",
        "Feed the kitten.",
        &["--tldr", "Feed the kitten twice a day."],
    );
    setup.request(
        "build",
        "Speed up CI.",
        &["--tldr", "Speed up the compile step."],
    );

    setup.start();

    until("the vector match", || setup.search(&["feline"]) == ["1"]);
}

#[test]
fn a_task_matched_by_keyword_and_vector_ranks_above_one_matched_either_way() {
    let mut setup = Setup::embedding("merged", "embed");
    setup.request("both", "Feed the kitten.", &["--tldr", "Feed the kitten."]);
    setup.request("vector", "Groom the pets.", &["--tldr", "Brush the cat."]);
    setup.request(
        "keyword",
        "Do it before the kitten wakes.",
        &["--tldr", "Speed up the compile step."],
    );

    setup.start();

    until("all three tasks", || setup.search(&["kitten"]).len() == 3);
    assert_eq!(setup.search(&["kitten"])[0], "1");
}

#[test]
fn changing_the_embedder_command_embeds_every_task_again() {
    let mut setup = Setup::embedding("re-embed", "embed");
    setup.request(
        "kitten",
        "Feed the kitten.",
        &["--tldr", "Feed the kitten twice a day."],
    );
    setup.start();
    until("the first embedding", || setup.search(&["feline"]) == ["1"]);
    setup.stop();

    setup.embed_with("other", EMBEDDER);
    setup.start();

    until("the second embedder's pass", || {
        setup
            .embedded("other")
            .contains("feed the kitten twice a day.")
    });
    assert_eq!(setup.search(&["feline"]), ["1"]);
}

#[test]
fn a_failing_embedder_leaves_keyword_search_and_search_says_why() {
    let mut setup = Setup::embedding("embed-fails", "embed");
    setup.embed_with("broken", "#!/bin/sh\necho 'no GPU here' >&2\nexit 3\n");
    setup.request(
        "kitten",
        "Feed the kitten.",
        &["--tldr", "Feed the kitten twice a day."],
    );

    setup.start();

    until("the daemon's log", || setup.log().contains("no GPU here"));
    let output = setup.priory(&["search", "kitten"]);
    let why = String::from_utf8_lossy(&output.stderr);
    assert!(
        why.contains("embeddings are unavailable") && why.contains("no GPU here"),
        "{why}"
    );
    assert_eq!(setup.search(&["kitten"]), ["1"]);
    assert_eq!(
        setup.log().matches("no GPU here").count(),
        1,
        "{}",
        setup.log()
    );
}

#[test]
fn a_changed_tldr_no_longer_matches_by_its_old_vector() {
    let mut setup = Setup::embedding("stale-vector", "embed");
    setup.request(
        "kitten",
        "Feed the kitten.",
        &["--tldr", "Feed the kitten twice a day."],
    );
    setup.start();
    until("the first embedding", || setup.search(&["feline"]) == ["1"]);
    setup.stop();

    succeeded(&setup.tldr("1", "Speed up the compile step."));

    assert!(setup.search(&["feline"]).is_empty());
}
