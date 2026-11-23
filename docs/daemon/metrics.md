# Metrics

Priory can send its own health as OpenTelemetry metrics, beside the spans of
[the trace export](trace.md#opentelemetry-export). Spans say what a task did after its step ends.
Metrics say how each Priory process is doing every interval: how long its loop takes, how much
memory and CPU it uses, whether the daemon reaches its log, how long its runs take to start, and whether the intake
keeps up. [The metrics reference](../reference/metrics.md) lists every metric. The design and the groups still to come
are in [the metrics design note](../design/metrics.md).

## Switching it on

Metrics go to the same endpoint as spans, at `<otlp>/v1/metrics`, with the same headers. They are
off unless `factory.toml` names the groups to send:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"
metrics = ["process", "log", "runs", "intake"]   # the groups, by name
metrics_every = "60s"                            # how often each process sends
metrics_cap = 1000                               # data points per minute, per process
```

[`[telemetry]`](../reference/factory.md#telemetry) lists the keys. The groups are `process`, `log`,
`runs` and `intake`. `priory check` refuses an unknown group, a `metrics_every` under 10 seconds and a
`metrics_cap` of 0.

`PRIORY_METRICS` replaces `metrics` for one process, as a comma-separated list such as `process,log`,
or `none` to send nothing. A person turns metrics on for one daemon while chasing a problem, or off
for one, without editing the factory every runner shares. A process refuses to start when the
variable names an unknown group:

```text
priory daemon: PRIORY_METRICS names `nope`, which is not a metrics group: the groups are process, log, runs, intake, or none
```

## Which process sends what

The daemon, the intake and `priory serve` each send their own requests, from a thread of their own.
Each sends its first request when it starts, then one every `metrics_every`. `--once` sends no
metrics. `priory daemon --view-only` sends as `daemon`. A daemon with `[serve]` runs its server
inside itself, so its server requests count in the daemon's tick.

Each process sends only the groups it owns. All three send `process`, about themselves. Only the
daemon sends `log` and `runs`: the intake and `priory serve` skip them when `metrics` names them.
Each process sends its own part of `intake`: the intake its polls, and the process whose server
takes webhooks, the daemon or `priory serve`, its deliveries.

The resource of every request names the factory, the host, the runner and the process, as
[the reference](../reference/metrics.md#resource) lists. `service.name` is `priory`, as on spans, so
a backend joins both. No data point carries a task, run, ask or key id. Per-task detail lives in
spans.

## The process group

The process group says how the process itself is doing: its loop's tick, its memory, its CPU time
and its uptime. `PRIORY_PROC` names another folder for `/proc`, as it does for [the room check](room.md), and then
the process reads `self` there. A process that cannot read its own use logs it once and sends the
rest.

The tick is a gauge of three statistics over the interval, not a histogram. `p95` is the nearest
rank. `max` shows the one slow clone that held the daemon's thread for 40 seconds, which a
backend's p95 from buckets would hide.

Counters and histograms are cumulative from the process's start, with that start as
`startTimeUnixNano`. A restart starts them again with a new start time, which backends read as a
reset.

## The log group

The log group says whether the daemon reaches its log, and what waits while it does not. It shows a
runner cut off from the log for an hour ([runners.md](runners.md#a-runner-cut-off-from-the-log))
as a history, where the desk's banner shows only the moment.

- **The probe.** The daemon times the first read of its log in each tick, and counts it by outcome
  in `priory.log.probe.duration`: `ok`, `timeout` when the log gave no answer within
  [`[log] timeout`](../reference/factory.md#log), or `error`. While the log does not answer, the
  daemon tries again after 2 seconds, then twice as long each time up to a minute, so a long cut
  adds a probe a minute.
- **What waits.** `priory.log.unreachable.age` counts the seconds since the log stopped answering,
  as the desk's banner says `the log is unreachable since <time>`. `priory.log.queued` counts the
  messages that wait to publish.
- **The backlog.** Once a tick reaches the log, the daemon asks it how many messages wait past each
  consumer's cursor on `factory.intake`. A runner sees another runner's backlog grow while that one
  is cut off. A daemon that cannot reach its log sends no backlog.
- **The leases.** `priory.runner.lease.age` gives each runner's lease age in log time, as leases
  are compared ([runners.md](runners.md#holds-and-leases)): the newest alive time among the leases,
  less that runner's own. A cut-off runner's age climbs on the other runners. Past 1800 seconds,
  its waiting tasks can move. On the cut-off runner itself, log time stops, so the ages hold still.
- **The connection.** On Postgres, `priory.log.connection.wait` times how long each call waited
  for the log's one connection, which a call that hangs holds until its timeout.

## The runs group

The runs group says how long a run takes to reach Claude's or Codex's `SessionStart`, which starts
hang, and how long each part of a worktree's start takes.

- **The start.** `priory.run.start.duration` counts each run's start, from the second its row
  records as `started` to its `SessionStart`, by harness and by `first` or `resume`: whether the
  run resumed a session. A run that ends before any `SessionStart`, on the trust dialog, after 60
  seconds with none, or because its harness exited, counts as `error`, up to its end
  ([sessions.md](sessions.md#trusting-the-worktrees-and-the-clone)). A run that carries its process
  into the next step has no start, and counts nothing.
- **The hung start.** `priory.run.starts.active` counts the live runs with no `SessionStart` yet,
  and `priory.run.start.oldest.age` gives the oldest one's age. Each harness the factory's agents
  use has a point, 0 with none. A start that hangs on a dialog the screen check does not know climbs
  to 60 seconds every time.
- **The worktree.** `priory.worktree.phase.duration` times each phase of a task's start:
  `clone`, `fetch` and `checkout` around their `git` commands, and the repository's `setup`,
  `resume` and `teardown` recipes ([runs.md](runs.md#recipes)) from their start to their end. A
  `git` command past `command_timeout`, or a recipe past its `timeout`, counts as `timeout`. The
  clone, the fetch and the checkout run on the daemon's thread, so a slow one also shows in the
  tick's `max`.
- **The worktrees.** `priory.worktrees.active` counts the task folders under `worktrees/` in
  Priory home, as [the sweep](sweep.md) lists them.

The daemon reads the starts and the recipes from the state file. It counts those that ended since
it started, so a restart counts again from zero. The state keeps when each run's `SessionStart`
arrived, to the millisecond, its harness, and whether it resumed a session. A run from before
Priory kept them counts once its `SessionStart` arrives, by its agent's harness, as `first`.

## The intake group

The intake group says whether the intake keeps up with its sources, and whether webhooks arrive.

- **The events.** `priory.intake.events` counts each source event a poll or a webhook delivery
  publishes, by source, by `poll` or `webhook`, and by outcome. `published` is an event the log
  lacked. `duplicate` is one the other writer already reported: a webhook's redelivery, a
  delivery the poll already published, or a poll's event a delivery already published
  ([webhooks.md](../intake/webhooks.md#duplicates)). A poll that reads its own event again, as a
  schedule's slot or a watched Slack thread's replies, counts nothing. `refused` is a delivery the
  server refused for its signature, secret or passcode, or Linear's stale timestamp.
- **The missed webhooks.** `priory.intake.poll.missed` counts the events a poll published for a
  source whose webhook the factory takes, GitHub or Linear, of a type that webhook publishes. A
  factory that opened its server to GitHub expects it near zero. A rising count says deliveries
  stopped, and the poll caught them up to a minute late. A change that a poll reads in the second
  before its delivery arrives counts too.
- **The failures.** `priory.intake.poll.failures` counts each source's failed polls, `timeout`
  when one of its calls passed `command_timeout`, else `error`.
- **The poll age.** `priory.intake.poll.age` gives the seconds since each source's last good poll,
  read from the state, as the desk's [intake banner](desk.md#intake-banner) reads it. The intake
  sends it only with `PRIORY_DB` set.

The counters start at the process's start. To count its publishes, the process reads the newest
message on `factory.intake` before each one, and the stored message when the log already held its
id.

## The cap

A process counts the points it sent over the last minute. When the next request would go past
`metrics_cap`, it drops gauges first, then histograms, then counters, each from the group with
most of that kind, until the request fits. A cumulative counter or histogram carries its total in
its next point, so a dropped one loses a minute's resolution, not counts.

Each dropped point counts in `priory.metrics.dropped`, a counter by `priory.group` and `kind`, which
the cap never drops. The process logs one line for each interval that dropped:

```text
priory daemon: metrics past metrics_cap = 3: dropped 2 gauge points
```

## A failed send

A request the endpoint refuses with a status other than 2xx, or that gets no answer, is not retried.
Its counters arrive with their totals in the next request. It counts in `priory.metrics.dropped` with
`kind = send` and no group, and the process logs it once per error text:

```text
priory daemon: sending metrics to http://127.0.0.1:4318 failed (HTTP 503)
```

`curl` sends each request, as it does spans, and gives up after `command_timeout`. The request body
goes to `metrics-<service>.json` in Priory home, and the header lines to `otlp-headers-<service>`,
readable by its user only.
