# Metrics

Priory can send its own health as OpenTelemetry metrics, beside the spans of
[the trace export](trace.md#opentelemetry-export). Spans say what a task did after its step ends.
Metrics say how each Priory process is doing every interval: how long its loop takes, how much
memory and CPU it uses, and how long it has run. The design and the groups still to come are in
[the metrics design note](../design/metrics.md).

## Switching it on

Metrics go to the same endpoint as spans, at `<otlp>/v1/metrics`, with the same headers. They are
off unless `factory.toml` names the groups to send:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"
metrics = ["process"]   # the groups, by name
metrics_every = "60s"   # how often each process sends
metrics_cap = 1000      # data points per minute, per process
```

[`[telemetry]`](../reference/factory.md#telemetry) lists the keys. `process` is the only group so
far. `priory check` refuses an unknown group, a `metrics_every` under 10 seconds and a
`metrics_cap` of 0.

`PRIORY_METRICS` replaces `metrics` for one process, as a comma-separated list such as `process`,
or `none` to send nothing. A person turns metrics on for one daemon while chasing a problem, or off
for one, without editing the factory every runner shares. A process refuses to start when the
variable names an unknown group:

```text
priory daemon: PRIORY_METRICS names `nope`, which is not a metrics group: the groups are process, or none
```

## Which process sends what

The daemon, the intake and `priory serve` each send their own requests, from a thread of their own.
Each sends its first request when it starts, then one every `metrics_every`. `--once` sends no
metrics. `priory daemon --view-only` sends as `daemon`. A daemon with `[serve]` runs its server inside itself, so its server requests count in the
daemon's tick.

The resource of every request carries:

| Attribute | Value |
| --------- | ----- |
| `service.name` | `priory`, as on spans, so a backend joins both |
| `priory.service` | `daemon`, `intake` or `serve`: the process that sent it |
| `priory.factory` | `name` in `factory.toml` |
| `priory.runner` | the runner's name in a factory of several, else absent |
| `host.name` | the machine, as on spans |
| `service.instance.id` | a UUID the process makes when it starts, so two processes of one kind on one machine stay apart |

No data point carries a task, run, ask or key id. Per-task detail lives in spans.

## The process group

| Metric | Type | Attributes | Source |
| ------ | ---- | ---------- | ------ |
| `priory.process.tick.duration` | gauge, s | `stat` = `avg`, `p95`, `max` | each loop tick in the interval: the daemon's 1-second tick, the intake's poll, a server request. Absent from an interval with no tick, such as the first |
| `process.memory.usage` | gauge, bytes | | `VmRSS` in `/proc/self/status` |
| `process.cpu.time` | counter, s | `cpu.mode` = `user`, `system` | `utime` and `stime` in `/proc/self/stat`, at 100 ticks a second |
| `process.uptime` | gauge, s | | since the process started |

`PRIORY_PROC` names another folder for `/proc`, as it does for [the room check](room.md), and then
the process reads `self` there. A process that cannot read its own use logs it once and sends the
rest.

The tick is a gauge of three statistics over the interval, not a histogram. `p95` is the nearest
rank. `max` shows the one slow clone that held the daemon's thread for 40 seconds, which a
backend's p95 from buckets would hide.

Counters are cumulative from the process's start, with that start as `startTimeUnixNano`. A
restart starts them again with a new start time, which backends read as a reset.

## The cap

A process counts the points it sent over the last minute. When the next request would go past
`metrics_cap`, it drops gauges first, from the group with most of them, then counters, until the
request fits. A cumulative counter carries its total in its next point, so a dropped counter loses a
minute's resolution, not counts.

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
