# CloudWatch

CloudWatch can start a task when an alarm changes state. This page explains how an alarm state
change reaches Priory through EventBridge, what Priory checks first, and how to set up the AWS side.
The key is under [`[webhooks.cloudwatch]`](../reference/factory.md#webhookscloudwatch), the trigger
under [`cloudwatch`](../reference/triggers.md#cloudwatch), the endpoint in
[serve.md](../reference/serve.md#post-hookscloudwatch) and the message in
[events.md](../reference/events.md#cloudwatchalarmstate_changed).

## Through EventBridge only

CloudWatch sends every alarm state change to EventBridge, and its docs say that leg is guaranteed.
An EventBridge rule matches the change and sends it to an API destination: an HTTPS endpoint that
EventBridge calls with the event as the JSON body. Priory is that endpoint, at
`POST /hooks/cloudwatch`.

Priory never polls CloudWatch and never holds AWS credentials. It only checks the value that the
API destination's connection sends. `priory check` refuses a `cloudwatch` trigger without
`[webhooks.cloudwatch]`, since no other path would ever fire it.

EventBridge retries a delivery that times out or gets `401`, `407`, `409`, `429` or a `5xx`, by
default for up to 24 hours and 185 attempts. A change still lost after that, for example while
Priory is down for a day, never reaches the factory unless the target has a dead-letter queue.

## The token comes first

EventBridge does not sign the body. A connection with API key authorization sends a header whose
name and value the owner chooses. Priory reads the header `X-Priory-Webhook-Token`, so name the API
key that, and give it the value of the variable that `secret_env` names.

The server takes deliveries at `POST /hooks/cloudwatch`, with no token of its own. It runs these
checks in order:

1. The body is read up to 1 MiB. A larger one gets `413` before it is read.
2. `X-Priory-Webhook-Token` must equal the secret. The server compares them in constant time. A
   missing or wrong one gets `401`. The body is dropped unparsed, and the server never logs it.
3. Only then does it parse the JSON, map it, publish it and reply `202`.

The token proves that the sender knows the secret. It does not bind the secret to the body, so
anyone who reads one request can replay it or send another body. Keep the endpoint behind HTTPS,
as an API destination requires, and keep the secret out of logs. A replay of a known event publishes
nothing (see [Duplicates](#duplicates)).

EventBridge waits 5 seconds for a reply. The route makes no call to AWS: it checks, maps, publishes
on the factory's event log and replies.

A `401` makes EventBridge deauthorize an API key connection. Once the secret in Priory and the
value in the connection differ, the connection stops delivering until you edit it with the right
value, even after Priory is fixed. EventBridge emits a connection state event that you can alarm
on.

## Setting it up

You need an AWS account where the alarms live, and a URL that AWS can reach. See
[Reaching the server](webhooks.md#reaching-the-server).

1. Make a secret and put it in the services' environment file:

   ```sh
   echo "CLOUDWATCH_WEBHOOK_SECRET=$(openssl rand -hex 32)" >> ~/.config/priory/env
   ```

2. Add `[webhooks.cloudwatch]` to `factory.toml` and a `cloudwatch` trigger, then restart the
   daemon.
3. In the EventBridge console, under **Connections**, create a connection:
   - **Configure authorization**: **Custom configuration**.
   - **Authorization type**: API Key.
   - **API key name**: `X-Priory-Webhook-Token`.
   - **Value**: the secret from step 1.
4. Under **API destinations**, create an API destination:
   - **API destination endpoint**: `<url>/hooks/cloudwatch`.
   - **HTTP method**: `POST`.
   - **Connection**: the one from step 3.
5. Under **Rules**, create a rule on the default event bus with this event pattern:

   ```json
   {
     "source": ["aws.cloudwatch"],
     "detail-type": ["CloudWatch Alarm State Change"]
   }
   ```

   Narrow it here if you want, for example with `"detail": { "alarmName": ["ServerCpuTooHigh"] }`.
   Pick the API destination from step 4 as the target, with no input transformer, and let the
   console create the execution role it needs.
6. Optionally, give the target a dead-letter queue, so events EventBridge gives up on are kept.

Rules work per region. Repeat steps 3 to 5 in each region whose alarms should reach Priory.

To check the setup, set an alarm's state by hand:

```sh
aws cloudwatch set-alarm-state --alarm-name ServerCpuTooHigh \
  --state-value ALARM --state-reason "Testing the Priory intake"
```

## What a delivery publishes

| `source` and `detail-type` | Publishes |
| -------------------------- | --------- |
| `aws.cloudwatch` `CloudWatch Alarm State Change`, matching a `cloudwatch` trigger | `cloudwatch.alarm.state_changed`, then a task request per matching trigger |
| the same, matching no trigger | nothing; `202` |
| `aws.cloudwatch` `CloudWatch Alarm Configuration Change` or `CloudWatch Alarm Contributor State Change` | nothing; `202` |
| any other source or detail type | nothing; `202` |

A trigger matches when each of its `filter` keys equals the change's: `name` is the alarm name,
`state` the new state, and `previous_state` the state before. States are `OK`, `ALARM` and
`INSUFFICIENT_DATA`. A delivery that matches no trigger publishes nothing, so a broad EventBridge
rule never widens what the factory acts on.

Metric, metric math, anomaly detection, composite and log alarms all send the same detail type, so
they all map. A multi time series alarm also sends one event per contributor, which Priory drops:
the alarm's own event still maps.

The task key is `cloudwatch:<alarm ARN>`, from `resources[0]`. The alarm going back to `OK` then
reaches the task that its `ALARM` started, when a trigger takes `OK` too.

Its `time` is the event's `time`. Its `delivery` is the EventBridge event `id`.

## Duplicates

The message id is a UUIDv5 of `cloudwatch:event:{event id}`. EventBridge keeps the id across its
retries, so a retry, or a replay of the same body, makes the same id. herdr-bus keeps the first
message (see [envelope.md](envelope.md#duplicates)), and the server replies
`{"outcome": "dropped"}` and publishes no second task request.

## Limits

- A change lost after EventBridge's retries never reaches the factory. Nothing polls CloudWatch to
  recover it.
- A wrong or rotated secret deauthorizes the connection, and only editing the connection restores
  it.
- The token is a shared value, not a signature over the body.
- Alarm configuration changes publish nothing.
- A body over 1 MiB is refused. An alarm state change is a few kilobytes.

## Sources

Read on 2026-11-22:
[Alarm events and EventBridge](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/cloudwatch-and-eventbridge.html),
[API destinations](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-api-destinations.html),
[Connections](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-target-connection.html),
[Connection authorization methods](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-target-connection-auth.html),
[De-authorizing connections](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-target-connection-deauthorize.html),
[Creating a connection](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-target-connection-create.html),
[Creating an API destination](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-api-destination-create.html),
[Targets](https://docs.aws.amazon.com/eventbridge/latest/userguide/eb-targets.html).
