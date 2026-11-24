# Room

The daemon admits a run only when the machine has room for it, measured live:

| Measure | Read from | Admits when |
| ------- | --------- | ----------- |
| Memory | `MemAvailable` and `MemTotal` in `/proc/meminfo` | free memory, minus the memory live runs promised and do not use yet and the memory grants hold, minus the agent's `memory`, stays at or above `(1 - memory share) × MemTotal` |
| Disk | `df -Pk` on `PRIORY_HOME`, which reads `statvfs` | free disk, minus the disk promised to live runs, minus the agent's `disk`, stays above 10 % of the disk, or above 20 GB on a disk over 200 GB |
| CPU | the 1-minute load in `/proc/loadavg`, and the CPUs this process may use | the load stays below `cpu share × CPUs` |

Shares come from `agents` in `runners/<host>.toml`, where `<host>` is the name in
`/proc/sys/kernel/hostname`. Each part defaults to 90 % when the file or field is missing. By
default a run must leave 10 % of memory free, and the load must stay below 90 % of the CPUs.
`agents` has no disk part. Disk always keeps 10 % free, and at most 20 GB, so a large disk with tens of GB
free still admits runs.

An agent declares `memory` as a size such as `512MB` or `6GB`, where a GB is 1024³ bytes, as in
`/proc/meminfo`. An agent with no `memory` declares 4 GB: Claude Code takes about 0.5 GB, and an
agent runs builds and tests beside it. Declaring too much only delays a start. Declaring too little
lets agents run the machine out of memory.

An agent may also declare `disk`, a size reserved above the disk floor. An agent with no `disk`
reserves none. A run that waits for disk it declared gets the reason `waits for room: 22.0 GB of
disk free, needs 8.0 GB and keeps 20.0 GB free`.

A live run promises its agent's `memory` and `disk` from start to end. What a run already uses is
already missing from measured free memory, so the room subtracts only the part of each memory
promise the run does not use yet: `max(0, memory - use)`. A run's use is the `anon` plus `shmem`
in the `memory.stat` of its systemd scope in the agents slice (below), read from cgroup v2 under
`/sys/fs/cgroup`. The scope's file cache, such as a build's output, is left out, since
`MemAvailable` already counts it as free. The daemon finds the scope from the cgroup of the run's
tmux pane process. It keeps that process and its start time in memory for the run's life, so a
reused pid is never read, and asks tmux again after 30 seconds when it found no pane. A run with no
scope in the slice, as on a host without systemd, or whose use cannot be read, keeps its whole
promise. Disk promises always count in full, since no run
reports its disk use. Held memory grants (see Memory gate) also count in full, since a granted
command may not have allocated yet. So two runs admitted in turn cannot take the same free room,
and no run starts into memory a build holds. The daemon measures again before each admission, since a start's
clone and worktree take disk.

A task that does not fit stays `queued` or `ready`, with a reason such as
`waits for room: 3.1 GB of memory free, needs 6.0 GB and keeps 6.4 GB free`, `waits for room: 12.0
GB of disk free, keeps 20.0 GB free` or `waits for room: load 7.50 on 8 CPUs, keeps it under 7.20`.
`priory tasks` shows it.

The first task waiting for room holds back every task behind it in the queue, even a smaller one
that fits. Each gets the reason `waits behind task <id>, which waits for room`. A higher band only
starts earlier. It never skips the room check, and a large task never starves behind smaller ones
that keep taking its room. The cost is idle room while it waits. A task
whose agent declares more memory than the runner's share of the
machine, or more disk than the disk holds above its floor, can never fit, so it holds back no one. Nor does a task waiting for a shared
agent (see Shared sessions).

While the head of the queue waits for room it would lack even with no live run, the queue is
stalled. A head that waits only for live runs' promises to end is ordinary backpressure: it waits
with its reason and opens no stall. The daemon records the stall in the state file, so its start
survives a restart. `priory desk` starts with one banner line:
`stalled since 2026-11-19T10:00:00Z: short of disk by 10.0 GB; task 3 waits for room`. The amount
it is short by leaves out the promises too. Another task heading the queue for the same resource
keeps the stall. A different resource starts a new one. The stall ends when no task waits for room
at the head. A task waiting because room cannot be measured holds the queue without a stall. The
sweep scripts and the sweeper act on stalls (see Sweep).

## Urgent wakes

By default a higher band only starts earlier: a `blocker` task that finds the machine full waits
for a run to end. `[queue] preempt` in `factory.toml` names the bands whose waiting task may stop a
run of a lower band. None may by default:

```toml
[queue]
preempt = ["blocker"]
```

`priory check` refuses a band the factory lacks, and the last band, which has no band below it.

### When a run is marked

On each tick, the daemon marks runs when all of these hold:

1. The head of the queue waits for room, and its band is in `preempt`.
2. It lacks memory or declared disk, and it would fit if some live runs were gone. A stopped run
   gives back the memory it uses and the part of its promise it does not use yet. A head
   that waits for load, for a shared agent, for a pause or for a measurement that failed marks
   nothing.
3. Live runs of a lower band, which the next section does not protect, promise enough.

