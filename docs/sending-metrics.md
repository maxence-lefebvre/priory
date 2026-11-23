# How to send metrics to an OpenTelemetry backend

This guide shows how to turn on Priory's metrics, send them to an OpenTelemetry Collector on
127.0.0.1 or to a hosted backend, check that they arrive, and set the first alerts. It assumes a
factory that runs. [metrics.md](daemon/metrics.md) explains how each process samples and sends its
metrics, and [the metrics reference](reference/metrics.md) lists every metric.

The local collector recipe was run with `otelcol` 0.139.0 and a daemon of the `triage` starter, and
its output is quoted below. The Honeycomb recipe is configuration only: nobody ran it against a real
account.

## Turn the groups on

Metrics go to the endpoint your spans use, at `<otlp>/v1/metrics`, with the same headers. Name the
groups to send in `[telemetry]` of `factory.toml`:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"
metrics = ["process", "log", "runs", "intake", "work"]
metrics_every = "60s"   # the default; at least "10s"
metrics_cap = 1000      # the default; data points per minute, per process
```

Then run `priory check`. It refuses an unknown group, a `metrics_every` under 10 seconds and a
`metrics_cap` of 0. [`[telemetry]`](reference/factory.md#telemetry) lists the keys.

To turn groups on or off for one process without editing the factory every runner shares, set
`PRIORY_METRICS` in that process's environment. It replaces `metrics`:

```sh
PRIORY_METRICS=process,log priory daemon   # these two groups only
PRIORY_METRICS=none priory daemon          # no metric
```

Restart each process after a change. The daemon, the intake and `priory serve` each send their own
groups, as [metrics.md](daemon/metrics.md#which-process-sends-what) lists. `--once` sends no
metrics.

## Send to a local OpenTelemetry Collector

An [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) with a debug exporter prints
each request it takes. Use it to see the metrics before you pick a backend, or keep it and add an
exporter to that backend later.

1. Download `otelcol` for your machine from the collector's
   [releases](https://github.com/open-telemetry/opentelemetry-collector-releases/releases).
2. Save this as `collector.yaml`. The receiver listens on 127.0.0.1 only:

   ```yaml
   receivers:
     otlp:
       protocols:
         http:
           endpoint: 127.0.0.1:4318
   exporters:
     debug:
       verbosity: detailed
   service:
     pipelines:
       metrics:
         receivers: [otlp]
         exporters: [debug]
   ```

3. Start it, and keep its output in a file:

   ```sh
   ./otelcol --config=collector.yaml > collector.log 2>&1 &
   ```

4. Set `otlp = "http://127.0.0.1:4318"` in `[telemetry]`, with no `headers_env`, and start the
   daemon. Its first request goes out when it starts, then one every `metrics_every`.

To run the collector in Docker instead, give the receiver `endpoint: 0.0.0.0:4318` inside the
container, and publish the port on 127.0.0.1 only:

```sh
docker run --rm -p 127.0.0.1:4318:4318 \
  -v "$PWD/collector.yaml:/etc/otelcol/config.yaml:ro" \
  otel/opentelemetry-collector:0.139.0 --config=/etc/otelcol/config.yaml
