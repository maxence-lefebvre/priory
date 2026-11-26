# Kept sessions

This page explains how an agent could stay open after its step sleeps, so a person can type to it
and a wake reaches the same process. It also covers a person's own Claude Code session started and
kept by Priory, which is the same feature. It builds nothing. It holds the decisions with the
options each one rejects, the first slice, the slices after it, and the open questions with a
recommendation for each.

Two requests start it:

1. An agent such as the morning bonjour stays in Herdr after its report, idle, so a person can pick
   up the work at the keyboard, and a pick still reaches it.
2. A person's own session runs as a child of Priory, for example through `priory session`, instead
   of in a terminal Priory does not own. Priory then owns the pane and knows when the prompt is
   idle, so it can compact on demand. The session survives daemon upgrades and reboots, counts in
   the agents slice and the costs, and gets asks, says and memory grants without the plugin's hooks.

## What existed when this note was written

- A model step that sleeps on an ask or a wait ends its run. `tools::sleep` ends the run as
  `asleep`, sets the task `asleep` and returns the tmux session
  (`crates/priory-core/src/tools.rs:3214`). The daemon kills that session once it replied to the
  hook (`crates/priory-core/src/daemon.rs:1183`). See [waits.md](../daemon/waits.md).
- A wake makes an asleep task `ready`, and its next run resumes the step agent's session with
  `--resume` and types the queued prompt (`crates/priory-runtime/src/runtime.rs:1529`,
  [waits.md](../daemon/waits.md)). A prompt for a run that has not stopped yet is typed after its
  `Stop`, through the [outbox](../daemon/outbox.md).
- So each bonjour task takes two runs and two tmux sessions: the report, then a new run on a pick
  or at the 18:30 wait. On a desktop, tasks 512 and 920 each have 2 runs, and today's task 1256 is
  asleep after 1 run with no live session.
- A run already keeps its process across steps in one case: a result that moves into a model step
  of the same agent, not irreversible and not a debate, carries the run on and moves its keyboard
  grant to the new run (`crates/priory-core/src/tools.rs:3990`, `tools.rs:4016`).
