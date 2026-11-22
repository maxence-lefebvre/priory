# Priory and other agent factories

This page compares Priory with twenty other products: ten that run coding agents for a team, and
ten agent platforms, frameworks and workflow engines that Priory's ideas overlap. It also notes
three that shut down or changed product. It says what each one does that Priory does not, and what
Priory has that the pages read do not describe. Every claim about another product links the vendor's own page it comes from.
Those pages were read on 2026-11-22, and products change fast, so check the linked page before you
rely on a claim. No product was run for this page: "has a cap" means its docs say so, and "not
described" means the pages read do not mention it, not that the product lacks it.

## What Priory is for

Priory runs coding agents on machines you own and carries each task through a workflow you wrote,
usually to a pull request a person merges. Its daemon, not an agent, handles waits, crashes and
questions, so agents keep moving while nobody watches.

It suits an engineer, alone or in a small team, who runs agents on one to three machines they own
and wants to answer them from one place. It does not suit a team that wants a hosted service,
Kubernetes, one factory shared by dozens of people, or a chat interface. The
[product requirements](prd.md#users) leave these out of scope.

## Where Priory lacks something

Five gaps came out of the first reading. Each is something several products below have and Priory
did not. Priory has since built parts of all five, and this list says what remains.

1. **Spend caps inside a turn.** A factory sets caps per task, day and week on the factory, a
   trigger or a lane ([spend caps](daemon/spend.md)). A reached cap keeps new runs from starting.
   A task cap, or a factory or trigger day or week cap, also holds a running task with one ask.
   The daemon checks a running task's caps only between two turns, so one long turn can pass a
   cap. Only a step's own [`budget`](daemon/health.md#budget) is checked inside a turn, at most
   once a minute. With several runners, each daemon counts only its own spend.
2. **Graduated approval.** A trigger's [`intervention`](concepts.md) lets a person approve every
   plan, or a lead agent answer asks, and a workflow can route a plan by its grade. A
   [critic](daemon/score.md#gates) can refuse a plan no person reads, and a
   [decision model](daemon/decisions.md) settles feedback that asks the agent for nothing. A
   trigger's [`auto_answer`](daemon/waits.md#answered-by-auto) lets the daemon answer a reversible
   ask with the agent's recommendation, at the confidence the agent states. It grants no approval:
   Priory still has no policy that approves an irreversible action by a stated confidence or risk
   level.
3. **Proof for the reviewer.** A run's result can carry [evidence](daemon/evidence.md):
   screenshots and recordings that a step's `record` command makes, served by the task's API and
   listed in the pull request's body. The web view does not draw them yet. Priory has no live
   preview of a change, shows no before and after, and puts no image on the pull request itself.
   [reviewer-evidence.md](design/reviewer-evidence.md) designs them.
4. **A learning loop.** An agent proposes a memory with `remember`, and a
   [nightly dream](setting-up-a-nightly-dream.md) proposes memories from what tasks concluded and
   what people answered. A person accepts each before other tasks get it. A memory's
   [`paths`](reference/memories.md) bring it into a Claude Code run that reads, edits or creates a
   matching file. Priory does not flag or merge stale
   memories, and does not check a result against past rejections before a person sees it.
5. **The run's environment.** Agents run in the task's git worktree with the owner's credentials.
   A repository's [setup, resume and teardown recipes](reference/factory.md#repos) run around its
   worktrees, an agent names the MCP servers and skills its runs get, and
   [`priory check --repo`](reference/readiness.md) scores what an unattended agent needs from a
   repository. A run can [consult](daemon/tools.md#consulting-an-investigator) a read-only
   investigator, so MCP servers and their credentials stay away from the agent that pushes code. A factory cannot declare a token per agent or network rules.
   [agent-credentials.md](design/agent-credentials.md) designs them.

Three more features came after the first reading. A [review limit](daemon/runs.md#review-limit)
holds new starts while too many pull requests of a lane or batch wait for a person. When two
done-gate failures in a row on the same pull request have the same check names and conclusions,
the daemon asks a person instead of waking the agent again
([a fix that fails the same way](daemon/waits.md#a-fix-that-fails-the-same-way)). A
[staged trigger](daemon/tools.md#staged-tasks) runs its tasks as a trial: the model steps run on
the real event, and no irreversible step runs.

## Mastra and Mastra Factory

Mastra is a framework for building agents and workflows ([docs index](https://mastra.ai/llms.txt)).
Mastra Factory is its product for coding work, organized as phases on a board
([Mastra Factory](https://factory.mastra.ai/llms-full.txt)).

What Mastra does that Priory does not:

- `TokenCostControl` checks a dollar limit before each model call, over a time window, and its own
  docs call the control approximate
  ([TokenCostControl](https://mastra.ai/reference/processors/token-cost-control)).
- An eval run separates gates that must pass from tracked scorers, and returns a verdict of
  `passed`, `scored` or `failed` ([gates and verdicts](https://mastra.ai/docs/evals/gates-and-verdicts)).
- Observational Memory turns long message histories into dated observations and condenses them
  ([observational memory](https://mastra.ai/docs/memory/observational-memory)).
- A Mastra Factory phase can start and approve work on its own, each limited by a condition. Each
  session runs in a sandbox, hosted or local, that pauses and resumes with its files, and can share
  an app preview ([Mastra Factory](https://factory.mastra.ai/llms-full.txt)).

What Priory has that the pages read do not describe: priority bands for the queue, and a check,
before anything runs, of which steps a person approves.

## Vercel AI SDK and eve

The AI SDK is Vercel's library for building agents
([workflow agent](https://ai-sdk.dev/docs/agents/workflow-agent)). This section covers eve, Vercel's
open-source agent framework, in public preview since 2026-06-17
([eve announcement](https://vercel.com/changelog/introducing-eve-an-open-source-agent-framework))
under the Apache-2.0 license ([eve repository](https://github.com/vercel/eve)).

What eve does that Priory does not:

- A tool's approval can be `always()`, `once()`, `auto()`, `never()` or a policy
  ([tools](https://eve.dev/docs/tools)). A turn that waits on an approval parks and holds no compute
  ([execution model](https://eve.dev/docs/concepts/execution-model-and-durability)).
- The framework recalls memory before each turn and captures it after, and recalled text enters as
  user messages, never as system instructions ([memory](https://eve.dev/docs/memory)).
- Eval files send messages to a real agent server, assert on tool calls and can stub a tool's
  result ([evals](https://eve.dev/docs/evals)).
- A session is a durable workflow that saves progress at step boundaries, on Vercel or self-hosted
  with a local or Postgres store
  ([execution model](https://eve.dev/docs/concepts/execution-model-and-durability)).

What Priory has that the pages read do not describe: a queue with priority bands, and spend
reports per task.

## OpenHands

OpenHands is an agent platform with an SDK, a cloud service and a Kubernetes deployment
([docs index](https://docs.openhands.dev/llms.txt)).

What OpenHands does that Priory does not:

- Its software factory guide gives the triage, developer and reviewer automations a token each,
  and the reviewer's token cannot push code
  ([software factory](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)).
- Path-triggered rules are injected whenever the agent reads, edits or creates a matching file
  ([path-triggered rules](https://docs.openhands.dev/overview/skills/path)).
- An experimental critic scores a run from 0 to 1 while it works and can retry it
  ([critic](https://docs.openhands.dev/sdk/guides/critic)).
- A confirmation policy can ask a person only for actions a security analyzer rates risky
  ([security and confirmation](https://docs.openhands.dev/sdk/guides/security)).
- Persistent memory is opt in, with an index and daily logs
  ([persistent memory](https://docs.openhands.dev/sdk/guides/persistent-memory)).

What Priory has that the pages read do not describe: priority bands. The software factory guide
polls every five minutes, where a Priory agent sleeps until the event it named, such as a pull
request's checks.

## GitHub Copilot cloud agent

Copilot cloud agent is GitHub's coding agent. Automations run it on a schedule, on repository events
or through the API
([automations](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-automations)).

What Copilot does that Priory does not:

- Budgets apply at user, organization, cost center and enterprise level, and a user budget is
  always a hard stop
  ([budgets](https://docs.github.com/en/copilot/concepts/billing-and-usage/organizations-and-enterprises/budgets)).
- Automations rate each issue change high, medium or low, hold low-confidence changes as
  suggestions, and record each change's reason
  ([rationale, confidence and approvals](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-automation-rationale-and-approvals)).
- Copilot Memory stores repository facts with citations to the code and checks them against the
  current branch ([Copilot Memory](https://docs.github.com/en/copilot/concepts/prompting-and-context/copilot-memory)).
- Agentic workflows are Markdown files compiled to GitHub Actions workflows
  ([agentic workflows](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-github-agentic-workflows)).

What Priory has that the pages read do not describe: priority bands, and scorers and replay
benchmarks.

## OpenAI Codex

Codex is OpenAI's coding agent. Its docs call the older web experience "Codex Cloud (Legacy)" and
plan to deprecate it ([Codex docs](https://developers.openai.com/codex/llms-full.txt)). Some
features below belong to cloud tasks and some to Codex in general, as each line says.

What Codex does that Priory does not:

- A cloud task can request several attempts, best-of-N
  ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).
- A cloud environment runs a setup script, then an optional maintenance script on a cached
  container, and keeps the agent's internet off by default
  ([cloud environments](https://developers.openai.com/codex/cloud/environments)).
- In Codex's configuration, `approvals_reviewer = "auto_review"` sends approval prompts to a
  reviewer agent that checks for exfiltration, credential probing and destructive actions
  ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).
- A Codex goal has a `tokenBudget` ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).

What Priory has that the pages read do not describe for cloud tasks: runs on machines you own,
priority bands, and scorers and replay benchmarks.

## Devin

Devin is Cognition's coding agent ([docs index](https://docs.devin.ai/llms.txt)).

What Devin does that Priory does not:

- Budgets for shared pools and user limits block new work on every surface once reached
  ([usage policies](https://docs.devin.ai/enterprise/features/usage-policies)).
- Devin saves corrections and decisions to a person's memory by itself, tags each note with the
  session it came from, and "Dreaming" keeps the notes organized
  ([memory](https://docs.devin.ai/product-guides/memory)).
- Session Insights analyses completed sessions
  ([Session Insights](https://docs.devin.ai/product-guides/session-insights)).
- Testing mode records a video of the test and sends it to you
  ([testing and recordings](https://docs.devin.ai/work-with-devin/testing-and-recordings)).
- Security profiles limit network, MCP, git and GitHub CLI access and bind to an automation
  ([security profiles](https://docs.devin.ai/product-guides/security-profiles)).

Outposts run Devin's sessions on machines you own over an outbound connection, while the agent loop
stays in Cognition's cloud ([Outposts](https://docs.devin.ai/cloud/outposts/overview)). With Priory,
the agent loop runs on your machines too. The pages read do not describe scorers or replay
benchmarks.

## Factory

Factory sells coding agents called droids. Automations run them on a schedule, from Slack
messages, on GitHub events or from webhooks ([docs index](https://docs.factory.ai/llms.txt)).

What Factory does that Priory does not:

- Agent Readiness scores a repository on five levels of what an agent needs, and can fix failing
  criteria ([Agent Readiness](https://docs.factory.com/agent-readiness/overview)).
- Droid Control records before and after videos to verify claims about a change's behaviour
  ([Droid Control](https://docs.factory.com/automations/droid-control)).
- Missions plan a large project into features and milestones
  ([Missions planning](https://docs.factory.com/missions/planning)).
- Workstreams, in beta, put an orchestrator over a team of agents on cloud or local computers with a
  shared folder ([Workstreams](https://docs.factory.com/workstreams/overview)).
- AutoWiki generates a wiki for a repository ([docs index](https://docs.factory.ai/llms.txt)).

What Priory has that the pages read do not describe: priority bands, and replay benchmarks that
compare a configuration change on past tasks. Factory's cost page describes model policy, usage
patterns and analytics, and no hard cap
([cost and productivity](https://docs.factory.com/agent-effectiveness/cost-and-productivity)).

## Cursor cloud agents

Cursor's cloud agents, formerly called background agents, run Cursor's agent on isolated machines
([cloud agents](https://cursor.com/docs/cloud-agent)).

What Cursor does that Priory does not:

- Each automation can keep named notes outside the working tree, on by default
  ([automations](https://cursor.com/docs/cloud-agent/automations)).
- Bugbot reviews pull requests, can start an agent to fix what it finds, and reports a resolution
  status per finding ([Bugbot](https://cursor.com/docs/bugbot)).
- PR Routing and Approval can approve low-risk pull requests. On GitHub, it picks reviewers from code
  ownership or recent commits, and also accepts reviewers named in its prompt
  ([PR Routing and Approval](https://cursor.com/docs/approval-agents)).
- Self-hosted workers and team pools run the tools on your machines while Cursor runs the agent
  loop ([self-hosted machines](https://cursor.com/docs/cloud-agent/self-hosted),
  [team pools](https://cursor.com/docs/cloud-agent/self-hosted/pool)).

With Priory, the agent loop runs on your machines too. The pages read do not describe priority
bands, scorers or replay benchmarks.

## Amp

Amp is a coding agent whose threads run on machines called orbs.

What Amp does that Priory does not:

- A schedule lives on a thread, once or repeating, with a completion condition that clears it
  ([automations and webhooks](https://ampcode.com/docs/orbs/automations)).
- Each thread gets its own orb. Self-hosted orbs need outbound HTTPS only
  ([self-hosted orbs](https://ampcode.com/docs/orbs/self-hosted)).
- A project's Ship setting decides what shipping does, by default running the tests and pushing to
  the base branch ([shipping changes](https://ampcode.com/docs/orbs/shipping)).
- A checklist says how to make a codebase work well with agents
  ([optimizing your codebase](https://ampcode.com/docs/orbs/optimizing)).

What Priory has that the pages read do not describe: learned memory, priority bands, and scorers
and replay benchmarks. Amp prices orbs per hour by size
([sizes and costs](https://ampcode.com/docs/orbs/sizes-and-costs)).

## Claude Code GitHub Actions and Code Review

Claude Code runs in GitHub Actions on any GitHub event, an `@claude` mention or a cron schedule
([GitHub Actions](https://code.claude.com/docs/en/github-actions)). Code Review is Anthropic's
review service for pull requests, run on Anthropic's infrastructure
([Code Review](https://code.claude.com/docs/en/code-review)).

What they do that Priory does not:

- Code Review runs several agents, verifies each finding to drop false positives, and posts the
  findings ranked by severity. It never approves or blocks
  ([Code Review](https://code.claude.com/docs/en/code-review)).
- Code Review has a monthly spend limit and shows the average cost per review per repository
  ([Code Review costs](https://code.claude.com/docs/en/code-review-costs)).
- An Actions run is limited with `--max-turns` and workflow timeouts
  ([GitHub Actions](https://code.claude.com/docs/en/github-actions)).

What Priory has that the pages read do not describe: memory learned between runs, priority bands,
and an agent that sleeps between events in one session, where each event starts a new Actions run.

## GitHub Agentic Workflows

Copilot's section above links GitHub's description of agentic workflows. Their prompt files for
agents ([prompt index](https://github.github.com/gh-aw/llms.txt)) describe the following, and a
persistent memory strategy.

What it does that Priory does not:

- An `experiments:` block gives each run the variant used least so far by default, or a weighted
  random one, and names a hypothesis, a metric and guardrail metrics. Once every mandatory
  guardrail has its minimum samples and supported observations, a failing one gives a `REJECT`
  decision
  ([experiments](https://raw.githubusercontent.com/github/gh-aw/main/.github/aw/experiments.md)).
- Writes go through declared safe outputs, and the guide sets a `max` on outputs such as
  `create-issue` and `add-comment`
  ([safe outputs](https://raw.githubusercontent.com/github/gh-aw/main/.github/aw/safe-outputs.md)).
- The intent guide puts no-op conditions, cases that must create no attention and no writes, in
  the workflow's prompt body
  ([intent](https://raw.githubusercontent.com/github/gh-aw/main/.github/aw/intent.md)).
- `max-ai-credits` sets a budget per run. `max-daily-ai-credits` blocks new runs once the
  triggering user's use of the workflow over the last 24 hours passes it
  ([frontmatter keys](https://raw.githubusercontent.com/github/gh-aw/main/.github/aw/syntax-agentic.md)).
- Evals are yes or no questions, answered `YES`, `NO` or `UNKNOWN`
  ([evals](https://raw.githubusercontent.com/github/gh-aw/main/.github/aw/evals.md)).

Its loop guide stops a fix loop on a repeated identical signature
([loop patterns](https://raw.githubusercontent.com/github/gh-aw/main/.github/aw/loop.md)), as
Priory does for red checks. What Priory has that the pages read do not describe: priority bands,
and runs on machines you own outside a CI service.

## Linear's agent platform

Linear lets other vendors' agents work on its issues
([agents](https://linear.app/developers/agents)), and runs Loops of its own
([Loops](https://linear.app/docs/loops)).

What Linear does that Priory does not:

- An issue delegated to an agent keeps its assignee, "so humans maintain ownership while agents act
  on their behalf" ([agents](https://linear.app/developers/agents)).
- An agent session shows the agent's thoughts, actions, questions and errors on the issue, and an
  `elicitation` waits for a person's answer there
  ([agent interaction](https://linear.app/developers/agent-interaction)).
- An agent offers options with the `select` signal, and a person can send `stop` to stop it
  ([signals](https://linear.app/developers/agent-signals)).
- Loops run on a schedule or on supported events, and stop when the workspace runs out of AI
  credits ([Loops](https://linear.app/docs/loops)).

Priory's Linear intake reads issues and writes nothing back to them. What Priory has that the pages
read do not describe: memory learned between runs, and scorers or benchmarks.

## SuperPlane

SuperPlane is a factory whose 1.0.0 release came out on 2026-10-27
([releases](https://github.com/superplanehq/superplane/releases)). Its guide names four stages,
Backlog, Implement, Verify and Done, and verification can include a person's approval
([one-shot engineering](https://docs.superplane.com/fundamentals/one-shot-engineering/)). It
installs on one VM or on Kubernetes in your cloud
([docs index](https://docs.superplane.com/llms.txt)).

What SuperPlane does that Priory does not:

- The Backlog stage analyses and qualifies each request before work starts
  ([backlog automation](https://docs.superplane.com/fundamentals/automation-backlog/)).
- The Verify guide recommends evidence that matches the task's risk
  ([verify](https://docs.superplane.com/fundamentals/verify/)).
- Team and Enterprise plans set budget limits per group, user or workspace, and a maximum spend
  per task
  ([costs and budgets](https://docs.superplane.com/billing/costs-budgets/)).
- A task time report uses the median, and separates time running from time waiting
  ([task time](https://docs.superplane.com/fundamentals/performance-task-time/)).

What Priory has that the pages read do not describe: priority bands, and memory learned between
runs.

## Kiro and Kiro Crew

Kiro is an agent in an IDE, a CLI and the web ([docs index](https://kiro.dev/llms.txt)). Kiro
Crew is an open-source agent that runs locally or remotely on your hardware
([Crew](https://kiro.dev/docs/crew.md)).

What they do that Priory does not:

- Crew's Task Runner has an independent reviewer session read the actual `git diff`. The Task Runner
  reverts and retries a failed step, at most 3 times, then plans again
  ([Task Runner](https://kiro.dev/docs/crew/features/task-runner.md)).
- Crew keeps semantic, episodic and lesson memories. A consolidator replaces its preferences every
  30 messages, and a model's semantic memory write needs a confidence of 0.8 or more
  ([Crew memory](https://kiro.dev/docs/crew/features/memory.md)).
- One Crew dashboard reaches remote crews over SSH, AWS SSM or Fargate, with a status badge for each
  ([multi-instance](https://kiro.dev/docs/crew/features/multi-instance.md)).
- A Crew heartbeat checks every 60 seconds and reports only when something changes
  ([cron and heartbeats](https://kiro.dev/docs/crew/features/cron.md)).
- A Kiro workflow can show its proposed plan for approval before any step runs. A saved
  permission decision, or trusting all tools, approves it without asking
  ([workflows](https://kiro.dev/docs/workflows.md)).

What Priory has that the pages read do not describe: a queue with priority bands, and a dollar cap
per task.

## Cline

Cline is a coding agent with a CLI, an SDK, cloud agents and a Kanban board. It can run on a
remote machine over SSH, and a Memory Bank keeps context across sessions
([docs index](https://docs.cline.bot/llms.txt)).

What Cline does that Priory does not:

- The Kanban board, a research preview, gives each card its own worktree
  ([Kanban](https://docs.cline.bot/usage/kanban.md)). A linked card starts when the card before it
  is done and moved to trash ([Kanban workflow](https://docs.cline.bot/kanban/core-workflow.md)).
- `cline schedule` runs cron schedules, and can list upcoming runs, trigger one and pause one
  ([scheduling](https://docs.cline.bot/cli/scheduling.md)).
- Auto Approve is set per type of action
  ([auto approve](https://docs.cline.bot/features/auto-approve.md)).
- The SDK limits a session with `maxTokensPerTurn` and `maxIterations`, and stops it after
  consecutive mistakes
  ([going to production](https://docs.cline.bot/sdk/guides/going-to-production.md)).
- A cloud agent runs in a sandbox per session that pauses after 15 idle minutes
  ([cloud agents](https://docs.cline.bot/usage/cloud-agents.md)).

What Priory has that the pages read do not describe: priority bands, a built-in dollar cap, and
scorers or benchmarks. The SDK guide shows code that aborts a session past a cost
([going to production](https://docs.cline.bot/sdk/guides/going-to-production.md)).

## Temporal

Temporal is a durable workflow engine. Its docs cover AI agents built on it, with agent memory and
state, and workflows that block until an outside approval ([durable AI](https://docs.temporal.io/ai.md)).

What Temporal does that Priory does not:

- A task queue takes a priority from 1 to 5, and fairness keys with weights keep one key with a
  large backlog from taking all the dispatches
  ([priority and fairness](https://docs.temporal.io/develop/task-queue-priority-fairness.md)).
  That page covers self-hosted Temporal and Temporal Cloud.
- A schedule has an overlap policy, a catchup window, backfill, jitter, and a note on pause
  ([schedules](https://docs.temporal.io/schedule.md)).
- Worker versioning pins a workflow to the version it started on, or lets it upgrade, and can send a
  share of new workflows to a new version
  ([worker versioning](https://docs.temporal.io/production-deployment/worker-deployments/worker-versioning.md)).

What Priory has that the pages read do not describe: memory learned between tasks, spend caps,
scorers, and a desk where a person answers agents' asks.

## Inngest

Inngest is a durable workflow engine for AI applications
([docs index](https://www.inngest.com/llms.txt)). `step.waitForEvent()` pauses an agent for an
approval, with a timeout
([human in the loop](https://www.inngest.com/docs-markdown/durable-execution/durable-agents/human-in-the-loop)),
as a Priory ask with a time wait does.

What Inngest does that Priory does not:

- Concurrency takes a key, so one account's steps do not use another account's slots
  ([concurrency](https://www.inngest.com/docs-markdown/durable-execution/flow-control/concurrency)).
- Priority is a number of seconds a run moves ahead, computed from the event
  ([priority](https://www.inngest.com/docs-markdown/durable-execution/flow-control/priority)).
- An experiment keeps its variant on retries and records it in the trace
  ([experiments](https://www.inngest.com/docs-markdown/agent-evals/experiments)).
- A deferred scorer, in beta, waits for a later signal and scores the run that started it
  ([deferred scoring](https://www.inngest.com/docs-markdown/agent-evals/deferred-scoring)).

What Priory has that the pages read do not describe: memory, and spend caps.

## LangSmith and the LangGraph platform

LangChain documents LangSmith, Fleet and its open-source packages, LangGraph among them, in one
place ([docs index](https://docs.langchain.com/llms.txt)).

What LangSmith does that Priory does not:

- The LLM Gateway enforces spend policies per organization, workspace, user or key, over an hour, a
  day, a week or a month. The request that crosses a cap still succeeds, and later calls get a `402`
  ([spend policies](https://docs.langchain.com/langsmith/llm-gateway-spend-policies.md)). It routes
  Claude Code, Codex and Gemini CLI
  ([gateway for coding agents](https://docs.langchain.com/langsmith/llm-gateway-coding-agents.md)).
- Engine groups failing production traces into tracked issues, proposes a fix, and attaches new
  traces that match the same failure ([Engine](https://docs.langchain.com/langsmith/engine.md)).
  Fix validation, in private beta, replays an issue's traces against the fix
  ([fix validation](https://docs.langchain.com/langsmith/engine-fix-validation.md)).
- Fleet has a central inbox for reviewing agent actions across all agents, and runs in the cloud,
  self-hosted or hybrid
  ([Fleet access and oversight](https://docs.langchain.com/langsmith/fleet/access-and-oversight.md)).
- Engine turns a reviewer's actions on its issues into general rules for later scans
  ([Engine](https://docs.langchain.com/langsmith/engine.md)).
- A chosen strategy rejects or enqueues a message that arrives during a run, or lets it interrupt
  the run or roll it back ([double texting](https://docs.langchain.com/langsmith/double-texting.md)).

What Priory has that the pages read do not describe: priority bands.

## CrewAI

CrewAI's docs cover crews of agents and flows ([memory](https://docs.crewai.com/edge/en/concepts/memory.md),
[human feedback in flows](https://docs.crewai.com/edge/en/learn/human-feedback-in-flows.md)).

What CrewAI does that Priory does not:

- `@human_feedback(learn=True)` draws lessons from a person's corrections, and can apply past
  lessons to a text or eligible Pydantic output before the person sees it
  ([human feedback in flows](https://docs.crewai.com/edge/en/learn/human-feedback-in-flows.md)).
- Memory has a model infer scope, categories and importance on save, and recall ranks by semantic
  similarity, recency and importance ([memory](https://docs.crewai.com/edge/en/concepts/memory.md)).
- `crewai test` runs a crew several times and scores each task with a model
  ([testing](https://docs.crewai.com/edge/en/concepts/testing.md)).
- Checkpointing lets a crew resume after a failure or fork into another branch
  ([checkpointing](https://docs.crewai.com/edge/en/concepts/checkpointing.md)).

What Priory has that the pages read do not describe: priority bands, and spend caps.

## Dagger

Dagger runs CI checks on a local or a shared remote engine, the same for machine-generated or
handwritten changes ([introduction](https://docs.dagger.io/getting-started/introduction)). Cloud
Checks start them on repository events ([Cloud Checks](https://docs.dagger.io/cloud-checks)). Its
agent runtime is an experimental API ([Agent API](https://docs.dagger.io/reference/api/agent)).

What Dagger does that Priory does not:

- container-use gives each agent a fresh container in its own git branch, and keeps the complete
  command history and logs of what the agent did
  ([container-use](https://github.com/dagger/container-use)).

What Priory has that the pages read do not describe: a task queue, approvals, memory learned
between tasks, and spend caps.

## Google Jules

Google is shutting Jules down on 2026-12-31, and it has been read-only since 2026-11-11
([turndown notice](https://jules.google/docs/changelog/2026-10-29)). The export Google offers
includes memories and scheduled tasks. It is listed for its ideas.

- A Planning Critic reviewed plans that no person approved, and Google reports a 9.5 % reduction in
  task failures ([Planning Critic](https://jules.google/docs/changelog/2026-01-26-1)).
- Suggested Tasks scanned a repository for `#TODO`s and proposed plans
  ([suggested tasks](https://jules.google/docs/suggested-tasks)).

## Roo Code and Sweep

Roo Code's docs say the extension was shut down on May 15th
([Roo Code docs](https://docs.roocode.com/)). Sweep's repository now says "We're now building an AI
coding assistant for JetBrains" ([repository](https://github.com/sweepai/sweep)). Neither is
compared here.

## Side by side

Each cell sums up the section above it, with its source there. "Not described" means the pages
read do not mention it. The first table holds the coding agent products:

| Theme | Priory | Mastra and Mastra Factory | eve | OpenHands | Copilot cloud agent | Codex | Devin | Factory | Cursor cloud agents | Amp | Claude Code Actions and Code Review | Jules |
| ----- | ------ | ------------------------- | --- | --------- | ------------------- | ----- | ----- | ------- | ------------------- | --- | ----------------------------------- | ----- |
| Where it runs | Machines you own, each task in its own git worktree with the repository's setup recipe | Factory sandboxes per session, hosted or local | Vercel, or self-hosted | Cloud or Kubernetes | Not described | OpenAI's cloud environments for cloud tasks | Cognition's cloud; tools on your machines through Outposts | Cloud or local computers | Cursor's machines; tools on your machines through self-hosted pools | An orb per thread, in Amp's cloud or self-hosted | Actions on GitHub's or your runners; Code Review on Anthropic's | Read-only, no longer runs tasks |
| Who approves | A person for each irreversible step unless marked autonomous with a reason; a lead agent, or the daemon at the agent's stated confidence, can answer other asks | A person, or a phase's condition | A policy per tool: always, once, auto or never | Always, never, or only risky actions | Issue changes held below a confidence level | Approval prompts can go to a reviewer agent | Not described | Not described | Low-risk pull requests approved by policy | A project's Ship setting | Code Review never approves | A person, with a critic for plans nobody approved |
| Scheduling and priority | Schedule triggers and events; bands from blocker to later; a review limit on waiting pull requests | Board phases; bands not described | Bands not described | A five-minute poll in the factory guide | Schedules, repository events and the API | Not described | Not described | Schedules, Slack, GitHub and webhooks | Bands not described | A schedule per thread, with a completion condition | GitHub events and cron | Suggested tasks |
| Memory | Files in git, brought into a Claude Code run by path; an agent or a nightly dream proposes, a person accepts | Observations condensed from message history | Recall before and capture after each turn | An index plus daily logs, opt in | Repository facts with citations | Not described | Notes saved by Devin, tagged by session | A generated repository wiki | Named notes per automation | Learned memory not described | Not described | In the export |
| Evaluation | Scorers as gates on a result, a critic for plans no person reads, calibrated judges and replay benchmarks | Eval runs with gates and tracked scorers | Eval files against a real agent | An experimental critic that scores a run while it works | Not described | Not described | Session Insights after a session | Agent Readiness for the repository | Resolution status per Bugbot finding | Not described | A verification step inside Code Review | A critic for plans |
| Cost control | Caps per task, day and week on the factory, a trigger or a lane, checked at run starts and between turns | A dollar limit before each model call, approximate | Not described | Not described | Budgets; the user budget is a hard stop | A token budget on a goal | Budgets that block new work | Model policy and analytics; no cap described | Not described | Orbs priced per hour | Turn limits and timeouts; a monthly limit for Code Review | Not described |
| Multi-machine | Several runners you own, sharing one event log | A sandbox per session | Durable sessions on Vercel or a Postgres store | Kubernetes | Not described | Cached cloud containers | Cloud machines and Outposts | Several computers with a shared folder | Team pools of self-hosted workers | An orb per thread | Not described | Not described |

The platforms, frameworks and workflow engines:

| Theme | Priory | GitHub Agentic Workflows | Linear | SuperPlane | Kiro and Kiro Crew | Cline | Temporal | Inngest | LangSmith | CrewAI | Dagger |
| ----- | ------ | ------------------------ | ------ | ---------- | ------------------ | ----- | -------- | ------- | --------- | ------ | ------ |
| Where it runs | Machines you own, each task in its own git worktree with the repository's setup recipe | GitHub Actions workflows | Not described | One VM or Kubernetes in your cloud | Crew on your hardware, local or remote | A sandbox per session for cloud agents | Self-hosted or Temporal Cloud | Not described | Cloud, self-hosted or hybrid | Not described | A container per agent, in its own git branch |
| Who approves | A person for each irreversible step unless marked autonomous with a reason; a lead agent, or the daemon at the agent's stated confidence, can answer other asks | Not described | A person answers in the issue | A person's approval can be part of Verify | A person or a saved permission approves a workflow's plan | Auto Approve per type of action | A workflow blocks until an outside approval | An approval event, with a timeout | Fleet's inbox | A person's feedback on a flow step | Not described |
| Scheduling and priority | Schedule triggers and events; bands from blocker to later; a review limit on waiting pull requests | Bands not described | Loops on a schedule or on events | Backlog qualifies each request first | A heartbeat every 60 seconds | Cron schedules; linked cards | Priorities 1 to 5 and weighted fairness keys | Keyed concurrency; priority in seconds | A message during a run is rejected, enqueued, interrupts or rolls back | Not described | Checks on repository events |
| Memory | Files in git, brought into a Claude Code run by path; an agent or a nightly dream proposes, a person accepts | A persistent memory strategy | Learned memory not described | Learned memory not described | Semantic, episodic and lesson memories; confidence-gated semantic writes | A Memory Bank across sessions | Agent memory and state in workflows; learned memory not described | Not described | Engine learns review preferences for later scans | Scope and importance inferred on save; lessons from feedback | Learned memory not described |
| Evaluation | Scorers as gates on a result, a critic for plans no person reads, calibrated judges and replay benchmarks | Yes or no evals; experiments with guardrails | Not described | Risk-matched evidence, recommended; a task time report | A reviewer session that reads the diff | Not described | Not described | Experiments and deferred scorers | Issues from failing traces; fix validation by replay | `crewai test` scores each task | The same checks on every change |
| Cost control | Caps per task, day and week on the factory, a trigger or a lane, checked at run starts and between turns | A budget per run; a 24-hour limit per triggering user | Loops stop at the workspace's credits | Team and Enterprise budgets, and a maximum per task | Dollar cap not described | Tokens per turn and iterations; custom code can abort past a cost | Not described | Not described | Spend policies at the gateway; the request that crosses a cap succeeds | Not described | Not described |
| Multi-machine | Several runners you own, sharing one event log | Not described | Not described | Not described | One dashboard over remote crews | Remote execution over SSH | Not described | Not described | Not described | Not described | Local or a shared remote engine |
