# How to start from a starter factory

This guide shows how to start your factory from one of the ready-made factories in the repository's
`examples/` folder. It assumes you built Priory and have a clone of the repository.

The `examples/` folder holds these starters. Each one passes `priory check`, and
its `README.md` says what it needs and which steps ask you first.

| Starter | What it does |
| ------- | ------------ |
| [`bug-investigation`](../examples/bug-investigation/README.md) | A `bug` label starts a task. The agent reproduces the bug, finds the root cause, fixes it, and asks before it opens the pull request |
| [`review`](../examples/review/README.md) | A review request starts a task. The agent writes a pending GitHub review, and you submit it |
| [`ci-failure`](../examples/ci-failure/README.md) | Every hour on workdays, the agent fixes a failed check and asks before it opens the pull request |
| [`triage`](../examples/triage/README.md) | A new issue starts a task. The agent labels it, and asks before it posts a comment that asks for what is missing |
| [`dreaming`](../examples/dreaming/README.md) | Each night, an agent reads the day's results and answers and proposes memories, which you accept or drop the next morning. A daily pull request moves accepted ones into your kit |
| [`morning-digest`](../examples/morning-digest/README.md) | On workday mornings, the agent posts the open asks, stuck tasks and yesterday's merges to your desk |
| [`release-checklist`](../examples/release-checklist/README.md) | A runbook: you start a release with `priory add`, and each step asks you. Answering `run` at the test step runs `cargo test` once |
| [`groom`](../examples/groom/README.md) | A `groom` label starts a task. The agent grades the ticket, writes the brief, and takes it to a pull request you review |
| [`batches`](../examples/batches/README.md) | `priory batch add` hands several issues to a planner, which orders them. After you answer `go`, each issue becomes a task that starts once the ones it needs merged |
| [`benchmark`](../examples/benchmark/README.md) | A `benchmark` label on a pull request of the factory's own repository asks you to compare it. Answering `run` replays past tasks under both commits and posts the table and a status on the pull request |
| [`readiness`](../examples/readiness/README.md) | `priory add` starts a check of the repository. The agent drafts one issue per gap that `priory check --repo` finds, files them once you approve, and each issue becomes a task that ends on a pull request |
| [`decisions`](../examples/decisions/README.md) | Not a whole factory: a `[decisions.triage]` table, a script for TypeSafe's Jev and ten labeled cases. Once calibrated, a review comment Jev sorts as `nothing-to-do` wakes no agent |
| [`ship-and-watch`](../examples/ship-and-watch/README.md) | A `ship` label starts a task. After you merge its pull request, the agent watches the release for an hour, then drafts an announcement that posts once you approve it |
| [`design-debate`](../examples/design-debate/README.md) | A `design-debate` label starts a task. A Claude Code agent and a Codex agent debate the linked design for up to three rounds, and a split asks you |

Copy one into your working folder, then check it:

```sh
cp -r <your clone>/examples/review my-factory
priory check my-factory
```

Replace the placeholders `owner/repo` and `your-login` in its `triggers/` files, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname. Then run it as [Getting started](../GETTING_STARTED.md#4-run-it) runs its
`example` folder: use `my-factory` in place of `example`, and the starter's workflow, the file name
under its `workflows/`, in place of `implement`.
