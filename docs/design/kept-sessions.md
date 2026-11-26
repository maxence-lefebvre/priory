# Kept runs

This page explains how an agent could stay open after its step sleeps, so a person can type to it
and a wake reaches the same process. It also covers a person's own Claude Code session started and
kept by Priory, which is the same feature. It builds nothing. It holds the decisions, each with the
options it rejects, the first slice and the slices after it. A person approved every decision and
the slice cut on 2026-11-26.

A **kept run** is a run that ended asleep while Priory keeps its agent process and tmux session.
The run that its task's next wake starts uses that same process.

Two requests start it:

1. An agent such as the morning bonjour stays in Herdr after its report, idle, so a person can pick
   up the work at the keyboard, and a pick still reaches it.
2. A person's own session runs as a child of Priory, for example through `priory session`, instead
   of in a terminal Priory does not own. Priory then owns the pane and knows when the prompt is
   idle, so it can compact on demand. The session survives daemon upgrades and reboots, counts in
   the agents slice and the costs, and gets asks, says and memory grants without the plugin's hooks.

## What existed when this note was written

- Eight places end a model run asleep, each with `end(tx, run, "asleep")` and a tmux session to
  kill:
  - `settle` (`crates/priory-core/src/tools.rs:3288`), when a turn ends: the ordinary `Stop` with
    an open ask, wait or consult (`tools.rs:3405`), an answer whose command still runs
    (`tools.rs:3384`), and a plan gate's ask (`tools.rs:3350`). Bonjour sleeps on the first, a
    time wait.
  - `tools::sleep` (`tools.rs:3214`), reached only from `gates::park` (`gates.rs:316`), when a
    result waits on a person's ask about a gate.
  - `step_in`, through `finish("asleep")`, when a result moves the task into a wait: a closed pull
    request of a stack (`tools.rs:3920`), the checks of a pull request (`tools.rs:3951`), and a
    human step's ask (`tools.rs:4084`).
  - `debate.rs:603`, when a debate ends on an ask.
- The daemon kills the returned tmux session once it replied to the hook. See
  [waits.md](../daemon/waits.md).
- A wake makes an asleep task `ready`, and its next run resumes the step agent's session with
  `--resume` and types the queued prompt (`crates/priory-runtime/src/runtime.rs:1529`). A prompt
  for a run that has not stopped yet is typed after its `Stop`, through the
  [outbox](../daemon/outbox.md).
- So each bonjour task takes two runs and two tmux sessions: the report, then a new run on a pick
  or at the 18:30 wait. On a desktop, every bonjour task that reached its evening wait has 2 runs.
- A run already keeps its process across steps in one case: a result that moves into a model step
  of the same agent, not irreversible and not a debate. `step_in` ends the old run `moved`,
  inserts a new run row on the same tmux session and harness session, and moves the run's
  unconfirmed outbox prompts and its keyboard grant to the new row (`tools.rs:3998-4016`).
- "A live run" means a row of `runs` with `ended IS NULL`. About 120 queries in 36 files of
  `priory-core` read it that way. Among them: `running_in` for `max_running`
  (`state.rs:4134`), the preemption candidates (`preempt.rs:112`), the spend caps' candidates
  (`spend.rs:415`), the sweep's `UNUSED` (`sweep.rs:1436`), idle compaction (`idle.rs:33`), the
  room's measured use of live runs (`state.rs:4255`), a task's cancel (`tools.rs:5044`), and
  `priory attach` (`state.rs:3742`).
- At start, the daemon kills every tagged tmux session that belongs to an ended run and to no live
  run (`state::ended_sessions`, `state.rs:5276`, called at `daemon.rs:308`). It then ends as
  `lost` each live run whose pane is gone (`daemon.rs:322`).
- `[runs] attach` opens each run of an `attach = true` step through `adapters/herdr/attach`, one
  kept hc terminal per run, always read-only (`adapters/herdr/attach:50`). The Priory bar's attach
  is read-write unless a panel asks for read-only ([herdr.md](../adapters/herdr.md)).
