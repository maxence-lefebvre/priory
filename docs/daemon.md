# The daemon

`priory daemon [DIR] [--once]` reads the herdr-bus output `factory.intake` as the consumer `priory`,
records each message in the inbox and routes it to a task. It checks the factory first, as
`priory check` does, and refuses to start while it has problems. `--once` routes what is pending
and exits; it starts no runs, since no daemon would stay to hear their hooks. Without it, the
daemon looks for new messages every second and runs agents, as described under Runs.

State is one SQLite file, named by `PRIORY_DB`. The daemon refuses to start without it, or with an
empty name, `:memory:` or a `file:` URI, which SQLite may not keep on disk.

## One message at a time

Each message is handled in one SQLite transaction: its inbox entry, any task it creates, and the
entry's outcome. The daemon acks the message on the bus only after that transaction commits. A
message delivered again, after a crash between the commit and the ack, adds nothing: the inbox
drops a message id it already holds. An error stops the daemon before the ack, so the next start
handles that message again.

## Routing

A task owns resources, stored in one table: its trigger key from its creation, its branch
`github:{repo}@priory/{id}` from its first run, and each pull request `github:{repo}#{n}` a wait of
its names. A message names resources too: a task request its `key`, a CLI request its subject, a
GitHub event its issue or pull request `github:{repo}#{n}` and, for a pull request event, its head
branch `github:{repo}@{head_branch}`. The oldest active task that owns any of them decides the route.
A task is active in any state except done, failed and dropped. So an agent that opens a pull request
from its branch gets that pull request's events, with no trigger involved.

| Message                                   | Outcome   | Task                                         |
| ----------------------------------------- | --------- | -------------------------------------------- |
| Any, with an active task owning a resource it names | `kept` | that task                             |
| `factory.task.requested`, no active task  | `created` | a new task, `queued` at its workflow's start |
| `factory.task.requested`, malformed       | `ignored` | none                                         |
| Any other                                 | `ignored` | none                                         |

A task request for a workflow the factory lacks stops the daemon with an error, before its ack. The
request waits on the bus until the workflow is back. A malformed request is ignored instead, so that
it cannot block the output.

A source event comes before its task request, so it is recorded as `ignored`. The task keeps its
id in `source`.

A message kept for a `running` task also gets an outbox prompt, so the agent reads it after its
current turn. A message kept for an asleep task that satisfies its wait wakes it, as described under
Waits. A message kept for a task in any other state waits for the task's next run.

## Priory home

`PRIORY_HOME` names Priory's own directory. It defaults to `$XDG_STATE_HOME/priory-oss`, then
`~/.local/state/priory-oss`. It holds:

- `repos/<owner>/<name>`: a clone of `https://github.com/<owner>/<name>.git`, with no checkout.
  Git's own `url.<base>.insteadOf` setting can point it elsewhere, such as an SSH remote or a
  local mirror.
- `worktrees/<owner>/<name>/<task id>`: one worktree per task, on branch `priory/<task id>`, from
  the remote's default branch. The task's `worktree` names it only when Priory created it. A start
  reuses an existing worktree only when it belongs to Priory's clone and is on that branch.
- `priory.sock`: the socket hooks report to. `signals.spool`: the signals sent while no daemon
  listened, which the next daemon applies first.
- `daemon.lock`: one daemon per home. A second one waits up to 5 seconds for the lock, then refuses to start.

## Runs

A run is one task step worked by one agent's `claude` process. When a step leads to another model
step with the same agent, the process carries on into a new run, so each run has at most one result.
A model step with another agent gets a new run and a new process, as described under Moving on. The daemon starts at most one
run at a time until the runner's room check lands. It picks the oldest `queued` or `ready` task whose
step decides `model`, and finds the repository through the task's trigger.

The daemon creates the worktree, records the run and the task as `running`, then starts
`claude` in a detached tmux session named `priory-<task id>`, in the worktree:

- `--session-id`: a UUID Priory chooses and stores, so a later run can resume the conversation. A
  task keeps one session per agent: when an earlier run of the step's agent reached `SessionStart`,
  the run gets `--resume <that id>` instead, in the same worktree.
- `--model` and `--effort`: the agent's, when it sets them.
- `--append-system-prompt`: the agent body, the Markdown after its frontmatter.
- `--settings`: hooks that run `priory hook SessionStart`, `UserPromptSubmit` and `Stop`, a
  permission for the `priory` MCP server's tools, and a `PreToolUse` hook that denies
  `AskUserQuestion` and tells the agent to call `ask` instead.
