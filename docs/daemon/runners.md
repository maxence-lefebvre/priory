# Several runners

A factory can span several machines, each a runner with its own file in `runners/`. Each task is
owned by one runner at a time, which places its runs. This page explains how the runners agree on
owners through the log, what happens while one is away, and how a person moves a task. The decision
and its alternatives are in
[ADR 0003](../adr/0003-runners-own-tasks-through-claims-and-leases-on-the-shared-log.md).

## When ownership applies

Ownership applies when the factory has two or more runner files. A daemon is the runner named by
its machine's hostname. With several runner files, a daemon on a machine without one refuses to
start:

```text
this machine, `<hostname>`, has no runner file in runners/, and the factory has several runners
```

A factory with one runner file or none works as before: its one daemon owns every task.

A daemon claims the key of each active task of its own whose key it does not hold, such as a task
from before the factory got its second runner, or one that feedback reopened. It starts no run of
that task until it reads its claim back. When that claim loses, because the log gives the key to
another runner, the daemon drops its copy with the reason `owned by runner <name>`.

The daemon also claims, once, the key of each task of its own that ended before the factory had
runners, with `ended` set. A new runner reads every old request of the log, and claims it. That
claim comes after the ended one in the log, so it loses, and the new runner does not run a finished
task again.

When a factory that had one runner gets a second, start the first runner's daemon first. If the new
runner's claim of one of those keys lands first, the first runner drops its copy, and the task
starts over on the new runner. A finished task runs again there.

## Every runner reads the whole log

Each runner reads `factory.intake` as its own consumer, `priory.<runner>`, so every runner sees every
message in the same order. Each keeps only its own tasks in its own state file, `PRIORY_DB`. A
source event of a task that another runner owns finds no task here and is recorded as `ignored`,
while the owner routes it as [routing.md](routing.md) says.

