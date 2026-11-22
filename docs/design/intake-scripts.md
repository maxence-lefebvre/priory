# Scripts before and after a task

This page explains how a trigger could name two scripts of the factory. `pre` runs before an
event becomes a task request, and can drop the request or change it. `post` runs when the task
ends, to notify, move an issue or label a pull request, with no change to the workflow. It builds
nothing. It holds the model, two code slices with their tests, the slices after them, and the
open questions with a recommendation for each.

Both scripts are code a person wrote and merged into the factory. That is Principle 2 of the
[PRD](../prd.md#principles): code decides what code can decide. A filter on a Sentry level needs
no model.

## What existed when this note was written

- A trigger matches events with a fixed `filter` per source: a label for GitHub, a team, project,
  label and state for Linear, a project and level for Sentry
  ([triggers.md](../reference/triggers.md)). A condition the filter lacks needs a code change.
- For each event a trigger matches, the intake publishes one `factory.task.requested` message
  (`crates/priory-intake/src/intake.rs:75`, `:764`). Its id is a UUIDv5 of the trigger's name and
  the source event's id (`intake.rs:765`), so a retried publish keeps its id and the log drops
  the copy.
- That message carries the workflow, key, band, lane, trigger and source event. It carries no
  title and no text: a task reads those from its source event
  ([events.md](../reference/events.md#factorytaskrequested)).
- The intake is its own process. It builds the request for the sources it polls and for
  schedules. For a webhook source such as Sentry, the server takes the delivery, checks its
  signature, maps it and publishes it ([webhooks.md](../intake/webhooks.md)). Both publish on the
  log, and every runner's daemon reads the log
  ([runners.md](../daemon/runners.md#every-runner-reads-the-whole-log)).
- The intake records each poll in the state file, posts one report when a source's poll fails,
  and marks it read when the next poll succeeds. That is the desk's intake banner
  ([desk.md](../daemon/desk.md#intake-banner)).
- The factory already names scripts in three places. `[asks] deliver` is a path in the factory
  folder, run with each value as one argument and never through a shell
  ([asks.md](../daemon/asks.md)). `[sweep] scripts` run a shell line with a timeout
  ([factory.md](../reference/factory.md#sweep)). A `run` action runs a step's shell line in the
  task's worktree, or in the factory folder for a task with none, with `PRIORY_TASK` in its
  environment, and its last line can be the step's
  outcome ([tools.md](../daemon/tools.md#the-run-action)).
- A workflow's last step can be such a `run` action. It runs only on a path that reaches it, so
  not for a task that failed, was dropped or was cancelled.
- A task with a pull request and a result that reaches `done` first waits on its checks gate,
  which comes after the step that led to `done`. A step with `after = "merged"` runs after the
  gate ([waits.md](../daemon/waits.md#done-and-checks)).

## The model

### The two keys

```toml
source = "sentry"
on = "issue.created"
repo = "owner/name"
filter = { project = "api" }
workflow = "bug-investigation"
key = "issue.id"

[pre]
argv = ["scripts/sentry-pre", "--min-level", "error"]
timeout = "10s"
env = ["AMPLITUDE_READ_TOKEN"]

[post]
argv = ["scripts/notify-slack", "#api-alerts"]
timeout = "30s"
autonomous = "Posts the task's outcome to #api-alerts and labels its pull request."
```

### Rules both scripts share

1. **`argv` comes from the factory only.** It is an array in the trigger file. Priory execs it as
   it is, never through a shell. `argv[0]` is a path inside the factory folder, and `priory
   check` refuses one that leaves it or is not an executable file, as it does for `deliver`.
2. **Event and task text arrive on stdin only.** No field of an event, a request or a task is
   ever put in an argument, an environment variable or a shell line. An issue title that holds
   `$(...)` reaches the script as bytes of a JSON string.
3. **The environment is a short list**: `PATH`, `LANG`, `HOME`, the Priory variables named
   below, and the names in `env`, copied from the process that runs the script. A script that
   needs a token to read another system names it there. Once
   [agent-credentials.md](agent-credentials.md) is built, `env` takes declared secrets.
4. **A timeout and an output cap.** `timeout` defaults to 10 seconds for `pre` and 30 for `post`,
   and `priory check` refuses more than 60 seconds for `pre` and 10 minutes for `post`. It counts
   from the start, the writing of stdin included, so a script that never reads its input is
   killed like any other. Past it, the script and its process group are killed. Priory reads
   both streams as they come and stores at most 64 KiB of stdout and 64 KiB of stderr. A script
   that prints more on either is killed and counts as failed. Priory keeps the last 2000 bytes
   of stderr.
5. **The working folder is the factory folder**, with stdin closed after the JSON.
6. **What a script prints is still untrusted text.** It was computed from an event an outsider
   wrote. It reaches an agent's prompt as the event's own text does, and no part of it is ever a
   command.

### `pre`: before the request

The intake runs `pre` once it knows an event matches the trigger, before it publishes the task
request. Stdin is one JSON object:

```json
{
  "event": { "id": "...", "type": "sentry.issue.created", "subject": "...", "time": "...", "data": {} },
  "request": { "workflow": "bug-investigation", "key": "sentry:4812", "band": "today", "lane": null,
               "trigger": "sentry-api" }
}
```

What the script does decides the request:

| The script | The request |
| ---------- | ----------- |
| exits 0 and prints nothing | is published unchanged |
| exits 0 and prints a JSON object | is published with the object's fields applied |
| exits 1 | is dropped. A first line on stdout, if any, is kept as the reason |
| exits with any other code, is killed, runs past its timeout, cannot start, or prints what the rules below refuse | failed: see below |

A drop withholds the task request only. The source event is still published, so an event about
an existing task still reaches it, and another consumer of the log still sees it.

The object may hold these fields, and any other is refused:

| Field | Type | Effect | Checked |
| ----- | ---- | ------ | ------- |
| `title` | string | the task's title, in place of the event's | one line, at most 200 characters |
| `text` | string | the task's request text, in place of the event's | at most 16 KiB |
| `context` | string | text added after the request, under a line that names the script | at most 16 KiB |
| `band` | string | the task's band | one of the factory's bands |
| `lane` | string | the task's lane | `<team>/<theme>` |
| `key` | string | which events are the same work | Priory adds the source's scope, as it does for a trigger's key, so a script cannot name another source's task |

The request gains `title`, `text` and `context` as optional fields, and `pre`, an object with the
script's first argument and the names of the fields it changed.

A script cannot change the workflow, the trigger, the repository, a hold or `intervention`.
`priory check` reads those from files to say which paths reach an irreversible step and who
answers asks. A script that could change them would make that answer depend on an event.

The four uses, each a few lines of script:

- **Drop Sentry issues below a level**: read `event.data.level`, exit 1 when it is under the
  argument.
- **Enrich the text**: ask another system how many users hit the error, and print it as
  `context`.
- **Set the band from a label**: print `{"band": "blocker"}` when the labels hold `p0`.
- **Merge duplicate alerts**: print a `key` built from the alert's fingerprint and not its id. A
  factory holds at most one active task per key, so the second alert creates no second task.

#### When `pre` fails

A script that fails did not decide. Two mistakes are possible, and they cost different things. A
filter that fails open lets noise through as tasks. A filter that fails closed loses a real alert
with nobody told.

`on_error` says which, and defaults to the one that loses nothing:

| `on_error` | The request |
| ---------- | ----------- |
| `"pass"`, the default | is published unchanged, with `pre.failed` set to how the script failed |
| `"drop"` | is dropped, and the drop's record says the script failed |

Either way the desk says so, through the mechanism the intake banner already uses. The process
that ran the script posts one report from `intake`, `pre of trigger sentry-api: exit 2: <stderr
tail>`, and posts again only when the text changes. It marks the report read at that trigger's
next run that exits 0 or 1, whether or not the run changed anything.

#### One decision per event

For one event and one trigger, exactly one message says what `pre` decided:

- a `factory.task.requested`, whose `pre` object names the script, the fields it changed, the old
  and new `band`, `lane` and `key`, and `failed` when it failed;
- or a `factory.intake.dropped`, which takes the id the request would have had.

Both share one id, the UUIDv5 of the trigger and the source event that the request has today. The
log drops a second message with an id it holds. So a script that ran twice, and answered
differently the second time, still leaves one decision: the first one published. No separate
record can disagree with the request, since the request is the record.

A process that stops after the script ran and before the publish runs the script again. So a
`pre` script must be safe to run twice: it reads, and it changes nothing outside. `priory check`
cannot see that. The how-to guide says it, and `post` is the place for effects.

#### Where `pre` runs

In the process that builds the request, on its machine, from that machine's checkout of the
factory:

| Source | Process |
| ------ | ------- |
| Polled sources and schedules | the intake |
| Webhook deliveries, such as Sentry's | the server that took the delivery |

The request for this note names the intake. A webhook source has no intake step today, so the
server runs the script there. See open question 8.

A webhook sender waits for its reply, and a script may take 60 seconds. So the server keeps
replying `202` as it does today, after it published the source event. It stores the pending
request in the state file in that same step, and runs `pre` on stored requests afterwards, oldest
first. A server that restarts finds them and runs them, with the once-or-twice rule above.

Each process runs one script at a time, so a slow script delays the events behind it by at most
its timeout each. Every request is published once for the whole factory, so `pre` runs once per
event and not once per runner.

### `post`: after the task

The daemon runs `post` when a task ends: `done`, `failed` or `dropped`, a cancel included. `done`
means done: for a task that waits on its checks gate, after the gate.

Stdin is the task as `priory task ID --json` prints it. The environment adds `PRIORY_TASK`,
`PRIORY_STATE` (`done`, `failed` or `dropped`), `PRIORY_HOME`, `PRIORY_DB` and `PRIORY_FACTORY`,
as a `run` action's does. Those values are Priory's own, never event text.

The daemon treats it as it treats a command an answer runs
([tools.md](../daemon/tools.md#commands-an-answer-runs)):

- It stores the script as an action, with the task's generation, in the transaction that ends
  the task, and starts it at its next tick.
- It runs at most once per task and generation. A daemon that restarts while it runs kills it
  and ends the action `uncertain`. It never runs it again by itself.
- Its exit changes nothing about the task. A `failed` or `uncertain` action adds a desk item with
  the tail of its output, and `priory actions retry ID`, a person's verb, runs it again.
- A revived task that ends again runs `post` again, for its new generation.
- `priory actions` lists it with its status, exit and tails.

#### `post` and Principle 4

A `post` script posts to Slack, labels a pull request or moves an issue. Those are effects
nobody can take back, and no person approves each one. Principle 4 allows that in one way: the
factory marks it autonomous with a reason.

So `post` needs `autonomous = "<reason>"`, a sentence a person wrote. `priory check` refuses a
`post` without it, and `priory check --explain` lists each trigger's `post` script and its reason
in its table of what runs with no human decision. A factory that wants a person before the
effect uses a workflow step with an approving human step, which is what steps are for.

The tool guard does not read a `post` script: it guards agents. The script is the factory's code,
reviewed in the pull request that added it, as the `gate` and `deliver` are.

#### Where `post` runs

On the runner that owns the task when it ends, by that runner's daemon, from its checkout of the
factory. A machine that only views runs nothing.

#### How `post` differs from a workflow's last code step

| | `post` on a trigger | A last `run` step in the workflow |
| --- | --- | --- |
| Runs when | the task ends, however it ends: `done`, `failed`, `dropped`, cancelled | the task reaches that step |
| Runs for a failed or cancelled task | yes | no |
| Runs relative to the checks gate | after it | before it, unless the step sets `after = "merged"` |
| Can change the task | no | yes: its outcome moves the task |
| Its failure | a desk item; the task stays as it ended | the step's `failed` outcome |
| Can wait for a person's approval | no: it is `autonomous` with a reason | yes: a human step `approves` it |
| Belongs to | the trigger: two triggers of one workflow notify differently | the workflow: every trigger gets it |
| Recorded as | an action, and a `factory.task.post` record | a step, with `factory.step.ended` |
| Input | the task's JSON on stdin | `PRIORY_TASK`, and the worktree |

Use a step when the effect is part of the work, needs an approval, or decides what happens next.
Use `post` to tell the world how a task ended.

### What the trace records

| Record | Written by | When | Fields |
| ------ | ---------- | ---- | ------ |
| `factory.task.requested`, with `pre` | the process that ran `pre` | the request passed, changed or not | as today, plus `pre`: script, the fields changed, the old and new `band`, `lane` and `key`, the old and new lengths of `title`, `text` and `context`, seconds, and `failed` |
| `factory.intake.dropped` | the same | `pre` dropped a request, by exit 1 or by `on_error = "drop"` | trigger, source event id, the request's key, script, exit, reason, seconds, and `failed` |
| `factory.task.post` | the daemon | a `post` action ends | task, generation, attempt, state, script, status, exit, seconds |

A request with no `pre` script, or whose script changed nothing, carries no `pre` object beyond
the script's name.

A `factory.task.post` record's id derives from its task, generation and attempt, so a retry is
its own record. `priory inbox` shows a dropped request beside its source event, with the reason.
`priory report intake` counts, per trigger, the events matched, dropped, changed and failed.

## The first slice: `pre`

This slice is built, with every open question's recommendation taken, which Max approved on
2026-11-22. [pre.md](../intake/pre.md) describes it, and
[How to filter and enrich events with a script](../filtering-and-enriching-events.md) shows how to
write one. It differs from this note in these ways:

- The script's environment also holds `PRIORY_FACTORY` and `PRIORY_TRIGGER`, both Priory's own
  values.
- A schedule trigger runs its script for a new firing, and for the latest firing at the intake's
  start, so a firing the log already holds runs no script on each poll.
- The server stores waiting requests in the state's `pending_requests` table, and a task keeps the
  script's title, text and context in `pre_title`, `pre_text`, `pre_context` and `pre_script`.
- `priory intake pre` prints `{ "outcome", "failed", "message" }`.
- `priory check` reports a `[pre]` problem at the line of the `[pre]` table.


1. `[pre]` on a trigger, in the schema and the JSON Schemas, with the check rules: `argv` inside
   the factory folder and executable, `timeout` at most 60 seconds, `on_error` of `pass` or
   `drop`, and `env` names that are valid variable names.
2. The intake runs the script as the shared rules say, and applies its result.
3. `title`, `text`, `context` and `pre` on `factory.task.requested`, and the daemon's use of them
   when it creates the task.
4. The `pre` object on the request, the `factory.intake.dropped` record with the request's id,
   the report on the desk, and the dropped request in `priory inbox`.
5. For webhook sources, the pending request the server stores and runs `pre` on.
6. `priory intake pre TRIGGER < event.json`, which runs the script on one event and prints what
   the intake would do, and publishes nothing.
7. [triggers.md](../reference/triggers.md), [events.md](../reference/events.md),
   [task-requests.md](../intake/task-requests.md), a how-to guide, "How to filter and enrich
   events with a script", and a `pre` script in the `bug-investigation` starter.

The slice runs a child process with a timeout, a process group, JSON on stdin and capped
output. `crates/priory-runtime/src/script.rs` gives the first two. It writes both streams to
files with no limit and trims them afterwards (`script.rs:66`, `:132`), and it has no way to
feed stdin. So the slice adds the bounded reading and the stdin writer there, and names any
crate it rejected.

Tests, through the CLI and the intake's own tests, with scripts written into a scratch factory:

- A script that exits 0 and prints nothing leaves the request as it was.
- A script that exits 1 publishes the source event, no task request and one
  `factory.intake.dropped`, with its first stdout line as the reason and the id the request
  would have had. No task is created.
- A script that prints `band`, `lane`, `title`, `text` and `context` creates a task with each, and
  the request's `pre` object names the five fields.
- Two alerts whose script prints the same `key` create one task, and the second event routes to
  it.
- A printed `key` of `github:owner/name#42` from a Sentry trigger is scoped to Sentry and owns no
  GitHub task.
- An event whose title is `$(touch /tmp/x)` and whose body holds quotes and newlines reaches the
  script's stdin byte for byte, appears in no argument and no environment variable, and creates
  no file.
- The script's environment holds the short list and the `env` names, and no other variable of
  the intake.
- An unknown field, a `band` the factory lacks, a lane of another shape, a title of two lines, a
  `text` over 16 KiB, stdout that is not JSON and stdout over 64 KiB each count as a failure.
- With the default `on_error`, a script that exits 2, one that sleeps past its timeout and one
  that is missing each publish the request unchanged with `pre.failed`, and the desk shows the
  report. With `on_error = "drop"`, each drops the request.
- After a failure, a run that exits 0 and changes nothing marks the report read. So does a run
  that exits 1.
- A script that writes 1 MiB to stderr is killed and counts as failed, and Priory stored at most
  64 KiB of it. A script that never reads stdin is killed at its timeout, for an event larger
  than a pipe's buffer.
- A script that leaves a child running past its timeout has the child killed too.
- An intake stopped after the script ran and before the publish runs it again, and the log holds
  one message for the event and trigger.
- A script that passes on its first run and drops on a second run of the same event leaves the
  request and no drop record. The reverse leaves the drop and no request.
- A Sentry delivery gets its `202` before the script ends. The server then publishes the request
  the script decided, and a server restarted in between runs the script and publishes once.
- An event about an existing task still routes to it when `pre` drops the request.
- `priory intake pre` prints the changed request for one event and publishes nothing.
- `priory check` fails, with the line, for an `argv[0]` outside the factory folder, one that is
  not executable, a `timeout` of 2 minutes and an unknown `on_error`.
- A `proptest` feeds the output reader random and mutated JSON and asserts it returns or refuses
  without panicking.

## The second slice: `post`

1. `[post]` on a trigger, with `argv`, `timeout`, `env` and the required `autonomous`, and its
   check rules.
2. The action stored when a task ends, its run, and the `uncertain` end after a restart.
3. The desk item for a failed or uncertain action, and `priory actions retry ID` in the
   `priory-person` guard.
4. The `factory.task.post` record, and the `--explain` table's rows.
5. [triggers.md](../reference/triggers.md), [desk.md](../daemon/desk.md),
   [cli.md](../reference/cli.md) and the how-to guide.

Tests, through the CLI:

- A task that reaches `done` runs the script once, with the task's JSON on stdin and
  `PRIORY_STATE=done`. The same for a task that failed, one a result sent to `stop`, and one a
  person cancelled, each with its state.
- A task with a pull request runs `post` after its checks turn green, and not when its workflow
  reaches `done` before them.
- A task whose summary holds `$(...)` and quotes reaches stdin byte for byte and appears in no
  argument.
- A script that exits 1 leaves the task `done`, adds a desk item with its stderr tail, and
  `priory actions retry` runs it again. The verb is refused in a run's pane.
- A daemon stopped while the script runs ends the action `uncertain` at its next start and does
  not run it again.
- A revived task that ends again runs the script a second time, and the first generation's
  action stays as it was.
- A task of a factory of two runners runs `post` on the runner that owns it, once.
- A trigger with `post` and no `autonomous` fails `priory check` with its line, and
  `priory check --explain` lists the script with its reason.
- A script past its timeout is killed with its children and ends `failed`.

## Later slices

1. `pre` for `priory add` and `priory request`, through `triggers/cli.toml`, with the drop and
   its reason printed to the person.
2. Declared secrets in `env`, from [agent-credentials.md](agent-credentials.md).
3. `priory report intake`, with counts beside every rate.
4. A `pre` that asks: exit 3 publishes the request held, with an ask to the person. It would give
   a script a third answer between pass and drop.
5. Scripts that run beside the poll, so one slow script does not delay other triggers.

## Open questions

Max approved every recommendation below on 2026-11-22.

1. **Does every non-zero exit drop the request?** The request for this note says a non-zero exit
   drops it. A
   script with a typo exits non-zero too, and would then drop every alert in silence.
   Recommendation: exit 1 drops, as a decision. Any other failure follows `on_error`, which
   defaults to `pass` and posts a report on the desk. A factory that prefers silence over noise sets
   `on_error = "drop"` for that trigger.
2. **May `pre` change the workflow?** Routing an event to another workflow by its content is
   useful. It also makes the answer of `priory check --explain` depend on an event. Recommendation:
   no. Write two triggers with two filters, and let each `pre` drop what is not its own.
3. **May `pre` have side effects?** It can run twice for one event, and it runs before any task
   exists to tie an effect to. Recommendation: no, by convention and in the guide. Effects belong
   to `post` or to a step.
4. **Does `post` need `autonomous`?** It adds a line to every trigger that only posts to a
   channel. Without it, a trigger file could label and close things with nothing in
   `--explain` saying so. Recommendation: require it. One sentence is cheap, and it keeps
   Principle 4 without an exception.
5. **Should `post` run for a task that was dropped before any run?** A request dropped by a
   person on the desk never did work. Recommendation: yes, with `PRIORY_STATE=dropped`. The
   script has the task's JSON and can exit early when it holds no result. One rule is easier to
   explain than two.
6. **Is `post` retried by itself?** A notification sent twice is a small harm, and an issue moved
   twice can be a large one. Recommendation: never. It runs at most once, and a person retries
   from the desk, as for a command an answer runs.
7. **A shell line or an `argv` array?** `[sweep] scripts` and the `run` action take a shell
   line. Recommendation: `argv` here. These two scripts are the ones that sit next to untrusted
   text, and an array leaves no place to interpolate it.
8. **Where does `pre` run for a webhook?** In the server, as written above, or in the intake,
   after a handoff. A handoff would put every decision in one process. It needs a durable queue
   from the server to the intake that does not exist. Recommendation: the server, with the
   pending request in the state file. One function runs the script in both processes, so the
   rules cannot drift.
