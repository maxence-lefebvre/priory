# The daemon

`priory daemon [DIR] [--once]` reads the herdr-bus output `factory.intake` as the consumer `priory`,
records each message in the inbox and routes it to a task. It checks the factory first, as
`priory check` does, and refuses to start while it has problems. `--once` handles what is pending
and exits. Without it, the daemon looks for new messages every second.

State is one SQLite file, named by `PRIORY_DB`. The daemon refuses to start without it, or with an
empty name or `:memory:`, which SQLite would not keep.

## One message at a time

Each message is handled in one SQLite transaction: its inbox entry, any task it creates, and the
entry's outcome. The daemon acks the message on the bus only after that transaction commits. A
message delivered again, after a crash between the commit and the ack, adds nothing: the inbox
drops a message id it already holds. An error stops the daemon before the ack, so the next start
handles that message again.

## Routing

A message names a task key: a task request its `key`, a GitHub event `github:{repo}#{issue}`. The
active task that owns that key, if any, decides the route. A task is active in any state except
done, failed and dropped.

| Message                                   | Outcome   | Task                                         |
| ----------------------------------------- | --------- | -------------------------------------------- |
| Any, with an active task owning its key   | `kept`    | that task                                    |
| `factory.task.requested`, no active task  | `created` | a new task, `queued` at its workflow's start |
| `factory.task.requested`, malformed       | `ignored` | none                                         |
| Any other                                 | `ignored` | none                                         |

A task request for a workflow the factory lacks stops the daemon with an error, before its ack. The
request waits on the bus until the workflow is back. A malformed request is ignored instead, so that
it cannot block the output.

A source event comes before its task request, so it is recorded as `ignored`. The task keeps its
id in `source`.

## Listings

`priory inbox` prints each entry, oldest first: seq, type, subject, task and outcome.
`priory tasks` prints each task: id, key, workflow, step, state, band and lane. Both read
`PRIORY_DB` and print `-` for an empty cell, tab-separated after a header line.
