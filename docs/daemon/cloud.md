# Cloud runs

An agent with `harness = "claude-cloud"` runs its steps in a Claude cloud session, on Anthropic's
machines. Its builds and tests take no CPU, memory or disk on the runner, which only starts the
session and waits for its branch. This page explains how such a run starts, how it ends a step, and
what it gives up. To set one up, follow [How to run a step in the cloud](../running-a-step-in-the-cloud.md).
[ADR 0005](../adr/0005-a-cloud-run-reports-by-pushing-a-branch-the-daemon-polls.md) says why it
reports through a branch.

## What the CLI allows

Claude Code creates a cloud session with `claude --cloud PROMPT`, from the folder of a GitHub
clone. When the Claude GitHub App is installed on the repository, the session clones it from GitHub
at the folder's branch. When it is not, even after `/web-setup`, the command uploads the folder's
repository: its history across all branches, and the uncommitted changes to its tracked files. No
command says beforehand which of the two it will do.

The command needs a terminal: with its output piped, or with `--print`, it refuses to start. In a
terminal it prints the session's id in a `View: https://claude.ai/code/session_<id>` line, then
exits. No command reads a session's state or its transcript, and none stops it. Inside the session,
`gh pr create` fails, because the session's GitHub proxy blocks GitHub's GraphQL API. The session
can push a branch.

## How a cloud run starts

When a task reaches a step whose agent runs in the cloud, the daemon:

1. creates the task's worktree as for any run, on the task's own branch, `priory/<task>`;
2. checks that the worktree is on that branch with no staged, unstaged or untracked file, since the
   session would not see it. Otherwise the task blocks with the files;
   A trigger with `staged = true` starts no cloud run: the task blocks, since a start pushes. A task
   stacked on another blocks while its branch holds commits of its parent's branch that are not on
   `origin`, since the push would carry them. Push the parent's branch, then resume the task;
3. records the run, with a harness session id of its own and the commit the branch is at;
4. records Claude's trust of the folder below, when the factory sets `trust_worktrees`, since
   `claude --cloud` shows Claude's trust dialog in a folder it does not trust;
5. writes the prompt: the agent's prompt, the step's prompt, the task's event, everything queued
   for the task, and the rules below in place of the line that names the `result` tool.

The rest runs off the daemon's tick, so a slow command delays no other run. One session starts at a
time. The daemon:

1. pushes the task's branch to `origin`, without force, and checks that `origin` has that commit;
2. makes a clone of that branch alone in `cloud/<owner>/<name>` under Priory's home;
3. runs `claude --cloud PROMPT` in that clone, in a tmux session named `priory-cloud-<task>`, and
   waits up to `command_timeout` for it to exit;
4. removes the clone, and stores the `session_<id>` the command printed as the run's cloud session,
   which `priory runs` shows in its `session` column.

The rules tell the session to push its work to the branch `claude/priory-<task>-<token>`, where
the token is the first 8 characters of the run's harness session id. They tell it to open no pull
request, and to write any open question in the last commit message under `Questions:`, since
nobody answers during the session.

A cloud run never carries into the next step. The next cloud step, or the same step again, is a new
run with a new session, the whole prompt and a branch of its own.

A start that made no session ends the step `failed`: a refused push, a failed clone, the trust
dialog, or an error exit. Its summary starts with `the cloud session did not start:`. After any
`failed` ending, the feedback the run's prompt held waits for the task's next step.

A start that may have made one blocks the task with `cloud run <id> may have started a cloud
session`, so no retry starts a second: a timeout, a killed command, output with no session id, or a
session that started from another commit because the branch `is not on GitHub`. A daemon that stops
during a start blocks the task the same way. A person looks at claude.ai/code, then resumes the
task, which starts a new session.

## How a cloud run ends its step

At most once a minute, off the tick, the daemon asks `origin` for each live cloud run's branch with
`git ls-remote`, and fetches the commit it names. Each of these git calls has 15 seconds and never
prompts. The daemon takes the commit only when all of these hold:

- it descends from the commit the session started at;
- the worktree is on the task's branch, at that same commit, with nothing uncommitted.

