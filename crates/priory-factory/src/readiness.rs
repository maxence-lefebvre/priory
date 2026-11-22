//! `priory check --repo PATH`: what an unattended agent needs from a repository. It only reads.

use std::fs;
use std::path::Path;

use priory_runtime::protection::Required;
use schemars::JsonSchema;
use serde::Serialize;

/// What `priory check --repo PATH --json` prints.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Report {
    /// The repository folder as given.
    pub repo: String,
    /// How many criteria are met.
    pub met: usize,
    /// How many criteria are missing.
    pub missing: usize,
    /// How many criteria could not be checked.
    pub unknown: usize,
    /// Every criterion, in a fixed order.
    pub criteria: Vec<Criterion>,
}

/// One thing an unattended agent needs from the repository.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Criterion {
    /// `agents-file`, `test-command`, `linter`, `setup`, `ci`, `gate` or `required-checks`.
    pub id: &'static str,
    pub status: Status,
    /// What was found when met, what is missing, or why it could not be checked.
    pub detail: String,
    /// How to meet the criterion.
    pub fix: String,
    /// Who can make the fix.
    pub by: FixedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Met,
    Missing,
    Unknown,
}

/// Who can make a criterion's fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FixedBy {
    /// A pull request on the repository, which an agent can open.
    PullRequest,
    /// A change to the factory's own files.
    Factory,
    /// A person, in the repository's settings.
    Person,
}

/// The factory's `gate`, as far as `priory check` read a factory.
#[derive(Debug, Clone, Copy)]
pub enum Gate<'a> {
    NoFactory,
    Unset,
    Set(&'a str),
}

/// Scores `repo` on every criterion. `required` is what `gh` read of the default branch.
pub fn assess(repo: &Path, gate: Gate, required: Result<Required, String>) -> Report {
    let workflows = workflows(repo);
    let test = test_command(repo);
    let criteria = vec![
        agents_file(repo),
        criterion(
            "test-command",
            test.clone(),
            "no Cargo.toml, go.mod, package.json test script, Makefile test target or other test runner found",
            "Give the tests one command, such as a `test` script in package.json or a `test` target in a Makefile.",
            FixedBy::PullRequest,
        ),
        criterion(
            "linter",
            linter(repo, &workflows),
            "no linter config file found, and no CI workflow runs a linter",
            "Add a linter's config file, or run a linter in CI, such as `cargo clippy` or `eslint`.",
            FixedBy::PullRequest,
        ),
        criterion(
            "setup",
            first_file(repo, SETUP_FILES),
            "no file pins the toolchain or sets up the machine",
            "Pin the toolchain in a file, such as rust-toolchain.toml, .tool-versions or .nvmrc, or add a dev container.",
            FixedBy::PullRequest,
        ),
        criterion(
            "ci",
            ci(&workflows),
            "no workflow under .github/workflows runs on pull_request",
            "Add a workflow under .github/workflows that runs the tests on `pull_request`.",
            FixedBy::PullRequest,
        ),
        gate_criterion(gate, test.as_deref()),
        required_criterion(required),
    ];
    let count = |status| criteria.iter().filter(|c| c.status == status).count();
    Report {
        repo: repo.display().to_string(),
        met: count(Status::Met),
        missing: count(Status::Missing),
        unknown: count(Status::Unknown),
        criteria,
    }
}

fn criterion(
    id: &'static str,
    found: Option<String>,
    missing: &str,
    fix: &str,
    by: FixedBy,
) -> Criterion {
    let (status, detail) = match found {
        Some(found) => (Status::Met, found),
        None => (Status::Missing, missing.to_owned()),
    };
    Criterion {
        id,
        status,
        detail,
        fix: fix.to_owned(),
        by,
    }
}

fn agents_file(repo: &Path) -> Criterion {
    let found: Vec<&str> = ["AGENTS.md", "CLAUDE.md"]
        .into_iter()
        .filter(|name| repo.join(name).is_file())
        .collect();
    criterion(
        "agents-file",
        (!found.is_empty()).then(|| found.join(", ")),
        "no AGENTS.md or CLAUDE.md at the root",
        "Add AGENTS.md at the repository's root: how to build, test and lint, and the rules a change must follow.",
        FixedBy::PullRequest,
    )
}

