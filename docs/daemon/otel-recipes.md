# How to send traces to an OpenTelemetry backend

This guide shows how to send the daemon's task traces to a local ClickStack, to Honeycomb or to
Grafana Cloud. It assumes a factory that runs, and it uses the export that
[trace.md](trace.md#opentelemetry-export) describes.

The local ClickStack recipe was run through a collector, before `headers_env` existed, and its
trace read back. Sending straight to ClickStack, Honeycomb or Grafana Cloud with `headers_env` is
configuration only: nobody ran it against a real account.

## Give the daemon the backend's key

A hosted backend wants an API key in a header, and ClickStack answers `401` without its ingestion
key. `[telemetry] headers_env` names an environment variable that holds the headers, in the
standard `OTEL_EXPORTER_OTLP_HEADERS` form:

```toml
[telemetry]
otlp = "https://api.honeycomb.io"
headers_env = "OTEL_EXPORTER_OTLP_HEADERS"
```

```sh
export OTEL_EXPORTER_OTLP_HEADERS='x-honeycomb-team=<key>'
```

- Separate several headers with commas: `key1=value1,key2=value2`.
- URL-encode a comma, a space or a `%` in a value: `Basic%20<token>` sends `Basic <token>`.
- The daemon refuses to start when the variable is unset, empty or malformed, and its error never
  quotes the value.
- The key stays out of `factory.toml`, the process list, the daemon's log and the trace: the daemon
  hands the headers to `curl` in a file only its user can read.

## Send to a local ClickStack

ClickStack is ClickHouse's observability stack. Its all-in-one image holds the UI, the collector
and the database in one container. This recipe binds every published port to 127.0.0.1.

1. Save this as `compose.yaml`:

   ```yaml
   services:
     clickstack:
       image: clickhouse/clickstack-all-in-one:latest
       ports:
         - "127.0.0.1:8080:8080"
         - "127.0.0.1:4318:4318"
       volumes:
         - clickstack-data:/data/db
         - clickstack-ch-data:/var/lib/clickhouse
         - clickstack-ch-logs:/var/log/clickhouse-server
   volumes:
     clickstack-data:
     clickstack-ch-data:
     clickstack-ch-logs:
   ```

2. Start ClickStack, then create its first user. Step 3 reads the key through the cookie that this
   command saves. Registration answers `{"status":"success"}`, and the collector inside the
   container only starts listening after it.

   ```sh
   docker compose up -d
   curl -s -X POST http://localhost:8080/api/register/password \
     -H 'Content-Type: application/json' -c cookies.txt \
     -d '{"email":"you@example.com","password":"<password>","confirmPassword":"<password>"}'
   ```

3. Read the team's ingestion key into the header variable, in the shell that runs the daemon:

   ```sh
   export OTEL_EXPORTER_OTLP_HEADERS="authorization=$(curl -s -b cookies.txt http://localhost:8080/api/team | jq -r .apiKey)"
   ```

4. Set the export in `factory.toml`, run the daemon and let a task finish. Its root span is sent
   when the task reaches `done`, `failed` or `dropped`.

   ```toml
   [telemetry]
   otlp = "http://127.0.0.1:4318"
   headers_env = "OTEL_EXPORTER_OTLP_HEADERS"
   ```

5. Read the spans back. ClickStack keeps them in `otel_traces`:

   ```sh
   docker compose exec clickstack clickhouse-client --query \
     "SELECT TraceId, SpanName, SpanAttributes['priory.step'] AS step
      FROM default.otel_traces ORDER BY Timestamp FORMAT PrettyCompact"
   ```

   A task of two model steps, each ended by one run, shows one trace of five spans: the root
   `task <key>`, and an `invoke_agent <agent>` span and a step span for each step.

6. Remove everything, images and volumes included:

   ```sh
   docker compose down -v --rmi all
   ```

Alternatives and limits:

- The first start pulls about 2 GB of images. Check `df -h /` before you start.
- SigNoz is another local backend whose OTLP receiver takes no key. Its Compose files are
  deprecated in favour of `foundryctl`, so this page does not cover it.
- Anything that listens for OTLP/HTTP on `/v1/traces` without a key, such as a bare collector with
  a debug exporter, needs no `headers_env`: set `otlp` to its address.

## Send to Honeycomb

This recipe is configuration only.

1. In Honeycomb, make an API key that can send events.
2. Put it in the header variable, in the shell that runs the daemon:

   ```sh
   export OTEL_EXPORTER_OTLP_HEADERS='x-honeycomb-team=<key>'
   ```

3. Set the export in `factory.toml`:

   ```toml
   [telemetry]
   otlp = "https://api.honeycomb.io"   # https://api.eu1.honeycomb.io in the EU
   headers_env = "OTEL_EXPORTER_OTLP_HEADERS"
   ```

A Honeycomb Classic environment also wants an `x-honeycomb-dataset` header next to the key:
`x-honeycomb-team=<key>,x-honeycomb-dataset=<dataset>`.

Free plan, as [honeycomb.io/pricing](https://www.honeycomb.io/pricing) shows it:

- Up to 20 million events and 100 million metric data points per month.
- 2 triggers. Distributed tracing, BubbleUp and OpenTelemetry support are included.
- Check the page's retention row and its definition of an event for trace data before you size a
  factory. Priory sends one span for each root, run and step.

Sources for the endpoint and header:
[docs.honeycomb.io/send-data/opentelemetry](https://docs.honeycomb.io/send-data/opentelemetry/).

## Send to Grafana Cloud

This recipe is configuration only.

1. In Grafana Cloud, open the stack's OpenTelemetry tile and generate a token. It shows the OTLP
   endpoint, the instance ID and the token.
2. Build the header from the instance ID and the token, in the shell that runs the daemon. `%20`
   is the URL-encoded space after `Basic`:

   ```sh
   export GRAFANA_INSTANCE_ID=...
   export GRAFANA_API_TOKEN=...
   export OTEL_EXPORTER_OTLP_HEADERS="authorization=Basic%20$(printf '%s:%s' "$GRAFANA_INSTANCE_ID" "$GRAFANA_API_TOKEN" | base64 -w0)"
   ```

3. Set the export in `factory.toml`. `otlp` is the gateway URL ending in `/otlp`, and the daemon adds
   `/v1/traces` itself. Read the traces in Grafana's Explore, on the stack's Tempo data source.

   ```toml
   [telemetry]
   otlp = "https://otlp-gateway-prod-us-east-0.grafana.net/otlp"
   headers_env = "OTEL_EXPORTER_OTLP_HEADERS"
   ```

Free plan, as [grafana.com/pricing](https://grafana.com/pricing/) shows it:

- Traces: up to 50 GB ingested per month.
- 14 days of retention for metrics, logs, traces and profiles.

Sources for the endpoint and header:
[Grafana's OTLP guide](https://grafana.com/docs/grafana-cloud/send-data/otlp/send-data-otlp/).

## Put a collector in between instead

An [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) on 127.0.0.1 can sit between
Priory and the backend. Use one to batch spans, to queue them while the backend is away, or to send
them to more than one backend. The collector reads the key from its own environment, so
`factory.toml` keeps `otlp = "http://127.0.0.1:4318"` and no `headers_env`.

1. Save this as `collector.yaml`, with the backend's endpoint and header:

   ```yaml
   receivers:
     otlp:
       protocols:
         http:
           endpoint: 0.0.0.0:4318
   exporters:
     otlphttp:
       endpoint: https://api.honeycomb.io
       headers:
         x-honeycomb-team: ${env:HONEYCOMB_API_KEY}
   service:
     pipelines:
       traces:
         receivers: [otlp]
         exporters: [otlphttp]
   ```

2. Export the key the collector reads, and run it:

   ```sh
   export HONEYCOMB_API_KEY=<key>
   docker run --rm -p 127.0.0.1:4318:4318 -e HONEYCOMB_API_KEY \
     -v "$PWD/collector.yaml:/etc/otelcol/config.yaml:ro" \
     otel/opentelemetry-collector:latest --config=/etc/otelcol/config.yaml
   ```

## Known limits

- A free tier's limits change. Each figure above was read from the vendor's page on 2026-11-21.
- While the backend or collector is down, the daemon retries a failed batch every 5 seconds. Behind
  a collector, once it answers `2xx`, Priory counts the batch as sent, and what reaches the backend
  after that is the collector's job. The configuration above keeps nothing on disk.
- A bad key sent with `headers_env` comes back as a `401` or `403`, which OTLP says will never be
  accepted: the daemon logs it and skips the batch. Behind a collector, a bad key shows up only in
  the collector's log.
- The daemon reads the header variable once, when it starts. Restart it after you change the key.
