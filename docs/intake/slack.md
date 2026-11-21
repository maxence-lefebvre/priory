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
as a join.

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
| Calls per poll | one `conversations.history` per trigger channel, one `conversations.replies` per watched thread, and one per extra page |
| Scopes | `channels:history`, and `groups:history` for a private channel. The bot must be a channel member |

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
