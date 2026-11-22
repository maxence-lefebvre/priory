# The daemon

The daemon turns the intake's messages into tasks and carries each task through its workflow. It
starts agents in tmux, types their prompts, hears their hooks and tool calls, and moves each task
from step to step.

```sh
priory daemon [DIR] [--once]
```

`DIR` is the factory folder, `.` by default. The daemon checks the factory first, as `priory check`
does, and refuses to start while it has problems.

It reads the herdr-bus output `factory.intake` as the consumer `priory`, or `priory.<runner>` in a
factory of several runners (see [runners.md](runners.md)). It records each message in
the inbox and routes it to a task (see [routing.md](routing.md)). Without `--once`, the daemon looks
for new messages every second and runs agents, as [runs.md](runs.md) describes.

`--once` routes what is pending and exits. It starts no runs, since no daemon would stay to hear
their hooks.

## State

The daemon keeps its state in one SQLite file, which `PRIORY_DB` names. It holds the inbox, tasks,
runs, prompts, asks, waits, results and trace records. The listing commands, such as `priory tasks`,
read the same file.

The daemon refuses to start without `PRIORY_DB`. It also refuses an empty name, `:memory:` or a
`file:` URI, because SQLite may not keep those on disk.

Clones, worktrees and the daemon's socket live in a separate folder, Priory home (see
[home.md](home.md)).

## A task's life

For a task that a labelled issue starts, on a workflow whose first step is a model step:

1. The intake publishes the issue event and a task request. The daemon creates the task, `queued`
   at the workflow's start step ([routing.md](routing.md)).
2. When the machine has room ([room.md](room.md)), the daemon adds a worktree and starts the step's
   agent in tmux. The task is `running` ([runs.md](runs.md)).
3. The daemon types the step's prompt through the outbox ([outbox.md](outbox.md)). Hooks tell it
   when the agent is idle or busy ([signals.md](signals.md)).
4. The agent ends its turn with a tool: `result`, `ask` or `wait` ([tools.md](tools.md)). A result
   moves the task to the next step. An ask or a wait puts the task to sleep with no process, until
   an answer or an event wakes it ([waits.md](waits.md)).
5. Every second, the daemon checks each live run's health. A crash, a timeout or a permission
   prompt fails or blocks the task, and the desk shows it ([health.md](health.md),
   [desk.md](desk.md)).
6. At `done`, a task with a pull request waits for the pull request's checks before it is done
   ([waits.md](waits.md)). Later, the sweep removes its worktree ([sweep.md](sweep.md)).

## Which file to read

The keys of `factory.toml`, workflows and agents are in [docs/reference/](../reference/README.md).

- [routing.md](routing.md): how one message is recorded, acked and routed to a task.
- [home.md](home.md): what `PRIORY_HOME` holds: clones, worktrees, the socket and the lock.
- [runs.md](runs.md): how a run starts its harness, and the queue, bands and holds.
- [room.md](room.md): when the machine has room to admit a run, and queue stalls.
- [runners.md](runners.md): which runner owns each task in a factory of several, leases,
  `priory give`, and answering another runner's ask.
- [agents-slice.md](agents-slice.md): the systemd slice that caps every agent's CPU and memory.
- [memory-gate.md](memory-gate.md): `priory memory acquire`, which queues heavy commands for memory.
- [sessions.md](sessions.md): shared agent sessions, attaching to a run's terminal, and trusting a
  clone for Claude.
- [codex.md](codex.md): how a Codex run differs from a Claude run.
- [signals.md](signals.md): the hook signals a run sends, the nudge after a silent turn, and the
  nudge's evaluation.
- [skills.md](skills.md): how the factory's skills, `instructions.md` and memories reach every
  run, the `[kit]` repository each runner fetches, and the hash and commit each run records.
- [writing.md](writing.md): the factory's writing guide, which every agent's system prompt carries,
  and its evaluation.
- [guard.md](guard.md): the tool guard, which denies irreversible tool calls no person approved.
- [health.md](health.md): run verdicts, failed or blocked tasks, repair, and the stall handler.
- [desk.md](desk.md): `priory desk`, reports and `priory post`, resume, cancel, revive, snooze, say,
  pause, and `priory seen` for Since you looked.
- [sweep.md](sweep.md): worktree removal, retiring asleep tasks, sweep scripts and the sweeper.
- [tools.md](tools.md): the MCP tools, proofs and `priory gate`, moving between steps, code steps,
  the `run` action and approvals.
- [waits.md](waits.md): asks and answers, wait conditions, and the checks gate before `done`.
- [asks.md](asks.md): delivering an outside ask's answer to the session that asked.
- [outbox.md](outbox.md): prompt states and how the daemon types a prompt.
- [reports.md](reports.md): reports of sections that a workflow writes, and the picks a person
  makes on them.
- [trace.md](trace.md): trace records, token counts, `priory report`, `priory costs`, the attention report, outcome labels, `priory grades`, `priory report risk` and the OpenTelemetry export.
- [score.md](score.md): `priory score`, which grades a task's finished runs with the factory's
  scorers.
- [bench.md](bench.md): `priory bench capture`, which turns a past task into a benchmark case.
- [otel-recipes.md](otel-recipes.md): how to send the export to a local ClickStack, Honeycomb or Grafana Cloud.
- [batches.md](batches.md): batches, `priory batch add`, `priory batch show` and `priory batch edit`, split issues, the plan a
  planning task sends, the go and the `merged` hold condition.
- [relations.md](relations.md): how tasks relate, `priory relate`, the relations Priory records
  itself, and `priory tasks --graph`.
- [search.md](search.md): TL;DRs, the summarizer, `priory search`, and the optional embeddings.
- [load.md](load.md): the load test, how to rerun it and the limits it measured.
- [serve.md](serve.md): the HTTP server over the state, its token and its limits.
- [services.md](services.md): running Priory as systemd user services, and the listing commands.
