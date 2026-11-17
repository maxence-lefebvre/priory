# Priory: product requirements

Status: draft, 2026-11-17.

## Summary

Priory runs a software factory on machines you own. A factory is a directory of files: agents,
triggers, runners, workflows, views and skills. Priory reads it and turns events into tasks. An event
can be an issue, a pull request change, a schedule or a person's request. Priory places each task's
agent on a machine with room and carries the task through its workflow, usually to a pull request
that a person merges.

An agent that waits on CI, a review or an answer holds no process. It retires and resumes when the
event it named arrives. Every workflow step declares who decides it and what it changes, so a
factory can be linted before it runs and measured after.

With Priory you can leave coding work running unattended, answer its questions in one place, and
come back to pull requests you can review.

## Why

Fleet, the author's private factory, runs coding agents every day on one desktop. It is about 24,000
lines of Rust (2026-11-17). It taught four things:

1. Most of the work goes into keeping agents moving without a person watching: waits, wakes,
   stalls, crashes, full disks and memory pressure.
2. A coordinator agent that polls spends tokens on every tick. A deterministic daemon that starts
   short agent sessions for judgement is cheaper and easier to predict.
3. People need one place to answer agents. A question asked in an agent's own terminal blocks the
   agent and goes unseen.
4. Without typed steps, nobody can say how risky a workflow is or whether a change made it better.

Other tools cover parts of this. From their public pages on 2026-11-17:

