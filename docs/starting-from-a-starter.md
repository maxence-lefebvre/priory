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
| [`morning-digest`](../examples/morning-digest/README.md) | On workday mornings, the agent posts the open asks, stuck tasks and yesterday's merges to your desk |
| [`release-checklist`](../examples/release-checklist/README.md) | A runbook: you start a release with `priory add`, and each step asks you. Answering `run` at the test step runs `cargo test` once |
| [`groom`](../examples/groom/README.md) | A `groom` label starts a task. The agent grades the ticket, writes the brief, and takes it to a pull request you review |
| [`batches`](../examples/batches/README.md) | `priory batch add` hands several issues to a planner, which orders them. After you answer `go`, each issue becomes a task that starts once the ones it needs merged |
| [`benchmark`](../examples/benchmark/README.md) | A `benchmark` label on a pull request of the factory's own repository asks you to compare it. Answering `run` replays past tasks under both commits and posts the table and a status on the pull request |
| [`ship-and-watch`](../examples/ship-and-watch/README.md) | A `ship` label starts a task. After you merge its pull request, the agent watches the release for an hour, then drafts an announcement that posts once you approve it |

Copy one into your working folder, then check it:

```sh
cp -r <your clone>/examples/review my-factory
priory check my-factory
```

Replace the placeholders `owner/repo` and `your-login` in its `triggers/` files, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname. Then run it as [Getting started](../GETTING_STARTED.md#5-run-it) runs its
`example` folder: use `my-factory` in place of `example`, and the starter's workflow, the file name
under its `workflows/`, in place of `implement`.
