# Sentry

Sentry can start a task when it creates an issue. This page explains how a Sentry delivery becomes
a `sentry.issue.created` message, and what Priory checks first. The key is under
[`[webhooks.sentry]`](../reference/factory.md#webhookssentry), the trigger under
[`sentry`](../reference/triggers.md#sentry), the endpoint in
[serve.md](../reference/serve.md#post-hookssentry) and the message in
[events.md](../reference/events.md#sentryissuecreated).

## Webhook only

Sentry events arrive only by webhook. The intake never polls Sentry: Sentry's rate limit page says
polling its API "is likely to quickly trigger rate limiting" and recommends webhooks. Priory makes no
Sentry API call, so it needs no Sentry token, only the integration's Client Secret.

So a delivery that never arrives is lost. `priory check` refuses a `sentry` trigger without
`[webhooks.sentry]`, since no other path would ever fire it.

## The signature comes first

The server takes deliveries at `POST /hooks/sentry`, with no token. It runs these checks in order:

1. The body is read up to 1 MiB. A larger one gets `413` before it is read.
2. `Sentry-Hook-Signature` must be the hex HMAC-SHA256 of the exact body bytes, keyed with the
   Client Secret. The server compares them in constant time. A missing or wrong one gets `401`.
   The body is dropped unparsed, and the server never logs it.
3. Only then does it parse the JSON, map it, publish it and reply `202`.

Sentry also sends `Sentry-Hook-Timestamp`, but its docs do not say the signature covers it, so
Priory does not read it. Replays are handled by the message id instead (see
[Duplicates](#duplicates)).

Sentry waits 1 second for a reply. The route makes no network call: it checks, maps, publishes on
the local log and replies.

## Setting it up

1. In Sentry, create an internal integration. Set its webhook URL to `<url>/hooks/sentry`, and enable the Issue webhook.
   See [Reaching the server](webhooks.md#reaching-the-server) for a URL Sentry can reach.
2. Copy its Client Secret into the services' environment file:

   ```sh
   echo "SENTRY_CLIENT_SECRET=<the Client Secret>" >> ~/.config/priory/env
   ```

3. Add `[webhooks.sentry]` to `factory.toml` and a `sentry` trigger, then restart the daemon.

Sentry's pricing page lists "API and third-party integrations" under the paid Team plan. Check
that your plan can create an internal integration.

## What a delivery publishes

| `Sentry-Hook-Resource` and `action` | Publishes |
| ----------------------------------- | --------- |
| `issue` `created`, matching a `sentry` trigger | `sentry.issue.created`, then a task request per matching trigger |
| `issue` `created`, matching no trigger | nothing; `202` |
| `issue` `resolved`, `assigned`, `archived` or `unresolved` | nothing; `202` |
| any other resource, such as `error` or `installation` | nothing; `202` |

A trigger matches when each of its `filter` keys equals the issue's: `project` is the project's
slug, `level` the issue's level. A delivery that matches no trigger publishes nothing, so a webhook
never widens what the factory acts on.

Sentry sends `issue.created` only for the categories `error`, `outage` and `feedback`. The message
carries the category, so a workflow can tell them apart.

The task key is `sentry:<issue id>`, from `data.issue.id`. A later slice that maps `resolved` or
`assigned` would route those to the same task.

Its `time` is the issue's `firstSeen`. Its `delivery` is the `Request-ID` header, which Sentry does
not sign, so treat it as a label.

## Duplicates

The message id is a UUIDv5 of `sentry:issue:{issue id}:created`. A redelivery, or a replay of the
signed body under another `Request-ID`, makes the same id. herdr-bus keeps the first message (see
[envelope.md](envelope.md#duplicates)), and the server replies `{"outcome": "dropped"}` and
publishes no second task request.

## Limits

- A delivery lost while the server is down or unreachable never reaches the factory. Nothing polls
  Sentry to recover it.
- Only `issue.created` publishes. Resolved, assigned, archived and unresolved issues publish nothing
  yet.
- A body over 1 MiB is refused. Sentry's issue payload is a few kilobytes.

## Sources

Read on 2026-11-22, from the source of Sentry's docs in `getsentry/sentry-docs`:
[Webhooks](https://docs.sentry.io/organization/integrations/integration-platform/webhooks/),
[Issue webhooks](https://docs.sentry.io/organization/integrations/integration-platform/webhooks/issues/),
[Rate limits](https://docs.sentry.io/api/ratelimits/).
