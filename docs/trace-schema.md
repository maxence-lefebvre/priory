# Trace schema

Status: draft, 2026-11-17. Proposed as a shared format, so any factory can emit it and any
measurement tool can read it.

A factory appends one record each time a workflow step ends. A later judgement about that step,
such as a revert, arrives as a second record that points at the first. Each record is a
[CloudEvents 1.0](https://cloudevents.io) event in JSON, and the fields below live in `data`.

## `factory.step.ended`

```json
{
  "specversion": "1.0",
  "id": "0196f1c2-7a3e-7c41-9b2d-5e8f0a1b2c3d",
  "source": "factory://example/xps13",
  "type": "factory.step.ended",
  "time": "2026-11-17T16:20:05Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "github:example/app#42",
    "lane": "3D/mobile",
    "run": "0196f1b0-0000-7000-8000-000000000002",
    "generation": 1,
    "step": "plan",
    "decider": "model",
    "effect": "reversible",
    "outcome": "green",
    "harness": "claude-code",
    "model": "claude-opus-5-5",
    "machine": "xps13",
    "started": "2026-11-17T16:12:41Z",
    "duration_ms": 444000,
    "tokens": { "input": 51234, "output": 3120, "cache_read": 402113, "cache_write": 20480 }
  }
}
```

| Field              | Required | Meaning                                                               |
| ------------------ | -------- | --------------------------------------------------------------------- |
| `factory`          | yes      | The factory's name                                                    |
| `factory_revision` | yes      | The commit of the factory's files, so runs of two configs can be compared |
| `workflow`         | yes      | The workflow the step belongs to                                      |
| `task`             | yes      | The task's id                                                         |
| `key`              | no       | What the task works on, prefixed by its source                        |
| `lane`             | no       | A `<team>/<theme>` grouping                                           |
| `run`              | no       | The agent run that produced the step; absent for `human` and `code`   |
| `generation`       | yes      | Starts at 1 and grows each time the task is revived                   |
| `step`             | yes      | The step's id in the workflow                                         |
| `decider`          | yes      | `code`, `model` or `human`                                            |
| `effect`           | yes      | `none`, `read`, `reversible` or `irreversible`                        |
| `outcome`          | yes      | The label that chose the next step, such as `approved` or `failed`    |
| `grade`            | no       | For a `model` step whose outcome is `red`, `yellow` or `green`: that outcome |
| `harness`, `model` | no       | For `model` steps                                                     |
| `machine`          | yes      | Where the step ran                                                    |
| `started`          | yes      | When the step started                                                 |
| `duration_ms`      | yes      | Wall time from start to end                                           |
| `tokens`           | no       | For `model` steps: `input`, `output`, `cache_read`, `cache_write`, summed over the step's turns |

## `factory.task.repaired`

A factory's attempt to repair a task whose agent failed. It shares `factory`, `factory_revision`,
`workflow`, `task`, `key`, `generation`, `step` and `machine` with `factory.step.ended`, and `run`
names the run that failed.

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-000000000004",
  "source": "factory://example/xps13",
  "type": "factory.task.repaired",
  "time": "2026-11-19T10:04:12Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "github:example/app#42",
    "run": "0196f1b0-0000-7000-8000-000000000002",
    "generation": 1,
    "step": "plan",
    "machine": "xps13",
    "cause": "disk full (10.0 GB of disk free, keeps 20.0 GB free)",
    "action": "sweep",
    "outcome": "failed"
  }
}
```

| Field     | Required | Meaning                                                                   |
| --------- | -------- | ------------------------------------------------------------------------- |
| `cause`   | yes      | Why the run failed, as the factory classified it                          |
| `action`  | yes      | What the factory did about the cause, such as `sweep` or `wait for room`  |
| `outcome` | yes      | `ready` when the task waits to start its agent again, `failed` when it stays failed |

## `factory.task.reopened`

Feedback on a done task's open pull request, issue, Linear issue or Slack thread moved it back to a
step, as its next generation.
It shares `factory`, `factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and
`machine` with `factory.step.ended`. `generation` is the new one and `step` the step it went back to.

