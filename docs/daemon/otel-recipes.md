# How to send traces to an OpenTelemetry backend

This guide shows how to send the daemon's task traces to a local ClickStack, to Honeycomb or to
Grafana Cloud. It assumes a factory that runs, and it uses the export that
[trace.md](trace.md#opentelemetry-export) describes.

The local recipe was run and its trace read back. The Honeycomb and Grafana Cloud recipes are
configuration only: nobody made an account to try them.

## One limit shapes every recipe

`[telemetry]` has one key, `otlp`, and the daemon sends no authentication header: its request carries
only `Content-Type`, and `curl` runs with `--disable`, so a `.curlrc` cannot add one. A hosted backend
wants an API key in a header, and ClickStack answers `401` without its ingestion key. So the recipes
put an [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) on 127.0.0.1 between
Priory and the backend. The collector takes Priory's request and adds the header, which it reads
from an environment variable. No key lands in `factory.toml`.

In each recipe, `factory.toml` holds the same lines:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"
```

The collector's receiver is OTLP/HTTP on port 4318, and Priory sends to its `/v1/traces`. The
exporter below the receiver is the only part that changes.

## Send to a local ClickStack

ClickStack is ClickHouse's observability stack. Its all-in-one image holds the UI, the collector
and the database in one container. This recipe binds every published port to 127.0.0.1.

1. Save this as `compose.yaml`. The `collector` service is the header-adding collector.

   ```yaml
   services:
     clickstack:
       image: clickhouse/clickstack-all-in-one:latest
       ports:
         - "127.0.0.1:8080:8080"
       volumes:
         - clickstack-data:/data/db
         - clickstack-ch-data:/var/lib/clickhouse
         - clickstack-ch-logs:/var/log/clickhouse-server
     collector:
       image: otel/opentelemetry-collector:latest
       command: ["--config=/etc/otelcol/config.yaml"]
       environment:
         CLICKSTACK_INGESTION_KEY: ${CLICKSTACK_INGESTION_KEY:?start clickstack first}
       ports:
         - "127.0.0.1:4318:4318"
       volumes:
         - ./collector.yaml:/etc/otelcol/config.yaml:ro
       depends_on:
         - clickstack
   volumes:
     clickstack-data:
     clickstack-ch-data:
     clickstack-ch-logs:
   ```

2. Save this as `collector.yaml`:

   ```yaml
   receivers:
     otlp:
       protocols:
         http:
           endpoint: 0.0.0.0:4318
   exporters:
     otlphttp:
       endpoint: http://clickstack:4318
       headers:
         authorization: ${env:CLICKSTACK_INGESTION_KEY}
   service:
     pipelines:
       traces:
         receivers: [otlp]
         exporters: [otlphttp]
   ```

3. Start ClickStack alone, then create its first user. Step 4 reads the key through the cookie
   that this command saves. Registration answers `{"status":"success"}`, and the collector inside
   the container only starts listening after it.

   ```sh
   CLICKSTACK_INGESTION_KEY=unset docker compose up -d clickstack
   curl -s -X POST http://localhost:8080/api/register/password \
     -H 'Content-Type: application/json' -c cookies.txt \
     -d '{"email":"you@example.com","password":"<password>","confirmPassword":"<password>"}'
   ```

4. Read the team's ingestion key into the variable, and start the collector:

   ```sh
   export CLICKSTACK_INGESTION_KEY=$(curl -s -b cookies.txt http://localhost:8080/api/team | jq -r .apiKey)
   docker compose up -d collector
   ```

5. Point `[telemetry] otlp` at `http://127.0.0.1:4318`, run the daemon and let a task finish. Its
   root span is sent when the task reaches `done`, `failed` or `dropped`.

6. Read the spans back. ClickStack keeps them in `otel_traces`:

   ```sh
   docker compose exec clickstack clickhouse-client --query \
     "SELECT TraceId, SpanName, SpanAttributes['priory.step'] AS step
      FROM default.otel_traces ORDER BY Timestamp FORMAT PrettyCompact"
   ```

   A task of two model steps, each ended by one run, shows one trace of five spans: the root
   `task <key>`, and an `invoke_agent <agent>` span and a step span for each step.

7. Remove everything, images and volumes included:

   ```sh
   docker compose down -v --rmi all
   ```

Alternatives and limits:

- The first start pulls about 2 GB of images. Check `df -h /` before you start.
- SigNoz is another local backend whose OTLP receiver takes no key. Its Compose files are
  deprecated in favour of `foundryctl`, so this page does not cover it.
- Anything that listens for OTLP/HTTP on `/v1/traces` without a key, such as a bare collector with
  a debug exporter, works with no sidecar: set `otlp` to its address.

## Send to Honeycomb

This recipe is configuration only.

1. In Honeycomb, make an API key that can send events. Export it as `HONEYCOMB_API_KEY`.
2. Run the collector with this `collector.yaml`:

   ```yaml
   receivers:
     otlp:
       protocols:
         http:
           endpoint: 0.0.0.0:4318
   exporters:
     otlphttp:
       endpoint: https://api.honeycomb.io   # https://api.eu1.honeycomb.io in the EU
       headers:
         x-honeycomb-team: ${env:HONEYCOMB_API_KEY}
   service:
     pipelines:
       traces:
         receivers: [otlp]
         exporters: [otlphttp]
   ```

   ```sh
   docker run --rm -p 127.0.0.1:4318:4318 -e HONEYCOMB_API_KEY \
     -v "$PWD/collector.yaml:/etc/otelcol/config.yaml:ro" \
     otel/opentelemetry-collector:latest --config=/etc/otelcol/config.yaml
   ```

3. Set `otlp = "http://127.0.0.1:4318"` in `factory.toml`.

A Honeycomb Classic environment also wants an `x-honeycomb-dataset` header next to the key.

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
2. Export four variables. `GRAFANA_OTLP_ENDPOINT` is the gateway URL ending in `/otlp`, such as
   `https://otlp-gateway-prod-us-east-0.grafana.net/otlp`. `GRAFANA_OTLP_AUTH` is the whole header
   value, built from the instance ID and the token:

   ```sh
   export GRAFANA_OTLP_ENDPOINT=...
   export GRAFANA_INSTANCE_ID=...
   export GRAFANA_API_TOKEN=...
   export GRAFANA_OTLP_AUTH="Basic $(printf '%s:%s' "$GRAFANA_INSTANCE_ID" "$GRAFANA_API_TOKEN" | base64 -w0)"
   ```

3. Run the collector with this `collector.yaml`. The exporter adds `/v1/traces` itself.

   ```yaml
   receivers:
     otlp:
       protocols:
         http:
           endpoint: 0.0.0.0:4318
   exporters:
     otlphttp:
       endpoint: ${env:GRAFANA_OTLP_ENDPOINT}
       headers:
         authorization: ${env:GRAFANA_OTLP_AUTH}
   service:
     pipelines:
       traces:
         receivers: [otlp]
         exporters: [otlphttp]
   ```

   ```sh
   docker run --rm -p 127.0.0.1:4318:4318 \
     -e GRAFANA_OTLP_ENDPOINT -e GRAFANA_OTLP_AUTH \
     -v "$PWD/collector.yaml:/etc/otelcol/config.yaml:ro" \
     otel/opentelemetry-collector:latest --config=/etc/otelcol/config.yaml
   ```

4. Set `otlp = "http://127.0.0.1:4318"` in `factory.toml`. Read the traces in Grafana's Explore,
   on the stack's Tempo data source.

Free plan, as [grafana.com/pricing](https://grafana.com/pricing/) shows it:

- Traces: up to 50 GB ingested per month.
- 14 days of retention for metrics, logs, traces and profiles.

Sources for the endpoint and header:
[Grafana's OTLP guide](https://grafana.com/docs/grafana-cloud/send-data/otlp/send-data-otlp/).

## Known limits

- A free tier's limits change. Each figure above was read from the vendor's page on 2026-11-21.
- While the collector is down, the daemon retries a failed batch every 5 seconds. Once the collector
  answers `2xx`, Priory counts the batch as sent. What reaches the backend after that is the
  collector's job, and these configurations keep nothing on disk.
- A bad key shows up at the collector's exporter, not in Priory's request, so the daemon does not see
  it. Read the collector's log after the first task (`docker compose logs collector`, or the
  `docker run` output).
- Priory has no header setting, so each recipe needs the collector. A `headers_env` key under
  `[telemetry]` would remove it. It is not built.