- A keyboard grant exists only with `[serve] terminal = "type"`: `keyboard::typing` refuses every
  other setting (`crates/priory-core/src/keyboard.rs:125`). A grant lasts 15 minutes, and while
  one is open the outbox types nothing into that run
  ([serve.md](../daemon/serve.md#a-runs-keyboard)). A read-write tmux client takes no grant, so the
  outbox cannot see it.
- The room marks runs to stop when the queue's head does not fit, and a mark stops a run at its
  turn's end ([room.md](../daemon/room.md)). The `day` and `week` spend caps hold new runs and
  never stop a running one ([spend.md](../daemon/spend.md#new-runs-wait)).
- A live run and its tmux session survive a daemon restart. Its process keeps the `priory mcp`,
  kit, plugin and tool list it started with, since Claude Code starts its servers once per session
  ([zero-downtime-restarts.md](zero-downtime-restarts.md#mcp-servers-survive-a-binary-swap)).
- A jailed run's writes outside its declared paths land in the jail's folders in memory, and are
  gone when the run ends ([sandboxes.md](../daemon/sandboxes.md)).
- The `compact` tool queues a `/compact` prompt in the outbox, which types it once the run is idle
  (`tools.rs:719`, [outbox.md](../daemon/outbox.md#compact)). `idleCompactAt` compacts a shared
  session idle for 55 minutes by resuming it in `priory-idle-<agent>`
  ([sessions.md](../daemon/sessions.md#compacting-an-idle-session)).
- #455, closed, let a person's own session ask for a compaction through hc. It typed into a
  terminal Priory does not own: the terminal check and `hc send` are two steps, and hc reports an
  idle agent as `idle` or `done` without proving the prompt box is empty.

## The model

### A kept run belongs to a task

| Option | What it gives | What it costs |
| ------ | ------------- | ------------- |
| **A task.** A kept run is a run of a task whose step keeps its process. A person's own session is a task of a `session` workflow with one model step | The Desk, Costs, `priory tasks`, asks, says, waits, memory grants, the trace and the queue all work as they do, since each one keys on a task | A person's session shows in task listings, so it needs a lane, such as `none/session`, and a title |
| A run with no task | A person's session stays out of task listings | Every reader of runs assumes a task: the outbox, spend caps, the room, the trace, asks and `say`. Each would need a second path |

Decided: a task. The second request then becomes a workflow, not a new kind of thing.

### A step opts in with `keep`

| Option | Why |
| ------ | --- |
| **A step key, `keep = true`** | The step is where a run sleeps, and where `attach` already lives. Bonjour keeps its report step, and its other steps end as today |
| A workflow key | Too coarse: a workflow with a long review step would keep every step |
| An agent key | One agent serves several workflows and steps, and a keep belongs to only some of them |

`keep` needs a model step, so `priory check` refuses it on a code step, a human step or a debate
step. So `debate.rs:603` never keeps a process. `keep` does not need `attach`: a kept run without a
terminal still saves the resume.

### The data model: a kept run ends, and a wake starts a new run in its process

A kept run's row ends as today, with `ended` set and the new end reason `kept`. A new column,
`runs.kept`, holds the time it was kept, and is cleared when its process stops or a wake moves it
on. A task has at most one kept run, by a partial unique index.

A wake into a task with a kept run does what the carried run does today: it inserts a new run row
on the kept run's tmux and harness sessions, copies its kit, harness and sandbox columns, moves its
open keyboard grant and unconfirmed outbox prompts to the new row, clears `kept`, sets the task
`running` and queues the wake's prompt. The outbox types it at the process's next idle point.

| Option | Why |
| ------ | --- |
| **`ended` set, plus `runs.kept`. Each wake inserts a new run row** | Every reader of "a live run is working" stays right with no change. One run is still one turn of work, so Costs and the trace split by wake, as they split by step for the carried run. Only the readers of the kept process change, listed below |
| `ended` stays `NULL` while kept | Every one of the 120 queries that read a live run as working would need `AND kept IS NULL` or a reason not to: `max_running` would count idle kept runs, preemption and spend would treat them as busy, and the sweep would keep their worktrees as it does now. Missing one lets an idle process hold a lane |

The readers of `ended IS NULL` that stay as they are, and why:

- `running_in` (`state.rs:4134`) does not count a kept run toward `max_running`, which is the
  exemption the first slice needs.
- The spend caps' candidates (`spend.rs:415`) read running tasks between turns. A kept run's task
  is asleep.
- The sweep's `UNUSED` (`sweep.rs:1436`) frees a worktree with no live run. A kept run's task is
  asleep, and the sweep already keeps an asleep task's worktree.

The readers that learn the kept process, in the first slice:

- `state::ended_sessions` (`state.rs:5276`) excludes kept runs, so the daemon's start does not kill
  a kept process as a crash's leftover.
- The start's pane check (`daemon.rs:322`) also checks each kept run's pane. A kept run whose pane
  is gone clears `kept`, and its task stays asleep with its waits. The next wake resumes the
  session in a new process, as today.
- The room's measured use (`state.rs:4255`) adds each kept process's scope use.
- The preemption candidates (`preempt.rs:112`) add kept runs. See
  [Room, caps and spend](#room-caps-and-spend).
- A task's cancel, drop and done stop its kept process, as `tools::cancel` (`tools.rs:5039`)
  stops a live run.
- `adapters/herdr/attach` follows the process, not the run. It calls `priory attach --run N
  --read-only` when a run starts, while that run is live, so it needs no change to
  `state::live_run`.
- Idle compaction (`idle.rs:33`) skips a session with a kept run, since the fourth slice compacts it
  in place.

The readers that learn the kept process in the second slice, which lets a person type:

- `tools::run` (`tools.rs:115`) finds a hook's run by `runs.ended IS NULL AND runs.session = ?`.
  Every hook from a kept process between wakes finds no run: a person's `UserPromptSubmit` and
  `Stop`, and the `SessionStart` that follows an auto-compaction. It learns to find the kept run of
  the session, so the person's turn below can start a run.
- `state::live_run` (`state.rs:3705`) refuses an ended run, so the web view's Terminal read
  (`screen.rs:276`) and the grant's reply (`serve.rs:1282`) fail on a kept run. It accepts a kept
  run.
- The grant sweep (`keyboard.rs:344`) ends every open grant whose run has `ended` set. It spares a
  kept run's grant.
- `keyboard::take` accepts a kept run.
- A person's turn makes a new run, as the next section says.

### Which sleep paths change

One helper replaces `end(tx, run, "asleep")` and the returned tmux session at seven of the eight
paths: the three in `settle`, `tools::sleep`, and the three `finish("asleep")` in `step_in`. When
the run's step has `keep`, the helper ends the run `kept`, sets `runs.kept` and returns no tmux
session to kill. Otherwise it does what it does today. `debate.rs:603` never meets a kept step.

In `step_in`, `keep` is read from the step of the run that stored the result, the step that ends.
The two waits of a stack (`tools.rs:3920`, `tools.rs:3951`) stay at that step, so its agent's
process is kept for the wake, as at `settle`. A human step's ask (`tools.rs:4084`) has no agent:
the run still ends `kept`, and the answer decides. When the answer moves the task to a model step of
the same agent that is not irreversible, that step's run starts in the kept process, as the carried
run does. Any other target stops the kept process, and the next step's run starts a new one.

In the second slice, a person's turn typed into a kept run, through a keyboard grant, makes a new
run as a wake does, when its `UserPromptSubmit` arrives and `tools::run` finds the kept run. Its `Stop` with no result goes through `settle` again, so the
run ends `kept` once more, with no nudge. A result the agent stores in that turn moves the task as
usual.

### Who holds the keyboard

The keyboard grant is the one lock on a kept run's input, for the web view and Herdr. In the first
slice no one takes a grant on a kept run: people only watch it, and `priory attach` on a kept run
refuses read-write. The second slice ships the grant and its renewal together, so a grant never
lapses at 15 minutes while its read-write client stays attached, and a wake never types into a
half-typed line.

- `adapters/herdr/attach` keeps opening a read-only terminal for a kept process, and reuses it
  across wakes, since the tmux session stays the same.
- To type, a person takes the keyboard. Herdr's bar takes a grant through the same endpoint as the
  web view, then opens a read-write attach. Detaching or the grant's end returns it. A grant on a
  kept run renews while its client stays attached, instead of ending at 15 minutes. When renewal
  stops, the read-write attach ends with the grant.
- While a grant is open, the outbox waits, as it does today. A wake or a `say` queues and is typed
  at the first idle point after the grant ends. The desk shows `waits for the keyboard` on it.
- `priory attach` without `--read-only` on a kept run takes a grant too, so a tmux client that can
  type is always visible to the outbox.

Without `[serve] terminal = "type"`, no one can take a grant. A kept run then stays read-only to
people: Herdr opens only read-only terminals, and `priory attach` on a kept run refuses
read-write. Wakes still type into it. `priory check` warns when a step has `keep` and `attach` in a
factory without that setting.

| Option | Why rejected |
| ------ | ------------ |
| Read-write Herdr terminals with no grant | The outbox cannot tell a person is mid-sentence, so a wake lands in their prompt box |
| Priory always wins, typing into a held session | A person's half-typed line would be submitted with the wake |
| A person always wins, dropping wakes while they type | Loses picks and wait firings, which the outbox exists to keep |

### Room, caps and spend

A kept idle process costs memory and no tokens. Count what it uses, and bound how many stay.

- **Room.** The room counts a kept process by its scope's measured use, with no unused promise. An
  idle Claude process holds a few hundred megabytes, and a jailed one adds its in-memory writes.
- **Agents slice.** The process stays in its scope in the slice, so `CPUQuota` and `MemoryHigh`
  cover it. It uses almost no CPU while idle.
- **Lane and trigger caps, the review limit.** A kept run holds no work and does not count toward
  `max_running`. A wake into it starts its new run without the queue's `max_running` and room
  checks, since the process already exists and the room already counts it. A lane can so run over
  its cap by its kept runs, which `[keep] max` bounds.
- **Spend caps.** A kept process spends only on turns, and every turn counts toward the caps as
  today. A wake into it checks the `day` and `week` caps before it starts its new run, and waits
  with the same reason when one is reached.
- **Runner filters.** A filter's hours hold new starts only. The daemon stops a kept process whose
  task a filter excludes when the filter's excluded hours begin. The next wake resumes it, as today.
- **Stopping one reuses the preemption marks.** The room's marks ([room.md](../daemon/room.md))
  take kept runs as candidates, before any live run, the longest idle first. A mark on a kept run
  stops its process at once, since it has no turn to end. `[keep] max` in `factory.toml`, 2 by
  default per runner, and `[keep] idle`, 8 hours by default, mark kept runs the same way. A
  stopped kept run clears `kept`, and its task stays asleep with its waits.

### Daemon restarts, upgrades and reboots

A kept process survives a restart as any live run does: its tmux session outlives the daemon, and
the start leaves it alone once `ended_sessions` excludes kept runs.

The daemon upgrades itself, but a kept process keeps the `priory mcp`, kit, plugin and tool list it
started with. Only a resumed session starts new ones. So after a start, the daemon stops each kept
process whose run's `kit_hash` differs from the current kit, and every kept process when the hook
socket's line format changed. Its next wake resumes the session with `--resume` in a new process,
with the new kit and server. A live run with a turn in flight is left alone, as today.

A reboot kills the tmux server. A kept run whose pane is gone at the start clears `kept`, and its
task stays asleep. Its next wake resumes the session. For a person's own session, the daemon
restarts it at once with `--resume`, so it is back in Herdr after a reboot.

### Compaction on demand

A kept run compacts on demand through the existing `compact` tool (`tools.rs:719`) and a person's
`/priory:compact`. Both queue `/compact <focus>` in the outbox, which types it at the next idle
point with no grant open ([outbox.md](../daemon/outbox.md#compact)). It is safe here and was not in
#455 for three reasons:

1. Priory owns the pane. It resolves it from the run's tag and types by pane id, so no other
   program can receive the text.
2. Priory knows the prompt is idle. The run's `Stop` and `UserPromptSubmit` hooks mark it, and an
   open grant marks a person at the keyboard. hc could only report `idle` or `done`.
3. The outbox stores the prompt before it types, confirms it on `UserPromptSubmit`, and never types
   it twice across a restart.

The outbox keys a prompt by task, and a kept run's task has no live run to type into. So a compact
queued for a kept run's task first starts a run on the kept process, as a wake does, and the outbox
types `/compact` into that run.

`idleCompactAt` then compacts a kept run in place, without the `priory-idle-<agent>` resume, since
the process is already live and its cache is warm.

### Security

- **A jailed run lasts days.** A kept jailed process keeps its declared read-write paths, the
  daemon's socket and its spool open between wakes, and its in-memory writes stay until the
  process stops. The room counts those writes, and `[keep] idle` bounds how long they last.
- **A task's end stops its process.** Cancel, drop and done stop a task's kept process, as they
  stop its live run today. A kept process never outlives its task.
- **A person steering a task's agent.** A person who holds the grant can type anything into a kept
  run. The agent still has only its step's tool list and sandbox, a result still goes through its
  step's gates, and the trace records each turn with who held the keyboard. It is the same trust
  the web view's Terminal gives a person over a live run today, for longer.
- **`priory session` and the jail.** A `session` task runs with its agent's sandbox, as any step
  does. The built-in `session` workflow's default agent has none, so a person's own session runs
  as that person, as it does in their own terminal today. A factory can give it a jailed agent.

### Herdr

`adapters/herdr/attach` keeps one terminal per kept process. Since the tmux session spans its
sleeps, the terminal keeps showing the same agent: its last answer and its empty prompt while idle.
Priory does not draw into the pane. The Priory bar shows the kept state on the task's tab instead:
`kept, idle 2h 10m, wakes at 18:30` from the task's open waits, and `you hold the keyboard` during a
grant. When the process stops, the terminal shows tmux's exit, as today.

### The PRD's Run

The [PRD](../prd.md) defines a Run as "One agent process working on a task, from start to sleep or
exit". A kept run's process outlives its sleep, so the PRD changes to:

> **Run**: one stretch of an agent process's work on a task, from its start or a wake to its sleep
> or exit. A kept run's process outlives its sleep, and the run its task's next wake starts uses
> the same process.

Add **kept run** to the Concepts table: a run that ended asleep while Priory keeps its process.

## The first slice: bonjour's process stays alive after its report

It ships on its own, with every part a kept run needs to be safe for a day. People only watch a
kept run in it: no grants and no human turns.

- `keep = true` on a model step, with its `priory check` rules and the warning without
  `[serve] terminal = "type"`.
- `runs.kept`, the `kept` end reason and the helper at the seven sleep paths, with `keep` read from
  the step of the run that ends.
- A wake into a kept run starts a new run on its process, without the `max_running` and room
  checks, after the spend caps.
- The room counts a kept process's measured use, and the preemption marks, `[keep] max` and
  `[keep] idle` stop kept processes at once.
- The start keeps kept processes, ends those whose pane is gone, and stops those with an old kit
  hash or socket line format.
- Herdr's attach stays read-only, `priory attach` on a kept run refuses read-write, and cancel,
  drop and done stop the process.

Tests, at the seam the daemon uses: a kept step's sleep at the ordinary `Stop` leaves its tmux
session alive and the task asleep; a time wait that fires starts a new run on the same tmux
session and types the prompt into it; a third kept run is marked and stopped; a daemon start keeps
a kept process and stops one with an old kit hash; `priory attach` on a kept run refuses
read-write. Measured on a desktop's bonjour: 2 tmux sessions per task before, 1 after, and its
Herdr terminal shows the agent at 18:30.

## Later slices

2. A person types: the keyboard grant from Herdr's bar and `priory attach`, its renewal while
   attached, the desk's `waits for the keyboard`, and the person's turn that starts a run. The
   readers it needs: `tools::run`, `state::live_run` for the Terminal read and the grant's reply,
   and the grant sweep.
3. Runner filters stop excluded kept processes.
4. Compaction on demand for kept runs: a compact queued for a kept run's task first starts a run on
   its process. `idleCompactAt` compacts in place.
5. `priory session [--agent NAME] [TITLE]`: a task of a built-in `session` workflow whose one step
   keeps its process, opens a read-write attach with a grant, and restarts after a reboot.
6. The Priory bar's kept line in Herdr.
7. The PRD amendment, with the first slice's merge.

## Decided questions

- **Does `[keep] max` count a person's own sessions?** No: a person starts those by hand, so a
  separate `[keep] sessions` cap, 4 by default, keeps an agent's kept runs from stopping them.
- **What does `keep` do for a Codex agent?** Codex sends no `SessionStart` on resume, so idle
  compaction cannot work. `keep` is allowed, and only compaction is refused, as `idleCompactAt`
  does.
- **Does a person's turn in a kept run count as the task's spend?** Yes, toward the same caps: the
  tokens are real, and a separate account would hide them from Costs.
