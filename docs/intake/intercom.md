# Intercom

An Intercom conversation that a customer starts can start a task. This page explains how an
Intercom notification becomes an `intercom.conversation.created` message, and what Priory checks
first. The keys are under [`[webhooks.intercom]`](../reference/factory.md#webhooksintercom), the
trigger under [`intercom`](../reference/triggers.md#intercom), the endpoint in
[serve.md](../reference/serve.md#post-hooksintercom) and the message in
[events.md](../reference/events.md#intercomconversationcreated).

## Webhook only

Intercom conversations arrive only by webhook. The intake never polls Intercom, and Priory makes no
Intercom API call, so it needs no access token, only the app's client secret. The first message's
text comes in the notification, so the agent starts from it without a call.

So a notification that never arrives is lost. `priory check` refuses an `intercom` trigger without
`[webhooks.intercom]`, since no other path would ever fire it.

## The signature comes first

The server takes notifications at `POST /hooks/intercom`, with no token. It runs these checks in
order:

1. The body is read up to 1 MiB. A larger one gets `413` before it is read.
2. `X-Hub-Signature` must be `sha1=` followed by the hex HMAC-SHA1 of the exact body bytes, keyed
   with the app's client secret. The server compares them in constant time. A missing or wrong one,
   or one without the `sha1=` prefix, gets `401`. The body is dropped unparsed, and the server never
   logs it.
3. Only then does it parse the JSON. A notification whose `app_id` is not the workspace that
   `[webhooks.intercom] workspace` names gets `202` and publishes nothing.
4. It maps the notification, publishes it and replies `202`.

SHA-1 is a weak hash, but HMAC-SHA1 still resists forgery, and it is the only signature Intercom
sends. The server answers `HEAD /hooks/intercom` with `200`, which Intercom asks before it saves the
webhook URL.

Intercom waits 5 seconds for a reply, and sends a notification again after a timeout or an error
reply. The route makes no call to Intercom: it checks, maps, publishes on the event log and replies.

## The workspace filter

Intercom sends an app's notifications from every workspace that installed it. `workspace` names the
one whose conversations Priory takes, so another workspace that installs the app never reaches the
factory. The workspace id is the `app_id` of a notification. It also shows in your Intercom URLs,
such as `https://app.intercom.com/a/apps/<workspace id>/`.

## Setting it up

1. In the Intercom Developer Hub, open your app. A private app reads your own workspace.
2. Under **Configure**, then **Webhooks**, set the endpoint URL to `<url>/hooks/intercom`. Intercom
   needs HTTPS. See [Reaching the server](webhooks.md#reaching-the-server) for a URL Intercom can
   reach.
3. Subscribe to the topic `conversation.user.created`. It needs the Read conversations permission
   on the app's **Authentication** page.
4. Copy the app's client secret from its **Basic Info** page into the services' environment file:

   ```sh
   echo "INTERCOM_CLIENT_SECRET=<the client secret>" >> ~/.config/priory/env
   ```

5. Add `[webhooks.intercom]` with your workspace id to `factory.toml` and an `intercom` trigger,
   then restart the daemon.

Intercom's docs say webhooks work from a paid or development workspace.

## What a notification publishes

| `topic` | Publishes |
| ------- | --------- |
| `conversation.user.created`, from the workspace | `intercom.conversation.created`, then a task request per `intercom` trigger |
| `conversation.user.created`, from another workspace | nothing; `202` |
| any other topic, such as `conversation.user.replied` or `ping` | nothing; `202` |

The task key is `intercom:<conversation id>`, from `data.item.id`. A later slice that maps replies
would route them to the same task.

The message carries the first message's `subject` and `body`. From API version 2.15, Intercom sends
them as plain text; before it, the body may hold HTML. Both are the customer's words, so a workflow
must treat them as untrusted text. The message also names the author and their type, such as `user`
or `lead`, and leaves out their email.

Its `time` is the conversation's `created_at`. Its `delivery` is the notification's `id`, which
sits in the signed body.

## Duplicates

The message id is a UUIDv5 of `intercom:conversation:{id}:created`. A resend, or a replay of the
signed body, makes the same id. herdr-bus keeps the first message (see
[envelope.md](envelope.md#duplicates)), and the server replies `{"outcome": "dropped"}` and
publishes no second task request.

## Limits

- Intercom sends a failed notification once more, a minute later. A notification that fails both
  times, while the server is down or unreachable, never reaches the factory: nothing polls Intercom
  to recover it. Intercom also suspends a private app's webhooks after seven days of error replies.
- Only `conversation.user.created` publishes. Replies, admin messages and other topics publish
  nothing yet.
- One `[webhooks.intercom]` takes one workspace.
- A body over 1 MiB is refused.

## Sources

Read on 2026-11-22:
[Webhooks](https://developers.intercom.com/docs/webhooks),
[Set up webhooks](https://developers.intercom.com/docs/webhooks/setting-up-webhooks),
[Webhook notifications](https://developers.intercom.com/docs/webhooks/webhook-notifications),
[Webhook topics and signature](https://developers.intercom.com/docs/references/webhooks/webhook-models),
[The conversation model](https://developers.intercom.com/docs/references/rest-api/api.intercom.io/conversations/conversation).
