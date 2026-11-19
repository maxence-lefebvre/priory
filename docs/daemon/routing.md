# Messages and routing

## One message at a time

Each message is handled in one SQLite transaction: its inbox entry, any task it creates, and the
entry's outcome. The daemon acks the message on the bus only after that transaction commits. A
message delivered again, after a crash between the commit and the ack, adds nothing: the inbox
drops a message id it already holds. An error stops the daemon before the ack, so the next start
handles that message again.

## Routing

A task owns resources, stored in one table: its trigger key from its creation, its branch
`github:{repo}@priory/{id}` from its first run, and each pull request `github:{repo}#{n}` a wait of
its names. A message names resources too: a task request its `key`, a CLI request its subject, a
GitHub event its issue or pull request `github:{repo}#{n}` and, for a pull request event, its head
branch `github:{repo}@{head_branch}`. The oldest active task that owns any of them decides the route.
A task is active in any state except done, failed and dropped. So an agent that opens a pull request
from its branch gets that pull request's events, with no trigger involved.

| Message                                   | Outcome   | Task                                         |
| ----------------------------------------- | --------- | -------------------------------------------- |
| A schedule's `factory.task.requested`, with a task asleep at a model step owning its key | `woke` | that task, now `ready` |
| A schedule's `factory.task.requested`, with a task in another active state owning its key | `coalesced` | that task, unchanged |
| Any other, with an active task owning a resource it names | `kept` | that task                   |
| `factory.task.requested`, no active task  | `created` | a new task, `queued` at its workflow's start |
| `factory.task.requested`, malformed       | `ignored` | none                                         |
| Any other                                 | `ignored` | none                                         |

A task request for a workflow the factory lacks still creates its task, `queued` with no step, so
the daemon keeps serving. Each tick gives it the reason `waits for workflow <name>, which the
factory lacks`. Once a daemon starts with that workflow, the task moves to the workflow's start. A
malformed request is ignored.

A source event comes before its task request, so it is recorded as `ignored`. The task keeps its
id in `source`.

A schedule's task request carries `scheduled` (see `docs/intake/schedule.md`). Only a schedule with a fixed
`key` sends a second one for the same key. One that finds the task asleep at a model step wakes it,
even when its wait does not match: its prompt is the trigger's `prompt`, then the firing's line,
which carries the firing time, then the events the task kept while it slept. Its open ask and wait
stay open: each new run of a task takes over its open asks and agent waits, so the next `Stop`
puts the task back to sleep on them. One that finds the task in any other state, such as running,
asleep at a human step, or asleep on the daemon's checks gate, is recorded as `coalesced` and is
never typed, so firings never stack.

A message kept for a `running` task also gets an outbox prompt, so the agent reads it after its
current turn. A message kept for an asleep task that satisfies its wait wakes it, as described under
Waits. A message kept for a task in any other state waits for the task's next run.
