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
  "source": "factory://example/desktop",
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
    "machine": "desktop",
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
| `outcome`          | yes      | The label that chose the next step, such as `approved` or `failed`. `written` for a person's written answer to a human step, which sends the task back to the model step it ran last that is neither irreversible nor a debate |
| `grade`            | no       | For a `model` step whose outcome is `red`, `yellow` or `green`: that outcome |
| `harness`, `model` | no       | For `model` steps                                                     |
| `kit_hash`         | no       | For `model` steps: a hash of the factory's skills, instructions and memories the run got; absent when it got none of them. See [skills.md](daemon/skills.md) |
| `kit_commit`       | no       | For `model` steps: the commit of the factory's `[kit]` repository the run got; absent without one. See [skills.md](daemon/skills.md#a-kit-repository) |
| `memories_commit`  | no       | For `model` steps: the commit of the factory's `[memories]` repository the run got; absent without one. See [skills.md](daemon/skills.md#a-memories-repository) |
| `sandbox`          | no       | For `model` steps: the sandbox the run's agent started in; absent without one. See [sandboxes.md](daemon/sandboxes.md) |
| `session_start_ms` | no       | For `model` steps: the time from the run's launch to its harness's `SessionStart`, jailed or not; absent when it never started |
| `machine`          | yes      | Where the step ran                                                    |
| `started`          | yes      | When the step started                                                 |
| `duration_ms`      | yes      | Wall time from start to end                                           |
| `tokens`           | no       | For `model` steps: `input`, `output`, `cache_read`, `cache_write`, summed over the step's turns |
| `overrun`          | no       | For a `model` step with a `budget` whose turn sent its result over it: `budget_usd` and `spent_usd`, priced with `prices.toml`. See [health.md](daemon/health.md#budget) |
| `consult`          | no       | For an investigator's step: `id`, the consult it answers, and `task`, the asking task's id. See [tools.md](daemon/tools.md#consulting-an-investigator) |
| `debate`           | no       | For a debate step: the debate, below. Its record has no `harness`, `model` or `tokens`, since each turn's `factory.debate.turn_ended` carries its own |

A debate step's `decider` is `model` when its agents agreed, and `human` when a person picked the
outcome. Its `debate` holds:

| Field             | Meaning                                                              |
| ----------------- | -------------------------------------------------------------------- |
| `id`              | The debate's id                                                      |
| `agents`          | The two agents, in speaking order                                    |
| `rounds`          | The rounds run after the openings, 0 when the openings agreed        |
| `openings_agreed` | Whether both openings named the same verdict                         |
| `changed`         | The agents whose verdict changed during the debate, possibly none    |
| `cost_usd`        | The debate's cost, the sum of its turns, each priced at its own model |

## `factory.debate.turn_ended`

One turn of a debate step, appended when the turn ends, so each turn is priced at its own agent's
model. Its `data` holds `factory`, `factory_revision`, `workflow`, `task`, `key`, `lane`, `run`,
`generation`, `step`, `harness`, `model`, `machine` and `tokens` as above, and:

| Field     | Required | Meaning                                                          |
| --------- | -------- | ---------------------------------------------------------------- |
| `debate`  | yes      | The debate's id, as the step's record names it                   |
| `turn`    | yes      | The turn's place in the debate: 1 and 2 are the openings          |
| `agent`   | yes      | The agent that spoke                                             |
| `verdict` | no       | The verdict of the turn's position; absent for a turn with none  |

A turn that a nudge follows appends one record per harness turn. `priory costs` and spend caps
count these records, and not the debate step's own.

## `factory.run.trusted`

What the daemon did about a harness's or direnv's folder trust in one path of a run, when the
factory sets `trust_worktrees`. A run gets one record for each path before it starts, one more
when its harness asks anyway, and one when direnv fails before its harness starts. See [sessions.md](daemon/sessions.md#with-trust_worktrees).

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-000000000006",
  "source": "factory://example/desktop",
  "type": "factory.run.trusted",
  "time": "2026-11-23T10:04:12Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "github:example/app#42",
    "run": "0196f1b0-0000-7000-8000-000000000002",
    "machine": "desktop",
    "harness": "claude-code",
    "path": "/home/a/.local/state/priory/worktrees/example/app/42",
    "file": "/home/a/.claude.json",
    "outcome": "granted"
  }
}
```

| Field     | Required | Meaning |
| --------- | -------- | ------- |
| `harness` | yes      | `claude-code`, `codex`, or `direnv` for direnv's trust in a worktree's checked `.envrc` |
| `path`    | yes      | The worktree or clone the trust covers |
| `file`    | no       | The config the daemon wrote or read: Claude's `.claude.json`, `$CODEX_HOME/priory-trust.config.toml`, or the run's `direnv/<session>/direnv.toml` in Priory home |
| `outcome` | yes      | `granted` when the daemon wrote it, `present` when it was already there, `refused` when it wrote nothing, `missed` when the harness asked anyway, `failed` when direnv failed before the harness started, which then started without the `.envrc` |
| `why`     | no       | For `refused`: why, such as a path outside Priory home, a repository the factory does not name, or an `.envrc` that a person denied, is untracked, differs from the commit the task started from and from the default branch, or could not be checked because a git command failed, with its exit code. For `missed`: `the grant was gone from the config` when the harness overwrote it. For `failed`: direnv's error, redacted, such as `direnv: error .envrc is blocked` |
| `version` | no       | For `missed`: the first line of the harness's `--version` |

## `factory.run.attach_failed`

The factory's `[runs] attach` command did not open a run of a step with `attach = true`. The run
goes on. See [sessions.md](daemon/sessions.md#opening-a-run-when-it-starts).

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-000000000007",
  "source": "factory://example/desktop",
  "type": "factory.run.attach_failed",
  "time": "2026-11-24T07:00:04Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "task": "0196f1b0-0000-7000-8000-000000000001",
    "key": "schedule:nightly@2026-11-24T06:00:00Z",
    "run": "0196f1b0-0000-7000-8000-000000000002",
    "step": "implement",
    "machine": "desktop",
    "reason": "exit 1",
    "output": "attach: hc is not on PATH; install herdr-core or fix the daemon's PATH"
  }
}
```

| Field    | Required | Meaning |
| -------- | -------- | ------- |
| `step`   | yes      | The step whose run did not open |
| `reason` | yes      | `exit <code>`, `killed by a signal`, `timed out after 30s`, or why the command could not start |
| `output` | no       | The tail of what the command printed, at most 2000 bytes, with the factory's secrets hidden |

## `factory.task.repaired`

A factory's attempt to repair a task whose agent failed. It shares `factory`, `factory_revision`,
`workflow`, `task`, `key`, `generation`, `step` and `machine` with `factory.step.ended`, and `run`
names the run that failed.

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-000000000004",
  "source": "factory://example/desktop",
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
    "machine": "desktop",
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

## `factory.worktree.recovered`

A start found the task's worktree off its branch, on a detached HEAD or on another branch, and put
it back on the task's branch. It shares `factory`, `factory_revision`, `workflow`, `task`, `key`,
`generation`, `step` and `machine` with `factory.step.ended`. See
[home.md](daemon/home.md#worktrees) for when the daemon does this and when it blocks instead.

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-000000000009",
  "source": "factory://example/desktop",
  "type": "factory.worktree.recovered",
  "time": "2026-11-23T09:12:40Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "234",
    "key": "github:example/app#20",
    "generation": 1,
    "step": "implement",
    "machine": "desktop",
    "worktree": "/home/me/.local/share/priory/worktrees/example/app/234",
    "head": "4b1c9e0d2f7a8e6b5c3d1f0a9e8b7c6d5e4f3a2b",
    "branch": "priory/234",
    "kept_branch": "priory/234-kept-4b1c9e0",
    "kept_commits": 1
  }
}
```

| Field          | Required | Meaning                                                                  |
| -------------- | -------- | ------------------------------------------------------------------------ |
| `worktree`     | yes      | The worktree's folder                                                    |
| `head`         | yes      | The commit the worktree had checked out                                  |
| `from_branch`  | no       | The branch the worktree was on; absent when its HEAD was detached        |
| `branch`       | yes      | The task's branch the worktree went back to                              |
| `kept_branch`  | no       | The branch Priory made to keep the commits of a detached HEAD that no remote holds |
| `kept_commits` | yes      | How many commits of the old HEAD no remote holds, as is or rebased. `kept_branch` keeps them, or `from_branch` when the HEAD was on a branch |

## `factory.worktree.reclaimed`

The sweep deleted one build output folder in an idle task's worktree, because free disk was under
`[sweep] reclaim`. It shares `factory`, `factory_revision`, `workflow`, `task`, `key` and `machine`
with `factory.step.ended`. See [sweep.md](daemon/sweep.md#reclaiming-build-output).

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-00000000000a",
  "source": "factory://example/desktop",
  "type": "factory.worktree.reclaimed",
  "time": "2026-11-23T19:30:02.412Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "304",
    "key": "github:example/app#20",
    "machine": "desktop",
    "path": "/home/me/.local/share/priory/worktrees/example/app/304/target",
    "bytes": 6979321856
  }
}
```

