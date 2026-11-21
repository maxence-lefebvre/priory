# Several runners

A factory can span several machines, each a runner with its own file in `runners/`. Each task is
owned by one runner at a time, which places its runs. This page explains how the runners agree on
owners through the log, what happens while one is away, and how a person moves a task. The decision
and its alternatives are in
[ADR 0002](../adr/0002-runners-own-tasks-through-claims-and-leases-on-the-shared-log.md).

## When ownership applies

Ownership applies when the factory has two or more runner files. A daemon is the runner named by
its machine's hostname. With several runner files, a daemon on a machine without one refuses to
start:

```text
this machine, `<hostname>`, has no runner file in runners/, and the factory has several runners
```

A factory with one runner file or none works as before: its one daemon owns every task.

## Every runner reads the whole log

Each runner reads `factory.intake` as its own consumer, `priory.<runner>`, so every runner sees every
message in the same order. Each keeps only its own tasks in its own state file, `PRIORY_DB`. A
source event of a task that another runner owns finds no task here and is recorded as `ignored`,
while the owner routes it as [routing.md](routing.md) says.

The runners publish four more message types on the same output, which
[events.md](../reference/events.md#runners) lists: `priory.task.claimed`, `priory.task.held`,
`priory.runner.alive` and `priory.ask.answered`. The daemon queues each one in the transaction of
the change it follows, then publishes it. A message published twice after a crash adds nothing,
because the log keeps the first message of each id.

## Claims

A task request whose key no runner holds creates no task at once. Each runner that reads it
publishes a claim of the key and records the request as `claiming`. The first claim in log order
wins. The winner creates the task from the request it stored. The others record the owner, and a
later request for the key goes to it: `elsewhere` on the others' inboxes.

A key whose task ended, done, failed or dropped, is free again: a new request claims it afresh.

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

When a runner reads its own alive message, it claims each `waiting` key of another runner whose
lease ended before it. That takeover wins only if the old owner sent no alive message in between
and the key is still `waiting`. The new owner creates the task from its request. No run of it had
started, so nothing is lost.

A `kept` task never moves on its own. A runner cut off from the others keeps its started tasks
going, so a second runner that started them over would run them twice. A task asleep on an ask
keeps its ask and sessions on its runner, so an answer given elsewhere reaches it there.

## A runner back from away

A daemon reads the log before it starts any run. A task that moved while it was away is dropped
with the reason `moved to runner <name>: its lease ended`. Answers given meanwhile on other runners
apply in log order, as below.

## Answering another runner's ask

```sh
priory answer <runner>:<ask> <option-or-text> --revision <n>
```

publishes the answer on the log instead of sending it to this machine's daemon. Only that runner
applies it, through the same check as a local answer: the first answer to a revision stands, and
a later one is refused and recorded. A runner that is away applies it when its daemon starts again.
The ask's id and revision are the ones that runner's `priory asks` prints.

## Giving a task to another runner

```sh
priory give <key> <runner>
```

publishes a claim of the key for `<runner>`, from the runner that owns it, whatever the hold and the
lease. Use it to move a started task off a runner that will not come back. It prints
`<key> given to runner <runner>: it moves when <owner> reads the log`.

The new owner starts the task over from its request. When the old owner reads the claim, it ends its
live run and drops its copy with the reason `given to runner <runner>`. If the old runner was only
cut off, its run went on until then, so giving a started task trades that overlap for progress.

`priory give` reads this machine's state file to find the owner. It refuses with
`no runner holds <key>` for a key no runner holds or whose task ended, and with
`runner <runner> holds <key> already`.

## Limits

- The log is herdr-bus's SQLite file, so the runners share it as one file. A daemon that cannot
  reach its log stops with an error. Riding out a network partition needs a networked log adapter.
- A task that moves starts over at its workflow's start. Its old worktree, branch and sessions stay
  on the old runner.
- This runner's `priory asks` and desk list only its own asks.
- Each runner adds one alive message a minute to `factory.intake`.
