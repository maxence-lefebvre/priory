# Memory gate

`priory memory acquire [--gb N] [--timeout D] -- <command>` runs a heavy command, such as a build
or a test suite, only when the machine has room for it. Builds from several agents then queue
instead of exhausting memory. N is a GB count, such as `8` or `1.5`, and 10 by default. D is a
duration such as `90s`, `15m` or `2h`.

1. The command asks the daemon over its socket for a grant of N GB. It sends its pid and, inside
   a run, the run's `PRIORY_SESSION`, which names the grant's task.
2. Grants wait in one queue, in the order they were requested. The daemon grants the oldest waiting grant when the
   measured free memory, minus the memory live runs promised and do not use yet and every held
   grant, minus N, stays at or above `(1 - memory share) × MemTotal`, as the room check for runs
   does (see [Room](room.md)). A grant leaves out the promise of its own task's run, which would
   otherwise wait for itself. A waiting grant that does not fit holds back every grant behind it,
   even a smaller one that fits, so smaller grants do not starve a large build. Once it has waited 5 minutes,
   later grants that fit go first, in order, and it keeps its place ahead of them for the next
   room. A build it waits for usually ends within 5 minutes, and a grant behind it still has most
   of its 1-hour default timeout left. Once it has waited 15 minutes, later grants stop going first
   while the end of the held grants would make it fit, so the room drains towards it. To predict
   that room, the daemon adds back each held grant's GB and the memory that the command's processes
   use, at most those GB. It takes off what the run of the grant's task would promise again. The
   prediction only holds grants back: every grant still needs the measured room. A grant held for 30
   minutes or more does not count: it may be a hung test or a dev server's lease. So a head holds
   back the grants behind it for at most 30 minutes after the last grant given before it, and later
   grants that fit then go first again. They also do when the prediction changes and the head would
   not fit. A retry that keeps its request time does not restart the 30 minutes.
