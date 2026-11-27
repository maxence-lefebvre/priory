# Cloud runs

An agent with `harness = "claude-cloud"` runs its steps in a Claude cloud session, on Anthropic's
machines. Its builds and tests take no CPU, memory or disk on the runner, which only starts the
session and waits for its branch. This page explains how such a run starts, how it ends a step, and
what it gives up. To set one up, follow [How to run a step in the cloud](../running-a-step-in-the-cloud.md).
[ADR 0005](../adr/0005-a-cloud-run-reports-by-pushing-a-branch-the-daemon-polls.md) says why it
reports through a branch, and [ADR 0006](../adr/0006-a-cloud-run-reports-through-the-gateway-and-its-branch-push-carries-its-code.md)
why, with `[cloud] gateway = true`, it also reaches `ask`, `wait` and `result` through the
diocese's gateway.

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
2. checks that the repository opted in with `cloud = true`, and that the worktree is on that branch
   with no staged, unstaged or untracked file, since the session would not see it. Otherwise the
   task blocks and says which. When git runs past 30 seconds here, the task waits a minute, twice
   as long after each further timeout, and blocks after the fifth in a row.
   A daemon restart starts the count again;
3. records the run, with a harness session id of its own and the commit the branch is at;
4. records Claude's trust of the folder below, when the factory sets `trust_worktrees`, since
   `claude --cloud` shows Claude's trust dialog in a folder it does not trust;
5. writes the prompt: the agent's prompt, the step's prompt, the task's event, everything queued
   for the task, and the rules below in place of the line that names the `result` tool. It leaves out the lines
   about a duplicate that tell an agent to use the `message`, `hand_over` or `ask` tool, and holds them with the message
   they answer for the next ask and the next local run.

Some tasks start no cloud run:

- A trigger with `staged = true`, whose task blocks, since a start pushes.
- A replay, which ends with `agent error`.
- A task stacked on another, while its branch holds commits of its parent that are not on `origin`,
  since the push would carry them. Priory checks the parent's branch and the commits the task
  started from, so a commit the parent amended away counts too. The state of the parent's pull
  request does not matter. Push those commits, or rebase the task's branch onto a branch that is on
  `origin`, then resume the task. Priory sets `remote.origin.prune = false` in the clone of a
  repository with `cloud = true`, over any value there, so a merged parent's branch still counts as
  pushed once GitHub deletes it. All tasks of that repository share the clone. Every other clone keeps a person's setting.
  The setting stays after the repository drops `cloud = true`, since Priory saves no earlier value.
  To undo it, run `git -C <Priory home>/repos/<owner>/<name> config --local --unset remote.origin.prune`,
  or set the value the clone had.
