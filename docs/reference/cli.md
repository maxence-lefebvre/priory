# `priory` commands

Every command of the `priory` binary, as `priory` with no arguments prints them, with what each
does and the page that describes it. `DIR` is the factory folder, `.` by default. A command whose
arguments it cannot read prints this usage and exits 2.

Commands that read or change tasks need `PRIORY_DB`, the daemon's state file (see
[the daemon](../daemon/README.md#state)). `--expected N` refuses the change once the state's revision
differs from `N`. On `answer` and `dismiss`, `--revision N` and `--expected N` are the same flag
and name the ask's revision: an answer to another revision is refused.

## The factory

- `priory --version`: prints the version.
- `priory check [DIR] [--explain [--json]]`: checks the factory's files and prints each problem with
  its file and line. `--explain` lists each irreversible step with what approves it, and the
  outcomes the daemon verifies. See [workflows.md](workflows.md#irreversible-steps-and-their-gates).
- `priory draw [DIR] WORKFLOW`: prints a workflow as a Mermaid flowchart.
- `priory flows [DIR] --json`: prints every workflow as the Flows panels draw it. See
  [serve.md](serve.md#flows).
- `priory views [DIR] --json`: prints the resolved views. See [views.md](views.md).
- `priory links [DIR] [--json]`: prints the factory's link rules. See [factory.md](factory.md#links).
- `priory schedules [DIR] [--json]`: prints each schedule trigger with its next firing. See
  [schedule.md](../intake/schedule.md#next-firing).

## The processes

- `priory intake [DIR] [--once]`: polls the sources and publishes events. `--once` polls once and
  exits. See [the intake](../intake/README.md).
- `priory daemon [DIR] [--once]`: routes events to tasks and runs agents. `--once` routes what is
  pending, starts no run and exits. See [the daemon](../daemon/README.md).
- `priory daemon --view-only [DIR]`: on a machine with no runner file, mirrors every runner's asks
  and tasks, serves them and runs nothing. See
  [runners.md](../daemon/runners.md#a-machine-that-only-views).
- `priory serve [DIR] [--listen ADDR]`: runs the HTTP server on its own, without a daemon. See
  [serve.md](../daemon/serve.md#where-it-runs).

## Asking for work

- `priory add [DIR] [--workflow <name>] [--band <band>] [--lane <lane>] [--repo <owner/name>] [--intervention plan|hands-off] [--relates KIND:TASK]... <text>`:
  asks for a task from one line of text and prints its key. See [cli.md](../intake/cli.md).
- `priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--intervention plan|hands-off] [--relates KIND:TASK]... [--hold <reason> [CONDITION]...] <text>`:
  asks for a task with a key of your own, and every field as a flag. See
  [cli.md](../intake/cli.md).
- `priory batch add [DIR] ISSUE... [--repo <owner/name>] [--entries <workflow>] [--band <band>] [--lane <lane>]`:
  hands several issues to a planning task that orders them under one ask. See
  [batches.md](../daemon/batches.md).
- `priory batch show [BATCH] [--json]`: prints each batch, or one batch with its entries.
- `priory batch edit BATCH --expected REVISION (--drop ENTRY [--with-successors] | --move ENTRY (--top | --before OTHER | --after OTHER) | --moment ENTRY OTHER MOMENT)`:
  drops or moves an entry of a plan, or changes when an entry starts, before the go.
- `priory post SOURCE LINE`: stores one report line on the inbox for a person to read. See
  [desk.md](../daemon/desk.md#reports).

## The desk and its asks

- `priory desk [--all] [--json] [--person NAME]`: lists what waits on a person. See
  [desk.md](../daemon/desk.md).
- `priory seen [--person NAME] [--at TIME]`: records a person's visit to the desk, for Since you
  looked. See [desk.md](../daemon/desk.md#since-you-looked).
- `priory asks [--all] [--band <band>[,<band>...]]... [--risk <risk>[,<risk>...]]... [--json]`:
  prints the open asks. See [waits.md](../daemon/waits.md#asks-and-answers).
- `priory answer [--revision N | --expected N] [--run] [--] [DIR] [RUNNER:]ASK OPTION-OR-TEXT`:
  answers an ask. `--run` confirms an answer that runs the ask's command. `RUNNER:ASK` answers
  another runner's ask. See [waits.md](../daemon/waits.md#asks-and-answers),
  [tools.md](../daemon/tools.md#commands-an-answer-runs) and
  [runners.md](../daemon/runners.md#answering-another-runners-ask).
- `priory dismiss [--revision N | --expected N] [DIR] [RUNNER:]ASK`: closes an ask with no answer.
- `priory deliver ASK (--retry | --clear)`: sends a failed answer delivery again, or clears it from
  the desk. See [asks.md](../daemon/asks.md).
- `priory ask ID JSON`: stores an ask from a session Priory did not start. See
  [tools.md](../daemon/tools.md#asks-from-outside-priory).
- `priory snooze (ask | blocked | failed | prompt) ID (--for DURATION | --until TIME | --clear)`:
  hides a desk item until then, or brings it back. See [desk.md](../daemon/desk.md#snooze).
- `priory todo add TEXT [--band B] [--due TIME]`, `priory todo done ID`, `priory todo list [--json]`:
  adds a to-do of your own to the desk, marks it done, or lists them. See
  [desk.md](../daemon/desk.md#to-dos).
- `priory memories [DIR] [--all] [--json]`: lists the memories agents saved that wait for a person.
  See [skills.md](../daemon/skills.md#memories-agents-write).
- `priory memories (accept | drop) [DIR] CANDIDATE`: accepts or drops one.
- `priory wait --since REV [--timeout D]`: waits until the state's revision passes `REV`, then
  prints it. See [serve.md](../daemon/serve.md#waiting-for-a-change).

## Tasks

- `priory tasks [--lane <lane>] [--grade red|yellow|green] [--json]`: lists tasks in start order.
- `priory tasks --graph [TASK] [--mermaid]`: draws how tasks relate, as a tree or a Mermaid
  flowchart. See [relations.md](../daemon/relations.md).
- `priory task TASK [--json]`: prints one task with its brief, plan, pull request, runs, asks and
  the events routed to it. See [routing.md](../daemon/routing.md#the-tasks-inbox).
- `priory queue [DIR] [--json]`: prints the waiting tasks in start order, then the held ones. See
  [runs.md](../daemon/runs.md#queue).
- `priory move TASK [--band <band>] [--top | --before TASK | --after TASK] [--expected REVISION]`:
  moves a task to another band or place in the queue.
- `priory hold TASK --reason TEXT [--until TIME] [--after TASK-OR-ISSUE]... [--merge PR]... [--if-closed release|drop|ask] [--blockers] [--expected REVISION]`:
  holds a task back until a time, another task's end, a merge or its last open blocker. See
  [runs.md](../daemon/runs.md#holds).
- `priory release TASK [--expected REVISION]`: lets a held task go.
- `priory resume TASK`: resumes a blocked task. See [desk.md](../daemon/desk.md#resume).
- `priory say TASK TEXT`: sends a task's agent a message, and wakes it if it sleeps. See
  [desk.md](../daemon/desk.md#say).
- `priory cancel TASK [--expected REVISION]`: drops an active task and ends its run.
- `priory revive TASK [--step STEP]`: restarts a failed or dropped task as a new generation. See
  [desk.md](../daemon/desk.md#revive).
- `priory (resume | cancel | release | revive [--step STEP]) [--generation N] [DIR] RUNNER:KEY`,
  `priory say [--generation N] [DIR] RUNNER:KEY TEXT`: sends the action to another runner's task.
  See [runners.md](../daemon/runners.md#acting-on-another-runners-task).
- `priory give [DIR] KEY RUNNER [--force]`: moves a task to another runner, once its runner's lease
  ended or with `--force`. See
  [runners.md](../daemon/runners.md#giving-a-task-to-another-runner).
- `priory pause [REASON]`, `priory unpause`: stops new tasks from starting, then lets them start
  again. See [desk.md](../daemon/desk.md#pause).
- `priory subscribe TASK RESOURCE [--expected REVISION]`, `priory unsubscribe TASK RESOURCE`: routes
  a pull request's, Linear issue's or Slack thread's events to a task, or stops routing them. See
  [tools.md](../daemon/tools.md#subscribeon-and-unsubscribeon).
- `priory relate TASK builds-on|fixes|takes-up|relates OTHER [--remove]`: records how a task relates
  to another, or removes the relation. See [relations.md](../daemon/relations.md).
- `priory search QUERY [--state STATE[,STATE...]] [--limit N]`: finds tasks by what they are about.
  See [search.md](../daemon/search.md).
- `priory tldr TASK TEXT`: sets a task's summary by hand.
- `priory progress TASK JSON`: sets a task's progress fields, as the `progress` tool does. See
  [tools.md](../daemon/tools.md).
- `priory attach (TASK | --run RUN) [--read-only] [--print]`: attaches your terminal to a task's
  agent, or prints the command that would. See
  [sessions.md](../daemon/sessions.md#attaching-to-a-run).
- `priory screen [DIR] (TASK | --run RUN) [--history N] [--json]`: prints what a task's agent shows
  now, with secrets hidden.

## Listings of the state

Each prints one row per item, tab-separated after a header line, with `-` for an empty cell. See
[Columns](#columns).

- `priory inbox [--unrouted] [--since SEQ] [--json]`: prints the inbox, or the events that reached
  no task. See [routing.md](../daemon/routing.md#events-that-reach-no-task).
- `priory inbox read SEQ`: marks a report line read, so it leaves the desk.
- `priory runs [--json]`: prints every run.
- `priory outbox`: prints every prompt. See [outbox.md](../daemon/outbox.md).
- `priory waits`: prints each open wait. See [waits.md](../daemon/waits.md#waits).
- `priory actions`: prints each code step's and each answer's action, with its status, error and
  output. Read it when a code step failed. See [tools.md](../daemon/tools.md#code-steps).
- `priory trace`: prints every trace record, one JSON object per line. See
  [trace.md](../daemon/trace.md).

## Reports and measures

- `priory report put TASK FILE|-`, `priory report show [TASK] [--json]`, `priory report list [--json]`:
  stores a report a workflow wrote, prints the last one, or lists them. See
  [reports.md](../daemon/reports.md).
- `priory report pick PICK OPTION --expected REVISION`,
  `priory report comment ROW --expected REVISION TEXT`,
  `priory report (reply COMMENT TEXT | resolve COMMENT | reopen COMMENT)`: picks an option of a
  report, comments on a row, or answers, resolves or reopens a comment.
- `priory report [DIR] --workflow <name> [--since <instant-or-duration>] [--cost] [--by lane]`:
  prints outcome counts and rates per step, or tokens and cost. See
  [trace.md](../daemon/trace.md#reports).
- `priory report risk [DIR] [--workflow <name>] [--since <instant-or-duration>] [--by lane] [--json]`:
  prints, for each irreversible step, the share of tasks through it that were judged wrong. See
  [trace.md](../daemon/trace.md#risk).
- `priory report attention [--since <instant-or-duration>] [--by workflow|lane|kind] [--json]`:
  prints the person-minutes per merged pull request and how long asks waited. See
  [trace.md](../daemon/trace.md#attention).
- `priory costs [DIR] [--since <instant-or-duration>] [--by workflow|agent|model|machine|repo] [--json]`:
  prints the cost per day and split by one dimension. See [trace.md](../daemon/trace.md#costs).
- `priory grades [--since <instant-or-duration>] [--by lane] [--json]`: prints how each grade's
  tasks turned out. See [trace.md](../daemon/trace.md#grades).
- `priory label [DIR] TASK wrong (--evidence URL [--happened DATE] | --clear)`: marks a task's work
  wrong for the risk report, or withdraws the mark. See [label.md](label.md).
- `priory score [DIR] TASK [--scorer NAME] [--budget USD] [--json]`: grades a task's finished runs
  with the factory's scorers. See [score.md](../daemon/score.md).
- `priory score [DIR] --calibrate SCORER --budget USD [--json]`: measures a judge scorer against a
  person's labels.
- `priory bench capture [DIR] TASK --into FOLDER`, `priory bench replay [DIR] CASE --budget USD [--turn-timeout DURATION]`:
  turns a past task into a benchmark case, or plays one case. See
  [bench.md](../daemon/bench.md).
- `priory bench FILE --base REF --head REF --budget USD [--runs N] [--case NAME] [--turn-timeout DURATION] [--post OWNER/NAME#N]`:
  compares two commits of the factory on a benchmark.
- `priory eval FILE --budget USD [--case NAME] [--runs N] [--model MODEL]`: runs a prompt evaluation
  file against a real model. See [prompt-evals.md](prompt-evals.md).

## Commands a run calls

An agent's settings and its shell call these. A person rarely does.

- `priory mcp --session ID`: the MCP server of one run, on stdio. See
  [tools.md](../daemon/tools.md#the-mcp-server).
- `priory mcp --eval LOG [--describe TOOL TEXT]`: the fake MCP server of `priory eval`. See
  [prompt-evals.md](prompt-evals.md#priory-mcp---eval).
- `priory hook EVENT`: sends one of the harness's hook signals to the daemon. See
  [signals.md](../daemon/signals.md).
- `priory gate`: runs the factory's `gate` in the run's worktree and records its exit code for the
  branch's head.
  See [tools.md](../daemon/tools.md#proofs).
- `priory memory [(acquire | lease) [--gb N] [--timeout D] -- COMMAND...]`: runs a heavy command once
  the machine has the memory for it, or prints the grants. See
  [memory-gate.md](../daemon/memory-gate.md).

## Columns

`priory inbox` prints each entry, oldest first: seq, type, subject, task and outcome.

`priory tasks` prints each task in start order: id, key, workflow, step, state, band, lane, reason,
position, generation, and the retirement time of a retired task (see
[sweep.md](../daemon/sweep.md#retiring-asleep-tasks)). The position is the task's queue place, from
1, when it is `queued` or `ready`. `--lane L` prints only the tasks in lane `L`.

`priory runs` prints each run: id, task, step, tmux session, harness session (a Codex run's
thread), started, ended, how it ended, and the hash and commit of the factory's skills,
instructions and memories it got (see [skills.md](../daemon/skills.md#what-a-run-records)).
`priory runs --json` prints every run as `/api/runs?all=1` serves it, with its task, outcome, model
and tokens.

`priory outbox` prints each prompt: id, task, run, state and text, with newlines shown as `\n`.

`priory queue` prints the waiting tasks in start order, then the held ones: position, position in
the band, id, key, workflow, step, state, band, lane and reason. When a task is asleep, a blank
line and an `asleep` table follow: id, key, since when it sleeps, and what each open wait waits on
and since when, or `an ask`. `priory queue DIR` reads the factory in `DIR` too, so its up next
includes schedule firings.

`priory schedules [DIR]` prints each schedule trigger: its name, next firing, key, task and
workflow.

`priory desk`, `inbox`, `asks`, `tasks` and `queue` take `--json`, and then print on one line the
JSON that the HTTP server returns for the same listing. [serve.md](serve.md) lists its fields.
