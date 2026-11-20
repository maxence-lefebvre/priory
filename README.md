# Priory

Priory runs coding agents, such as Claude Code and Codex, on machines you own. It turns events
into tasks: a labelled GitHub issue, a Linear issue that enters a view, a Slack message, a schedule
or a request you type. It then carries each task through a workflow you wrote, usually to a pull
request that a person reviews and merges.

Priory is a Rust program with two long-running processes. The intake polls GitHub, Linear, Slack
and your schedules. The daemon turns what the intake finds into tasks and runs one agent per task
in its own tmux session and git worktree.

## The problem it solves

Running one coding agent is easy. Keeping several of them moving while nobody watches is not. An
agent waits on CI, on a review or on a person's answer. It crashes, fills the disk or runs the
machine out of memory. A question it asks in its own terminal goes unseen.

Priory handles that work in deterministic code, not in a coordinator agent that spends tokens on
every poll:

- An agent that waits holds no process. It names what it waits for, such as a pull request's
  checks, and goes to sleep. The daemon resumes its session when that event arrives.
- Every question an agent asks lands on one list, the desk, with options and a recommendation.
  You answer it from the command line.
- Every workflow step declares who decides it (a model, a person or code) and what it changes. So
  `priory check` can refuse a workflow before it runs, for example one that pushes code with no
  person's approval.
- Every step's outcome, tokens and time are recorded, so you can measure a workflow and its cost.

[docs/prd.md](docs/prd.md) holds the full product requirements.

## An example

A factory is a folder of files. This one has one agent, one workflow and one trigger.

`factory.toml` names the factory:

```toml
name = "example"
```

`agents/implementer/agent.md` describes who runs. The frontmatter between the `+++` lines sets the
harness, the model and the permissions. The Markdown body becomes the agent's system prompt:

```markdown
+++
description = "Implements one task and opens its pull request."
harness = "claude-code"
model = "sonnet"
memory = "6GB"
permissions = { defaultMode = "auto", allow = ["Bash(cargo test:*)"] }
+++
Read the request, make the smallest correct change, and test it.
```

`workflows/implement.toml` lists the steps a task goes through:

```toml
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the request and run the tests."
effect = "reversible"
next = { passed = "approve-pr", failed = "stop" }

[[steps]]
id = "approve-pr"
decides = "human"
effect = "none"
approves = ["open-pr"]
options = ["open", "stop"]
next = { open = "open-pr", stop = "stop" }

[[steps]]
id = "open-pr"
decides = "code"
action = "open-pr"
effect = "irreversible"
next = { opened = "done", failed = "stop" }
```

`triggers/agent-label.toml` starts that workflow when someone adds the label `agent` to an issue:

```toml
source = "github"
repo = "owner/name"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
```

When someone labels issue 42 of `owner/name` with `agent`, this happens:

1. The intake sees the label on its next poll, within a minute. It publishes the GitHub event and a
   task request for the `implement` workflow, with the key `github:owner/name#42`.
2. The daemon creates task 1 and queues it at the `implement` step. When the machine has enough
   free memory, disk and CPU, it clones the repository once, adds a worktree on the branch
   `priory/1`, and starts `claude` in the tmux session `priory-1`.
3. The daemon types the step's prompt and the issue event into the agent. The agent works, then
   calls Priory's `result` tool with the outcome `passed`.
4. The task moves to `approve-pr`, a human step. The agent's process ends and the task sleeps.
   `priory desk` now shows an ask with the options `open` and `stop`.
5. You run `priory answer 1 open --revision 1`. The answer approves `open-pr` for the branch's
   current commit.
6. The `open-pr` code step pushes the branch and opens a pull request. The task waits, with no
   process, until the pull request's checks finish. Green checks mark it done. A red check wakes
   the agent with the failure, in the same conversation, so it can fix it.

`priory draw` prints this workflow as a Mermaid flowchart, with each step's decider and effect.

## Build and run

Priory needs Linux, the Rust toolchain that `rust-toolchain.toml` pins, and on `PATH`: `git`,
`tmux`, `gh` logged in to GitHub, and `claude` or `codex`.

```sh
cargo build --release
./target/release/priory check path/to/factory
./target/release/priory draw path/to/factory implement
```

`cargo install --path crates/priory` puts `priory` in `~/.cargo/bin`. The in-process embedding model
for search sits behind a cargo feature: `cargo build --release --features model`.

Priory keeps its state in three places, each named by an environment variable:

