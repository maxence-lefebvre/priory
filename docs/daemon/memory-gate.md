# Memory gate

`priory memory acquire --gb N [--timeout D] -- <command>` runs a heavy command, such as a build or
a test suite, only when the machine has room for it. Builds from several agents then queue instead
of exhausting memory. N is a GB count, such as `8` or `1.5`. D is a duration such as `90s`, `15m`
or `2h`.

1. The command asks the daemon over `priory.sock` for a grant of N GB. It sends its pid and, inside
   a run, the run's `PRIORY_SESSION`, which names the grant's task.
2. Grants wait in one queue, oldest first. The daemon grants the oldest waiting grant when the
   measured free memory, minus the memory promised to live runs and every held grant, minus N,
   stays at or above `(1 - memory share) × MemTotal`, as the room check for runs does. A grant
   that does not fit holds back every grant behind it, even a smaller one that fits, so a large
   build is never starved.
3. The command polls the daemon every 250 ms. Once its grant is held, it runs the command with its
   own working directory, environment and terminal, and hands the grant to the command's process.
4. When the command exits, however it exits, the grant is released. `acquire` exits with the
   command's code, or 128 plus the killing signal.

The daemon removes a grant whose holder is gone, on each tick and each gate message. The holder is
the `acquire` process while the grant waits, then the command's process. The daemon identifies a
holder by pid and start time in `/proc/<pid>/stat`, so a zombie or reused pid counts as gone. A
killed `acquire` that waits frees its place within a second. A killed `acquire` whose command still
runs keeps its grant until that command ends.

`acquire` exits 125 and never runs the command when no daemon listens, when the daemon cannot
measure the room or refuses the grant, or when the grant still waits after `--timeout`. A grant
above the runner's share of the machine can never fit, so the daemon refuses it at once, as
`9.0 GB never fits: this runner's agents may take 7.2 GB`. A command that cannot start exits 127.

`priory memory` prints each grant in request order: id, task, state (`held` or `waiting`), GB,
command and age. A held grant's age counts from its grant, a waiting one's from its request.
`priory desk` shows each waiting grant as a `memory` row.

The MCP server's instructions teach every agent the gate. They ask it to wrap each heavy command in
`priory memory acquire --gb N -- <command>`, with the path of this `priory`.

Known limits:

- A grant counts on top of the memory its run already promised, so a build inside a run counts
  twice, as a run's own use does under Room. The gate errs toward waiting.
- Runs and grants queue apart. A run that fits starts even while a grant waits, and can take the
  room the grant waits for.
- The command's process holds the grant, not the processes it leaves behind. A daemonized child of
  a build runs on without a grant.
- A grant that arrives in the last poll before `--timeout` still runs its command, up to 250 ms late.
- A daemon that stops while a command waits fails that command with 125. A held grant survives a
  daemon restart in the state database.
