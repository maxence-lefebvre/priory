# Cloud runs

An agent with `harness = "claude-cloud"` runs its steps in a Claude cloud session, on Anthropic's
machines. Its builds and tests take no CPU, memory or disk on the runner, which only starts the
session and waits for its branch. This page explains how such a run starts, how it ends a step, and
what it gives up. To set one up, follow [How to run a step in the cloud](../running-a-step-in-the-cloud.md).
[ADR 0005](../adr/0005-a-cloud-run-reports-by-pushing-a-branch-the-daemon-polls.md) says why it
reports through a branch.

## What the CLI allows

Claude Code creates a cloud session with `claude --cloud PROMPT`, from the folder of a GitHub
clone. The session clones that repository's GitHub remote. The command needs a terminal: with its
output piped, or with `--print`, it refuses to start. In a terminal it prints the session's id in a
`View: https://claude.ai/code/session_<id>` line, then exits. No command reads a session's state or
its transcript, and none stops it. Inside the session, `gh pr create` fails, because the
session's GitHub proxy blocks GitHub's GraphQL API. The session can push a branch.

## How a cloud run starts

When a task reaches a step whose agent runs in the cloud, the daemon:

1. creates the task's worktree as for any run, on the task's own branch, `priory/<task>`;
2. records the run, with a harness session id of its own, as for a local run;
3. records Claude's trust of the worktree and the clone, when the factory sets `trust_worktrees`,
   since `claude --cloud` shows Claude's trust dialog in a folder it does not trust;
4. writes the prompt: the agent's prompt, the step's first prompt with its events, and the rules
   below in place of the line that names the `result` tool;
5. runs `claude --cloud PROMPT` in a tmux session named `priory-cloud-<task>`, in the worktree, and
   waits up to `command_timeout` for it to exit;
6. stores the `session_<id>` it printed as the run's cloud session, which `priory runs` shows in its
   `session` column, and kills the tmux session.

The rules tell the session to push its work to the branch `claude/priory-<task>-<token>`, where
the token is the first 8 characters of the run's harness session id. They tell it to open no pull
request, and to write any open question in the last commit message under `Questions:`, since
nobody answers during the session.

A command that exits with an error, prints no session id, or shows the trust dialog, ends the
step `failed`. Its summary
starts with `the cloud session did not start:` and holds what the command printed.

The daemon waits for `claude --cloud` inside its tick, so the rest of that tick waits the few seconds the command takes.

## How a cloud run ends its step

At most once a minute, the daemon asks `origin` for each live cloud run's branch with `git
ls-remote`. When the branch is there, it fetches it and resets the task's branch and worktree to
it. Then it stores the run's result as the run would with the `result` tool: outcome `pushed`, the
head commit's subject as the summary, its body as the evidence, and its SHA as the commit. It ends
the turn as a `Stop` hook does, so the task moves by the step's `next`, as after a local run.

The usual next step is a human step that approves `open-pr`. `open-pr` pushes the task's branch,
now at the session's commit, and opens the pull request from it. From then on the task is a task
with a pull request like any other: its checks, review feedback and merge step apply.

A run whose branch has not arrived when the step's `timeout` passes, 2 hours when unset, ends
`failed`, with the summary `cloud session <id> pushed no branch <branch> within <n>s. A cloud run
cannot ask, so open the session to see why: https://claude.ai/code/<id>`. The step's `next.failed`
routes the task. The deadline counts from the run's start, so a runner that slept while the session
worked finds the branch at its next poll.

A step with no `pushed` or `failed` outcome cannot store that result, so the task blocks with the
summary and the refusal. `priory check` refuses such a step first.

A daemon that stops while `claude --cloud` runs cannot tell whether the session started. At its
next tick, it blocks the task with `cloud run <id> may or may not have started a cloud session`,
and never runs the command again for that run. A person checks claude.ai/code before resuming.

## Asks

A cloud run cannot ask a person. The session has no way to reach the `ask` tool, and nobody
answers inside it. A session that needs a decision writes it in its last commit message, which
becomes the result's evidence, for the human step after it. A session that waits for an answer
instead pushes nothing, and the deadline ends the step `failed` with the session's link.

## What still applies, and what a cloud run gives up

| Guarantee | Cloud run |
| --------- | --------- |
| Room and the memory gate | A cloud run reserves no memory or disk, waits for no room, and waits behind no task that does |
| Worktree, sweep and recipes | Apply: the run has a worktree, and the sweep removes it as any other |
| Approvals, `open-pr`, checks and merge | Apply: they act on the task's branch after the push |
| Hooks, health checks and nudges | None: the deadline replaces them |
| Tool guard and `permissions` | None: the cloud environment's own policy applies |
| `ask`, `wait`, `remember`, `progress` and MCP servers | None |
| Proofs, gates and `plan_requires` | Refused by `priory check` |
| Trust grants | Apply with `trust_worktrees`, for `claude --cloud` itself |
| Tokens and cost | Unknown: see Cost |

The session's GitHub access is the person's, through the Claude GitHub App or `/web-setup`. It can
push any branch the proxy lets it, so the repository's default branch needs a protection rule.

## Secrets

Priory passes the session its prompt and nothing else: no environment variable, no file and no
token. The session clones the repository with the person's own claude.ai GitHub connection. The
cloud environment's setup script installs tools and writes instructions, and holds no secret,
since anyone who uses the environment can read it.

## Instructions and skills

A cloud session reads the repository's own `CLAUDE.md` and `.claude/` folder, and the session
user's `~/.claude`. It does not get the plugin folder a local run gets. `priory cloud setup-script`
prints a bash script that writes the factory's instructions and skills, the kit's first, under
`~/.claude`, then runs each repository's `cloud_setup` script. A person pastes it into a cloud
environment's setup script once, and again after the kit changes. The cloud environment caches
what its setup script made, so it runs again only when the script changes, or after about seven
days.

## Cost

Priory cannot read a cloud session's tokens: the transcript stays in the cloud. The step's
`factory.step.ended` record has no `tokens`, so `priory costs` counts it under `tokens unknown for
N records` and never prices it at `$0`. The session's use counts against the person's Claude plan,
which claude.ai shows. `priory runs` shows each cloud run's session id, whose page is
`https://claude.ai/code/<id>`.
