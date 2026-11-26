# Watches

Triggers tell the intake which repositories, Linear views and Slack channels to poll. An agent's
wait can name something no trigger covers, such as a Slack thread where it asked a question. This
page explains how the intake learns about those waits and polls for them.

## Where the list comes from

Each runner's daemon posts what its intake would watch on the log, as
[`priory.watch.posted`](../reference/events.md#priorywatchposted), when another machine can read
that log: in a factory of several runners, or with a Postgres log. It posts on start, then when the
list changed, at most once every 30 seconds, and once a day, with its runner's name, or its hostname
in a factory of one runner. So
an intake on another machine, such as the diocese, watches every runner's waits without their state
files.

The intake reads the log as its own consumer, `priory.intake`, on every poll, and keeps each
runner's newest posting. On start, it reads the log back, at most 7 days, to each runner's newest
posting. What it watches depends on the state file that `PRIORY_DB` names:

| `PRIORY_DB` | Factory | Watches |
| ----------- | ------- | ------- |
| set | one runner | what the state file holds, as before |
| set | several runners | what the state file holds of its own runner, and the other runners' newest postings |
| unset, as on the diocese | one runner | the newest posting, whatever runner name it carries |
| unset | several runners | every runner's newest posting |

The state file is fresher than its runner's posting, which follows up to 30 seconds later. So a
single-machine factory with a SQLite log works as before.

- A runner that ends a task posts a list without it, so the intake stops watching it on its next
  poll after that posting.
- A runner's list expires 7 days after its last posting. A running daemon posts daily, so only a
  runner stopped for a week loses its watches, and it posts again when it starts. A runner asleep
  for less keeps them: the log holds the events for it until it wakes.
- In a factory of several runners, a posting from a runner without a runner file is dropped. The
  intake reads the runner files when it starts, so removing a runner's file stops its watches once
  the intake restarts.

Without a state file or a posting, the intake watches none of them.

## What it watches

Each runner's list holds what its state file has of:

| From | Kinds |
| ---- | ----- |
| open waits | `linear_comment`, `linear_state`, `slack_reply`, `pull_request`, `checks` and `review` (see [waits.md](../daemon/waits.md)) |
| open holds | unmet `--merge` and `--after <issue>` conditions (see Holds in [runs.md](../daemon/runs.md)) |
| subscriptions | every resource a task subscribes to, its own key's among them (see `subscribe` in [tools.md](../reference/tools.md#subscribeon-and-unsubscribeon)) |
| task branches | `github:<owner/name>@priory/<task>` of an active task, or of one done in the last 10 minutes whose branch no event named yet |

It skips the waits and holds of a task that is `done`, `failed` or `dropped`, and the subscriptions
of one that is `failed` or `dropped`, except its own key while it is the newest task on that key.
A done task's subscriptions stay for 7 days after it was done, so feedback can reopen it. A comment
on a failed or dropped task's key posts a report instead (see
[routing.md](../daemon/routing.md#feedback-on-a-failed-or-dropped-task)).
An ended task's own Slack thread, its key, is read for 7 days only after the task ended, since each
thread costs a call per poll. A thread that a wait and a subscription
both name is read once a poll, from the earlier of their times.

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

For each Slack thread, the intake reads `conversations.replies` from the wait's or subscription's
time on, every poll, until the wait fires or is withdrawn, or the subscription ends. herdr-bus drops
the replies it already holds. A task a Slack trigger started subscribes to its own thread, so the
intake reads that thread while the task is active and for 7 days after it is done.

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
