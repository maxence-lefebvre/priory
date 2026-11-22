# Figma

A comment on a Figma file can start a task. This page explains how a Figma delivery becomes a
`figma.file.comment_created` message, what the passcode check proves and what it does not, and how
to create the webhook. The key is under
[`[webhooks.figma]`](../reference/factory.md#webhooksfigma), the trigger under
[`figma`](../reference/triggers.md#figma), the endpoint in
[serve.md](../reference/serve.md#post-hooksfigma) and the message in
[events.md](../reference/events.md#figmafilecomment_created).

## Webhook only

Figma comments arrive only by webhook, through Figma's v2 webhooks API. The intake never polls
Figma, and Priory makes no Figma API call. You create the webhook yourself, once, with your own
Figma token, and Priory keeps only the passcode.

So a delivery that never arrives is lost. `priory check` refuses a `figma` trigger without
`[webhooks.figma]`, since no other path would ever fire it.

## What the passcode proves

Figma does not sign its deliveries. Each body repeats, in its `passcode` field, the passcode you
chose when you created the webhook. The server takes deliveries at `POST /hooks/figma`, with no
token, and runs these checks in order:

1. The body is read up to 1 MiB. A larger one gets `413` before it is read.
2. The body is parsed as JSON, since the passcode is inside it. `passcode` must equal the value of
   the variable that `secret_env` names. The server compares them in constant time. A body that is
   not JSON, or a missing or wrong passcode, gets `400`. No other field is mapped before this
   check, and the server never logs the body.
3. Only then does it map the comment, publish it and reply `200`.

The passcode proves one thing: the sender knows the passcode. That is the same proof as a shared
secret in a header, such as Honeycomb's.

It does not prove that Figma wrote the rest of the body. The passcode travels inside the body, so
anyone who sees one delivery can send any comment text with it. It does not prove the delivery is
new, so a captured delivery can be sent again. And Figma's API never shows the passcode after you
create the webhook, so you cannot check which one it holds. A passcode that leaks stays valid until
you replace it.

Priory reads the passcode when it starts, and a delivery from Figma with the wrong passcode stops
the webhook. So replace it in this order: pause the webhook with `PUT /v2/webhooks/<webhook id>`
and `"status": "PAUSED"`, set the new passcode there and in the services' environment file,
restart the daemon, then set `"status": "ACTIVE"`.

So keep the endpoint behind HTTPS, use a long random passcode, and treat a comment's text as a
report to check, never as an instruction. A replayed comment publishes nothing (see
[Duplicates](#duplicates)).

Figma's docs ask for `400` on a wrong passcode, and say it stops the webhook. A forged request
cannot stop yours: only a delivery from Figma itself with the wrong passcode does, which means the
passcode in `secret_env` does not match the webhook's.

Figma counts any reply but `200` as a failure and retries it after 5 minutes, 30 minutes and 3
hours. So every delivery Priory takes, or drops on purpose, gets `200`. The route makes no call to
Figma: it checks, maps, publishes on the factory's event log and replies.

## Setting it up

You need a Figma personal access token with the `webhooks:write` scope, and a URL that Figma can
reach. See [Reaching the server](webhooks.md#reaching-the-server).

1. Make a passcode and put it in the services' environment file:

   ```sh
   echo "FIGMA_WEBHOOK_PASSCODE=$(openssl rand -hex 32)" >> ~/.config/priory/env
   ```

2. Add `[webhooks.figma]` to `factory.toml` and a `figma` trigger, then restart the daemon.
3. Create the webhook. `context` is `file`, `project` or `team`, and `context_id` is that file's
   key, or the project's or team's id:

   ```sh
   set -a; . ~/.config/priory/env; set +a
   curl -X POST https://api.figma.com/v2/webhooks \
     -H "X-Figma-Token: $FIGMA_TOKEN" -H "Content-Type: application/json" \
     -d "{\"event_type\": \"FILE_COMMENT\", \"context\": \"file\", \"context_id\": \"<file key>\",
          \"endpoint\": \"<url>/hooks/figma\", \"passcode\": \"$FIGMA_WEBHOOK_PASSCODE\"}"
   ```

   Figma then sends a `PING`, which Priory answers with `200` and publishes nothing.

A team webhook skips files in invite-only folders. Figma limits webhooks to 20 per team, 5 per
folder and 3 per file, plus a plan cap on file webhooks.

## Checking that deliveries still arrive

In June 2026, a report on Figma's forum said a team `FILE_COMMENT` webhook stopped delivering after
1 to 4 hours, while its status stayed `ACTIVE` and its request log showed no failure. A second
team saw the same with `LIBRARY_PUBLISH`. On 2026-07-30 a Figma employee replied that a fix was
released for newly created webhooks only, and a reporter confirmed it on 2026-08-03. Figma's docs
do not mention it.

So recreate any webhook made before 2026-07-30. Priory cannot see a delivery that never arrives,
so also check the webhook now and then. Figma lists every request it sent in the last 7 days:

```sh
curl -H "X-Figma-Token: $FIGMA_TOKEN" https://api.figma.com/v2/webhooks/<webhook id>/requests
```

A comment you made that the list lacks points to Figma not sending, though the webhook's context
may not cover that file, and the list keeps only 7 days. A comment the list shows with `200` but
no new task matched no trigger, or reached a task already open for its file, or repeated a comment
already published.

## What a delivery publishes

| `event_type` | Publishes |
| ------------ | --------- |
| `FILE_COMMENT`, matching a `figma` trigger | `figma.file.comment_created`, then a task request per matching trigger |
| `FILE_COMMENT`, matching no trigger | nothing; `200` |
| `PING` | nothing; `200` |
| any other event, such as `FILE_UPDATE` or `LIBRARY_PUBLISH` | nothing; `200` |

A trigger matches when its `file` filter equals the comment's file key. A trigger with no `file`
filter matches every file the webhook covers, so give each trigger a `file` filter to keep a team
or project webhook from widening what the factory acts on. A delivery that matches no trigger
publishes nothing.

The message's `text` joins the comment's fragments, each mention written as `@<handle>`. A reply
carries its `parent` comment id.

The task key is `figma:<file key>`. Every comment on one file reaches the same task while that task
is open.

Its `time` is the comment's `created_at`, else the delivery's `timestamp`. Its `url` is the file's page.

## Duplicates

The message id is a UUIDv5 of `figma:comment:{comment id}`, the comment's own id. Figma's retries
carry the same comment id, so a retry, or a replay of the same comment, makes the same id even when
its `timestamp` or `webhook_id` differ. herdr-bus keeps the first message (see
[envelope.md](envelope.md#duplicates)), and the server replies `{"outcome": "dropped"}` and
publishes no second task request.

## Limits

- The passcode is a shared value inside the body, not a signature: see
  [What the passcode proves](#what-the-passcode-proves).
- A delivery Figma never sends, or gives up on after 3 hours, never reaches the factory. Nothing
  polls Figma to recover it.
- Only `FILE_COMMENT` publishes. Version, library and Dev Mode events publish nothing.
- A body over 1 MiB is refused. A comment is a few kilobytes.

## Sources

Read on 2026-11-22:
[Webhooks](https://developers.figma.com/docs/rest-api/webhooks/),
[Webhook events](https://developers.figma.com/docs/rest-api/webhooks-events/),
[Webhook endpoints](https://developers.figma.com/docs/rest-api/webhooks-endpoints/),
[Webhook security](https://developers.figma.com/docs/rest-api/webhooks-security/),
[Authentication](https://developers.figma.com/docs/rest-api/authentication/),
[Scopes](https://developers.figma.com/docs/rest-api/scopes/),
[Forum report and Figma's reply](https://forum.figma.com/report-a-problem-6/rest-v2-team-webhook-file-comment-silently-stops-delivering-after-1-to-4h-status-stays-active-no-failed-attempts-logged-55036).
