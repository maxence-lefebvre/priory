# Spend caps

A spend cap is the most a task, a trigger's tasks, a lane's tasks or the whole factory may spend,
in dollars, per task, per local day or per week. An unattended factory cannot notice by itself that
a task went round in circles all night, so the daemon checks the caps and stops spending before a
person looks.

## Where caps live

`factory.toml` holds the factory's caps and each lane's, and a trigger file holds its own:

```toml
# factory.toml
[caps]
task = 5.0      # dollars one task may spend
day = 50.0      # dollars the factory may spend in a local day
week = 200.0    # dollars the factory may spend in a week from Monday 00:00 local time

[caps.lanes."core/intake"]
day = 10.0      # dollars the lane's tasks may spend in a local day
```

```toml
# triggers/agent-label.toml
caps = { task = 2.0, day = 10.0 }
```

Each key is optional. A cap counts toward every task it covers: the factory's toward all, a
trigger's toward the tasks it started, and a lane's toward the tasks in the lane now. See
[`[caps]`](../reference/factory.md#caps) and [the trigger keys](../reference/triggers.md).

## What counts as spent

The daemon prices tokens with `prices.toml`, as [`priory costs`](trace.md#costs) does: the records
of ended model steps, handler calls and summarizer calls, and the tokens of the step each active task
is at now. A task's spend is all its records, across every generation. A day or week's spend is the
records the window holds, from the start of the local day or Monday 00:00 local time, plus the
current steps of the active tasks the cap covers.

A model with no price counts nothing toward a cap. Price every model your agents use, or its spend
goes unseen. `priory check` refuses a cap in a factory with no `prices.toml`. It warns for the agent
of a model step whose `model` has no price, unless that model is a Claude Code alias such as
`sonnet` on a Claude Code agent.

A Codex run counts like a Claude run. The daemon reads its tokens from Codex's session file (see
[trace.md](trace.md)) and prices them at the model that file names. Codex names the model it was
given, so a Codex agent's `model` must be in `prices.toml` exactly as the agent names it.

## Past a cap

The daemon checks the caps every second. What it does depends on the cap:

| Cap | When the daemon checks | What happens |
| --- | ---------------------- | ------------ |
| Any `task` cap | A running task between two turns, and a queued or ready task at a model step | The task is held with one ask |
| A factory or trigger `day` or `week` cap | A running task between two turns | The task is held with one ask |
| Any `day` or `week` cap | A queued or ready task about to start a run | The run waits, with no ask |

A cap is reached once the spend equals it or goes past it.

### Held with one ask

The daemon never ends a run that an [urgent wake](room.md#what-is-never-stopped) would not stop
either: a run at a step whose effect is `irreversible`, with an open keyboard grant, with a tmux
client attached, or in a shared session. Such a run carries on past the cap, and the daemon holds
its task at the first moment between two turns outside them, such as the next step.

When the task has a run, the daemon reads its transcript once more, ends the run as `capped` and
kills its tmux session. It then holds the task with the reason `held: spent $203.00 of trigger
agent-label's day cap of $150.00, until a person raises the cap or stops the task, or the day ends`
and puts one ask on the desk, with the options `raise` and `stop`. A `task` cap's reason ends at
`stops the task`, since no window ends it:

```
Task 7 spent $203.00 of trigger agent-label's day cap of $150.00. Raise the cap for this task by $150.00, or stop the task?
```

- `raise` lets this task spend one more cap from what it had spent, here up to $353.00 under
  this cap. Other tasks keep the cap of the file. A task whose run was ended goes back to `ready`,
  and its next run resumes the session with `A person raised the spend cap this task reached. Check
  the worktree, then continue.` first. A task held before it started goes back to the state it was
  held from.
- `stop` drops the task, with the reason `stopped by a person at a spend cap`.

A raise lasts for the task's life. To change the cap for every task, change the factory's files in a
pull request.

When the day or week of the cap a task was held at ends with no answer, the daemon lets the task go
by itself and answers its ask with `the cap's day ended` or `the cap's week ended`. The task goes
back to the state it was held from, with no raise. A task whose run was ended resumes its session
with `The day of the spend cap this task reached ended. Check the worktree, then continue.` first,
or `week` for a week cap. It then waits like any new run, as [New runs wait](#new-runs-wait) says.

### New runs wait

A queued or ready task that a reached `day` or `week` cap covers does not start. It waits, with no
ask, and `priory tasks` shows why: `waits: spend cap: lane core/intake spent $12.40 of its day cap
of $10.00`. It starts once the next day or week begins, or once the cap is raised in the factory's
files. A running task carries on.

A lane's caps only ever hold new runs. Its `task` cap holds the task with an ask like any task cap.

A lead's or an investigator's task waits too, though the pause, the review limit and `max_running`
let it pass. It would spend past the cap, and the task it answers could not use the answer yet:
that task's next run waits for the same cap. A consult whose deadline passes in the wait wakes its
task with no report. See [runs.md](runs.md#work-already-running).

## What a person sees

- A browser [subscribed to notifications](serve.md#notifications) gets one when a `day` or `week`
  cap is first reached, with the desk's line for it.
- `priory desk` prints one line per `day` or `week` cap reached, after the pause line:
  `spend cap reached: lane core/intake spent $12.40 of its day cap of $10.00; its new runs wait for
  the next day`. `priory desk --json` carries the lines as `caps`.
- A held task shows as a `held` row and an `ask` row, as any hold with an ask does.
- `priory costs` ends with a table of every cap, its limit and what it spent. A `task` cap shows the
  most an active task under it spent, and which task. `--json` and `GET /api/costs` carry the same
  rows as `caps`, and each task held at a cap as `held` (see
  [the serve reference](../reference/serve.md#costs)).
- The Herdr adapter's [usage line](../reference/herdr-panels.md#the-usage-line) shows today's and this
  week's spend against the factory's caps, and how many tasks are held at a cap. A click lists
  every cap and each held task.
- The web view's Desk and the Herdr Desk show each line of `caps` as a banner. The web view shows
  no spend or cap table yet.

## Known limits

- A run can overshoot a cap by one turn. Tokens arrive when a turn ends, from the session
  transcript, so the daemon learns of the spend only after it happened, and never cuts a turn short.
  A long turn can overshoot by a lot.
- Spend is approximate. A step that ran across a window's start counts in the window its record
  lands in. A step whose agent changed model is priced at its last model, as for `priory costs`.
  Prices apply when the daemon reads them, so a changed price changes what counts as spent.
- A lane cap follows each task's current lane. A task that changes lane takes its spend with it.
- `priory release` on a task held at a cap lets it go without a raise. The next check holds it
  again, with a new ask.
- A code step never waits for a cap: it spends nothing.
- A window's spend counts the current step of every active task, whenever its tokens were spent.
  A task let go when its day ends can wait again at once if the steps of held or running tasks
  alone reach the cap.
- With several runners, each daemon counts only the records of its own state file, so a `day` or
  `week` cap applies to each runner apart.
