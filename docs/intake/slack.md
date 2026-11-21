# Slack

The intake turns Slack messages into events: messages posted in a trigger's channel, and replies in
threads that a task waits on or subscribes to. A task a Slack trigger started subscribes to its own
thread, so replies under the message that started it reach it.

## What it watches

| Stream | Reads | Started by |
| ------ | ----- | ---------- |
| `slack.channel:<channel>` | `conversations.history` after the position, every page of 200 | a Slack trigger naming the channel |
| a thread | `conversations.replies` from the wait's or subscription's time on | an open `slack_reply` wait, or a subscription such as a Slack task's to its own thread (see [watches.md](watches.md)) |

The intake publishes nothing for its own bot's messages, nor for a message with a `subtype`, such
as a join. Two subtypes are a person's message and publish: `file_share`, a message with an upload,
whose `files` lists the file names, and `thread_broadcast`, a reply also sent to the channel. A
broadcast is published once, as a reply of its thread, and never as a channel message.

## Trigger

A Slack trigger starts on `message.posted` in one channel, named by id such as `C0123`. Its keys are
in [triggers.md](../reference/triggers.md#slack).

| Key | Value |
| --- | ----- |
| `on` | `message.posted` |
| `filter` | `text`: the message contains it, ignoring case |
| `key` | `message.ts`, when set |
| `backfill` | whether the first poll's messages start tasks |
| `repo` | the repository its tasks work in; without it they block before their first run |

```toml
source = "slack"
channel = "C0123"
on = "message.posted"
filter = { text = "@priory" }
repo = "owner/name"
workflow = "implement"
```

## Events

Their fields and examples are in [events.md](../reference/events.md#slack).

| Type | One message per |
| ---- | --------------- |
| `slack.message.posted` | message posted in a trigger's channel, outside a thread |
| `slack.thread.replied` | reply in a watched thread, posted at or after the time its watch reads from |

A reply never names a trigger, and its `baseline` is always false. The daemon routes both types to
the active task owning `slack:<channel>:<thread ts>`: a task a Slack trigger started, or one whose
wait or subscription names the thread. A reply in the thread of a done task reopens it (see
[routing.md](../daemon/routing.md#feedback-reopens-a-done-task)).

A reply's `actor` is its author's display name, so a prompt reads `Ana replied in ...` rather than
`U0123 replied in ...`. The intake asks `users.info` once per user id and keeps each name it gets for
the life of the process. Without the `users:read` scope, or when Slack fails to answer, `actor` is
the user id, and the intake prints the error once.

## Keys and ids

| What | Value |
| ---- | ----- |
| Task key | `slack:<channel>:<ts>` |
| Message id | the UUIDv5 of `slack:{channel}:{ts}`, for both types |
| Subject | `<channel>:<ts>`, or `<channel>:<thread ts>` for a reply |

## Positions and baselines

A channel's position is the `ts` of the newest message seen.

- The first poll reads the 50 newest messages as a baseline.
- A poll that moves the position publishes a `factory.intake.baseline` marker with it.
- A restart starts from the newest marker.
- A crash before the marker rereads the same messages, and herdr-bus drops the repeats.

## Calls and cost

| What | Value |
| ---- | ----- |
| Token | `SLACK_BOT_TOKEN`. The intake refuses to start without it when the factory has a Slack trigger |
| Endpoint | `https://slack.com/api/<method>`, through `curl --get` |
| On start | one `auth.test` call, to learn its own bot |
| Calls per poll | one `conversations.history` per trigger channel, one `conversations.replies` per watched thread, and one per extra page; one `users.info` per reply author the process has not named yet |
| Scopes | `channels:history`, and `groups:history` for a private channel. The bot must be a channel member. `users:read` lets a reply name its author; without it, replies carry the user id |

`curl` reads the `Authorization` header from stdin, as for Linear, so the token never shows in a
process list or an error.

## Limits

- Slack limits both methods per app. An app outside the Slack Marketplace created after May 2025
  gets one call a minute and 15 messages per call, so it can watch one channel or thread per minute
  at most.
- Each poll starts at the next watched thread, so a rate limit delays every thread in turn rather
  than starving the later ones. A new intake starts at the thread the current minute picks, so
  successive `--once` runs rotate too.
- A failing channel publishes nothing. It fails the poll only after the other channels and the
  watched threads are published.
- A thread the intake cannot read, such as one in a channel the bot is not a member of
  (`not_in_channel`) or a deleted one, fails the poll the same way, with an error that names
  `thread <channel>:<thread_ts>`. A rate limit does not fail it: each poll starts at the next
  thread, so every thread is read in turn. The desk then shows the Slack intake as failing until the thread
  reads again or no wait or subscription names it.

## Message shortcut

"Queue in Priory" is a message shortcut: in a message's menu, it opens a form with the workflow,
the repository and a note, then queues a task for the message's thread. Its key is
`slack:<channel>:<thread ts>`, so the replies a `slack_reply` wait watches in that thread reach the
task once the intake watches the thread, which needs the history scopes in [Calls and
cost](#calls-and-cost). Its settings are in [`[slack.shortcut]`](../reference/factory.md#slackshortcut).

### How it reaches Priory

`priory intake` opens one [Socket Mode](https://docs.slack.dev/apis/events-api/using-socket-mode)
connection: it calls `apps.connections.open` with the app-level token and connects to the WebSocket
URL Slack returns. Nothing listens on the machine. `priory intake --once` opens no connection.

| Step | What Priory does |
| ---- | ---------------- |
| Any envelope | Acknowledges it with its `envelope_id` before anything else, since Slack shows the person an error after 3 seconds. Then hands the payload to a worker. Up to 16 payloads wait; up to 16 more get a "busy" reply, and the rest are dropped |
| Shortcut (`callback_id` `priory_queue`) | Opens the form with `views.open`. The workflow starts at `[request] workflow`; an empty repository means `triggers/cli.toml`'s; the note starts as the message's text |
| Form submitted | Queues the task through the same request code as `priory add`, so `check_repo`, workflow and band rules apply. The task's text is the note and a link to the message. The `priory.task.requested.cli` event's `person` is the Slack user who sent the form, as `<username> (<user id>)` |
| Reply | Posts an ephemeral message to the shortcut's `response_url`: the task key, "already queued", or why nothing was queued |
| `disconnect` | Opens a new connection at once. Slack sends one every few hours |
| A dropped or silent socket, or a connection, handshake or write that takes over 30 seconds | Reconnects after 1 second, doubling to a minute while it keeps failing. A socket silent for 30 seconds gets a ping; one silent for 60 seconds is dropped |
| `ratelimited` from `apps.connections.open` | Waits the seconds of Slack's `Retry-After` header, up to an hour, then reconnects. The log line says `retrying in <n>s` |

Only the first submission for a thread queues a task while that task is active. A later one, even
with another note, replies that the thread is already queued. Once the thread's task is done,
failed or dropped, the next submission queues a new task with the same key, as a task request for a
key with no active task does (see [routing.md](../daemon/routing.md#outcomes)). The intake counts
the thread's ended tasks in `PRIORY_DB` to tell the two apart. Without `PRIORY_DB`, a thread is
queued once.

### Who can use it

The app-level token authenticates the connection, so only Slack sends on it, and Slack does not sign
the payloads. Slack's `user.id` says who used the shortcut, and the allowlist says who may: a
shortcut or form from a user id not in `[slack.shortcut] users` gets an ephemeral refusal, and
nothing is queued.

Each person makes their own Slack app, so the shortcut in their Slack reaches their own Priory.
Slack lets an app hold up to 10 connections and sends each payload to any one of them, so two
Priory intakes on one app would each get some of the shortcuts.

### Set it up

1. In [Slack's app settings](https://api.slack.com/apps), create an app from the manifest
   [slack-app-manifest.yaml](slack-app-manifest.yaml), and install it in your workspace.
2. Under Basic Information, generate an app-level token with the scope `connections:write`. Put it
   in `SLACK_APP_TOKEN` for `priory intake`.
3. Put the app's bot token, `xoxb-`, in `SLACK_BOT_TOKEN`. It opens the form.
4. Add your user id to `factory.toml`, then run `priory check`:

   ```toml
   [slack.shortcut]
   users = ["U0123ABCD"]
   ```

5. Restart `priory intake`. In Slack, open a message's menu and pick "Queue in Priory".

| What | Value |
| ---- | ----- |
| Tokens | the app-level token from `token_env`, for `apps.connections.open` only; `SLACK_BOT_TOKEN`, for `views.open` |
| Bot scope | `commands` |
| Calls per shortcut | one `views.open`, one `response_url` post. One `apps.connections.open` per connection |

Both tokens go to `curl` on its standard input, and its errors show `<SLACK_APP_TOKEN>` or
`<SLACK_BOT_TOKEN>` in their place.

### Limits

- A `response_url` lasts 30 minutes, so a form submitted later queues the task with no reply.
