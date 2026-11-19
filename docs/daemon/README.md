# The daemon

`priory daemon [DIR] [--once]` reads the herdr-bus output `factory.intake` as the consumer `priory`,
records each message in the inbox and routes it to a task. It checks the factory first, as
`priory check` does, and refuses to start while it has problems. `--once` routes what is pending
and exits; it starts no runs, since no daemon would stay to hear their hooks. Without it, the
daemon looks for new messages every second and runs agents, as described under Runs.

State is one SQLite file, named by `PRIORY_DB`. The daemon refuses to start without it, or with an
empty name, `:memory:` or a `file:` URI, which SQLite may not keep on disk.

## Which file to read

- [routing.md](routing.md): how one message is recorded, acked and routed to a task.
- [home.md](home.md): what `PRIORY_HOME` holds: clones, worktrees, the socket and the lock.
- [runs.md](runs.md): how a run starts its harness, and the queue, bands and holds.
- [room.md](room.md): when the machine has room to admit a run, and queue stalls.
- [agents-slice.md](agents-slice.md): the systemd slice that caps every agent's CPU and memory.
- [memory-gate.md](memory-gate.md): `priory memory acquire`, which queues heavy commands for memory.
- [sessions.md](sessions.md): shared agent sessions, attaching to a run's terminal, and trusting a
  clone for Claude.
- [codex.md](codex.md): how a Codex run differs from a Claude run.
- [signals.md](signals.md): the hook signals a run sends, and the nudge after a silent turn.
- [health.md](health.md): run verdicts, failed or blocked tasks, repair, and the stall handler.
- [desk.md](desk.md): `priory desk`, resume, cancel and revive.
- [sweep.md](sweep.md): worktree removal, retiring asleep tasks, sweep scripts and the sweeper.
- [tools.md](tools.md): the MCP tools, moving between steps, code steps and approvals.
- [waits.md](waits.md): asks and answers, wait conditions, and the checks gate before `done`.
- [outbox.md](outbox.md): prompt states and how the daemon types a prompt.
- [trace.md](trace.md): trace records, token counts, `priory report` and the OpenTelemetry export.
- [services.md](services.md): running Priory as systemd user services, and the listing commands.