3. The command polls the daemon every 250 ms. A poll that gets no reply, because the daemon is
   busy, restarting or gone, does not end the wait. The command reports it and polls the same grant
   again until `--timeout`. The daemon keeps a waiting grant for as long as its `acquire` process
   lives, so the grant keeps its id and its place through a daemon restart. An `acquire` that asks
   again after a lost reply gets the grant it already has: the one with its process, GB, command
   and task, even when its run ended in between. Once its grant is held, it starts a shell that waits
   for a go-ahead, hands the grant to that shell's process, and only then lets the shell run the
   command with `acquire`'s own working directory, environment and terminal. It sends the hand-over
   again while a reply is lost and the shell lives. The shell becomes the
   command, so the grant follows it. In a run, that environment is the run's own: its `PATH` and
   every variable its harness got, including the worktree's direnv environment when the run starts
   in it (see [sessions.md](sessions.md#with-trust_worktrees)). The gate adds and removes nothing. An `acquire` that dies before the hand-off closes the shell's
   go-ahead pipe, and the command never runs.
4. When the command exits, however it exits, the grant is released. `acquire` exits with the
   command's code, or 128 plus the killing signal.

The daemon removes a grant whose holder is gone, on each tick and each gate message. The holder is
the `acquire` process while the grant waits, then the command's process. The daemon identifies a
holder by pid and start time in `/proc/<pid>/stat`, so a zombie or reused pid counts as gone. A
killed `acquire` that waits frees its place within a second. A killed `acquire` whose command still
runs keeps its grant until that command ends.

`acquire` exits 125 and never runs the command when no daemon listens to its first request, when
the daemon cannot measure the room or refuses the grant, or when the grant still waits after
`--timeout`, 1 hour by default. No daemon listens when its socket refuses the first request, and `acquire` does
not wait for a restart then. A grant of a run that stops waiting, because it timed out or its
`acquire` was killed, leaves its request time behind for an hour. The daemon records it even when
it only sees that the process is gone. When the same task acquires the same command with the same
GB again within that hour, the new grant
takes the oldest such request time, so a retry keeps its place in the queue. Each retry takes one. A grant above the runner's share of the machine can never fit, so the daemon refuses it at
once, as
`9.0 GB never fits: this runner's agents may take 7.2 GB`. A command that cannot start exits 127, or 126 when it is not executable.
`acquire` and `lease` look a program name without `/` up on `PATH` before they ask for a grant. A
missing one exits 127 at once and takes no place in the queue, as
`moon is not on PATH (/usr/bin:/bin); the command did not run`.

`priory memory lease [--gb N] [--timeout D] -- <command>` runs a server or stack that stays up,
such as a dev server, with N GB reserved for as long as it runs. It takes the same flags, waits in
the same queue and exits with the same codes as `acquire`. Use it instead of `acquire` for a
command that outlives one job.

A lease differs from `acquire` in who holds the grant. The `lease` process keeps it for the
command's whole life, so killing the lease, even with `SIGKILL`, frees its memory within a second.
The command runs with no input, in the process group of a small guard. Once the lease process ends,
however it ends, the guard sends that group `SIGTERM`, then `SIGKILL` 5 seconds later. So
`kill <lease pid>` ends the server too, and a command that exits leaves no process of its group
behind.

## Giving up

Off by default. With [`[memory] give_up_after`](../reference/factory.md#memory) in `factory.toml`,
an `acquire` without `--timeout` that still waits at that limit exits 75 and never runs its command.
It prints one line the agent can act on:

```text
priory memory: waited 10m for 8 GB behind 2 grants; the command did not run: push and let CI run it
```

- The count is the held grants and the waiting ones requested earlier. With none, the line says
  `with no grant ahead`: live runs hold the room.
- The daemon decides on each poll, so a reloaded limit applies to commands that already wait. A
  grant that fits in the poll at the limit is held.
- The daemon ends the grant in the step that gives up, so it cannot be granted later. Inside a run it
  leaves its request time to a retry, as a timed-out grant does. After a lost reply, the next poll
  gives up again, as `waited 10m` without the GB and the count.
- The limit counts the command's own wait, not its request time, so a retry waits its full limit.
- `--timeout` wins over the limit, and expires with 125. `lease` has no limit: CI cannot replace a
  server.
- The MCP server's instructions tell an agent to push and wait for CI on exit 75, without asking a
  person. An investigator, which never pushes, is not told, but a give-up prints it the same line.

It is off by default because Priory cannot know that a repository's CI runs the command. A limit
below 15 minutes ends a wait before later grants stop going first (see step 2), unless the command
is retried within the hour and keeps its request time.

## What a person sees

`priory memory` prints each grant in request order: id, task, state (`held` or `waiting`), GB,
command, age, pid and flag. An age is such as `1h 51m`, `12m` or `40s`. A held grant's age counts from its grant, a waiting one's from its request.
The pid is the grant's holder: the `acquire` process while the grant waits, then the command; the
`lease` process for a lease.
The flag is `long` on a grant held past an hour while another waits, and `-` otherwise.
`priory desk` shows each waiting grant as a `memory` row that names the longest held grant, and each
`long` grant as a row of its own (see [desk.md](desk.md)). The limit is a fixed hour, since Priory
keeps no usual time of a command. Nothing stops a flagged grant.

The MCP server's instructions teach every agent the gate. They ask it to wrap each heavy command in
`priory memory acquire --gb N -- <command>`, with the path of this `priory`.

## Known limits

- When a reload removes the limit between a lost give-up reply and the next poll, `acquire` exits 125
  with `no memory grant`. The command did not run.
- A time on the desk is as of the last read. The web view reads again when the state changes, so
  the time and a new `long` row can lag.
- A lease that holds its grant past an hour is flagged too while another grant waits.
- A command that waits while no daemon listens gives up at its first poll after the daemon is back.
- A held grant counts in full until its command ends, whatever the command uses. A command that
  asks for far more than its peak holds that room back for its whole life.
- Between its 5 and 15 minutes, a large grant can wait on while smaller grants take the room it
  needs.
- A build held for 30 minutes or more no longer protects a large grant that waits for it: smaller
  grants can take its room again.
- A held grant whose command uses its memory counts twice: in the measured free memory and in full
  as a promise. A hung command keeps both until it ends. Counting only what a command does not use
  yet would admit too much, because a build's use already lowers its run's promise.
- The prediction past 15 minutes reads the anonymous memory of the command and its descendants. It
  misses shared memory, memory above the grant's GB and a process that left the tree, so smaller
  grants can still overtake a large one. It can also hold them back for a head that does not fit
  once the grants end, because processes that share pages count them once each, and live runs or
  other programs can take the room.
- The prediction takes the command's use off its run scope's use. Pages that a build's processes
  share copy-on-write can make the command's use sum to more than the scope's. The scope's use then
  counts as 0, so the daemon expects the run to promise its whole declaration again.
- The prediction finds descendants in `/proc/<pid>/task/<tid>/children`, which the kernel has only
  with `CONFIG_PROC_CHILDREN`. Without it, only the command's own process counts.
- A jailed run's `acquire` sends its pid inside the jail. The daemon looks that number up among the
  host's processes, so it can watch and measure another process instead of the holder.
- Grants ignore bands. A request from a `now` task waits behind an older one from a `later` task.
- After the grants behind a head went first, their own 30 minutes can hold the queue back for it
  again.
- Two requests of the same task with the same command, GB and pid share a grant. Two jailed runs
  of one task can have the same pid.
- A retry keeps its request time only inside a run, and only with the same command text and GB. Another
  run of the same task that retries the same command also takes it.
- Two runs' grants can each wait for the other run's promise. The default timeout ends that wait
  after an hour, and the agent sees the failure.
- Runs and grants queue apart. A run that fits starts even while a grant waits, and can take the
  room the grant waits for.
- The command's process holds the grant, not the processes it leaves behind. A daemonized child of
  a build runs on without a grant.
- A grant that arrives in the last poll before `--timeout` still runs its command. It runs up to
  250 ms late, or up to 40 seconds late when that poll waits on a busy or restarting daemon.
  `--timeout` does not bound the hand-over that follows: while no daemon replies to it, the command
  waits without limit, and `acquire` prints each new reason.
- A grant that the daemon grants just as its `acquire` times out leaves no request time, and a retry
  queues at the back.
- A lease's memory frees as soon as the lease ends, while its command may still take up to 5
  seconds to stop.
- A leased command runs outside the terminal's foreground group with no input. Ctrl-C reaches the
  lease, whose guard then sends the command `SIGTERM`, not `SIGINT`.
- A grant waiting while the daemon is gone waits until its `--timeout`, even when no daemon comes
  back. Held and waiting grants survive a daemon restart in the state database.
