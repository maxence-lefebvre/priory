# Benchmark cases

A case is a past task captured into a folder by `priory bench capture`
([bench.md](../daemon/bench.md)). `priory bench replay` plays it again under another configuration. Keep
each case in `benchmarks/cases/<name>/` of the factory, and commit it with the factory once a
person has read it.

| File | Holds | Schema |
| ---- | ----- | ------ |
| `case.toml` | The task, its request, where a replay starts and ends, its gaps and its real history | [`config/case.json`](../schemas/config/case.json) |
| `gh/<n>.toml` | One `gh` command the task's agent ran before the first irreversible step, with what it printed. Numbered from `001`, in the order the agents ran them | [`config/case-gh.json`](../schemas/config/case-gh.json) |
| `answers.toml` | The answer a person gave at each human step before the first irreversible step. Absent when there was none | [`config/case-answers.json`](../schemas/config/case-answers.json) |

Each file starts with a `#:schema` line, so an editor validates it as you fill a gap by hand.

## `case.toml`

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `workflow` | string | yes | The workflow the task ran | `"groom"` |
| `key` | string | yes | What the task worked on, prefixed by its source | `"github:owner/repo#42"` |
| `repo` | string | no | The repository of the task's worktree, as `owner/name` | `"owner/repo"` |
| `start` | string | yes | The commit the task's worktree was created from, its `base_sha` | `"280754dd..."` |
| `ends_at` | string | yes | The first irreversible step the task reached. A replay ends there and never runs it. For a `failed` or `blocked` task that reached none, the step it stopped at | `"open-pr"` |
| `partial` | boolean | yes | Whether `gaps` names anything | `true` |
| `gaps` | array of strings | yes | What the capture could not keep, one line each | `["run 2: its transcript is gone"]` |
| `request` | table | no | The first message the inbox routed to the task: `type`, `subject` and `data`, with its null fields left out | |
| `reference.path` | array of strings | yes | Every step the task finished, in order, each as `step:outcome` | `["groom:red", "approve-plan:approved"]` |
| `reference.labels` | array of strings | yes | The task's outcome labels that no `priory label --clear` withdrew | `["marked-wrong"]` |
| `spent` | array of tables | no | What each model step before `ends_at` spent, in order. A replay's estimate prices it. Absent in a case captured before Priory recorded it | |
| `spent[].step` | string | yes | The model step | `"groom"` |
| `spent[].model` | string | no | The model the run reported | `"claude-sonnet-5-5"` |
| `spent[].minutes` | integer | yes | How long the step took, in whole minutes rounded up, at least 1 | `4` |
| `spent[].tokens` | table | no | The step's `input`, `output`, `cache_read` and `cache_write` tokens. Absent when Priory could not read them | `{ input = 3, output = 812, cache_read = 40210, cache_write = 2210 }` |

## `gh/<n>.toml`

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `run` | integer | yes | The run whose agent ran the command | `1` |
| `step` | string | yes | That run's step | `"groom"` |
| `args` | array of strings | yes | The command's arguments after `gh` | `["issue", "view", "42", "--json", "title,body"]` |
| `output` | string | yes | What the command printed, as the transcript holds it | `'{"title":"Add a widget"}'` |

## `answers.toml`

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `answers` | array of tables | yes | One entry per answered ask of a human step, in order | |
| `answers[].step` | string | yes | The human step | `"approve-plan"` |
| `answers[].answer` | string | yes | The option or text the person answered | `"approved"` |

## Gaps

| Gap | Cause |
| --- | ----- |
| `` run <n>: `<command>` is not one gh command alone `` | The call ran `gh` in a pipe, a list, a script, a substitution or behind a variable or redirection |
| `` run <n>: `<command>` has no output in the transcript `` | The transcript holds the call and no result for it |
| `` run <n>: `<command>` failed, and the transcript does not hold its exit code `` | The call failed or was interrupted |
| `` run <n>: `<command>` printed more than the transcript holds `` | Claude cut the output down |
| `run <n>: its transcript is gone` | The run's transcript file no longer exists, as a harness deletes old ones |
| `run <n>: its transcript cannot be read` | The file exists and cannot be read |
| `run <n>: no transcript was recorded` | No hook of the run named a transcript |
| `run <n>: it ran on Codex, whose transcript the capture does not read yet` | The run's agent uses the `codex` harness |
| `the task's request is not in the inbox` | No inbox message was routed to the task |

A command in a gap is shown on one line, cut to 80 characters.

## Example

`crates/priory/tests/capture/red-case/` is the case a test captures from
`crates/priory/tests/capture/red.toml`, a red issue played through `examples/groom`:

```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/case.json
# Captured from task 1 by priory bench capture. Read it before you share it.
workflow = "groom"
key = "github:owner/repo#42"
repo = "owner/repo"
start = "280754ddff3d9064d54abdfab6e40999b7156695"
ends_at = "open-pr"
partial = true
gaps = ["run 1: `gh issue view 42 --comments | head -n 20` is not one gh command alone"]

[request]
type = "factory.task.requested"
subject = "github:owner/repo#42"

[request.data]
key = "github:owner/repo#42"
source = "e2e-request"
trigger = "groom-label"
workflow = "groom"

[reference]
path = ["groom:red", "approve-plan:approved", "implement:passed", "review:clean", "risk:low", "describe:described", "open-pr:opened"]
labels = []

[[spent]]
step = "groom"
model = "claude-fake-5"
minutes = 1

[spent.tokens]
input = 101
output = 11
cache_read = 1001
cache_write = 51
```

The file goes on with one `[[spent]]` entry for each of the other model steps.