| Field   | Required | Meaning                                                    |
| ------- | -------- | ---------------------------------------------------------- |
| `path`  | yes      | The folder the sweep deleted                               |
| `bytes` | yes      | The disk its files took before the deletion, as `du` counts |

## `factory.run.preempted`

A run ended at its turn end to make room for the head of the queue, in a band of `[queue]
preempt` (see [room.md](daemon/room.md#urgent-wakes)). It shares `factory`, `factory_revision`,
`workflow`, `task`, `key`, `generation`, `step` and `machine` with `factory.step.ended`. `run`
names the stopped run and `task` its task.

```json
{
  "specversion": "1.0",
  "id": "0196f1d4-0000-7000-8000-000000000007",
  "source": "factory://example/xps13",
  "type": "factory.run.preempted",
  "time": "2026-11-22T10:04:12Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "7",
    "key": "github:example/app#42",
    "run": "31",
    "generation": 1,
    "step": "implement",
    "machine": "xps13",
    "band": "later",
    "head_task": "12",
    "head_band": "blocker",
    "waited_ms": 81000
  }
}
```

| Field       | Required | Meaning                                                              |
| ----------- | -------- | -------------------------------------------------------------------- |
| `band`      | yes      | The stopped task's band                                              |
| `head_task` | yes      | The task the run stopped for                                         |
| `head_band` | yes      | That task's band                                                     |
| `waited_ms` | no       | How long the head waited between the mark and the stop               |

## `factory.task.reopened`

Feedback on a done task's open pull request, issue, Linear issue or Slack thread, or a `priory say`
to it while its pull request is open, moved it back to a step, as its next generation.
It shares `factory`, `factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and
`machine` with `factory.step.ended`. `generation` is the new one and `step` the step it went back to.

```json
{
  "specversion": "1.0",
  "id": "0196f1d5-0000-7000-8000-000000000006",
  "source": "factory://example/desktop",
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
    "machine": "desktop",
    "resource": "github:example/app#57",
    "pull_request": "github:example/app#57"
  }
}
```

| Field          | Required | Meaning                                               |
| -------------- | -------- | ----------------------------------------------------- |
| `resource`     | yes      | The pull request, issue, Linear issue or Slack thread whose feedback reopened the task, or the open pull request of one a `priory say` reopened |
| `pull_request` | no       | The pull request whose feedback reopened the task, or the open pull request of one a `priory say` reopened; absent for other feedback |

## `factory.tool.denied`

The tool guard denied a run's irreversible call (see [guard.md](daemon/guard.md)). It shares
`factory`, `factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and `machine` with
`factory.step.ended`. `step` is the task's step when the run made the call.

```json
{
  "specversion": "1.0",
  "id": "0196f1d5-0000-7000-8000-000000000007",
  "source": "factory://example/desktop",
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
    "machine": "desktop",
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
  "source": "factory://example/desktop",
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
    "machine": "desktop",
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
[desk.md](daemon/desk.md#snooze)). The daemon writes one per snooze, at its next tick after the end. It
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
  "source": "factory://example/desktop",
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
    "machine": "desktop",
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
task, label and source event, judging the step that graded the task, as [Outcome
labels](daemon/trace.md#outcome-labels) describes. Its `data` also holds `factory`,
`factory_revision`, `workflow`, `task`, `key`, `generation` and `machine` as above, and `grade`, the
task's grade when it was labeled. A task no step graded has no `step_event`.

| Field        | Required | Meaning                                                              |
| ------------ | -------- | -------------------------------------------------------------------- |
| `label`      | yes      | One of the outcome labels, `marked-wrong` from `priory label`, or `scored` from a `held` scorer |
| `evidence`   | yes      | The event's URL, the human step and its answer, or a mark's URL      |
| `happened`   | no       | When the event the label reads happened. The record's `time` is when Priory wrote it. A record written before this field has none. For `scored`, the moment the scorer's `after` names |
| `scorer`     | for `scored` | The `held` scorer that graded the task |
| `kind`       | for `scored` | `held` |
| `verdict`    | for `scored` | `pass` or `fail`. The `evidence` is its reason. See [Held scorers](daemon/score.md#held-scorers) |

```json
{
  "specversion": "1.0",
  "id": "0196f1d0-0000-7000-8000-000000000003",
  "source": "factory://example/desktop",
  "type": "factory.step.labeled",
  "time": "2026-11-18T09:02:11Z",
  "data": {
    "step_event": "0196f1c2-7a3e-7c41-9b2d-5e8f0a1b2c3d",
    "label": "reverted",
    "evidence": "https://github.com/example/app/pull/57",
    "happened": "2026-11-18T08:47:30Z"
  }
}
```

## `factory.step.unlabeled`

A person ran `priory label TASK wrong --clear`. It withdraws every `factory.step.labeled` record of
the task and label written before it, whenever their `happened` lies, so `priory report risk` reads
none of them. A record of the same label after it counts again. Its `time` is when the person
cleared the mark. Its `data` holds `factory`, `factory_revision`, `workflow`, `task`, `key`,
`generation` and `machine` as above.

| Field   | Required | Meaning                                  |
| ------- | -------- | ---------------------------------------- |
| `label` | yes      | The label withdrawn: `marked-wrong`      |

```json
{
  "specversion": "1.0",
  "id": "0196f1d0-0000-7000-8000-000000000004",
  "source": "factory://example/desktop",
  "type": "factory.step.unlabeled",
  "time": "2026-11-19T10:15:00Z",
  "data": {
    "workflow": "implement",
    "task": "27",
    "label": "marked-wrong"
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
  "source": "factory://example/desktop",
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
    "machine": "desktop",
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

## `factory.terminal.taken`, `factory.terminal.returned`, `factory.terminal.keys` and `factory.terminal.attach_issued`

A person took a run's keyboard, a grant of it ended, a text or key went to its pane under one, or
`priory attach` issued a command that can type (see
[A run's keyboard](daemon/serve.md#a-runs-keyboard) and
[sessions.md](daemon/sessions.md#attaching-to-a-run)). Each shares `factory`, `factory_revision`,
`workflow`, `task`, `key`, `generation`, `step` and `machine` with `factory.step.ended`. No record
holds the text typed or the grant's secret.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000009",
  "source": "factory://example/desktop",
  "type": "factory.terminal.keys",
  "time": "2026-11-21T10:05:02Z",
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
    "machine": "desktop",
    "grant": "3",
    "request": "r7",
    "input": "text",
    "length": 12,
    "outcome": "sent"
  }
}
```

| Field      | Record                 | Required | Meaning |
| ---------- | ---------------------- | -------- | ------- |
| `run`      | all                    | yes      | The run whose pane it concerns |
| `grant`    | taken, returned, keys  | yes      | The grant's public id |
| `label`    | taken                  | yes      | The first 64 bytes of the taking page's `User-Agent`. Anyone can set it |
| `ends`     | taken                  | yes      | When the grant ends unless something ends it first |
| `reason`   | returned               | yes      | `expired`, `returned`, `replaced`, `run ended` or `terminal not "type"` |
| `request`  | keys                   | yes      | The request id the page picked |
| `input`    | keys                   | yes      | `text`, or the key's name, such as `Escape` |
| `length`   | keys                   | no       | The text's length in characters, for `text` only |
| `outcome`  | keys                   | yes      | `sent`, `failed`, or `uncertain` when the daemon stopped between storing the request and its outcome, or tmux timed out |
| `error`    | keys                   | no       | Why a `failed` or timed out request did not send |
| `printed`  | attach_issued          | yes      | Whether attach printed the command rather than ran it |

## `factory.run.scored`

A scorer graded one finished run: a run whose turn ended with a result or an ask (see
[score.md](daemon/score.md)). It shares `factory`, `factory_revision`, `workflow`, `task`, `key`,
`generation`, `step` and `machine` with `factory.step.ended`. `step` and `generation` are the run's,
and `time` is when the score was recorded. Scoring a run again appends another record.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000009",
  "source": "factory://example/desktop",
  "type": "factory.run.scored",
  "time": "2026-11-21T11:20:02Z",
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
    "machine": "desktop",
    "scorer": "gate",
    "kind": "command",
    "verdict": "pass",
    "reason": "the tests pass"
  }
}
```

| Field     | Required | Meaning                                                              |
| --------- | -------- | -------------------------------------------------------------------- |
| `run`     | yes      | The run the scorer graded                                            |
| `scorer`  | yes      | The scorer's name                                                    |
| `kind`    | yes      | How it answers, such as `outcome`, `command` or `ask-shape`          |
| `verdict` | yes      | `pass`, `fail`, `not applicable` or `error`                          |
| `reason`  | yes      | One line of why                                                      |
| `model`     | for a judge | The model the judge called                                        |
| `agreement` | for a judge | The share of labeled cases its calibration got right              |
| `cases`     | for a judge | The number of labeled cases its calibration ran                   |
| `gate`      | for a gate  | The outcome a step's `gate`, the factory's `critic` or its `pre_review` graded before it was stored. Absent for `priory score` |
| `evaluated` | for an intent scorer | Whether its decision model answered. A failed call is `false`, never an `UNKNOWN` answer |
| `model`     | for an intent scorer that answered | The model its decision model named |
| `answers`   | for an intent scorer that answered | Each question's answer, `YES`, `NO` or `UNKNOWN`, by name: `intent`, then `noop-1` and on |
| `input_tokens` | for an intent scorer that answered | The tokens the model reported, or `"unavailable"` when it reported none, never 0 |
| `noop_case` | for an intent scorer | The trigger's no-op case the model answered `YES` for, if any |

## `factory.scorer.calibrated`

`priory score --calibrate` ran a judge scorer on every one of its labeled cases (see
[score.md](daemon/score.md#calibration)). `priory score` and `priory bench` read the latest record
whose `scorer`, `model`, `prompt_sha256` and `cases_sha256` match the judge as it is now. A
calibration that the budget cut short appends no record.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-00000000000b",
  "source": "factory://example/desktop",
  "type": "factory.scorer.calibrated",
  "time": "2026-11-22T10:04:41Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "machine": "desktop",
    "scorer": "does-what-was-asked",
    "model": "opus",
    "prompt_sha256": "5d41402abc4b2a76b9719d911017c592ae0b6c5c3b4c2d1e8f0a9b8c7d6e5f40",
    "cases_sha256": "7d793037a0760186574b0282f2f435e7aa1c5b2a5e4d3c2b1a0f9e8d7c6b5a49",
    "cases": 10,
    "agreed": 9,
    "agreement": 0.9,
    "needs": 0.8,
    "spent": 0.21
  }
}
```

| Field           | Required | Meaning                                                           |
| --------------- | -------- | ----------------------------------------------------------------- |
| `scorer`        | yes      | The judge scorer's name                                           |
| `model`         | yes      | The model it called                                               |
| `prompt_sha256` | yes      | The SHA-256 of its prompt file                                    |
| `cases_sha256`  | yes      | The SHA-256 of its evaluation file                                |
| `cases`         | yes      | The labeled cases it ran                                          |
| `agreed`        | yes      | The cases whose verdict equals the person's label                 |
| `agreement`     | yes      | `agreed` over `cases`                                             |
| `needs`         | yes      | The scorer's `agreement` when it was calibrated                   |
| `spent`         | yes      | What the calls cost, in dollars                                   |
| `factory_revision` | no    | The factory folder's Git commit, when it is in a repository       |

## `factory.decision.calibrated`

`priory calibrate triage` asked the triage decision model every labeled case, and each call
answered with the same model (see [decisions.md](daemon/decisions.md#calibration)). The daemon
reads the latest record whose `point`, `run_sha256`, `cases_sha256` and `threshold` match the
factory as it is now. A calibration with a failed call, or answered by two models, appends no
record.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-00000000000c",
  "source": "factory://example/desktop",
  "type": "factory.decision.calibrated",
  "time": "2026-11-22T10:04:41Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "point": "triage",
    "model": "jev-1.14.0",
    "run_sha256": "86ae35d58a6aa3b5742df94ef9d7162219f0106a911ae1954c1f0604aaec805d",
    "cases_sha256": "9d878db9853640b8319a3081477425ecf0c93bab7313bf31da408990a86a738c",
    "threshold": 0.9,
    "cases": 10,
    "agreed": 10,
    "agreement": 1.0,
    "needs": 0.9,
    "misses": []
  }
}
```

| Field              | Required | Meaning                                                                 |
| ------------------ | -------- | ----------------------------------------------------------------------- |
| `point`            | yes      | The decision point, `triage`                                            |
| `model`            | yes      | The model that answered every case, as the answers named it             |
| `run_sha256`       | yes      | The SHA-256 of the point's `run`, each file it names in the factory's folder, and Priory's question |
| `cases_sha256`     | yes      | The SHA-256 of its evaluation file                                      |
| `threshold`        | yes      | The point's `threshold` when it was calibrated                          |
| `cases`            | yes      | The labeled cases it asked                                              |
| `agreed`           | yes      | The cases whose choice equals the person's label                        |
| `agreement`        | yes      | `agreed` over `cases`                                                   |
| `needs`            | yes      | The point's `agreement` when it was calibrated                          |
| `misses`           | yes      | The cases labeled other than `nothing-to-do` that the model sorted `nothing-to-do` at the threshold. One is enough to leave the point not calibrated |
| `factory_revision` | no       | The factory folder's Git commit, when it is in a repository             |

## `factory.decision.answered`

The daemon considered asking the triage decision model about a feedback item that a task owns or
subscribes to (see [decisions.md](daemon/decisions.md)). A factory with no `[decisions.triage]`
appends none. The record holds no text of the item: the inbox holds the event.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-00000000000d",
  "source": "factory://example/desktop",
  "type": "factory.decision.answered",
  "time": "2026-11-22T10:06:02Z",
  "datacontenttype": "application/json",
  "data": {
    "point": "triage",
    "item": "https://github.com/owner/name/pull/57#issuecomment-701",
    "model": "jev-1.14.0",
    "choice": "nothing-to-do",
    "probability": 0.97,
    "input_tokens": 212,
    "millis": 340,
    "called": true,
    "acted": true
  }
}
```

| Field          | Required | Meaning                                                                      |
| -------------- | -------- | ---------------------------------------------------------------------------- |
| `point`        | yes      | The decision point, `triage`                                                 |
| `item`         | yes      | The feedback item's URL                                                      |
| `acted`        | yes      | Whether Priory settled the item on the answer                                |
| `called`       | yes      | Whether Priory ran the command. `daily_calls` counts these                   |
| `millis`       | yes      | How long the check and the call took                                         |
| `why`          | no       | Why Priory did not act, such as `under the threshold 0.9` or `it ran past 5s` |
| `model`        | no       | The model that answered, when a call answered                                |
| `choice`       | no       | Its choice                                                                   |
| `probability`  | no       | The probability it gave its choice                                           |
| `input_tokens` | no       | The input tokens the answer reported                                         |

## `factory.ask.auto_answered`

The daemon answered an agent's ask with its recommendation, since the confidence the agent stated
met its task's `auto_answer` level (see [waits.md](daemon/waits.md#answered-by-auto)). It shares
`factory`, `factory_revision`, `workflow`, `task`, `key`, `generation`, `step` and `machine` with
`factory.step.ended`, and `run` names the run that asked.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-00000000000e",
  "source": "factory://example/desktop",
  "type": "factory.ask.auto_answered",
  "time": "2026-11-22T10:08:41Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "14",
    "key": "github:example/app#42",
    "run": "31",
    "generation": 1,
    "step": "implement",
    "machine": "desktop",
    "ask": "14",
    "revision": 1,
    "option": "document --format",
    "confidence": "high",
    "rationale": "the flag duplicates --format",
    "level": "high"
  }
}
```

| Field        | Required | Meaning                                                          |
| ------------ | -------- | ---------------------------------------------------------------- |
| `ask`        | yes      | The ask the daemon answered                                      |
| `revision`   | yes      | The revision it answered                                         |
| `option`     | yes      | The recommended option, which became the answer                  |
| `confidence` | yes      | The confidence the agent stated: `high` or `medium`              |
| `rationale`  | yes      | The agent's one-line reason                                      |
| `level`      | yes      | The task's `auto_answer` level when the ask was stored           |

## `factory.ask.autopilot_answered`

The daemon answered an ask while a person had autopilot on (see
[waits.md](daemon/waits.md#autopilot)). It shares `factory`, `factory_revision`, `workflow`, `task`,
`key`, `generation`, `step` and `machine` with `factory.step.ended`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000010",
  "source": "factory://example/desktop",
  "type": "factory.ask.autopilot_answered",
  "time": "2026-11-25T01:12:09Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "14",
    "key": "github:example/app#42",
    "run": "31",
    "generation": 1,
    "step": "implement",
    "machine": "desktop",
    "ask": "14",
    "revision": 1,
    "option": "document --format",
    "why": "its confidence, high, meets high, and a person decides again before any irreversible step",
    "level": "high",
    "window": 3,
    "confidence": "high",
    "rationale": "the flag duplicates --format"
  }
}
```

| Field                     | Required | Meaning                                                |
| ------------------------- | -------- | ------------------------------------------------------ |
| `ask`                     | yes      | The ask the daemon answered                            |
| `revision`                | yes      | The revision it answered                               |
| `option`                  | yes      | The option that became the answer                      |
| `why`                     | yes      | Why the ask was eligible                               |
| `level`                   | yes      | The level autopilot was on at                          |
| `window`                  | yes      | The window, as `priory autopilot report` takes it      |
| `run`                     | no       | The run that asked. A plan's ask has none              |
| `confidence`, `rationale` | no       | What the agent stated. A plan's ask has neither        |
| `grade`                   | no       | A plan's grade, `green`                                |

## `factory.ask.autopilot_replaced`

A person's answer took the place of autopilot's before the agent got it. It shares the same fields
with `factory.step.ended` as `factory.ask.autopilot_answered`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-000000000011",
  "source": "factory://example/desktop",
  "type": "factory.ask.autopilot_replaced",
  "time": "2026-11-25T01:12:40Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "14",
    "key": "github:example/app#42",
    "generation": 1,
    "step": "implement",
    "machine": "desktop",
    "ask": "14",
    "revision": 1,
    "option": "drop --format",
    "replaced": "document --format",
    "by": "cli"
  }
}
```

| Field      | Required | Meaning                               |
| ---------- | -------- | ------------------------------------- |
| `ask`      | yes      | The ask the person answered           |
| `revision` | yes      | The revision both answered            |
| `option`   | yes      | The person's answer                   |
| `replaced` | yes      | The answer autopilot gave             |
| `by`       | yes      | Who answered, such as `cli` or `serve` |

## `factory.ask.session_answered` and `factory.ask.session_dismissed`

A person's own session answered or dismissed an ask through the `answer` or `dismiss` tool of
[`priory mcp --outside`](daemon/tools.md#answers-from-the-session). The record names the session,
never a person. An ask of a task shares `factory`, `factory_revision`, `workflow`, `task`, `key`,
`generation`, `step` and `machine` with `factory.step.ended`. An outside ask has no task, so its
record has no `workflow`, `task`, `generation` or `step`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-00000000000f",
  "source": "factory://example/desktop",
  "type": "factory.ask.session_answered",
  "time": "2026-11-22T10:12:03Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "14",
    "key": "github:example/app#42",
    "generation": 1,
    "step": "implement",
    "machine": "desktop",
    "ask": "14",
    "revision": 1,
    "session": "t9 8f3c2a10",
    "option": "sqlite"
  }
}
```

| Field      | Required | Meaning                                                                         |
| ---------- | -------- | ------------------------------------------------------------------------------- |
| `ask`      | yes      | The ask: its number, or an outside ask's name                                   |
| `revision` | yes      | The revision the session decided                                                |
| `session`  | yes      | The session: its hc terminal, then a space and its Claude session, or `outside` |
| `option`   | answered | The answer. A `factory.ask.session_dismissed` record has none                   |

## `factory.bench.compared`

A comparison of two commits of the factory on a benchmark ended (see
[bench.md](daemon/bench.md#compare)). `priory bench` reads these records to number a comparison
and to mark a repeat. `factory` is the head commit's factory name, and `source` names the machine.

```json
{
  "specversion": "1.0",
  "id": "0196f1d6-0000-7000-8000-00000000000a",
  "source": "factory://example/desktop",
  "type": "factory.bench.compared",
  "time": "2026-11-22T15:04:11Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "benchmark": "implement-november",
    "base": "999daf0c8e1f6a1b2c3d4e5f60718293a4b5c6d7",
    "head": "4c1e2b7f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d",
    "number": 1,
    "complete": true,
    "repeat": false,
    "verdict": "better",
    "primary": "gate",
    "cases": 12,
    "runs": 3,
    "spent": 31.2,
    "budget": 40.0,
    "machine": "desktop",
    "pull_request": "owner/factory#12"
  }
}
```

| Field       | Required | Meaning                                                              |
| ----------- | -------- | -------------------------------------------------------------------- |
| `benchmark` | yes      | The benchmark file's name                                            |
| `base`      | yes      | The base commit, in full                                             |
| `head`      | yes      | The head commit, in full                                             |
| `number`    | yes      | This comparison's number among those of the same commits and benchmark on this machine, or with `--post` among those of the same pull request and benchmark |
| `complete`  | yes      | Whether every pair ran                                               |
| `repeat`    | yes      | Whether an earlier complete comparison of the same commits and benchmark gave the verdict |
| `verdict`   | no       | `better`, `worse`, `no clear difference` or `too few cases`. Null when incomplete or a repeat |
| `primary`   | yes      | The primary scorer                                                   |
| `cases`     | yes      | The cases compared, without those a broken scorer dropped            |
| `runs`      | yes      | The pairs each case was to get                                       |
| `spent`     | yes      | What the replays spent, in dollars                                   |
| `budget`    | yes      | The budget, in dollars                                               |
| `machine`   | no       | The machine's hostname                                               |
| `pull_request` | no    | The pull request `--post` named, `owner/name#12`. Null without `--post` |

## `factory.consult.asked` and `factory.consult.reported`

A run asked an investigator one question, and the report or the reason none came was queued for
its task (see [tools.md](daemon/tools.md#consulting-an-investigator)). Both share `factory`,
`factory_revision`, `workflow`, `task`, `key`, `generation`, `lane` and `machine` with
`factory.step.ended`, all of the asking task. The question and the report stay in the state file,
which `priory task <id> --json` reads, and never enter the trace.

```json
{
  "specversion": "1.0",
  "id": "0196f1d5-0000-7000-8000-000000000011",
  "source": "factory://example/desktop",
  "type": "factory.consult.reported",
  "time": "2026-11-22T10:04:51Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "workflow": "implement",
    "task": "12",
    "key": "github:example/app#42",
    "generation": 1,
    "machine": "desktop",
    "consult": 4,
    "investigation": "13",
    "outcome": "reported",
    "seconds": 162.4,
    "report_length": 412,
    "model": "claude-haiku-5-5",
    "tokens": { "input": 20412, "output": 610, "cache_read": 81220, "cache_write": 9120 },
    "cost_usd": 0.0413
  }
}
```

`factory.consult.asked`, when the consult is stored:

| Field             | Required | Meaning                                         |
| ----------------- | -------- | ----------------------------------------------- |
| `run`             | yes      | The run that asked                              |
| `step`            | yes      | The asking task's step                          |
| `agent`           | yes      | The asking run's agent                          |
| `investigator`    | yes      | The investigator asked                          |
| `consult`         | yes      | The consult's id                                |
| `question_length` | yes      | The question's length, in characters            |

`factory.consult.reported`, when the report or a line for no report is queued for the asking task,
or would be for a task that moved on:

| Field           | Required | Meaning |
| --------------- | -------- | ------- |
| `consult`       | yes      | The consult's id |
| `investigation` | no       | The id of the investigator's task |
| `outcome`       | yes      | `reported`, `unknown`, `deadline`, `cap` or `failed` |
| `seconds`       | yes      | From the question to this record |
| `report_length` | yes      | The report's length, in characters; 0 with no report |
| `tokens`, `model` | no     | What the investigator's run spent so far, since a run ended by its deadline or cap writes no `factory.step.ended` |
| `cost_usd`      | no       | `tokens` priced with `prices.toml` |

## `factory.pattern.matched`

One piece of evidence of a failure pattern: a task that failed, blocked, was repaired or was marked
wrong, which `priory patterns find` recorded (see [patterns.md](daemon/patterns.md)). A pattern is
the records of its `pattern` id. Its `data` also holds `factory`, `factory_revision` and `machine`.

```json
{
  "specversion": "1.0",
  "id": "0196f1d7-0000-7000-8000-00000000000b",
  "source": "factory://example/desktop",
  "type": "factory.pattern.matched",
  "time": "2026-11-22T02:45:03.120Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "pattern": "3fa2b1c0",
    "workflow": "implement",
    "step": "implement",
    "signature": "cargo test failed: # tests failed in #s",
    "text": "cargo test failed: 12 tests failed in 9s",
    "task": "27",
    "key": "github:example/app#42",
    "generation": 1,
    "kind": "failed",
    "happened": "2026-11-21T16:20:05Z",
    "source": "task 27 generation 1 failed at implement: cargo test failed: # tests failed in #s",
    "machine": "desktop"
  }
}
```

| Field        | Required | Meaning                                                              |
| ------------ | -------- | -------------------------------------------------------------------- |
| `pattern`    | yes      | The pattern's id: the first 8 hex digits of the SHA-256 of its workflow, step and signature |
| `workflow`, `step` | yes | Where the task stopped                                        |
| `signature`  | yes      | The text's shape, which groups the pattern                           |
| `text`       | yes      | The text's first line, with credential shapes redacted               |
| `task`, `key`, `generation` | yes | The task                                                |
| `kind`       | yes      | `failed`, `blocked`, `repaired` or `marked-wrong`                    |
| `happened`   | yes      | When the task stopped, was repaired or was marked                   |
| `source`     | yes      | What the evidence is, so `find` records it once: a stopped task's id, generation, kind, step and signature, or the id of the `factory.task.repaired` or `factory.step.labeled` record |

## `factory.pattern.closed`

A person ran `priory patterns close` or `priory patterns dismiss`. A closed pattern is open again
once a match recorded after this record happened in its second or later; a dismissed one stays
dismissed. Its `data` also holds `factory`, `factory_revision` and `machine`.

| Field       | Required | Meaning                                              |
| ----------- | -------- | ---------------------------------------------------- |
| `pattern`   | yes      | The pattern's id                                     |
| `workflow`, `step` | yes | The pattern's workflow and step                  |
| `tasks`     | yes      | How many tasks the pattern held when it was closed   |
| `dismissed` | yes      | `true` for `dismiss`, `false` for `close`            |

## `factory.config.reloaded` and `factory.config.refused`

The daemon loaded an edit of the factory's files, or refused one and kept the last good factory
(see [reload.md](daemon/reload.md)). Its `data` holds `factory`, `factory_revision` and `machine`:
after a reload, the new revision; after a refusal, the revision still live.

```json
{
  "specversion": "1.0",
  "id": "0196f1d7-0000-7000-8000-00000000000c",
  "source": "factory://example/desktop",
  "type": "factory.config.refused",
  "time": "2026-11-23T09:12:40.512Z",
  "datacontenttype": "application/json",
  "data": {
    "factory": "example",
    "factory_revision": "999daf0",
    "machine": "desktop",
    "problems": ["/home/me/factory/factory.toml:14: `[links]` `(unclosed` is not a regex: ..."]
  }
}
```

| Field          | Required | Meaning                                                            |
| -------------- | -------- | ------------------------------------------------------------------ |
| `restart_only` | reloaded | The keys the edit changed that wait for a restart, such as `[serve]`; empty for none |
| `problems`     | refused  | Each problem, as `priory check` prints it, or each task the edit would strand |

## `factory.upgrade.installed`, `factory.upgrade.rolled_back`, `factory.upgrade.put_off` and `factory.upgrade.refused`

The daemon installed a newer release by itself, went back to the old one, put one off for a fault that is
not its own, or refused one before it changed anything (see [upgrades.md](daemon/upgrades.md)). Its `data` also holds `factory`,
`factory_revision` and `machine`.

| Field | Required | Meaning |
| ----- | -------- | ------- |
| `op` | installed, rolled back, put off | The upgrade's id, one record each |
| `from` | yes | The version that ran before |
| `to` | installed, rolled back, put off | The version it installed or tries again later |
| `schema_from`, `schema_to` | installed, rolled back | The state's schema before, and the one the new version uses |
| `kept` | installed, rolled back | The old binary |
| `backups` | installed, rolled back | The copies of the state and the log, without those a rollback used |
| `reason` | rolled back, put off, refused | Why. A refusal that names one release holds its version in `to` |

## Spans

A factory that exports OpenTelemetry spans (Priory does: see
[OpenTelemetry export](daemon/trace.md#opentelemetry-export)) maps one task to one trace, with a root span
per generation, and `factory.step.ended` records to spans below it:

- A root span per task generation, from its creation to its end, which no record carries. The
  exporter reads it from the task, so a task needs no record to end.
- A span per run, child of the root: from a `factory.step.ended` record's `run`, or, for a run that
  ended with no step, from the run itself.
- A span per step, child of its run span, or of the root when it has no run.
- A span per `factory.config.reloaded` or `factory.config.refused` record, alone in a trace of its
  own, starting and ending at the record's time.

## Rules

- `id` is a UUID, stable across retries, so a reader drops duplicates by `id`.
- Readers ignore fields and types they do not know.
- A factory may add fields under its own prefix, such as `priory_` or `bowtie_`.
- Report counts next to every rate, and leave a rate unknown when no record supports it.