```json
{
  "specversion": "1.0",
  "id": "0196f1d5-0000-7000-8000-000000000006",
  "source": "factory://example/xps13",
  "type": "factory.task.reopened",
  "time": "2026-11-20T18:12:40Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "github:example/app#42",
    "generation": 2,
    "step": "implement",
    "machine": "xps13",
    "resource": "github:example/app#57",
    "pull_request": "github:example/app#57"
  }
}
```

| Field          | Required | Meaning                                               |
| -------------- | -------- | ----------------------------------------------------- |
| `resource`     | yes      | The pull request, issue, Linear issue or Slack thread whose feedback reopened the task |
| `pull_request` | no       | The pull request whose feedback reopened the task; absent for other feedback |

## `factory.tool.denied`

The tool guard denied a run's irreversible call (see [guard.md](daemon/guard.md)). It shares
`factory`, `factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and `machine` with
`factory.step.ended`. `step` is the task's step when the run made the call.

```json
{
  "specversion": "1.0",
  "id": "0196f1d5-0000-7000-8000-000000000007",
  "source": "factory://example/xps13",
  "type": "factory.tool.denied",
  "time": "2026-11-21T10:02:11Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "run": "0196f1b0-0000-7000-8000-000000000002",
    "key": "github:example/app#42",
    "generation": 1,
    "step": "implement",
    "machine": "xps13",
    "tool": "Bash",
    "guard": "gh-pr-merge",
    "call": "gh -R example/app pr merge 57",
    "reason": "Priory denied `gh -R example/app pr merge 57`: it is irreversible (guard `gh-pr-merge`) and step `implement` is not irreversible, so no person approved a call like this. ..."
  }
}
```

| Field    | Required | Meaning                                                        |
| -------- | -------- | -------------------------------------------------------------- |
| `run`    | yes      | The run that made the call                                     |
| `tool`   | yes      | The tool, such as `Bash` or an `mcp__<server>__<tool>` name    |
| `guard`  | yes      | The guard the call matched                                     |
| `call`   | yes      | The command that matched, its wrappers removed, or the MCP tool |
| `reason` | yes      | What the agent was told                                        |

## `factory.task.snoozed` and `factory.task.unsnoozed`

A person snoozed a desk item of a task, or cleared its snooze. Both share `factory`,
`factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and `machine` with
`factory.step.ended`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000005",
  "source": "factory://example/xps13",
  "type": "factory.task.snoozed",
  "time": "2026-11-20T17:02:11Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "github:example/app#42",
    "generation": 1,
    "step": "approve-pr",
    "machine": "xps13",
    "item": { "kind": "ask", "id": "7" },
    "until": "2026-11-23T08:00:00Z"
  }
}
```

| Field   | Required | Meaning                                                              |
| ------- | -------- | -------------------------------------------------------------------- |
| `item`  | yes      | The desk item: `kind` is `ask`, `blocked`, `failed` or `prompt`, and `id` its id |
| `until` | no       | When the item comes back by itself; only on `factory.task.snoozed`   |

## `factory.task.snooze_ended`

A snooze ended by itself: at its time, or early on a change (see
[desk.md](daemon/desk.md#snooze)). The daemon writes one per snooze, within a second of its end. It
shares the fields of `factory.task.snoozed`, without `until`, and adds:

| Field    | Required | Meaning                                                              |
| -------- | -------- | -------------------------------------------------------------------- |
| `item`   | yes      | As on `factory.task.snoozed`, but `kind` is `task` for a `blocked` or `failed` item |
| `reason` | yes      | `time`, or the early reason: `new revision`, `new failure` or `task state changed` |

## `factory.task.progressed`

A task's progress changed, through the `progress` tool or `priory progress`. It shares `factory`,
`factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and `machine` with
`factory.step.ended`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000006",
  "source": "factory://example/xps13",
  "type": "factory.task.progressed",
  "time": "2026-11-20T17:10:42Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "github:example/app#42",
    "generation": 1,
    "step": "implement",
    "machine": "xps13",
    "progress": {
      "goal": "Fix the docs",
      "acceptanceCriteria": [{ "id": "AC1", "description": "cargo test passes", "required": true }],
      "nextAction": "Open the pull request"
    }
  }
}
```

| Field      | Required | Meaning                                                            |
| ---------- | -------- | ------------------------------------------------------------------ |
| `progress` | yes      | The task's whole progress after the change, as `/api/tasks` shows it |

## `factory.step.labeled`

A judgement that arrives after the step, such as a rejected approval, red CI after a merge, or a
revert. `step_event` is the `id` of the `factory.step.ended` record it judges. Priory writes one per
task and label, judging the step that graded the task, as [Outcome
labels](daemon/trace.md#outcome-labels) describes. Its `data` also holds `factory`,
`factory_revision`, `workflow`, `task`, `key`, `generation` and `machine` as above, and `grade`, the
task's grade when it was labeled. A task no step graded has no `step_event`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d0-0000-7000-8000-000000000003",
  "source": "factory://example/xps13",
  "type": "factory.step.labeled",
  "time": "2026-11-18T09:02:11Z",
  "data": {
    "step_event": "0196f1c2-7a3e-7c41-9b2d-5e8f0a1b2c3d",
    "label": "reverted",
    "evidence": "https://github.com/example/app/pull/57"
  }
}
```

