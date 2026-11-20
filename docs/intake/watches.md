# Watches

Triggers tell the intake which repositories, Linear views and Slack channels to poll. An agent's
wait can name something no trigger covers, such as a Slack thread where it asked a question. This
page explains how the intake learns about those waits and polls for them.

## What it watches

On every poll, the intake reads the state file that `PRIORY_DB` names and collects:

| From | Kinds |
| ---- | ----- |
| open waits | `linear_comment`, `linear_state`, `slack_reply`, `pull_request`, `checks` and `review` (see [waits.md](../daemon/waits.md)) |
| open holds | unmet `--merge` and `--after <issue>` conditions (see Holds in [runs.md](../daemon/runs.md)) |
| subscriptions | every resource a task subscribes to (see `subscribe` in [tools.md](../daemon/tools.md#subscribeon-and-unsubscribeon)) |
| task branches | `github:<owner/name>@priory/<task>` of an active task, or of one done in the last 10 minutes that subscribes to nothing |

It skips the waits and holds of a task that is `done`, `failed` or `dropped`, and the subscriptions
of one that is `failed` or `dropped`. A done task's subscriptions stay, so feedback can reopen it. Without `PRIORY_DB`,
the intake watches none of them.

Each wait gives a resource and the time of its agent's last prompt. A subscription gives its
resource and when it was made, and a branch the time its task last changed state. Each hold condition gives its
pull request or issue and the time the hold was set. The intake reads from that time on.

| Resource | Example | Polled as |
| -------- | ------- | --------- |
| `github:<owner/name>#<number>` | `github:owner/name#42` | its repository |
| `linear:<issue>` | `linear:ENG-7` | the Linear issues and comments streams |
| `slack:<channel>:<thread ts>` | `slack:C0123:1763500000.000100` | that thread |

A missing key, such as an unset `SLACK_BOT_TOKEN`, fails only that source's poll. So a wait cannot
stop GitHub polling.

## Pull requests

The intake polls the repository of a pull request that a task waits on, is held for or subscribes
to, the same way it polls a trigger's repository (see [github-polling.md](github-polling.md)).

- A repository that no trigger names starts with a baseline. The reviews and checks of that baseline
  still reach the wait or the subscriber.
- After a restart, the intake resumes the repository from where it stopped, as long as a wait, a
  hold, a subscription or a task branch still names it.

## Slack threads

For each Slack thread, the intake reads `conversations.replies` from the wait's time on, every poll,
until the wait fires or is withdrawn. herdr-bus drops the replies it already holds.

For example, an agent posts a question in a thread, then waits on:

```json
{ "on": { "type": "slack_reply", "channel": "C0123", "thread_ts": "1763500000.000100" } }
```

From the next poll on, the intake reads that thread's replies posted at or after the agent's last
prompt. A person's reply becomes a `slack.thread.replied` event, which fires the wait and wakes the
agent (see [slack.md](slack.md)).

## Linear issues

For a Linear issue, the intake starts the issues and comments streams, if no Linear trigger has
started them already. In two queries, these streams read every issue updated and every comment
created in the workspace since their position. So the intake never polls one issue at a time.

The first poll of a stream is a baseline. Its events still reach the wait, because the daemon judges
events by their time, not by their `baseline` flag.
