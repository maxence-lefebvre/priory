# Trace

The daemon appends one `factory.step.ended` record per step end, in the transaction that moves the
task on, as [the trace schema](../trace-schema.md) defines it. A model step ends with its run's
result, a human step with its answer, a code step with its action's status (the record's
`outcome`). A run that ends otherwise, such as blocked, lost or cancelled, ends no step and appends
no record. A repair appends `factory.task.repaired` instead (see Repair), and a reopen appends
`factory.task.reopened` (see [routing.md](routing.md#feedback-reopens-a-done-task)). A judgement that comes later,
such as a merge or a revert, appends `factory.step.labeled` (see Outcome labels). `priory report` reads only
`factory.step.ended`.

Records live in the `trace` table of the state file, one CloudEvents JSON object per row. R9.3 keeps
task state in Priory's own SQLite file and the event log in herdr-bus's. A bus write could not commit
with the state change it describes, so a crash between them would lose a record or write one twice.
A row commits with the move. An exporter publishes rows later, using `seq` as cursor (see
OpenTelemetry export).

`priory trace` prints every record, oldest first, one JSON object per line.

| Field | Priory's value |
| ----- | -------------- |
| `factory` | `name` in `factory.toml` |
| `factory_revision` | the last commit that changed the factory's folder; absent outside git |
| `task`, `run` | the task's and the run's ids; `run` only for a model step, the run that sent the result |
| `generation` | the task's generation when the step ended: 1, plus one per revive |
| `lane` | the task's lane when the step ended; absent for a task created before lanes |
| `machine` | the host name, which also names the runner |
| `started` | when the task entered the step, so it includes the time the task waited for a run |
| `harness`, `model`, `tokens` | for a model step: the agent's harness, the model the transcript names, else the agent's |

A model step's `tokens` come from Claude's session transcript, the file that `transcript_path` names. A run records the file
and its length at `SessionStart`, since a resumed session's file already holds earlier runs' turns.
At each `Stop`, the daemon reads the lines added since and adds each assistant message's usage to the
task's current step. Claude writes a message once per content block, each line with its usage, so a
message id counts once. A run the daemon blocks or finds lost is read as it ends, since a resume
carries on the same step. Entering a step starts its count again. No assistant message read means no
`tokens`. An unreadable transcript keeps its offset, so the next read counts its turns once.

Known limits:

- A run live when the daemon upgraded to this schema has no transcript recorded. Its first `Stop`
  only marks where reading starts, so that turn's tokens are lost, not counted with other runs'
  turns.
- A step using two models, such as an agent whose model changed between runs, records the last one.
  `--cost` prices all its tokens at that model's price.
- `factory_revision` names the last commit, even with uncommitted changes.

## OpenTelemetry export

The daemon can send its `factory.step.ended` records to an OTLP/HTTP endpoint as spans. It is off
unless `factory.toml` names the endpoint, and then nothing is sent:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"   # spans go to <otlp>/v1/traces as OTLP JSON
```

A task is one trace, with a root span named `task <key>`, from the task's creation to its end. A
model step's run is a span named `invoke_agent <agent>` under the root, from the run's start to the
step's end, and the step is a span under it. A human or code step has no run, so its span is a child
of the root. A run that ended with no step, such as a cancelled, blocked or lost one, is a span of
the same name under the root, from its start to its end, with `priory.run.ended_as` naming how it
ended. An OTLP span is immutable once sent, so the root span, and the spans of runs that ended no
step, are sent when the task reaches `done`, `failed` or `dropped`, and not before. A task that a
revive or a reopen sends on starts a new root span, as its next generation, and the same trace. The step span starts when the task entered the step, so it includes the time the task
waited for a run and may start before its run. The ids derive from the record's `source` and the
task id and generation, run id or record id, so sending a record again gives the same ids.

| Attribute | Span | Value |
| --------- | ---- | ----- |
| `priory.task.id`, `priory.run.id`, `priory.step`, `priory.lane` | run, step | the record's `task`, `run`, `step` and `lane` |
| `priory.task.id`, `priory.key`, `priory.workflow`, `priory.lane`, `priory.band`, `priory.grade`, `priory.state`, `priory.generation` | root | the task's id, key, workflow, lane, band, grade, final state and generation when it ended |
| `priory.run.ended_as` | run of no step | how the run ended: `cancelled`, `lost` or `blocked` |
| `priory.outcome`, `priory.grade`, `priory.workflow`, `priory.key`, `priory.generation`, `priory.decider`, `priory.effect`, `priory.harness` | step | the record's fields of the same name |
| `gen_ai.request.model` | both | the record's `model` |
| `gen_ai.usage.input_tokens` | step | the record's `input`, `cache_read` and `cache_write` tokens together |
| `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens` | step | the record's `output`, `cache_read` and `cache_write` tokens |
| `gen_ai.operation.name`, `gen_ai.agent.name` | run | `invoke_agent` and the run's agent |

The resource carries `service.name = priory`, `priory.factory` and `host.name`, the machine.

Each tick, the daemon sends up to 200 records and 200 task ends past the endpoint's cursors in one request, through
`curl`, and moves on without waiting for it. `curl` gives up after `command_timeout`. The cursors, in
the state file's `exports` table (`seq` for records, `ended` for the `task_ends` table, which
the daemon fills when a task ends), move once the endpoint answers with a 2xx status, so a daemon
restart resends nothing it saw accepted. Any other answer, or no answer, is logged and the batch is
tried again 5 seconds later, except a 4xx other than 408 and 429: OTLP says such a request will never
be accepted, so the daemon logs it and skips the batch. A new endpoint starts from the first record.

Each harness process gets `OTEL_RESOURCE_ATTRIBUTES` with `priory.task.id` and `priory.run.id`,
after the daemon's own value when it has one. A process lives as long as its first run's agent keeps
the task, so a run carried into the next step keeps the run id of the run that started it. A harness that exports its own telemetry, such as Claude
Code with `CLAUDE_CODE_ENABLE_TELEMETRY`, then tags it with the same ids in the same backend. Priory
sets nothing else of the harness's telemetry.

Known limits:

- A daemon that stops after `curl` succeeded but before its next tick sends that batch again, with
  the same ids.
- `factory.task.repaired` and `factory.handler.called` records are not exported.
- A task that is still active has no root span yet, so its spans have a parent that arrives later,
  and a run that ended with no step has no span until its task ends.
- A task that ended before the daemon upgraded to this schema has no root span, so its run and step
  spans, when sent after the upgrade, name a parent that was never sent. A task revived before the
  upgrade attaches its earlier generations' runs without a step to its next root span.
- A root span's id uses the daemon's current factory name and host name, so renaming either
  between the steps and the end of a task leaves the root with another id than its children name.
- A task's root span names its lane, band and grade as they stand when it is sent.
- A partial success, a 2xx whose body names rejected spans, counts as accepted.

## Reports

`priory report [DIR] --workflow <name> [--since <window>] [--by lane]` prints, per workflow step, each
outcome with its count, the records that reached the step, and its rate:

```
step	outcome	count	reached	rate
review	approved	1	2	50.0%
review	changes	1	2	50.0%
```

A visit reaches its step when it ends there or while an active task sits at it, so outcome rates for
a step with tasks in progress add up to under 100 %. `--since` takes an RFC 3339 instant or a duration
such as `7 days` (the default), counted back from now. `--by lane` splits counts by the task's current
lane and adds a leading `lane` column. It does not combine with `--cost`.

`--cost` prints each task's tokens and cost instead, per step, then in total. Prices come from
`prices.toml` in the factory folder `DIR` (default: the current folder). Each table is a model, named
as the records name it, with dollars per million tokens of each kind:

```toml
["claude-opus-5-5"]
input = 5.0
output = 25.0
cache_read = 0.5
cache_write = 6.25
```

A model with no price keeps its tokens. Its cost reads `price missing for <model>`, as does its task's
total. A model step with no `tokens` shows `-` and `tokens unknown`, and its task's total names it.
`priory check` refuses a price below zero or not finite. Prices apply at report time, so a changed
price changes past costs too. A task's handler calls show as one more row per model, with step
`handler`, and count in its total. Its summarizer call does too, with step `summarizer`.

## Costs

`priory costs [DIR] [--since <window>] [--by workflow|agent|model|machine|repo] [--json]` prints what
the factory spent in the current window against the window before it, then each day, then the
current window split by one dimension:

```
period	since	until	cost	notes	merges
current	2026-11-15T23:00:00.000Z	2026-11-17T11:00:00.000Z	$2.000000	-	1
previous	2026-11-08T23:00:00.000Z	2026-11-15T23:00:00.000Z	$5.000000	-	1

day	cost	notes	merges
2026-11-14	$2.000000	-	0
2026-11-15	$3.000000	-	1
2026-11-16	$2.000000	-	1

workflow	cost	notes
implement	$2.000000	-
```

A `notes` cell names what has no cost, such as `price missing for m2: 1500 tokens` or
`tokens unknown for 1 records`.

It reads the same records as `report --cost`: model steps, handler calls and summarizer calls, and
prices them from the same `prices.toml` in `DIR` (default: the current folder).

- Without `--since`, the current window starts this Monday at 00:00 in the local timezone (`TZ`) and
  ends now, and the previous window is the full week before it.
- `--since` takes what `report --since` takes, and must start before now. The previous window then
  has the current window's length and ends where the current one starts.
- Days are local dates. A day appears once it has a record or a merge.
- `--by` defaults to `workflow`. `agent` is the run's agent, `handler` for a handler call and
  `summarizer` for a summarizer call. `repo` is the task's repository. A record with no value reads
  `-`.
- A model with no price keeps its tokens apart, as `price missing for <model>`. A row whose records
  were all unpriced or had no tokens has no cost, `-`, never `$0`. A model step with no `tokens`
  counts as `tokens unknown`.
- `merges` counts the pull requests whose stored `github.pull_request.closed` event says they
  merged, when a task owns the pull request or its head branch, as a worker's task owns its branch.
  A pull request counts once, on the local day of its first merged close. The split has no merges.

`--json` prints one object, which `GET /api/costs` also answers (see
[the serve reference](../reference/serve.md#costs)).

Known limits:

- Prices apply at report time, as for `report --cost`.
- A merge counts only once its webhook or poll stored the closed event.
- A pull request a task only subscribed to does not count: its subscription ends when it closes.
- `--since` also takes what SQLite reads as a time, such as `12:00`, which is noon on 2000-01-01.

## Outcome labels

The daemon appends a `factory.step.labeled` record when an event judges a task, in the transaction
that stores the event or the answer. It names the record of the step that graded the task, when one
did, and the task's grade. A task gets each label once.

A task's pull requests are those of its stack (see [tools.md](tools.md)) and those whose head is a
branch it owns. A pull request that only names the task's key, as a review task's does, is not its.
A review whose author ends in `[bot]` does not count as a human review.

| Label | When |
| ----- | ---- |
| `approved-clean` | The last open pull request of the task merged, no human review requested changes on any of them, and no head changed after a pull request's first human review |
| `changes-requested` | A human review requested changes on one of its pull requests; or, as its last one merged, a stored event of one of them named another head than its first human review did, so a commit came after the review |
| `approval-rejected` | A person answered one of its human steps with an option that starts with `reject`, such as `rejected` |
| `closed-unmerged` | One of its pull requests closed without merging |
| `red-after-merge` | A check run on the merge commit of one of its merged pull requests completed with a conclusion other than `success`, `neutral` or `skipped` |
| `reverted` | A later pull request of the same repository merged, and its title or body names the merge commit of one of the task's, as `git revert` writes, or says `Reverts owner/name#N`, as GitHub's revert button writes |

The first four are final: a task with one of them has an outcome its grade can be compared with.
`red-after-merge` and `reverted` can come later, on top of `approved-clean`.

Known limits:

- The intake reads check runs only on pull requests' head commits, so it does not see a check on a
  merge commit on the default branch yet, and `red-after-merge` comes only from a merge commit that
  is also a pull request's head.
- A head change counts only when an event of the pull request stored after the review names the
  new head, such as a check run or the close.
- A task whose pull request closed unmerged and whose next one merged clean has both
  `closed-unmerged` and `approved-clean`.

## Grades

`priory grades [--since <window>] [--json]` compares each grade with its tasks' outcome labels. It
counts the tasks with a grade, created since `--since`, which takes what `report --since` takes, or
every graded task without it. A task counts under its grade now.

```
green one-shot: 50.0% (1 of 2), target 90%

grade	tasks	outcomes	approved-clean	changes-requested	approval-rejected	closed-unmerged	red-after-merge	reverted
red	0	0	0 (-)	0 (-)	0 (-)	0 (-)	0 (-)	0 (-)
yellow	1	1	1 (100.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)
green	2	2	1 (50.0%)	1 (50.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)
```

The headline is green's one-shot rate: its `approved-clean` tasks over its tasks with a final
outcome, against the 90 % target. `outcomes` counts the tasks with a final outcome, and each label's
share is its count over them; `-` when there are none. A task still waiting for its pull request's
first review has no outcome yet.

`--json` prints one object, which `GET /api/grades` also answers (see
[the serve reference](../reference/serve.md#grades)).
