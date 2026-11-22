# Repository readiness

This note designs `priory check --repo PATH`, from idea 9 of the scout report of other agent
factories (read 2026-11-22). [readiness.md](../reference/readiness.md) describes what works today.

## Why

Most unattended runs that fail fail because of the repository, not the agent: no single test
command, no CI on pull requests, nothing that says how to build. Other factories score this.
Factory's Agent Readiness scores a repository on five levels and remediates with a command. Amp
publishes a checklist with a prompt per item. A score also gives a new user a first result before
any task runs.

## Design

- **Read-only.** The check writes no file and calls only GitHub's REST reads. A fix is a pull
  request a person merges, or a settings change a person makes (principle 4).
- **Few, objective criteria.** Each one is a file or a setting that exists or does not: an agents
  file, a test command, a linter, a setup file, CI on pull requests, the factory's `gate`, and
  required checks on the default branch. No weights and no levels: seven criteria read at a glance.
- **Each criterion says who fixes it.** `by` is `pull-request`, `factory` or `person`. A workflow
  files issues only for `pull-request` gaps, so no agent is asked to change a repository's settings.
- **Unknown is not missing.** A check that cannot run, such as required checks outside a GitHub
  checkout, says so and counts apart.
- **Exit 0.** Gaps are not errors of the factory. `--json` is what a workflow reads.

## Later slices

1. Built. `priory check [DIR] --repo PATH [--json]`, its seven criteria, `docs/schemas/readiness.json`
   and the [`readiness`](../../examples/readiness/README.md) starter, which files one issue per gap
   after a person approves and fixes each in a pull request.
2. Count a repository's [setup recipe](../daemon/runs.md#recipes) beside the setup files. The
   recipes are built, and the check does not read them yet.
3. Show the score in the web view, once `DESIGN.md` describes a panel for it.

## Open questions

1. **Levels?** Factory unlocks a level at 80 % of its criteria. Recommendation: not with seven
   criteria; revisit if the list grows past a dozen.
2. **Exit 1 on a gap?** It would let CI fail on a regression. Recommendation: add a flag such as
   `--strict` when someone asks, and keep exit 0 by default.
