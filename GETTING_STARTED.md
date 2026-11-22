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
`approve-pr` that approves it. Once tasks ran it, the row adds its history: how many tasks a person
stopped at the gate, and how many were judged wrong within 30 days. With no state file it shows `-`.
See [workflows.md](docs/reference/workflows.md#irreversible-steps-and-their-gates).
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
[adding-a-second-machine.md](docs/adding-a-second-machine.md). To give each machine's agents a build
cache, such as sccache, follow [sharing-build-caches.md](docs/sharing-build-caches.md).

## 4. Pick a starter

Start from a ready-made factory instead of writing one: see
[How to start from a starter factory](docs/starting-from-a-starter.md).

### Adopt a writing guide

Seven starters ship a `WRITING.md`, a short guide on how an agent writes asks, reports and results
for someone who was away, and name it in `factory.toml` with `writing = "WRITING.md"`. Priory
appends its text to the system prompt of every agent it starts. To use one in your own factory,
copy the file next to `factory.toml`, edit it to your voice, add the key, and run `priory check`.
See [writing.md](docs/daemon/writing.md) for what the guide changes.

### Share facts with every run

A fact every run should know, such as the day releases go out, goes in its own file under
`memories/`, with a `name` and a `description` in its frontmatter. Each run gets an index of the
memories and reads a file when its description fits the task. `priory check` counts them in its
`ok` line. To change a memory, edit the file and merge it like any other change to the factory.
See [memories.md](docs/reference/memories.md) for the file's keys.

Agents save memories too, with the `remember` tool. The task that saved one gets it at once; every
other task gets it once you accept it. The desk lists the candidates that wait. Open that row on
the web or Herdr Desk to read each one and accept or drop it, or run `priory memories accept ID` or
`priory memories drop ID` (see
[skills.md](docs/daemon/skills.md#memories-agents-write)).

### Share a kit across factories

To keep skills, instructions and memories in their own repository, name it under `[kit]` in
`factory.toml`, with `repo` and the `ref` to follow. Each runner fetches it at start and every five
minutes, with git's own credentials, and every run gets its files before the factory's. A failed
fetch keeps the last good commit and shows a banner on the desk. See
[skills.md](docs/daemon/skills.md#a-kit-repository) for the details.

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
[How to run Priory as services](docs/running-as-services.md).

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
| `priory answer ASK OPTION --revision N` | Answers an ask, at the revision `priory asks` prints for it |
| `priory tasks` | Lists tasks in start order |
| `priory task TASK --json` | Prints one task with its brief, plan, pull request, runs and asks |
| `priory actions` | Lists each code step's action with its error and output: read it when a code step failed |
| `priory attach TASK [--read-only]` | Attaches your terminal to a task's agent |
| `priory say TASK TEXT` | Sends a task's agent a message, waking it if it sleeps |
| `priory resume TASK` | Resumes a blocked task |
| `priory cancel TASK` | Drops an active task and ends its run |
| `priory revive TASK` | Restarts a failed or dropped task |
| `priory pause [REASON]`, `priory unpause` | Stops new tasks from starting, then lets them start again |

[The command reference](docs/reference/cli.md) lists every command with its flags. Run `priory`
with no arguments to print its usage.

If you work in [Herdr](docs/adapters/herdr.md), its Desk panel shows the same desk, and its Asks
panel lists the same asks and answers them through these verbs. `priory views --json |
adapters/herdr/install` links them into Herdr and writes a layout from each view, `priory` for
your `default` view and `priory-NAME` for each other: narrow panels as asides you toggle, wide ones
as center tabs, and Herdr's own Files and Changes panels. `priory check` warns when those layouts
were built from other view files, and the Priory bar offers to regenerate them. On a Herdr that reads its folder through
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
same tasks and runs, and the Herdr Tasks and Runs panels show them too. In a factory of several
runners, both Tasks panels also list the tasks the other runners hold and filter by runner. Such a
task's page stays on its runner, so a tap opens its row in place, with the actions its hold allows:
Resume, Cancel, Release and Say for a `kept` task, Cancel for a `waiting` one. Each goes to its
runner through the log, and the row then shows "Sent to laptop: cancel", or what laptop did with it.

The Queue panel, under More on a phone, shows Up next, then each band's waiting tasks in the order
they start, with why each one waits. Its Move menu moves a task to the top of its band, up or down
one, or to another band, as `priory move` does. `priory queue` prints the same queue.

A workflow can write a report for you, such as a morning dossier: `priory report show` prints the
last one, and `priory report list` lists the earlier ones. The web view's Reports panel shows it, and takes its picks and row comments as
`priory report pick` and `priory report comment` do. See [docs/daemon/reports.md](docs/daemon/reports.md).

The web view's Costs panel shows what the factory spent this week against last week, each day, and
split by workflow, agent, model, machine or repository, as `priory costs` prints it. Beside it,
Attention shows the minutes of your attention per merged pull request, how long asks waited for
you, and where each task's time went, as `priory report attention` prints it. On a phone, both sit
under More. The Herdr Costs panel shows the same. See [docs/daemon/trace.md](docs/daemon/trace.md#costs).
Once you grade runs with `priory score`, the Costs panel adds each scorer's pass rate under the
split, and a task's page lists each run's scores under it. Herdr's Costs and Task panels show the same. See
[docs/daemon/score.md](docs/daemon/score.md#where-scores-show).

## Use it from a phone

To reach the web view from a phone, install it there and get notifications, follow
[How to use the web view from a phone](docs/using-the-web-view.md).

## Concepts

[Concepts](docs/concepts.md) explains each term this guide uses: factory, trigger, workflow, task,
run, ask, desk and queue.
