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
| `guard` | table | no | none allowed | `allow`: the guards whose calls every step of the workflow may make with no approval, built-in or from `[guard]`. It cannot name `gh-pr-merge`, `gh-pr-approve`, `priory-person`, `connector-sign-in` or `unreadable`. See [guard.md](../daemon/guard.md) | `{ allow = ["git-push-default"] }` |
| `steps` | array of tables | yes | | The steps, in any order | see below |

## Step keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `id` | string | yes | | The step's id, unique in the workflow. `done` and `stop` are reserved | `"implement"` |
| `decides` | `code`, `model` or `human` | yes | | Who decides the outcome: a built-in action, an agent's run or a person | `"model"` |
| `effect` | `none`, `read`, `reversible` or `irreversible` | yes | | What the step may change | `"reversible"` |
| `agent` | string | model steps without `debate` | | The agent that runs the step, by folder name under `agents/` | `"implementer"` |
| `debate` | table | no | none | Model steps only, in place of `agent`: two agents who debate the step's prompt. `agents`, exactly two different agents in speaking order; `rounds`, the rounds after the openings, 1 to 5; `cap`, the most the debate may cost, such as `"$3.00"`, priced from `prices.toml`. See [Debate steps](#debate-steps) | `{ agents = ["architect", "skeptic"], rounds = 3, cap = "$3.00" }` |
| `prompt` | string | model steps | | What the step tells its agent | `"Implement the plan."` |
| `timeout` | duration | no | none, or `command_timeout` for a command | How long one turn of a model step, a `run` action's command, or the command a human step's answer runs, may run before Priory ends it | `"30m"` |
| `budget` | string | no | none | Model steps only. The dollars the task may spend at this step, as an amount above zero. At each turn end, Priory prices the step's tokens with `prices.toml`, by the model the transcript names. A turn over it that sent a result moves the task and its `factory.step.ended` record carries `overrun`. Any other turn over it, or a turn whose cost is unknown, ends the run and fails the task. Inside a turn, the daemon reads the transcript at most once a minute and ends a run that passed the budget, unless it sent its result. See [health.md](../daemon/health.md#budget) | `"1.00"` |
| `record` | string | no | none | Model steps only. The factory's command that records evidence after the step's result, relative to the factory folder. It must be an executable file inside the folder. The daemon runs it in a checkout of the result's commit, and admits the files it writes as the result's items. See [evidence.md](../daemon/evidence.md) | `"scripts/record-login"` |
| `options` | array of strings | human steps | `[]` | The answers a person picks from | `["open", "stop"]` |
| `question` | string | no | the step's id | Human steps only: what the ask says. It may name fields of the task's event, as `command` does. See [Fields of the task's event](#fields-of-the-tasks-event) | `"Compare pull request {pull_request}?"` |
| `approves` | array of strings | no | `[]` | Human steps only: the irreversible steps this answer approves | `["open-pr"]` |
| `command` | table | no | none | Human steps only: a command an answer runs once. `argv`, the program and its arguments; `target`, `worktree` or a name in [`[asks.targets]`](factory.md#asks), default `worktree`; `runs`, the options that run it. See [Runbooks](#runbooks) | `{ argv = ["cargo", "test"], target = "repo", runs = ["run"] }` |
| `action` | string | code steps | | The built-in action: `open-pr`, `open-issue`, `open-memories-pr`, `run` or `start-batch` | `"open-pr"` |
| `run` | string | `run` actions | | The command a `run` action runs with `sh -c` | `"scripts/poll.sh"` |
| `next` | table of strings | yes | | Each outcome and where it leads: a step id, `done` or `stop` | `{ passed = "done", failed = "stop" }` |
| `by` | table of `code`, `model`, `human` or `asleep` | no | the step's `decides` | Who decides an outcome's edge, where it differs from the step's decider | `{ passed = "asleep" }` |
| `proof` | table of `gate-passed` | no | none: every outcome is the agent's word | Model steps only. The proof the daemon checks before it accepts an outcome, by outcome. `gate-passed`: the factory's [`gate`](factory.md#top-level-keys) passed at the head of the task's branch. See Proofs in [tools.md](../daemon/tools.md#proofs) | `{ passed = "gate-passed" }` |
| `gate` | table of scorer lists | no | none: no scorer grades an outcome before it is stored | Model steps only. The [scorers](scorers.md) that grade an outcome before the daemon stores it, by outcome. A scorer on `result` or `diff`, whose `fail` or `error` refuses the outcome. A gate never approves anything. See Gates in [score.md](../daemon/score.md#gates) | `{ passed = ["tests", "does-what-was-asked"] }` |
| `plan_requires` | array of `goal`, `testing`, `verification` | no | `[]` | Model steps only. The plan result fields this step's `prompt` or its agent's instructions must ask for. `priory check` refuses the step when neither mentions a named field. See `result` in [tools.md](tools.md#resultoutcome-summary-evidence-commit-reason-branches-risk-goal-testing-brief-verification-feedback-batch) | `["goal", "testing", "verification"]` |
| `after` | `merged` | no | none | What the task waits for before it enters the step. `merged`: every pull request of the task's stack merged. Until then the task sleeps on the checks of [Done and checks](../daemon/waits.md#done-and-checks), and green checks alone do not move it on. See [Steps after the merge](#steps-after-the-merge) | `"merged"` |
| `autonomous` | string | no | | Why an irreversible step may run without a person's approval | `"it only labels issues"` |
| `guard` | table | no | none allowed | Irreversible model steps only. `allow`: the guards whose calls the step's agent may make while the step is approved at the task's commit or `autonomous`. An `autonomous` step cannot name `gh-pr-merge` or `gh-pr-approve`, and no step can name `priory-person`, `connector-sign-in` or `unreadable`. See [guard.md](../daemon/guard.md) | `{ allow = ["gh-pr-comment"] }` |

The outcomes of a step come from:

| `decides` | Its outcome is |
| --------- | -------------- |
| `model` | the `outcome` field of the run's `result`, or a debate's shared verdict |
| `human` | the option the person picks; every option needs a `next` |
| `code` | the action's status, such as `opened` or `failed`. A `run` action's is the last line of its stdout when `next` names it, else `ok` on exit 0 and `failed` otherwise |

Each edge, an outcome and its target, has a `by`: who decides that the task follows it. It defaults
to the step's `decides`. Set `by` for an outcome when that is wrong, such as `asleep` for an outcome
that comes from a wait on CI rather than from the model. `priory draw` colors each edge by its `by`,
and `priory check --explain` counts only a `human` edge as a person's decision.

A model step outcome of `red`, `yellow` or `green` also grades the task: how risky or uncertain its
work is. The task keeps the latest grade and its reason. See Grades in
[tools.md](tools.md#grades).

A model step outcome of `low`, `medium` or `high` whose result carries `risk`, one line per risk
dimension, is a merge risk verdict on the finished change. The task keeps the latest verdict, and its asks
show it to the person. See Merge risk in
[tools.md](tools.md#merge-risk).

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
ask command (see [tools.md](tools.md#commands-an-answer-runs)). The task moves by that
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

### Fields of the task's event

A human step's `question` and each word of its command's `argv` may name a field of the event that
started the task, such as `{pull_request}`. When the task enters the step, Priory fills each field
from the event, so the ask shows the values that run.

| Field | Holds | Must be |
| ----- | ----- | ------- |
| `{repo}` | The event's repository | `owner/name` |
| `{pull_request}` | The pull request's number | A number |
| `{head_sha}` | The pull request's head commit | A full commit hash, of a head in `{repo}` itself |
| `{base_branch}` | The branch the pull request merges into | Printable, not starting with `-` |

The command runs with your sign-in, so `{head_sha}` takes only a head that the event names in its
own repository. Priory drops the task, with the reason, when the event's `head_repo` names a fork,
when the event does not name `head_repo`, when the event lacks a field the step names, or when a
value does not have its shape. `priory check` refuses a name that is not in the table.

A human step's `timeout` bounds its command, in place of `command_timeout`: a command such as
`priory bench` can run for hours. `examples/benchmark/` compares a pull request this way:

```toml
[[steps]]
id = "compare"
decides = "human"
effect = "reversible"
question = "Compare pull request {pull_request} at {head_sha} on the implement benchmark, for at most $40?"
options = ["run", "skip"]
command = { argv = ["priory", "bench", "benchmarks/implement.toml", "--base", "{base_branch}", "--head", "{head_sha}", "--budget", "40", "--post", "{repo}#{pull_request}"], target = "factory", runs = ["run"] }
timeout = "4h"
next = { run = "done", skip = "stop" }
```

A task that reaches `done` with no result changed nothing, so it skips the checks gate even when
its key names a pull request (see [waits.md](../daemon/waits.md)).

## Batches

A batch's planning workflow, the factory's [`[batch] workflow`](factory.md#batch), ends a model step
with a result that carries `batch`, the plan. The human step right after it asks the go: its
question and text come from the plan, such as `Start batch 4: 8 tasks from 9 issues, and close 2?`.
The answer that leads to a `start-batch` step starts the batch, which creates one task per entry,
and its outcome is `started` or `failed`. A `failed` start leads back to the go, with its error in
the question. `examples/batches/` plans issues this way. See [batches.md](../daemon/batches.md).

## Steps after the merge

A step with `after = "merged"` lets a task go on once its pull request merged, instead of ending at
`done`. `examples/ship-and-watch/` watches a release, then announces it:

```toml
[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
autonomous = "it only opens a pull request, which a person reviews before anyone merges it"
next = { opened = "watch", failed = "stop" }

[[steps]]
id = "watch"
after = "merged"
decides = "model"
agent = "shipper"
prompt = "The change merged. Watch its release every 10 minutes for an hour."
effect = "read"
next = { quiet = "draft", problem = "look" }
```

When the task reaches `watch`, the daemon stores the checks gates that `done` stores, and the task
sleeps at the step of its latest result. A red check, a moved head or feedback wakes that step's
agent, as before `done`. Green checks leave the task asleep. Once every pull request of its stack
merged, the task enters `watch`. A result that leads straight to such a step needs `commit`, as one
that leads to `done` does. A step after the merge can lead to `done` with no `commit`.

When the task reaches such a step after the merge, it enters it at once. It is blocked instead
when it owns no pull request, `step <id> waits for a merge, but the task owns no pull request`, or
when a pull request of its stack closed without a merge, `step <id> waits for a merge, but a pull
request of the stack closed without one`. The start step cannot wait for a merge.

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
workflow	step	gate	caught	judged wrong	landed wrong
implement	open-pr	approve-pr	4	17 of 53, 95% bound 43.4%	3 of 53, 95% bound 13.7%
triage	label	autonomous: it only labels issues	0	2 of 12, needs 20	0 of 12, needs 20

workflow	path with no human decision	caught	judged wrong	landed wrong
triage	read -> label	0	2 of 12, needs 20	0 of 12, needs 20

agent	guard	workflow	why

workflow	step	outcome	proof
implement	implement	passed	gate-passed
```

The first two tables end with the history of the irreversible step, the last step of a path, from
[`priory report risk`](../daemon/trace.md#risk) over its default window: `caught`, the tasks a person
stopped at the step's gate, then `judged wrong` and `landed wrong` as `k of n` with the 95 %
posterior upper bound, or `k of n, needs 20` under 20 trials. These columns read the state file
`PRIORY_DB` names, and show `-` when there is none this priory reads. `priory check` without
`--explain` never reads the state, and no column changes a check's verdict.

A workflow `docs` with `guard = { allow = ["git-push-default"] }` and a model step run by `writer`
adds the row `writer	git-push-default	docs	allowed by the workflow`.

`--json` prints the same rows as one object, with the history as each irreversible step's
`risk_estimate` (see [serve.md](serve.md#flows)), `null` with no state file. `priory flows [DIR] --json` prints the same object with
each workflow's steps and edges added, as `GET /api/flows` serves it, and the Flows panels of the web
view and the Herdr adapter draw them:

```json
{
  "irreversible": [
    { "workflow": "implement", "step": "open-pr", "approved_by": ["approve-pr"], "autonomous": null, "risk_estimate": null },
    { "workflow": "triage", "step": "label", "approved_by": [], "autonomous": "it only labels issues", "risk_estimate": null }
  ],
  "paths": [{ "workflow": "triage", "path": ["read", "label"] }],
  "tools": [],
  "proofs": [{ "workflow": "implement", "step": "implement", "outcome": "passed", "proof": "gate-passed" }]
}
```

When `factory.toml` sets `[sweep] agent` and no `workflows/sweep.toml` exists, the factory also gets
the built-in `sweep` workflow. See [sweep.md](../daemon/sweep.md).

## Debate steps

A debate step is a model step whose `debate` names two agents in place of `agent`. They answer the
step's prompt alone first, then answer each other's latest position for `rounds` rounds. The
verdicts are the keys of `next`. When both agents name the same verdict, the daemon writes the
step's result with it. When they do not, the task asks a person to pick one. See
[debates.md](../daemon/debates.md).

```toml
[[steps]]
id = "debate-design"
decides = "model"
effect = "read"
debate = { agents = ["architect", "skeptic"], rounds = 3, cap = "$3.00" }
prompt = "Should the cache live in the daemon or in its own process? Read docs/design/cache.md."
timeout = "15m"
next = { daemon = "implement", process = "implement-split", undecided = "ask-design" }
```

A debate step needs a `timeout` and an effect of `none` or `read`, and takes no `agent` or
`proof`. Each debater needs its own session and a model priced in `prices.toml`, so the cap counts
every turn. Both harnesses, Claude Code and Codex, report token counts.

`priory check --explain` prints each debate's worst case in a last table, `workflow`, `step` and
`debate`: `2 agents, up to 8 turns, cap $3.00`. It adds `both agents run <harness> with model
<model>, so the debate mostly agrees with itself` when both agents share a harness and a model.
`priory draw` draws a debate step as a model step with `debate: architect and skeptic` on its
second line.

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
| `` only a model step has a `debate` `` | `debate` on a human or code step |
| `` a debate step has no `agent`: its `debate` names both `` | A step with both `agent` and `debate` |
| `a debate needs exactly two agents, in speaking order, not <n>` | `debate.agents` names one agent, or three |
| `` a debate's two agents must differ: `<name>` is named twice `` | The same agent twice |
| `` `rounds` is <n>; a debate takes 1 to 5 rounds `` | `rounds` outside 1 to 5 |
| `` a debate step's effect is `none` or `read`, not `<effect>`: debaters judge and change nothing `` | A debate step that may change something |
| `` a debate step has no `proof`: debaters judge and change nothing `` | `proof` on a debate step |
| `` a debate step needs a `timeout`: its cap cannot bound one turn `` | A debate step without `timeout` |
| `` agent `<name>` keeps a shared session, and a debater needs its own `` | A debater with `session = "shared"` |
| `` agent `<name>`'s model `<model>` has no price in prices.toml, so the debate's cap cannot count it `` | A debater whose model has no price |
| `` agent `<name>` names no model, so prices.toml cannot price it for the debate's cap `` | A debater with no `model` |
| `"<text>" is not a cap such as "$3.00", above zero` | A bad `cap` |
| `` missing field `cap` `` | A `debate` without `cap` |
| `` option `<option>` has no `next` `` | A human step's option leads nowhere |
| `` `next` has outcome `<outcome>`, which is not an option `` | A human step's `next` names an answer it does not offer |
| `` a code step needs an `action` `` | A code step without `action` |
| `` a `run` action needs a `run` command `` | `action = "run"` without `run` |
| `` only a `run` action has a `run` command `` | `run` on a code step with another action |
| `` an `open-memories-pr` action writes to the memories repository or the kit, so factory.toml needs a `[memories]` or a `[kit]` `` | An `open-memories-pr` step in a factory with neither `[memories]` nor `[kit]` |
| `` only a code step has a `run` command `` | `run` on a model or human step |
| `` only a human step has a `command` `` | `command` on a model or code step |
| `` only a human step has a `question` `` | `question` on a model or code step |
| ``question names `{<name>}`, which is not a field of the task's event: use {repo}, {pull_request}, {head_sha} or {base_branch}`` | A `question` names another field |
| ``command argv names `{<name>}`, which is not a field of the task's event: use {repo}, {pull_request}, {head_sha} or {base_branch}`` | A command's `argv` names another field |
| `command argv must start with a program` | A `command` with an empty `argv` or an empty first word |
| `` command target `<target>` is not worktree nor a name in [asks.targets] `` | A `command` names a target the factory lacks |
| `` command runs `<option>`, which is not an option `` | A `runs` entry the step does not offer |
| `` `next` is empty `` | A step with no outcome |
| `` `by` has outcome `<outcome>`, which is not in `next` `` | `by` names an outcome the step does not have |
| `` only a model step has a `proof` `` | `proof` on a human or code step |
| `` `proof` has outcome `<outcome>`, which is not in `next` `` | `proof` names an outcome the step does not have |
| `` `proof` names `gate-passed`, and factory.toml sets no `gate` `` | A step needs the gate, and the factory has none |
| `` only a model step has a `gate` `` | `gate` on a human or code step |
| `` `gate` has outcome `<outcome>`, which is not in `next` `` | `gate` names an outcome the step does not have |
| `` `gate` of outcome `<outcome>` names `<name>`, which is no scorer in scorers/ `` | A gate names a scorer the factory lacks |
| `` `gate` of outcome `<outcome>` names `<name>`, which reads asks: a gate grades a result, so use a scorer on result or diff `` | A gate names an `ask` scorer |
| `` only a model step has `plan_requires` `` | `plan_requires` on a human or code step |
| `` `plan_requires` names `<field>`, which neither the step's prompt nor its agent's instructions mention `` | A required plan field that no text asks the agent for |
| `` outcome `<outcome>` leads to `<target>`, which is not a step, `done` or `stop` `` | A `next` target that does not exist |
| `no path from start reaches it` | No chain of `next` from `start` reaches the step |
| `` approves `<id>`, which is not an irreversible step `` | `approves` names a missing or reversible step |
| `` `autonomous` needs a reason `` | `autonomous` is empty |
| `irreversible, and a path from start reaches it with no human step that approves it` | An irreversible step without `autonomous` and without an approving human step on every path |
| `` band `<band>` is not one of <bands> `` | `band` is not a band of the factory |
| `` start `<id>` waits for a merge, and a task starts before it owns a pull request `` | The start step sets `after` |
| `` reopen `<id>` is not a model step `` | `reopen` names a missing step, or a human or code step |
| `"<text>" is not a duration such as 90s, 15m or 2h` | A bad `timeout` |
| `` step `<id>`: `budget` "<text>" is not an amount of dollars above zero, such as "1.00" `` | A `budget` such as `"-1"`, `"0"` or `"one"` |
| `` step `<id>`: `budget` needs prices.toml, which turns the step's tokens into dollars `` | A `budget` in a factory without `prices.toml` |
| `` step `<id>`: only a model step has a `budget` `` | A `budget` on a human or code step |
| `` step `<id>`: only a model step has a `record` command `` | A `record` on a human or code step |
| `` step `<id>`: `record`: <path> leads outside the factory folder `` | A `record` path through `..` or a symbolic link that leaves the factory folder |
| `` step `<id>`: `record`: <path> is not a file in the factory folder `` | A `record` path that does not exist or is a folder |
| `` step `<id>`: `record`: <path> is not executable: chmod +x it `` | A `record` file without an execute bit |
| ``invalid value: integer `0`, expected a nonzero usize`` | `max_prs` is 0 |
| `` `guard.allow` names `<name>`, which is no built-in guard nor one of `[guard]` in factory.toml `` | A workflow's or a step's `guard.allow` names a guard that does not exist |
| `` `guard.allow` names `<name>`, which only a person's approval grants, so only an irreversible step may name it `` | A workflow's `guard.allow` names `gh-pr-merge` or `gh-pr-approve` |
| `` step `<id>`: `guard.allow` names `<name>`, which only a person's approval grants, and the step is `autonomous` `` | An `autonomous` step's `guard.allow` names `gh-pr-merge` or `gh-pr-approve` |
| `` `guard.allow` names `priory-person`, which only a person runs, so nothing may grant it `` | A workflow's or a step's `guard.allow` names `priory-person` |
| `` `guard.allow` names `connector-sign-in`, which only an agent's `connectorSignIn` grants `` | A workflow's or a step's `guard.allow` names `connector-sign-in` |
| `` `guard.allow` names `unreadable`, which the guard cannot read, so nothing may grant it `` | A workflow's or a step's `guard.allow` names `unreadable` |
| `` step `<id>`: `guard.allow` grants nothing on a step that is not an irreversible model step `` | `guard.allow` on a step that is not an irreversible model step |