Then it moves the branch forward with `git merge --ff-only --no-overwrite-ignore`, so a local file
that git ignores is never replaced by one the push tracks, and stores the run's result as the run
would with the `result` tool: outcome `pushed`, the commit's subject as the summary, its body as the
evidence, and its SHA as the commit. It ends the turn as a `Stop` hook does, so the task moves by
the step's `next`, as after a local run.

A branch still at the start commit counts as no push yet. A commit that does not descend from the
start ends the step `failed`, and nothing moves. A failed fetch, a worktree that a person changed or
switched, or an ignored local file in the way waits: the daemon logs why and looks again
until the deadline.

The usual next step is a human step that approves `open-pr`. `open-pr` pushes the task's branch,
now at the session's commit, and opens the pull request from it. From then on the task is a task
with a pull request like any other: its checks, review feedback and merge step apply.

A run whose push has not been taken when the step's `timeout` passes, 2 hours when unset, ends
`failed`, with the summary `cloud session <id> pushed no branch <branch> within <n>s. A cloud run
cannot ask, so open the session to see why: https://claude.ai/code/<id>`, or with why it could not
take the push. The step's `next.failed` routes the task. The deadline counts from the run's start,
and the daemon looks once more after it, so a runner that slept while the session worked finds the
branch.

A step with no `pushed` or `failed` outcome cannot store that result, so the task blocks with the
summary and the refusal. `priory check` refuses such a step first.

A daemon restart keeps a live cloud run: it has no tmux pane to lose, so it is never `lost`, and the
next poll looks for its branch. A run stays what it started as when a person changes its agent's
`harness`.

`priory cancel` drops the task and prints the session's page. During a start it says that a session
may still start, and the dropped task's reason then names the session that did. The session itself goes on until a
person archives it, and the branches it pushed stay on `origin`.

## Asks and feedback

A cloud run cannot ask a person. The session has no way to reach the `ask` tool, and nobody
answers inside it. A session that needs a decision writes it in its last commit message, which
becomes the result's evidence, for the human step after it. A session that waits for an answer
instead pushes nothing, and the deadline ends the step `failed` with the session's link.

Feedback that comes before the start is in the prompt. Feedback that comes during the run does not
hold up its push: the next model step's prompt has it, and an ask that opens next shows it.

## What still applies, and what a cloud run gives up

| Guarantee | Cloud run |
| --------- | --------- |
| Room and the memory gate | A cloud run reserves no memory or disk, waits for no room, and waits behind no task that does |
| A bound on concurrent runs | `[cloud] max`, 4 when unset, on each runner |
| Worktree, sweep and recipes | Apply: the run has a worktree, and the sweep removes it as any other |
| Approvals, `open-pr`, checks and merge | Apply: they act on the task's branch after the push |
| Hooks, health checks and nudges | None: the deadline replaces them |
| Tool guard and `permissions` | None: the cloud environment's own policy applies |
| Staged triggers | A cloud step's task blocks before its push |
| Step effects | `none` and `read` are refused by `priory check`: a start pushes the task's branch |
| Sandbox | Refused by `priory check`, and at the start: Priory cannot jail a session |
| `ask`, `wait`, `remember`, `progress` and MCP servers | None |
| Proofs, gates, `plan_requires`, `budget`, `model` and `effort` | Refused by `priory check` |
| Trust grants | Apply with `trust_worktrees`, for the folder `claude --cloud` runs in |
| Tokens and cost | Unknown: see Cost |

The session's GitHub access is the person's, through the Claude GitHub App or `/web-setup`. It can
push any branch the proxy lets it, so the repository's default branch needs a protection rule. The
branch name is all that ties a push to its run: anyone who can push to the repository can push it.

## Secrets

Priory passes the session its prompt: no environment variable and no token. It pushes the task's
branch to GitHub before the session starts, so the branch's commits are there before a person
approves `open-pr`.

Without the Claude GitHub App on the repository, Claude Code uploads the repository it runs in. A
Priory worktree shares its clone with every other task, so the command never runs there. It runs in
a clone that holds one branch, with no uncommitted change and no other task's commit, and that
branch is already on GitHub. An upload can hold nothing more. The daemon logs the upload the CLI
reports, and the summary of a missed deadline names it, since such a session often cannot push.

The cloud environment's setup script installs tools and writes instructions, and holds no secret,
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
`https://claude.ai/code/<id>`. Spend caps cannot count it, so `[cloud] max` is the only bound.
