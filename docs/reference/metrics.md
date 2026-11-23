# Metrics

Every metric a Priory process sends to `<otlp>/v1/metrics`, by group. The
[`[telemetry]`](factory.md#telemetry) keys `metrics`, `metrics_every` and `metrics_cap` switch them
on. [metrics.md](../daemon/metrics.md) explains how each process samples and sends them.

## Groups

| Group | Sent by |
| ----- | ------- |
| `process` | the daemon, the intake and `priory serve`, each about itself |
| `log` | the daemon only. The intake and `priory serve` skip it when `metrics` or `PRIORY_METRICS` names it |
| `runs` | the daemon only, as for `log` |

## Resource

Every request carries these attributes on its resource.

| Attribute | Value |
| --------- | ----- |
| `service.name` | `priory` |
| `priory.service` | `daemon`, `intake` or `serve`: the process that sent it |
| `priory.factory` | `name` in `factory.toml` |
| `priory.runner` | the runner's name in a factory of several, else absent |
| `host.name` | the machine |
| `service.instance.id` | a UUID the process makes when it starts |

## Kinds

| Kind | OTLP shape |
| ---- | ---------- |
| gauge | `gauge`, the value now |
| counter | `sum`, cumulative and monotonic, with the process's start as `startTimeUnixNano` |
| histogram | `histogram`, cumulative, with the process's start as `startTimeUnixNano`. Every histogram has the bounds `0.005`, `0.01`, `0.025`, `0.05`, `0.1`, `0.25`, `0.5`, `1`, `2.5`, `5`, `10`, `30` and `60` seconds, and a last bucket for the rest |

## process

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.process.tick.duration` | gauge | `s` | `stat` = `avg`, `p95`, `max` | the loop ticks of the interval: the daemon's 1-second tick, the intake's poll, a server request. Absent from an interval with no tick |
| `process.memory.usage` | gauge | `By` | | `VmRSS` in `/proc/self/status` |
| `process.cpu.time` | counter | `s` | `cpu.mode` = `user`, `system` | `utime` and `stime` in `/proc/self/stat`, at 100 ticks a second |
| `process.uptime` | gauge | `s` | | since the process started |

## log

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.log.probe.duration` | histogram | `s` | `priory.log.backend` = `herdr-bus`, `postgres`; `outcome` = `ok`, `timeout`, `error` | the first read of the log in each daemon tick. `timeout` is no answer within `[log] timeout`, or a call refused because an earlier one still waits. `error` is any other failure |
| `priory.log.backlog` | gauge | `{message}` | `priory.log.output` = `factory.intake`; `priory.log.consumer` | the messages past each consumer's cursor. Absent from an interval in which the daemon did not reach its log |
| `priory.log.unreachable.age` | gauge | `s` | | since the log stopped answering; 0 while it answers |
| `priory.log.queued` | gauge | `{message}` | | the messages that wait to publish |
| `priory.runner.lease.age` | gauge | `s` | `priory.lease.runner` | in log time: the newest alive time among the runners' leases, less this runner's. Absent in a factory of one runner |
| `priory.log.connection.wait` | histogram | `s` | | Postgres only: how long each call waited for the log's one connection |

A consumer on herdr-bus is named as it reads, such as `priory.a`, when it reads on this machine,
and as `<name>@<machine id>` when it reads on another.

## runs

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.run.start.duration` | histogram | `s` | `priory.harness` = `claude-code`, `codex`; `priory.session` = `first`, `resume`; `outcome` = `ok`, `error` | from a run's `started`, to the second, to its `SessionStart`. `error` is a run that ended before one, up to its end: the trust dialog, 60 seconds with none, an exit. Counts the runs whose start ended since the daemon started |
| `priory.run.starts.active` | gauge | `{run}` | `priory.harness` | the live runs with no `SessionStart` yet. Each harness the factory's agents use has a point, 0 with none |
| `priory.run.start.oldest.age` | gauge | `s` | `priory.harness` | the oldest of those, since its `started`; 0 with none |
| `priory.worktree.phase.duration` | histogram | `s` | `priory.phase` = `clone`, `fetch`, `checkout`, `setup`, `resume`, `teardown`; `outcome` = `ok`, `error`, `timeout` | `clone`, `fetch` and `checkout` timed around their `git` commands as a task's worktree is added. `setup`, `resume` and `teardown` from the recipe's start to its end, `ok` for `exit 0`. `timeout` is a command past `command_timeout`, or a recipe past its `timeout` |
| `priory.worktrees.active` | gauge | `{worktree}` | | the task folders under `worktrees/` in Priory home, as the sweep lists them |

## Dropped points

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.metrics.dropped` | counter | `{point}` | `priory.group` = a group, absent for a send; `kind` = `gauge`, `histogram`, `counter`, `send` | the points `metrics_cap` dropped, and the requests that failed. The cap never drops it |

## Errors

| Message | Cause |
| ------- | ----- |
| `` priory <process>: PRIORY_METRICS names `<group>`, which is not a metrics group: the groups are process, log, runs, or none `` | `PRIORY_METRICS` names an unknown group. The process does not start |
| `` priory <process>: metrics past metrics_cap = <n>: dropped <count> <kind> points `` | an interval had more points than the cap left room for |
| `` priory <process>: sending metrics to <otlp> failed (<error>) `` | the endpoint refused a request or did not answer, logged once per error text |
| `` priory <process>: metrics: cannot read the process's own use: <error> `` | the process could not read `/proc/self`, or `self` in `PRIORY_PROC`. The interval sends the rest |
| `` priory daemon: metrics: cannot read the log's state: <error> `` | the daemon's metrics thread could not read the state file. The interval sends the rest |
| `` priory daemon: metrics: cannot read the runs' state: <error> `` | the same, for the `runs` group. The interval sends the rest |
