# A cloud run reports by pushing a branch the daemon polls

Date: 2026-11-24. Status: accepted.

## Context

A local run ends its step with the `result` tool, on the MCP server the daemon serves on its
socket. A Claude cloud session runs on Anthropic's machines and cannot reach that socket, which is
bound to the runner. Something else has to carry its outcome back.

Claude Code 2.1.331 offers these ways to observe a cloud session from a program:

- `claude --cloud PROMPT` creates a session and prints its id, in a terminal only.
- `claude -p MSG --cloud ID` sends a session a message and prints JSON.
- No command reads a session's state, its transcript or its final message.
- Inside the session, `gh pr create` fails: the session's GitHub proxy blocks GraphQL. The session
  can push a branch.

The channel decides what every later slice builds on: feedback, asks, costs and the drift between
sessions. Changing it later means changing the prompt every cloud step sends and how the daemon
matches work to a task.

## Decision

The daemon names a branch for each cloud run, `claude/priory-<task>-<token>`, and the prompt tells
the session to push its work there and to open no pull request. The daemon polls `origin` for that
branch with `git ls-remote`, at most once a minute per live cloud run. When the branch appears, the
daemon moves the task's own branch forward to it and stores the run's result itself, outcome
`pushed`. It takes only a commit that descends from the one the session started at, and that two polls in a
row saw.
A deadline, the step's `timeout`, ends a run whose branch never appears, outcome `failed`.

`open-pr`, a code step a person approves, then opens the pull request from the task's branch, as
for a local run.

## Alternatives

- **The session's final message, read by polling.** No command or documented API reads it.
- **A pull request the session opens, matched by a token in its body.** The session cannot open
  one through `gh`. A pull request opened by the session would also bypass the approval that
  `open-pr` needs, since `open-pr` is irreversible in every starter.
- **A result file the session commits.** It travels on the same branch, so it needs the same poll.
  It can come later on top of this channel, if a step needs outcomes other than `pushed`.

## Consequences

- A cloud step has two outcomes, `pushed` and `failed`, and `priory check` refuses others.
- The daemon needs no webhook and no new intake event: `git ls-remote` works from behind NAT and
  after a runner wakes from sleep.
- Branches named `claude/priory-*` stay on `origin` after `open-pr` pushes `priory/<task>`. A
  person deletes them, or a later slice does.
- The daemon pushes `priory/<task>` before it starts a session, so the session has the task's
  commits. They reach GitHub before a person approves `open-pr`.
- The branch name is the only thing that ties a push to a run. Anyone who can push to the repository
  can push that branch.
- A session that ignores the branch name looks like a silent session: the deadline ends it.
