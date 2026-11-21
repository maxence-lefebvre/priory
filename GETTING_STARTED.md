# Getting started

This guide takes you from a clone of this repository to a running factory. You build Priory, write a
factory with one agent, one workflow and one trigger, check it, then run the intake and the daemon.

## Before you start

Priory needs Linux, the Rust toolchain that `rust-toolchain.toml` pins, and on `PATH`: `git`,
`tmux`, `gh` logged in to GitHub, and `claude` or `codex`.

## 1. Build Priory

From the root of your clone:

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"
priory
```

The last command prints the usage. To install the binary instead, `cargo install --path
crates/priory` puts `priory` in `~/.cargo/bin`. The in-process embedding model for search sits
behind a cargo feature: `cargo build --release --features model`.

## 2. Write a factory

A factory is a folder of files. Make an empty working folder outside your clone, then create
these four files in an `example/` folder inside it. Together they hold one agent, one workflow
and one trigger.

`factory.toml` names the factory, and the workflow `priory add` starts by default:

```toml
name = "example"

[request]
workflow = "implement"
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


## 3. Check and draw it

Run these from your working folder:

```sh
priory check example
priory draw example implement
```

`priory check` prints `ok: 1 agent, 1 trigger, 1 workflow, 0 runners`. When a file has a
problem, it prints the file, the line and what is wrong, and exits 1. `priory draw` prints this
workflow as a Mermaid flowchart, with each step's decider and effect, and each arrow colored by
who decides it.