- `[runs] attach` opens each run of an `attach = true` step through `adapters/herdr/attach`, one
  kept hc terminal per run, always read-only (`adapters/herdr/attach:50`). Nothing closes it: once
  the run's tmux session is killed, the terminal has nothing left to show
  ([sessions.md](../daemon/sessions.md#opening-a-run-when-it-starts)). The Priory bar's attach is
  read-write unless a panel asks for read-only ([herdr.md](../adapters/herdr.md)).
- With `[serve] terminal = "type"`, the web view's Terminal takes a 15 minute keyboard grant. While
  a grant is open, the outbox types nothing into that run
  (`crates/priory-core/src/keyboard.rs:134`, [serve.md](../daemon/serve.md#a-runs-keyboard)). A
  read-write tmux client takes no grant, so the outbox cannot see it.
- Room, the agents slice and lane or trigger caps count live runs only. An asleep task counts for
  nothing ([runs.md](../daemon/runs.md), [room.md](../daemon/room.md),
  [agents-slice.md](../daemon/agents-slice.md)). The `day` and `week` spend caps hold new runs and
  never stop a running one ([spend.md](../daemon/spend.md#new-runs-wait)). A runner filter's days
  and hours decide which tasks a runner takes ([runners.md](../daemon/runners.md)).
- A live run and its tmux session survive a daemon restart. A run whose pane is gone when the
  daemon starts, as after a reboot, ends `lost`, and its task's next run resumes the session
  ([zero-downtime-restarts.md](zero-downtime-restarts.md), [runs.md](../daemon/runs.md)).
- `idleCompactAt` compacts a shared session idle for 55 minutes by resuming it in
  `priory-idle-<agent>`, typing `/compact` after `SessionStart`, and killing it at the
  `SessionStart` with `source: compact` (`crates/priory-core/src/daemon.rs:3406`,
  [sessions.md](../daemon/sessions.md#compacting-an-idle-session)).
- #455, closed, let a person's own session ask for a compaction through hc. It typed into a
  terminal Priory does not own: the terminal check and `hc send` are two steps, and hc reports an
  idle agent as `idle` or `done` without proving the prompt box is empty.

## The model

### A kept session is a task

| Option | What it gives | What it costs |
| ------ | ------------- | ------------- |
| **A task.** A kept session is a task whose step keeps its run. A person's own session is a task of a `session` workflow with one model step | The Desk, Costs, `priory tasks`, asks, says, waits, memory grants, the trace and the queue all work as they do, since each one keys on a task | A person's session shows in task listings, so it needs a lane, such as `none/session`, and a title |
| A run with no task | A person's session stays out of task listings | Every reader of runs assumes a task: the outbox, spend caps, the room, the trace, asks and `say`. Each would need a second path |

Recommend a task. The second request then becomes a workflow, not a new kind of thing.

### A step opts in with `keep`

| Option | Why |
| ------ | --- |
| **A step key, `keep = true`** | The step is where a run sleeps, and where `attach` already lives. Bonjour keeps its report step, and its other steps end as today |
| A workflow key | Too coarse: a workflow with a long review step would keep every step |
| An agent key | One agent serves several workflows and steps, and a keep belongs to only some of them |

`keep` needs a model step, so `priory check` refuses it on a code step, a human step or a debate
step. It does not need `attach`: a kept session without a terminal still saves the resume.

### What asleep means for a kept session

When a kept step's turn ends on an ask or a wait, the task is `asleep` as today, and the run stays
live with a new end reason held open: the run is `kept`. Its process and tmux session stay.

- Waits and asks still fire as today. A wake queues its prompt in the outbox, and the outbox types
  it into the live pane at its next idle point, instead of making the task `ready` for a new run.
  The task goes `asleep` to `running` without a start, so the queue, `max_running` and the room do
  not hold it.
- A person's turn typed into a kept session does not move the task. Its `Stop` with no result
  leaves the task asleep, and sends no nudge. A result the agent stores in that turn moves the task
  as usual.
- A step change ends the kept run as today, unless the next step carries the run on.

| Option for the wake | Why rejected |
| ------------------- | ------------ |
| A new run that resumes the session, as today | That is the behaviour to change: the terminal goes blank and the process restarts |
| Keep the pane, but start a new run record in it at the wake | Two runs on one process split its tokens and its keyboard grant for no gain. The carried run already shows one run can span a sleep |

### Who holds the keyboard

The keyboard grant becomes the one lock on a kept session's input, for the web view and Herdr.

- `adapters/herdr/attach` keeps opening a read-only terminal, one per kept run, and reuses it
  across wakes, since the run and its label stay the same.
- To type, a person takes the keyboard. Herdr's bar takes a grant through the same endpoint as the
  web view, then opens a read-write attach. Detaching or the grant's end returns it. A grant on a
  kept session renews while its client stays attached, instead of ending at 15 minutes.
- While a grant is open, the outbox waits, as it does today. A wake or a `say` queues and is typed
  at the first idle point after the grant ends. The desk shows `waits for the keyboard` on it.
- `priory attach` without `--read-only` on a kept run takes a grant too, so a tmux client that can
  type is always visible to the outbox.

| Option | Why rejected |
| ------ | ------------ |
| Read-write Herdr terminals with no grant | The outbox cannot tell a person is mid-sentence, so a wake lands in their prompt box |
| Priory always wins, typing into a held session | A person's half-typed line would be submitted with the wake |
| A person always wins, dropping wakes while they type | Loses picks and wait firings, which the outbox exists to keep |

### Room, caps and spend

A kept idle session costs memory and no tokens. Count what it uses, and bound how many stay.

- **Room.** A kept run is a live run, so the room subtracts its unused memory promise and its
  scope's real use, as for any run. An idle Claude process holds a few hundred megabytes, so the
  promise is replaced by the measured use once the run is kept.
- **Agents slice.** It stays in its scope in the slice, so `CPUQuota` and `MemoryHigh` cover it.
  It uses almost no CPU while idle.
- **Lane and trigger caps, the review limit.** A kept run does not count toward `max_running`,
  since it holds no work. It counts again from a wake to its next sleep.
- **Spend caps.** A kept session spends only on turns, and every turn counts toward the caps as
  today. A wake into a kept session is not a new run, so it checks the `day` and `week` caps before
  the outbox types it, and waits with the same reason when one is reached.
- **Runner filters.** A filter's hours hold new starts only. A kept session whose task a filter
  excludes is stopped when the window opens, and resumed by the next wake as today.
- **A bound.** `[keep] max` in `factory.toml`, 2 by default, caps kept runs per runner. Past it,
  and whenever the room is short of a start's need, the daemon stops the kept run idle the longest:
  it compacts it if its context is past `idleCompactAt`, then kills it. The task stays asleep, and
  its next wake resumes the session in a new run, as today. `[keep] idle`, 8 hours by default,
  stops a kept session idle that long the same way.

### Daemon restarts, upgrades and reboots

A kept run is a live run, so it needs nothing new to survive a restart or an upgrade: the tmux
session outlives the daemon, hooks during the gap are spooled, and the new daemon adopts the run
([zero-downtime-restarts.md](zero-downtime-restarts.md)). The one change is that adoption keeps the
`kept` state, so a restarted daemon does not nudge an idle kept run as stalled.

A reboot kills the tmux server. A kept run whose pane is gone at the daemon's start ends `lost` as
today, but leaves its task `asleep` with no prompt, instead of `ready`. Its next wake resumes the
session. For a person's own session, the daemon restarts it at once with `--resume`, so it is back
in Herdr after a reboot. Its Herdr terminal opens again through `[runs] attach`.

### Compaction on demand

A kept session can compact on demand: the `compact` tool, or a person's `/priory:compact`, queues
`/compact <focus>` in the outbox, and the outbox types it at the next idle point with no grant
open. It is safe here and was not in #455 for three reasons:

1. Priory owns the pane. It resolves it from the run's tag and types by pane id, so no other
   program can receive the text.
2. Priory knows the prompt is idle. The run's `Stop` and `UserPromptSubmit` hooks mark it, and an
   open grant marks a person at the keyboard. hc could only report `idle` or `done`.
3. The outbox stores the prompt before it types, confirms it on `UserPromptSubmit`, and never types
   it twice across a restart.

The idle compaction of `idleCompactAt` then compacts a kept session in place, without the
`priory-idle-<agent>` resume, since the process is already live and its cache is warm.

### Herdr

`adapters/herdr/attach` keeps one terminal per kept run. Since the run spans its sleeps, the
terminal keeps showing the same agent: its last answer and its empty prompt while idle. Priory
does not draw into the pane. The Priory bar shows the kept state on the task's tab instead: `kept,
idle 2h 10m, wakes at 18:30` from the task's open waits, and `you hold the keyboard` during a
grant. When the run ends, the terminal shows tmux's exit, as today.

## The first slice: bonjour's step stays alive after its report

- `keep = true` on a model step, with its `priory check` rules.
- `tools::sleep` keeps the run and its tmux session for a kept step, and the daemon kills nothing.
- A wake into a kept run queues its prompt for that run and sets the task `running` without a
  start. A turn end with no result in a kept run leaves the task asleep.
- `[keep] max` with its stop of the longest idle kept run.

Tests, at the seam the daemon uses: a kept step's sleep leaves the run live and its tmux session
alive; a time wait that fires types the prompt into the same run, and `priory runs` shows one run
for the task; a third kept run stops the oldest. Measured on a desktop's bonjour: 2 runs per task
before, 1 after, and its Herdr terminal shows the agent at 18:30.

## Later slices

1. The keyboard grant as the one lock: Herdr's bar takes a grant before a read-write attach, a
   grant renews while attached, `priory attach` takes one, and the desk shows `waits for the
   keyboard`.
2. Room and caps: the measured use replaces a kept run's promise, `max_running` skips kept runs,
   wakes check the spend caps, runner filters stop excluded kept runs, and `[keep] idle`.
3. Compaction on demand through the outbox for kept runs, and `idleCompactAt` in place.
4. Reboots: a lost kept run leaves its task asleep, and adoption after a restart keeps `kept`.
5. `priory session [--agent NAME] [TITLE]`: a task of a built-in `session` workflow whose one step
   keeps its run, opens a read-write attach with a grant, and restarts after a reboot.
6. The Priory bar's kept line in Herdr.

## Open questions

- **Does `[keep] max` count a person's own sessions?** Recommend no: a person starts those by hand,
  so a separate `[keep] sessions` cap, 4 by default, keeps an agent's kept runs from stopping them.
- **What does `keep` do for a Codex agent?** Codex sends no `SessionStart` on resume, so idle
  compaction cannot work. Recommend allowing `keep` and refusing only compaction, as
  `idleCompactAt` does.
- **Should a person's turn in a kept session count as the task's spend?** Recommend yes, toward
  the same caps: the tokens are real, and a separate account would hide them from Costs.
