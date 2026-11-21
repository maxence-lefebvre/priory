# Sessions

## Shared sessions

An agent's `session` is `per-task` (the default) or `shared`. A `per-task` agent gets one session
per task, as above. A `shared` agent, such as a lead, keeps one conversation across every task it
serves. The state stores it per agent. After a run reaches `SessionStart`, every later run of the
agent, for any task, resumes it with `--resume`. Until then, a new run starts a new session and records
it as the agent's.

A session runs one turn at a time, so a shared agent has at most one live run. A task needing the
agent while another task's run of it is live stays `queued` or `ready`, with the reason
`waits for agent <name>: its shared session serves task <id>`, which `priory tasks` shows. It starts
when that run ends by its result, ask, wait or a verdict. Then the room is checked, and a task
behind it that needs another agent can start ahead of it.

Each task keeps its own worktree and branch, and the resumed session starts in the task's worktree.
One worktree for the agent would put every task's commits on one branch and leave the sweep no task
to judge by. Each new run of a shared agent starts its task's first queued prompt with
`This is task <id> (<key>), from trigger <trigger>, in the worktree <path>. Earlier turns of this
conversation may belong to other tasks.` That lets the conversation tell its tasks apart, even after
a crash cut a prompt short.

A session an agent once shared stays shared. When the factory turns the agent back to `per-task`,
each task starts a new session instead of resuming the shared one.

Known limits:

- A long conversation is compacted by Priory only when its agent sets `compactAt` or calls the
  `compact` tool. See Compacting a long session.
- Claude stores a session's transcript under its starting directory. Whether `--resume` finds it
  from another task's worktree depends on the Claude version. No test runs a real Claude.
- Signals and tool calls name the harness session, not the process. The daemon kills a run's tmux
  session before admitting the next task, but a process that survived the kill could still report
  for the run that resumed its session.

## Compacting a long session

An agent's `compactAt` is a context size in tokens, such as `150000`. When a turn ends with the
conversation's context at or past it, the daemon queues a `/compact`, as the `compact` tool does. It
types it once the run is idle, then the same continuation line. [outbox.md](outbox.md) describes
that path under Compact. Unset, Priory never compacts on its own.

The context size is a count, not a fraction of the model's window: Priory does not know each
model's window.

The daemon reads the size at the turn's `Stop` hook, from the session transcript that the hook names.
It is the last assistant message's input, cached input, cache writes and output. So the size lags by
one turn: a turn that passes `compactAt` is compacted after it ends, never during it. Claude can
still compact a turn itself if it fills the window first.

It compacts once per crossing. After the daemon queues a `/compact`, it queues no other until a
turn's context reads below `compactAt` again. A compaction that leaves the context above it is not
repeated. The run keeps that state, so a new run of a shared session can compact once more.

A turn that stores a result is not compacted, since the result may move the task to another
agent's conversation. The next turn of the same conversation still sees the crossing.

A Codex agent cannot take `compactAt`: Priory reads no Codex tokens (see [codex.md](codex.md)).

## Attaching to a run

`priory attach TASK` attaches your terminal to the tmux session of the task's live run, so you see
the agent's screen. `priory attach --run RUN` names one run instead. It reads the state file, finds
the run's tmux session on the tmux server your `TMUX` and `TMUX_TMPDIR` name, and checks that a live
pane there carries the run's `@priory` tag. Then it runs
`tmux -S <socket> attach-session -t '=priory-<task id>'` in your terminal, with that server's
socket. Detach with tmux's
detach key, `C-b d` by default. tmux keeps owning the session, so the agent goes on after you
detach.

- `--read-only` adds tmux's `-r`: you watch without typing into the agent, and your terminal's size
  leaves the agent's window alone.
- `--print` prints the attach command instead of running it, so a view host can run it in a pane,
  such as `hc run -- <that command>` in a Herdr pane.

A task with no live run, a run that ended, or an unknown id exits with 1 and names the state, such
as `priory attach: task 1 is dropped, with no live run` or `run 3 ended as cancelled`. A session
missing from your tmux server, or one whose panes carry no live tag of the run, exits with 1 and
asks you to run attach with the daemon's `TMUX` and `TMUX_TMPDIR`, so you never attach to another
server's session of the same name.

The daemon sets a run's idle flag and turn from hooks, tool calls and its own typing of a prompt.
It never reads tmux clients, so attaching and detaching change no run's idle flag, turn or prompt. A client that can type can still type into the
agent, as a person at the pane would: the daemon then sees that turn's hooks like any other.

Known limits:

- tmux refuses to attach from inside another tmux client unless you unset `TMUX` first.
- An attach without `--read-only` resizes the agent's window to your terminal.

## Trusting a clone

Claude asks whether to trust a folder on its first run there, before any hook fires. Priory never
answers that dialog or writes Claude's settings. A worktree inherits trust from its repository, so a
person trusts each clone once, after Priory created it:

```sh
cd "$PRIORY_HOME/repos/<owner>/<name>" && claude   # accept the dialog, then /exit
```

A run that sends no `SessionStart` within 60 seconds of its start ends as `exited`. The daemon kills
its tmux session and blocks its task with the reason `claude showed no SessionStart within 60s;
trust the clone once: cd <clone path> && claude, accept, exit`, then `. Its pane ended with: ` and
the last 5 non-empty lines of its pane, joined by ` | `, so a person sees the dialog or warning that
stopped it. The daemon log gets the last 30. Its queued prompts stay queued. A prompt it was launched
with returns to the queue, so the task's next run gets them. A Codex run blocks with the Codex
instruction under Codex.
