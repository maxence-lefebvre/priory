# Messages and routing

## One message at a time

Each message is handled in one SQLite transaction: its inbox entry, any task it creates, and the
entry's outcome. The daemon acks it on the bus only after the commit. A message redelivered after
a crash between commit and ack adds nothing: the inbox drops a message id it already holds. An
error stops the daemon before the ack, so the next start handles it again.

## Routing

A task owns resources, stored in one table: its trigger key from its creation, its branch
`github:{repo}@priory/{id}` from its first run, and each pull request `github:{repo}#{n}` a wait of
its names. A message names resources too: a task request its `key`, a CLI request its subject, a
GitHub event its issue or pull request `github:{repo}#{n}` and, for a pull request event, its head
branch `github:{repo}@{head_branch}`. The active task whose own key is one of them decides the
route, else the oldest active task owning any of them. Each other active task that owns one of them
still has its open wait fired when the message satisfies it, so two tasks can share a pull request:
task B, created for pull request 7, gets its events, and task A, which waited on 7 before B existed,
still wakes on the close or review it waits for. A task may wait again on a resource it already
owns, even while another task's key decides its route.
A task request is the exception: only an active task whose own key is the request's `key` keeps it.
A task that owns that key only as a pull request or an issue it waits on does not, so the request
creates its task.
A task is active in any state except done, failed and dropped. So an agent that opens a pull request
from its branch gets that pull request's events, with no trigger.

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

A schedule's task request carries `scheduled` (see `docs/intake/schedule.md`). Only a schedule with
a fixed `key` sends a second one for the same key. A request that finds the task asleep at a model
step wakes it, even when its wait does not match. Its prompt is the trigger's `prompt`, then the
firing's line (which carries the firing time), then the events the task kept while asleep. Its open
ask and wait stay open. At its `Stop`, a run with no result takes over the task's open asks and agent
waits, so the task sleeps on them again. A run that calls `result`, `ask` or `wait` instead moves on:
the call withdraws the open asks and agent waits of earlier runs. A request that finds the task in any other state, such
as running, asleep at a human step, or asleep on the daemon's checks gate, is recorded as
`coalesced` and never typed, so firings never stack.

A message kept for a `running` task also gets an outbox prompt, so the agent reads it after its
current turn. A message kept for a task that satisfies its open wait fires the wait in any active
state (see Waits): an asleep task wakes, and a ready, blocked or held task gets the prompt for its
next run. Any other message kept for a task waits for its next run.
