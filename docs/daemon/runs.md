# Runs

A run is one task step worked by one agent's `claude` process. The daemon starts runs in queue
order, and only when they fit in the room, so a factory never runs more agents than its machines can
take.

A model step that leads to another
model step with the same agent carries the process on into a new run, so each run has at most one
result. A model step with another agent gets a new run and a new process (see Moving on). Each
second, the daemon starts a run for each `queued` or `ready` task whose step decides `model`, in
queue order, if it fits in the room (see Queue and Room). It works in the task's repository: its
task request's `repo`, else its trigger's, stored when the task is created. A task stored before
tasks kept a repository uses its trigger's.

Before that, each tick checks every `queued` and `ready` task's step. A step its workflow no longer
has blocks the task with `step <id> is not in workflow <name>`. A human step with no open ask gets
its ask and the task falls `asleep`, as a move into the step does, so a workflow can start at a human
step. A held task gets its ask once its hold releases it.

The daemon creates the worktree on a new branch from `origin/HEAD`, and stores that commit as the
task's `base_sha` the first time. A revived task keeps it, so a scorer and a benchmark know where
the task started. When the repository has recipes, a setup or resume runs first (see Recipes). The
daemon then records the run and task as `running`, and starts the agent's
harness in a detached tmux session named `priory-<task id>`, in the worktree. It first kills a
session of that name left by an ended run, since the task has no other live run. An agent with
`harness = "claude-code"` runs `claude` (below). One with `harness = "codex"` runs `codex` (see
Codex). Each step starts its own agent's harness, so one workflow can mix both.
`claude` gets:

- `--session-id`: a UUID Priory chooses and stores, so a later run can resume the conversation. A
  task keeps one session per agent. When an earlier run of the step's agent reached `SessionStart`,
  the run gets `--resume <that id>` instead, in the same worktree. An agent with
  `session = "shared"` keeps one session across tasks (see Shared sessions).
- `--model` and `--effort`: the agent's, when it sets them.
- `--append-system-prompt`: the agent body, the Markdown after its frontmatter, then a blank line and the factory's
  [writing guide](writing.md) when `factory.toml` sets `writing`. With `instructions.md`, the run
  gets `--append-system-prompt-file` instead, a file with the instructions before that text (see
  [skills.md](skills.md)).
- `--plugin-dir`: the factory's skills as a plugin, when it has `skills/` (see [skills.md](skills.md)).
- `--permission-mode`: the agent's `permissions.defaultMode`, or `manual` when it sets none, whatever
  the user's own settings choose. In `manual` mode, every permission request reaches Priory's deny
  hook (see Health). In `auto` mode, Claude's auto mode decides the run's permission prompts.
  `priory check` refuses `bypassPermissions` and accepts `default` as Claude's older name for
  `manual`.
- `--settings`: hooks that run `priory hook SessionStart`, `UserPromptSubmit` and `Stop`; a
  permission for the `priory` MCP server's tools plus the agent's own `allow` and `deny` rules, in
  Claude Code's syntax; a `PreToolUse` hook that denies `AskUserQuestion` and tells the agent to
  call `ask` instead; and the permission hooks under Health.
