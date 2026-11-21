<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="design/logo/lockup-dark.svg">
    <img src="design/logo/lockup.svg" alt="Priory" height="40">
  </picture>
</h1>

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

## The web view, as designed

Milestone M2 adds a web view that works on a phone. These are its design boards from
[DESIGN.md](DESIGN.md); the view itself is not built yet.

![Three phone screens in the Indigo theme, light mode. The desk shows "3 asks need you", saved views and three asks, each with what it recommends. An ask's detail shows the question "Merge #212 into main? Checks are green at a1f3c9e.", the recommendation and three options. The same ask scrolled into its brief shows rendered Markdown and a sticky answer bar.](docs/images/web-view-phone.png)

![The desk on a 1440 px screen in the Indigo theme, dark mode: a rail of views, the list of asks and stopped tasks with what each recommends, and a drawer on the right with the ask's question, recommendation, options with keys 1 to 3, and the agent's reasoning.](docs/images/web-view-desktop.png)

![Two phone screens in the Parchment theme, light mode. The asks list shows open and answered asks with facet chips. The answer sheet shows the Merge option armed with "Tap again to run", the exact command it will run and its target, over a dimmed backdrop.](docs/images/web-view-parchment.png)

## Get started

- [GETTING_STARTED.md](GETTING_STARTED.md): build Priory, write a first factory, run it, and the
  day to day commands and concepts. Once the factory has a `[request]` table,
  `priory add "Fix the typo in the README"`, or `N` in the web view, queues a first task. Its
  [phone section](GETTING_STARTED.md#use-it-from-a-phone) installs the web view on a phone, with
  notifications for new asks.
- [CONTRIBUTING.md](CONTRIBUTING.md): how to change Priory, the checks to run, and how pull
  requests work.

## Status

Priory is in early development. The milestones come from [docs/prd.md](docs/prd.md#milestones).

| Milestone | Scope | Status |
| --------- | ----- | ------ |
| M0 | One machine, tmux, Claude Code, CLI and GitHub intake, inbox, workflows, waits, asks, results, health | Done |
| M1 | Linear, Slack replies, schedules, queue order, holds, revive, lanes, bands, memory gate, agents slice, retiring tasks, traces, reports, nudges, repair, Codex | Done |
| M2 | Web view, webhooks, terminal attach, command answers, OpenTelemetry export, search | In progress: terminal attach, OpenTelemetry export and search have landed |
| M3 | Several machines, state adapters, shared skills and memories | In progress: runners own tasks through claims and leases on one shared log |
| M4 | Risk estimate, scorers and benchmarks | Not started |

## Documentation

- [docs/prd.md](docs/prd.md): what Priory does, and in which milestone.
- [docs/reference/](docs/reference/README.md): every key of the factory's files, every intake
  message, and their JSON Schemas.
- [docs/daemon/](docs/daemon/README.md): how the daemon runs tasks, one file per topic.
- [docs/intake/](docs/intake/README.md): the messages the intake publishes, one file per source.
- [docs/trace-schema.md](docs/trace-schema.md): the format of trace records.
- [docs/adapters/herdr.md](docs/adapters/herdr.md): the Herdr adapter, Priory's panels as Herdr
  mods: Desk, Asks, Queue, Bonjour, Costs and Flows.
- [DESIGN.md](DESIGN.md): the design system of the web view.

## License

MIT. See [LICENSE](LICENSE).