- A task whose commits past the default branch hold a private term of the factory, in a repository
  [`[private]`](../reference/factory.md#private) lists. It blocks before the push, and the reason
  names the rule and the place, never the term. It does not count as a session.
- A task that started `[cloud] sessions` cloud sessions, 10 when unset. It blocks until a person
  raises the limit.

A task whose last start made no session waits a minute before the next, and each further failure in
a row doubles the wait, to 30 minutes at most.

The rest runs off the daemon's tick, so a slow command delays no other run. One session starts at a
time. The daemon:

1. reads that the run is still live, after any check for private terms, so a cancel stops the push;
2. pushes the task's branch to `origin`, without force, and checks that `origin` has that commit;
3. makes a clone of that branch alone in `cloud/<owner>/<name>` under Priory's home;
4. reads again that the run is live, and runs `claude --cloud DESCRIPTION` in that clone, in a tmux session named `priory-cloud-<task>`, and
   waits up to `command_timeout` for it to exit. The description holds no task text: it says the task comes next;
5. removes the clone, and sends the prompt to the `session_<id>` the command printed, on the stdin
   of `claude -p --cloud SESSION`. A command's arguments are readable by every user of the machine,
   and its stdin is not. It reads that the run is live first, so a run cancelled meanwhile gets no
   prompt. When that read or the send fails, the task blocks as uncertain and names the session.
   The send does not wait for the session: a message sent while a turn runs joins that turn,
   checked on Claude Code 2.1.332;
6. stores that session as the run's cloud session, which `priory runs` shows in its `session` column.

When the state file cannot be read for those two checks, the daemon reads it again each second. One
start waits `command_timeout` at most for both together, past which a read under way still ends, and
only then fails.

The rules tell the session to push its work to the branch `claude/priory-<task>-<token>`, where
the token is the first 8 characters of the run's harness session id. They tell it to open no pull
request, and to write any open question in the last commit message under `Questions:`, since
nobody answers during the session.

The `claude/priory-<task>-*` branches stay on `origin` after the daemon takes the push, and after
`open-pr` opens the pull request from `priory/<task>`. A session can push more than one, such as a
copy with a suffix, and a copy can hold work the daemon did not take. Delete them by hand once the
pull request merges.

A cloud run never carries into the next step. The next cloud step, or the same step again, is a new
run with a new session, the whole prompt and a branch of its own.

A start that made no session ends the step `failed`: a refused push, a failed clone, the trust
dialog, or an error exit with a text Priory knows to mean that no session exists, such as `Not
logged in`. Its summary starts with `the cloud session did not start:`. After any
`failed` ending, the feedback the run's prompt held waits for the task's next step.

A start that may have made one blocks the task with `cloud run <id> may have started a cloud
session`, so no retry starts a second: a timeout, a killed command, any other error exit, output
with no session id, or a session that started from another commit because the branch `is not on GitHub`. A daemon that stops
during a start blocks the task the same way. A person looks at claude.ai/code, then resumes the
task, which starts a new session.

## How a cloud run ends its step

At most once a minute, or every 15 seconds to confirm a push it saw and once the deadline passed,
off the tick, the daemon asks `origin` for each live cloud run's branch with
`git ls-remote`, which has 15 seconds, and fetches the commit it names, which has `command_timeout`.
Neither prompts. The daemon takes the commit only when all of these hold:

- a second look, 15 seconds later or more, finds the same commit, so a first part of the work or a
  push about to be replaced is not taken;
- it descends from the commit the session started at;
- the worktree is on the task's branch, at that same commit, with nothing uncommitted.

Then, off the tick and after it read once more that the run has not ended, it moves the branch forward with `git merge --ff-only --no-overwrite-ignore`, so a local file
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
branch. Only a look that finds no push ends the run at once. After an error, or a push the worktree
cannot take, the daemon looks again every 15 seconds for as long as the `timeout`, counted from that
first late look, and then the run ends `failed` with the error.

A step with no `pushed` or `failed` outcome cannot store that result, so the task blocks with the
summary and the refusal. `priory check` refuses such a step first.

The daemon's self-upgrade waits for a cloud session that starts, even once its run was cancelled, and for any live run when the
release migrates. It does not wait for a cloud run whose session started, except while the daemon looks for its push.

A daemon restart keeps a live cloud run: it has no tmux pane to lose, so it is never `lost`, and the
next poll looks for its branch. A run stays what it started as when a person changes its agent's
`harness`.

`priory hold --now` refuses a live cloud run, since Priory cannot stop its session. A plain hold takes
effect once the run ends.

`priory cancel`, a review request's removal and a move to another runner drop the task, and its
reason names the session's page, which `priory cancel` prints too. During a start it says that a session
may still start. Once the start ends, the dropped task's reason says instead which session started,
that one may have, or that none did. When none did, the daemon looks on `origin` for the task's
branch, off the tick. The reason names the branch when it is there, so a person can delete it, and
says so when that look failed or the run ended after it. A daemon that stops during the start leaves the reason as it was.
A cancel that the last check before `claude --cloud` sees starts none. The session itself goes on until a
person archives it, and the branches it pushed stay on `origin`.

## Through the gateway

With `[cloud] gateway = true` in `factory.toml`, a cloud run also reaches `ask`, `wait` and
`result`, through the [diocese's gateway](../design/diocese.md#an-mcp-call-from-a-cloud-session).
With it false, the default, everything else on this page holds as written.

- **The grant.** Right after it records the run, the daemon makes a run grant: a random token that
  lasts twice the step's `timeout` and a minute, as long as the run may live with its grace. It publishes `priory.run.granted` with the token's SHA-256, the
  task's key, the run and the runner, and keeps only the hash. The token reaches the CLI once, as
  the `Authorization: Bearer` header of an HTTP MCP server named `priory` at `[cloud] gateway_url`,
  in a file only the daemon's user reads, passed as `claude --mcp-config FILE --cloud DESCRIPTION`. The
  file goes once the CLI exits, and a daemon that starts removes any such file a killed start left.
  The token is in no prompt, argument, log line or trace. Nobody has yet checked that a real
  `claude --cloud` passes `--mcp-config` on to the session.
- **The rules.** The prompt still tells the session to push its branch, then to call `result`, and
  to call `ask` or `wait` and end its turn when it needs a person or an event.
- **A call.** The gateway publishes `priory.run.called`. The runner that made the grant reads it on
  its next tick, applies it as a call on its socket from the run's session, and publishes
  `priory.run.replied` with the tool's text. The gateway answers with it, or that the runner is
  away after 40 seconds. The same arguments sent again before a reply reached the session make the
  same call, so a retry applies once.
- **A wake.** Once an ask is answered, a wait fires or anything else is queued for the task after
  `ask` or `wait`, the daemon sends it on the stdin of `claude -p --cloud SESSION`, which
  starts the session's next turn. A failed wake is logged, and the desk still shows the answer.
- **The end.** `result` stores nothing yet: the daemon keeps it, looks for the push within 15
  seconds, and ends the step with that outcome, summary and evidence. It takes the pushed commit
  as above, or ends the step without one when a look that began after the result finds no new
  commit. A step's `next` may
  name any outcome, and a `result` naming one the step lacks is refused at once.
- **The token in a push.** Before it takes a push, the daemon hashes every run of 32 hex digits in
  the pushed commits' messages, authors, committers and changes, and fails the step when one is the
  grant's token. It reads each change in full: a merge's against each parent, binary files and
  files marked `-diff` as text, and no diff driver or textconv of the repository. The summary never
  holds the token. It misses the token written in another form, such as base64 or split across
  lines; on another branch or tag the session pushed; in a commit a later push replaced; and in a
  file Git LFS stores outside the repository.
- **The end of a grant.** The daemon publishes `priory.run.revoked` within a second of its run
  ending: by a result, a failure, a cancel, a move or a give. The gateway also refuses a call once
  the task's claim names another runner, so a task given away gets a new run with a new grant, and
  the old session's calls are refused.

## While the daemon is behind its log

The daemon [holds work until it read the log to its end](README.md) after a start, after a lost log
answers again and, with a runner file, whenever it is behind. The rest of the log may withdraw the
task or give it to another runner. Meanwhile:

- **Start:** no cloud run starts. A start already under way waits before its push and before
  `claude --cloud`, with the `command_timeout` it has for a state file that cannot be read. Past that
  it ends `failed`, and a branch it pushed stays on `origin`.
- **Poll:** no new look begins. One under way may fetch, which moves nothing. A deadline that passes
  ends no run: the daemon looks once more after the catch-up, and its grace counts from that look.
- **Push:** none is taken. A look that would take one ends without it, and the next look tries again.

The end of a start is stored only after the catch-up, so a daemon that stops in between blocks the
task as one that may have started a session. A run that the backlog ends is dropped as by a cancel,
and its reason names the session.

Three windows stay. A start or look that checked just before the daemon found its backlog goes on,
as one does that checked just before a cancel. So does one while the log is lost. A later backlog without a runner file
holds no cloud run, as it holds no local one.

## Asks and feedback

The factory's pre-review and a step's gate send a result back to its session, which a cloud session
cannot act on, so neither reads a cloud push. The ask it leads to names the ones that did not run.

Without the gateway, a cloud run cannot ask a person. The session has no way to reach the `ask`
tool, and nobody answers inside it. A session that needs a decision writes it in its last commit message, which
becomes the result's evidence, for the human step after it. A session that waits for an answer
instead pushes nothing, and the deadline ends the step `failed` with the session's link.

Feedback that comes before the start is in the prompt. The session sorts none of it into the classes
a local run's result gives. A person's own items are still recorded as `persons-call`. The rest
has no class, so the human step after it is where a person reads what it did. Feedback that comes
during the run does not hold up its push: the next model step's prompt has it, and an ask that opens next shows it.
News that may end the work, such as an issue marked a duplicate, does the same: no session reads it,
so it never refuses the push, and the ask shows it with the task to agree with. The task's later
cloud prompts leave it out too: it stays for the next ask, which shows it once, and for the next
local run.

## What still applies, and what a cloud run gives up

| Guarantee | Cloud run |
| --------- | --------- |
| Room and the memory gate | A cloud run reserves no memory or disk, waits for no room, and waits behind no task that does |
| A bound on concurrent runs | `[cloud] max`, 4 when unset, on each runner |
| A bound on sessions per task | `[cloud] sessions`, 10 when unset |
| Worktree, sweep and recipes | Apply: the run has a worktree, and the sweep removes it as any other |
| Lane caps, time windows and runner filters | Apply: a live cloud run counts toward its lane's cap |
| Overlap holds | See a cloud run's files only once its push is taken |
| direnv | None: the session reads no `.envrc` of the runner |
| Private terms | The commits a start pushes are screened. What the session pushes is screened at `open-pr` only |
| Approvals, `open-pr`, checks and merge | Apply: they act on the task's branch after the push |
| Hooks, health checks and nudges | None: the deadline replaces them |
| Tool guard and `permissions` | None: the cloud environment's own policy applies |
| Staged triggers | A cloud step's task blocks before its push |
| Step effects | `none` and `read` are refused by `priory check`: a start pushes the task's branch |
| Sandbox | Refused by `priory check`, and at the start: Priory cannot jail a session |
| `ask`, `wait`, `remember`, `progress` and MCP servers | `ask`, `wait` and `result` through the gateway with `[cloud] gateway = true`, as [Through the gateway](#through-the-gateway) says. Otherwise none: the session loads neither the agent's MCP servers nor the account's claude.ai connectors (tested on 2026-11-26) |
| Proofs, gates, `plan_requires`, `budget`, `model` and `effort` | Refused by `priory check` |
| Trust grants | Apply with `trust_worktrees`, for the folder `claude --cloud` runs in |
| Tokens and cost | Unknown: see Cost |

The session's GitHub access is the person's, through the Claude GitHub App or `/web-setup`. It can
push any branch the proxy lets it, so the repository's default branch needs a protection rule. The
branch name is all that ties a push to its run: anyone who can push to the repository can push it.

## Secrets

Priory passes the session its prompt and no environment variable. With `[cloud] gateway = true`
it also passes the run grant's token, as [Through the gateway](#through-the-gateway) says. It pushes the task's
branch to GitHub before the session starts, so the branch's commits are there before a person
approves `open-pr`. A workflow of the repository with `on: push` runs on that push, with the
repository's secrets, before that approval too. A repository therefore opts in with `cloud = true`,
and `priory check` warns when a local step that may commit leads to a cloud step with no human step
between.

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
`https://claude.ai/code/<id>`. Spend caps cannot count it, so
`[cloud] max` bounds the sessions at once and `[cloud] sessions` those of one task.
