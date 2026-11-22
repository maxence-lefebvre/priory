//! `priory check --repo PATH`: what an unattended agent needs from a repository.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[allow(dead_code)]
mod conform;

struct Repo {
    root: PathBuf,
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Repo {
    /// A scratch folder holding `repo/`, `factory/` and the fake `gh`'s files under `fake/`.
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("readiness")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        for dir in ["repo", "factory", "fake"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        Self { root }
    }

    fn write(&self, path: &str, content: &str) {
        let file = self.root.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, content).unwrap();
    }

    fn repo(&self) -> String {
        self.root.join("repo").to_str().unwrap().to_owned()
    }

    fn factory(&self) -> String {
        self.root.join("factory").to_str().unwrap().to_owned()
    }

    /// A factory whose `factory.toml` holds `settings`, and which `priory check` accepts.
    fn write_factory(&self, settings: &str) {
        self.write("factory/factory.toml", settings);
        self.write(
            "factory/agents/implementer/agent.md",
            "+++\ndescription = \"Implements.\"\nharness = \"claude-code\"\n+++\nImplement.\n",
        );
        self.write(
            "factory/workflows/implement.toml",
            "start = \"implement\"\n[[steps]]\nid = \"implement\"\ndecides = \"model\"\nagent = \"implementer\"\nprompt = \"Implement.\"\neffect = \"reversible\"\nnext = { passed = \"done\" }\n",
        );
        self.write(
            "factory/runners/xps13.toml",
            "platform = { os = \"linux\" }\n",
        );
    }

    /// The fake `gh` resolves the repository to `owner/repo` with default branch `main`.
    fn on_github(&self) {
        self.write(
            "fake/repo",
            r#"{"full_name":"owner/repo","default_branch":"main"}"#,
        );
    }