The runners publish four more message types on the same output, which
[events.md](../reference/events.md#runners) lists: `priory.task.claimed`, `priory.task.held`,
`priory.runner.alive` and `priory.ask.answered`. The daemon stores each claim, hold and alive
message in its state file before it publishes it, so a crash between the two publishes it at the
next start. A message published twice adds nothing, because the log keeps the first message of
each id. `priory answer` and `priory give` publish straight to the log.

## Claims

A task request whose key no runner holds creates no task at once. Each runner that reads it
publishes a claim of the key and records the request as `claiming`. The first claim in log order
wins. The winner creates the task from the request it stored. The others record the owner, and a
later request for the key goes to it: `elsewhere` on the others' inboxes.

A key whose task ended, done, failed or dropped, is free again: a new request claims it afresh. A
claim of a request that came before the key's last winning claim never wins, so a runner that reads
the log late does not start a finished task over. Another request of the key that arrived while the
claim was pending goes to the winner's task, as it would had the task existed. A task that moves
gets those requests again on its new runner.

## Holds and leases

The owner publishes each task's hold whenever it changes:

| Hold | The task | Can another runner take it |
| ---- | -------- | -------------------------- |
| `waiting` | queued or ready, and no run of it ever started | yes, once its runner's lease ends |
| `kept` | running, blocked, held, or asleep on an ask or a wait | only when a person gives it |
| `ended` | done, failed or dropped | no |

Each daemon publishes `priory.runner.alive` once a minute. A runner's lease ends 30 minutes after
its last alive message. Lease times compare message times in log order, never a reader's clock, so
every runner agrees on which lease ended at which message.

On Postgres, an alive, claim or held message takes the time the database stored it, so the leases
compare one clock. On herdr-bus, each message keeps its sender's time, so a takeover compares two
machines' clocks. Keep the runners' clocks synchronized, with NTP, or a runner whose clock runs
ahead takes waiting tasks before their lease ends.

When a runner reads its own alive message, it claims each `waiting` key of another runner whose
lease ended before it. That takeover wins only if the old owner sent no alive message in between
and the key is still `waiting`. The new owner creates the task from its request.

A runner never starts the first run of a `waiting` task at once. It publishes `kept` for it first,
and starts the run only once it reads that message back from the log. If a takeover lands before
it, the takeover wins and the runner drops the task. If `kept` lands first, the takeover loses. So
no run of a task that moved had started, and nothing is lost.

A `kept` task never moves on its own. A runner cut off from the others keeps its started tasks
going, so a second runner that started them over would run them twice. A task asleep on an ask
keeps its ask and sessions on its runner, so an answer given elsewhere reaches it there.

## A runner cut off from the log

A daemon that cannot reach its log keeps running. It goes on with its own tasks:

- A run that started goes on, and its turns, results, local asks and answers, and waits work as
  before. A person answers a local ask with `priory answer` on that machine.
- No run of a `waiting` task starts, and no code step's action runs while the log says `waiting`.
  The daemon queues `kept` for the task and starts it only once it reads that back from the log.
- Each claim, hold and alive message waits in the state file's queue, oldest first. Nothing reaches
  the other runners until the link is back.
- The daemon tries the log again 2 seconds after it fails, then waits twice as long after each
  failure, up to 1 minute.

The log is unreachable when the server cannot be reached, closes the connection, or does not answer
within the `[log]` `timeout`, 10 seconds by default (see
[factory.md](../reference/factory.md#log)). A call that waits past that timeout is given up on. Until
it ends, each later call gives up at once, so a hung link costs the daemon one timeout per try and
leaves no stack of waiting calls. A refused password or a bad statement is no lost link: the
daemon stops with that error.

Meanwhile a person sees the desk banner
`the log is unreachable since <time>: <error>; <n> messages wait to publish`, before the intake's
banner. It is `intake` in `priory desk --json` and `/api/desk`, so the web Desk and the Herdr Desk
show it too. On that machine, `priory give` and `priory answer <runner>:<ask>` refuse with
`the log is unreachable since <time>: try again when it is back`, since this runner's view of the
owners and asks is as old as its last read. They refuse the same way when their own publish gets no
answer.

A daemon that starts while its log is unreachable starts all the same, and connects on a later
try. `priory add` on a cut-off runner publishes its request to the log, so it fails until the link
is back.

## A runner back from away

A daemon reads the log before it starts any run and before it publishes what it queued. A task that
moved while it was away is dropped with the reason `moved to runner <name>: its lease ended`.
Answers given meanwhile on other runners apply in log order, as below. Then it publishes its queue
in order. Each queued message keeps its id, so one published twice adds nothing.

## Why no task runs twice

A task runs on two runners only if one runner takes over a task that runs on the other. A takeover
wins only when both hold:

- the owner's last alive message is 30 minutes or more before the takeover, in log order, and
- the task is still `waiting` at the takeover's place in the log.

A runner cut off from the log never starts a `waiting` task, since it cannot read its `kept` back.
So a task that runs there was `kept` on the log before the cut, and no takeover wins it, however
long the cut lasts. A task that was `waiting` stays unstarted on the cut-off runner, so the runner
that takes it over runs it first and alone. `priory give` is the one exception: a person moves a
started task on purpose, as below.

## Answering another runner's ask

```sh
priory answer [DIR] <runner>:<ask> <option-or-text> --revision <n>
```

publishes the answer on the log of the factory in `DIR`, `.` by default, instead of sending it to
this machine's daemon. Only that runner
applies it, through the same check as a local answer: the first answer to a revision stands, and
a later one is refused and recorded. A runner that is away applies it when its daemon starts again.
The ask's id and revision are the ones that runner's `priory asks` prints. An ask of this machine
whose name is `<runner>:<ask>` is answered here first. `priory answer` refuses a runner without a
runner file with `runner <runner> has no runner file in runners/`.

## Giving a task to another runner

```sh
priory give [DIR] <key> <runner>
```

publishes a claim of the key for `<runner>` on the log of the factory in `DIR`, `.` by default, from
the runner that owns it, whatever the hold and the lease. Use it to move a started task off a runner that will not come back. It prints
`<key> given to runner <runner>: <runner> takes it and <owner> drops it as each reads the log`.

Each runner acts when it reads the claim. The new owner starts the task over from its request. The
old owner ends its live run and drops its copy with the reason `given to runner <runner>`. If the
old runner was only cut off, its run went on until then, so giving a started task trades that
overlap for progress. A task can be given back to a runner that had it before.

`priory give` reads this machine's state file to find the owner. It refuses with
`no runner holds <key>` for a key no runner holds or whose task ended, with
`runner <runner> holds <key> already`, and with `runner <runner> has no runner file in runners/`.
Every runner also refuses a claim for a runner without a runner file.

## Limits

- On herdr-bus, the runners share the log as one file. On Postgres (see
  [ADR 0002](../adr/0002-a-factory-chooses-herdr-bus-or-postgres-for-its-event-log.md)), it travels
  between machines, and a runner cut off from it keeps going as above.
- A runner cut off for more than 30 minutes loses its `waiting` tasks to the others, and a request
  that arrives meanwhile goes to a runner that can read it.
- A task that moves starts over at its workflow's start. Its old worktree, branch and sessions stay
  on the old runner.
- This runner's `priory asks` and desk list only its own asks.
- Each runner adds one alive message a minute to `factory.intake`.
