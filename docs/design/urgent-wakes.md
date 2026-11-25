# Urgent wakes

This page explains how a task in a high band could get room at once, by stopping a run of a lower
band at the end of its turn. Today a higher band only starts earlier: a `blocker` task that finds
the machine full waits for a run to end, however long that takes. It builds nothing. It holds the
model, the first code slice with its tests, the slices after it, and the open questions with a
recommendation for each.

The first slice is built, with every open question's recommendation taken:
[room.md](../daemon/room.md#urgent-wakes) describes it. It differs from this note in three ways.
The head stores the runs it may not stop in `tasks.protected` and stamps each mark's time in
`runs.marked`, so `priory queue --json` and the trace record can read them. A stopped run's
resumed session compacts after its first turn, as any session that resumes past `compactAt` does,
since a new run's first prompt is never a `/compact`. And a reason names a run that is too small to
help as `the runs it may stop free too little`.

Later slice 1, `[queue] age`, is built: [runs.md](../daemon/runs.md#aging) describes it. The daemon
records `age` and the highest band a task can reach in the state file's `aging` table, so
`priory queue` reads the aged band without the factory. A task's wait counts from when it last
became `queued` or `ready`. In the band it reaches, an aged task starts after the tasks a person
ordered there. The other later slices are not built.

## What existed when this note was written

- The queue starts tasks in band order, then a person's order, then age: `START_ORDER`
  (`crates/priory-core/src/state.rs:1608`, [runs.md](../daemon/runs.md#queue)).
- Priory chooses a task's band when it creates the task: its request's, else its trigger's, else
  its workflow's, else the last band (`crates/priory-core/src/route.rs:158`). Every trigger takes
  `band` ([triggers.md](../reference/triggers.md#keys-every-trigger-takes)). After that, only a
  person changes it, with `priory move`.
- `admissible` walks the queue and admits the first task that fits
  (`crates/priory-core/src/state.rs:2777`). The first task that waits for room becomes the head,
  and every task behind it waits with `waits behind task <id>, which waits for room`
  (`state.rs:2808` to `2872`, [room.md](../daemon/room.md)).
- The room subtracts what live runs promise, their agents' `memory` and `disk`, from the measured
  free room (`promised`, `state.rs:2967`, and `room::wait`, `crates/priory-core/src/room.rs:98`). A
  head that waits only for those promises is backpressure, not a stall, so no banner shows
  (`state.rs:2861`).
- A run ends by itself at the end of a turn in three cases, all in `settle`
  (`crates/priory-core/src/tools.rs:2268`): its `result` leads to a step of another agent, to a
  human or code step, to `done` or to `stop`; or it called `ask` or `wait`. The daemon then kills
  its tmux session, and the task holds no process. An answer or a `say` queued before the `Stop`
  keeps the run instead, as its next prompt (`tools.rs:2360`).
- Two turn ends keep the process. A `result` that leads to a model step of the same agent carries
  the process into a new run (`tools.rs:2864`). A turn that ends with no tool gets one nudge
  (`tools.rs:2229`). Both are decided in the `Stop` signal (`state.rs:3542`, `tools.rs:2210`).
- A stopped session resumes. Priory stores each run's harness session id, and the next run of the
  same agent on the task gets `--resume <id>`, or `codex resume <thread>`, in the same worktree
  ([runs.md](../daemon/runs.md), [codex.md](../daemon/codex.md)). A run lost with its tmux server
  already resumes this way, once, with a prompt that says what happened (`lost`, `tools.rs:3917`).
- An irreversible step runs under an approval bound to one commit, unless the factory marks it
  `autonomous` with a reason. Either way it never shares a run with the step before it
  (`tools.rs:2863`).
- A person can hold a run's keyboard for 15 minutes: an open row of `keyboard_grants`
  (`crates/priory-core/src/keyboard.rs:134`). The outbox types nothing into that run meanwhile
  ([serve.md](../daemon/serve.md#a-runs-keyboard)).
- `priory attach` builds the tmux attach command (`crates/priory-runtime/src/runtime.rs:1527`).
  Priory never reads tmux clients, so it does not know who watches a run
  ([sessions.md](../daemon/sessions.md#attaching-to-a-run)).
- A pause, a hold and a cancel exist. None of them stops a run and lets it resume later:
  `priory pause` only stops admissions (`state.rs:2888`), `priory hold` refuses a running task, and
  `priory cancel` drops the task.

So Priory has every part of a resume, and no rule that ends a run for another task's sake.

## The model

### What an urgent wake is

`factory.toml` names the bands that may stop runs. None may by default:

```toml
[queue]
preempt = ["blocker"]          # bands whose waiting tasks may stop a run of a lower band
```

A band decides when a task starts. `preempt` decides, apart from that, which bands are worth
another task's lost time. A factory can put a week's focus in `now` so its tasks start first, and
keep `preempt` for `blocker` alone.

An urgent wake happens when all of these hold on one runner:

1. The head of the queue waits for room, and its band is in `preempt`. The head may be a new task
   or a task an event just woke: both are in the queue.
2. It lacks memory or declared disk, and it would fit if some live runs' promises were gone. A
   head that waits for load, for a shared agent, for a pause or for a measurement that failed
   stops no run.
3. Live runs of a lower band, which the next section does not protect, promise enough.

The daemon then picks runs in reverse start order: the lowest band first, then the last in a
person's order, then the youngest task. It adds runs until the head fits by the room's own
arithmetic, `room::wait` with those promises left out. If every run it may pick is still not
enough, it picks none: stopping work that frees too little helps nobody.

Each picked run gets a mark that names the head task. The mark changes nothing until the run's turn
ends. The daemon withdraws the marks when the head starts, is held, is cancelled or is no longer
the head.

The room check stays as it is. An urgent wake never admits a run into room the machine lacks. It
changes what the room holds, then the head goes through the same check as any task.

### What happens at the turn end

The `Stop` signal settles the turn, and the settlement reads the mark before it decides whether
the process goes on. The result, the next step, the run's end and the prompts' order commit in one
transaction, so a marked run never gets a live successor:

| The turn ended with | Without a mark | With a mark |
| ------------------- | -------------- | ----------- |
| `ask` or `wait`, with no answer or `say` queued | The run ends and the task sleeps | The same. The mark had nothing to do |
| `ask` or `wait`, with an answer or a `say` already queued | The run goes on, with that prompt next | The run ends as `preempted`. The task is `ready`, and the answer or `say` stays queued behind the stop prompt |
| A `result` that leads to another agent's step, a human step, a code step, `done` or `stop` | The run ends | The same |
| A `result` that leads to a model step of the same agent | The process carries on into the next run | The run ends as `preempted`. The task is `ready` at the next step, with that step's prompt queued, as when the next step has another agent |
| No tool | One nudge | The run ends as `preempted`. The task is `ready`, with the prompt below queued |

A marked run that is already idle between turns, with no prompt attempted, ends on the tick that
marks it. Its queued prompts stay queued for the task's next run.

Two cases wait instead of stopping:

- A run with a prompt the outbox attempted and has not confirmed keeps its mark and goes on. The
  daemon cannot tell whether the agent took that prompt, and a stop there would make it
  `uncertain`. The mark applies at the next turn end.
- A compaction due at this `Stop` gives way: the run ends, and the resumed session compacts at its
  `SessionStart`, as a session that resumes past `compactAt` does today. A compaction already
  typed finishes first, and the mark applies at its turn end.

The daemon then kills the tmux session, as it does for a run that sleeps. Background processes the
agent started die with it, as they do on a `wait`.

The stopped task keeps its worktree, its branch, its uncommitted files and its session. Nothing is
copied or saved at that moment: the harness wrote the conversation as it went, and Priory stored
the session id when the run started. The next run resumes that session with this prompt first:

```text
Priory stopped your last run at the end of your turn to make room for task 12, in band blocker.
Your session and worktree are as you left them. Continue.
```

A turn end is the only stop point in the first slice. At a turn end no tool call is half done and
the conversation on disk is whole. A turn can run for a long time, up to the step's `timeout`, 3
hours by default, so the head may still wait. Open question 1 covers stopping inside a turn.

Only the declared `disk` of a stopped run comes back. Its worktree stays, since the task resumes in
it.

### What is never stopped

The daemon never marks a run, and withdraws a mark it finds at the turn end, when:

| The run | Why |
| ------- | --- |
| Is at a step whose effect is `irreversible` | Its approval covers one commit, and a stop between two of its calls could leave a push without its pull request. The step is short: let it end |
| Has an open keyboard grant | A person is typing in it. The grant ends by itself within 15 minutes |
| Has a tmux client attached that can type | A person may type. Read-only clients do not count: Herdr opens one by itself on a flagged step, so counting them would protect every such run |
| Is a shared session | It serves several tasks, and a lead's stop would hold the asks routed to it |
| Belongs to the sweeper | It frees the room the queue waits for |
| Belongs to a task in the head's band or a higher one | A band never stops itself. A person protects a run by moving its task up with `priory move --band` |
| Belongs to a task already stopped since its last `result`, `ask` or `wait` | Fairness, below |

The keyboard rule is exact: the stop reads the grant in its own transaction. The attach rule is
best effort, and needs something new. The daemon reads `tmux list-clients` for the runs it may
pick, on the tick that marks and again at the turn end, just before the kill. It reads how many
clients a session has and nothing else, and a read that fails counts as a client. A person who
attaches between that read and the kill, a few milliseconds later, still loses the pane: tmux
gives no way to check and kill in one step.

A code step's `run` action is not a run, holds no promise and is never stopped.

### Fairness

A stop costs the stopped task time. Four rules bound that cost:

1. **Once per step of progress.** A task that was stopped is not stopped again until its next
   `result`, `ask` or `wait`. So every resume gets one whole turn, and a task loses at most the
   restart of one session for each step of progress it makes.
2. **The stopped task keeps its place.** It is `ready` in its own band at its own position, so it
   starts before the younger tasks of its band, as soon as the head has started and room remains.
3. **No chains.** Only the head of the queue marks runs, and a task woken by a stop marks nothing
   unless it becomes the head itself in a `preempt` band.
4. **Waiting counts.** Built in later slice 1. `[queue] age = "24h"` moves a task up by one band,
   for start order only, for each `age` it has waited in the queue. It stops at the highest band
   that is not in `preempt`, and it never gains the right to stop a run.

Rule 4 answers a starvation that exists today without any stop: strict band order lets a steady
flow of `today` tasks keep a `later` task waiting forever. With it, every task reaches the front of
the bands that do not preempt.

One starvation remains by design. A factory whose `blocker` tasks never end keeps every other band
waiting, because that is what its owner asked for. The desk says so, below, and the answer is fewer
triggers in `blocker`, not a smarter queue.

### Which band a source gets

A trigger chooses its tasks' band, so the choice per source is one line of a trigger file. Three
of the four cases below need no new code:

| Source | How it gets its band | State |
| ------ | -------------------- | ----- |
| A Linear project as the week's focus | A `linear` trigger with `filter = { project = "Mobile" }` and `band = "now"`, or a `linear.view` trigger on a saved view of the project | Exists |
| A CloudWatch alarm | A `cloudwatch` trigger with `filter = { state = "ALARM" }` and `band = "blocker"` | Exists |
| A review someone asks of the person | A `pull_request.review_requested` trigger with its `band` | Exists |
| A review or comment on the pull request of a sleeping task | No trigger starts this: the event wakes a task that already has a band. A workflow's `wake_band` gives the woken task a higher band until it next sleeps | Later slice 2 |

Changing the week's focus is a one-line pull request on the trigger. Tasks created before it keep
their band, and a person moves them with `priory move --band`.

With `preempt = ["blocker"]`, the alarm's task stops a `later` run. The focus project's tasks only
start first.

`wake_band` exists because a person waits on a review. Finishing work that already has a pull
request and a reviewer is worth more than starting new work in the same band, and today the woken
task queues behind every younger task of a higher band. `wake_band = "now"` on a workflow makes each
of its tasks take the higher of its own band and `now` when a fired wait, an answer or a `say`
wakes it. The task returns to its own band at its next `ask`, `wait` or end.

### What the desk shows

An urgent wake is backpressure with a plan, so it opens no stall and no banner. It shows on the
rows that exist:

- **The `queue` row of the head**, and its reason in `priory tasks` and `priory queue`:

  ```text
  waits for room: 3.1 GB of memory free, needs 6.0 GB and keeps 6.4 GB free; task 7 (later) stops at its next turn end
  ```

  When no run can be stopped, the reason names each protected run, so a person can act:

  ```text
  waits for room: 3.1 GB of memory free, needs 6.0 GB and keeps 6.4 GB free; stops no run: task 3 is at irreversible step open-pr, task 5's keyboard is held, task 9 was already stopped once
  ```

- **The marked task**, still `running`, in `priory tasks`: `stops at its next turn end for task 12
  (blocker)`.
- **The stopped task**, `ready`: `stopped for task 12 (blocker); resumes when there is room`.
  `priory runs` shows its run as ended `preempted`.
- **A trace record**, `factory.run.preempted`, with the run, its task and band, the head task and
  band, and how long the head waited between the mark and the stop. `priory report` can then count
  stops per workflow, and the wait says whether a turn end is soon enough.
- **`--json`**: the head's queue item carries `stops`, the marked tasks, and `protected`, each
  protected task with its reason.

The web and Herdr Desks print the same reasons with no change. A badge or a count on the Desk is
later slice 4, and starts from a design a person reviewed.

## The first slice: stop a lower run at its turn end

Built.

1. `[queue] preempt`, a list of bands. `priory check` refuses a band the factory lacks and the last
   band, which has no band below it, each with its line.
2. `admissible` returns, beside the head's reason, the runs to mark. It applies the rules of
   [What an urgent wake is](#what-an-urgent-wake-is) and
   [What is never stopped](#what-is-never-stopped). The core stays pure: the daemon measures the
   room and reads the tmux clients, and passes both in.
3. Schema: `runs.preempted_for`, the head task or null, and `tasks.preempted`, set at a stop and
   cleared at the task's next `result`, `ask` or `wait`.
4. `settle` reads the mark before `enter` chooses whether the process carries on
   (`tools.rs:2321`): it refuses the carry into the next step, refuses to keep the run for a
   queued answer, and replaces the nudge, as the table above says. The run ends as `preempted`
   through the `end` that `lost` uses (`tools.rs:2667`), and the resume prompt goes ahead of the
   task's queued prompts, all in the transaction that stores the result.
5. The reasons above, `stops` and `protected` in the JSON, and the `factory.run.preempted` record.
6. Docs: a section in [room.md](../daemon/room.md), the mark in [runs.md](../daemon/runs.md#queue),
   `preempt` in [factory.md](../reference/factory.md#queue), the record in
   [trace-schema.md](../trace-schema.md), and one sentence in R4.4 of the PRD, which says today
   that a higher band only starts earlier.

Tests, through the CLI with the fake `claude` and `PRIORY_PROC`, as `crates/priory/tests/daemon.rs`
steers the room:

- With room for one run, a `later` task runs and a `blocker` task arrives. The `later` run's turn
  ends with no tool: it ends `preempted`, the `blocker` task starts, and the `later` task is
  `ready` with the stop prompt.
- The `later` task's next run gets `--resume` with the first run's session id, in the same
  worktree, and the stop prompt is typed before any other queued prompt.
- A marked run whose `result` leads to a model step of the same agent ends, and the task is `ready`
  at the next step with that step's prompt. Without a mark, the same result carries the process on.
  No run of the task is live between the two.
- A marked run that calls `wait` sleeps as usual, and its run does not end `preempted`.
- A marked run that calls `ask`, with the answer queued before its `Stop`, ends `preempted`. Its
  next run gets the stop prompt, then the answer.
- A marked run with an attempted, unconfirmed prompt goes on, and stops at its next turn end. A
  daemon restart there leaves no prompt `uncertain` that was not already.
- A marked run past `compactAt` ends without a compaction, and its next run compacts first.
- A client that attaches after the mark withdraws it at the turn end. A `tmux list-clients` that
  fails protects the run.
- Without `preempt`, or with the head in a band outside it, no run is marked and the reasons read
  as today.
- A run at an irreversible step, a run with an open keyboard grant, a run with a tmux client
  attached, a shared session's run and a run of the head's own band are never marked. The head's
  reason names each one.
- A keyboard grant taken after the mark withdraws it at the turn end, and the run gets its nudge.
- With two `later` runs and a head that needs one promise, only the run later in start order is
  marked. With a head that needs both, both are. With a head that both would not fit, neither is.
- A task stopped once is not marked again before its next `result`, `ask` or `wait`, and is after
  it.
- A head that waits for load, or for a shared agent, marks nothing.
- The head is cancelled before the turn ends: the mark is gone and the run gets its nudge.
- A daemon restart between the mark and the turn end keeps the mark.
- `priory check` refuses `preempt = ["urgent"]` in a factory without that band, and
  `preempt = ["later"]`, with the line.

## Later slices

1. `[queue] age`, as fairness rule 4 says, with the aged band in `priory queue`. Built.
2. `wake_band` on a workflow, as [Which band a source gets](#which-band-a-source-gets) says.
3. A stop inside a turn, after the head has waited a set time. See open question 1.
4. A count of stops on the web and Herdr Desks, from a design a person reviewed.
5. A hand-over in a factory of several runners: give the head to a runner with room before
   stopping a run here. See open question 6.

## Open questions

1. **Should Priory stop a run inside a turn?** A turn can last hours, and an alarm should not wait
   that long. Sending the harness its interrupt key would end the turn early, and the keyboard
   endpoint already sends keys to a pane. But a command may be half done, and Claude Code's hook
   documentation says `Stop` does not run after a user interrupt, so the daemon would have to end
   the run by itself. Nobody has checked that by hand. Recommendation: not in the first slice. Ship
   the turn end, read the wait in `factory.run.preempted` for a month, and build slice 3 with
   `[queue] preempt_after = "10m"` only if heads often wait longer.
2. **Which bands preempt by default?** `blocker` is the natural default. But a default would stop
   runs in factories whose owners never asked for it, on the day they upgrade. Recommendation:
   none. A factory opts in with one line, and the starters that use `blocker` for alarms set it.
3. **Which run is stopped: the lowest band, or the one that frees the most?** The run with the
   largest promise would stop fewer runs. Reverse start order is the rule a person already knows
   from the queue, and it stops the work the factory ranked last. Recommendation: reverse start
   order, as above.
4. **Does a read-only watcher protect a run?** A person who only watches loses nothing they typed.
   Recommendation: yes, protect it. The watcher's pane dies with the session, which reads as a
   failure, and the cost is one run that waits for the person to detach.
5. **Does a person need `priory preempt TASK` and a way to protect a task?** Recommendation: no
   new verb. `priory move --band` into the head's band protects a task, and `priory cancel` or a
   `say` that asks the agent to wait already ends a run by hand.
6. **Should the head move to another runner instead?** In a factory of several runners, another
   machine may have room, and `priory give` moves a task. A move costs a clone and loses the local
   session of a woken task. Recommendation: later, for new tasks only, once a factory of several
   runners has used urgent wakes for a while.
7. **Which band wins when two triggers match one issue?** A focus trigger and a general trigger
   can both match, and each publishes a request. The first one the daemon reads creates the task
   with its band. This note did not trace what happens to the second. Recommendation: a later
   request for an active key that names a higher band raises the task's band, and never lowers
   it. Check the routing first, and write the rule in [routing.md](../daemon/routing.md).
8. **Should a load shortage stop a run?** Load is not declared per agent, so the daemon cannot tell
   which run to stop or whether stopping it helps. Recommendation: no, until runs report their own
   use.
