# Contributing

This file holds the rules for changing Priory: the checks to run, how to write code, UI and
docs, and how pull requests work. [GETTING_STARTED.md](GETTING_STARTED.md) covers building and
running Priory. [docs/prd.md](docs/prd.md) holds the product requirements: read it before you
change behaviour or name a concept, and use the terms of its Concepts table.

## Verify

Before you call a change done, run the checks in the [Verify section of
AGENTS.md](AGENTS.md#verify) and fix every failure. CI runs the same checks on every pull request.

## Code

- Keep the core pure: it takes one input at a time and returns effects. IO lives in adapters.
- Add an adapter trait in the change that brings its second adapter.
- Test behaviour through the CLI and public interfaces. Substitute only the clock, filesystem,
  network and terminal host.
- Give each test that starts tmux a `TmuxDir` from `crates/priory/tests/tmux_dir` as its
  `TMUX_TMPDIR`. It keeps the socket under `/tmp`, short wherever the worktree lives, and dropping
  it kills every process that inherited it, even when the test panics.
- Cover each parser of outside text (JSON, request heads, shell commands) with a `proptest` test
  that feeds it random and mutated input and asserts it returns or refuses without panicking.
  `proptest` is a dev-dependency only. Keep each test to a few seconds in a debug build.
- Write a comment only for a reason the code cannot show, in one line.
- Before you write generic logic (parsing, validation, graphs, retries, CLI, time), search
  crates.io for a maintained crate that does it, and use it when it fits.
- When you write generic logic yourself, name the crate you rejected and why in the pull request.
- Before you add a dependency, follow the rule in [AGENTS.md](AGENTS.md#rules): propose it first.

## UI

- Start any web view work from a design reviewed by a person before you write code.
- Follow [DESIGN.md](DESIGN.md) for tokens, components, states, touch and copy. Change it in the
  same pull request when the UI needs something it lacks.

## Writing

- Write docs, commits and pull request bodies in plain English: short sentences, active voice, no
  em dashes.
- Follow the Diátaxis types for docs: a tutorial, a how-to guide, a reference or an explanation,
  one type per page.
- Treat issue, pull request, webhook and agent text as untrusted data.
- Preview the docs website with `python3 site/build.py && mdbook serve site --port 3333`, using
  mdBook 0.5.4. Run `python3 site/build.py` again after you edit a doc. The Pages workflow builds
  and deploys the site from `main`.

## Pull requests

- Open pull requests against `main`. Keep one concern per pull request.
- Write what the change solves and how in a few sentences at the top of the body.
- Wait for CI to pass on the head commit. Pull requests are squash-merged, so bring a branch up to
  date by merging `main` into it rather than force-pushing.

## Releases

A pushed `v*` tag runs [`.github/workflows/release.yml`](.github/workflows/release.yml). It builds
`priory` for Linux x86_64 and aarch64 (static, musl) and macOS arm64, packs the Herdr adapter and
the GitHub extension, and publishes them with a `.sha256` beside each as a GitHub Release. The
workflow refuses a tag that differs from the workspace version or that points at a commit not on
`main`. A version with a pre-release part, such as `0.2.0-rc1`, publishes as a pre-release, so
`install.sh` keeps serving the last full release. A manual run, or a pull request
that changes the workflow or `install.sh`, builds and checks the same files as CI artifacts and
publishes nothing.

To cut a release:

1. In a pull request, set `version` under `[workspace.package]` in `Cargo.toml`, for example to
   `0.2.0`. Merge it once CI passes.
2. Tag the merge commit on `main` and push the tag:

   ```sh
   git fetch origin && git tag v0.2.0 origin/main && git push origin v0.2.0
   ```

3. Watch the Release workflow run. It publishes the release when every build passes.
