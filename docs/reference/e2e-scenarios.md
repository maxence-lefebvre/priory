# End-to-end scenarios

A scenario file drives `crates/priory/tests/factory_e2e.rs` through one factory folder. It is TOML.
[testing-a-factory.md](../testing-a-factory.md) shows how to run one.

## Environment

| Variable | Default | Meaning |
| -------- | ------- | ------- |
| `PRIORY_E2E_FACTORY` | `examples/groom` | The factory folder to copy and play |
| `PRIORY_E2E_SCENARIO` | `<factory>/e2e/green.toml` | The scenario file |

## Keys

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `request` | table | one of `request` or `fire` | The `factory.task.requested` message to publish, as the intake would. Its fields join the message's `data` | `{ trigger = "groom-label", key = "github:owner/repo#42" }` |
| `request.trigger` | string | yes | The trigger's file name, without `.toml`. Its `workflow` and `band` are the defaults | `"groom-label"` |
| `request.key` | string | yes | The task key | `"linear:ENG-7"` |
| `request.repo` | string | no | The task's repository, as a review request or a CLI request carries it | `"owner/repo"` |
| `fire` | table | one of `request` or `fire` | Runs `priory intake --once` at `fire.at`, which fires every schedule trigger due then | `{ at = "2026-11-23T07:00:30+01:00" }` |
| `workflow` | string | no | The workflow of the task to follow when a firing starts several. The first task otherwise | `"bonjour"` |
| `files` | table | no | Files to write over the factory copy, path to content. A path that starts with `~/` goes under the test's home. An absolute path or `..` is refused. Content that starts with `#!` is executable | `{ "~/.local/bin/tool" = "#!/bin/sh\nexec priory \"$@\"\n" }` |
| `gh` | table | no | Answers of the fake `gh`, call prefix to output. The longest matching prefix wins. An answer that starts with `#!` runs as a script with the call's arguments. Any other call goes to the shared fake | `{ "api search/issues" = "{\"total_count\": 0, \"incomplete_results\": false, \"items\": []}" }` |
| `turns` | array of tables | no | The fake agent's turns, in order, across every step and task | |
| `turns[].calls` | array of tables | yes | The turn's tool calls, in order: `{ name, arguments }`, or `{ name, run }` | `[{ name = "result", arguments = { outcome = "passed", summary = "Done" } }]` |
| `turns[].calls[].run` | string | no | A shell command run in the factory copy before the daemon starts. The JSON it prints is the call's `arguments` | `"python3 -I e2e/report.py"` |
| `turns[].calls[].output` | string | no | Stands for a built-in tool call that ran and printed this text, such as a `Bash` call to `gh`. The fake writes the call and its result to its Claude transcript, and no hook hears it | `"[]"` |
| `events` | array of tables | no | What reaches the task while it sleeps, in order. Each plays once the task sleeps at the step `at` names, or at any step without `at` | |
| `events[].at` | string | no | The step the task sleeps at | `"implement-green"` |
| `events[].answer` | string | no | Answers the task's open ask with this option instead of publishing a message | `"approved"` |
| `events[].type`, `subject`, `data` | string, string, table | without `answer` | The bus message to publish. `{head}` in `data` becomes the commit the task's branch points to in its origin | `"github.check_run.completed"` |
| `expect.path` | array of strings | yes | The steps the task finished, each as `step:outcome`, in order | `["groom:red"]` |
| `expect.state` | string | yes | The task's state at the end | `"asleep"` |
| `expect.step` | string | no | The task's step at the end | `"approve-plan"` |
| `expect.denied` | array of strings | no | Commands the tool guard denied the task, each matched as a substring of its trace record | `["gh pr merge 7"]` |
| `expect.allowed` | array of strings | no | Commands the tool guard denied none of | `["gh pr comment 7"]` |
| `expect.replies` | array of strings | no | Text the fake agent's tool calls got back | `["report"]` |
| `expect.output` | array of tables | no | `{ args, contains }`: the output of `priory <args>` holds each string | `{ args = ["asks"], contains = ["approved"] }` |

## When the test stops playing

The test plays until the task is `done`, `dropped`, `failed` or `blocked`, or sleeps with no event
left to play, or 120 seconds pass. It fails when an event does not wake the task within 10 seconds,
or when the task ends with an event left to play.
