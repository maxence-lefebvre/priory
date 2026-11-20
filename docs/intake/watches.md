# Watches

Triggers tell the intake which repositories, Linear views and Slack channels to poll. An agent's
wait can name something no trigger covers, such as a Slack thread where it asked a question. This
page explains how the intake learns about those waits and polls for them.

## What it watches

On every poll, the intake reads the state file that `PRIORY_DB` names and collects:

| From | Kinds |
| ---- | ----- |
| open waits | `linear_comment`, `linear_state`, `slack_reply`, `pull_request`, `checks` and `review` (see [waits.md](../daemon/waits.md)) |
| open holds | `--merge` (see Holds in [runs.md](../daemon/runs.md)) |

It skips the waits and holds of a task that is `done`, `failed` or `dropped`. Without `PRIORY_DB`,
the intake watches none of them.

Each wait gives a resource and the time of its agent's last prompt. Each hold gives its pull request
and the time the hold was set. The intake reads from that time on.

| Resource | Example | Polled as |
| -------- | ------- | --------- |
| `github:<owner/name>#<number>` | `github:owner/name#42` | its repository |
| `linear:<issue>` | `linear:ENG-7` | the Linear issues and comments streams |
| `slack:<channel>:<thread ts>` | `slack:C0123:1763500000.000100` | that thread |

A missing key, such as an unset `SLACK_BOT_TOKEN`, fails only that source's poll. So a wait cannot
stop GitHub polling.

## Pull requests

The intake polls the repository of a pull request that a task waits on or is held for, the same way
it polls a trigger's repository (see [github-polling.md](github-polling.md)).

- A repository that no trigger names starts with a baseline. The reviews and checks of that baseline
  still reach the wait.
- After a restart, the intake resumes the repository from where it stopped, as long as a wait or
  hold still names it.

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
