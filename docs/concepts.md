# Concepts

This page explains the terms Priory uses, each with the page that covers it in depth.

The drawing shows how a task moves, from an event to `done`. The terms below name its parts.

![How a task moves, from top to bottom. GitHub, Linear, Slack and schedules reach the intake, and a webhook or a request you type goes straight to the log. The daemon finds the task each message concerns, and a task request creates a task or joins the active one. A run starts a model step's agent in the task's git worktree and a tmux session. The agent ends its turn with result, wait or ask. A result picks the next step, done or stop. A wait or an ask puts the task to sleep until an event or a person's answer wakes it, and the next run resumes the agent's session.](images/diagrams/task-flow.svg)

**Factory.** The folder of files that defines everything else: `factory.toml`, `agents/`,
`workflows/`, `triggers/` and `runners/`. Optional files add to them: `views/`, `memories/`,
`skills/`, `instructions.md`, `scorers/`, `benchmarks/`, `lanes.toml`, `prices.toml` and a writing
guide. Each config file can be TOML or YAML; agents and memories are Markdown files with
frontmatter. Changing the factory is a pull request. Every key is in
[docs/reference/](reference/README.md).

**Trigger.** One event source, a filter and the workflow its events start. The intake polls the
sources `github`, `linear`, `linear.view`, `slack` and `schedule`. Webhooks, each checked by a signature or a shared secret, deliver `sentry`, `honeycomb`,
`intercom`, `cloudwatch` and `figma` events, and GitHub and Linear events sooner than a poll. A trigger's `key` says which events are the same
work, and a factory holds at most one active task per key. See
[docs/intake/README.md](intake/README.md).

**Workflow and steps.** A workflow is a graph of steps. Each step has one decider: a `model` step
runs an agent, a `human` step asks a person, and a `code` step runs a built-in action such as
`open-pr`. Each outcome maps to the next step, `done` or `stop`. A step marked `irreversible` runs
only after a person approved it, unless it is marked `autonomous` with a reason. See
[workflows.md](reference/workflows.md) for the keys of a step, and [tools.md](daemon/tools.md)
for how a task moves between steps.

**Intervention.** A trigger's `intervention` sets how much a person steps in. `plan` makes a person
approve each graded plan, even a green one. `hands-off` lets a lead agent answer the task's asks in
your place, but an ask that lets an irreversible step run still comes to you. A routed ask shows
"with <lead>" in the web view and the Herdr panels, you can still answer it first, and an answered
ask says who answered. When the lead ends without answering, the ask comes back to you. See
[docs/reference/triggers.md](reference/triggers.md#intervention-and-lead).

**Auto-answer.** A trigger's `auto_answer`, `high` or `medium`, lets the daemon answer an agent's ask
with its own recommendation when the agent states that confidence or more, with a one-line
rationale. It never answers an ask whose answer lets an irreversible step run, an approval, or an
ask that runs a command. Each such answer is marked `by auto` and listed in the desk's Since you
looked. See [docs/daemon/waits.md](daemon/waits.md#answered-by-auto).

**Task and run.** A task is one unit of work, from its event to `done`, `failed` or `dropped`. A run
is one agent process working on a task's step. A task can have many runs, and its agent's session
carries over between them. See [docs/daemon/runs.md](daemon/runs.md).

**Inbox and routing.** Every event the intake publishes lands in the daemon's inbox before anything
acts on it. The daemon then finds the task the event concerns, such as the task that owns the pull
request a check ran on. `priory task ID --json` lists the events routed to a task, and
`priory inbox --unrouted` lists those that reached none, as the web and Herdr Inbox panels do. It
also lists the events a sleeping task kept that no agent read yet. See
[docs/daemon/routing.md](daemon/routing.md).

**Waits and asks.** An agent ends a turn with one of three tools: `result` reports the step's
outcome, `wait` names an event to sleep until, and `ask` puts a question on the desk. A waiting or
asking agent has no process. See [docs/daemon/waits.md](daemon/waits.md).

**Desk.** Everything that waits on a person, in one list: asks, blocked and failed tasks, prompts
Priory could not confirm, a banner when the queue is stalled, and one when the intake fails to poll
or stopped. See
[docs/daemon/desk.md](daemon/desk.md).

**Queue, bands and room.** Tasks start in band order (`blocker`, `now`, `today`, `later` by
default), then in an order you set, then oldest first, or in turns between lanes with
`[queue] fair = "lane"`. A task's lane, such as `core/intake`, groups tasks by team and theme for
`priory tasks --lane` and `priory report --by lane`. Without a lane or a matching rule, it is
`<team>/misc`, or `none/misc` for a task with no team. A lane or a trigger can cap how many of its
tasks run at once. A run starts only when the machine has room for the memory and disk its agent
declares. See [docs/daemon/room.md](daemon/room.md) and [docs/daemon/runs.md](daemon/runs.md#queue).

**Health.** Every second, the daemon checks each run for an unexpected exit, a timeout or a
permission prompt. It never approves a permission prompt. A crashed run gets one repair attempt.
See [docs/daemon/health.md](daemon/health.md).

**Trace and reports.** Each step's end is recorded with its outcome, tokens and time. `priory report`
reads these records, and the daemon can export them as OpenTelemetry spans. See
[docs/daemon/trace.md](daemon/trace.md) and [docs/trace-schema.md](trace-schema.md).
