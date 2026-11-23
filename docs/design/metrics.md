# Metrics over OpenTelemetry

This page explains how Priory could export its own health as OpenTelemetry metrics over OTLP: how
long its loops take, whether its log answers, how long a run takes to start, whether the intake
keeps up, and what waits on a person. It holds the model, the first code slice with its tests,
the slices after it, what stays out, and the open questions with a recommendation for each.

Every slice is built. [metrics.md](../daemon/metrics.md) describes the first one: the exporter,
the keys, `PRIORY_METRICS`, the cap and the `process` group. Its tests use a fake collector on
127.0.0.1 rather than a fake `curl`. The `log`, `runs`, `intake` and `work` groups follow in
[metrics.md](../daemon/metrics.md#the-log-group), and [the metrics reference](../reference/metrics.md)
lists every metric. The how-to is [How to send metrics to an OpenTelemetry backend](../sending-metrics.md).

Max proposed it on 2026-11-22, after Mastra Factory's
[health metrics page](https://factory.mastra.ai/configure/metrics). This note takes the idea of
health metrics a factory exports and nothing else. Every metric below comes from something Priory
already records or already measures, named under each group.

## Why

The trace answers what a task did and cost, after the step ends ([trace.md](../daemon/trace.md)).
It cannot say that the daemon's loop takes 3 seconds a tick, that Postgres answers in 8 seconds of
its 10, that the intake has not polled cleanly for an hour, or that a run has waited 50 seconds for
Claude's `SessionStart`. Priory knows each of these at a moment, and shows some of them on the
desk as a banner ([desk.md](../daemon/desk.md#intake-banner)). A banner is there or not. It has no
history, no trend and no alert a person sets.

M3's done-when line is a runner cut off from the log for an hour, then back, with no task run twice
([PRD](../prd.md#milestones)). Watching that happen needs the log's probe times, its backlog and
each runner's lease age over the hour, not one banner at the end. The PRD's success measures
(attention per merge, memory held by waiting agents) are reports today: a person runs
`priory report attention` to read one. A metric draws it every minute.

## What existed when this note was written

- `[telemetry] otlp` sends `factory.step.ended` records as spans to `<otlp>/v1/traces`, as OTLP
  JSON through `curl`, with `headers_env` for a backend's key
  ([factory.md](../reference/factory.md#telemetry), [trace.md](../daemon/trace.md#opentelemetry-export)).
  `crates/priory-core/src/otlp.rs` builds the JSON by hand, in 479 lines, with no OpenTelemetry
  crate. The resource carries `service.name = priory`, `priory.factory` and `host.name`.
- The daemon runs one thread that ticks every second ([README](../daemon/README.md)). A start's
  clone, fetch and `git worktree add` run on that thread (`worktree` in
  `crates/priory-runtime/src/runtime.rs`), bounded by `command_timeout`
  ([home.md](../daemon/home.md#command-timeout)). The intake polls every 60 seconds
  ([intake README](../intake/README.md)). `priory serve`, or the daemon's own server, answers on
  threads with read-only connections ([serve.md](../daemon/serve.md)).
- The room check reads `/proc/meminfo` and `/proc/loadavg`, or the folder `PRIORY_PROC` names
  ([room.md](../daemon/room.md)). Nothing reads the process's own memory or CPU.
- The log is herdr-bus or Postgres ([factory.md](../reference/factory.md#log)). The Postgres log
  holds one connection behind a mutex, with no pool. A call that outlives `[log] timeout` is given
  up on, and until it ends every later call gives up at once with
  `a call got no answer within <n>s and still waits` (`run` in
  `crates/priory-intake/src/log/postgres.rs`). `Unreachable` marks a failure that a later call may
  not have.
- The state's `log_link` row holds since when, and why, the log is unreachable, and `sends` the
  messages that wait to publish. The desk shows both as
  `the log is unreachable since <time>: <error>; <n> messages wait to publish`
  ([runners.md](../daemon/runners.md#a-runner-cut-off-from-the-log)).
- `leases` holds each runner's last alive message, in log time. A lease ends 30 minutes after it
  ([runners.md](../daemon/runners.md#holds-and-leases)). Postgres keeps each consumer's cursor in
  `priory_log_cursors`, and herdr-bus its own.
- A run's row has `started` and `ended`. `SessionStart` sets the run idle and records its
  transcript, but no time. The daemon reads the screen of each run with no `SessionStart` every
  second: Claude's trust dialog blocks the task at once, and 60 seconds with no `SessionStart`
  blocks it too ([sessions.md](../daemon/sessions.md#trusting-the-worktrees-and-the-clone),
  [health.md](../daemon/health.md)). Whether a run resumes a session is known at its start
  ([runs.md](../daemon/runs.md)).
- The `recipes` table holds each `setup`, `resume` and `teardown` with `started`, `ended` and
  `status` ([runs.md](../daemon/runs.md#recipes)). Clone, fetch and checkout record no time.
- `intake_sources` holds each polled source's last poll, its error and `failing_since`. The desk
  shows a failing source, and an intake that has not polled for 5 minutes
  ([desk.md](../daemon/desk.md#intake-banner)). A webhook and a poll that see one change publish it
  once: herdr-bus drops a repeated id, and the other types look for the other writer's report first
  ([webhooks.md](../intake/webhooks.md#duplicates)).
- `priory report attention` computes person-minutes per merge and ask waits
  ([trace.md](../daemon/trace.md#attention)). `priory costs` prices tokens by workflow, agent and
  model, and lists each spend cap with what it spent ([trace.md](../daemon/trace.md#costs),
  [spend.md](../daemon/spend.md)).

## The model

### Switching it on

Metrics go to the same endpoint as spans, at `<otlp>/v1/metrics`, with the same headers. They are
off unless `factory.toml` names the groups to send:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"
metrics = ["process", "log", "runs", "intake", "work"]   # the groups, by name
metrics_every = "60s"                                     # how often each process sends
metrics_cap = 1000                                        # data points per minute, per process
```

| Key | Default | Meaning |
| --- | ------- | ------- |
| `metrics` | `[]`: no metric is sent | The groups each process sends. `priory check` refuses a name it does not know |
| `metrics_every` | `"60s"` | How often a process samples its groups and sends one request. At least `"10s"` |
| `metrics_cap` | `1000` | The data points a process sends per minute at most, counted over a rolling minute |

`PRIORY_METRICS` replaces `metrics` for one process, as a comma-separated list such as
`process,log`, or `none`. A person turns a group on for one daemon while chasing a problem, or
turns all of them off, without editing the factory every runner shares. A process refuses to start
when the variable names an unknown group, as it does for a malformed `headers_env`. `--once` sends
no metrics.

Each process sends only the groups it owns:

| Group | Sent by |
| ----- | ------- |
| `process` | the daemon, the intake and `priory serve`, each about itself |
| `log` | the daemon |
| `runs` | the daemon |
| `intake` | the intake, and whichever process takes webhooks for its deliveries |
| `work` | the daemon |

### What a data point carries

The resource of every request carries:

| Attribute | Value |
| --------- | ----- |
| `service.name` | `priory`, as on spans, so a backend joins both |
| `priory.service` | `daemon`, `intake` or `serve`: the process that sent it |
| `priory.factory` | `name` in `factory.toml` |
| `priory.runner` | the runner's name in a factory of several, else absent |
| `host.name` | the machine, as on spans |
| `service.instance.id` | a UUID the process makes when it starts, so two processes of one kind on one machine stay apart |

A data point's attributes never carry a task, run, ask or key id. Each attribute takes values from
a set the factory bounds: its workflows, agents, models, lanes, sources, runners and outputs. That
keeps a minute's points near a few hundred for a factory of 5 workflows, 4 agents and 2 models.
Per-task detail lives in spans, which carry those ids.

Counters are cumulative from the process's start, as OTLP's default temporality is. A restart
starts them again with a new start time, which backends read as a reset. Durations are histograms
in seconds with explicit buckets, except the loop's tick, which needs avg, p95 and max per interval
(see Process). Names follow OpenTelemetry's semantic conventions where one exists, such as
`process.memory.usage`, as R10.2 asks of spans, and start with `priory.` otherwise.

### The cap

A process counts the points it sent over the last minute. When the next request would go past
`metrics_cap`, it drops points in this order until it fits:

1. gauges, the group with most points first. The next interval sends the current value again, and
   nothing sums a gauge's history.
2. histograms, the same way.
3. counters last. A cumulative counter carries its total in its next point, so a gap loses a
   minute's resolution, not counts.

Each drop counts in `priory.metrics.dropped`, a counter by `priory.group` and `kind`, which the cap
never drops. A process logs one line per interval that dropped, with the counts. A request the
endpoint refuses, or that gets no answer, is not retried: its counters arrive in the next one. It
counts in `priory.metrics.dropped` with `kind = send`, and is logged once per error text.

### Process

From `/proc/self`, or `PRIORY_PROC`'s `self` as the tests use, and the process's own loop.

| Metric | Type | Attributes | Source |
| ------ | ---- | ---------- | ------ |
| `priory.process.tick.duration` | gauge, s | `stat` = `avg`, `p95`, `max` | each loop tick in the interval, timed around the whole tick: the daemon's 1-second tick, the intake's poll, a server request |
| `process.memory.usage` | gauge, bytes | | `VmRSS` in `/proc/self/status` |
| `process.cpu.time` | counter, s | `cpu.mode` = `user`, `system` | `utime` and `stime` in `/proc/self/stat` |
| `process.uptime` | gauge, s | | since the process started |

The tick is a gauge of three statistics, computed by the process over the interval's ticks, rather
than a histogram: at 60 ticks a minute the exact p95 is cheap, and a backend's p95 from buckets
would hide the one slow clone that held the daemon's thread for 40 seconds. `max` shows it.

### Log

From the daemon's calls to its log, `log_link`, `sends` and `leases`.

| Metric | Type | Attributes | Source |
| ------ | ---- | ---------- | ------ |
| `priory.log.probe.duration` | histogram, s | `priory.log.backend` = `herdr-bus`, `postgres`; `outcome` = `ok`, `timeout`, `error` | the first `read` of each tick, timed. `timeout` is an `Unreachable` with no answer within `[log] timeout`, or one refused because an earlier call still waits. `error` is any other `Unreachable` |
| `priory.log.backlog` | gauge, messages | `priory.log.output`, `priory.log.consumer` | the messages on each output past the consumer's cursor. `factory.intake` is the only output today |
| `priory.log.unreachable.age` | gauge, s | | now less `log_link.since`; 0 while the log answers |
| `priory.log.queued` | gauge, messages | | the rows of `sends`, which the banner counts as `messages wait to publish` |
| `priory.runner.lease.age` | gauge, s | `priory.lease.runner` | the last log time read less each runner's `leases.alive`. Past 1800 s, its waiting tasks can move |
| `priory.log.connection.wait` | histogram, s | | Postgres only: the time a call waits for the connection's mutex before it runs |

The brief for this note asked for Postgres pool waits. The log has no pool: one connection, one call
at a time. The wait for that one connection is the same signal, and a `timeout` probe outcome
counts the calls the busy flag refuses.

The lease age compares log time, as leases do, never the reader's clock. On herdr-bus each alive
message keeps its sender's clock, so a runner whose clock runs ahead shows a young lease
([runners.md](../daemon/runners.md#holds-and-leases)).

### Runs

From the `runs` and `recipes` tables, and timers around the start's git commands.

| Metric | Type | Attributes | Source |
| ------ | ---- | ---------- | ------ |
| `priory.run.start.duration` | histogram, s | `priory.harness`, `priory.session` = `first`, `resume`; `outcome` = `ok`, `error` | from the run's `started` to its `SessionStart`. `error` is a run that ended before one: the trust dialog, 60 seconds with none, an exit |
| `priory.run.starts.active` | gauge, runs | `priory.harness` | live runs with no `SessionStart` yet |
| `priory.run.start.oldest.age` | gauge, s | `priory.harness` | the oldest of those, since its `started`; 0 with none |
| `priory.worktree.phase.duration` | histogram, s | `priory.phase` = `clone`, `fetch`, `checkout`, `setup`, `resume`, `teardown`; `outcome` = `ok`, `error`, `timeout` | clone, fetch and checkout timed around their `git` commands; setup, resume and teardown from the `recipes` row's `started`, `ended` and `status` |
| `priory.worktrees.active` | gauge, worktrees | | the worktree folders under `worktrees/` in Priory home, as the sweep lists them |

`priory.run.start.oldest.age` is the hung start. Claude's trust dialog blocks its task within a
second, but a start that hangs on something the screen check does not know, such as a new dialog in
a later Claude Code, climbs to 60 seconds every time. A start's duration needs the time of
`SessionStart`, which the run's row does not keep. The slice that builds this group adds a column,
so it needs a migration, numbered after the ones open pull requests reserve.

Clone, fetch and checkout run on the daemon's thread, so a slow one also shows in the tick's `max`.
The phase says which.

### Intake

From the intake's polls, its publishes and the server's webhook deliveries.

| Metric | Type | Attributes | Source |
| ------ | ---- | ---------- | ------ |
| `priory.intake.events` | counter, events | `priory.source` = `github`, `linear`, `slack`, `schedule`, `sentry`, and so on; `priory.via` = `poll`, `webhook`; `outcome` = `published`, `duplicate`, `refused` | each event the intake or the server publishes. `duplicate` is one herdr-bus dropped, or one whose other writer's report the look found. `refused` is a webhook whose signature or secret failed |
| `priory.intake.poll.missed` | counter, events | `priory.source` | events a poll published, not as a duplicate, for a source whose webhooks the factory takes: a webhook that never arrived |
| `priory.intake.poll.failures` | counter, polls | `priory.source`; `reason` = `timeout`, `error` | each failed poll, `timeout` for `timed out after <n>s` |
| `priory.intake.poll.age` | gauge, s | `priory.source` | now less the last poll with no error: `intake_sources.polled`, or `failing_since` while it fails |

`priory.intake.poll.missed` tells whether webhooks work. A factory that took the trouble to open its
server to GitHub expects the count near zero. A rising count says deliveries stopped, and the poll
caught them a minute late.

### Work

The `work` group is the factory's own state: what Priory does, not how its processes feel. The
daemon samples it from the state file, with the queries its reports already use.

| Metric | Type | Attributes | Source |
| ------ | ---- | ---------- | ------ |
| `priory.asks.open` | gauge, asks | `priory.ask.kind` = `step`, `agent`, `outside`; `priory.lane` | the asks waiting on a person, as the desk lists them, less snoozed ones |
| `priory.asks.oldest.age` | gauge, s | `priory.ask.kind` | the oldest open ask's current revision, since it was asked |
| `priory.attention.per_merge` | gauge, min | | `per merge` of `priory report attention` over its default window, this week from Monday |
| `priory.tasks` | gauge, tasks | `priory.state`, `priory.workflow` | the tasks in each state, active states only; ended tasks count in spans |
| `priory.tokens` | counter, tokens | `priory.workflow`, `gen_ai.agent.name`, `gen_ai.request.model`, `gen_ai.token.type` = `input`, `output`, `cache_read`, `cache_write` | the tokens of trace records appended since the process started, as `priory costs` reads them |
| `priory.cost` | counter, USD | `priory.workflow`, `gen_ai.agent.name`, `gen_ai.request.model` | the same records priced with `prices.toml`. A model with no price adds tokens and no cost |
| `priory.spend.headroom` | gauge, USD | `priory.cap.scope` = `factory`, `trigger`, `lane`; `priory.cap.name`; `priory.cap.window` = `task`, `day`, `week` | each cap's limit less what it spent, as the caps table of `priory costs` reads it. A `task` cap uses the most an active task under it spent. Below 0 past the cap |

Attention costs a query over the week's acts and asks. The daemon samples it once every 10
intervals and sends the last value in between.

## How to send OTLP

Two ways send these points to `<otlp>/v1/metrics`.

**OpenTelemetry's Rust SDK.** `opentelemetry` 0.33.1, `opentelemetry_sdk` 0.33.1 and
`opentelemetry-otlp` 0.33.1, released 2026-10-09, each Apache-2.0. `opentelemetry-otlp`'s
`http-json` feature sends OTLP JSON, and its default HTTP client is `reqwest-blocking-client`,
which adds `reqwest` 0.13.5 (2026-09-08, MIT OR Apache-2.0) and the tree under it. The SDK gives
instruments, aggregation and a periodic reader on its own thread. Its cardinality limit folds
excess series into one overflow series. It has no per-minute cap that drops gauges first, so that
rule would wrap it. The crates are pre-1.0 and change their API between minor releases. Priory
would then build spans by hand and metrics through the SDK, with two HTTP clients: `curl` and
`reqwest`.

**Hand-written OTLP/HTTP JSON.** A module beside `otlp.rs` keeps a few counters, gauges and
histograms per process, writes one `ExportMetricsServiceRequest` as JSON per interval, and sends it
with `curl` and the same header file, as spans go now. The JSON needs a resource, a scope, and
three point shapes: gauge, cumulative sum and explicit-bucket histogram. The cap and the drop order
are Priory's own rule, in Priory's code. It adds no dependency.

**Recommendation: hand-written JSON.** Spans already go this way, and a local ClickStack read them back
([the how-to](../sending-traces-to-opentelemetry.md)). The metrics here are about 25 instruments with
bounded attributes, sampled at an interval: the SDK's aggregation solves a harder problem than
Priory has. One transport, one header file and one set of retry rules is less to test. The SDK is
the rejected crate, for its size, its second HTTP client and its unstable API. If Priory later wants
gRPC or protobuf, `opentelemetry-proto` 0.33.1 (Apache-2.0, 2026-10-09) gives the message types
alone, with `prost` 0.14.6 (Apache-2.0, 2026-11-12).

## The first slice: the exporter and the process group

1. The keys `metrics`, `metrics_every` and `metrics_cap` in `[telemetry]`, in the schema, with
   `priory check` refusing an unknown group, an interval under 10 seconds and a cap under 1.
2. `PRIORY_METRICS`, read when each process starts, and its refusal of an unknown group.
3. The exporter: one request per interval to `<otlp>/v1/metrics` through `curl` with the header
   file, the resource above, the cap, the drop order and `priory.metrics.dropped`.
4. The `process` group in the daemon, the intake and `priory serve`.
5. The keys in [factory.md](../reference/factory.md#telemetry), the export in
   [trace.md](../daemon/trace.md), or a page of its own linked from the daemon's README, and the
   schema in `docs/schemas/` regenerated.

Tests, through the CLI with a fake `curl` on `PATH` that records each request body, as the span
export's tests do, and a `PRIORY_PROC` folder for `/proc/self`:

- With `metrics = ["process"]`, a daemon sends a request whose resource names the factory, the
  host and `priory.service = daemon`, with the memory and CPU from the fake `/proc/self`. Without
  `metrics`, no request goes to `/v1/metrics`, and spans still go to `/v1/traces`.
- `PRIORY_METRICS=none` sends nothing. `PRIORY_METRICS=nope` stops the daemon with the group's
  name.
- With `metrics_cap = 3` and more points than that, the request holds counters, not gauges, and
  the next request's `priory.metrics.dropped` counts the gauges dropped.
- A fake `curl` that fails: the next request's counters carry the totals, and the drop counter
  counts one `send`.
- The intake and `priory serve` each send their own request with their `priory.service`.
- `priory check` names each refused key value in its line.

## Later slices

1. **Built.** The `log` group. M3's done-when needs it first: a runner cut off for an hour shows as
   the probe's outcomes, the unreachable age, the queued messages and the other runners' lease
   ages. Tests use the existing Setup with runners a and b on one log, cut b's link, and wait for
   the state to say it is unreachable before they read the request. As built, the `Log` trait has
   `backlog`, which herdr-bus answers from each connection's `lag`. The lease age takes the newest
   alive time among the leases as the last log time read, so it needs no new column. The cap drops
   histograms after gauges and before counters, and the intake and `priory serve` skip the group.
2. **Built.** The `runs` group, with the `SessionStart` time on the run's row and its migration.
   Tests use the fake `claude`, one that never sends `SessionStart`, and one that shows the trust
   dialog. As built, migration 106 adds `session_started`, `harness` and `resumes` to `runs`. The
   daemon reads ended starts and recipes from the
   state file each interval, counting those that ended since it started, and `runtime::worktree`
   times clone, fetch and checkout. A start's duration begins at `started`, which the row keeps to
   the second.
3. **Built.** The `intake` group, with the duplicate and missed counts from the publish path.
   Tests run the intake with a fake `gh` after a baseline poll, a failing `gh`, and `priory serve`
   taking a signed delivery, its redelivery and a forged one. As built, the counts live in the
   process beside its timed durations, and each publish reads the newest message first to tell an
   append from an id the log already held. A poll counts `duplicate` only when the held message is
   a webhook's, so a schedule's slot or a Slack thread read again counts nothing. The webhook reply
   is unchanged. It needs no migration: the poll age reads `intake_sources`.
4. **Built.** The `work` group. Tests run a task that asks under a day cap, run tasks to done
   before and after a daemon's restart to count only the second one's tokens, and seed a week's
   answer and merge. As built, the open asks and the tasks keep each series they sent before and
   send it at 0 once it empties, so a backend's last value is never stale. The tokens read the
   trace past the newest record at the daemon's start. A week with no merge sends no attention
   point. It needs no migration.
5. **Built.** A how-to for metrics, with one alert per group a person would set first: the probe's
   `timeout` share, the oldest start's age, the poll age, the oldest ask's age and the spend
   headroom. As built, it is a page of its own,
   [How to send metrics to an OpenTelemetry backend](../sending-metrics.md), rather than a section of
   the traces how-to, since turning metrics on and setting alerts is a task of its own. Its local
   recipe was run against `otelcol` 0.139.0. Its alerts add the lease age and the tick's p95, and
   take each threshold from a limit Priory enforces. The oldest ask's age has no such limit, so the
   page leaves its threshold to the person.

## What stays out

- **Spans.** The trace export stays as it is. Spans are durable: each comes from a record in the
  state file, with a cursor, sent again until the endpoint accepts it. Metrics are samples: lost
  when a send fails, sent fresh the next interval. Both carry `service.name = priory`,
  `priory.factory` and `host.name`, so a backend filters both by factory and machine. Step counts,
  outcomes and per-task durations stay in spans, where a backend can derive them. No metric repeats
  them.
- **Exemplars** that point a histogram's bucket at a span. A run's start has no span until its step
  ends.
- **A Prometheus endpoint** to scrape. It would need the server and its token, and a factory with no
  `[serve]` listens on nothing. A collector turns OTLP into Prometheus for a person who wants that.
- **The harness's own metrics.** Claude Code exports its own with `CLAUDE_CODE_ENABLE_TELEMETRY`,
  tagged with the task and run ids through `OTEL_RESOURCE_ATTRIBUTES` as now. Priory sets nothing
  more of it.
- **Logs** as an OpenTelemetry signal. The daemon's stderr stays its log.
- **A metrics panel** in the web view or Herdr. The desk's banners say what needs a person now. A
  backend draws the history.
- **Per-task attributes** on any metric, for the cardinality reason above.

## Open questions

1. **Should `service.name` name the process, such as `priory-daemon`?** OpenTelemetry's convention
   names a logical service by it, and the daemon and the intake are two. Changing it would also
   change the spans' resource and break a backend's saved queries. Recommendation: keep `priory`,
   and name the process in `priory.service`.
2. **How does the daemon read herdr-bus's backlog?** The Postgres backlog is one count over
   `priory_log_messages` past the consumer's row in `priory_log_cursors`. herdr-bus keeps its
   cursors in its own file, and the `Log` trait has no call for it. Recommendation: add `backlog` to
   the trait, and ask herdr-bus for a call that counts past a cursor, rather than reading its file.
   Settled by the `log` slice: herdr-bus already gives each connection's `lag`, the messages past
   its cursor, so `backlog` reads that and herdr-bus needs no new call.
3. **Should metrics have an endpoint of their own?** Some backends take traces and metrics at
   different URLs or keys. Recommendation: not until a person needs it. One `otlp` serves both, as
   OTLP's own `OTEL_EXPORTER_OTLP_ENDPOINT` does.
4. **Should the cap count per minute or per request?** Per request is simpler, but a 10-second
   interval would then send six times the cap a minute. Recommendation: per minute, as the brief
   asks, since a backend's free tier counts by time.
