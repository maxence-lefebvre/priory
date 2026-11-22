# How to test a factory end to end

This guide shows how to play a task through your own factory folder, from its request to its final
state, before any real agent runs. It assumes a clone of this repository with its
[build prerequisites](../GETTING_STARTED.md#1-build-priory), and a factory that passes
`priory check`.

The test copies your factory into a scratch folder and replaces its runners with one for the test
host. It then starts this build's daemon with the fake `claude`, `codex` and `gh` of
`crates/priory/tests/fake/`, and a bare Git origin for each repository your triggers name. It
asks for one task, plays your scenario, and checks where the task ended. `gh`, `claude`, `codex`
and Git's GitHub remotes go to fakes, and every process starts with a cleared environment, a
scratch home and a scratch Priory home. Your factory's own scripts still run as written, so read
what a `run` step calls before you play it. The fakes serve tests such as this one only.
Each script's opening comments list its options. Keep them off the `PATH` of a real run.

## 1. Write a scenario

A scenario is a TOML file that says how the task starts, what the fake agent does on each turn,
what reaches the task while it sleeps, and what you expect. Keep yours in your factory's `e2e/`
folder. [e2e-scenarios.md](reference/e2e-scenarios.md) lists every key.

This scenario, `examples/groom/e2e/green.toml`, takes a green issue through its merge risk to a
pull request whose checks pass:

```toml
[request]
trigger = "groom-label"
key = "github:owner/repo#42"

[[turns]]
calls = [{ name = "result", arguments = { outcome = "green", summary = "Green: one function and its test" } }]

[[turns]]
calls = [{ name = "result", arguments = { outcome = "passed", summary = "Widget added", evidence = "cargo test passed" } }]

[[turns]]
calls = [{ name = "result", arguments = { outcome = "low", summary = "Merge risk low", risk = { ask = "low: adds the widget the issue asks for", blast_radius = "low: one new function", verification = "low: a test covers it", rollback = "low: a revert removes it", detection = "low: its test fails on a wrong box" } } }]

[[turns]]
calls = [{ name = "result", arguments = { outcome = "described", summary = "Add the widget\nIt draws a box. cargo test passes. Merge risk low." } }]

[[events]]
at = "describe"
type = "github.check_run.completed"
subject = "owner/repo#7"
data = { repo = "owner/repo", pull_request = 7, head_branch = "priory/1", head_sha = "{head}", base_branch = "main", name = "test", conclusion = "success", github_id = 1, pending = 0, failing = 0 }

[expect]
path = ["groom:green", "implement-green:passed", "risk:low", "describe:described", "open-pr:opened"]
state = "done"
```

Each turn's calls are the tool calls the fake agent makes, in order, before its turn ends. One
model step usually takes one turn. A step whose agent calls `wait` takes another turn once an event
wakes it. A wait that a code step's action made, such as `open-pr` waiting on checks, takes none.
Each event must wake the task within 10 seconds, and every event must play before the task ends.

## 2. Run it

From the repository root:

```sh
PRIORY_E2E_FACTORY=~/my-factory PRIORY_E2E_SCENARIO=~/my-factory/e2e/green.toml \
  cargo test -p priory --test factory_e2e -- --ignored --nocapture
```

The test prints the path the task took, its final state, the guard's denials, the runs, actions,
asks and waits, and the fakes' logs. It fails on the first expectation that does not hold, with
the same report.

To keep a result, redirect the output to a file:

```sh
... --nocapture > ~/my-factory/e2e/results/green.txt 2>&1
```

Without the two variables, the ignored test plays `examples/groom` with its green scenario. CI
plays that scenario, `examples/review/e2e/pending.toml` and `examples/ship-and-watch/e2e/merged.toml`,
so the harness itself stays covered.

## 3. Read a failure

| What you see | What to check |
| ------------ | ------------- |
| `timed out` with the task `running` | The scenario has fewer turns than the task's model steps take. Add the missing turn |
| `timed out` with the task `queued` | A step names an agent the factory lacks, or the agent declares more memory than 60 GiB |
| A path that stops early | The outcome in a `result` call is not one the step's `next` names. `fake/tools.log` in the report shows the tool's reply |
| `no denial of ...` or `the guard denied ...` | The step's `guard.allow`, its effect and its `autonomous` key. See [guard.md](daemon/guard.md) |
| A `priory.report` line from `intake` | A source your scenario does not fake failed its poll. Add a `[gh]` answer for the call `fake/gh.log` shows |

## Limits

- The fake `claude` does not read an agent's `permissions`. A built-in tool call that the guard
  lets through reaches Priory's permission hook, which denies it, as in a run with no
  `defaultMode`. The guard's own decision is what `allowed` and `denied` check.
- The fake agent runs no commands. A call's `run` key runs a command in the factory copy before
  the daemon starts, and uses what it prints as that call's arguments. A call's `output` key puts
  a built-in tool call and what it printed into the fake's transcript, as `priory bench capture`
  reads it ([bench.md](daemon/bench.md)).
- The turns form one queue, so a scenario follows one task whose agent runs at a time. A firing
  that also starts a code-step task, such as a poll, is fine; name the task to follow with
  `workflow`.
- The fakes test the graph, not what a real model does on a real event. To see that before a new
  trigger can write anywhere, set [`staged = true`](daemon/tools.md#staged-tasks) on the trigger.
- A Linear or Slack source has no fake answers. Its poll fails, and a `[fire]` scenario still fires
  its schedule triggers.