    fn priory(&self, args: &[&str]) -> Outcome {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake");
        let path = std::env::join_paths(
            std::iter::once(fake).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("PATH", path)
            .env("FAKE_DIR", self.root.join("fake"))
            .env("HERDR_APP_HOME", self.root.join("no-herdr"))
            .output()
            .unwrap();
        Outcome {
            code: output.status.code().unwrap(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    fn json(&self, args: &[&str]) -> Value {
        let outcome = self.priory(args);
        assert_eq!(outcome.code, 0, "{}", outcome.stderr);
        serde_json::from_str(&outcome.stdout).unwrap()
    }

    /// Every file under `repo/`, by path, with its bytes.
    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, files);
                } else {
                    files.insert(path.clone(), fs::read(&path).unwrap());
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(&self.root.join("repo"), &mut files);
        files
    }

    fn gh_calls(&self) -> Vec<Vec<String>> {
        fs::read_to_string(self.root.join("fake/gh.log"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn statuses(report: &Value) -> Vec<(String, String)> {
    report["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|criterion| {
            (
                criterion["id"].as_str().unwrap().to_owned(),
                criterion["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn criterion<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .find(|criterion| criterion["id"] == id)
        .unwrap()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(id, status)| (id.to_string(), status.to_string()))
        .collect()
}

const CI: &str = "on:\n  pull_request:\njobs:\n  test:\n    steps:\n      - run: cargo clippy -- -D warnings\n      - run: cargo test\n";

#[test]
fn an_empty_repository_misses_everything_and_prints_each_fix() {
    let repo = Repo::new("empty");
    let outcome = repo.priory(&["check", "--repo", &repo.repo()]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let lines: Vec<&str> = outcome.stdout.lines().collect();
    assert_eq!(lines[0], "readiness: 0 of 7 met, 5 missing, 2 unknown");
    for id in ["agents-file", "test-command", "linter", "setup", "ci"] {
        assert!(
            outcome
                .stdout
                .lines()
                .any(|line| line.starts_with(&format!("missing\t{id}\t"))),
            "{id} in {}",
            outcome.stdout
        );
    }
    assert!(
        outcome
            .stdout
            .lines()
            .any(|line| line.starts_with("unknown\tgate\tno factory named")),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .lines()
            .any(|line| line.starts_with("unknown\trequired-checks\t")),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("\tfix: Add AGENTS.md at the repository's root"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_ready_repository_meets_every_criterion() {
    let repo = Repo::new("ready");
    repo.write("repo/AGENTS.md", "# Agents\n");
    repo.write("repo/Cargo.toml", "[package]\nname = \"demo\"\n");
    repo.write(
        "repo/rust-toolchain.toml",
        "[toolchain]\nchannel = \"1.90\"\n",
    );
    repo.write("repo/.github/workflows/ci.yml", CI);
    repo.write_factory("name = \"demo\"\ngate = \"cargo test\"\n");
    repo.on_github();
    repo.write(
        "fake/rules",
        r#"[{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"test"}]}}]"#,
    );
    let report = repo.json(&["check", &repo.factory(), "--repo", &repo.repo(), "--json"]);
    assert_eq!(
        statuses(&report),
        pairs(&[
            ("agents-file", "met"),
            ("test-command", "met"),
            ("linter", "met"),
            ("setup", "met"),
            ("ci", "met"),
            ("gate", "met"),
            ("required-checks", "met"),
        ])
    );
    assert_eq!(report["met"], 7);
    assert_eq!(report["missing"], 0);
    assert_eq!(report["unknown"], 0);
    assert_eq!(criterion(&report, "test-command")["detail"], "cargo test");
    assert_eq!(
        criterion(&report, "gate")["detail"],
        "gate = \"cargo test\""
    );
    assert_eq!(
        criterion(&report, "required-checks")["detail"],
        "main requires test"
    );
    assert_eq!(criterion(&report, "required-checks")["by"], "person");
    assert_eq!(criterion(&report, "agents-file")["by"], "pull-request");
    assert_eq!(criterion(&report, "gate")["by"], "factory");
    conform::matches("readiness.json", &report);
}

#[test]
fn the_check_only_reads() {
    let repo = Repo::new("read-only");
    repo.write("repo/CLAUDE.md", "# Claude\n");
    repo.write("repo/package.json", r#"{"scripts":{"test":"vitest"}}"#);
    repo.write("repo/.github/workflows/ci.yml", CI);
    repo.on_github();
    let before = repo.snapshot();
    let outcome = repo.priory(&["check", "--repo", &repo.repo()]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(repo.snapshot(), before);
    let calls = repo.gh_calls();
    assert!(!calls.is_empty());
    for call in calls {
        assert!(
            call.len() == 2 && call[0] == "api" && call[1].starts_with("repos/"),
            "{call:?} may write"
        );
    }
}

#[test]
fn classic_branch_protection_counts_as_required_checks() {
    let repo = Repo::new("protection");
    repo.on_github();
    repo.write(
        "fake/branch",
        r#"{"name":"main","protection":{"enabled":true,"required_status_checks":{"contexts":["ci","lint"],"checks":[{"context":"ci"},{"context":"lint"}]}}}"#,
    );
    let report = repo.json(&["check", "--repo", &repo.repo(), "--json"]);
    let required = criterion(&report, "required-checks");
    assert_eq!(required["status"], "met");
    assert_eq!(required["detail"], "main requires ci, lint");
}

#[test]
fn a_branch_without_required_checks_is_missing_them() {
    let repo = Repo::new("unprotected");
    repo.on_github();
    let report = repo.json(&["check", "--repo", &repo.repo(), "--json"]);
    let required = criterion(&report, "required-checks");
    assert_eq!(required["status"], "missing");
    assert_eq!(required["detail"], "main requires no check");
    assert!(
        required["fix"]
            .as_str()
            .unwrap()
            .contains("ruleset on main"),
        "{required}"
    );
}

#[test]
fn the_gate_fix_names_the_test_command_found() {
    let repo = Repo::new("gate");
    repo.write(
        "repo/Makefile",
        "build:\n\tcc main.c\n\ntest: build\n\t./run-tests\n",
    );
    repo.write_factory("name = \"demo\"\n");
    let report = repo.json(&["check", &repo.factory(), "--repo", &repo.repo(), "--json"]);
    assert_eq!(criterion(&report, "test-command")["detail"], "make test");
    let gate = criterion(&report, "gate");
    assert_eq!(gate["status"], "missing");
    assert!(
        gate["fix"]
            .as_str()
            .unwrap()
            .contains("gate = \"make test\""),
        "{gate}"
    );
}

#[test]
fn npm_s_placeholder_test_script_is_no_test_command() {
    let repo = Repo::new("npm-placeholder");
    repo.write(
        "repo/package.json",
        r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
    );
    let report = repo.json(&["check", "--repo", &repo.repo(), "--json"]);
    assert_eq!(criterion(&report, "test-command")["status"], "missing");
}

#[test]
fn a_workflow_without_pull_request_is_no_ci() {
    let repo = Repo::new("push-only");
    repo.write(
        "repo/.github/workflows/release.yml",
        "on:\n  push:\n    tags: ['v*']\n",
    );
    let report = repo.json(&["check", "--repo", &repo.repo(), "--json"]);
    assert_eq!(criterion(&report, "ci")["status"], "missing");
}

#[test]
fn a_missing_folder_is_an_error_and_explain_does_not_mix_with_repo() {
    let repo = Repo::new("errors");
    let gone = format!("{}/nowhere", repo.repo());
    let outcome = repo.priory(&["check", "--repo", &gone]);
    assert_eq!(outcome.code, 1);
    assert!(
        outcome.stderr.contains("is not a folder"),
        "{}",
        outcome.stderr
    );
    assert_eq!(
        repo.priory(&["check", "--repo", &repo.repo(), "--explain"])
            .code,
        2
    );
    assert_eq!(repo.priory(&["check", "--repo"]).code, 2);
}

#[test]
fn the_gate_fix_puts_the_key_before_the_first_table() {
    let repo = Repo::new("gate-top");
    repo.write("repo/Cargo.toml", "[package]\nname = \"demo\"\n");
    repo.write_factory("name = \"demo\"\n\n[serve]\nlisten = \"127.0.0.1:7340\"\n");
    let report = repo.json(&["check", &repo.factory(), "--repo", &repo.repo(), "--json"]);
    let fix = criterion(&report, "gate")["fix"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(fix.contains("gate = \"cargo test\""), "{fix}");
    assert!(fix.contains("before its first table"), "{fix}");
}

#[test]
fn the_gate_fix_names_no_command_when_the_repository_has_none() {
    let repo = Repo::new("gate-none");
    repo.write("repo/README.md", "# Demo\n");
    repo.write_factory("name = \"demo\"\n");
    let report = repo.json(&["check", &repo.factory(), "--repo", &repo.repo(), "--json"]);
    assert_eq!(criterion(&report, "test-command")["status"], "missing");
    let fix = criterion(&report, "gate")["fix"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!fix.contains("cargo test"), "{fix}");
    assert!(fix.contains("test-command"), "{fix}");
}
