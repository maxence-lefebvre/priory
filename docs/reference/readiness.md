# `priory check --repo`

`priory check [DIR] --repo PATH [--json]` scores what an unattended agent needs from the repository
checked out at `PATH`. It prints each criterion as met, missing or unknown, with how to fix it. It
only reads: it writes no file, and its only `gh` calls are reads of GitHub's REST API.

`DIR` is the factory folder. The `gate` criterion needs it. Without `DIR`, `gate` is unknown.

Schema of `--json`: [`readiness.json`](../schemas/readiness.json).

## Criteria

They come in this order.

| `id` | Met when | `by` | Fix |
| ---- | -------- | ---- | --- |
| `agents-file` | `AGENTS.md` or `CLAUDE.md` is a file at the root | `pull-request` | Add `AGENTS.md`: how to build, test and lint, and the rules a change must follow |
| `test-command` | One command runs the tests. See [Test commands](#test-commands) | `pull-request` | Add a `test` script to `package.json` or a `test` target to a Makefile |
| `linter` | A linter's config file is at the root, `Cargo.toml` has `[lints]` or `[workspace.lints]`, `pyproject.toml` has `[tool.ruff]`, `[tool.pylint]` or `[tool.flake8]`, or a CI workflow runs a linter. See [Linters](#linters) | `pull-request` | Add a linter's config file, or run a linter in CI |
| `setup` | A file pins the toolchain or sets up the machine. See [Setup files](#setup-files) | `pull-request` | Pin the toolchain, or add a dev container |
| `ci` | A file under `.github/workflows/` ending in `.yml` or `.yaml` contains `pull_request` | `pull-request` | Add a workflow that runs the tests on `pull_request` |
| `gate` | The factory in `DIR` sets [`gate`](factory.md#top-level-keys) | `factory` | Set `gate` to the test command found, such as `gate = "cargo test"`, before the first table of `factory.toml`. With no test command found, give the tests one first |
| `required-checks` | The default branch requires at least one check, by a branch ruleset or by classic branch protection | `person` | In the repository's settings, add a branch ruleset on the default branch that requires the CI checks |

`by` says who can make the fix:

| `by` | Who |
| ---- | --- |
| `pull-request` | A pull request on the repository, which an agent can open |
| `factory` | A change to the factory's own files |
| `person` | A person, in the repository's settings |

### Test commands

The first that matches gives `detail`:

| Found at the root | `detail` |
| ----------------- | -------- |
| `Cargo.toml` | `cargo test` |
| `package.json` whose `scripts.test` is not the `no test specified` placeholder of `npm init` | `npm test` |
| `go.mod` | `go test ./...` |
| `GNUmakefile`, `makefile` or `Makefile` with a `test:` target | `make test` |
| `justfile`, `Justfile` or `.justfile` with a `test` recipe | `just test` |
| `tox.ini` | `tox` |
| `noxfile.py` | `nox` |
| `pytest.ini` | `pytest` |
| `mix.exs` | `mix test` |
| `pom.xml` | `mvn test` |
| `build.gradle` or `build.gradle.kts` | `gradle test` |
| `pyproject.toml` with `[tool.pytest` | `pytest` |

### Linters

Config files at the root: `rustfmt.toml`, `.rustfmt.toml`, `clippy.toml`, `.clippy.toml`,
`eslint.config.js`, `.mjs`, `.cjs` or `.ts`, `.eslintrc` and its `.js`, `.cjs`, `.json`, `.yml` and
`.yaml` forms, `biome.json`, `biome.jsonc`, `.golangci.yml`, `.yaml` or `.toml`, `ruff.toml`,
`.ruff.toml`, `.flake8`, `.pylintrc`, `.rubocop.yml`, `.swiftlint.yml` and
`.pre-commit-config.yaml`.

Commands in a CI workflow: `clippy`, `cargo fmt`, `eslint`, `biome`, `golangci-lint`, `ruff`,
`flake8`, `pylint`, `rubocop`, `swiftlint`, `ktlint` and `pre-commit`.

### Setup files

At the root: `rust-toolchain.toml`, `rust-toolchain`, `.tool-versions`, `mise.toml`, `.mise.toml`,
`.nvmrc`, `.node-version`, `.python-version`, `.ruby-version`, `.go-version`, `flake.nix`,
`shell.nix`, `devbox.json`, `.devcontainer.json` and `.devcontainer/devcontainer.json`.

### Required checks

`priory check` runs these, in `PATH`:

1. `gh api repos/{owner}/{repo}`: gh fills the repository from the checkout's git remote. It gives
   the default branch.
2. `gh api repos/<owner>/<name>/rules/branches/<branch>`: the contexts of each
   `required_status_checks` rule of the rulesets on the branch.
3. When the rulesets require none, `gh api repos/<owner>/<name>/branches/<branch>`: the
   `contexts` and `checks` of the branch's classic protection.

When a call fails, such as outside a GitHub checkout or without `gh` signed in, the criterion is
`unknown` and `detail` holds the first line of the error.

## Statuses

| `status` | Meaning |
| -------- | ------- |
| `met` | `detail` says what was found, such as `cargo test` or `main requires ci, lint` |
| `missing` | `detail` says what was looked for |
| `unknown` | It could not be checked, and `detail` says why |

## Output

The text output starts with a score line, then one tab-separated line per criterion: status, `id`
and `detail`. A missing or unknown criterion is followed by a line with its fix.

```text
readiness: 5 of 7 met, 1 missing, 1 unknown
met	agents-file	AGENTS.md
met	test-command	cargo test
met	linter	clippy in .github/workflows/ci.yml
met	setup	rust-toolchain.toml
met	ci	.github/workflows/ci.yml
missing	gate	factory.toml sets no gate
	fix: Set `gate = "cargo test"` in factory.toml, before its first table, so a step can require `gate-passed` before it ends.
unknown	required-checks	gh could not read it: ...
	fix: In the repository's settings, add a branch ruleset on the default branch that requires the CI checks to pass before a merge.
```

`--json` prints the same as one object:

```json
{
  "repo": "../my-project",
  "met": 6,
  "missing": 1,
  "unknown": 0,
  "criteria": [
    {
      "id": "agents-file",
      "status": "met",
      "detail": "AGENTS.md",
      "fix": "Add AGENTS.md at the repository's root: how to build, test and lint, and the rules a change must follow.",
      "by": "pull-request"
    }
  ]
}
```

## Exit codes and errors

It exits 0 whatever it finds, so read `--json` to act on the gaps.

| Case | Exit | Message |
| ---- | ---- | ------- |
| `PATH` is not a folder | 1 | `priory check: <PATH> is not a folder` |
| The factory in `DIR` has a problem | 1 | each problem, as `priory check DIR` prints it |
| `--repo` without a path, `--explain` with `--repo`, or an unknown flag | 2 | the usage |

The [`readiness`](../../examples/readiness/README.md) starter turns each gap a pull request can
close into an issue, and each issue into a task.