```

## Send to a hosted backend

Honeycomb takes OTLP/HTTP with a JSON body, which is how Priory sends. Its key goes in a header
that the daemon reads from an environment variable. The key never goes in `factory.toml`.

1. In Honeycomb, make an API key that can send events.
2. Put the headers in a variable, in the shell or service file that runs each Priory process.
   Honeycomb stores metrics in the dataset that `x-honeycomb-dataset` names:

   ```sh
   export OTEL_EXPORTER_OTLP_HEADERS='x-honeycomb-team=<key>,x-honeycomb-dataset=priory-metrics'
   ```

3. Name the variable in `factory.toml`:

   ```toml
   [telemetry]
   otlp = "https://api.honeycomb.io"   # https://api.eu1.honeycomb.io in the EU
   headers_env = "OTEL_EXPORTER_OTLP_HEADERS"
   metrics = ["process", "log", "runs", "intake", "work"]
   ```

The same headers go with spans. Check
[docs.honeycomb.io/send-data/opentelemetry](https://docs.honeycomb.io/send-data/opentelemetry/)
for whether your environment files spans by `service.name` or by the dataset header.
[Give the daemon the backend's key](sending-traces-to-opentelemetry.md#give-the-daemon-the-backends-key)
explains the header form: commas between headers, URL-encoded values, and a process that refuses to
start when the variable is unset or malformed. Every process that sends metrics reads the variable,
so the intake and `priory serve` need it too.

Another backend works the same way: set `otlp` to its OTLP/HTTP base URL, without `/v1/metrics`,
and put its key in the variable. A backend that takes only protobuf or gRPC needs a collector in
between, as in [Put a collector in between instead](sending-traces-to-opentelemetry.md#put-a-collector-in-between-instead),
with a `metrics` pipeline beside the `traces` one.

## Check that points arrive

1. Wait one `metrics_every` after the daemon starts.
2. Read the daemon's log. A request the endpoint refused, or that got no answer, logs one line per
   error text:

   ```text
   priory daemon: sending metrics to http://127.0.0.1:4318 failed (HTTP 401)
   ```

   A `401` or `403` means a wrong key. A `curl: (7)` error means nothing listens at `otlp`.
3. On the local collector, list the metric names it took:

   ```sh
   grep -E -- '-> Name: |priory.service:' collector.log | sort | uniq -c
   ```

   A daemon of one runner with no task sends about 12 metrics, such as:

   ```text
         1      -> Name: priory.asks.oldest.age
         1      -> Name: priory.log.probe.duration
         1      -> Name: priory.process.tick.duration
         1      -> Name: priory.run.start.oldest.age
         1      -> Name: process.memory.usage
         1      -> priory.service: Str(daemon)
   ```

   On Honeycomb, query the `priory-metrics` dataset for `priory.process.tick.duration`.

Some metrics have no point until there is something to count. A counter such as
`priory.intake.events` appears after its first event. `priory.runner.lease.age` needs a factory of
two runners. `priory.intake.poll.age` comes from the intake, and only with `PRIORY_DB` set.
`priory.metrics.dropped` with `kind = send` counts the requests that failed, such as one sent before
the collector started.

## Set the first alerts

Start with these six. Each threshold comes from a limit Priory itself enforces, so an alert fires
before the limit acts.

| Metric | Alert when | Why that threshold |
| ------ | ---------- | ------------------ |
| `priory.log.probe.duration`, `outcome = timeout` | its count rises over 5 minutes | Each `timeout` is a read that waited the whole [`[log] timeout`](reference/factory.md#log), 10 seconds by default, for no answer |
| `priory.runner.lease.age` | above 900 seconds | At 1800 seconds the other runners may move the cut-off runner's waiting tasks ([runners.md](daemon/runners.md#holds-and-leases)) |
| `priory.run.start.oldest.age` | above 30 seconds | At 60 seconds with no `SessionStart` the daemon blocks the task ([health.md](daemon/health.md)) |
| `priory.intake.poll.age` | above 300 seconds | The intake polls every 60 seconds, and the desk's [intake banner](daemon/desk.md#intake-banner) shows an intake that has not polled for 5 minutes |
| `priory.process.tick.duration`, `stat = p95` | above 1 second for 5 minutes, on `priory.service = daemon` | The daemon ticks every second, so a longer tick means it falls behind. `stat = max` names the one slow clone or fetch |
| `priory.spend.headroom` | below a tenth of the cap's limit | At 0 the daemon holds or stops starting the tasks under the cap ([spend.md](daemon/spend.md#past-a-cap)) |

`priory.asks.oldest.age` is worth an alert too, at the time you want a person to answer in. Priory
sets no limit on how long an ask waits.

## Known limits

- Metrics are samples. A request that fails is not retried: its counters carry their totals in the
  next one, and its gauges are lost.
- Past `metrics_cap`, a process drops gauges first, then histograms, then counters, and counts each
  in `priory.metrics.dropped`. Raise the cap, or name fewer groups, when that counter rises.
- On a daemon's very first start, before it creates its state file, the first interval logs
  `cannot read the log's state`, `cannot read the runs' state` and `cannot read the factory's work`
  once, and sends the `process` group alone.
- Each process reads the header variable once, when it starts. Restart it after you change the key.