- `--mcp-config`: the `priory` MCP server, `priory mcp --session <id>`, described under Tools.

The same tmux command tags the new pane with a pane option `@priory` set to the harness session
id, so a pane is never left untagged. The daemon types only into that pane, even when a person
opens other panes in the session. Each second, it ends a run whose tagged pane is gone as
`exited`, and blocks its task with the reason. A git or tmux error ends the run as `failed`, or blocks the task when no run exists yet.

tmux owns the agents, so the daemon can stop and start again without stopping them. When it starts,
it keeps every live run whose session still exists. It kills each tmux session whose tagged pane
carries the harness session of an ended run that no live run continues, as a crash between ending a
run and killing its session leaves.

### Trusting a clone

Claude asks whether to trust a folder the first time it runs there, before any hook fires. Priory
never answers that dialog and never writes Claude's settings. A worktree inherits the trust of the
repository it belongs to, so a person trusts each clone once, after Priory created it:

```sh
cd "$PRIORY_HOME/repos/<owner>/<name>" && claude   # accept the dialog, then /exit
```

A run that sends no `SessionStart` within 60 seconds of its start ends as `exited`. The daemon kills
its tmux session and blocks its task with the reason `claude showed no SessionStart within 60s;
trust the clone once: cd <clone path> && claude, accept, exit`. Its queued prompts stay queued, so
the task's next run types them.

## Signals

`priory hook <event>` reads Claude's hook JSON on stdin and sends one JSON line to the daemon over
`priory.sock`: `{"event": "Stop", "session": "<id>", "prompt": "<text>"}`. `prompt` comes only with
`UserPromptSubmit`. The daemon answers `ok` once the signal is committed, and the hook waits up to
10 seconds for it. When no daemon takes the line, it goes to `signals.spool` instead. The next
daemon applies the spool before it removes it.

- `SessionStart`: the harness is up and its input is ready, so the run is idle.
- `UserPromptSubmit`: the run is busy. Its attempted prompt is confirmed if the submitted text has
  the same words.
- `Stop`: the turn ended, so the run is idle.

The daemon never reads the terminal screen to decide that an agent is idle.

## Tools

`priory mcp --session <id>` is a stdio MCP server. It answers `initialize`, `ping`, `tools/list`
and `tools/call`, and sends each tool call to the daemon over `priory.sock` as
`{"tool": ..., "session": ..., "arguments": ...}`. The daemon applies it in one transaction and
replies with the tool's text or error. With no daemon listening, the call fails and nothing is
stored.

- `result(outcome, summary, evidence?, commit?)` stores the run's result, which nothing can change
  or delete. The outcome must be a key of the step's `next`; otherwise the error lists the valid
  outcomes. An outcome that leads to `done` needs `commit` when the task owns a pull request,
  as described under Done and checks.
- `ask(question, options, recommendation, tradeoff)` stores an ask at revision 1.

- `wait(on)` stores a wait, described under Waits.

A run has at most one of a result, an open ask and an open wait: any later call is refused.

### Moving on

When the run's turn ends, its `Stop` moves the task by its result to `next[outcome]`:

| Target     | Task                                                 | Run                       |
| ---------- | ---------------------------------------------------- | ------------------------- |
| model step, same agent | at that step, `running`, with its prompt queued | `moved`, into a new run |
| model step, other agent | at that step, `ready`, with its prompt queued | `moved`                 |
| human step | at that step, `asleep`, with an ask of its `options` | `asleep`                  |
| code step  | at that step, `blocked`: code steps are not built yet | `blocked`                 |
| `done`     | `done`, or `asleep` on its checks: see Done and checks | `done`, or `asleep`       |
| `stop`     | `dropped`                                            | `dropped`                 |

A prompt that a move or an answer queues goes ahead of the prompts already queued for the task, and
carries them after it, so an event kept during the last turn never reaches the agent before its new
instructions.

A `ready` task's next run starts the step's agent, with its own system prompt, model and settings,
and resumes that agent's session for the task, or starts a new one when the agent has not run for
it yet.

A run with an open ask or wait and no result ends `asleep`, and its task sleeps. The daemon kills a run's
tmux session when the run ends, after the hook has its reply. The worktree stays.

### Asks and answers

`priory asks` prints each open ask: id, task, revision, question and options.
`priory answer <ask> <option-or-text> --revision <n>` sends the answer to the daemon. The first
answer to the ask's revision stands. An answer to another revision, a second answer, or, for a
step's ask, a text that is not one of its options, is refused.

