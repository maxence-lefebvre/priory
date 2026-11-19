# Room

The daemon admits a run only when the machine has room for it, measured live:

| Measure | Read from | Admits when |
| ------- | --------- | ----------- |
| Memory | `MemAvailable` and `MemTotal` in `/proc/meminfo` | free memory, minus the memory promised to live runs and held by memory grants, minus the agent's `memory`, stays at or above `(1 - memory share) × MemTotal` |
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

A live run promises its agent's `memory` and `disk` from start to end, since no run reports its own
use yet. The room subtracts every promise and held memory grant (see Memory gate) from measured
free memory and disk. So two runs admitted in turn cannot take the same free room, and no run
starts into memory a build holds. The daemon measures again before each admission, since a start's
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

While the head of the queue waits for room, the queue is stalled. The daemon records the stall in
the state file, so its start survives a restart. `priory desk` starts with one banner line:
`stalled since 2026-11-19T10:00:00Z: short of disk by 10.0 GB; task 3 waits for room`. Another task
heading the queue for the same resource keeps the stall. A different resource starts a new one. The
stall ends when no task waits for room at the head. A task waiting because room cannot be measured
holds the queue without a stall. The sweep scripts and the sweeper act on stalls (see Sweep).

`PRIORY_PROC` names a directory to read in place of `/proc`. The tests use it, with a fake `df` on
`PATH`, to steer measurements.

Known limits:

- A run's actual use already lowers measured free memory, and its promise counts again on top. So
  the room errs toward starting fewer runs. A run that reports its own use would end the double
  count.
- An agent that grows past its declared `memory` can still crowd the machine. The memory gate covers
  builds an agent wraps in it. The agents slice (below) slows the rest down.
- Load is not declared per agent, so when load leaves no room, no task fits anyway.
- The measurements need Linux's `/proc`.
