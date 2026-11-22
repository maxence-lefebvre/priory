# Several runners

A factory can span several machines, each a runner with its own file in `runners/`. Each task is
owned by one runner at a time, which places its runs. This page explains how the runners agree on
owners through the log, what happens while one is away, and how a person moves a task. The decision
and its alternatives are in
[ADR 0003](../adr/0003-runners-own-tasks-through-claims-and-leases-on-the-shared-log.md).

The drawing follows two runners, `a` and `b`: a claim, the holds, a cut, the takeover of `a`'s
waiting tasks and the return. The sections below explain each part.

![Two runners, a and b, and the log between them. Both read a task request and claim its key, and the first claim in the log wins, so a owns the task. Runner a publishes an alive message once a minute, the hold waiting, then the hold kept, and starts the first run once it reads kept back. When a is cut off, its kept tasks go on and its waiting tasks stay unstarted. Its lease ends 30 minutes after its last alive message, and b claims its waiting tasks and runs them. Back on the log, a reads it before any run and drops each task that moved to b.](../images/diagrams/runners.svg)

Each runner has its own Priory home, so Claude's folder trust does not carry between machines. On
each runner, a person trusts the worktrees folder before its first run and each clone once Priory
made it, as [Trusting the worktrees and the clone](sessions.md#trusting-the-worktrees-and-the-clone)
says. A task that moves to another runner can stop on that runner's trust dialog.

## When ownership applies

Ownership applies when the factory has two or more runner files. A daemon is the runner named by
its machine's hostname. With several runner files, a daemon on a machine without one refuses to
start:

```text
this machine, `<hostname>`, has no runner file in runners/, and the factory has several runners
```

A factory with one runner file or none works as before: its one daemon owns every task.

A [lead task](tools.md#hands-off), which answers an ask of a `hands-off` task, stays on the runner of that
ask. Its source event exists only in that runner's state file, so the runner claims no key for it,
starts it at once, and no other runner lists it.

A daemon claims the key of each active task of its own whose key it does not hold, such as a task
from before the factory got its second runner, or one that feedback reopened after another runner
claimed its key. It starts no run of that task until it reads its claim back, and the task waits
with the reason `waits for its claim to reach the log`. When that claim loses, because the log gives the key to
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

The runners publish more message types on the same output, which
[events.md](../reference/events.md#runners) lists: `priory.task.claimed`, `priory.task.held`,
`priory.runner.alive`, `priory.ask.posted`, `priory.ask.applied`, `priory.ask.answered`,
`priory.ask.dismissed`, `priory.task.acted` and `priory.task.applied`, and the memory messages
`priory.memory.written`, `priory.memory.accepted` and `priory.memory.dropped` (see
[skills.md](skills.md#memories-agents-write)). The daemon stores each claim,
hold, alive, post and applied message in its state file before it publishes it, so a crash between
the two publishes it at the next start. A message published twice adds nothing, because the log
keeps the first message of each id. `priory answer`, `priory dismiss`, `priory give`, an action on
another runner's task and their endpoints publish straight to the log.

A runner refuses an alive, hold, post, applied, answer, dismiss, task action or task applied
message that names a runner with no file in `runners/`, and a claim for one. It records the message
as `refused`, so a runner name the factory lacks owns no task and shows no ask or submission on any
desk.

No runner checks who wrote a message. Write access to the log equals a person's approval on every
runner: whoever can publish on it can answer any runner's ask, which runs an irreversible step or an
ask's command, resume, revive, cancel, release or say to any task, give away any task, or keep a stopped runner's lease alive. Give the log's
credentials only to the runners and the people who may approve.

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

The owner publishes each task's hold, with the task's generation, whenever either changes:

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

A task that feedback reopens on the runner that finished it waits the same way. The log still says
its key `ended`, so the runner publishes `kept` and starts the task only once it reads that back. A
request that another runner claims before then wins the key, and the runner drops its reopened
copy, with the reason `claimed by runner <name>`, before any run of it starts.

A `kept` task never moves on its own. A runner cut off from the others keeps its started tasks
going, so a second runner that started them over would run them twice. A task asleep on an ask
keeps its ask and sessions on its runner, so an answer given elsewhere reaches it there.

## A runner cut off from the log

A daemon that cannot reach its log keeps running. It goes on with its own tasks:

- A run that started goes on, and its turns, results, local asks and answers, and waits work as
  before. A person answers a local ask with `priory answer` on that machine.
- No run of a `waiting` task starts, and no code step's action runs while the log says `waiting`.
  The daemon queues `kept` for the task and starts it only once it reads that back from the log.
  Until then the task's reason is `waits for its claim to reach the log`, in `priory tasks`, its
  JSON and the Desk.
- Each claim, hold and alive message waits in the state file's queue, oldest first. Nothing reaches
  the other runners until the link is back. Only the newest alive message is kept to publish,
  since a lease runs from the last one.
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
owners and asks is as old as its last read. When their own publish gets no answer, they refuse with
`the log is unreachable: <error>: try again when it is back`.

A daemon that starts while its log is unreachable starts all the same, and connects on a later
try. `priory add` on a cut-off runner publishes its request to the log, so it fails until the link
is back.

## A runner back from away

A daemon reads the log before it starts any run and before it publishes what it queued. A task that
moved while it was away is dropped with the reason `moved to runner <name>: its lease ended`.
Answers given meanwhile on other runners apply in log order, as below. Then it publishes its queue
in order: its claims and holds, and one alive message, the newest. Each queued message keeps its
id, so one published twice adds nothing.

## Why no task runs twice

A task runs on two runners only if one runner takes over a task that runs on the other. A takeover
wins only when both hold:

- the owner's last alive message is 30 minutes or more before the takeover, in log order, and
- the task is still `waiting` at the takeover's place in the log.

A runner cut off from the log never starts a `waiting` task, since it cannot read its `kept` back.
So a task that runs there was `kept` on the log before the cut, and no takeover wins it, however
long the cut lasts. A task that was `waiting` stays unstarted on the cut-off runner, so the runner
that takes it over runs it first and alone. `priory give --force` is the one exception: a person
moves a task whose owner may still run it, on purpose, as below.

## Every runner's asks

Each runner posts its asks on the log, as `priory.ask.posted`, each time any part of one that a view
shows changes: asked, revised, answered, refused, dismissed or withdrawn. Every other runner keeps
the latest post of each ask, so `priory asks`, `priory desk`, `/api/asks`, `/api/desk` and the web
view list every runner's asks, mixed in the usual order. Another runner's ask has the id
`<runner>:<ask>`, its `runner`, and `last_seen`, when that runner was last alive. Asks of this
runner carry its `runner` too.

A post carries a `version`, apart from the question's `revision`. A refused answer changes the ask
without revising the question, and an answer closes it at the same revision, so only the version
orders posts. A runner keeps the post with the highest version, so a post read twice, or late,
changes nothing. The daemon stores each post in its state file before it publishes it, as it does a
claim.

A runner that is away keeps its asks listed, with its last alive time, and they still take answers,
which it applies when it reads the log again.

The web view and the Herdr Desk and Asks panels tag each of another runner's asks with its runner,
and say when that runner was last seen once it is quiet for over 3 minutes. The Asks list offers a
Runner facet. Each answer sent there shows as `Sent to <runner>` until that runner applies it, then
as the answer, or as the refusal and its reason. The ask's answer history in the web view and the
Herdr Asks panel lists every submission, with its runner, its outcome and when it was sent.

## Every runner's tasks

Every runner already stores each claim, hold and alive message it reads. So `priory tasks`,
`/api/tasks` and the web Tasks panel list each task another runner holds, from its claim: its key,
its runner, its hold, `waiting` or `kept`, and `last_seen`, when that runner was last alive. The
workflow, band, lane and title come from the task's request, and its `generation` from its last
held message.

Only the owner and hold cross. The task's state, steps, runs and terminal stay on its runner, so its
`id` is null and the row opens no page here: open that runner's own view for them. The row does not
link there either. A runner file names no view address, and each machine's `priory serve` listens on
its own loopback address by default, so Priory does not know where the owner's view is. Open the
address you set up for that machine, such as the one `tailscale serve` gives it. A task whose hold
is `ended` leaves the list. Tasks of this runner carry its `runner` and `hold` too. Every row lists
the actions sent to it from other machines under `submissions`, as below. The Herdr Tasks
panel lists them as well, and both panels offer a Runner facet that marks this runner `(here)`.

Both panels open such a row in place with the actions of [Acting on another runner's
task](#acting-on-another-runners-task) its hold allows: Resume, Cancel and Release for a `kept` task,
and Cancel for a `waiting` one, which has no run yet. The web adds Say for a `kept` task. The row's
newest submission shows under its title: sent, refused with why, or applied.

## A machine that only views

```sh
priory daemon --view-only [DIR]
```

serves every runner's asks and tasks on a machine with no runner file, such as a laptop. It needs
the factory in `DIR`, `.` by default, its log, and a state file of its own in `PRIORY_DB`. It reads
`factory.intake` as the consumer `priory.view.<hostname>`, and records each runner's claims,
holds, alive messages, posts, outcomes and memory candidates as a runner does. With `[serve]`, it serves them on the
web view and the API, as [serve.md](serve.md) describes.

A view-only machine can:

- list every runner's open asks on its desk and Asks list, and every runner's tasks with their
  hold and when their runner was last seen, from `priory asks`, `priory tasks`, `priory desk` and
  the endpoints;
- answer or dismiss an ask as `<runner>:<ask>`, from the web view, the endpoints or the command
  line, which publishes the write for that runner, as the next section says;
- resume, revive, cancel, release or say on a task as `<runner>:<key>`, from the Tasks panels, the
  endpoints or the command line, which publishes the action for that runner, as
  [Acting on another runner's task](#acting-on-another-runners-task) says;
- give a task to a runner with `priory give`;
- list the memory candidates that agents wrote, and accept or drop one with
  `priory memories accept` or `priory memories drop`, which publishes the verdict for every runner
  (see [skills.md](skills.md));
- record Since you looked with `priory seen` and `POST /api/seen`.

A view-only machine cannot:

- claim a task, start a run or apply an answer: it records each task request as `elsewhere` and
  every other source event as `ignored`;
- publish an alive, held, claimed, posted, applied or memory candidate message: no runner's
  lease, claim or memory depends on it;
- change anything else. Its socket refuses every other write, such as `priory resume`, a snooze,
  a pick or a todo, with:

```text
this machine is view-only: it runs no task and changes none here. Answer or dismiss an ask as <runner>:<ask>, or act on a task as <runner>:<key>, which goes to its runner through the log
```

Runners refuse a message that names a runner with no file in `runners/`. A view-only machine sends
only answers, dismisses and task actions, each of which names the runner that applies it, so that
runner applies them as it applies another runner's, and memory accepts and drops, which name no
runner.

`priory daemon --view-only` refuses to start on a machine that has a runner file, and in a factory
of fewer than two runners, which posts no ask:

```text
this machine, `<hostname>`, has a runner file in runners/: run `priory daemon` without --view-only
--view-only needs a factory of several runners: with one, its daemon serves every task and posts no ask
```

It takes no `--once`. Give each view-only machine its own hostname, since two that share one read
the log as one consumer and each miss what the other read.

## Answering another runner's ask

```sh
priory answer [DIR] <runner>:<ask> <option-or-text> --revision <n> [--run]
priory dismiss --revision <n> [DIR] <runner>:<ask>
```

publish the answer or the dismiss on the log of the factory in `DIR`, `.` by default, instead of
sending it to this machine's daemon. `POST /api/asks/<runner>:<ask>/answer` and `/dismiss` do the
same and reply `202`. Each write first checks this runner's copy of the ask: one that copy shows
closed, or at another revision, is refused at once, and nothing is published. Only the named runner
applies the write, through the same check as a local one: the first answer to a revision stands, and
a later one is refused and recorded. A runner that is away applies it when its daemon starts again.

Each write is a submission, named by its message's id, which the command prints and the endpoint
returns. The runner publishes `priory.ask.applied` with the submission and its outcome, `answered`,
`dismissed` or `refused` with the reason. Every runner lists each ask's submissions, pending until
that message arrives, so a person sees what became of an answer on any machine.

An answer that runs the ask's [command](../reference/tools.md#commands-an-answer-runs) runs it on the ask's runner.
Without `--run`, or `run: true`, it is refused with `Runs on <runner>`.

An ask of this machine whose name is `<runner>:<ask>` is answered here first. `priory answer` and
`priory dismiss` refuse a runner without a runner file with
`runner <runner> has no runner file in runners/`. An ask this runner has no copy of yet goes to its
runner unchecked from the command line, and gets `404` from the endpoint.

## Acting on another runner's task

```sh
priory (resume | cancel | release | revive [--step STEP]) [--generation N] [DIR] <runner>:<key>
priory say [--generation N] [DIR] <runner>:<key> <text>
```

publish the action on the log of the factory in `DIR`, `.` by default, for runner `<runner>` to
apply. `POST /api/tasks/<runner>:<key>/<act>` does the same with the key percent-encoded, and
replies `202` (see [serve.md](../reference/serve.md#writes)). Each names the task by its key and its
generation, never by its number, which only its runner knows. Without `--generation`, the command
sends the generation this runner last read for the key. The endpoint always takes it from the page.

The generation tells the task the person saw from its revived or reopened self. A cancel that waits
in the log could otherwise apply after a revive, to a task that is no longer the one the person
saw. So each check refuses an action when the key or the generation moved:

1. Here, from this runner's copy of the claims. A key no runner holds is refused with
   `no runner holds <key>`. A key another runner holds now is refused with
   `runner <owner> holds <key>, not <runner>`. A generation that is not the one this runner last
   read is refused with `task <runner>:<key> is at generation <now>, not <sent>`. The endpoint
   answers `404` for the first and `409` for the others, and nothing is published.
2. At the named runner, when it reads the action. It refuses it when another runner holds the key
   now, or when its newest task of the key is at another generation. Otherwise it applies the
   action through the same checks as the local command, such as `task <n> is running, not blocked`
   for a resume. Only this check decides.

Each action is a submission, named by its message's id, which the command prints and the endpoint
returns:

```text
cancel of task a:github:owner/repo#42 sent to a as submission <id>: it applies when a reads the log
```

The runner publishes `priory.task.applied` with the submission and its outcome, `applied` or
`refused` with the reason. Every runner lists each task's submissions, pending until that message
arrives. A cancel that applies ends the task, so its row leaves the other runners' lists once its
runner publishes the `ended` hold. A runner that is away applies the actions sent to it when its
daemon starts again, in log order.

An ended task leaves the list, so a revive names its key from the person's notes or from
`priory tasks` on its runner. This runner still holds the key's claim and generation, so the command
needs no `--generation`.

## Giving a task to another runner

```sh
priory give [DIR] <key> <runner> [--force]
```

publishes a claim of the key for `<runner>` on the log of the factory in `DIR`, `.` by default, from
the runner that owns it, whatever the hold. Use it to move a started task off a runner that will
not come back. It prints
`<key> given to runner <runner>: <runner> takes it and <owner> drops it as each reads the log`.

While the owner's lease is live, its last alive message less than 30 minutes before this
machine's clock, `priory give` refuses:

```text
runner <owner>'s lease lasts until <time>, and <owner> may still run <key>: give it then, or pass --force
```

A runner cut off from the log goes on running its started tasks, so a task given then runs on two
runners until the old one reads the claim. Wait until the lease ends, or pass `--force` when the
owner is down for good or you accept the overlap.

Each runner acts when it reads the claim. The new owner starts the task over from its request. The
old owner ends its live run and drops its copy with the reason `given to runner <runner>`. If the
old runner was only cut off, its run went on until then, so `--force` trades that overlap for
progress. A task can be given back to a runner that had it before.

`priory give` reads this machine's state file to find the owner. It refuses with
`no runner holds <key>` for a key no runner holds or whose task ended, with
`runner <runner> holds <key> already`, with `runner <runner> has no runner file in runners/`, and
with `<key> is a lead task, which stays on the runner of the ask it answers`, and with `<key> is a
consult task, which stays on the runner of the consult it answers`.
Every runner also refuses a claim for a runner without a runner file.

## Limits

- On herdr-bus, the runners share the log as one file. On Postgres (see
  [ADR 0002](../adr/0002-a-factory-chooses-herdr-bus-or-postgres-for-its-event-log.md)), it travels
  between machines, and a runner cut off from it keeps going as above.
- A runner cut off for more than 30 minutes loses its `waiting` tasks to the others, and a request
  that arrives meanwhile goes to a runner that can read it.
- A task that moves starts over at its workflow's start. Its old worktree, branch and sessions stay
  on the old runner.
- Each ask change adds one post to `factory.intake`.
- Snoozes stay on their machine: another runner's ask cannot be snoozed here.
- A machine without a runner file serves the asks and tasks only with `priory daemon --view-only`.
- Another runner's task shows its hold, not its state. Its detail page and runs show only on its
  runner, and its row links to no other machine's view.
- Actions on another runner's task are sent through the log, and hold has no remote form: hold it
  on its runner.
- A key's newer task starts at generation 1 again. An action sent to the old task and read after
  the key's new task started can apply to the new one.
- Each runner adds one alive message a minute to `factory.intake`.