- A step's ask moves the task by `next[option]`, as in the table. A model step makes the task
  `ready` with that step's prompt queued.
- An agent's ask queues the answer as the task's next prompt. An asleep task becomes `ready`. A
  task whose run has not stopped yet stays `running`, and the answer is typed after its `Stop`.

A `ready` task's next run resumes the session of the current step's agent and types its queued
prompts.

### Waits

`wait(on)` names one condition in `on.type`. Each condition fires the wait on:

| `type`         | Fields                  | Fires on                                                                |
| -------------- | ----------------------- | ----------------------------------------------------------------------- |
| `pull_request` | `repo`, `pull_request`  | `github.pull_request.closed` for it, merged or not                      |
| `checks`       | `repo`, `pull_request`, optional `head_sha` | `github.check_run.completed` for it, with `pending` 0, on `head_sha` if given |
| `review`       | `repo`, `pull_request`  | `github.pull_request_review.submitted` for it                           |
| `time`         | `until`                 | the daemon's loop, once `until` has passed                              |
| `task`         | `task`                  | the daemon's loop, once that task is done, failed or dropped            |

`until` is an RFC 3339 instant, or a duration such as `30 seconds`, `5 minutes` or `2 hours`: an
SQLite date modifier, which Priory adds to the time of the call. An unknown type or a missing field
is refused, and nothing is stored. A pull request condition makes the task own the pull request; it
is refused when another active task owns it already.

The intake publishes one `github.check_run.completed` per check run, and only once it completed, so
the daemon cannot count the runs it has not seen. The intake reads every check run on the head
commit in the same poll, so each check event carries `pending`: how many of them had not completed
then. "Every check run has completed" is a check event with `pending` 0. Two runs that complete
between two polls both carry 0: the first fires the wait, and the second is kept. A check run that
GitHub has not created yet is not counted. A pull request with no check runs never fires a checks
wait. A checks wait without `head_sha` takes a check event on any head, so a late event for an older
head can fire it. More than 100 check runs and statuses on a commit keep `pending` above 0, so such a
pull request never fires a checks wait.

When the wait is stored, the daemon looks at the events the task kept since the agent's last prompt:
those not typed yet, and on which no wait fired. The first one that satisfies the wait fires it at
once, and the time and task conditions are checked at once too. The run then does not sleep: the
event is typed after its `Stop`.

A wait that fires queues one prompt ahead of the task's other queued prompts: the event's line, or
for time and task, `wait <id> fired: <until> passed` or `wait <id> fired: task <id> is <state>`, then
a line for each event the task kept while it slept. An asleep task becomes `ready`, and its next run
resumes the session of its step's agent with that prompt. An event that does not satisfy the wait so
stays kept until the wake.

A check event also carries `failing`: how many of the completed check runs on the head commit had a
conclusion other than `success`, `neutral` or `skipped` in the same poll.

`priory waits` prints each open wait: id, task, run, kind, what it waits on and when it was stored.

### Done and checks

A task owns a pull request when one of its waits named it, when an event it kept comes from a pull
request on its branch, or when a pull request event created it. A waited one comes first, then the
latest from its branch. When such a task reaches `done`, by a result or by an answer, it is not done
yet. The daemon stores a checks wait of its own on that pull request, pinned to the `commit` of the
task's latest result. The task sleeps with no process, at the step of that result, so the agent
that made it fixes what the checks find. A task whose latest result names no commit is blocked
instead.

The daemon judges this wait on every event the inbox kept or ignored about the pull request,
including events the agent already read and those an earlier task kept. It judges
when it stores the wait and at each new event:

| Condition                                                                           | Effect                                 |
| ----------------------------------------------------------------------------------- | -------------------------------------- |
| The newest event names another head and came after the result                    | wakes the agent: `wait <id> fired: the head of <pull request> moved to <sha>, away from your result's commit <commit>` |
| The newest event names another head and came before the result                   | the task sleeps on, since a later event tells whether the head is the commit |
| Else, the newest check at the commit is red, or counts `failing` and a red check at the commit was seen | wakes the agent with that red check: `wait <id> fired: check <name> at <commit> concluded <conclusion> <url>` |
| Else, the newest check at the commit has `pending` 0 and `failing` 0               | marks the task done and starts no run  |
| Otherwise                                                                           | the task sleeps on                     |

