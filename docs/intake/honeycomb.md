# Honeycomb

A Honeycomb Trigger can start a task when it fires. This page explains how a Honeycomb delivery
becomes a `honeycomb.trigger.fired` message, and what Priory checks first. The key is under
[`[webhooks.honeycomb]`](../reference/factory.md#webhookshoneycomb), the trigger under
[`honeycomb`](../reference/triggers.md#honeycomb), the endpoint in
[serve.md](../reference/serve.md#post-hookshoneycomb) and the message in
[events.md](../reference/events.md#honeycombtriggerfired).

## Webhook only

Honeycomb alerts arrive only by webhook. The intake never polls Honeycomb, and Priory makes no
Honeycomb API call, so it needs no API key, only the webhook recipient's Shared Secret.

So a delivery that never arrives is lost. `priory check` refuses a `honeycomb` trigger without
`[webhooks.honeycomb]`, since no other path would ever fire it.

## The Shared Secret comes first

Honeycomb does not sign the body. It sends the recipient's Shared Secret as plain text in
`X-Honeycomb-Webhook-Token`. The server takes deliveries at `POST /hooks/honeycomb`, with no token,
and runs these checks in order:

1. The body is read up to 1 MiB. A larger one gets `413` before it is read.
2. `X-Honeycomb-Webhook-Token` must equal the Shared Secret. The server compares them in constant
   time. A missing or wrong one gets `401`. The body is dropped unparsed, and the server never
   logs it.
3. Only then does it parse the JSON, map it, publish it and reply `202`.

The header proves the sender only while the secret stays private, so give the recipient a long
random Shared Secret and reach the server over HTTPS. Honeycomb lets you save a recipient with no
secret. Priory refuses to start while the variable that `secret_env` names is unset or empty, so it
never takes a delivery without one.

Honeycomb waits 15 seconds for a reply. The route makes no network call: it checks, maps, publishes
on the local log and replies. When the log cannot take the message, the reply is `503` with
`Retry-After`, the one failure Honeycomb retries. Honeycomb drops a delivery after any other `4xx`
or `5xx`, so a payload without the template below is lost.

## Setting it up

1. Make a long random secret, and put it in the services' environment file:

   ```sh
   echo "HONEYCOMB_WEBHOOK_SECRET=$(openssl rand -hex 32)" >> ~/.config/priory/env
   ```

2. In Honeycomb, open **Team Settings**, then **Integrations**. Under **Trigger, SLOs, and Anomaly
   Detection Recipients**, add a **Webhook**. Set its URL to `<url>/hooks/honeycomb` and its
   **Shared Secret** to the value from step 1. See
   [Reaching the server](webhooks.md#reaching-the-server) for a URL Honeycomb can reach.
3. On its **Payload** tab, enable the **Triggers** payload and paste the
   [template](#the-payload-template).
4. Add the webhook as a recipient of each Honeycomb Trigger that should start a task.
5. Add `[webhooks.honeycomb]` to `factory.toml` and a `honeycomb` trigger, then restart the daemon.

## The payload template

Honeycomb's standard payload does not name the fields Priory needs, so the webhook must use this
Triggers payload template. `toJson` quotes and escapes each value, so a name with quotes stays
valid JSON.

```go-template
{
  "trigger": {{ toJson .ID }},
  "name": {{ toJson .Name }},
  "description": {{ toJson .Description }},
  "url": {{ toJson .URL }},
  "environment": {{ toJson .Environment }},
  "status": {{ toJson .Alert.Status }},
  "summary": {{ toJson .Alert.Summary }},
  "instance": {{ toJson .Alert.InstanceID }},
  "test": {{ toJson .Alert.IsTest }},
  "time": {{ toJson .Alert.Timestamp }},
  "result_url": {{ toJson .Result.URL }}
}
```

A payload that lacks `trigger`, `name`, `url`, `instance` or `time` gets `400`, which names the
missing field. That is how a webhook still on the standard payload shows up.

## What a delivery publishes

| `status` and `test` | Publishes |
| ------------------- | --------- |
| `TRIGGERED`, not a test, matching a `honeycomb` trigger | `honeycomb.trigger.fired`, then a task request per matching trigger |
| `TRIGGERED`, not a test, matching no trigger | nothing; `202` |
| `OK`, when the Trigger resolves | nothing; `202` |
| a test from Honeycomb's UI | nothing; `202` |

A test still proves the URL and the secret: a wrong secret gets `401`. A trigger matches when each
of its `filter` keys equals the alert's: `environment` is the Honeycomb environment's name,
`trigger` the Honeycomb Trigger's id. A delivery that matches no trigger publishes nothing, so a
webhook never widens what the factory acts on.

The task key is `honeycomb:<trigger id>`. A Trigger that fires again while its task is active
reaches that task instead of starting another.

Its `time` is the alert's evaluation time. Its `delivery` is the `X-Honeycomb-Webhook-Delivery-ID`
header.

## Duplicates

The message id is a UUIDv5 of `honeycomb:trigger:{trigger id}:fired:{instance}`, where `instance`
is the id Honeycomb gives each firing. A retry, or a replay of the same body under another delivery
id, makes the same id. herdr-bus keeps the first message (see
[envelope.md](envelope.md#duplicates)), and the server replies `{"outcome": "dropped"}` and
publishes no second task request.

## Limits

- A delivery lost while the server is down or unreachable never reaches the factory. Nothing polls
  Honeycomb to recover it.
- Only Trigger alerts publish. SLO burn alerts and anomaly detection have their own payload
  templates, and Priory maps neither yet.
- Anyone who holds the Shared Secret can post an alert, since Honeycomb does not sign the body.
- A body over 1 MiB is refused. A Trigger payload from the template is under a kilobyte.

## Sources

Read on 2026-11-22:
[Send alerts to webhooks](https://docs.honeycomb.io/notify/webhooks/),
[Custom webhook variables](https://docs.honeycomb.io/notify/webhooks/variables/),
[Webhook functions](https://docs.honeycomb.io/notify/webhooks/functions/),
[Example webhook templates](https://docs.honeycomb.io/notify/webhooks/example-templates/).
