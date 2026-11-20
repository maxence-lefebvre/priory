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
| `action` | string | code steps | | The built-in action: `open-pr`, `open-issue` or `run` | `"open-pr"` |
| `run` | string | `run` actions | | The command a `run` action runs with `sh -c` | `"scripts/poll.sh"` |
| `next` | table of strings | yes | | Each outcome and where it leads: a step id, `done` or `stop` | `{ passed = "done", failed = "stop" }` |
| `by` | table of `code`, `model`, `human` or `asleep` | no | the step's `decides` | Who decides an outcome's edge, where it differs from the step's decider | `{ passed = "asleep" }` |
| `autonomous` | string | no | | Why an irreversible step may run without a person's approval | `"it only labels issues"` |

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

## Irreversible steps and their gates

`priory check --explain [DIR]` runs the checks, then prints two tab-separated tables:

- each irreversible step of each workflow with its gate: the human steps that approve it, or
  `autonomous:` and its reason;
- every path of steps from `start` to an irreversible step that follows no `human` edge.

For the example above and a workflow `triage` whose autonomous `label` step follows `read`:

```text
workflow	step	gate
implement	open-pr	approve-pr
triage	label	autonomous: it only labels issues

workflow	path with no human decision
triage	read -> label
```

`--json` prints the same rows as one object:

```json
{
  "irreversible": [
    { "workflow": "implement", "step": "open-pr", "approved_by": ["approve-pr"], "autonomous": null },
    { "workflow": "triage", "step": "label", "approved_by": [], "autonomous": "it only labels issues" }
  ],
  "paths": [{ "workflow": "triage", "path": ["read", "label"] }]
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
| `` `next` is empty `` | A step with no outcome |
| `` `by` has outcome `<outcome>`, which is not in `next` `` | `by` names an outcome the step does not have |
| `` outcome `<outcome>` leads to `<target>`, which is not a step, `done` or `stop` `` | A `next` target that does not exist |
| `no path from start reaches it` | No chain of `next` from `start` reaches the step |
| `` approves `<id>`, which is not an irreversible step `` | `approves` names a missing or reversible step |
| `` `autonomous` needs a reason `` | `autonomous` is empty |
| `irreversible, and a path from start reaches it with no human step that approves it` | An irreversible step without `autonomous` and without an approving human step on every path |
| `` band `<band>` is not one of <bands> `` | `band` is not a band of the factory |
| `` reopen `<id>` is not a model step `` | `reopen` names a missing step, or a human or code step |
| `"<text>" is not a duration such as 90s, 15m or 2h` | A bad `timeout` |
