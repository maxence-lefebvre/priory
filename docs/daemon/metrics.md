# Metrics

Priory can send its own health as OpenTelemetry metrics, beside the spans of
[the trace export](trace.md#opentelemetry-export). Spans say what a task did after its step ends.
Metrics say how each Priory process is doing every interval: how long its loop takes, how much
memory and CPU it uses, and whether the daemon reaches its log. [The metrics
reference](../reference/metrics.md) lists every metric. The design and the groups still to come
are in [the metrics design note](../design/metrics.md).

## Switching it on

Metrics go to the same endpoint as spans, at `<otlp>/v1/metrics`, with the same headers. They are
off unless `factory.toml` names the groups to send:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"
metrics = ["process", "log"]   # the groups, by name
metrics_every = "60s"          # how often each process sends
metrics_cap = 1000             # data points per minute, per process
```

[`[telemetry]`](../reference/factory.md#telemetry) lists the keys. The groups are `process` and
`log`. `priory check` refuses an unknown group, a `metrics_every` under 10 seconds and a
`metrics_cap` of 0.

`PRIORY_METRICS` replaces `metrics` for one process, as a comma-separated list such as `process,log`,
or `none` to send nothing. A person turns metrics on for one daemon while chasing a problem, or off
for one, without editing the factory every runner shares. A process refuses to start when the
variable names an unknown group:

```text
priory daemon: PRIORY_METRICS names `nope`, which is not a metrics group: the groups are process, log, or none
```

## Which process sends what

The daemon, the intake and `priory serve` each send their own requests, from a thread of their own.
Each sends its first request when it starts, then one every `metrics_every`. `--once` sends no
metrics. `priory daemon --view-only` sends as `daemon`. A daemon with `[serve]` runs its server
inside itself, so its server requests count in the daemon's tick.

Each process sends only the groups it owns. All three send `process`, about themselves. Only the
daemon sends `log`: the intake and `priory serve` skip it when `metrics` names it.

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