/// The one command that runs the tests, from the first manifest that names one.
fn test_command(repo: &Path) -> Option<String> {
    let read = |name: &str| fs::read_to_string(repo.join(name)).ok();
    if repo.join("Cargo.toml").is_file() {
        return Some("cargo test".to_owned());
    }
    if read("package.json").is_some_and(|text| npm_test(&text)) {
        return Some("npm test".to_owned());
    }
    if repo.join("go.mod").is_file() {
        return Some("go test ./...".to_owned());
    }
    let target = |names: &[&str], line: fn(&str) -> bool| {
        names
            .iter()
            .any(|name| read(name).is_some_and(|text| text.lines().any(line)))
    };
    if target(&["GNUmakefile", "makefile", "Makefile"], |line| {
        line.strip_prefix("test").is_some_and(|rest| {
            rest.trim_start().starts_with(':') && !rest.trim_start().starts_with(":=")
        })
    }) {
        return Some("make test".to_owned());
    }
    if target(&["justfile", "Justfile", ".justfile"], |line| {
        line.strip_prefix("test").is_some_and(|rest| {
            rest.starts_with([' ', ':']) && rest.contains(':') && !rest.contains(":=")
        })
    }) {
        return Some("just test".to_owned());
    }
    let tools = [
        ("tox.ini", "tox"),
        ("noxfile.py", "nox"),
        ("pytest.ini", "pytest"),
        ("mix.exs", "mix test"),
        ("pom.xml", "mvn test"),
        ("build.gradle", "gradle test"),
        ("build.gradle.kts", "gradle test"),
    ];
    if let Some((_, command)) = tools.iter().find(|(name, _)| repo.join(name).is_file()) {
        return Some((*command).to_owned());
    }
    read("pyproject.toml")
        .is_some_and(|text| text.contains("[tool.pytest"))
        .then(|| "pytest".to_owned())
}

/// Whether a package.json has a `test` script other than the one `npm init` writes.
fn npm_test(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|package| {
        package["scripts"]["test"].as_str().is_some_and(|script| {
            !script.trim().is_empty() && !script.contains("no test specified")
        })
    })
}

const LINT_FILES: &[&str] = &[
    "rustfmt.toml",
    ".rustfmt.toml",
    "clippy.toml",
    ".clippy.toml",
    "eslint.config.js",
    "eslint.config.mjs",
    "eslint.config.cjs",
    "eslint.config.ts",
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintrc.json",
    ".eslintrc.yml",
    ".eslintrc.yaml",
    "biome.json",
    "biome.jsonc",
    ".golangci.yml",
    ".golangci.yaml",
    ".golangci.toml",
    "ruff.toml",
    ".ruff.toml",
    ".flake8",
    ".pylintrc",
    ".rubocop.yml",
    ".swiftlint.yml",
    ".pre-commit-config.yaml",
];

const LINT_COMMANDS: &[&str] = &[
    "clippy",
    "cargo fmt",
    "eslint",
    "biome",
    "golangci-lint",
    "ruff",
    "flake8",
    "pylint",
    "rubocop",
    "swiftlint",
    "ktlint",
    "pre-commit",
];

const LINT_SECTIONS: &[(&str, &str)] = &[
    ("Cargo.toml", "[lints"),
    ("Cargo.toml", "[workspace.lints"),
    ("pyproject.toml", "[tool.ruff"),
    ("pyproject.toml", "[tool.pylint"),
    ("pyproject.toml", "[tool.flake8"),
];

