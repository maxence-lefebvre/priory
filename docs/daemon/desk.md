# Desk

`priory desk` prints the pause line first while admissions are paused (see Pause), then the stall banner while the queue is stalled (see Room), then the intake banner while the intake fails or stopped (see Intake banner), then what waits on a person, oldest first: since, kind, id, task, lane, what and the task's generation.

| Kind | Id | What | Since |
| ---- | -- | ---- | ----- |
| `ask` | the ask | its question, then its options; `routed to <lead>: ` first for an ask a lead answers (see [tools.md](tools.md#hands-off)) | it was asked |
| `delivery` | the outside ask | `the answer did not reach <session> after <N> tries: <error>`, until the ask is asked again (see [asks.md](asks.md)) | its last try ended |
| `blocked` | the task | its reason | the task last changed state |
| `failed` | the task | its reason, then `revive: priory revive <task>` | the task last changed state |
| `prompt` | the outbox prompt | its text, for an `uncertain` prompt | it was attempted |
| `command` | the answer's action | `Priory stopped while answer <answer> to ask <ask> ran <argv> in <dir>: check the target`, for an `uncertain` command (see [tools.md](tools.md#commands-an-answer-runs)) | it ended |
| `held` | the hold | the task's reason: why it is held, and until what | it was held |
| `memory` | the memory grant | `waits for <N> GB: <command>` | it was requested |
| `queue` | the task's place in the queue | its band, then its reason when it has one | the task last changed state |
| `sweep` | the script's run | its command, then `running`, or its status and output, for the open stall | it started |
| `report` | the report's inbox sequence number | `<source>: <line>`, for an unread report | it was posted |
| `todo` | the to-do | its text, then `, due <time>` when it has one (see To-dos). `priory desk` prints the time in UTC; the web and Herdr Desks show it in local time | it was added |
| `retired` | the task | `its worktree was removed to free disk; it resurrects from <branch> when its wait fires` | it retired |
| `worktree` | the worktree's path | why it stays: see Sweep | `-` |

The lane column is the task's lane, or `-` for a `worktree` row.

In `priory desk --json`, an `ask` item carries `routed_to`: the lead it is routed to, or null. The
web Desk and the Herdr Desk show it as "with <lead>" on the ask's row and in the next thing.

It lists only asks and prompts of active tasks, holds of held tasks and waiting memory grants. Then the `queue` rows in start order, then one `worktree` row per worktree the sweep keeps. `priory asks` prints every open ask. Both leave snoozed items out, unless you pass `--all` (see Snooze).

## The next thing

The desk ranks what waits on a person, so a client can show one next thing. `priory desk --json`
and `/api/desk` carry the ranked items as `ranked` and the first as `next`, or null when nothing
waits. `priory desk` without `--json` prints neither. The ranking has six groups, first to last:

| Group | Items | `why` |
| ----- | ----- | ----- |
| Alarms | `blocked`, `failed`, an `uncertain` `prompt` or `command`, and an unread `intake` report: a source that fails to poll | `an alarm: work stopped and waits on a person` |
| Asks that open a door | an ask whose answer lets an irreversible step run (its risk is `irreversible`), and an ask of a task that reviews a pull request, whose key ends in `:review:<login>` | ``it runs `<argv[0]>` in <target>`` for an ask that runs a command (see [tools.md](tools.md#commands-an-answer-runs)), ``it lets step `<step>` run`` for another ask that approves an irreversible step, or `it reviews a pull request` |
| Plans | another ask at a human step, such as `approve-plan` | `it approves a plan` |
| Other asks | every other ask, such as an agent's or an outside one | `it waits on a person` |
| To-dos | each open to-do | `a to-do of yours` |
| The rest | every other item: deliveries, holds, sweeps, memory waits, reports and retired tasks | `it waits on a person` |

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

## Reports

`priory post SOURCE LINE` stores one report line on the inbox for a person to read, and prints its
sequence number. The daemon routes it to no task. `SOURCE` matches `[a-z0-9-]{1,32}`. The sources
the intake and the daemon publish under are refused: `cli`, `factory`, `github`, `intake`, `linear`,
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

- When a source's poll fails, the intake posts one report from `intake`, `<source>: <error>`, as
  `priory post` stores it. It posts again only when the error's text changes.
- When that source's next poll succeeds, its report is marked read, so it leaves the desk.
- When a source the triggers need cannot start, such as Linear without `LINEAR_API_KEY`, the
  intake records the failure the same way, then exits, and systemd restarts it.
- `priory desk` prints the banner `the intake fails to poll <source> since <time>: <error>` while a
  source fails, one per failing source joined by `; `.
- When no source polled in the last 5 minutes, the banner is instead `the intake has not polled
  since <time>: waits on GitHub, Linear and Slack cannot fire`. A factory whose intake never ran
  shows no banner.

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

It replies `task <id> will hear it`. It refuses a `done`, `failed` or `dropped` task, a task at a
human or code step, and a task that has neither a run nor a queued prompt, such as a `queued` one,
since its first run builds the step's prompt then.

## Pause

`priory pause [REASON]` stops admissions: no `queued` task starts a run or its first code step
until `priory unpause`. Everything else goes on. Live runs keep working, and an answer, a wait's
event, `say`, `resume` and `revive` still wake their tasks, whose new runs start as usual. A resume
that makes a task `queued`, because no run ever took its step's prompt, leaves it waiting too. A
queued task at a model step waits with the reason `waits: priory is paused: <reason>`, which
`priory tasks` shows.

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

The item comes back by itself at its time. No loop runs for that: each listing compares the time
with the clock. It comes back early when something meaningful changes:

| Item | Comes back early when | Reason |
| ---- | --------------------- | ------ |
| `ask` | the ask gets a new revision, as when its approval is asked again | `new revision` |
| `ask` | the task changes state while the ask stays open, as when a schedule wakes it | `task state changed` |
| `blocked`, `failed`, `prompt` | the task becomes blocked or failed | `new failure` |
| `blocked`, `failed`, `prompt` | the task changes to any other state | `task state changed` |

For a `blocked`, `failed` or `prompt` item, the latest change gives the reason. For an ask, a new
revision always does, and a task state change counts only while the ask is open and not yet back.
A task's `blocked` and `failed` items share one snooze, so a blocked task that fails keeps it.

The desk and `priory asks` end each row with the item's `snoozed_until` time while it is snoozed,
and its `back_early` once a change brought it back, as `{"at":"<time>","reason":"<reason>"}`. The
snoozed rows show only with `--all`. With `--json`, the desk's `counts.snoozed` says how many it
leaves out (see the [serve reference](../reference/serve.md#snooze)).

The trace records each snooze as `factory.task.snoozed` and each `--clear` as
`factory.task.unsnoozed` (see the [trace schema](../trace-schema.md)). The end of a snooze, by
time or by a change, records nothing.

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
each step that ended, repair, snooze or label, and each message and report the inbox took. Each has
its `time`, its `task` and the task's `key`, and `what`: the record's type, step and outcome, the
message's type, subject and outcome, or a report's `<source>: <line>`. `priory desk` without
`--json` prints neither.

Reading the desk records no visit, so a client calls `priory seen` once the person has read the
events. `POST /api/seen` does the same through the daemon, for `me`, at the time its body names (see
[serve.md](../reference/serve.md#writes)). The web Desk sends it once per page load, after it draws
Since you looked, with the time its first read began, so an event that arrived during that read
shows again on the next load. It keeps the events it opened on until the page reloads.
