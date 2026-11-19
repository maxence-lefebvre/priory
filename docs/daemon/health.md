# Health

Each second, the daemon gives every live run one of these verdicts, in this order:

| Verdict | Detected by | Run ends | Task | Reason |
| ------- | ----------- | -------- | ---- | ------ |
| Unexpected exit | the tagged pane is gone, or dead under `remain-on-exit`, and no result, ask or wait ended the turn | `exited` | `failed`, then repaired: see Repair | `run <id> exited: claude no longer runs in tmux session <name>` |
| No `SessionStart` | 60 seconds with no `SessionStart`, as under Trusting a clone | `exited` | `blocked` | `claude showed no SessionStart within 60s; ...` |
| Permission prompt | a `PermissionPrompt` signal | `permission` | `blocked` | `run <id> stopped on a permission prompt: <message>` |
| Timeout | a turn older than its step's `timeout` | `timeout` | `blocked` | `run <id> timed out: its turn ran past <n>s` |

A verdict blocks the task when a person must act: the trust dialog, a permission the agent lacks,
or a turn that may be legitimately long. A person reads the reason on the desk, then resumes or
cancels the task. An unexpected exit fails the task instead, since the daemon can often tell why
the agent died and act on it (see Repair). An exit before `SessionStart` still blocks the task,
because a harness that never came up dies the same way when restarted. Such a run never took the
prompt it was launched with, even one a daemon restart made `uncertain`, so that prompt is queued
again for the next run. The daemon commits the verdict, then kills the tmux session. A crash in
between leaves an ended run, which the next start cleans up.

## Failed or blocked

These ends make a task `failed`:

- An unexpected exit after the run's `SessionStart`, whatever its cause. Its repair decides whether
  it stays failed.
- A second unexpected exit of the run its repair restarted, before that run stored a result, ask or
  wait.

Everything else that stops a task needs a person and makes it `blocked`: no `SessionStart`, an exit
before `SessionStart`, a permission prompt, a timeout, a run lost twice, a code step's action that
is not built, a step with no known agent, a task with no repo, and a git or tmux error at the
start of a run. `open-pr`'s own failure is not an end. It is the `failed` outcome, which the
workflow routes like any other.

## Repair

When an unexpected exit fails a task, the daemon makes one repair attempt. It classifies the cause
from the room, measured as for admission, after the run's promise is released:

| Cause | Found when | Action | Restarts when |
| ----- | ---------- | ------ | ------------- |
| `out of memory (<room>)` | the room has no memory for the agent | `wait for room` | the room check admits the run again |
| `disk full (<room>)` | the room has no disk | `sweep`: a sweep runs at once, and the room is measured again | the disk is no longer short |
| `the harness crashed` | the room has space | `restart` | at once |

A cause that is gone, or that the room check will confirm gone before the run starts, makes the task
`ready` with the prompt `Priory repaired this task after: <reason>. Cause: <cause>. Action:
<action>. Check the worktree, then continue.` ahead of its other prompts. The next run resumes the
step's agent session, as a resume does. A disk still short after the sweep leaves the task `failed`
with the reason `<reason>. Cause: <cause>. Action: sweep. The cause persists, so Priory did not
restart the agent.`

A task gets one repair per run. When the run a repair restarted exits too, before it stored a
result, ask or wait, the task stays `failed` with `<reason>; it failed again after its repair, so
Priory did not restart the agent`. Once a run stores a result, ask or wait, a later exit gets a
repair again, and so does the first exit after a revive. `priory revive` restarts a failed task
either way.

Each repair appends a `factory.task.repaired` record to the trace, with its cause, action and
outcome, `ready` or `failed`, as [the trace schema](../trace-schema.md) defines it.

Known limits:

- The cause is read after the agent died, when its memory is already free. An agent the kernel
  killed for memory often reads as `the harness crashed` and still restarts once.
- A harness that crashes for a reason the room cannot see, such as a bad binary after its first
  turn, gets one restart before it stays failed.
- Waiting for room has no deadline. A task waits as any queued task does.
- The daemon ends the run, then repairs in a second transaction, since the sweep runs outside any
  transaction. A daemon that stops in between leaves the task `blocked` with the exit's reason, as
  before repair existed.
- `ready` means the task waits to start its agent, not that the agent started. A cancel or a room
  that never frees can still stop it.
- A sweeper task never waits for room, so its memory repair starts it at once.
- A repair's run lost with its tmux server is replaced by a run that gets a repair of its own.
- A room that cannot be measured confirms no freed disk. A crash with an unreadable room reads as
  `the harness crashed`, and admission then waits until the room reads again.

