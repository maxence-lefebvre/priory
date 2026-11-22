<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="design/logo/lockup-dark.svg">
    <img src="design/logo/lockup.svg" alt="Priory" height="40">
  </picture>
</h1>

Priory runs coding agents, such as Claude Code and Codex, on machines you own. It turns events
into tasks: a labelled GitHub issue, a Linear issue that enters a view, a Slack message, a Sentry
issue or another signed webhook, a schedule, or a request you type. It then carries each task
through a workflow you wrote, usually to a pull request that a person reviews and merges.

Priory is a Rust program with two long-running processes. The intake polls GitHub, Linear, Slack
and your schedules. The daemon turns what the intake finds, and what webhooks deliver, into tasks,
and runs one agent per task in its own tmux session and git worktree. The daemon also serves a web
view that works on a phone, where you answer what the agents ask. A Herdr adapter and a browser
extension for GitHub's pull request page show the same asks, and ten starter factories give you a
workflow to begin from.

## The problem it solves

Running one coding agent is easy. Keeping several of them moving while nobody watches is not. An
agent waits on CI, on a review or on a person's answer. It crashes, fills the disk or runs the
machine out of memory. A question it asks in its own terminal goes unseen.

Priory handles that work in deterministic code, not in a coordinator agent that spends tokens on
every poll:

- An agent that waits holds no process. It names what it waits for, such as a pull request's
  checks, and goes to sleep. The daemon resumes its session when that event arrives.
- Every question an agent asks lands on one list, the desk, with options and a recommendation.
  You answer it from the command line, or a trigger hands its tasks' questions to a lead agent and
  keeps only the irreversible ones for you.
- Every workflow step declares who decides it (a model, a person or code) and what it changes. So
  `priory check` can refuse a workflow before it runs, for example one that pushes code with no
  person's approval. `priory check --explain` lists what a person approves before each
  irreversible step.
- A step can refuse an agent's `passed` until the project's gate, such as its tests, passed at the
  branch's head. `priory check --explain` lists the outcomes the daemon verifies.
- A workflow can be a runbook of human steps only, where an answer runs a step's command once.
  The desk also holds your own to-dos, ranked after the asks.
- Every step's outcome, tokens and time are recorded, so you can measure a workflow and its cost.

## The web view

