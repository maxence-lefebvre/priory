# Getting started

This guide takes you from nothing to a running factory. You install Priory, write a factory with
one agent, one workflow and one trigger, check it, then run the intake and the daemon.

## Before you start

Priory needs Linux and, on `PATH`: `git`, `tmux`, `gh` logged in to GitHub, and `claude` or
`codex`. Building from source also needs the Rust toolchain that `rust-toolchain.toml` pins.

## 1. Install Priory

```sh
curl -fsSL https://raw.githubusercontent.com/maxence-lefebvre/priory/main/install.sh | sh
priory --version
```

The script downloads the binary of the latest
[release](https://github.com/maxence-lefebvre/priory/releases) for Linux x86_64, Linux aarch64 or
macOS on Apple silicon, checks it against its `.sha256` and installs it to `~/.local/bin`. The
`.sha256` sits beside the binary in the same release, so it only catches a broken download. When
[gh](https://cli.github.com/) is on your `PATH` and signed in, the script also runs
`gh attestation verify`, which proves this repository's release workflow built the binary, and
refuses a binary it cannot verify. Without gh, it prints a note and skips that check. Releases
made before the workflow attested its builds, such as `v0.1.0`, fail that check. Run
`sh install.sh --prefix /usr/local` from a clone to install to `/usr/local/bin` instead. Set
`PRIORY_RELEASE_URL=https://github.com/maxence-lefebvre/priory/releases/download/v0.1.0` to pick a
release other than the latest. With [cargo-binstall](https://github.com/cargo-bins/cargo-binstall),
`cargo binstall --git https://github.com/maxence-lefebvre/priory priory` downloads the same binary.

Each release also holds `priory-herdr.tar.gz`, the [Herdr adapter](adapters/herdr/), and
`priory-github-extension.zip`, the [GitHub extension](adapters/github/README.md).

### Build from source

The release binaries leave out the cargo features. Build from your clone to get them, or to run
an unreleased commit:

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"
priory --version
```

To install the binary instead, `cargo install --path crates/priory` puts `priory` in
`~/.cargo/bin`. The in-process embedding model for search sits behind a cargo feature:
`cargo build --release --features model`. So does the Postgres event log, which lets several
machines share one log: `--features postgres`, then `[log]` in
[`factory.toml`](docs/reference/factory.md#log).

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
The web view's Flows panel, under More, draws each workflow as a timeline, with the paths that
reach an irreversible step with no person's decision highlighted. `priory flows example --json`
prints what it draws, and the [Herdr](docs/adapters/herdr.md) Flows panel draws the same.

The same rule holds inside a model step. Priory's tool guard denies an agent's irreversible call,
such as `gh pr merge` or `git push origin main`, and tells the agent to `ask`. The call passes when
its step names the guard in `guard.allow` and is approved or `autonomous`, or when its workflow's
`guard.allow` names it. A merge or an approving review needs an approved step. See [guard.md](docs/daemon/guard.md).

An outcome can need proof too. Set `gate = "cargo test"` in `factory.toml` and
`proof = { passed = "gate-passed" }` on the `implement` step. The agent then runs `priory gate`,
and the daemon refuses `passed` until the gate passed at the branch's head. `priory check --explain`
lists the outcomes it verifies. See Proofs in [tools.md](docs/daemon/tools.md#proofs).

To play a task through the whole factory with fake agents before a real one runs, follow
[testing-a-factory.md](docs/testing-a-factory.md).

To run one factory on two machines that share one event log, follow
[adding-a-second-machine.md](docs/adding-a-second-machine.md).

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
| [`release-checklist`](examples/release-checklist/README.md) | A runbook: you start a release with `priory add`, and each step asks you. Answering `run` at the test step runs `cargo test` once |
| [`groom`](examples/groom/README.md) | A `groom` label starts a task. The agent grades the ticket, writes the brief, and takes it to a pull request you review |
| [`batches`](examples/batches/README.md) | `priory batch add` hands several issues to a planner, which orders them. After you answer `go`, each issue becomes a task that starts once the ones it needs merged |
| [`ship-and-watch`](examples/ship-and-watch/README.md) | A `ship` label starts a task. After you merge its pull request, the agent watches the release for an hour, then drafts an announcement that posts once you approve it |

Copy one into your working folder, then check it:

```sh
cp -r <your clone>/examples/review my-factory
priory check my-factory
```

Replace the placeholders `owner/repo` and `your-login` in its `triggers/` files, and rename
`runners/my-machine.toml` after your machine's Tailscale hostname. In the next step, use
`my-factory` in place of `example`, and the starter's workflow, the file name under its
`workflows/`, in place of `implement`.

### Adopt a writing guide

Seven starters ship a `WRITING.md`, a short guide on how an agent writes asks, reports and results
for someone who was away, and name it in `factory.toml` with `writing = "WRITING.md"`. Priory
appends its text to the system prompt of every agent it starts. To use one in your own factory,
copy the file next to `factory.toml`, edit it to your voice, add the key, and run `priory check`.
See [writing.md](docs/daemon/writing.md) for what the guide changes.

## 5. Run it

Priory keeps its state in three places, each named by an environment variable:


| Variable       | Holds                                                         | Default                          |
| -------------- | ------------------------------------------------------------- | -------------------------------- |
| `PRIORY_DB`    | The daemon's SQLite state: tasks, runs, asks, waits, results  | none: the daemon refuses to start |
| `HERDR_BUS_DB` | The [herdr-bus](https://github.com/flupke/herdr-bus) file the intake publishes to, unless `[log]` names Postgres | `~/.local/share/herdr-bus/bus.db` |
| `PRIORY_HOME`  | Clones and worktrees; it names the daemon's socket            | `~/.local/state/priory-oss`      |

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
   the agent with the failure, in the same conversation, so it can fix it. An agent can split a
   large change into a stack of pull requests with the `branches` field of its result; a stack is
   done once every pull request merged (see [Stacks](docs/daemon/routing.md#stacks)).

## Day to day

| Command | What it does |
| ------- | ------------ |
| `priory desk` | Lists what waits on you: asks, blocked and failed tasks, held tasks, the queue |
| `priory answer ASK OPTION --revision N` | Answers an ask. `--expected N` is the same flag. `priory answer [DIR] RUNNER:ASK ...` answers another runner's ask (see [runners.md](docs/daemon/runners.md)) |
| `priory give [DIR] KEY RUNNER [--force]` | Moves a task to another runner of a factory of several, once its runner's lease ends or with `--force` (see [runners.md](docs/daemon/runners.md)) |
| `priory dismiss [--expected N] ASK` | Closes an ask with no answer |
| `priory deliver ASK (--retry \| --clear)` | Sends a failed answer delivery again, or clears it from the desk |
| `priory snooze KIND ID --for 2h`, `--until TIME`, `--clear` | Hides a desk item until then, or brings it back |
| `priory todo add TEXT [--band B] [--due TIME]`, `priory todo done ID`, `priory todo list` | Adds a to-do of your own to the desk, marks it done, or lists them |
| `priory tasks [--lane LANE] [--grade GRADE]` | Lists tasks in start order, with their grade |
| `priory batch add [DIR] ISSUE...`, `priory batch show [BATCH] [--json]`, `priory batch edit BATCH --expected N ...` | Hands several issues to a planner that orders them under one go, shows each batch and its entries, and drops or moves an entry before the go (see [batches.md](docs/daemon/batches.md)) |
| `priory tasks --graph [TASK] [--mermaid]` | Draws how tasks relate, as a tree or a Mermaid flowchart |
| `priory relate TASK KIND OTHER [--remove]` | Records that a task `builds-on`, `fixes`, `takes-up` or `relates` to another, or removes it |
| `priory task TASK --json` | Prints one task with its brief, plan, pull request, runs and asks |
| `priory attach TASK [--read-only]` | Attaches your terminal to a task's agent |
| `priory screen TASK [--json]` | Prints what a task's agent shows now, with secrets hidden |
| `priory resume TASK` | Resumes a blocked task |
| `priory say TASK TEXT` | Sends a task's agent a message, waking it if it sleeps |
| `priory subscribe TASK RESOURCE [--expected REVISION]`, `priory unsubscribe TASK RESOURCE` | Routes a pull request's, Linear issue's or Slack thread's events to a task until it closes, or stops routing them. Every task already subscribes to its own key, such as the Slack thread that started it. `--expected` refuses the subscribe once the state moved past that revision |
| `priory pause [REASON]`, `priory unpause` | Stops new tasks from starting, then lets them start again |
| `priory cancel TASK [--expected N]` | Drops an active task and ends its run |
| `priory revive TASK [--step STEP]` | Restarts a failed or dropped task |
| `priory hold TASK --reason TEXT`, `priory release TASK` | Holds a task back, then lets it go. Both take `--expected N` |
| `priory move TASK --band BAND [--expected N]` | Moves a task to another priority band. `--expected N` refuses the move once the state's revision differs from `N` |
| `priory search QUERY` | Finds tasks by what they are about |
| `priory report DIR --workflow NAME [--cost]` | Prints outcome rates, or tokens and cost, per step |
| `priory report attention [--by workflow\|lane\|kind] [--json]` | Prints this week's person-minutes per merged pull request, how long asks waited on you, and where each task's time went |
| `priory costs DIR [--by workflow\|agent\|model\|machine\|repo] [--json]` | Prints this week's cost against last week's, per day with merges, and split by one dimension |
| `priory grades [--since WINDOW] [--json]` | Prints how each grade's tasks turned out, headed by green's one-shot rate against 90 % |
| `priory report risk [DIR] [--workflow NAME] [--json]` | Prints, for each irreversible step, the share of tasks through it judged wrong and landed wrong within 30 days |
| `priory label [DIR] TASK wrong --evidence URL`, `--clear` | Marks a task's work wrong for the risk report, then withdraws the mark |

Run `priory` with no arguments to print its usage.

If you work in [Herdr](docs/adapters/herdr.md), its Desk panel shows the same desk, and its Asks
panel lists the same asks and answers them through these verbs. `priory views --json |
adapters/herdr/install` links them into Herdr and writes a layout from your `default` view: narrow panels as asides you toggle, wide ones as center tabs, and Herdr's own Files and Changes panels. `priory check`
warns when that layout was built from other view files. On a Herdr that reads its folder through
Syncthing, which carries no links, use `install --copy`.

If you review on GitHub, the [GitHub extension](adapters/github/README.md) shows the task that holds
a pull request on its page, answers its asks, says to its agent, and walks you through the agent's
review guide. Load `adapters/github/` unpacked in Chrome or Edge, and add its origin to
[`[serve] extension_origins`](docs/reference/factory.md#serve).

The web view's Desk shows the same desk. It opens on the next thing: the alarm or ask that ranks
first, with why it ranks there. Under the list, Since you looked shows what happened since your last
visit, and opening the Desk records the visit as `priory seen` does. `priory desk` prints the same
events after its list.

The web view's Asks panel has the saved views Open, Snoozed, Answered and All. A snoozed ask leaves
Open and waits in Snoozed. On a phone, once the options scroll out of view, the answer bar's
"N options" opens a sheet with every option. On a desktop, `S` snoozes the selected ask for 1 h.
A batch's go ask shows the plan as a lane graph, and each entry's menu moves or drops it before you
answer. The desk's batch row opens the batch page, with each entry's state and when it starts.

The web view's Tasks panel lists every task, with the saved views Active, Blocked and Ended and
filters by state, lane and workflow; each row opens the task's page, and a batch's entries link to
the batch. The Runs panel lists the runs of the last 7 days with their agent, model, outcome,
duration and tokens, filtered by outcome, agent or step. `priory tasks` and `priory runs` print the
same tasks and runs.

The Queue panel, under More on a phone, shows Up next, then each band's waiting tasks in the order
they start, with why each one waits. Its Move menu moves a task to the top of its band, up or down
one, or to another band, as `priory move` does. `priory queue` prints the same queue.

A workflow can write a report for you, such as a morning dossier: `priory report show` prints the
last one. The web view's Reports panel shows it, and takes its picks and row comments as
`priory report pick` and `priory report comment` do. See [docs/daemon/reports.md](docs/daemon/reports.md).

The web view's Costs panel shows what the factory spent this week against last week, each day, and
split by workflow, agent, model, machine or repository, as `priory costs` prints it. Beside it,
Attention shows the minutes of your attention per merged pull request, how long asks waited for
you, and where each task's time went, as `priory report attention` prints it. On a phone, both sit
under More. The Herdr Costs panel shows the same. See [docs/daemon/trace.md](docs/daemon/trace.md#costs).

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
or failed shows `task <key> <state>`, and a failing intake shows `the intake fails to poll <source>`. A snoozed item sends nothing. Turn notifications off on the
same screen.

On the machine that runs the daemon, `[notify] desktop = true` in `factory.toml` shows the same
title as a desktop notification, through `notify-send`, for the one next thing on the desk.

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

**Intervention.** A trigger's `intervention` sets how much a person steps in. `plan` makes a person
approve each graded plan, even a green one. `hands-off` lets a lead agent answer the task's asks in
your place, but an ask that lets an irreversible step run still comes to you. A routed ask shows
"with <lead>" in the web view and the Herdr panels, you can still answer it first, and an answered
ask says who answered. When the lead ends without answering, the ask comes back to you. See
[docs/reference/triggers.md](docs/reference/triggers.md#intervention-and-lead).

**Task and run.** A task is one unit of work, from its event to `done`, `failed` or `dropped`. A run
is one agent process working on a task's step. A task can have many runs, and its agent's session
carries over between them. See [docs/daemon/runs.md](docs/daemon/runs.md).

**Inbox and routing.** Every event the intake publishes lands in the daemon's inbox before anything
acts on it. The daemon then finds the task the event concerns, such as the task that owns the pull
request a check ran on. `priory task ID --json` lists the events routed to a task, and
`priory inbox --unrouted` lists those that reached none, as the web and Herdr Inbox panels do. See
[docs/daemon/routing.md](docs/daemon/routing.md).

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