A run whose agent is gone after its result, ask or wait ended the turn is not an unexpected exit.
It settles as its `Stop` would, as under Moving on, but a next model step always gets a new run,
since no process carries on.

A model step's `timeout` is a duration such as `90s`, `15m` or `2h`, and defaults to `3h`. A run
whose agent exits after a wait already fired, or after its ask was answered, but before its `Stop`,
counts as an unexpected exit; `priory resume` recovers it. The timeout bounds one turn, from
`UserPromptSubmit` to `Stop`, not the step.

Priory never approves a permission prompt. Each run's settings carry a `PermissionRequest` hook
that denies every request and tells the agent: `Priory runs this session unattended and never
approves a permission prompt, so it denied this one. Go on without it. If the task cannot go on
without it, call the ask or result tool of the priory MCP server.` What the agent does next is its
own. The desk hears of it only through that ask, or a verdict if the run then stalls. A
`Notification` hook with the matcher `permission_prompt` runs `priory hook PermissionPrompt`, so a
prompt that shows anyway, when the deny hook failed, ends the run on the next tick.

## Stalls and the handler

A live run is stalled when it sent no hook event and its pane did not change for 10 minutes, or for
the `stalled` duration of `factory.toml`. Each tick, the daemon reads the screen of every live run
that reached `SessionStart`, shows no permission prompt and has no open ask or wait. Runs asleep on
a wait or an ask never count, nor do runs whose spinner or output still moves.

The handler is opt-in. `handler` in `factory.toml` names the agent whose model it uses, which needs
`harness = "claude-code"`. Without it, no handler runs, as no sweeper runs without `[sweep] agent`.
On a stall, the daemon makes one headless `claude -p --output-format json` call in
`PRIORY_HOME/handlers`, with no settings or MCP servers of the user's and no session kept. The model is the agent's,
or `haiku` when the agent sets none.

```toml
handler = "handler"   # an agent with harness = "claude-code"; no handler runs when unset
stalled = "10m"
```

The handler reads the step's prompt, the last three prompts the task received (not counting
`withdrawn` ones), the task's last three trace records and the last 30 screen lines. It answers one of:

- `nudge` with a text. Priory queues `Priory's handler read your screen after this run stalled:
  <text>` ahead of the task's other prompts and marks the run idle, since the handler read on its
  screen that it waits for input. The outbox types it on the next tick. It counts as a nudge, so a
  silent turn after it gets no Stop nudge.
- `restart` with a reason. Priory ends the run as `restarted`, kills its tmux session and makes the
  task `ready` with the prompt `Priory's handler restarted this run after it stalled: <why>. Check
  the worktree, then continue.` The next run resumes the same session in the same worktree.
- `leave` with a reason, which Priory records and acts on no further.

The call runs in the background, in its own process group, so the daemon keeps ticking. It is
killed after 2 minutes. Only a call that exits 0 with a successful result may act. Any other outcome
(a failure, a timeout or an unknown action) counts as `leave`, with the error as its reason, and
keeps the tokens it reported.

Before it applies an answer, the daemon reads the screen again. It records the answer with
`(not applied: the run moved on)` when, since the call, the run ended, sent a hook event, changed its
pane, stored a result, ask or wait, or was typed a prompt. A run whose earlier prompt is still
`attempted` or `uncertain` cannot take a nudge. The handler's prompt says so, and a nudge it still
answers is recorded with `(not applied: an earlier prompt still waits for its confirmation)`.

A run gets one handler call per stall. A stall ends at the next hook event or pane change. Each call
stores the screen it read, so a restarted daemon does not call again on that screen until a hook
event arrives or the pane shows something else. A run gets at most 3 calls, after which the step's
`timeout` and the other verdicts above still apply. The handler never answers an ask or a permission
prompt. Such runs are not read, and its prompt tells it to answer `leave` on a question or a menu.
Priory cannot check the text of a nudge.

Each answered call appends a `factory.handler.called` record to the trace, with the action, its text, the model,
the tokens and Claude's own `cost_usd`. A daemon that stops mid-call kills that call's process group
when it starts again, after checking its start time in `/proc`. It then records the call's answer, if
the call wrote one, as not applied, or `leave` with `the handler call failed: interrupted by a daemon
restart`. The call still counts.

Known limits:

- Only Claude Code runs the handler. A Codex agent can be stalled and handled, but not be the handler.
- A nudge waits in the outbox until the next tick types it. A run that stores a result in that second
  carries the nudge into its next step's prompt.
