# Runners own tasks through claims and leases on the shared log

Date: 2026-11-21. Status: accepted.

## Context

R9.1 in the PRD says each task is owned by one runner at a time, which places its runs. R9.2 says
a runner keeps its tasks going while the other machines are offline, and an answer given on
another machine meanwhile reaches the task when its runner reconnects, through the revision check
of R6.3. M3 is done when a runner is disconnected for an hour while tasks run, then reconnected,
with no task run twice and no answer lost.

Today one daemon reads the log as the consumer `priory` and keeps every task in its own SQLite
file. Two daemons on one log would share that cursor, so each would see half the messages, and
each would create a task for every request it saw.

## Decision

### Who takes part

Ownership applies when the factory has two or more runner files. The daemon is the runner named
by its machine's hostname, and refuses to start when `runners/<hostname>.toml` is missing. A
factory with one runner file or none keeps today's behaviour: one daemon owns every task and
publishes nothing new. So every example factory, which ships `runners/my-machine.toml`, runs as
before.

### Every runner reads the whole log

Each runner reads `factory.intake` as its own consumer, `priory.<runner>`, and records every
message in its own inbox. Ownership messages go on the same output, so every runner sees requests,
claims and leases in one order. Each runner computes the same owner for every key from that order,
with no coordinator.

Each runner keeps only its own tasks in its own state file. Nothing else is shared. A source event
for a task owned elsewhere finds no owner in this runner's state and is recorded as `ignored`, as
today.

### Claims

A task request whose key no runner owns does not create a task at once. The runner publishes
`priory.task.claimed` with the key, its name and the request's id, and records the request as
`claiming`. The first claim of a key in log order wins. The winner creates the task from the
request it stored in its inbox. Every other runner records the owner. A request for a key that is
owned already goes to the owner, as a second request goes to an active task today.

A claim message has an id derived from the request and the runner, so a daemon that restarts and
claims again adds nothing. A claim of a request that came before the key's last winning claim
never wins, so a runner that reads the log late does not start a finished task over. Each runner
records the log in its order, so it judges that alike.

### Leases

Each runner publishes `priory.runner.alive` every minute, with the time of its clock. A runner's
lease ends 30 minutes after its last alive message. Lease times compare message times in log
order, never a reader's clock, so every runner agrees on which lease ended at which message.

The owner also publishes `priory.task.held` when one of its tasks changes between these three
holds:

| Hold | The task | Can another runner take it |
| ---- | -------- | -------------------------- |
| `waiting` | queued or ready, and no run of it ever started | yes, once its runner's lease ends |
| `kept` | any other active state: running, blocked, held, or asleep on an ask or a wait | only when a person gives it |
| `ended` | done, failed or dropped | no |

The daemon publishes these on its tick by comparing each task with the hold it published last. A
runner starts the first run of a `waiting` task only after it published `kept` for it and read that
back from the log, so a takeover and a first run never both win.

A `kept` task never moves on its own. In a partition, the cut-off runner keeps its started tasks
going, as R9.2 asks, so a second runner that restarted them would run them twice. A task asleep on
an ask keeps its ask and sessions on its runner, so an answer given elsewhere reaches it there.

### Takeover

When a runner reads its own alive message, it looks for `waiting` keys of other runners whose lease
ended before that message. It publishes a takeover claim for each, naming the runner it takes from.
The claim wins only if, at its place in the log, that runner is still the owner, the key is still
`waiting`, and the lease is still over. An alive message from the old owner in between voids it.

The new owner creates the task from its request, at the workflow's start. No run of it started, so
nothing is lost. When the old owner's daemon starts again, it reads the log before it starts any
run, finds the takeover, and drops its copy of the task with the reason
`moved to runner <name>: its lease ended`.

### Giving a task to another runner

`priory give KEY RUNNER` publishes a claim of the key for `RUNNER`, straight to the log, as
`priory request` does. It names the owner it takes from and wins whatever the hold and the lease,
as long as that runner still owns the key at the claim's place in the log. A person uses it to move
a started task off a runner that will not come back. The new owner starts the task over from its
request. When the old owner reads the claim, it ends its live run and drops its copy with the
reason `given to runner <name>`. If the old runner was only cut off, its run went on until then:
giving a started task trades that overlap for progress, and the person chooses it.

### Answers from another runner

`priory answer <runner>:<ask> REVISION TEXT` publishes `priory.ask.answered` with the runner, the
ask, the revision and the text, straight to the log, as `priory request` does. Only the named
runner applies it, through the same path as a local answer. So the revision check of R6.3 holds:
the first answer to a revision stands, a later one is refused and recorded, and a redelivered
message adds nothing because the inbox drops an id it holds. A runner that is offline applies the
answer when it reads the log again.

### Why not herdr-bus requests

R9.3 names requests with a deadline, a take and a reply. A request needs one side that takes it.
With claims on one ordered output, the log's order is that side: the first claim wins, and every
runner reaches the same verdict alone. The lease is the deadline. Requests stay out of the `Log`
trait, as ADR 0001 left them.

## Consequences

- One migration adds a `claims` table (key, owner, request, hold, hold published), a `leases`
  table (runner, last alive time) and a `sends` table, the messages queued for the log.
- A runner keeps every started task through any absence. Only tasks that never started move after
  30 minutes, so no task runs on two runners unless a person gives a started one away.
- A started task on a runner that never comes back waits for `priory give`.
- A given task starts over, so work its old runner had not pushed is not carried.
- A daemon that cannot reach its log stops with an error, so a runner cut off from a Postgres log
  stops instead of riding out the partition. On herdr-bus, the runners share one file, so a
  disconnect can only be a stopped daemon.
- Every runner posts its asks on the log, and every desk lists them all, as R8.5 asks. See
  [runners.md](../daemon/runners.md#every-runners-asks).
- Heartbeats add one message per runner per minute to `factory.intake`.