- `--strict-mcp-config`, so the run loads no MCP server of the person's own Claude config but the
  ones `--mcp-config` holds. Claude Code loads no claude.ai connector under it, so a run whose agent
  names `connectors` starts without it. That run's `--settings` deny every other server of the
  person's config, and its tool guard denies a tool of any server or connector it does not name.
  Before it starts, the daemon checks that the person's account has each connector. See
  [agents.md](../reference/agents.md#connectors).
- `--mcp-config`: a file, `$PRIORY_HOME/prompts/<session>.mcp.json`, that only its owner may read.
  It holds the `priory` MCP server, `priory mcp --session <id>` (see Tools), and the servers the
  agent's `mcpServers` names, copied from the user scope of the person's Claude config as the run
  starts. A definition can hold tokens, so it never goes on the command line, and no log or block
  reason quotes it. Each start writes the file again, so a later change to the person's config
  reaches the next run. A name the config lacks blocks the task before the run starts, with the
  reason in [agents.md](../reference/agents.md#mcpservers).

The same tmux command tags the new pane with a pane option `@priory` set to the harness session
id, so a pane is never left untagged. The daemon types only into that pane, even when a person
opens other panes in the session. Each second, it checks every live run's health (see Health). A
git or tmux error ends the run as `failed`, or blocks the task when no run exists yet.

tmux owns the agents, so the daemon can stop and start again without stopping them. On start, it
keeps every live run whose session still exists. It kills each tmux session whose tagged pane
carries the harness session of an ended run that no live run continues in that tmux session. A
crash between ending a run and killing its session leaves such sessions.

A live run whose pane is gone when the daemon starts was lost with its tmux server, by a reboot or
a killed server, so the agent is not at fault. Unless its result, ask or wait ended the turn (see
Health), the run ends `lost` and its task becomes `ready`, with the prompt `Priory restarted and
found tmux session <name> gone, as after a reboot, so this run was interrupted. Check the worktree,
then continue.` The next run resumes the session in the same worktree. This happens once. If the
task's previous run was lost too and this run stored no ask or wait, the task blocks with
`run <id> was lost again before it made progress: tmux session <name> is gone`. A prompt the lost
run had attempted stays `uncertain` and is never typed again, unless the run never reached
`SessionStart`: then it never took the prompt, as a Codex run takes its first one at launch, so the
prompt is queued again after the one that says the run was lost.

A code step's `run` action is not a run: it starts a command without tmux, an agent or a room check,
and a restarted daemon kills that command and ends its step `failed` instead of keeping it. See The
`run` action in [tools.md](tools.md#the-run-action).

## Recipes

A repository's recipes tell the daemon how to prepare a worktree, so no run has to find out how to
install and build it. `factory.toml` holds them under [`[repos."owner/name"]`](../reference/factory.md#repos):

```toml
[repos."owner/name"]
setup = { argv = ["npm", "ci"], timeout = "15m" }
resume = { argv = ["npm", "ci", "--prefer-offline"] }
teardown = { shell = "docker compose down --remove-orphans" }
```

| Recipe | When it runs |
| ------ | ------------ |
| `setup` | After the daemon adds a task's new worktree, before the first run's agent starts |
| `resume` | After the daemon adds back a worktree that a retirement or a sweep removed, before the next run's agent starts. Also after a teardown ran in a worktree that then stayed |
| `teardown` | Before the sweep removes the worktree of an ended task, and before a retirement removes the worktree of an asleep task (see [sweep.md](sweep.md)) |

A recipe runs as a code step's command does. Priory execs `argv` as it is, with no shell, or runs
`shell` with `sh -c`. It runs in the task's worktree, in its own process group, with stdin closed and
the environment of the [`run` action](tools.md#the-run-action): `PRIORY_TASK`, `PRIORY_HOME`,
`PRIORY_DB` and `PRIORY_FACTORY`. Past its `timeout`, or `command_timeout` when it sets none, it is
killed with its children. Children it leaves in the background are killed when it ends. Its status
is `exit <code>`, `killed by a signal`, `timed out after <n>s` or `cannot start: <error>`, and only
`exit 0` succeeds. The daemon keeps the last 2000 bytes of its stdout and stderr together, in the
order it wrote them.

A recipe runs in the background, so the daemon goes on with other tasks and hooks. While a setup
or resume runs, its task stays `queued` or `ready` with the reason `runs its setup` or
`runs its resume`, and no run starts for it. When it succeeds, the reason clears and the run starts
at the next tick, with the step's prompt as usual.

A failed setup or resume blocks the task, so it reaches the desk. Its reason names the status and the
last 5 lines of output, with secrets hidden as a run's screen hides them (see
[Run screen](../reference/serve.md#run-screen)):

```text
setup failed: exit 3. Its output ended with: installing | token [redacted] | npm ERR! missing script: build
```

`priory resume <task>` runs the recipe again, then starts the run once it succeeds. A cancelled
task's setup or resume is killed. The daemon reads `factory.toml` when it starts, so a fix to the
recipe itself takes a daemon restart.

A recipe has no decider or effect of its own: it works in the task's worktree, which is reversible,
and the factory's pull request is the person's approval of what it runs. So `priory check` refuses a
recipe whose command the [tool guard](guard.md) holds irreversible, such as `gh pr merge` or a
`[guard]` command of the factory.

A daemon restart never leaves a recipe running. The restarted daemon kills a recipe its predecessor
left running and ends it `interrupted by a daemon restart`. A setup or resume it ends blocks its task,
and `priory resume` runs it again.

A code step that adds back a retired worktree, such as `open-pr`, runs no resume. The task owes it,
and its next run starts it first.

Known limits:

- The room check does not count a recipe's memory or CPU: the agent's admission comes after it.
- A recipe's output stays in the state file. Only the block reason and the desk show it.
- A recipe runs on the runner that owns the task, so each runner needs the recipe's tools.

## Queue

The queue holds the `queued` and `ready` tasks at a model step. It starts their runs in band order,
then in the order a person set inside the band, then oldest first, or in turns between lanes (see
[Turns between lanes](#turns-between-lanes)). Code steps still run oldest first (see Code steps).

A lead's task, which answers a `hands-off` task's ask, and an investigator's task, which answers a
consult, start ahead of every other task of their band, even the ones a person ordered. Each
finishes the work of a task that already ran and now sleeps on it. See
[Work already running](#work-already-running).

Bands default to `blocker`, `now`, `today` and `later`, highest first. `factory.toml` can name
its own, highest first:

```toml
name = "demo"
bands = ["urgent", "normal", "someday"]
```

A new task's band is its request's `band`, else its trigger's, else its workflow's `band`, else the
last band. A request band the factory lacks, from a message published before a rename, counts as no
band. `priory check` refuses a trigger or workflow naming a band the factory lacks, with its line;
`priory request --band` refuses one too. The daemon records the bands in the state file when it
starts, so listings and `priory move` read them without the factory. A task whose band a later
`factory.toml` dropped comes after the last band. A task from before bands existed gets the last
band when the daemon starts.

`priory move <task> --band <band>` puts an active task in another band, after the tasks a person
ordered there. `priory move <task> --top`, `--before <other>` or `--after <other>` sets its place in
the person's order of its band, or of the other task's band, which it joins. `--band` combines with
`--top`, or with `--before` and `--after` when it names the other task's band. A move that sets a
place numbers every active task of the band in its current order, so tasks that arrive later go
after them, oldest first. `priory move` writes the state file directly. It needs no daemon, and the
daemon reads the new order on its next tick.

A task at the head of the queue in a band of `[queue] preempt` may mark live runs of lower bands to
stop at their next turn end, so it fits. A marked task stays `running`, with the reason `stops at
its next turn end for task 12 (blocker)`. The stopped task is `ready` again in its own band, at its
own place, and its next run resumes its session. See
[room.md](room.md#urgent-wakes).

### Aging

Strict band order lets a steady flow of `today` tasks keep a `later` task waiting forever.
`[queue] age` in `factory.toml` bounds that wait:

```toml
[queue]
age = "24h"
```

For each `age` a task has waited in the queue, it starts as if it were one band higher. Its wait
counts from when it last became `queued` or `ready`, so a run, a sleep or a hold starts it again.
It stops at the highest band that is not in `preempt`: with `preempt = ["blocker"]`, a `later`
task reaches `now` after two days and goes no higher. In the band it reaches, it starts after the
tasks a person ordered there, then with the other tasks by age. With
[`fair = "lane"`](#turns-between-lanes), it takes its lane's turn among them, counted in its own
band.

Aging changes start order only. The task keeps its own band: `priory move` and the listings name
it, and an urgent wake reads it, so an aged task never stops a run. `priory queue` shows the band a
task moved up to in its `aged` column. The daemon records `age` in the state file when it starts,
as it records the bands. Without `age`, no task moves.

One wait remains by design. An aged task never passes a task of a band above the highest band
that does not preempt, so a factory whose `blocker` tasks never end still keeps every other band
waiting.

`priory move`, `hold`, `release` and `cancel` take `--expected REVISION`, the state revision that
every listing carries. When the state's revision differs from it, the verb changes nothing and fails with `the
state is at revision <current>, not <REVISION>`, so a change made on a stale read never applies.
Without the flag, the verb applies to the state as it is.

### Turns between lanes

Inside a band, the queue starts the oldest task first, so 40 tasks in one lane hold every other
lane's work of that band behind them. `[queue] fair = "lane"` in `factory.toml` makes the queue
take turns between lanes instead:

```toml
[queue]
fair = "lane"
```

Among the tasks of a band that no person ordered, the queue starts the oldest task of each lane
before the second task of any lane. Among those, the lane whose last run started longest ago goes
first, and a lane with no run yet goes before any other. A tie goes to the oldest task. So three new
tasks in `a/x` and two in `b/y` start as `a/x`, `b/y`, `a/x`, `b/y`, `a/x`. A lane's last run is the
latest run of any of its tasks in the same band, ended or not.

A person's order still wins. The tasks a person placed with `priory move` start first, in that
order. A move that sets a place numbers every active task of the band in the order it had, the fair
one, and tasks that arrive later take turns behind them.

The daemon records `fair` in the state file when it starts, as it does the bands, so `priory tasks`
and `priory queue` list tasks in the order they start. Fairness changes only the order: a task that
waits for room still holds back every task behind it, as [room.md](room.md) says.

### At most so many running

`max_running` on a lane or a trigger holds new starts past a count:

```toml
# lanes.toml
[lane."core/intake"]
max_running = 2
```

```toml
# triggers/sentry.toml
max_running = 3
```

A run counts from its start to its end, so an asleep task counts for nothing. Once a lane's or a
trigger's tasks have that many live runs, each of its other `queued` or `ready` tasks waits with a
reason such as `waits: lane core/intake has 2 running tasks, its max_running`. The tasks behind it
start if they fit: unlike a task that waits for room, a task at its cap holds no one back. It starts
on the daemon's tick after one of those runs ends. A woken task waits too, since its run is a new
start, and so does a task a person ordered. A running task carries on.

The cap is hard: a task of the first band waits too, unlike under the [review limit](#review-limit).
A lane or a trigger caps its runs for a service that limits its rate, such as an API that takes so
many calls a minute, and an urgent task would hit that limit as any other. Only a lead's or an
investigator's task passes the cap (see [Work already running](#work-already-running)).

With several runners, each daemon counts only its own runs, so the count applies to each runner
apart. See [`lanes.toml`](../reference/lanes.md) and [the trigger keys](../reference/triggers.md).

### Review limit

A factory can open pull requests faster than a person reviews them. Each waiting pull request then
costs attention, and a branch that waits long costs a rebase later. `[queue] review_limit` in
`factory.toml` holds new starts while too many pull requests wait for a person:

```toml
[queue]
review_limit = 5
```

A pull request waits for a person while it is open and its task is not `queued`, `ready`,
`running`, `failed` or `dropped`. So the agent works on none of it: the task sleeps on its checks,
on an ask or on a wait, is held or blocked, or is done with the pull request still open. A task
that a review wakes is `ready`, so its pull request stops counting while its agent answers.

Each tick, a `queued` task at a model step, whose first run has not started, counts the pull
requests that wait for a person:

1. In its batch, when it is a batch entry (see [batches.md](batches.md)): those of the batch's
   entries.
2. In its lane: those of the tasks in the same lane now. Tasks with no lane count together.

When either count reaches the limit, the task does not start. It stays `queued` with the reason
`waits: review limit: lane core/intake has 5 pull requests waiting for a person, at the limit of 5`,
or `batch 4 has ...` for a batch. The batch counts first. The task holds back no other task: a task
of another lane or batch starts as usual. It starts on the first tick after a merge, a close or a
woken task brings the count under the limit. A batch's entries are thus spread over the days its
person takes to review, instead of all waiting at once.

A task of the factory's first band, `blocker` unless `factory.toml` names its own `bands`, starts
whatever the count. An incident fix does not wait for an unrelated review. Lower bands still wait.

A lead's or an investigator's task starts whatever the count too (see
[Work already running](#work-already-running)).

The limit holds only a task's first run, as a pause does. A `ready` task, woken by an answer, a
fired wait or a `say`, or resumed, revived or stopped for room, starts whatever the count, since it
works on a pull request that already exists. A running task carries on.

`priory desk` prints one line per lane or batch whose limit holds a task, after the spend cap lines:
`review limit reached: lane core/intake has 5 pull requests waiting for a person, at the limit of
5; its new runs wait`. `priory desk --json` carries the lines as `reviews`.

Known limits:

- The count reads the pull requests Priory knows: those a task opened or claimed. A pull request a person opened by hand does not count.
- A pull request stays open in the count until the intake reports its merge or close. A draft
  counts as any open pull request.
- A workflow whose first step is a code step starts that step whatever the count.

### Work already running

A task that consults an investigator, or asks under `intervention = "hands-off"`, sleeps and gives
back its room. The daemon queues a task for the investigator or the lead in the asking task's band
and lane, with the trigger `priory/consult` or `priory/lead`. Its answer is the rest of work
already started, so the queue treats it as such:

- It starts ahead of every other task of its band, so a backlog does not take the room the asking
  task gave back. A consult's `[consult] deadline` counts the wait (see
  [tools.md](tools.md#consulting-an-investigator)).
- `priory pause` does not hold it, as it holds no woken task.
- The review limit does not hold it. A lane at the limit is full of pull requests that wait for a
  person, and the person is not told about a routed ask.
- `max_running` does not hold it. The asking task's run, which counted, has ended.

A reached `day` or `week` spend cap still holds it, with the other new runs (see
[spend.md](spend.md#new-runs-wait)). The cap exists to stop spending before a person looks, and the
answer would buy nothing: the asking task's next run is held by the same cap.

A higher band's task, a task that waits for room ahead of it and a busy shared session still hold
it, as they hold any task.
- With several runners, each daemon counts only the tasks of its own state file.
- One limit applies to every lane and batch. The factory sets no limit per lane.

## Holds

`priory hold <task> --reason <text>` holds a `queued` or `ready` task, so it never starts until
its hold is released. A `running` task is held at the end of its turn (see
[Holding a running task](#holding-a-running-task)). It refuses any other state: an asleep task, for
example, waits on its own ask or wait, whose wake would race the hold. Hold such a task once it is
back in the queue. `priory release <task>` lets a held
task go, back to the state it was held from. Both go to the daemon over its socket, as
`priory resume` does, so a hold and a run start never race.

The daemon holds a task itself when it passes a spend cap, with an ask to raise the cap or stop
the task, and ends its run first when it has one. See [spend.md](spend.md).

A hold takes conditions, which release it without a person once all of them are met. `--after` and
`--merge` repeat. `--until` and `--blockers` come once.

| Flag | Met when |
| ---- | -------- |
| none | a person runs `priory release` |
| `--until <time>` | the time passes: an RFC 3339 instant, or a duration such as `2 hours` from the hold, as for a time wait |
| `--after <task>` | the other task is done, failed or dropped |
| `--after <issue>` | the Linear issue, such as `ENG-7`, is Done or Canceled |
| `--merge <pr>` | the pull request merges: `owner/name#42`, or `42` in the task's repository |
| `--blockers` | the task's Linear issue has no open blocker left |
| `parked`, set by `priory adopt` | a comment or review trigger requests a task for the task's pull request (see [Parked pull requests](routing.md#parked-pull-requests)) |

For example, `priory hold 12 --reason "needs both" --after 9 --after ENG-7 --merge 42` releases task
12 once task 9 ends, ENG-7 is Done or Canceled, and pull request 42 merges, in any order. A condition
met stays met. `priory release` lets the task go whatever its conditions.

`--merge` takes `--if-closed release|drop|ask` for a pull request closed without a merge, and it
applies to every `--merge`. `release` counts the condition as met. `drop` drops the task with the
reason `<pr> closed without a merge`. `ask`, the default, puts an ask on the desk with the options
`release` and `drop` while the task stays held. `priory answer` takes either: `release` meets that
condition only. A merge after the ask meets the condition and answers the ask. A close whose
`merged` is null, because GitHub no longer returns the pull request, does not say whether it merged,
so it asks whatever `--if-closed` says.

`--blockers` reads the `blockers` field of Linear events (see `docs/intake/linear.md`). It is refused
for a task whose source event is not a Linear event, such as a GitHub issue or a CLI request. The
newest event about the issue that carries `blockers` decides: an empty list releases the task, and a
null one, which Linear's paging left unknown, does not. That event is a creation or state change of
the issue, or a `linear.issue.blockers` that one of its blockers published when it changed.

`--after <issue>` reads the newest Linear event about the issue: the `to` of a
`linear.issue.state_changed`, else the `state` of another issue event. The intake polls Linear for
the issue while the condition is unmet (see `docs/intake/watches.md`). When the daemon has
`LINEAR_API_KEY`, it also reads the issue's current state through the intake's Linear transport,
off its loop, when the hold is stored and then once a minute. So an issue already Done or Canceled
meets the condition within a tick or two, and a failed read is tried again.

The daemon judges each condition when the hold is stored. Then it judges time and task conditions
on each tick, and merge, blockers and issue conditions on each message that names their pull request
or issue, whatever the message's route. A condition that already holds, such as a pull request
merged before the hold, is met at once. An issue or blockers condition met earlier is met no more
when a later message, or the daemon's read of the issue, shows it open again or a blocker back.
The hold then waits for it again, and the intake keeps watching the issue until the hold releases.
A task request with `hold` (see `docs/intake/task-requests.md`) creates its task held the same way.

A held task has the reason `held: <reason>, <condition> and <condition>`, which `priory tasks` and
the desk show. For example: `held: needs the schema, until github:owner/repo#7 merges, and asks if it
closes unmerged`. `priory tasks --json`, `priory queue --json` and serve list each condition of a held
task under `conditions`, with `met` and `met_as` once it is met.
A batch's entry is held on one `merged` condition per entry it starts after, which no flag sets (see
[batches.md](batches.md)). Its reason reads `held: batch 4, until task 31 merges`.

### Holding a running task

`priory hold` on a running task marks it and changes nothing until its turn ends. The task's reason
reads `holds after this turn: <reason>, <condition>`, and the desk lists it with its `held` rows.
The mark is stored in the state file, so it survives a daemon restart. A second hold replaces it.
`priory release` withdraws it, with the reply `task 12 is no longer to be held`.

At the turn's end, the run settles as an [urgent wake](room.md#what-happens-at-the-turn-end) does,
but ends as `held`:

- A turn that ended with no tool, or whose process would carry on, parks the task held. A result
  moves the task to its next step first.
- A turn that ended with an ask or a wait puts the task to sleep. The task is held once it is ready.
- A result that ends the task closes the hold.

The daemon judges the hold's conditions from when the task is held. A condition met by then releases
it at once. The task keeps its worktree, branch, uncommitted files and session. Once released, its
next run resumes the session, with a prompt that names the hold ahead of any other queued prompt.

`--now` does not wait for the turn's end, for an emergency such as a full disk. It ends the run at
once and kills its process. The worktree is left as it is, so a file the agent was writing may be
incomplete, and the prompt of the next run tells the agent to check. A result, ask or wait the turn
already sent still counts. While a gate grades the result, the run goes on, since ending it would drop the result: the task is
held after the verdict, and the reply says whether the turn still runs. A prompt typed but not
confirmed when the run ends stays on the desk as `uncertain`. A run that never took a prompt gives it
back to the queue, and the next run starts a new session with it. The first prompt of a resumed Codex
run stays `uncertain` once the run has read its transcript, since the session may hold that
prompt's turn. On a queued or ready task, `--now`
changes nothing.

`--now` kills the run's tmux pane. A process that left the pane's session, such as a daemonized
server, goes on, and the [memory grants](memory-gate.md) of a command that goes on stay until it
exits.

A run at an `irreversible` step, in a shared session, in a debate turn, or with a keyboard a person
holds is not stopped. A debate turn settles as usual, and the task is held before the next one. `--now` refuses it, with the reason. A hold without `--now` keeps its mark and applies at
the next turn end, or as soon as the run is idle and none of those is true. A run whose gate grades,
one with a prompt that is not confirmed, and one that took no prompt yet also keep the mark. A tmux client attached
to the run does not keep it from being held.

When a task's Linear issue leaves the watched view before the task's first run, the daemon holds
the task and asks a person whether to drop or keep it, since the view may have said "not this one":
`ENG-7 left the view My loop backlog. Drop the task?`, with the options `drop` and `keep`. The
reason reads `held: left the view My loop backlog, until a person answers whether to keep it`.
`drop` drops the task with the reason `dropped by a person after its issue left the view`. `keep`
releases that hold, back to the state it was held from. When the issue enters the view it left
again before anyone answers, the daemon answers `keep` itself and the hold's condition reads
`ENG-7 entered the view My loop backlog again`. Entering another view answers nothing. A task that already ran gets no ask: its
agent hears `linear.issue.left` as an event of its own key, which wakes it when it sleeps at a model
step (see [Subscriptions in tools.md](tools.md#subscriptions)). A batch entry gets the same ask.

Events for a held task are kept for its first run, as for a queued one, and a schedule's tick for it
is coalesced. `priory cancel` drops a held task and ends its hold.

### Overlapping files

When two active tasks change the same files, they conflict at merge. So the daemon holds the later
task, the one with the higher id, until the earlier one merges.

At the end of each run, the daemon records the files the task's branch changes against the
repository's default branch (`git diff origin/HEAD...<branch>`). On each tick, it looks at each
`queued` or `ready` task. If an earlier task that is not failed or dropped changed any of the same
files, the task gets an ordinary hold with the reason `overlaps task N on <files>`. The hold waits
for the earlier task's open pull request to merge, or, when it has none yet, for that task to end.
A pull request closed without a merge releases it. `priory release` lets the task go as for any
hold, and the daemon does not hold it for that earlier task again.

The check keeps new work from starting beside open work. A task that already has work on an open
pull request, such as a review fix or a CI fix, is never held by it, whichever earlier task it
overlaps. A task has work on an open pull request of its repository in three cases:

- The pull request is recorded for the task.
- The task's key names it, with or without the `cli:` prefix.
- The task's branch and the pull request's pushed branch are in one line: either one holds every
  commit of the other. The task pushed to it, or is built on it.

The daemon checks this only when it would place a hold. It never releases a hold already placed: a
person does, with `priory release`.

A stacked batch entry never waits on a task it is stacked on, since its branch starts with that
task's files (see [batches.md](batches.md)). The pull requests it is stacked on do not count as its
own work, so it can still wait on another task.

Set `[queue] overlap = false` in `factory.toml` to turn this off (see
[factory.md](../reference/factory.md#queue)).

Known limits:

- Only a task between runs is held for an overlap. A running task is not marked.
  A task that overlaps while running is held at its next queued step, if it has one.
- The branches are compared as the clone last pushed or fetched them. When an amend or a rebase made
  the two branches diverge, the task is held.
- New work built on an open pull request's branch counts as work on it, unless it is a stacked batch
  entry.
- The daemon keeps a match until the task's next run, the pull request's close or its own restart,
  even when the pull request's branch is rewritten.
- When git cannot compare the branches, the daemon places no hold and asks again on the next tick.
- The files come from the end of a run. Later commits in a running task show at its next run's end.


- A pull request closed, reopened and merged later is judged on its newest close only. With
  `drop` or `ask`, the first close already acted.
- `--after <issue>` knows Linear's default state names, Done and Canceled. A team that renamed them
  needs a person to release the task.
