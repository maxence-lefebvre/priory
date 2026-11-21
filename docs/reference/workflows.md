# Workflows

A workflow is the typed steps a task goes through. Each file `workflows/<name>.toml`, `.yaml` or
`.yml` defines one workflow, named after the file. A trigger or `priory request` names the
workflow its tasks start.

A task starts at `start` and follows `next` from step to step. Reaching `done` ends it as done,
and reaching `stop` ends it as dropped. The [PRD](../prd.md#workflows) explains deciders, effects
and the approval rule. `priory draw` prints a workflow as a diagram, and `priory check --explain`
lists what gates each irreversible step.

Schema: [`config/workflow.json`](../schemas/config/workflow.json).

## Top-level keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `start` | string | yes | | The id of the first step | `"plan"` |
| `band` | string | no | the last band | The band of its tasks when neither their request nor their trigger sets one | `"today"` |
| `reopen` | string | no | the step that opened the pull request | The model step a done task goes back to when its open pull request gets feedback (see [routing.md](../daemon/routing.md#feedback-reopens-a-done-task)) | `"implement"` |
| `max_prs` | integer, at least 1 | no | no cap | The most pull requests a task's stack may hold. `open-pr` fails with more branches (see [tools.md](../daemon/tools.md#code-steps)) | `3` |
| `guard` | table | no | none allowed | `allow`: the guards whose calls every step of the workflow may make with no approval, built-in or from `[guard]`. It cannot name `gh-pr-merge` or `gh-pr-approve`. See [guard.md](../daemon/guard.md) | `{ allow = ["git-push-default"] }` |
| `steps` | array of tables | yes | | The steps, in any order | see below |

## Step keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `id` | string | yes | | The step's id, unique in the workflow. `done` and `stop` are reserved | `"implement"` |
| `decides` | `code`, `model` or `human` | yes | | Who decides the outcome: a built-in action, an agent's run or a person | `"model"` |
| `effect` | `none`, `read`, `reversible` or `irreversible` | yes | | What the step may change | `"reversible"` |
| `agent` | string | model steps | | The agent that runs the step, by folder name under `agents/` | `"implementer"` |
| `prompt` | string | model steps | | What the step tells its agent | `"Implement the plan."` |
| `timeout` | duration | no | none, or `command_timeout` for a `run` action | How long one turn of a model step, or a `run` action's command, may run before Priory ends it | `"30m"` |
| `options` | array of strings | human steps | `[]` | The answers a person picks from | `["open", "stop"]` |
| `approves` | array of strings | no | `[]` | Human steps only: the irreversible steps this answer approves | `["open-pr"]` |
| `command` | table | no | none | Human steps only: a command an answer runs once. `argv`, the program and its arguments; `target`, `worktree` or a name in [`[asks.targets]`](factory.md#asks), default `worktree`; `runs`, the options that run it. See [Runbooks](#runbooks) | `{ argv = ["cargo", "test"], target = "repo", runs = ["run"] }` |
| `action` | string | code steps | | The built-in action: `open-pr`, `open-issue` or `run` | `"open-pr"` |
| `run` | string | `run` actions | | The command a `run` action runs with `sh -c` | `"scripts/poll.sh"` |
| `next` | table of strings | yes | | Each outcome and where it leads: a step id, `done` or `stop` | `{ passed = "done", failed = "stop" }` |
| `by` | table of `code`, `model`, `human` or `asleep` | no | the step's `decides` | Who decides an outcome's edge, where it differs from the step's decider | `{ passed = "asleep" }` |
| `proof` | table of `gate-passed` | no | none: every outcome is the agent's word | Model steps only. The proof the daemon checks before it accepts an outcome, by outcome. `gate-passed`: the factory's [`gate`](factory.md#top-level-keys) passed at the head of the task's branch. See Proofs in [tools.md](../daemon/tools.md#proofs) | `{ passed = "gate-passed" }` |
| `autonomous` | string | no | | Why an irreversible step may run without a person's approval | `"it only labels issues"` |
| `guard` | table | no | none allowed | Irreversible model steps only. `allow`: the guards whose calls the step's agent may make while the step is approved at the task's commit or `autonomous`. An `autonomous` step cannot name `gh-pr-merge` or `gh-pr-approve`. See [guard.md](../daemon/guard.md) | `{ allow = ["gh-pr-comment"] }` |

The outcomes of a step come from:

| `decides` | Its outcome is |
| --------- | -------------- |
| `model` | the `outcome` field of the run's `result` |
| `human` | the option the person picks; every option needs a `next` |
| `code` | the action's status, such as `opened` or `failed`. A `run` action's is the last line of its stdout when `next` names it, else `ok` on exit 0 and `failed` otherwise |

Each edge, an outcome and its target, has a `by`: who decides that the task follows it. It defaults
to the step's `decides`. Set `by` for an outcome when that is wrong, such as `asleep` for an outcome
that comes from a wait on CI rather than from the model. `priory draw` colors each edge by its `by`,
and `priory check --explain` counts only a `human` edge as a person's decision.

A model step outcome of `red`, `yellow` or `green` also grades the task: how risky or uncertain its
work is. The task keeps the latest grade and its reason. See Grades in
[tools.md](../daemon/tools.md#grades).

## Example

<!-- example: factory/workflows/implement.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/workflow.json
start = "plan"
band = "today"                 # when neither the request nor the trigger sets one

[[steps]]
id = "plan"
decides = "model"              # an agent's run decides the outcome
agent = "implementer"
prompt = "Plan the change. Grade it red, yellow or green."
effect = "reversible"
timeout = "30m"                # one turn may run this long
next = { red = "approve-plan", yellow = "implement", green = "implement" }

[[steps]]
id = "approve-plan"
decides = "human"              # a person picks one of the options
effect = "none"
options = ["approved", "rejected"]
next = { approved = "implement", rejected = "stop" }

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the plan and run the tests."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }
proof = { passed = "gate-passed" }  # the daemon checks the gate passed at the head

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]         # the irreversible step this answer approves
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"               # a built-in action decides the outcome
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
```

A `run` step polls a script with no agent. See Code steps in [tools.md](../daemon/tools.md#code-steps)
for its folder, environment and restarts:

<!-- example: factory/workflows/poll.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/workflow.json
start = "poll"

[[steps]]
id = "poll"
decides = "code"
action = "run"                 # runs a command, with no agent
run = "scripts/poll.sh"        # in the factory folder, as the task has no worktree
effect = "read"
timeout = "2m"                 # else the factory's command_timeout
next = { found = "done", ok = "stop", failed = "stop" }  # found: the script's last line
```

## Runbooks

A runbook is a workflow made only of human steps, so no agent runs. Each step is an ask on the
desk, and the task moves by the option the person picks. A human step's `command` runs once when
the person picks an option in its `runs`, with the at-most-once and uncertain rules of an agent's
ask command (see [tools.md](../daemon/tools.md#commands-an-answer-runs)). The task moves by that
option once the command exits 0. A failed or uncertain command asks the step again, with the exit
or the uncertain outcome in the question. `examples/release-checklist/` is a runbook with a
`cargo test` step:

```toml
[[steps]]
id = "test"
decides = "human"
effect = "read"
options = ["run", "skip", "abort"]
command = { argv = ["cargo", "test"], target = "repo", runs = ["run"] }
next = { run = "tag", skip = "tag", abort = "stop" }
```

`priory add` or `priory request --workflow <runbook>` starts one, and the desk shows its current
step as an ask.

## Irreversible steps and their gates

`priory check --explain [DIR]` runs the checks, then prints four tab-separated tables:

- each irreversible step of each workflow with its gate: the human steps that approve it, or
  `autonomous:` and its reason;
- every path of steps from `start` to an irreversible step that follows no `human` edge;
- per agent, each guarded call it can make with no human decision (see
  [guard.md](../daemon/guard.md)): the guards a workflow it runs a model step of allows, the
  guards an `autonomous` irreversible model step it runs names in `guard.allow`, and `mcp-write` and `[guard]` tools for
  a Codex agent with MCP servers, since Codex may run MCP calls without the guard's hook;
- each outcome the daemon verifies before it accepts it, with its `proof`.

For the example above and a workflow `triage` whose autonomous `label` step follows `read`:

```text
workflow	step	gate
implement	open-pr	approve-pr
triage	label	autonomous: it only labels issues

workflow	path with no human decision
triage	read -> label

agent	guard	workflow	why

workflow	step	outcome	proof
implement	implement	passed	gate-passed
```

A workflow `docs` with `guard = { allow = ["git-push-default"] }` and a model step run by `writer`
adds the row `writer	git-push-default	docs	allowed by the workflow`.

`--json` prints the same rows as one object. `priory flows [DIR] --json` prints the same object with
each workflow's steps and edges added, as `GET /api/flows` serves it, and the Flows panels of the web
view and the Herdr adapter draw them:

```json
{
  "irreversible": [
    { "workflow": "implement", "step": "open-pr", "approved_by": ["approve-pr"], "autonomous": null },
    { "workflow": "triage", "step": "label", "approved_by": [], "autonomous": "it only labels issues" }
  ],
  "paths": [{ "workflow": "triage", "path": ["read", "label"] }],
  "tools": [],
  "proofs": [{ "workflow": "implement", "step": "implement", "outcome": "passed", "proof": "gate-passed" }]
}
```

When `factory.toml` sets `[sweep] agent` and no `workflows/sweep.toml` exists, the factory also gets
the built-in `sweep` workflow. See [sweep.md](../daemon/sweep.md).

## Errors from `priory check`

Errors about one step start with `` step `<id>`: `` and point at the step's line.

| Message | Cause |
| ------- | ----- |
| `` unknown field `<key>`, expected one of ... `` | A key the tables above do not list |
| `` unknown variant `<value>`, expected one of ... `` | A `decides` or `effect` value not listed above |
| `` step id `<id>` is reserved for the end of a workflow `` | A step is named `done` or `stop` |
| `` step `<id>` is defined twice `` | Two steps share an id |
| `` start `<id>` is not a step `` | `start` names no step |
| `` only a model step has an `agent` `` | `agent` on a human or code step. `prompt`, `options`, `approves` and `action` get the same error on the wrong kind of step |
| `` agent `<name>` does not exist `` | A model step names no agent under `agents/` |
| `` a model step needs an `agent` `` | A model step without `agent` |
| `` a model step needs a `prompt` `` | A model step without `prompt` |
| `` option `<option>` has no `next` `` | A human step's option leads nowhere |
| `` `next` has outcome `<outcome>`, which is not an option `` | A human step's `next` names an answer it does not offer |
| `` a code step needs an `action` `` | A code step without `action` |
| `` a `run` action needs a `run` command `` | `action = "run"` without `run` |
| `` only a `run` action has a `run` command `` | `run` on a code step with another action |
| `` only a code step has a `run` command `` | `run` on a model or human step |
| `` only a human step has a `command` `` | `command` on a model or code step |
| `command argv must start with a program` | A `command` with an empty `argv` or an empty first word |
| `` command target `<target>` is not worktree nor a name in [asks.targets] `` | A `command` names a target the factory lacks |
| `` command runs `<option>`, which is not an option `` | A `runs` entry the step does not offer |
| `` `next` is empty `` | A step with no outcome |
| `` `by` has outcome `<outcome>`, which is not in `next` `` | `by` names an outcome the step does not have |
| `` only a model step has a `proof` `` | `proof` on a human or code step |
| `` `proof` has outcome `<outcome>`, which is not in `next` `` | `proof` names an outcome the step does not have |
| `` `proof` names `gate-passed`, and factory.toml sets no `gate` `` | A step needs the gate, and the factory has none |
| `` outcome `<outcome>` leads to `<target>`, which is not a step, `done` or `stop` `` | A `next` target that does not exist |
| `no path from start reaches it` | No chain of `next` from `start` reaches the step |
| `` approves `<id>`, which is not an irreversible step `` | `approves` names a missing or reversible step |
| `` `autonomous` needs a reason `` | `autonomous` is empty |
| `irreversible, and a path from start reaches it with no human step that approves it` | An irreversible step without `autonomous` and without an approving human step on every path |
| `` band `<band>` is not one of <bands> `` | `band` is not a band of the factory |
| `` reopen `<id>` is not a model step `` | `reopen` names a missing step, or a human or code step |
| `"<text>" is not a duration such as 90s, 15m or 2h` | A bad `timeout` |
| ``invalid value: integer `0`, expected a nonzero usize`` | `max_prs` is 0 |
| `` `guard.allow` names `<name>`, which is no built-in guard nor one of `[guard]` in factory.toml `` | A workflow's or a step's `guard.allow` names a guard that does not exist |
| `` `guard.allow` names `<name>`, which only a person's approval grants, so only an irreversible step may name it `` | A workflow's `guard.allow` names `gh-pr-merge` or `gh-pr-approve` |
| `` step `<id>`: `guard.allow` names `<name>`, which only a person's approval grants, and the step is `autonomous` `` | An `autonomous` step's `guard.allow` names `gh-pr-merge` or `gh-pr-approve` |
| `` step `<id>`: `guard.allow` grants nothing on a step that is not an irreversible model step `` | `guard.allow` on a step that is not an irreversible model step |
