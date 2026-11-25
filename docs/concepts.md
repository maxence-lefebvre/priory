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
sources `github`, `linear`, `linear.view`, `linear.project`, `slack` and `schedule`. Webhooks, each checked by a signature or a shared secret, deliver `sentry`, `honeycomb`,
`intercom`, `cloudwatch` and `figma` events, and GitHub and Linear events sooner than a poll. A trigger's `key` says which events are the same
work, and a factory holds at most one active task per key. See
[docs/intake/README.md](intake/README.md). A trigger can run a [`pre` script](intake/pre.md) on
each event, which passes, changes or drops the task request. A `staged` trigger runs its tasks as
a trial: the model steps run, and no irreversible step does. See
[staged tasks](daemon/tools.md#staged-tasks).

**Workflow and steps.** A workflow is a graph of steps. Each step has one decider: a `model` step
runs an agent, a `human` step asks a person, and a `code` step runs a built-in action such as
`open-pr`. Each outcome maps to the next step, `done` or `stop`. A step marked `irreversible` runs
only after a person approved it, unless it is marked `autonomous` with a reason. A
[debate step](daemon/debates.md) has two agents answer one question in turns, and ends with their
shared verdict or a person's pick. See
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

**Autopilot.** For a while you choose, `priory autopilot on` lets the daemon answer the asks that
auto-answer could, whatever their trigger says. It ends by itself, marks what it leaves for you, and
reports what it answered, why, and how to reverse it. See
[docs/daemon/waits.md](daemon/waits.md#autopilot).

**Task and run.** A task is one unit of work, from its event to `done`, `failed` or `dropped`. A run
is one agent process working on a task's step. A task can have many runs, and its agent's session
carries over between them. A repository's [recipes](daemon/runs.md#recipes) are the commands
that prepare and clean up its worktrees. See [docs/daemon/runs.md](daemon/runs.md).

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
[docs/daemon/desk.md](daemon/desk.md). The [Claude Code plugin](adapters/claude-plugin.md) lets
your own session read the desk, ask you a question and add a task.

**Queue, bands and room.** Tasks start in band order (`blocker`, `now`, `today`, `later` by
default), then in an order you set, then oldest first, or in turns between lanes with
`[queue] fair = "lane"`. A task's lane, such as `core/intake`, groups tasks by team and theme for
`priory tasks --lane` and `priory report --by lane`. A trigger or a request names it, or
[`lanes.toml`](reference/lanes.md) derives it. A lane or a trigger can cap how many of its tasks
run at once. A run starts only when the machine has room for the memory and disk its agent
declares. See [docs/daemon/room.md](daemon/room.md) and [docs/daemon/runs.md](daemon/runs.md#queue).
Two more settings change who starts. A [review limit](daemon/runs.md#review-limit) holds new starts
while too many pull requests wait for a person. An [urgent wake](daemon/room.md#urgent-wakes) lets a
task in a band you name stop a lower band's run at the end of its turn.

**Labels and runner filters.** A label is a `key=value` pair on a task, such as `project=oss`,
which its trigger's `labels` or `priory request --label` set. Labels differ from the marks
`priory label` writes and from a GitHub issue's labels. A runner filter, in a runner file, matches
labels: `project=oss` or `!project=oss`, optionally only on some days and hours. A runner claims and
starts only the tasks its filters accept, so a desktop can leave a project's tasks to a server by
day. See [docs/daemon/runners.md](daemon/runners.md#placing-tasks-with-filters).

**Health.** Every second, the daemon checks each run for an unexpected exit, a timeout or a
permission prompt. It never approves a permission prompt. A crashed run gets one repair attempt.
A step's `budget` limits the dollars a task may spend at that step. When a fix fails the
same checks as the commit before it, the daemon asks a person instead of waking the agent again.
See [docs/daemon/health.md](daemon/health.md) and
[a fix that fails the same way](daemon/waits.md#a-fix-that-fails-the-same-way).

**Spend caps.** The most a task, a trigger's tasks, a lane's tasks or the whole factory may spend,
in dollars, per task, day or week. Past a cap, new runs wait, and a held task asks you to raise
the cap or stop it. See [docs/daemon/spend.md](daemon/spend.md).

**Skills and memories.** A skill is a folder of instructions an agent follows when its description
fits the task. A memory is one fact in one file. A run gets the factory's skills and memories,
and those of two optional git repositories: a kit of skills and a memories repository. An agent
can name the skills its runs get. When a Claude Code run reads, edits or creates a file that
matches the `paths` of a skill or memory, Priory adds that skill or memory to the run's context.
An agent
proposes a memory with `remember`, a [nightly dream](setting-up-a-nightly-dream.md) proposes
memories from the day's tasks, and a person accepts each one before other tasks get it. See
[docs/daemon/skills.md](daemon/skills.md).

**Scorers, gates and benchmarks.** A scorer grades a finished run.
A `judge` scorer asks a model, and counts only once its answers agree with a person's labels. A
gate is a scorer that refuses a step's result when it fails. A benchmark replays past tasks to
compare two commits of the factory. See [docs/daemon/score.md](daemon/score.md) and
[docs/daemon/bench.md](daemon/bench.md).

**Evidence.** Screenshots and recordings a run attaches to its result, so a person sees the change
work before reading the diff. See [docs/daemon/evidence.md](daemon/evidence.md).

**Decision model.** A command that answers a typed question with a probability for each option.
Once measured against a person's labels, it settles feedback on a pull request that asks the agent
for nothing, so the task is not woken. See [docs/daemon/decisions.md](daemon/decisions.md).

**Consult.** A run asks an investigator one question with the `consult` tool and gets the report
in its next prompt. An investigator is an agent that only reads, with MCP servers of its own, such
as an error tracker's. The asking run never gets those servers. See
[consulting an investigator](daemon/tools.md#consulting-an-investigator).

**Connector.** An MCP server a person connects once in claude.ai, which follows their Claude login
to every machine. An agent names the ones its runs get in `connectors`, and its runs get no other.
See [connectors](reference/agents.md#connectors).

**Repository readiness.** `priory check --repo PATH` scores what an unattended agent needs from a
repository, such as one test command and CI on pull requests. See
[docs/reference/readiness.md](reference/readiness.md).

**Trace and reports.** Each step's end is recorded with its outcome, tokens and time. `priory report`
reads these records, and the daemon can export them as OpenTelemetry spans. See
[docs/daemon/trace.md](daemon/trace.md) and [docs/trace-schema.md](trace-schema.md).

**Failure pattern.** The tasks that failed, blocked, were repaired or were marked wrong at one step
for the same reason. A nightly code step groups the day's into patterns, the desk shows a pattern
once it holds two tasks, and its count grows as new tasks match. A person closes it once a fix
merged, dismisses it, or captures its tasks as benchmark cases to compare the fix on. See
[docs/daemon/patterns.md](daemon/patterns.md).
