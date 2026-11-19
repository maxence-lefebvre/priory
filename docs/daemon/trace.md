# Trace

Each time a step ends, the daemon appends one `factory.step.ended` record, as
[the trace schema](../trace-schema.md) defines it, in the transaction that moves the task on. A model
step ends with its run's result, a human step with its answer, and a code step with its action's
status, which is the record's `outcome`. A run that ends any other way, such as blocked, lost or
cancelled, ends no step and appends no record. A repair appends a `factory.task.repaired` record
instead, as under Repair. `priory report` reads only `factory.step.ended` records.

The records live in the `trace` table of the state file, one CloudEvents JSON object per row. R9.3
keeps the task state in Priory's own SQLite file and the event log in herdr-bus's. A record written
to the bus could not commit with the state change it describes, so a crash between the two would
lose a record or write one twice. A row in the state file commits with the move. An exporter can
publish the rows to the bus or to OpenTelemetry later, with the row's `seq` as its cursor and the
record's `id` to drop a duplicate.

`priory trace` prints every record, oldest first, one JSON object per line.

| Field | Priory's value |
| ----- | -------------- |
| `factory` | `name` in `factory.toml` |
| `factory_revision` | the last commit that changed the factory's folder; absent outside git |
| `task`, `run` | the task's and the run's ids; `run` only for a model step, the run that sent the result |
| `generation` | the task's generation when the step ended: 1, plus one per revive |
| `lane` | absent until lanes land |
| `machine` | the host name, which also names the runner |
| `started` | when the task entered the step, so it includes the time the task waited for a run |
| `harness`, `model`, `tokens` | for a model step: the agent's harness, the model the transcript names, else the agent's |

A model step's `tokens` come from Claude's session transcript, the file that `transcript_path` names.
A run records the file and its length at `SessionStart`, since a resumed session's file already
holds earlier runs' turns. At each `Stop`, the daemon reads the lines added since and adds the usage
of each assistant message to the task's current step. Claude writes a message once per content block,
each line with the message's usage, so a message id counts once. A run the daemon blocks or finds
lost is read as it ends, since a resume carries on the same step. Entering a step starts its count
again. A step with no assistant message read has no `tokens`. A transcript that cannot be read keeps
its offset, so the next read counts its turns once.

Known limits:

- A run that was live when the daemon upgraded to this schema has no transcript recorded. Its first
  `Stop` only marks where its reading starts, so that turn's tokens are lost rather than counted
  with other runs' turns.
- A step that used two models, as when an agent's model changed between its runs, records the last
  one, and `--cost` prices all its tokens at that model's price.
- `factory.step.labeled` is not written yet.
- `factory_revision` names the last commit, even when the factory has uncommitted changes.

## Reports

`priory report [DIR] --workflow <name> [--since <window>] [--by lane]` prints, for each step of the workflow,
each outcome with its count, the records that reached the step and its rate:

```
step	outcome	count	reached	rate
review	approved	1	2	50.0%
review	changes	1	2	50.0%
```

A visit reaches its step when it ends there, or while an active task is still at it, so the
outcome rates of a step with tasks in progress add up to less than 100 %.
`--since` takes an RFC 3339 instant or a duration such as `7 days`, the default, back from now.
`--by lane` counts each lane apart, by the lane its task has now, and adds a leading `lane` column. It
does not combine with `--cost`.

`--cost` prints each task's tokens and cost instead, per step, then in total. The prices come from
`prices.toml` in the factory folder `DIR`, which defaults to the current folder. Each table is a model,
as the records name it, with dollars per million tokens of each kind:

```toml
["claude-opus-5-5"]
input = 5.0
output = 25.0
cache_read = 0.5
cache_write = 6.25
```

A model with no price keeps its tokens, and its cost reads `price missing for <model>`, as does its
task's total. A model step with no `tokens` shows `-` and `tokens unknown`, and its task's total names
it. `priory check` refuses a price below zero or not finite. Prices apply when the report runs, so a changed price changes past costs too.
The handler calls of a task show as one more row per model, with the step `handler`, and count in its
total.
