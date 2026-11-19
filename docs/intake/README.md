# Intake messages

`priory intake [DIR] [--once]` polls GitHub for the repositories that the factory's triggers name,
Linear when the factory has a Linear trigger or an open wait names a Linear issue, and Slack when
the factory has a Slack trigger or an open wait names a Slack thread (see Slack events). It publishes on the herdr-bus output `factory.intake`. This page is the contract for every consumer of
that output: the Priory daemon, and another factory such as herdr-bowtie.

The intake checks the factory first, as `priory check` does, and refuses to start while it has
problems. `--once` polls once and exits. Without it, the intake polls every 60 seconds. It reads
GitHub through `gh api` and `gh api graphql`, as the user `gh` is logged in as, and Linear through
`curl` (see Linear calls). Each poll also fires the factory's schedule triggers that are due (see
`schedule.fired`).

## Which file to read

- [envelope.md](envelope.md): the fields every message carries, and how each message id is made.
- [github-issues.md](github-issues.md): `github.issues.labeled` and `github.issues.opened`.
- [pull-requests.md](pull-requests.md): pull request, review and check run events, and check run snapshots.
- [schedule.md](schedule.md): `schedule.fired` and when a schedule trigger fires.
- [linear.md](linear.md): Linear events, Linear streams and Linear API calls.
- [slack.md](slack.md): Slack events, the channel stream and Slack API calls.
- [watches.md](watches.md): the Linear issues and Slack threads an open wait makes the intake watch.
- [cli.md](cli.md): `priory request` and the `priory.task.requested.cli` event.
- [task-requests.md](task-requests.md): `factory.task.requested`, which creates tasks, and its `hold`.
- [github-polling.md](github-polling.md): GitHub streams, baselines, API cost and recovery after a restart.
