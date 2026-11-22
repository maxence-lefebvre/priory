# Intake messages

The intake is the process that finds new work. It polls GitHub, Linear and Slack, fires schedules,
and publishes what it finds as messages. The Priory daemon reads those messages and turns them into
tasks.

```sh
priory intake [DIR] [--once]
```

`DIR` is the factory folder, `.` by default. The intake checks the factory first, as `priory check`
does, and refuses to start while it has problems. Without `--once`, it polls every 60 seconds.
With `--once`, it polls once and exits.

For lookup, [events.md](../reference/events.md) lists every message type with its fields and an
example, and [triggers.md](../reference/triggers.md) lists every trigger key. These pages explain
how each source behaves.

## What it polls

| Source | Polled when | Read through |
| ------ | ----------- | ------------ |
| GitHub | a trigger names the repository, whatever the trigger's source, or an open wait or `--merge` hold names a pull request in it | `gh api` and `gh api graphql`, as the user `gh` is logged in as |
| GitHub review requests | a `pull_request.review_requested` trigger names the reviewer; every repository | `gh api search/issues` (see [review-requests.md](review-requests.md#calls-and-cost)) |
| Linear | the factory has a Linear trigger, or an open wait names a Linear issue | `curl` (see [linear.md](linear.md#calls-and-cost)) |
| Slack | the factory has a Slack trigger, or an open wait names a Slack thread | `curl` (see [slack.md](slack.md#calls-and-cost)) |
| Schedules | on every poll, for each schedule trigger that is due | no network (see [schedule.md](schedule.md)) |

[watches.md](watches.md) explains how open waits add to what the intake polls.

Each `gh api` call runs in its own process group. Past 5 minutes, or past `command_timeout` in
`factory.toml`, the call is killed and the poll fails with `timed out after <n>s`. So a hung call
cannot hold up schedules, Linear and Slack forever.

With `PRIORY_DB` set, the intake records each poll in the state. A source whose poll fails posts
one `intake` report and shows a banner on the desk, and an intake that has not polled for 5 minutes
shows one too: see [desk.md](../daemon/desk.md#intake-banner).

## Where messages go

The intake publishes on the output `factory.intake` of [herdr-bus](https://github.com/flupke/herdr-bus),
a message log in one SQLite file that `HERDR_BUS_DB` names, `~/.local/share/herdr-bus/bus.db` by
default. Several consumers can read one output,
each at its own position.

These pages are the contract for every consumer of that output: the Priory daemon, and another
factory such as herdr-bowtie. Each message type also has a JSON Schema in
[`docs/schemas/events/`](../schemas/events/).

## Which file to read

| File | Covers |
| ---- | ------ |
| [envelope.md](envelope.md) | The fields every message carries, and how each message id is made |
| [github-issues.md](github-issues.md) | `github.issues.labeled`, `github.issues.opened`, `github.issues.closed` and `github.issues.comment` |
| [pull-requests.md](pull-requests.md) | Pull request, review and check run events, and check run snapshots |
| [github-polling.md](github-polling.md) | GitHub streams, baselines, API cost and recovery after a restart |
| [review-requests.md](review-requests.md) | `github.pull_request.review_requested` and `review_request_removed`, and the review tasks they start and withdraw |
| [linear.md](linear.md) | Linear events, streams and API calls |
| [slack.md](slack.md) | Slack events, the channel stream, API calls and the "Queue in Priory" message shortcut |
| [schedule.md](schedule.md) | `schedule.fired` and when a schedule trigger fires |
| [webhooks.md](webhooks.md) | GitHub and Linear webhooks: the signature, what a delivery publishes, and duplicates with polling |
| [sentry.md](sentry.md) | `sentry.issue.created`, which only Sentry's signed webhook publishes |
| [cloudwatch.md](cloudwatch.md) | `cloudwatch.alarm.state_changed` from an EventBridge API destination, and how to set up EventBridge |
| [honeycomb.md](honeycomb.md) | `honeycomb.trigger.fired`, which only Honeycomb's webhook publishes, and its payload template |
| [figma.md](figma.md) | `figma.file.comment_created`, what Figma's passcode proves, and how to create the webhook |
| [intercom.md](intercom.md) | `intercom.conversation.created`, which only Intercom's signed webhook publishes, and the workspace filter |
| [watches.md](watches.md) | The Linear issues, Slack threads and pull requests an open wait makes the intake watch |
| [cli.md](cli.md) | `priory request`, `priory add`, `priory batch add` and the `priory.task.requested.cli` event |
| [task-requests.md](task-requests.md) | `factory.task.requested`, which creates tasks, and its `hold` |