Each wake line is followed by the event's own line, unless a queued prompt already carries it. The woken agent can fix the failure and send a
new result, which stores a new checks wait at its new commit. Checks of the old commit that finish
before that result are history, so they do not count as a moved head. One that the intake reports
after the result does, since the daemon cannot tell it from a head moved back.

The `result` tool refuses an outcome that leads to `done` without `commit` when the task owns a pull
request.

Known limits:

- Priory reads no branch protection, so every check run reported for the head counts as required.
  Commit statuses are not check runs and do not count.
- A pull request whose head never gets a check run keeps the task asleep.
- A check event from an intake older than `failing` cannot mark a task done.

## Outbox

Every prompt the daemon types goes through the outbox. A prompt moves through these states:

| State       | Meaning                                                                          |
| ----------- | -------------------------------------------------------------------------------- |
| `queued`    | Waits for the task's run to be idle                                              |
| `attempted` | Recorded, with the run marked busy, in the transaction before typing             |
| `confirmed` | Its `UserPromptSubmit` arrived                                                   |
| `uncertain` | Attempted before a daemon stop, never confirmed. Priory never types it again     |

A run's first prompt is its step's `prompt`, then one line per event: the task's source event and
every event it kept that no prompt carried yet. Each line holds the type, the subject and the data.
A `ready` task's run has only those event lines, and none when there are no new events: the prompt
that moved or woke the task is already queued.

The daemon types the oldest queued prompt of a task once its run is idle. It loads the text into a
tmux buffer from stdin and pastes it as one bracketed paste, so newlines do not submit and no text
is read as a key name. Then it presses Enter. A task with an attempted or uncertain prompt gets no
further prompt, since text may still sit in the agent's input.

## Running as services

`contrib/systemd/` holds two user units: `priory-daemon.service` runs `priory daemon` and
`priory-intake.service` runs `priory intake`. Both restart on failure after 5 seconds. They share one
environment file, so a person sets the factory folder and the state paths once.

Install the binary where the units look for it, `~/.cargo/bin/priory`, then the units and the file:

```sh
cargo install --path crates/priory
mkdir -p ~/.config/systemd/user ~/.config/priory
cp contrib/systemd/*.service ~/.config/systemd/user/
$EDITOR ~/.config/priory/env
systemctl --user daemon-reload
```

`~/.config/priory/env` holds one `NAME=value` per line, with no quotes and no `export`:

```sh
PRIORY_FACTORY=/home/me/factory
PRIORY_DB=/home/me/.local/state/priory-oss/priory.db
HERDR_BUS_DB=/home/me/.local/state/priory-oss/bus.db
PATH=/home/me/.local/bin:/usr/local/bin:/usr/bin:/bin
```

`PRIORY_HOME` is optional and defaults as under Priory home. A user service does not read your shell
profile, so `PATH` must reach `claude`, `git`, `gh` and `tmux`. Systemd does not create the parent
folders of `PRIORY_DB` and `HERDR_BUS_DB`; create them first.

| Action        | Command                                                         |
| ------------- | --------------------------------------------------------------- |
| Start now and at login | `systemctl --user enable --now priory-intake priory-daemon` |
| Status        | `systemctl --user status priory-intake priory-daemon`           |
| Logs          | `journalctl --user -u priory-daemon -u priory-intake -f`        |
| Restart       | `systemctl --user restart priory-daemon`                        |
| Stop          | `systemctl --user stop priory-daemon priory-intake`             |

Without lingering, user services stop when your last session ends. On an always-on machine, run
this once, which changes the machine and so is yours to run:

```sh
loginctl enable-linger "$USER"
```

The daemon unit sets `KillMode=process`. With systemd's default, `control-group`, stopping or
restarting the unit would also kill the tmux server the daemon started, since tmux stays in the
unit's cgroup, and with it every agent. `process` signals only the daemon, so the agents keep
running and the new daemon reconnects to them. Systemd logs `Unit process ... (tmux: server) remains
running after unit stopped`, which is expected. `systemctl --user stop` therefore leaves agents
running; end one with `tmux kill-session -t =priory-<task id>` when you mean to.

## Listings

`priory inbox` prints each entry, oldest first: seq, type, subject, task and outcome.
`priory tasks` prints each task: id, key, workflow, step, state, band, lane and reason.
`priory runs` prints each run: id, task, step, tmux session, harness session, started, ended and
how it ended. `priory outbox` prints each prompt: id, task, run, state and text, with newlines
shown as `\n`. `priory asks` and `priory waits` are described under Tools. They read `PRIORY_DB` and print `-` for an empty cell, tab-separated after a header
line.
