# Intake sources after V1

This page explains how five more sources could join the intake after V1: Sentry, Intercom,
CloudWatch, Honeycomb and Figma. It builds nothing. Each section holds what a later M3 slice needs:
the event to trigger on, how it arrives, the task key, the credential, the limits, and what Priory
already covers.

The vendor facts come from each vendor's public docs, read on 2026-11-20. Each section ends with
its sources. Vendors change limits, so re-read them when you start a slice.

## What Priory already covers

Every source below reuses the same pieces:

- `factory.intake` and the envelope. A new message type needs a JSON Schema under
  `docs/schemas/events/` and a row in [events.md](../reference/events.md). See
  [envelope.md](../intake/envelope.md).
- Duplicate dropping. Each message id is a UUIDv5 of a stable name, so herdr-bus drops a repeat
  (see [envelope.md](../intake/envelope.md#duplicates)).
- Triggers, `backfill`, `repo` and routing by task key. The daemon routes an event to the active
  task whose key matches, as it does for `linear:<issue>` (see [linear.md](../intake/linear.md)).
- The server and its signed-delivery path. [webhooks.md](../intake/webhooks.md) describes the GitHub
  path: read the raw body up to 1 MiB, check the signature in constant time, map the payload, publish
  and reply `202`. The secret lives in the environment variable that `secret_env` names.
- The polling loop and its baselines, for a source with no push (see
  [linear.md](../intake/linear.md#positions-and-baselines)).

Two sources have a webhook route: `POST /hooks/github` and `POST /hooks/linear`, each with its own
signature check, mapping and `[webhooks.<source>]` section. A new source adds one route, one check
and one mapping beside them.

All five sources are read-only: Priory receives or reads events and never writes to the vendor.

Event type names follow the existing style, `<source>.<object>.<past tense verb>`, such as
`linear.issue.created`.

## Sentry

**Built.** `POST /hooks/sentry` publishes `sentry.issue.created`. See
[sentry.md](../intake/sentry.md). The other issue actions are not mapped yet.

**Trigger on.** A new issue: `sentry.issue.created`. Sentry's issue webhook also sends `resolved`,
`assigned`, `archived` and `unresolved` actions, which could become `sentry.issue.resolved` and so
on. Sentry sends `issue.created` only for the categories `OUTAGE`, `ERROR` and `FEEDBACK`.

**Poll or webhook.** Webhook. Sentry's rate limit page says polling the API "is likely to quickly
trigger rate limiting" and recommends webhooks. The route would be `POST /hooks/sentry`.

**Signature.** Sentry signs the exact request body with HMAC-SHA256, keyed with the integration's
Client Secret, and sends the lowercase hex digest in `Sentry-Hook-Signature`. The event kind is in
the `Sentry-Hook-Resource` header, not the body, so the mapper reads both. `Request-ID` is unique
per request. `Sentry-Hook-Timestamp` is sent, but the docs do not say the signature covers it, so
do not rely on it against replays. Sentry expects a response within 1 second, so the route must
verify, map and publish, then reply `202`, as the GitHub route does, within that second.

**Task key.** `sentry:<issue id>`, from `data.issue.id`. Later `assigned` and `resolved` actions
then reach the task that the first one started.

**Credential.** `SENTRY_CLIENT_SECRET`, the Client Secret of an internal integration that has the
Issue webhook enabled. Priory only verifies with it. It makes no API call, so it needs no token.

**Limits.**

- Webhooks: 1 second response. The docs list no delivery cap.
- REST API: Sentry limits each caller and endpoint, per second and by concurrency, with no fixed
  numbers on the page. The `X-Sentry-Rate-Limit-*` headers report the current ones.
- Plan: the free Developer plan is limited to one user. The pricing page lists "API and third-party
  integrations" first under the paid Team plan, so confirm that the Developer plan can create an
  internal integration before relying on it.

**Already covered.** The server, the envelope, duplicate dropping, triggers and routing.

**New.** The route, the Sentry signature check, the header-driven mapper for `issue`, a
`[webhooks.sentry]` section, the trigger source `sentry` with filters (`project`, `level`), and the
schemas.

**Sources.**
[Webhooks](https://docs.sentry.io/organization/integrations/integration-platform/webhooks/),
[Issue webhooks](https://docs.sentry.io/organization/integrations/integration-platform/webhooks/issues/),
[Rate limits](https://docs.sentry.io/api/ratelimits/),
[Pricing](https://sentry.io/pricing/).

## Honeycomb

**Built.** `POST /hooks/honeycomb` publishes `honeycomb.trigger.fired` from Priory's payload
template. See [honeycomb.md](../intake/honeycomb.md). Resolved Triggers, SLO burn alerts and
anomaly detection are not mapped yet. The message id uses the firing's `.Alert.InstanceID` rather
than the delivery id, so a replay under a new delivery id publishes nothing.

**Trigger on.** A Honeycomb Trigger firing: `honeycomb.trigger.fired`. A resolved state could be
`honeycomb.trigger.resolved`, and SLO burn alerts could be `honeycomb.slo.burn_alert`. Both depend
on the payload template (see Task key).

**Poll or webhook.** Webhook. Honeycomb sends Trigger, SLO burn alert and anomaly detection alerts
to a webhook recipient. The route would be `POST /hooks/honeycomb`.

**Signature.** None. Honeycomb does not sign the body. It sends the recipient's Shared Secret as
plain text in `X-Honeycomb-Webhook-Token`, so the check is a constant-time comparison of that
header with the stored secret. That authenticates the sender only if the transport is HTTPS and the
secret stays private. The docs say a webhook may be saved without a secret, so Priory must refuse
a delivery when it has no secret configured. `X-Honeycomb-Webhook-Delivery-ID` stays the same
across retries, which suits the message id.

**Task key.** `honeycomb:<trigger id>`. A webhook with no payload template sends Honeycomb's standard payload. Its field names are not on
the pages read here, so the slice should ship a documented custom template that uses the variables
`.ID`, `.Name`, `.Description` and `.URL`, and fail loudly when a payload lacks the id.

**Credential.** `HONEYCOMB_WEBHOOK_SECRET`, the Shared Secret of the recipient.

**Limits.**

- Delivery: Honeycomb waits 15 seconds for a response. It retries up to three times on `408`,
  `502`, a `429` or `503` with `Retry-After`, and on connection failures. Any other failure is
  dropped without a retry, so a bad response loses the alert.
- REST API: the rate limit page gives the window in the `RateLimit` and `RateLimit-Policy`
  response headers. Not every endpoint sends them.
- Free plan: up to 20M events a month and 2 Triggers. The Pro plan lists 100 Triggers and 2 SLOs.

**Already covered.** The server, the envelope, duplicates through the delivery id, triggers and
routing.

**New.** The route, a shared-secret check, a payload template for the owner to paste, the trigger
source `honeycomb`, and the schemas.

**Sources.**
[Send alerts to webhooks](https://docs.honeycomb.io/notify/webhooks.md),
[Webhook variables](https://docs.honeycomb.io/notify/webhooks/variables.md),
[Rate limits](https://docs.honeycomb.io/api/rate-limit.md),
[Pricing](https://www.honeycomb.io/pricing).

## Intercom

**Trigger on.** A customer starting a conversation: `intercom.conversation.created`, from the topic
`conversation.user.created` ("Contact initiated conversations"). A customer reply could be
`intercom.conversation.replied`, from `conversation.user.replied`. Both need the Read conversations
permission.

**Poll or webhook.** Webhook. Intercom delivers by HTTPS `POST` to one endpoint URL per app. The
endpoint must answer a `HEAD` request, which Intercom uses to validate it. The route would be
`POST /hooks/intercom` plus a `HEAD` reply.

**Signature.** HMAC-SHA1 of the JSON body, keyed with the app's `client_secret`, sent in
`X-Hub-Signature` as `sha1=<hex>`. SHA-1 is weaker than GitHub's SHA-256. The check is still sound
for HMAC, but the slice must not reuse the `X-Hub-Signature-256` code path.

**Task key.** `intercom:<conversation id>`, from the conversation object in the notification's `data`.

**Credential.** `INTERCOM_CLIENT_SECRET`. A subscription belongs to the app, not one workspace, and
delivers events from every workspace that installs the app, so the mapper should filter on the
workspace id.

**Limits.**

- REST API: 10,000 calls a minute per app, and 25,000 a minute per workspace across all apps, spread
  over 10-second windows. A private app shares its 10,000 across the workspace. Priory needs no
  calls for the first slice.
- Free tier: the docs describe no free API tier. They say a private app reads the owner's own data.
  Confirm how a workspace without a paid plan creates the app.

**Already covered.** The server, the envelope, duplicates, triggers and routing.

**New.** The route with a `HEAD` reply, the SHA-1 check, the mapper, the workspace filter, the
trigger source `intercom`, and the schemas. The v2.15 payload carries `body` and `subject` for conversations, so the first slice needs no API
call. Fetching the full history would, so leave it to a later slice.

**Sources.**
[Webhook topics and signature](https://developers.intercom.com/docs/references/webhooks/webhook-models.md),
[Set up webhooks](https://developers.intercom.com/docs/webhooks/setting-up-webhooks.md),
[Rate limiting](https://developers.intercom.com/docs/references/rest-api/errors/rate-limiting.md).

## CloudWatch

**Trigger on.** An alarm entering `ALARM`: `cloudwatch.alarm.state_changed`, with the new state in
the event. CloudWatch sends it to EventBridge as `detail-type` `CloudWatch Alarm State Change` from
`source` `aws.cloudwatch`. The event carries `state` and `previousState`, so a trigger filter can
select `ALARM` after `OK`.

**Poll or webhook.** Two options, and they differ in what the owner must set up:

- Push, preferred. An EventBridge rule on that pattern targets an API destination, which is an HTTPS
  endpoint that EventBridge invokes with the event. The route would be `POST /hooks/cloudwatch`.
  CloudWatch states that it guarantees delivery of alarm state change events to EventBridge.
- Poll. `DescribeAlarmHistory` returns up to 100 records per call, newest first with `ScanBy`
  `TimestampDescending`, and allows 20
  requests a second per account and region. The intake would use the newest record time as its
  stream position, as the Linear streams do. It needs AWS credentials on every machine that runs
  the intake.

**Signature.** None of the GitHub kind. EventBridge sends the connection's credentials with the
request, so the check is a stored secret compared in constant time. The docs read for this page do
not say which header the connection uses. Check the authorization types of an EventBridge
connection before choosing the header.

**Task key.** `cloudwatch:<alarm ARN>`, from `resources[0]`. An alarm that goes back to `OK` then
reaches the task that its `ALARM` started.

**Credential.** Push: `CLOUDWATCH_WEBHOOK_SECRET`, the value stored in the EventBridge connection.
Poll: the standard AWS credential chain, which Priory should never read into its own config.

**Limits.**

- `DescribeAlarmHistory`: 20 requests a second. `DescribeAlarms`: 9 a second.
- EventBridge: the invocation throttle differs by region, for example 18,750 a second in
  `us-east-1`.
- Free tier: the pricing page lists 10 alarm metrics for standard-resolution alarms that name their
  metrics directly.
- The CloudWatch page says it guarantees delivery of alarm state change events to EventBridge. It
  says nothing about the leg from EventBridge to the endpoint, which fails if Priory is down, so
  polling stays useful as the recovery path.

**Already covered.** The server, the envelope, duplicates through a UUIDv5 of the EventBridge event `id`,
triggers and routing, and the polling loop with baselines.

**New.** The route and its check, the mapper, an owner guide for the rule and connection, the
trigger source `cloudwatch` with a `state` filter, and the schemas. The poll option adds AWS
credentials handling that no other source has, and a rule so that a polled alarm record and an
EventBridge event for one change publish once, as [webhooks.md](../intake/webhooks.md#duplicates)
does for GitHub: their ids differ.

**Sources.**
[Alarm events and EventBridge](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/cloudwatch-and-eventbridge.html),
[DescribeAlarmHistory](https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/API_DescribeAlarmHistory.html),
[CloudWatch quotas](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/cloudwatch_limits.html),
[API destinations](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-api-destinations.html),
[EventBridge quotas](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-quota.html),
[CloudWatch pricing](https://aws.amazon.com/cloudwatch/pricing/).

## Figma

Figma is the weakest fit, and the only source Max marked "maybe". A task needs something an agent
can act on, and a Figma comment is that.

**Trigger on.** A comment on a file: `figma.file.comment_created`, from the event `FILE_COMMENT`.
Figma's docs suggest it for creating tasks in Asana or Jira. Others are `FILE_VERSION_UPDATE`
(a named version), `LIBRARY_PUBLISH` and `DEV_MODE_STATUS_UPDATE`. Skip `FILE_UPDATE`, which fires
within 30 minutes of editing inactivity in a file: too coarse and too frequent for tasks.

**Poll or webhook.** Webhook, through the v2 webhooks API. The events page lists each event with its own `event_type`, and subscribing to `FILE_UPDATE`
also includes `FILE_DELETE`, so check which events one webhook receives before assuming one webhook
per type. The route would be `POST /hooks/figma`.

**Signature.** None. Each payload repeats the `passcode` that the owner chose when creating the
webhook, in the JSON body. The check is a constant-time comparison of that field, after the body
size cap and before any mapping. It is weaker than a body signature, because it does not bind the
passcode to the content. Figma answers `PING` when a webhook is created, which the route must accept
and map to nothing.

**Task key.** `figma:<file key>`, from `file_key`. Comments on one file then reach one task. A key
per comment is the alternative, from `comment_id`.

**Credential.** `FIGMA_WEBHOOK_PASSCODE`. Priory never calls Figma, so the owner creates the webhook by hand.

**Limits.**

- Delivery: Figma expects `200 OK`. It retries three times, after 5 minutes, 30 minutes and 3 hours,
  and never deactivates an endpoint.
- Webhook quotas: 20 per team, 5 per folder and 3 per file. File webhooks add a plan cap: 150
  (Professional), 300 (Organization), 600 (Enterprise).
- REST API: Figma limits by endpoint tier, seat type and plan, with a leaky bucket and `429`
  replies. A personal token reading a file in a Starter plan gets 6 requests a month.
- A forum report from June 2026 says a team FILE_COMMENT webhook silently stopped delivering after
  1 to 4 hours, with no failure logged. It has no confirmed answer, so the slice needs a check
  before it trusts the source.

**Already covered.** The server, the envelope, duplicates, triggers and routing.

**New.** The route and the `PING` case, the passcode check, the mapper, a note on which events each
webhook receives, the trigger source `figma`, and the schemas.

**Sources.**
[Webhooks](https://developers.figma.com/docs/rest-api/webhooks),
[Webhook events](https://developers.figma.com/docs/rest-api/webhooks-events),
[Rate limits](https://developers.figma.com/docs/rest-api/rate-limits),
[Forum report, seen in a search result only](https://forum.figma.com/report-a-problem-6/rest-v2-team-webhook-file-comment-silently-stops-delivering-after-1-to-4h-status-stays-active-no-failed-attempts-logged-55036).

## Suggested order

| # | Source | Size | Why here |
| - | ------ | ---- | -------- |
| 1 | Sentry | S | Real HMAC-SHA256, a clean `issue.created`, and the first extra route. It sets the per-source check pattern |
| 2 | Honeycomb | S | Reuses the route pattern. Only the check (a shared header) and a payload template are new |
| 3 | Intercom | M | One more check (HMAC-SHA1), a `HEAD` reply and a workspace filter |
| 4 | CloudWatch | M | Needs an owner guide for EventBridge, and polling would add AWS credentials |
| 5 | Figma | M | Weak passcode check, and an unconfirmed delivery bug |

Sizes: S is about one pull request and a day. M is one pull request plus an owner guide or a
second mode.
