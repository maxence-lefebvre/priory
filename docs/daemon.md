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

A message names a task key: a task request its `key`, a GitHub event `github:{repo}#{issue}`. The
active task that owns that key, if any, decides the route. A task is active in any state except
done, failed and dropped.

| Message                                   | Outcome   | Task                                         |
| ----------------------------------------- | --------- | -------------------------------------------- |
| Any, with an active task owning its key   | `kept`    | that task                                    |
| `factory.task.requested`, no active task  | `created` | a new task, `queued` at its workflow's start |
| `factory.task.requested`, malformed       | `ignored` | none                                         |
| Any other                                 | `ignored` | none                                         |

A task request for a workflow the factory lacks stops the daemon with an error, before its ack. The
request waits on the bus until the workflow is back. A malformed request is ignored instead, so that
it cannot block the output.

A source event comes before its task request, so it is recorded as `ignored`. The task keeps its
id in `source`.

A message kept for a `running` task also gets an outbox prompt, so the agent reads it after its
current turn. A message kept for a task in any other state waits for the task's first run.

## Priory home

`PRIORY_HOME` names Priory's own directory. It defaults to `$XDG_STATE_HOME/priory`, then
`~/.local/state/priory`. It holds:

- `repos/<owner>/<name>`: a clone of `https://github.com/<owner>/<name>.git`, with no checkout.
  Git's own `url.<base>.insteadOf` setting can point it elsewhere, such as an SSH remote or a
  local mirror.
- `worktrees/<owner>/<name>/<task id>`: one worktree per task, on branch `priory/<task id>`, from
  the remote's default branch. The task's `worktree` names it only when Priory created it. A start
  reuses an existing worktree only when it belongs to Priory's clone and is on that branch.
- `priory.sock`: the socket hooks report to. `signals.spool`: the signals sent while no daemon
  listened, which the next daemon applies first.
- `daemon.lock`: one daemon per home. A second one refuses to start.

## Runs

A run is one `claude` process working on one task step. The daemon starts at most one run at a time
until the runner's room check lands. It picks the oldest `queued` task whose step decides `model`,
and finds the repository through the task's trigger.

The daemon creates the worktree, records the run and the task as `running`, then starts
`claude` in a detached tmux session named `priory-<task id>`, in the worktree:

- `--session-id`: a UUID Priory chooses and stores, so a later run can resume the conversation.
- `--model` and `--effort`: the agent's, when it sets them.
- `--append-system-prompt`: the agent body, the Markdown after its frontmatter.
- `--settings`: hooks that run `priory hook SessionStart`, `UserPromptSubmit` and `Stop`.

The same tmux command tags the new pane with a pane option `@priory` set to the harness session
id, so a pane is never left untagged. The daemon types only into that pane, even when a person
opens other panes in the session. Each second, it ends a run whose tagged pane is gone as
`exited`, and blocks its task with the reason. A git or tmux error ends the run as `failed`, or blocks the task when no run exists yet.

tmux owns the agents, so the daemon can stop and start again without stopping them. When it starts,
it keeps every live run whose session still exists.

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

The daemon types the oldest queued prompt of a task once its run is idle. It loads the text into a
tmux buffer from stdin and pastes it as one bracketed paste, so newlines do not submit and no text
is read as a key name. Then it presses Enter. A task with an attempted or uncertain prompt gets no
further prompt, since text may still sit in the agent's input.

## Listings

`priory inbox` prints each entry, oldest first: seq, type, subject, task and outcome.
`priory tasks` prints each task: id, key, workflow, step, state, band, lane and reason.
`priory runs` prints each run: id, task, step, tmux session, harness session, started, ended and
how it ended. `priory outbox` prints each prompt: id, task, run, state and text, with newlines
shown as `\n`. They read `PRIORY_DB` and print `-` for an empty cell, tab-separated after a header
line.
