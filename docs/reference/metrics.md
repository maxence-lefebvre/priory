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
| `intake` | the intake, for its polls, and the daemon or `priory serve`, for the webhook deliveries its server takes. Only the intake sends `priory.intake.poll.age` |
| `work` | the daemon only, as for `log` |
| `room` | the daemon only, as for `log`. A daemon with `--view-only` starts no run and sends none |

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

## intake

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.intake.events` | counter | `{event}` | `priory.source` = `github`, `linear`, `slack`, `schedule`, `sentry`, `figma`, `cloudwatch`, `honeycomb`, `intercom`; `priory.via` = `poll`, `webhook`; `outcome` = `published`, `duplicate`, `refused` | each source event the process published or turned away. `published`: the log lacked it. `duplicate`: the log held its id, or the other writer's report of the same change. A poll counts `duplicate` only for a webhook's report, not for an event it reads again. `refused`: a delivery the server refused for its signature, secret, passcode or a stale Linear timestamp. Task requests and markers count nothing |
| `priory.intake.poll.missed` | counter | `{event}` | `priory.source` = `github`, `linear` | the events a poll published, not as a duplicate, of a type the factory's webhook for that source publishes: for GitHub, an issue opened or labeled, a pull request opened, labeled or closed, a review or a pull request comment, on a repository a trigger names; for Linear, a created issue, a state change, or a comment a trigger requests. Never from a first poll's baseline |
| `priory.intake.poll.failures` | counter | `{poll}` | `priory.source` = `github`, `linear`, `slack`; `reason` = `timeout`, `error` | each poll of a source that failed. `timeout` when one of its calls ended with `timed out after <n>s` |
| `priory.intake.poll.age` | gauge | `s` | `priory.source` | from the state's `intake_sources`: since the source's last poll, or since it started failing while it fails. Absent without `PRIORY_DB` |

A counter has no point until it counts once, so a process that took no delivery sends no
`priory.intake.events`.

## work

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.asks.open` | gauge | `{ask}` | `priory.ask.kind` = `step`, `agent`, `outside`; `priory.lane`, absent for an ask with no lane | the open asks of tasks that have not ended, and outside asks, less those snoozed now. A series sent before sends 0 once it has none |
| `priory.asks.oldest.age` | gauge | `s` | `priory.ask.kind` | since the oldest of those asks was asked, in its current revision. Each kind has a point, 0 with none |
| `priory.attention.per_merge` | gauge | `min` | | `person_minutes_per_merge` of `priory report attention` over its default window, this week from Monday. Read at the first interval and every 10th after, the last value sent in between. Absent while the window has no merge |
| `priory.tasks` | gauge | `{task}` | `priory.state`; `priory.workflow` | the tasks in each state other than `done`, `failed` and `dropped`. A series sent before sends 0 once it has none |
| `priory.tokens` | counter | `{token}` | `priory.workflow`; `gen_ai.agent.name`; `gen_ai.request.model`; `gen_ai.token.type` = `input`, `output`, `cache_read`, `cache_write` | the tokens of the trace records appended since the daemon started that `priory costs` prices: model steps, debate turns, handler calls and summarizer calls. The agent is `handler` or `summarizer` for those calls. An attribute the record lacks is absent |
| `priory.cost` | counter | `USD` | `priory.workflow`; `gen_ai.agent.name`; `gen_ai.request.model` | the same records priced with `prices.toml`. A model with no price adds no cost |
| `priory.spend.headroom` | gauge | `USD` | `priory.cap.scope` = `factory`, `trigger`, `lane`; `priory.cap.name`, the trigger or lane, absent for `factory`; `priory.cap.window` = `task`, `day`, `week` | each spend cap's limit less what it spent, as `priory costs` lists its caps. A `task` cap uses the most an active task under it spent. Below 0 past the cap |

## room

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.room.load` | gauge | `1` | | the 1-minute load |
| `priory.room.load.limit` | gauge | `1` | | the cpu share × the CPUs the daemon may use. A load at or above it admits no run |
| `priory.room.memory.available` | gauge | `By` | | `MemAvailable` |
| `priory.room.memory.total` | gauge | `By` | | `MemTotal` |
| `priory.room.memory.limit` | gauge | `By` | | the memory share × `MemTotal`: what agents may take together |
| `priory.room.memory.promised` | gauge | `By` | | what live runs declared and do not use yet, without the memory grants hold |
| `priory.room.disk.free` | gauge | `By` | | free disk on `PRIORY_HOME` |
| `priory.room.disk.floor` | gauge | `By` | | the free disk the room keeps |
| `priory.room.grants` | gauge | `{grant}` | `priory.grant.state` = `waiting`, `held` | the memory gate's grants. Each state has a point, 0 with none |
| `priory.room.grants.memory` | gauge | `By` | `priory.grant.state` | the bytes those grants ask for or hold |
| `priory.room.tasks.waiting` | gauge | `{task}` | `priory.room.cause` = `load`, `memory`, `disk`, `unmeasured` | the tasks waiting for room. A task behind the queue's head counts under the cause of the head. `unmeasured` is a room that cannot be measured. Each cause has a point, 0 with none. Absent while the head to start waits for its claim to reach the log |

The daemon's thread fills the group, so a daemon's first request may hold none of it. While the room
cannot be measured, the group sends only the grants, the promised memory and the tasks waiting.

## Dropped points

| Metric | Kind | Unit | Attributes | Value |
| ------ | ---- | ---- | ---------- | ----- |
| `priory.metrics.dropped` | counter | `{point}` | `priory.group` = a group, absent for a send; `kind` = `gauge`, `histogram`, `counter`, `send` | the points `metrics_cap` dropped, and the requests that failed. The cap never drops it |

## Errors

| Message | Cause |
| ------- | ----- |
| `` priory <process>: PRIORY_METRICS names `<group>`, which is not a metrics group: the groups are process, log, runs, intake, work, room, or none `` | `PRIORY_METRICS` names an unknown group. The process does not start |
| `` priory <process>: metrics past metrics_cap = <n>: dropped <count> <kind> points `` | an interval had more points than the cap left room for |
| `` priory <process>: sending metrics to <otlp> failed (<error>) `` | the endpoint refused a request or did not answer, logged once per error text |
| `` priory <process>: metrics: cannot read the process's own use: <error> `` | the process could not read `/proc/self`, or `self` in `PRIORY_PROC`. The interval sends the rest |
| `` priory daemon: metrics: cannot read the log's state: <error> `` | the daemon's metrics thread could not read the state file. The interval sends the rest |
| `` priory daemon: metrics: cannot read the runs' state: <error> `` | the same, for the `runs` group. The interval sends the rest |
| `` priory daemon: metrics: cannot measure the room: <error> `` | the daemon could not read the machine's memory, load or disk. The interval sends the grants, the promised memory, the tasks waiting and the rest |
| `` priory daemon: metrics: cannot read the factory's work: <error> `` | the daemon's metrics thread could not read the state file, or `PRIORY_NOW` does not parse. The interval sends no `work` point and the rest |
| `` priory intake: metrics: cannot read the intake's polls: <error> `` | the intake's metrics thread could not read the state file named by `PRIORY_DB`. The interval sends the rest |
