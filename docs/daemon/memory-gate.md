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
   even a smaller one that fits, so a large build is never starved. Once it has waited 5 minutes,
   later grants that fit go first, in order, and it keeps its place ahead of them for the next
   room. A build it waits for usually ends within 5 minutes, and a grant behind it still has most
   of its 1-hour default timeout left. Once it has waited 15 minutes, later grants stop going first
   while any grant is held, so the room drains towards it as held grants end. When no grant is
   held and it still does not fit, live runs take its room, and no release could make it. Later
   grants that fit then go first, so it never blocks the queue for good.
3. The command polls the daemon every 250 ms. A poll that gets no reply, because the daemon is
   busy, restarting or gone, does not end the wait. The command reports it and polls the same grant
   again until `--timeout`. The daemon keeps a waiting grant for as long as its `acquire` process
   lives, so the grant keeps its id and its place through a daemon restart. An `acquire` that asks
   again after a lost reply gets the grant it already has. Once its grant is held, it starts a shell that waits
   for a go-ahead, hands the grant to that shell's process, and only then lets the shell run the
   command with `acquire`'s own working directory, environment and terminal. The shell becomes the
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
`--timeout`, 1 hour by default. A grant that times out inside a run leaves its request time behind
for an hour. When the same task acquires the same command again within that hour, the new grant
takes the old request time, so a retry keeps its place in the queue. A grant above the runner's share of the machine can never fit, so the daemon refuses it at
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

`priory memory` prints each grant in request order: id, task, state (`held` or `waiting`), GB,
command, age and pid. A held grant's age counts from its grant, a waiting one's from its request.
The pid is the grant's holder: the `acquire` process while the grant waits, then the command; the
`lease` process for a lease.
`priory desk` shows each waiting grant as a `memory` row.

The MCP server's instructions teach every agent the gate. They ask it to wrap each heavy command in
`priory memory acquire --gb N -- <command>`, with the path of this `priory`.

Known limits:

- A held grant counts in full until its command ends, whatever the command uses. A command that
  asks for far more than its peak holds that room back for its whole life.
- Between its 5 and 15 minutes, a large grant can wait on while smaller grants take the room it
  needs.
- A retry keeps its request time only inside a run, and only with the same command text. Another
  run of the same task that retries the same command also takes it.
- Two runs' grants can each wait for the other run's promise. The default timeout ends that wait
  after an hour, and the agent sees the failure.
- Runs and grants queue apart. A run that fits starts even while a grant waits, and can take the
  room the grant waits for.
- The command's process holds the grant, not the processes it leaves behind. A daemonized child of
  a build runs on without a grant.
- A grant that arrives in the last poll before `--timeout` still runs its command. It runs up to
  250 ms late, or up to 40 seconds late when that poll waits on a busy or restarting daemon.
- A grant that times out while no daemon listens loses its request time, and a retry queues at the
  back.
- A lease's memory frees as soon as the lease ends, while its command may still take up to 5
  seconds to stop.
- A leased command runs outside the terminal's foreground group with no input. Ctrl-C reaches the
  lease, whose guard then sends the command `SIGTERM`, not `SIGINT`.
- A grant waiting while the daemon is gone waits until its `--timeout`, even when no daemon comes
  back. Held and waiting grants survive a daemon restart in the state database.