- [Warp Factories](https://www.warp.dev/factories) define the factory as code, with scorers and
  benchmarks. Runs execute in Warp's cloud or the customer's VPC and are priced per run.
- [Gas City](https://github.com/gastownhall/gascity) is open source. It runs a work graph of
  formulas and beads on many runtimes.
- [herdr-bowtie](https://github.com/flupke/herdr-bowtie) and
  [herdr-bus](https://github.com/flupke/herdr-bus) run tasks across machines with offline
  messaging. They are built around one structure of leads and chiefs of staff, on Herdr and jj.

Priory aims at the gap: open source, on a few machines you own, factory as code, agents that sleep
without a process, and workflows you can measure.

## Users

The primary user is an engineer who runs coding agents on one to three machines, such as a
desktop, an always-on box and a laptop. They work alone or in a small team. They want agents to
keep working while they are away, and they want to answer agents from any device.

Out of scope for now: a hosted service, Kubernetes, and one factory shared by dozens of people.

## Principles

1. The factory is files in a repository. Changing it is a pull request.
2. Code decides what code can decide. Models judge; they do not schedule.
3. A waiting agent holds no process.
4. Every step declares its decider and its effect. An irreversible step runs only after a person
   approved that step, unless the factory marks it autonomous with a reason.
5. Offline is normal. A machine that goes away and comes back loses nothing.
6. Code gets a swappable adapter boundary only in the milestone that brings its second adapter.
7. Anything a person must answer or unblock goes to the inbox, never only to a terminal.

## Concepts

The Warp column comes from Warp's public page and is blank where that page says nothing.

| Concept    | Meaning                                                                   | Warp         | Fleet            |
| ---------- | ------------------------------------------------------------------------- | ------------ | ---------------- |
| Factory    | The directory of files that defines everything below                     | factory      | skills and nix   |
| Agent      | Who runs: harness, model, effort, MCP servers, skills, prompt, session   | agent        | worker, handler  |
| Runner     | A machine that runs agents, named by its Tailscale hostname              | runner       | the host         |
| Trigger    | An event source, a filter and the workflow its events start              | automation   | watch            |
| Workflow   | The typed steps a task goes through                                       |              | brief and rules  |
| Task       | One unit of work, from intake to an outcome                               |              | pick             |
| Run        | One agent process working on a task, from start to retire or exit        | run          | worker launch    |
| Session    | An agent's conversation, which survives the runs that resume it          |              | session          |
| Event      | Something that happened: a poll result, a webhook, a schedule, an answer | delivery     | inbox row, event |
| Wait       | The conditions an agent names before it retires                           |              | `retire --on`    |
| Ask        | A question to a person, with options, a recommendation and a trade-off    | review gate  | ask              |
| Result     | The immutable record of a run's outcome and its evidence                  |              | result           |
| Trace      | One record per step: decider, effect, outcome, tokens and time            |              |                  |
| View       | A layout of panels that shows the factory and acts on it                  |              | Herdr mods       |

## Configuration

```
factory.toml                 # or factory.yaml
agents/<name>/agent.md
triggers/<name>.toml         # or .yaml
workflows/<name>.toml        # or .yaml
runners/<tailscale-hostname>.toml
views/<name>.toml
skills/<name>/SKILL.md
webhooks/  scorers/  benchmarks/
```

Markdown files carry frontmatter fenced by `---` for YAML or `+++` for TOML. Standalone files pick
their format by extension. Both formats share one schema, defined in code and published as JSON
Schema. YAML is parsed as YAML 1.2, so `on` and `no` stay strings. Durations and sizes are strings,
such as `"15m"` and `"4GB"`.

`agents/implementer/agent.md`:

```markdown
+++
description = "Implements one task and opens its pull request."
harness = "claude-code"
model = "sonnet"
runner = "any"
session = "per-task"         # or "shared": one conversation across tasks, such as a lead
memory = "6GB"               # what a runner must have free to admit a run
mcpServers = ["github"]
+++
# Implementation
Read the issue and acceptance criteria, make the smallest correct change, test it.
```

`runners/xps13.toml`, named by the machine's Tailscale hostname. Capacity is measured live, so the
file holds no limits:

```toml
platform = { os = "linux" }
```

A trigger names one event source, its filter and the workflow it starts. The workflow names the
agent of each step. `key` says which events are the same work: a factory holds at most one active
task per key. Priory scopes each key to its source, and a GitHub key to its repository, so issue 42
of two repositories stays two tasks.

`triggers/linear-agent-ready.toml`:

```toml
source = "linear.view"
view = "agent-ready"         # a saved Linear view
on = "issue.entered"         # an issue entered the view
workflow = "implement"
key = "issue.id"
```

`triggers/agent-label.toml`:

```toml
source = "github"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
```

### Workflows

A workflow starts at `start`. Reaching `done` ends the task as done, and reaching `stop` ends it as
dropped. Each step has one decider, one effect and one way to produce its outcome:

| `decides` | What runs                                  | Its outcome comes from                    |
| --------- | ------------------------------------------ | ----------------------------------------- |
| `model`   | `agent`, with the step's `prompt`          | the `outcome` field of the run's `result` |
| `human`   | an ask with the step's `options`           | the option the person picks               |
| `code`    | a built-in `action`, such as `open-pr`     | the action's status                       |

`next` maps each outcome to a step id, `done` or `stop`. `priory check` rejects an unknown step, an
outcome with no `next`, a step no path reaches, and an irreversible step without the approval the
next section describes.

`workflows/implement.toml`:

```toml
start = "plan"

[[steps]]
id = "plan"
decides = "model"
agent = "implementer"
prompt = "Plan the change. Grade it red, yellow or green."
effect = "reversible"
next = { red = "approve-plan", yellow = "implement", green = "implement" }

[[steps]]
id = "approve-plan"
decides = "human"
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

### Approval of irreversible steps

An irreversible step needs either a human step that lists it in `approves` on every path that
reaches it, or `autonomous = "<reason>"` on the step itself. An approval covers one task and the
commit it was given for. An answer grants the approval when its `next` leads toward the approved
step. If the task's commit changes before the irreversible step runs, Priory
asks again.

## Requirements

Milestones are defined below. Each requirement names the milestone that delivers it.

### R1. Configuration

- R1.1 `priory check` validates a factory in either format and reports errors with file and line. M0
- R1.2 `priory check` enforces the workflow rules and the approval rule above. M0
- R1.3 `priory draw` renders a workflow in the decider and effect notation. M1

### R2. Events and triggers

- R2.1 Load triggers from `triggers/` and start the named workflow for each matching event. M0
- R2.2 Accept tasks from the CLI, and GitHub events: issues, pull requests, check runs, reviews and
  merges. M0
- R2.3 Deliver each event at least once and drop duplicates by event id. M0
- R2.4 Start no task for an event whose `key` already has an active task. A task is active in any
  state except done, failed and dropped. Record the event on that task instead, where a wait can
  pick it up. M0
- R2.5 Trigger on Linear saved views (an issue enters or leaves) and on Linear issues and comments.
  Poll the Slack threads that waits name. M1
- R2.6 Trigger on schedules. M1
- R2.7 Accept signed webhooks and check the signature before any trigger sees the payload. M2

### R3. Tasks and scheduling

- R3.1 A task moves through queued, ready, running, waiting, blocked, done, failed and dropped. M0
- R3.2 A runner admits a run only when its live free memory, disk and CPU leave room for the agent's
  declared needs. M0
- R3.3 Tasks have priority bands. A higher band only starts earlier: it never skips the room
  check. M1
- R3.4 A task can be held until a time, another task's end, a pull request's merge, or its issue's
  last open blocker. M1
- R3.5 Heavy commands inside a run queue behind one memory gate:
  `priory memory acquire --gb N -- <command>`. M1

### R4. Agent runtime

- R4.1 Start a run in its own git worktree, under one terminal host and one harness: Claude Code in
  M0, Codex in M1. M0
- R4.2 The terminal host owns agent processes. The daemon can restart without stopping them, and it
  reconnects to them when it comes back. M0
- R4.3 A `per-task` session starts with the task and ends with it. A `shared` session carries one
  conversation across tasks. Either way, a waiting session holds no process. M0
- R4.4 Prompts go through a stored outbox and are typed only when the agent is idle. Priory records
  each attempt before it types. After a crash with an attempt and no confirmation, Priory marks the
  prompt uncertain in the inbox instead of typing it again. M0
- R4.5 Resume a retired session with the waking event as its prompt. M0
- R4.6 A second terminal host. M1
- R4.7 Let a person attach to any run's terminal from a view. M2

### R5. Agent tools: wait, ask, result

Priory serves these as MCP tools.

- R5.1 `wait(on)` retires the agent until one condition fires. M0 conditions: a pull request merges
  or closes, its checks finish, it gets a review or comment, an ask is answered, a time passes,
  another task ends. M1 adds Linear comments and states, Slack replies, and free disk.
- R5.2 A wait is stored before the agent retires. Priory then checks whether the condition already
  holds for events after the agent's last prompt, and wakes it at once if so. A task has at most one
  running run. M0
- R5.3 `ask` records a question with options, a recommendation and a trade-off, then retires the
  agent. An answer carries the question revision it read, and an answer to a changed question is
  refused. The first answer to a revision stands, and later ones are refused. M0
- R5.4 A hook denies the harness's built-in question tool and points to `ask`. M0
- R5.5 `result` stores an immutable record for the run, with its outcome and evidence. M0
- R5.6 Priory marks a task done when its workflow reaches `done`. For a task with a pull request,
  that also needs the pull request head to match the result's commit, and every required check at
  that head to have finished green. Until then the task waits. M0
- R5.7 When a turn Priory started ends without `wait`, `ask` or `result`, the stop hook sends one
  nudge. M1
- R5.8 An ask can carry a command that the person's answer runs. The answer is bound to the command,
  its target and the question revision, runs at most once, and reports an uncertain outcome if
  Priory crashes mid-run. Views need authentication first. M2

### R6. Health and housekeeping

- R6.1 Every tick checks each run for an unexpected exit, a timeout and a blocked permission prompt.
  A run that retired through `wait` or `ask`, or ended with a `result`, is not unexpected. Each verdict moves the task to blocked or failed and adds an inbox item with resume and cancel
  actions. M0
- R6.2 Priory never approves a permission prompt. It denies it, tells the agent why, and adds an
  inbox item if the task cannot go on without it. M0
- R6.3 Remove a worktree only when Priory created it, its branch merged, it has no unpushed commits,
  no uncommitted changes and no untracked files, and no process runs in it. Report every other worktree instead. M0
- R6.4 A short handler session reads a stalled agent's screen and nudges or restarts it. M1
- R6.5 A failed task gets one repair attempt per run. The repair restarts the agent only on a cause
  it changed or confirmed gone. M1

### R7. Views

- R7.1 The CLI covers every action, the inbox included. M0
- R7.2 A view is a layout file in `views/`: a tree of rows, columns and tabs whose leaves are
  panels, in the manner of Herdr's layouts. The user can change every view, including the
  defaults Priory ships. M2
- R7.3 Priory ships panels for the inbox, tasks, runs, agent terminals, workflow diagrams and
  reports. A panel reads the factory's state and acts through the same commands as the CLI. M2
- R7.4 Views render in a browser and work on a phone. M2
- R7.5 Any machine of the factory can serve the views and act on the factory's state. M3
- R7.6 A second view host, such as a Herdr layout, renders the same view files as an adapter. M3

### R8. Several machines

- R8.1 A factory spans several runners. Each task is owned by one runner at a time, which places its
  runs. M3
- R8.2 A runner keeps its tasks going while the other machines are offline. Answers given on another
  machine meanwhile are delivered when it reconnects, through the revision check in R5.3. M3
- R8.3 State goes through an event log with cursors, plus requests with a deadline, a take and a
  reply. SQLite backs it from M0. Postgres and herdr-bus adapters arrive in M3, with one contract
  test suite run against each.

### R9. Measurement

- R9.1 Every step emits a CloudEvents trace: factory, workflow, task, run, step, decider, effect,
  outcome, tokens, wall time and machine. M1
- R9.2 Reports give per-step outcome counts and rates. A rate's denominator is the runs that reached
  the step, and the report shows it. M1
- R9.3 Cost per task, from token counts and a price table in the factory. M1
- R9.4 Estimate the chance that a wrong step outcome reaches an irreversible step. Its labels
  (rejected approval, red CI after merge, revert), estimator and assumptions are specified before it
  ships. M4
- R9.5 Scorers grade finished runs. Benchmarks replay past tasks under another model or
  configuration. M4

### R10. Sharing between machines

- R10.1 Skills, instructions and memories live in git and sync to every runner. M3
- R10.2 Build caches such as sccache or a Nix cache belong to the runner's environment. Priory
  documents them and does not run them.

## Architecture

- One thread owns the state. It takes one input at a time and returns effects. Effects run
  elsewhere and come back as inputs. A state change and its history event commit in one
  transaction.
- Prompts and slow effects are stored, so a daemon restart resumes them where they stood.
- Agents outlive the daemon. Their MCP tools reach it through a stdio bridge on a local socket.
- Four boundaries: state, agent runtime, event sources and views. Each becomes a swappable adapter
  only in the milestone that brings its second adapter.
- Written in Rust. The first version reuses Fleet's code where it fits.

## Milestones

| Milestone | Scope                                                                                    | Done when                                                                                                            |
| --------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| M0        | One machine, one harness, CLI and GitHub intake, workflows, waits, asks, results, health | The example workflow above runs end to end, with the agent retired on the `approve-pr` ask until it is answered, the task waiting on the pull request's checks, a daemon restart, a duplicate event and a failed run |
| M1        | Linear, schedules, holds, bands, memory gate, traces, reports, nudges, repair, Codex, a second terminal host | The author's own factory runs on Priory for one week                                                     |
| M2        | Web view, webhooks, terminal attach, command answers                                     | One working day of asks answered from a phone only                                                                   |
| M3        | Several machines, state adapters, shared skills and memories                             | A runner is disconnected for an hour while tasks run, then reconnected, with no task run twice and no answer lost    |
| M4        | Risk estimate, scorers and benchmarks                                                    | A configuration change is compared on replayed tasks before it merges                                                |

## Success measures

- Minutes of a person's attention per merged pull request.
- Share of tasks that reach a pull request with no prompt from a person beyond asks.
- Memory held by waiting agents, which should be zero.
- Time to add a machine to a factory.

## License

MIT.

## Non-goals

- A hosted service or billing.
- Kubernetes.
- Merging pull requests by default. A factory can allow it per workflow, with a reason.
- Replacing CI.
- A chat interface.

## Open questions

1. Which terminal host comes first: tmux, which most users have, or Herdr, which the author uses.
2. The default multi-machine state backend: a central Postgres or one herdr-bus per machine.
3. Whether to propose the trace schema to herdr-bowtie early, so both factories can be measured the
   same way.
4. Whether workflow branching stays limited to outcome labels or gains expressions.
5. Whether to support jj workspaces alongside git worktrees.
6. Whether a Windows machine runs agents through WSL, and whether a runner file then needs more than
   `platform`, such as an environment loader like direnv.
7. Whether third-party panels load as plugins, and in which form.

## Risks

- Scope. herdr-bowtie holds about 147,000 lines of Rust and Markdown (2026-11-17). Each milestone
  has a done-when line, and adapter boundaries wait for a second adapter.
- Harness changes. Claude Code's hooks and flags change often, and Priory depends on them.
- Security. Agents run with the user's credentials. Webhook payloads and issue text are untrusted.
  Secrets are resolved at runtime and never stored in the factory.
