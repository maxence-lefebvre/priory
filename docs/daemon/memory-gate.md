# Memory gate

`priory memory acquire [--gb N] [--timeout D] -- <command>` runs a heavy command, such as a build
or a test suite, only when the machine has room for it. Builds from several agents then queue
instead of exhausting memory. N is a GB count, such as `8` or `1.5`, and 10 by default. D is a
duration such as `90s`, `15m` or `2h`.

1. The command asks the daemon over `priory.sock` for a grant of N GB. It sends its pid and, inside
   a run, the run's `PRIORY_SESSION`, which names the grant's task.
2. Grants wait in one queue, oldest first. The daemon grants the oldest waiting grant when the
   measured free memory, minus the memory promised to live runs and every held grant, minus N,
   stays at or above `(1 - memory share) × MemTotal`, as the room check for runs does. A grant
   leaves out the promise of its own task's run, which would otherwise wait for itself. A grant
   that does not fit holds back every grant behind it, even a smaller one that fits, so a large
   build is never starved.
3. The command polls the daemon every 250 ms. Once its grant is held, it starts a shell that waits
   for a go-ahead, hands the grant to that shell's process, and only then lets the shell run the
   command with `acquire`'s own working directory, environment and terminal. The shell becomes the
   command, so the grant follows it. An `acquire` that dies before the hand-off closes the shell's
   go-ahead pipe, and the command never runs.
4. When the command exits, however it exits, the grant is released. `acquire` exits with the
   command's code, or 128 plus the killing signal.

The daemon removes a grant whose holder is gone, on each tick and each gate message. The holder is
the `acquire` process while the grant waits, then the command's process. The daemon identifies a
holder by pid and start time in `/proc/<pid>/stat`, so a zombie or reused pid counts as gone. A
killed `acquire` that waits frees its place within a second. A killed `acquire` whose command still
runs keeps its grant until that command ends.

`acquire` exits 125 and never runs the command when no daemon listens, when the daemon cannot
measure the room or refuses the grant, or when the grant still waits after `--timeout`, 1 hour by
default. A grant above the runner's share of the machine can never fit, so the daemon refuses it at
once, as
`9.0 GB never fits: this runner's agents may take 7.2 GB`. A command that cannot start exits 127.

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

- Two runs' grants can each wait for the other run's promise. The default timeout ends that wait
  after an hour, and the agent sees the failure.
- Runs and grants queue apart. A run that fits starts even while a grant waits, and can take the
  room the grant waits for.
- The command's process holds the grant, not the processes it leaves behind. A daemonized child of
  a build runs on without a grant.
- A grant that arrives in the last poll before `--timeout` still runs its command, up to 250 ms late.
- A lease's memory frees as soon as the lease ends, while its command may still take up to 5
  seconds to stop.
- A leased command runs outside the terminal's foreground group with no input. Ctrl-C reaches the
  lease, whose guard then sends the command `SIGTERM`, not `SIGINT`.
- A daemon that stops while a command waits fails that command with 125. A held grant survives a
  daemon restart in the state database.