The daemon's HTTP server, set by `[serve]` in `factory.toml`, serves a web view: the desk, asks,
the inbox, tasks, runs, the queue, flows and costs. By default it listens on this machine only. Reaching it
from a phone needs a tunnel, and which one is the owner's call:
[How to use the web view](docs/using-the-web-view.md) has the steps. These screenshots show
`priory serve` on a demo factory. [views.md](docs/reference/views.md#what-each-panel-shows) shows
every panel, and [DESIGN.md](DESIGN.md) holds the design rules.

![The desk on a 1440 px screen in light mode. A rail of views on the left, the list of seven open asks, three of them from the runners laptop and xps13 with their runner tag, one stopped task and a going batch in the middle. A drawer on the right shows the ask "Open the pull request for ENG-212?", its recommendation to open, the options open and stop, and the agent's reasoning with the checks.](docs/images/web-view-desk-ask-1440-light.png)

On a phone, an ask opens as a page with its recommendation and options. Scrolling keeps an answer
bar at the bottom.

![An ask on a 390 px phone in light mode: the question, the recommendation to open, and the options open and stop.](docs/images/web-view-ask-390-light.png)

Flows draw each workflow and mark who decides each edge and which step is irreversible. Costs show
spend, merges and your attention.

![The implement workflow in Flows on a 1440 px screen in light mode, from implement to approve-pr to open-pr, with model decisions as dashed arrows, human decisions as solid arrows and a padlock on the irreversible step open-pr.](docs/images/web-view-flows-1440-light.png)

![Costs on a 1440 px screen in light mode: cost this week, merges, cost per merge, a bar for each day, a table by workflow, and the minutes of your attention per merge.](docs/images/web-view-costs-1440-light.png)

## Get started

Build Priory from a clone, with the Rust toolchain that `rust-toolchain.toml` pins:

```sh
git clone https://github.com/maxence-lefebvre/priory
cd priory
cargo build --release
export PATH="$PWD/target/release:$PATH"
```

Then follow [GETTING_STARTED.md](GETTING_STARTED.md). It takes you from this build to a factory of
your own, a first task, and a first ask answered in the web view on `127.0.0.1`. To begin from a
ready-made factory instead, see
[How to start from a starter factory](docs/starting-from-a-starter.md).

## Status

Priory is in early development. The milestones come from [docs/prd.md](docs/prd.md#milestones).
Each one names a test that says when it is done.

| Milestone | Scope | Features | Done-when test |
| --------- | ----- | -------- | -------------- |
| M0 | One machine, tmux, Claude Code, CLI and GitHub intake, inbox, workflows, waits, asks, results, health | Built | Each part is covered by an automated test with a stand-in agent |
| M1 | Linear, Slack replies, schedules, queue order, holds, revive, lanes, bands, memory gate, agents slice, retiring tasks, traces, reports, nudges, repair, Codex | Built | The one-week run on the author's factory is for the author to confirm |
| M2 | Web view, webhooks, terminal attach, command answers, OpenTelemetry export, search | Built | Still to be run by a person: one working day of asks answered from a phone only |
| M3 | Several machines, state adapters, shared skills and memories | Built, except a remote task's detail page (R8.5) | The one-hour cut was run once on 2026-11-21 and no task ran twice. The procedure is in [adding-a-second-machine.md](docs/adding-a-second-machine.md) |
| M4 | Risk estimate, scorers and benchmarks | Built | Still to be run by a person: a configuration change compared on replayed tasks before it merges. The procedure is in [bench.md](docs/daemon/bench.md#before-a-change-merges) |

## Documentation

[maxence-lefebvre.github.io/priory](https://maxence-lefebvre.github.io/priory/) holds these pages
as a website, with search, built from `main`.

- Tutorial: [GETTING_STARTED.md](GETTING_STARTED.md), from a build to a first answered ask.
- How-to guides, one task each:
  [start from a starter](docs/starting-from-a-starter.md),
  [use the web view, on a phone too](docs/using-the-web-view.md),
  [run Priory as services](docs/running-as-services.md),
  [test a factory with fake agents](docs/testing-a-factory.md),
  [add a second machine](docs/adding-a-second-machine.md),
  [share build caches](docs/sharing-build-caches.md),
  [send traces to an OpenTelemetry backend](docs/sending-traces-to-opentelemetry.md) and
  [evaluate a prompt change](docs/evaluating-a-prompt.md).
- Reference: [docs/reference/](docs/reference/README.md) lists every
  [command](docs/reference/cli.md), every key of the factory's files, every HTTP endpoint and every
  intake message, with their JSON Schemas. [docs/trace-schema.md](docs/trace-schema.md) holds the
  format of trace records.
- How it works: [docs/concepts.md](docs/concepts.md) explains the terms.
  [docs/daemon/](docs/daemon/README.md) explains how the daemon runs tasks, and
  [docs/intake/](docs/intake/README.md) how each source becomes events, one file per topic.
- Adapters: [docs/adapters/herdr.md](docs/adapters/herdr.md), Priory's panels as Herdr mods, and
  [adapters/github/README.md](adapters/github/README.md), Priory on a pull request's page: its
  task, its asks and a review guide.
- Project: [docs/prd.md](docs/prd.md) says what Priory does and in which milestone.
  [CONTRIBUTING.md](CONTRIBUTING.md) says how to change Priory, and [DESIGN.md](DESIGN.md) holds
  the design system of the web view.

## License

MIT. See [LICENSE](LICENSE).