## `factory.handler.called`

A short model call that read a stalled run's screen and chose what to do with it. Its `data` holds
`factory`, `factory_revision`, `workflow`, `task`, `key`, `run`, `generation`, `step`, `harness`,
`model`, `machine` and `tokens` as above, and:

| Field      | Required | Meaning                                                            |
| ---------- | -------- | ------------------------------------------------------------------ |
| `action`   | yes      | `nudge`, `restart` or `leave`                                      |
| `text`     | yes      | The nudge's text, or why it restarted or left the run              |
| `cost_usd` | no       | The cost the harness reported for the call                         |

Priory writes these records as described under Trace in [the daemon](daemon/trace.md).

## `factory.summarizer.called`

A short model call that wrote, or failed to write, a new task's TL;DR. Its `data` holds `factory`,
`factory_revision`, `workflow`, `task`, `key`, `generation`, `step`, `harness`, `model`, `machine`
and `tokens` as above, and:

| Field      | Required | Meaning                                                            |
| ---------- | -------- | ------------------------------------------------------------------ |
| `status`   | yes      | `written`, or why the call wrote no TL;DR                          |
| `cost_usd` | no       | The cost the harness reported for the call                         |

Priory writes these records as described in [search](daemon/search.md).

## `factory.gate.ran`

`priory gate` ran the factory's gate in a task's worktree (see Proofs in
[tools.md](daemon/tools.md#proofs)). It shares `factory`, `factory_revision`, `workflow`, `task`,
`key`, `generation`, `step` and `machine` with `factory.step.ended`. `step` is the task's step when
the gate ran, and `time` is when it ended.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000008",
  "source": "factory://example/xps13",
  "type": "factory.gate.ran",
  "time": "2026-11-21T10:04:30Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "run": "0196f1b0-0000-7000-8000-000000000002",
    "key": "github:example/app#42",
    "generation": 1,
    "step": "implement",
    "machine": "xps13",
    "commit": "3f9c2a7e5b1d4c6f8a0e2b4d6f8a1c3e5b7d9f0a",
    "exit": 0,
    "duration_ms": 84200
  }
}
```

| Field         | Required | Meaning                                                          |
| ------------- | -------- | ---------------------------------------------------------------- |
| `run`         | yes      | The run whose agent ran the gate                                 |
| `commit`      | yes      | The head of the task's branch when the gate started              |
| `exit`        | yes      | The gate's exit code; `-1` when a signal killed it               |
| `duration_ms` | yes      | How long the gate ran                                            |

## Spans

A factory that exports OpenTelemetry spans (Priory does: see
[OpenTelemetry export](daemon/trace.md#opentelemetry-export)) maps one task to one trace, with a root span
per generation, and `factory.step.ended` records to spans below it:

- A root span per task generation, from its creation to its end, which no record carries. The
  exporter reads it from the task, so a task needs no record to end.
- A span per run, child of the root: from a `factory.step.ended` record's `run`, or, for a run that
  ended with no step, from the run itself.
- A span per step, child of its run span, or of the root when it has no run.

## Rules

- `id` is a UUID, stable across retries, so a reader drops duplicates by `id`.
- Readers ignore fields and types they do not know.
- A factory may add fields under its own prefix, such as `priory_` or `bowtie_`.
- Report counts next to every rate, and leave a rate unknown when no record supports it.
