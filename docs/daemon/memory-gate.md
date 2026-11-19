# Memory gate

`priory memory acquire --gb N [--timeout D] -- <command>` runs a heavy command, such as a build or
a test suite, only once the machine has room for it. Several agents' builds then queue instead of
running the machine out of memory. N is a number of GB, such as `8` or `1.5`. D is a duration such
as `90s`, `15m` or `2h`.

1. The command asks the daemon over `priory.sock` for a grant of N GB. It sends its own pid and,
   inside a run, the run's `PRIORY_SESSION`, which names the grant's task.
2. The grant waits in one queue, oldest first. The daemon grants the oldest waiting grant when the
   measured free memory, minus the memory promised to live runs and every held grant, minus N,
   stays at or above `(1 - memory share) × MemTotal`, as the room check for runs does. A grant
   leaves out the promise of its own task's run, which would otherwise wait for itself. A grant
   that does not fit holds back every grant behind it, even a smaller one that fits, so a large
   build is never starved.
3. The command polls the daemon every 250 ms. Once its grant is held, it runs the command with its
   own working directory, environment and terminal, and hands the grant to the command's process.
4. When the command exits, however it exits, the grant is released. `acquire` exits with the
   command's code, or 128 plus the signal that killed it.

The daemon removes a grant whose holder is gone, on each tick and on each gate message. The holder
is the `acquire` process while the grant waits, then the command's process. The daemon knows a
holder by its pid and its start time in `/proc/<pid>/stat`, so a zombie or a reused pid counts as
gone. So a killed `acquire` that waits frees its place within a second. A killed `acquire` whose
command still runs keeps its grant until that command ends.

`acquire` exits 125 and never runs the command when no daemon listens, when the daemon cannot
measure the room or refuses the grant, or when the grant still waits after `--timeout`, 1 hour by
default. A grant of more than the runner's share of
the machine can never fit, so the daemon refuses it at once, as
`9.0 GB never fits: this runner's agents may take 7.2 GB`. A command that cannot start exits 127.

`priory memory` prints each grant in the order it was requested: id, task, state (`held` or
`waiting`), GB, command and age. A held grant's age counts from its grant, a waiting one's from its
request. `priory desk` shows each waiting grant as a `memory` row.

Every agent learns the gate from the MCP server's instructions, which ask it to wrap each heavy
command in `priory memory acquire --gb N -- <command>`, with the path of this `priory`.

Known limits:

- Two runs' grants can each wait for the other run's promise. The default timeout ends that wait
  after an hour, and the agent sees the failure.
- Runs and grants queue apart: a run that fits starts even while a grant waits, and can take the
  room the grant waits for.
- The command's process holds the grant, not the processes it leaves behind: a daemonized child
  of a build runs on without a grant.
- A grant that arrives in the last poll before `--timeout` still runs its command, up to 250 ms late.
- A daemon that stops while a command waits fails that command with 125. A held grant survives a
  daemon restart in the state database.