| Variable       | Holds                                                         | Default                          |
| -------------- | ------------------------------------------------------------- | -------------------------------- |
| `PRIORY_DB`    | The daemon's SQLite state: tasks, runs, asks, waits, results  | none: the daemon refuses to start |
| `HERDR_BUS_DB` | The [herdr-bus](https://github.com/flupke/herdr-bus) file the intake publishes to | `~/.local/share/herdr-bus/bus.db` |
| `PRIORY_HOME`  | Clones, worktrees and the daemon's socket                     | `~/.local/state/priory-oss`      |

Run the intake and the daemon side by side, each pointed at the factory folder:

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake path/to/factory    # polls every 60 seconds
priory daemon path/to/factory    # routes events and runs agents
```

Both check the factory first, as `priory check` does, and refuse to start while it has problems.
`--once` makes the intake poll once, and the daemon route what is pending, then exit.

Before the first run in a repository, trust Priory's clone in Claude once, so its trust dialog does
not stop the run. See [Trusting a clone](docs/daemon/sessions.md#trusting-a-clone).

To start a task by hand, without a trigger:

```sh
priory request path/to/factory --workflow implement --key docs-1 --repo owner/name "Fix the typo in the README"
```

To keep both processes running, install them as systemd user services. See
[docs/daemon/services.md](docs/daemon/services.md).

### Day to day

| Command | What it does |
| ------- | ------------ |
| `priory desk` | Lists what waits on you: asks, blocked and failed tasks, held tasks, the queue |
| `priory answer ASK OPTION --revision N` | Answers an ask |
| `priory tasks [--lane LANE]` | Lists tasks in start order |
| `priory attach TASK [--read-only]` | Attaches your terminal to a task's agent |
| `priory resume TASK` | Resumes a blocked task |
| `priory cancel TASK` | Drops an active task and ends its run |
| `priory revive TASK [--step STEP]` | Restarts a failed or dropped task |
| `priory hold TASK --reason TEXT`, `priory release TASK` | Holds a task back, then lets it go |
| `priory move TASK --band BAND` | Moves a task to another priority band |
| `priory search QUERY` | Finds tasks by what they are about |
| `priory report DIR --workflow NAME [--cost]` | Prints outcome rates, or tokens and cost, per step |

Run `priory` with no arguments to print its usage.

## Concepts

**Factory.** The folder of files that defines everything else: `factory.toml`, `agents/`,
`workflows/`, `triggers/`, `runners/`, and optionally `lanes.toml` and `prices.toml`. Each file can be
TOML or YAML. Changing the factory is a pull request. Every key is in
[docs/reference/](docs/reference/README.md).

**Trigger.** One event source, a filter and the workflow its events start. Sources are `github`,
`linear`, `linear.view`, `slack` and `schedule`. A trigger's `key` says which events are the same
work, and a factory holds at most one active task per key. See
[docs/intake/README.md](docs/intake/README.md).

**Workflow and steps.** A workflow is a graph of steps. Each step has one decider: a `model` step
runs an agent, a `human` step asks a person, and a `code` step runs a built-in action such as
`open-pr`. Each outcome maps to the next step, `done` or `stop`. A step marked `irreversible` runs
only after a person approved it, unless it is marked `autonomous` with a reason. See
[docs/daemon/tools.md](docs/daemon/tools.md).

**Task and run.** A task is one unit of work, from its event to `done`, `failed` or `dropped`. A run
is one agent process working on a task's step. A task can have many runs, and its agent's session
carries over between them. See [docs/daemon/runs.md](docs/daemon/runs.md).

**Inbox and routing.** Every event the intake publishes lands in the daemon's inbox before anything
acts on it. The daemon then finds the task the event concerns, such as the task that owns the pull
request a check ran on. See [docs/daemon/routing.md](docs/daemon/routing.md).

**Waits and asks.** An agent ends a turn with one of three tools: `result` reports the step's
outcome, `wait` names an event to sleep until, and `ask` puts a question on the desk. A waiting or
asking agent has no process. See [docs/daemon/waits.md](docs/daemon/waits.md).

**Desk.** Everything that waits on a person, in one list: asks, blocked and failed tasks, prompts
Priory could not confirm, and a banner when the queue is stalled. See
[docs/daemon/desk.md](docs/daemon/desk.md).

**Queue, bands and room.** Tasks start in band order (`blocker`, `now`, `today`, `later` by
default), then in an order you set, then oldest first. A run starts only when the machine has room
for the memory and disk its agent declares. See [docs/daemon/room.md](docs/daemon/room.md).

**Health.** Every second, the daemon checks each run for an unexpected exit, a timeout or a
permission prompt. It never approves a permission prompt. A crashed run gets one repair attempt.
See [docs/daemon/health.md](docs/daemon/health.md).

**Trace and reports.** Each step's end is recorded with its outcome, tokens and time. `priory report`
reads these records, and the daemon can export them as OpenTelemetry spans. See
[docs/daemon/trace.md](docs/daemon/trace.md) and [docs/trace-schema.md](docs/trace-schema.md).

## Status

Priory is in early development. The milestones come from [docs/prd.md](docs/prd.md#milestones).

| Milestone | Scope | Status |
| --------- | ----- | ------ |
| M0 | One machine, tmux, Claude Code, CLI and GitHub intake, inbox, workflows, waits, asks, results, health | Done |
| M1 | Linear, Slack replies, schedules, queue order, holds, revive, lanes, bands, memory gate, agents slice, retiring tasks, traces, reports, nudges, repair, Codex | Done |
| M2 | Web view, webhooks, terminal attach, command answers, OpenTelemetry export, search | In progress: terminal attach, OpenTelemetry export and search have landed |
| M3 | Several machines, state adapters, shared skills and memories | Not started |
| M4 | Risk estimate, scorers and benchmarks | Not started |

## Documentation

- [docs/prd.md](docs/prd.md): what Priory does, and in which milestone.
- [docs/reference/](docs/reference/README.md): every key of the factory's files, every intake
  message, and their JSON Schemas.
- [docs/daemon/](docs/daemon/README.md): how the daemon runs tasks, one file per topic.
- [docs/intake/](docs/intake/README.md): the messages the intake publishes, one file per source.
- [docs/trace-schema.md](docs/trace-schema.md): the format of trace records.

## License

MIT. See [LICENSE](LICENSE).
