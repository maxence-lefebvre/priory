# Slack

## Slack events

A trigger with `source = "slack"` names a `channel` by id, such as `C0123`, and starts on
`message.posted`. It may filter on `text`: it matches a message whose text contains it, ignoring case. It takes `backfill`, and
its `key`, when set, is `message.ts`. It may name the `repo`, `owner/name`, that its tasks work in.
Without one, its tasks block before their first run. Requested tasks get the key
`slack:<channel>:<ts>`.

| Type                   | One message per                                                     |
| ---------------------- | ------------------------------------------------------------------- |
| `slack.message.posted` | message posted in a trigger's channel, outside a thread             |
| `slack.thread.replied` | reply in a thread that an open `slack_reply` wait names, posted at or after its agent's last prompt |

The intake publishes neither for its own bot's message nor for one with a `subtype`, such as a
join. A reply never names a trigger. The daemon routes both to the active task owning
`slack:<channel>:<thread ts>`: a task a Slack trigger started, or one whose wait names the thread.

| Field       | Type             | Meaning                                                           |
| ----------- | ---------------- | ----------------------------------------------------------------- |
| `channel`   | string           | The channel id                                                    |
| `ts`        | string           | Slack's id of the message: seconds since the epoch, with microseconds |
| `thread_ts` | string           | The `ts` of the thread's first message. `slack.thread.replied` only |
| `user`      | string or null   | The Slack user id of the author                                   |
| `text`      | string           | The message, in Slack's markup                                    |
| `time`      | RFC 3339         | The instant `ts` names                                            |
| `baseline`  | boolean          | True when the message came from the first poll of its channel. Always false for a reply |
| `triggers`  | array of strings | As for GitHub events; empty for a reply                           |

```json
{
  "id": "0b8a4c1e-56f3-5a0e-9d7b-0d6d1c3f6a21",
  "type": "slack.thread.replied",
  "subject": "C0123:1763500000.000100",
  "time": "2026-11-18T21:06:41.000200Z",
  "data": {
    "channel": "C0123",
    "thread_ts": "1763500000.000100",
    "ts": "1763500001.000200",
    "user": "U0PERSON",
    "text": "Looks good, ship it.",
    "time": "2026-11-18T21:06:41.000200Z",
    "baseline": false,
    "triggers": []
  },
  "links": []
}
```

A channel is a stream named `slack.channel:<channel>`, whose position is the `ts` of the newest
message seen. The first poll reads the 50 newest messages as a baseline. Later polls read
`conversations.history` after the position, every page of 200. A poll that moves the position
publishes a `factory.intake.baseline` marker with it. A restart starts from the newest marker. A
crash before the marker rereads the same messages, and herdr-bus drops the repeats.

## Slack calls

The intake reads a bot token from the environment variable `SLACK_BOT_TOKEN`. It refuses to start
without it when the factory has a Slack trigger. It calls `https://slack.com/api/<method>` with
`curl --get`, which reads the `Authorization` header from stdin as for Linear, so the token never
shows in a process list or error. On start it calls `auth.test` once to learn its own bot. Each poll
makes one `conversations.history` call per trigger channel, one `conversations.replies` call per
watched thread, plus one per extra page. The bot needs the `channels:history` scope, `groups:history`
for a private channel, and must be a channel member. Slack limits both methods per app. An app
outside the Slack Marketplace created after May 2025 gets one call a minute and 15 messages per call,
so it can watch one channel or thread per minute at most. Each poll starts at the next watched
thread, so a rate limit delays every thread in turn rather than starving the later ones. A new
intake starts at the thread the current minute picks, so successive `--once` runs rotate too. A
failing channel publishes nothing, and fails the poll only after the other channels and the watched
threads are published.
