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

A task is one trace. A model step's run is a span named `invoke_agent <agent>`, from the run's start
to the step's end, and the step is a span under it. A human or code step has no run, so its span has
no parent. The step span starts when the task entered the step, so it includes the time the task
waited for a run and may start before its run. The ids derive from the record's `source` and the
task id, run id or record id, so sending a record again gives the same ids.

| Attribute | Span | Value |
| --------- | ---- | ----- |
| `priory.task.id`, `priory.run.id`, `priory.step`, `priory.lane` | both | the record's `task`, `run`, `step` and `lane` |
| `priory.outcome`, `priory.grade`, `priory.workflow`, `priory.key`, `priory.generation`, `priory.decider`, `priory.effect`, `priory.harness` | step | the record's fields of the same name |
| `gen_ai.request.model` | both | the record's `model` |
| `gen_ai.usage.input_tokens` | step | the record's `input`, `cache_read` and `cache_write` tokens together |
| `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens` | step | the record's `output`, `cache_read` and `cache_write` tokens |
| `gen_ai.operation.name`, `gen_ai.agent.name` | run | `invoke_agent` and the run's agent |

The resource carries `service.name = priory`, `priory.factory` and `host.name`, the machine.

Each tick, the daemon sends up to 200 records past the endpoint's cursor in one request, through
`curl`, and moves on without waiting for it. `curl` gives up after `command_timeout`. The cursor, in
the state file's `exports` table, moves once the endpoint answers with a 2xx status, so a daemon
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
- `factory.task.repaired` and `factory.handler.called` records are not exported, and a run that ends
  no step, such as a blocked or lost one, has no span.
- A task has no span of its own, so its runs are the trace's top spans.
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