`priory check --explain example` lists the irreversible step `open-pr` and the human step
`approve-pr` that approves it. See [workflows.md](docs/reference/workflows.md#irreversible-steps-and-their-gates).

## 4. Pick a starter

The repository's `examples/` folder holds ready-made factories. Each one passes `priory check`, and
its `README.md` says what it needs and which steps ask you first.

| Starter | What it does |
| ------- | ------------ |
| [`bug-investigation`](examples/bug-investigation/README.md) | A `bug` label starts a task. The agent reproduces the bug, finds the root cause, fixes it, and asks before it opens the pull request |
| [`review`](examples/review/README.md) | A review request starts a task. The agent writes a pending GitHub review, and you submit it |
| [`ci-failure`](examples/ci-failure/README.md) | Every hour on workdays, the agent fixes a failed check and asks before it opens the pull request |
| [`triage`](examples/triage/README.md) | A new issue starts a task. The agent labels it, and asks before it posts a comment that asks for what is missing |
| [`morning-digest`](examples/morning-digest/README.md) | On workday mornings, the agent posts the open asks, stuck tasks and yesterday's merges to your desk |
| [`groom`](examples/groom/README.md) | A `groom` label starts a task. The agent grades the ticket, writes the brief, and takes it to a pull request you review |

Copy one into your working folder, then check it:

```sh
cp -r <your clone>/examples/review my-factory
priory check my-factory
```

Replace the placeholders `owner/repo` and `your-login` in its `triggers/` files, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname. In the next step, use
`my-factory` in place of `example`, and the starter's workflow, the file name under its
`workflows/`, in place of `implement`.

## 5. Run it

Priory keeps its state in three places, each named by an environment variable:


| Variable       | Holds                                                         | Default                          |
| -------------- | ------------------------------------------------------------- | -------------------------------- |
| `PRIORY_DB`    | The daemon's SQLite state: tasks, runs, asks, waits, results  | none: the daemon refuses to start |
| `HERDR_BUS_DB` | The [herdr-bus](https://github.com/flupke/herdr-bus) file the intake publishes to | `~/.local/share/herdr-bus/bus.db` |
| `PRIORY_HOME`  | Clones, worktrees and the daemon's socket                     | `~/.local/state/priory-oss`      |

Run the intake and the daemon side by side, each pointed at the factory folder:

```sh
export PRIORY_DB=~/.local/state/priory-oss/priory.db
priory intake example    # polls every 60 seconds
priory daemon example    # routes events and runs agents
```

Both check the factory first, as `priory check` does, and refuse to start while it has problems.
`--once` makes the intake poll once, and the daemon route what is pending, then exit.

Before the first run in a repository, trust Priory's clone in Claude once, so its trust dialog does
not stop the run. See [Trusting a clone](docs/daemon/sessions.md#trusting-a-clone).

To start a task by hand, without a trigger:

```sh
priory add example --repo owner/name "Fix the typo in the README"
priory tasks
```

`priory add` prints the task's key, such as `cli:2026-11-20-1`. `priory tasks` lists task 1 with
that key, at the step `implement`, in the state `queued`. A GitHub issue URL in the text sets the
repository and the key, so `--repo` is not needed then. `priory request` takes every field as a
flag, for a key of your own: see [docs/intake/cli.md](docs/intake/cli.md). In the web view, `N`, or the
`+` button, opens the same Add: one field, with workflow, band and lane chips set to the defaults. Use a repository of yours in place of `owner/name` for a real run. When a queued task
starts is in [docs/daemon/room.md](docs/daemon/room.md).

To queue a Slack thread from Slack itself, set up the "Queue in Priory" message shortcut: see
[docs/intake/slack.md](docs/intake/slack.md#message-shortcut).

To keep both processes running, install them as systemd user services. See
[docs/daemon/services.md](docs/daemon/services.md).

## What happens to a task

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

## Day to day

| Command | What it does |
| ------- | ------------ |
| `priory desk` | Lists what waits on you: asks, blocked and failed tasks, held tasks, the queue |
| `priory answer ASK OPTION --revision N` | Answers an ask. `--expected N` is the same flag |
| `priory dismiss [--expected N] ASK` | Closes an ask with no answer |
| `priory snooze KIND ID --for 2h`, `--until TIME`, `--clear` | Hides a desk item until then, or brings it back |
| `priory tasks [--lane LANE] [--grade GRADE]` | Lists tasks in start order, with their grade |
| `priory task TASK --json` | Prints one task with its brief, plan, pull request, runs and asks |
| `priory attach TASK [--read-only]` | Attaches your terminal to a task's agent |
| `priory resume TASK` | Resumes a blocked task |
| `priory say TASK TEXT` | Sends a task's agent a message, waking it if it sleeps |
| `priory subscribe TASK RESOURCE`, `priory unsubscribe TASK RESOURCE` | Routes a pull request's, Linear issue's or Slack thread's events to a task until it closes, or stops routing them |
| `priory pause [REASON]`, `priory unpause` | Stops new tasks from starting, then lets them start again |
| `priory cancel TASK [--expected N]` | Drops an active task and ends its run |
| `priory revive TASK [--step STEP]` | Restarts a failed or dropped task |
| `priory hold TASK --reason TEXT`, `priory release TASK` | Holds a task back, then lets it go. Both take `--expected N` |
| `priory move TASK --band BAND [--expected N]` | Moves a task to another priority band. `--expected N` refuses the move once the state's revision differs from `N` |
| `priory search QUERY` | Finds tasks by what they are about |
| `priory report DIR --workflow NAME [--cost]` | Prints outcome rates, or tokens and cost, per step |
| `priory costs DIR [--by workflow\|agent\|model\|machine\|repo] [--json]` | Prints this week's cost against last week's, per day with merges, and split by one dimension |

Run `priory` with no arguments to print its usage.

If you work in [Herdr](docs/adapters/herdr.md), its Desk panel shows the same desk, and its Asks
panel lists the same asks and answers them through these verbs. `adapters/herdr/install` links
them into Herdr.

A workflow can write a report for you, such as a morning dossier: `priory report show` prints the
last one. The web view's Reports panel shows it, and takes its picks and row comments as
`priory report pick` and `priory report comment` do. See [docs/daemon/reports.md](docs/daemon/reports.md).

## Use it from a phone

The web view installs as an app and can notify you of a new ask or a stopped task. A phone reaches
it over HTTPS only, because a browser offers notifications only on a secure origin. This section
uses [Tailscale](https://tailscale.com/kb/1242/tailscale-serve), so the daemon stays on loopback
and only your tailnet can connect. It assumes Tailscale runs on the machine and on the phone.

1. Serve the web view on loopback. In `factory.toml`:

   ```toml
   [serve]
   listen = "127.0.0.1:7340"
   web = true
   ```

   Set `PRIORY_TOKEN` in the daemon's environment, as [serve.md](docs/daemon/serve.md#the-token)
   shows, and restart `priory daemon`.

2. Put Tailscale's HTTPS proxy in front of it:

   ```sh
   tailscale serve --bg 7340
   ```

   It prints the address, such as `https://runner.example.ts.net`, and keeps serving across
   reboots. `tailscale serve --bg 7340 off` turns it off. Turning it on exposes the view to every
   device of your tailnet, so decide it for each machine.

3. On the phone, open that address and paste the token.

4. Install the view:
   - iPhone or iPad: tap Share, then Add to Home Screen, and open Priory from the Home Screen.
     iOS lets only the installed view get notifications.
   - Android: tap Install Priory under More, then Notifications, or Install app in Chrome's menu.

5. Under More, then Notifications, tap Turn on notifications, and allow them.

A new ask now shows its title on the phone, or its task and step, and tapping it opens the ask. A task that turns blocked
or failed shows `task <key> <state>`. A snoozed item sends nothing. Turn notifications off on the
same screen.

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
Priory could not confirm, a banner when the queue is stalled, and one when the intake fails to poll
or stopped. See
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
