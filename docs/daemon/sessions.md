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

- A long shared conversation is never compacted by Priory.
- Claude stores a session's transcript under its starting directory. Whether `--resume` finds it
  from another task's worktree depends on the Claude version. No test runs a real Claude.
- Signals and tool calls name the harness session, not the process. The daemon kills a run's tmux
  session before admitting the next task, but a process that survived the kill could still report
  for the run that resumed its session.

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
