# Trace

The daemon appends one `factory.step.ended` record per step end, in the transaction that moves the
task on, as [the trace schema](../trace-schema.md) defines it. A model step ends with its run's
result, a human step with its answer, a code step with its action's status (the record's
`outcome`). A run that ends otherwise, such as blocked, lost or cancelled, ends no step and appends
no record. A repair appends `factory.task.repaired` instead (see Repair), and a reopen appends
`factory.task.reopened` (see [routing.md](routing.md#feedback-reopens-a-done-task)). `priory report` reads only
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
- `factory.step.labeled` is not written yet.
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
