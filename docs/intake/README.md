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

## What it polls

| Source | Polled when | Read through |
| ------ | ----------- | ------------ |
| GitHub | a trigger names the repository, whatever the trigger's source, or an open wait or `--merge` hold names a pull request in it | `gh api` and `gh api graphql`, as the user `gh` is logged in as |
| Linear | the factory has a Linear trigger, or an open wait names a Linear issue | `curl` (see Linear calls in [linear.md](linear.md)) |
| Slack | the factory has a Slack trigger, or an open wait names a Slack thread | `curl` (see Slack calls in [slack.md](slack.md)) |
| Schedules | on every poll, for each schedule trigger that is due | no network (see [schedule.md](schedule.md)) |

[watches.md](watches.md) explains how open waits add to what the intake polls.

Each `gh api` call runs in its own process group. Past 5 minutes, or past `command_timeout` in
`factory.toml`, the call is killed and the poll fails with `timed out after <n>s`. So a hung call
cannot hold up schedules, Linear and Slack forever.

## Where messages go

The intake publishes on the output `factory.intake` of [herdr-bus](https://github.com/flupke/herdr-bus),
a message log in one SQLite file that `HERDR_BUS_DB` names. Several consumers can read one output,
each at its own position.

These pages are the contract for every consumer of that output: the Priory daemon, and another
factory such as herdr-bowtie.

## Which file to read

- [envelope.md](envelope.md): the fields every message carries, and how each message id is made.
- [github-issues.md](github-issues.md): `github.issues.labeled` and `github.issues.opened`.
- [pull-requests.md](pull-requests.md): pull request, review and check run events, and check run snapshots.
- [schedule.md](schedule.md): `schedule.fired` and when a schedule trigger fires.
- [linear.md](linear.md): Linear events, Linear streams and Linear API calls.
- [slack.md](slack.md): Slack events, the channel stream and Slack API calls.
- [watches.md](watches.md): the Linear issues, Slack threads and pull requests an open wait makes the intake watch.
- [cli.md](cli.md): `priory request` and the `priory.task.requested.cli` event.
- [task-requests.md](task-requests.md): `factory.task.requested`, which creates tasks, and its `hold`.
- [github-polling.md](github-polling.md): GitHub streams, baselines, API cost and recovery after a restart.