The daemon picks runs in reverse start order: the lowest band first, then the last in a person's
order, then the youngest task. It adds runs until the head fits by the room's own check. When every
run it may pick still frees too little, it picks none. Each tick works the marks out again, so a
mark is withdrawn once the head starts, is held or cancelled, or stops being the head. A mark is
stored in the state file, so it survives a daemon restart.

A mark changes nothing until the run's turn ends. The head then goes through the room check like
any task: a stop never admits a run into room the machine lacks.

### What is never stopped

The daemon never marks these runs, and withdraws a mark it finds on one at the turn end. A
[spend cap](spend.md#held-with-one-ask) never ends a run of the first four rows either:

| The run | Why |
| ------- | --- |
| Is at a step whose effect is `irreversible` | A stop between two of its calls could leave a push without its pull request |
| Has an open keyboard grant | A person is typing in it |
| Has a tmux client attached that can type | A person may type. A read-only client does not count. A `tmux list-clients` that fails counts as a client |
| Runs in a shared session | It serves several tasks |
| Belongs to the sweeper | It frees the room the queue waits for |
| Belongs to a task in the head's band or a higher one | A band never stops itself. `priory move --band` protects a task |
| Belongs to a task already stopped since its last `result`, `ask` or `wait` | Each resume gets one whole turn |

The daemon reads `tmux list-clients` only for the runs it may pick, on the tick that marks and again
at the turn end. A person who attaches in the few milliseconds between that read and the kill still
loses the pane.

### What happens at the turn end

At a marked run's `Stop`, the settlement reads the mark before it decides whether the process goes
on, in the transaction that stores the result:

| The turn ended with | Without a mark | With a mark |
| ------------------- | -------------- | ----------- |
| `ask` or `wait`, with no answer or `say` queued | The run ends and the task sleeps | The same |
| `ask` or `wait`, with an answer or a `say` already queued | The run goes on, with that prompt next | The run ends as `preempted`, and the answer or `say` stays queued behind the stop prompt |
| A `result` that leads to another agent's step, a human or code step, `done` or `stop` | The run ends | The same |
| A `result` that leads to a model step of the same agent | The process carries on into the next run | The run ends as `preempted`, and the task is `ready` at the next step with that step's prompt |
| No tool | One nudge | The run ends as `preempted`, with no nudge |

A step over its `budget` fails inside its turn or at its end, as it would without a mark.

A marked run that sits idle between turns, with no prompt attempted, ends on the tick that marks
it. Two cases wait instead:

- A run with a prompt the outbox attempted and has not confirmed keeps its mark and goes on, since
  a stop there would leave the prompt `uncertain`. The mark applies at its next turn end.
- A compaction due at this `Stop` gives way: the run ends, and the resumed session queues its
  `/compact` at its `SessionStart`, as a session that resumes past `compactAt` does. So it compacts
  after its first turn. A compaction already typed finishes first.

The daemon then kills the tmux session, as for a run that sleeps. The task keeps its worktree,
branch, uncommitted files and session. It is `ready` in its own band at its own place, and its next
run resumes the session with this prompt ahead of any other queued prompt:

```text
Priory stopped your last run at the end of your turn to make room for task 12, in band blocker.
Your session and worktree are as you left them. Continue.
```

Only the stopped run's declared `memory` and `disk` come back. Its worktree stays.

### What the desk shows

An urgent wake is backpressure with a plan, so it opens no stall. The head's reason in
`priory tasks` and `priory queue` names the runs that stop:

```text
waits for room: 3.1 GB of memory free, needs 6.0 GB and keeps 6.4 GB free; task 7 (later) stops at its next turn end
```

When it can stop no run, the reason names why, so a person can act:

```text
waits for room: 3.1 GB of memory free, needs 6.0 GB and keeps 6.4 GB free; stops no run: task 3 is at irreversible step open-pr, task 5's keyboard is held, task 9 was already stopped once
```

When the runs it may stop free too little, the list ends with `the runs it may stop free too
little`. A marked task reads `stops at its next turn end for task 12 (blocker)`. A stopped task reads
`stopped for task 12 (blocker); resumes when there is room`, and `priory runs` shows its run as
ended `preempted`. The head's item in `priory queue --json` carries `stops` and `protected` (see
[serve.md](../reference/serve.md)). Each stop appends a `factory.run.preempted` trace record (see
[trace-schema.md](../trace-schema.md#factoryrunpreempted)), whose `waited_ms` says how long the head
waited between the mark and the stop.

A turn end is the only stop point. A turn can run up to its step's `timeout`, so the head may still
wait that long.

## Measurements

`PRIORY_PROC` names a directory to read in place of `/proc`, and `PRIORY_CGROUP` one to read in
place of `/sys/fs/cgroup`. The tests use them, with a fake `df` on `PATH`, to steer measurements.

Known limits:

- A run without a scope still counts twice: its use lowers measured free memory, and its whole
  promise counts again on top. So the room errs toward starting fewer runs there.
- A build that holds a memory grant runs inside its run's scope, so its use also lowers the run's
  unused promise, while the grant counts in full. That errs toward fewer starts too.
- An agent that grows past its declared `memory` can still crowd the machine. The memory gate covers
  builds an agent wraps in it. The agents slice (below) slows the rest down.
- Load is not declared per agent, so when load leaves no room, no task fits anyway.
- The measurements need Linux's `/proc`.