fn linter(repo: &Path, workflows: &[(String, String)]) -> Option<String> {
    if let Some(file) = first_file(repo, LINT_FILES) {
        return Some(file);
    }
    for (file, section) in LINT_SECTIONS {
        if fs::read_to_string(repo.join(file)).is_ok_and(|text| text.contains(section)) {
            return Some(format!("{section}] in {file}"));
        }
    }
    workflows.iter().find_map(|(name, text)| {
        LINT_COMMANDS
            .iter()
            .find(|command| text.contains(*command))
            .map(|command| format!("{command} in .github/workflows/{name}"))
    })
}

const SETUP_FILES: &[&str] = &[
    "rust-toolchain.toml",
    "rust-toolchain",
    ".tool-versions",
    "mise.toml",
    ".mise.toml",
    ".nvmrc",
    ".node-version",
    ".python-version",
    ".ruby-version",
    ".go-version",
    "flake.nix",
    "shell.nix",
    "devbox.json",
    ".devcontainer.json",
    ".devcontainer/devcontainer.json",
];

/// The first of `names` that is a file in `repo`.
fn first_file(repo: &Path, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|name| repo.join(name).is_file())
        .map(|name| (*name).to_owned())
}

/// Each GitHub Actions workflow file, by name, with its text, in name order.
fn workflows(repo: &Path) -> Vec<(String, String)> {
    let Ok(entries) = fs::read_dir(repo.join(".github/workflows")) else {
        return Vec::new();
    };
    let mut files: Vec<(String, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !(name.ends_with(".yml") || name.ends_with(".yaml")) {
                return None;
            }
            let text = fs::read_to_string(entry.path()).ok()?;
            Some((name, text))
        })
        .collect();
    files.sort();
    files
}

fn ci(workflows: &[(String, String)]) -> Option<String> {
    let names: Vec<&str> = workflows
        .iter()
        .filter(|(_, text)| text.contains("pull_request"))
        .map(|(name, _)| name.as_str())
        .collect();
    (!names.is_empty()).then(|| format!(".github/workflows/{}", names.join(", ")))
}

fn gate_criterion(gate: Gate, test: Option<&str>) -> Criterion {
    let example = test.unwrap_or("cargo test");
    let fix = format!(
        "Set `gate = \"{example}\"` in factory.toml, so a step can require `gate-passed` before it ends."
    );
    let (status, detail) = match gate {
        Gate::Set(command) => (Status::Met, format!("gate = \"{command}\"")),
        Gate::Unset => (Status::Missing, "factory.toml sets no gate".to_owned()),
        Gate::NoFactory => (
            Status::Unknown,
            "no factory named: run priory check DIR --repo PATH to check its gate".to_owned(),
        ),
    };
    Criterion {
        id: "gate",
        status,
        detail,
        fix,
        by: FixedBy::Factory,
    }
}

fn required_criterion(required: Result<Required, String>) -> Criterion {
    let branch = required
        .as_ref()
        .map_or("the default branch", |required| required.branch.as_str())
        .to_owned();
    let (status, detail) = match &required {
        Ok(required) if required.checks.is_empty() => {
            (Status::Missing, format!("{branch} requires no check"))
        }
        Ok(required) => (
            Status::Met,
            format!("{branch} requires {}", required.checks.join(", ")),
        ),
        Err(error) => (Status::Unknown, format!("gh could not read it: {error}")),
    };
    Criterion {
        id: "required-checks",
        status,
        detail,
        fix: format!(
            "In the repository's settings, add a branch ruleset on {branch} that requires the CI checks to pass before a merge."
        ),
        by: FixedBy::Person,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn only_a_real_npm_test_script_counts() {
        assert!(npm_test(r#"{"scripts":{"test":"vitest run"}}"#));
        assert!(!npm_test(
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#
        ));
        assert!(!npm_test(r#"{"scripts":{"build":"tsc"}}"#));
        assert!(!npm_test("not json"));
    }

    proptest! {
        #[test]
        fn a_package_json_parses_or_is_refused_without_panicking(
            text in r#"(\{|\}|"scripts"|"test"|:|,|"vitest"|"no test specified"|null|[0-9]|\s){0,30}|.{0,80}"#
        ) {
            let _ = npm_test(&text);
        }
    }
}
