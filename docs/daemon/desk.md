# Desk

The desk is what waits on a person: asks, blocked and failed tasks, reports and to-dos. It gathers
them in one list, oldest first, so a person sees every decision the factory needs from them without
reading each task.

`priory desk` prints the pause line first while admissions are paused (see Pause), then the autopilot line while [autopilot](waits.md#autopilot) is on, with its end and how to turn it off, then one line per spend cap reached whose new runs wait (see [spend.md](spend.md)), then one line per lane or batch whose review limit holds a new task (see [runs.md](runs.md#review-limit)), then the stall banner while the queue is stalled (see Room), then the intake banner while the intake fails or stopped (see Intake banner), then what waits on a person, oldest first: since, kind, id, task, lane, what and the task's generation.

| Kind | Id | What | Since |
| ---- | -- | ---- | ----- |
| `ask` | the ask | its question, then its options; `routed to <lead>: ` first for an ask a lead answers (see [tools.md](tools.md#hands-off)) | it was asked |
| `delivery` | the outside ask | `the answer did not reach <session> after <N> tries: <error>`, until the ask is asked again, or `priory deliver` clears or retries it (see [asks.md](asks.md)) | its last try ended |
| `blocked` | the task | its reason | the task last changed state |
| `failed` | the task | its reason, then `revive: priory revive <task>` | the task last changed state |
| `prompt` | the outbox prompt | its text, for an `uncertain` prompt | it was attempted |
| `command` | the answer's action | `Priory stopped while answer <answer> to ask <ask> ran <argv> in <dir>: check the target`, for an `uncertain` command (see [tools.md](../reference/tools.md#commands-an-answer-runs)) | it ended |
| `held` | the hold | the task's reason: why it is held, and until what. For a running task marked to be held, `holds after this turn: <reason>`, and for one that sleeps or is blocked since, `holds once it is ready: <reason>` (see [runs.md](runs.md#holding-a-running-task)) | it was held, or marked |
| `batch` | the batch | its title and counts, then which entries start next: `Invoice exports: 1 merged, 3 started, 2 planned. Next: ENG-41 and ENG-42 start when ENG-40 merges.`, each task named by its key without the source. One row per going batch, in place of the `held` rows of its entries (see Batches) | the go |
| `memory` | the memory grant | `waits for <N> GB: <command>` | it was requested |
| `memories` | `memories` | `` <n> memory candidates wait for a person: <name>, <name>; see them with `priory memories` ``. One row while any candidate waits (see Memory candidates) | the oldest candidate was written |
| `pattern` | the failure pattern's id | `` <workflow>/<step>: <n> tasks stopped for one reason: <text>; see it with `priory patterns show <id>` ``, for an open pattern of at least 2 tasks (see [patterns.md](patterns.md)) | its latest task stopped |
| `queue` | the task's place in the queue | its band, then its reason when it has one | the task last changed state |
| `sweep` | the script's run | its command, then `running`, or its status and output: for the open stall, or for a scheduled run, with no task, while it runs or after it failed | it started |
| `report` | the report's inbox sequence number | `<source>: <line>`, for an unread report | it was posted |
| `todo` | the to-do | its text, then `, due <time>` when it has one (see To-dos). `priory desk` prints the time in UTC; the web and Herdr Desks show it in local time | it was added |
| `retired` | the task | `its worktree was removed to free disk; it resurrects from <branch> when its wait fires` | it retired |
| `worktree` | the worktree's path | why it stays, then its size: see Sweep | `-` |

The lane column is the task's lane, or `-` for a `worktree` row.

In `priory desk --json`, an `ask` item carries `routed_to`: the lead it is routed to, or null. The
web Desk and the Herdr Desk show it as "with <lead>" on the ask's row and in the next thing.

It lists only asks and prompts of active tasks, holds of held tasks and of tasks marked to be held, and waiting memory grants. Then the `queue` rows in start order, then one `worktree` row per worktree the sweep keeps. `priory asks` prints every open ask. Both leave snoozed items out, unless you pass `--all` (see Snooze).

## Batches

A going batch shows as one `batch` row, until it ends. Its `task` and `key` are its planning
task's. A held entry whose every open condition is a batch edge leaves `items`: the batch row stands
for it. An entry held on a condition with an open ask, such as the ask a failed predecessor raises,
keeps its own `held` row and its Release button. A `batch` row never ranks as the next thing, since
it asks for nothing. See [batches.md](batches.md). The web Desk and the Herdr Desk show it under
Also waiting with a muted tile and the `layers` icon. The web Desk's phone list puts its `Next:`
sentence in a caption below its counts, and its key opens the batch page (see
[batches.md](batches.md#in-the-web-view)).

## Unrouted events

When events about a resource some task owned or subscribed to reached no task in the last 24
hours, `priory desk` prints one line before its rows. Events no task ever owned are not counted:

```text
3 events reached no task in the last 24 hours. See them with `priory inbox --unrouted`.
```

It is a count, not an alarm: nothing ranks for it. `priory desk --json` carries it as
`counts.unrouted`, 0 when there are none. See [routing.md](routing.md#events-that-reach-no-task).

An event a task kept while it was not running reached a task, but no agent read it yet, until the
task's next run. When such events came in the last 24 hours and are at least 5 minutes old,
`priory desk` prints one more line:

```text
1 event kept by a sleeping task reached no agent. See it with `priory inbox --unrouted`.
```

`priory desk --json` carries it as `counts.unheard`, 0 when there are none. The web Desk shows it
as a calm row under the unrouted one, and the Herdr Desk's count line ends with "2 events no agent
read." See [routing.md](routing.md#events-no-agent-read).

A request a trigger's `pre` script dropped made no task, and its issue stays in its view with none.
When a script dropped requests in the last 24 hours, and no task has their key since, `priory desk`
prints one more line:

```text
1 request a pre script dropped in the last 24 hours. See it with `priory inbox --unrouted`.
```

`priory desk --json` carries it as `counts.dropped`, 0 when there are none. The web Desk shows it
as a calm row under the unheard one, which opens the Inbox at its Dropped section, and the Herdr
Desk's count line ends with "1 request a script dropped in 24 h." See
[pre.md](../intake/pre.md#one-decision-per-event).

A comment or review from someone outside the repository is not unrouted: its outcome is
`outsider`. When some came in the last 24 hours, `priory desk` prints one more line:

```text
2 comments from outside the repository came in the last 24 hours. See them with `priory inbox --unrouted`.
```

`priory desk --json` carries it as `counts.outsider`, 0 when there are none.

## The next thing

The desk ranks what waits on a person, so a client can show one next thing. `priory desk --json`
and `/api/desk` carry the ranked items as `ranked` and the first as `next`, or null when nothing
waits. `priory desk` without `--json` prints neither. The ranking has six groups, first to last:

| Group | Items | `why` |
| ----- | ----- | ----- |
| Alarms | `blocked`, `failed`, an `uncertain` `prompt` or `command`, an unread `intake` report: a source that fails to poll, and an unread `landed wrong` report: a task's merge that is red on its base branch or was reverted | `work stopped and waits on a person` |
| Asks that open a door | an ask whose answer lets an irreversible step run (its risk is `irreversible`), and an ask of a task that reviews a pull request, whose key ends in `:review:<login>` | ``it runs `<argv[0]>` in <target>`` for an ask that runs a command (see [tools.md](../reference/tools.md#commands-an-answer-runs)), ``it lets step `<step>` run`` for another ask that approves an irreversible step, or `it reviews a pull request` |
| Plans | another ask at a human step, such as `approve-plan` | `it approves a plan` |
| Other asks | every other ask, such as an agent's or an outside one | `it waits on a person` |
| To-dos | each open to-do | `a to-do of yours` |
| The rest | every other item: deliveries, holds, sweeps, memory waits, failure patterns, reports and retired tasks | `it waits on a person` |

Each group lists its items oldest first. The queue and the worktrees that stay never rank. A snoozed
item never ranks, even with `--all`. Each ranked item is the desk item with two more fields: `why`,
and `title`, the line a notification carries (see [serve.md](serve.md#desktop-notifications)).

## To-dos

A to-do is an item a person owns on the desk, with no task or agent attached.

- `priory todo add TEXT [--band B] [--due TIME]` adds one and prints its id. `TEXT` is one line.
  `B` is one of the factory's bands. `TIME` is an RFC 3339 time, stored in UTC to the second.
- `priory todo done ID` marks it done, and it leaves the desk. It is refused for a to-do already
  done, with `to-do <id> is already done`, and for an id that names none.
- `priory todo list [--json]` prints every to-do, open ones first: id, text, band, due, created and
  done.

`add` and `done` go to the daemon over its socket, as `priory answer` does. `POST /api/todos` and
`POST /api/todos/{id}/done` are their twins (see [serve.md](../reference/serve.md#writes)). An open
to-do shows on the desk as a `todo` row and ranks after every ask, before the rest (see The next
thing). A to-do sends no notification.

The web Desk and the Herdr Desk show a to-do's due time in the viewer's local time, with a hint:
`due today 15:00`, `due tomorrow 09:00`, `due yesterday 09:00`, or `due Dec 1, 09:00 (in 11 days)` and
`due Nov 17, 09:00 (3 days ago)` further out. The JSON and the CLI keep the RFC 3339 UTC time.

## Sign-in to-dos

A to-do named `Sign in to <connector>` asks a person to sign in to an MCP server that an
unattended run could not use (see [guard.md](guard.md#connector-sign-in)). There is one open item
per connector, however many runs were denied.

It leaves the desk in two ways:

- A person runs `priory todo done ID`, as for any to-do.
- A later run calls another tool of that connector, and the guards allow the call. The run's tool
  guard hook sees the call, since the daemon gives each run the connectors with an open item in
  `PRIORY_CONNECTORS`, and tells the daemon, which marks the to-do done. A call a guard denies
  clears nothing. A daemon that does not answer holds the call for one second at most.

A run started before the item was added does not clear it. If the connector still needs a sign-in,
the next denial adds the item again.

## Memory candidates

While any memory candidate waits for a person, the desk shows one `memories` row for all of them,
so reviewing memories costs one look a day, not one per memory. In `priory desk --json` and
`/api/desk`, the item carries `candidates`, each as `priory memories --json` lists it. It ranks
with the rest, and its title is `memory candidates wait for a person`. A person accepts or drops
each with `priory memories accept ID` or `priory memories drop ID`, and the row leaves once none
waits. See [skills.md](skills.md#memories-agents-write).

In `/api/desk`, each candidate also carries `replaces` and `current_text`: whether runs get a memory
of its name now, and that memory's text (see
[the reference](../reference/memories.md#priory-memories)). `POST /api/memories/{id}/accept` and
`/drop` decide one, as the CLI does (see [serve.md](../reference/serve.md#writes)).

The web and Herdr Desks show it as a plain row with its line, without Accept or Drop buttons.

## Reports

`priory post SOURCE LINE` stores one report line on the inbox for a person to read, and prints its
sequence number. The daemon routes it to no task. `SOURCE` matches `[a-z0-9-]{1,32}`. The sources
the intake and the daemon publish under are refused: `autopilot`, `cli`, `factory`, `github`, `intake`, `linear`,
`priory`, `schedule` and `slack`, so a script cannot pass for an event. `LINE` is one line, cut at 500
characters.

`priory inbox` prints reports with the messages, with the line in its `line` column. `priory inbox
--since SEQ` prints only the entries after `SEQ`, so a caller that keeps the last sequence number it
printed can resume from it. `--json` prints the same entries as JSON.

An unread report shows on the desk as a `report` row. `priory inbox read SEQ` marks it read, and it
leaves the desk. It is refused for a sequence number that is not a report. `POST /api/inbox/{seq}/read`
is its twin. The Herdr library's `read` and the web desk's Read button run it.

A held task's row shows its reason. The web desk's Release button on it runs `POST /api/tasks/{id}/release`
with the state revision the page drew, as `priory release --expected` does. A newer state gets `409`,
and the desk redraws.

A systemd unit reports its own failure through `OnFailure=` and a template unit, such as
`priory-notice@.service`:

```ini
[Unit]
Description=Post that %i failed

[Service]
Type=oneshot
Environment=PRIORY_DB=%h/.local/state/priory-oss/priory.db
ExecStart=priory post systemd "%i failed"
```

Each watched unit adds `OnFailure=priory-notice@%n.service`, so `%i` is the failed unit's name.

## Intake banner

The intake records each poll of each source it polls, `github`, `linear` or `slack`, in the state.
A tick that polls no Linear or Slack source records only `github`.

- When a source's poll fails, the desk shows the banner `the intake retries <source> since <time>:
  <error>`. It posts no report yet, so nothing ranks as an alarm and no notification goes out. A
  single failed poll, such as a GitHub 504, clears on the next good poll.
- When the source's polls have failed for [`[intake] alarm_after`](../reference/factory.md#intake),
  15 minutes by default, the intake posts one report from `intake`, `<source>: <error>`, as
  `priory post` stores it. The banner then reads `the intake fails to poll <source> since <time>:
  <error>`. The intake posts again only when the error's text changes.
- When that source's next poll succeeds, its report is marked read, so it leaves the desk, and the
  banner clears.
- When a trigger's `pre` script fails, the process that ran it posts one report from `intake`,
  `pre of trigger <name>: <how it failed>`, and posts again only when the text changes. The
  trigger's next run that exits 0 or 1 marks it read. It shows as an alarm and adds no banner. See
  [pre.md](../intake/pre.md#when-it-fails).
- When a source the triggers need cannot start, such as Linear without `LINEAR_API_KEY`, the
  intake records the failure the same way, but posts its report at once, then exits, and systemd
  restarts it.
- `priory desk` prints one banner per failing source, joined by `; `.
- When no source polled in the last 5 minutes, the banner is instead `the intake has not polled
  since <time>: waits on GitHub, Linear and Slack cannot fire`. A factory whose intake never ran
  shows no banner.

While the daemon cannot reach its log, the banner starts with
`the log is unreachable since <time>: <error>; <n> messages wait to publish`, joined to the
intake's own by `; `. See [runners.md](runners.md#a-runner-cut-off-from-the-log).

While the last fetch of the factory's kit failed, the banner ends with `the kit fails to fetch since
<time>: <error>; runs keep commit <commit>`, or `; runs start without the kit` when the runner has
no good commit yet. See [skills.md](skills.md#a-kit-repository).

While the sweep cannot fetch a repository that still has worktrees, the banner ends with
`fetching <owner>/<name> fails since <time>: <error>; the sweep tries it again every hour. If its
remote is gone, remove <home>/worktrees/<owner>/<name> and <home>/repos/<owner>/<name>`. The
daemon reads it from Priory home, not the state, so a client sees it at its next read. See
[sweep.md](sweep.md#a-clone-that-fails-to-fetch).

For 24 hours after the sweep restored the agents slice's limits, the banner ends with `another
process set <slice> to <limits>; the sweep restored its limits at <time>`. While the daemon may not
set them, it ends with `left the limits of <slice> alone: <why>` instead. The daemon reads both from
Priory home too. See [agents-slice.md](agents-slice.md#restoring-the-limits).

The banner is `intake` in `priory desk --json` and `/api/desk`. A source that starts failing, or
recovers, moves the state's revision, so a client that waits on `/api/events` redraws the banner at
once. A repeated poll with the same outcome moves nothing. The stopped banner comes from time
passing, which no revision records, so a client sees it at its next read.

## Resume

`priory resume <task>` makes a blocked task `ready`. A task that has no run and no queued prompt,
as when its first clone failed, becomes `queued` instead, so its next run gets the step's prompt as
a new task does, and the reply is `task <id> is queued`. Otherwise it queues the prompt `A person
resumed this task after: <reason>. Check the worktree, then continue.` ahead of its other queued
prompts, so its next run resumes the step agent's session with it. If its latest run never reached
`SessionStart`, or it has none, the reason concerns the launch, not the agent's work, so the prompt is only
`A person resumed this task. Check the worktree, then continue.` A resume line that no run took yet is
replaced, not repeated. The line that ends the step is added only when no queued prompt carries it
already. `priory cancel <task>` drops an active task with the reason `cancelled from the desk`, ends its
live run as `cancelled` and kills its tmux session. With `--expected REVISION`, a cancel on a stale
read is refused, as [runs.md](runs.md#queue) says. Both go to the daemon over its socket, as
`priory answer` does, and are refused for a task in any other state. An answer to an ask of a done,
failed or dropped task is refused too. The daemon stores each refused answer with the revision it was
sent for, where it came from and why, and every revision of each ask: `priory asks --json` lists
them as `revisions` and `refused`.

## Say

`priory say TASK TEXT` sends TEXT to the agent of a task at a model step. It goes to the daemon over
its socket and queues `A person says: TEXT` through the outbox, as an answer does, then the line
that ends the step, when no queued prompt carries it already. The fixed words come first so that a
TEXT starting with `/` never runs as a slash command. It goes ahead of the task's other queued
prompts.

- A running task's agent gets TEXT once its run is idle. When the agent ends that turn with an ask
  or a wait, its run stays live for TEXT instead of going to sleep, as for an answer that beats the
  `Stop`. The ask or wait stays open.
- An asleep task becomes `ready`, and its next run resumes the agent's session with TEXT as its
  first prompt. Its open asks and waits stay open, so the agent sleeps on them again unless it
  ends them.
- A `ready` or `blocked` task's next run gets TEXT, ahead of the step's prompt when no run took the
  task yet.

- A task that waits on a step's ask, at a human step or a plan's approval, runs no agent while a
  person decides. TEXT is added to that ask as its next revision, `While this waited: A person
  says: TEXT`, which pushes, so the person who answers reads it. Past five such lines, the ask only
  counts the rest, as [routing.md](routing.md) says. TEXT is also queued for the task's
  next agent run, after that step's prompt, and the task stays asleep. It replies `task <id> waits
  at step <step>: its ask shows it, and its next agent run reads it`.

- A task whose result a gate holds on a person's ask ([score.md](score.md)) queues TEXT and stays
  asleep until the answer. It replies `task <id> waits for the answer to ask <ask>: its next run
  reads it`.

- A task in a debate ([debates.md](debates.md)) holds TEXT for the step after the debate, so no
  debater reads it. It replies `task <id> is in a debate at step <step>: the step after the debate
  reads it`.

- A `done` task whose pull request is open reopens as its next generation, with TEXT as the first
  prompt. Its step, session and branch are those of [feedback that reopens a done
  task](routing.md#feedback-reopens-a-done-task). It replies `task <id> reopened at <step>,
  generation <n>`. It is refused, with the reason, when the task owns no pull request, when that
  pull request merged or closed, when another active task holds its key or a resource it owns, or
  when no agent runs the step it would reopen at. A merge the intake has not published yet does not
  refuse it. A pick or comment on a done task's report reopens nothing.

Otherwise it replies `task <id> will hear it`. It refuses a `failed` or `dropped` task, a
task at a human or code step with no open ask, and a task that has neither a run nor a queued
prompt, such as a `queued` one, since its first run builds the step's prompt then.

## Pause

`priory pause [REASON]` stops admissions: no `queued` task starts a run or its first code step
until `priory unpause`. Everything else goes on. Live runs keep working, and an answer, a wait's
event, `say`, `resume` and `revive` still wake their tasks, whose new runs start as usual. A resume
that makes a task `queued`, because no run ever took its step's prompt, leaves it waiting too. A
queued task at a model step waits with the reason `waits: priory is paused: <reason>`, which
`priory tasks` shows. A lead's or an investigator's task still starts: it answers a task that
already ran (see [runs.md](runs.md#work-already-running)).

The pause lives in the state file, so it survives a daemon restart, and `pause` and `unpause` work
while the daemon is stopped. A second `pause` keeps the first one's start and takes the new reason.
While it lasts, `priory desk` starts with `paused since <time>: <reason>`, and `priory desk --json`
carries that line as `paused`.

## Revive

`priory revive <task> [--step <step>]` restarts a failed or dropped task as its next generation. A
task starts at generation 1. The revive goes to the daemon over its socket and, in one
transaction:

1. Withdraws every open ask and wait of the task with the reason `revived`, and marks its queued
   prompts `withdrawn`.
2. Raises the task's generation by one.
3. Enters the step it ended on, or `<step>`, as a move into it would (see Moving on): a model step
   makes the task `ready` with its prompt queued, a human step asks again, and a code step makes it
   `ready`.
4. At a model step, queues ahead of that prompt: `A person revived this task as generation <n>
   after it was <state>: <reason>. The asks and waits of its earlier generation are withdrawn. Check
   the worktree, then continue.`

It replies `task <id> is <state> at <step>, generation <n>`. It refuses a task in any other state, a
step the task's workflow lacks, and a task whose key another active task holds, naming that task. A
factory holds at most one active task per key. It also refuses a task that owns a resource, such as
a pull request, that another active task owns too, naming that task and the resource: the revived
task would take that resource's events from the other task.

The task keeps its id, key, branch, worktree and sessions. Its next run resumes the step's agent
session, like a resume. The agent keeps what it learned in the earlier generation, and the revive
prompt tells it that the asks and waits it remembers are gone. A new session would lose that context,
and the worktree already holds the earlier generation's work. When the sweep removed the worktree, the
next run adds it back from the branch (see Priory home). A revive straight into a code step, such as
`open-pr`, adds it back before the action runs.

A withdrawn ask or wait stays in the state file but counts as closed. An answer to one is refused with
`ask <id> was withdrawn when task <task> was revived`, or with `ask <id> was withdrawn when a later
run of task <task> called another tool` (see Routing). [Why an ask or wait was
withdrawn](waits.md#why-an-ask-or-wait-was-withdrawn) lists every reason. An event for a withdrawn wait
fires nothing, and the task keeps it like any event that matches no wait, for the next run to read.
`priory asks`, `priory waits` and the desk leave withdrawn ones out. A new run never takes over a
withdrawn ask or wait.

A failed task is one its repair could not restart (see Repair).

## Snooze

`priory snooze KIND ID --for DURATION`, or `--until TIME`, hides an `ask` of a task, or a `blocked`,
`failed` or `prompt` item, from `priory desk` and `priory asks` until that time. `KIND` and `ID` are the desk's
kind and id columns. `DURATION` is a span such as `2h` or `1d`. `TIME` is an RFC 3339 time such as
`2026-11-23T09:00:00+01:00`. `priory snooze KIND ID --clear` brings the item back at once.

The snooze goes to the daemon over its socket. The daemon refuses a time that is not in the
future, an item that the desk does not list, and a `--clear` for an item that is not snoozed. A
second snooze of an item replaces the first. Snoozing changes only what a person sees: the task,
its runs, asks and waits stay as they are, and the agent keeps waiting.

The item comes back by itself at its time: each listing compares the time with the clock. It comes
back early when something meaningful changes:

| Item | Comes back early when | Reason |
| ---- | --------------------- | ------ |
| `ask` | the ask gets a new revision, as when its approval is asked again | `new revision` |
| `ask` | the task changes state while the ask stays open, as when a schedule wakes it | `task state changed` |
| `blocked`, `failed`, `prompt` | the task becomes blocked or failed | `new failure` |
| `blocked`, `failed`, `prompt` | the task changes to any other state | `task state changed` |

For a `blocked`, `failed` or `prompt` item, the latest change gives the reason. For an ask, a new
revision always does, and a task state change counts only while the ask is open and not yet back.

At its next tick after a snooze ends, at its time or early, the daemon marks it ended. The mark raises
the state's revision, so a view waiting on it reads the item back (see
[serve.md](serve.md#waiting-for-a-change)), and writes one `factory.task.snooze_ended` trace record
with the reason, `time` for a snooze that reached its time. A person's `--clear` writes
`factory.task.unsnoozed` instead. The record's item kind is `task` for a `blocked` or `failed`
item, since those share one snooze.
A task's `blocked` and `failed` items share one snooze, so a blocked task that fails keeps it.

The desk and `priory asks` end each row with the item's `snoozed_until` time while it is snoozed,
and its `back_early` once a change brought it back, as `{"at":"<time>","reason":"<reason>"}`. The
snoozed rows show only with `--all`. With `--json`, the desk's `counts.snoozed` says how many it
leaves out (see the [serve reference](../reference/serve.md#snooze)).

The trace records each snooze as `factory.task.snoozed` and each `--clear` as
`factory.task.unsnoozed`, and each end by time or by a change as `factory.task.snooze_ended` (see
the [trace schema](../trace-schema.md)).

## Since you looked

`priory seen` records a person's visit to the desk, now by default. `--at TIME` records an RFC 3339
time instead, such as `2026-11-23T09:00:00+01:00`. `--person NAME` names the person, `me` by
default. A name is free text: Priory has no accounts. Each person keeps one visit, and a later
`seen` replaces it. It writes the state file, so it works while the daemon is stopped. It keeps
the time to the second, as the inbox does.

`priory desk --json` carries the visit as `last_visit` and the events from its second on as
`since_you_looked`, newest first, at most 50. An event in the same second as the visit shows
again rather than never. `--person NAME` reads that person's visit. Without a
visit, `since_you_looked` lists the latest 50 events. An event is a trace record or an inbox entry:
each step that ended, repair, snooze or label, and each message and report the inbox took. A
message routed to no task, `ignored`, is left out, so a busy repository does not fill the list
(the Desk counts those as unrouted events, see [Unrouted events](#unrouted-events)). Each has
its `time`, its `task` and the task's `key`, and `what`: the record's type, step and outcome, the
message's type, subject and outcome, or a report's `<source>: <line>`. A task request that an
outside session added ends with `, added by session <session>` (see
[tools.md](tools.md#the-outside-server)). An ask the daemon answered
at the task's `auto_answer` level reads `ask <id> answered "<option>" by auto (<confidence>):
<rationale>` (see [waits.md](waits.md#answered-by-auto)). An ask a person's own session decided
reads `ask <id> answered "<option>" by session <session>` or `ask <id> dismissed by session
<session>` (see [tools.md](tools.md#answers-from-the-session)). `priory desk` without
`--json` lists the events after its items, under a `since you looked` header with the columns
`time`, `task` and `what`, newest first. It prints no `last_visit` and, with no events, no list.

Reading the desk records no visit, so a client calls `priory seen` once the person has read the
events. `POST /api/seen` does the same through the daemon, for `me`, at the time its body names (see
[serve.md](../reference/serve.md#writes)). The web Desk sends it once per page load, after it draws
Since you looked, with the time its first read began, so an event that arrived during that read
shows again on the next load. It keeps the events it opened on until the page reloads.
