# Priory and other agent factories

This page compares Priory with ten other products that run coding agents for a team, and one that is
shutting down. It says what each one does that Priory does not, and what Priory has that the pages
read do not describe. Every claim about another product links the vendor's own page it comes from.
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

Five gaps came out of the reading. Each is something several products below have and Priory does
not.

1. **Spend caps at run time.** Priory prices a task after the fact, from token counts and the
   factory's `prices.toml`. Only its measuring commands, `priory score`, `priory bench` and
   `priory eval`, take a dollar budget. Nothing stops a factory task at a cap.
2. **Graduated approval.** A trigger's [`intervention`](concepts.md) lets a person approve every
   plan, or a lead agent answer asks, and a workflow can route a plan by its grade. Priory has no
   policy that approves a single action by a stated confidence or risk level.
3. **Proof for the reviewer.** Priory's docs describe no built-in recording, screenshot or live
   preview of a change for the reviewer.
4. **A learning loop.** An agent proposes a memory with `remember`, and a person accepts it before
   other tasks get it. Priory does not draw memories from review comments or finished sessions by
   itself, and does not flag or merge stale memories.
5. **The run's environment.** Agents run in the task's git worktree with the owner's credentials.
   A factory cannot declare setup, resume or teardown commands, a token per agent, or network rules.

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

## Google Jules

Google is shutting Jules down on 2026-12-31, and it has been read-only since 2026-11-11
([turndown notice](https://jules.google/docs/changelog/2026-10-29)). The export Google offers
includes memories and scheduled tasks. It is listed for its ideas.

- A Planning Critic reviewed plans that no person approved, and Google reports a 9.5 % reduction in
  task failures ([Planning Critic](https://jules.google/docs/changelog/2026-01-26-1)).
- Suggested Tasks scanned a repository for `#TODO`s and proposed plans
  ([suggested tasks](https://jules.google/docs/suggested-tasks)).

## Side by side

Each cell sums up the section above it, with its source there. "Not described" means the pages
read do not mention it.

| Theme | Priory | Mastra and Mastra Factory | eve | OpenHands | Copilot cloud agent | Codex | Devin | Factory | Cursor cloud agents | Amp | Claude Code Actions and Code Review | Jules |
| ----- | ------ | ------------------------- | --- | --------- | ------------------- | ----- | ----- | ------- | ------------------- | --- | ----------------------------------- | ----- |
| Where it runs | Machines you own, each task in its own git worktree | Factory sandboxes per session, hosted or local | Vercel, or self-hosted | Cloud or Kubernetes | Not described | OpenAI's cloud environments for cloud tasks | Cognition's cloud; tools on your machines through Outposts | Cloud or local computers | Cursor's machines; tools on your machines through self-hosted pools | An orb per thread, in Amp's cloud or self-hosted | Actions on GitHub's or your runners; Code Review on Anthropic's | Read-only, no longer runs tasks |
| Who approves | A person for each irreversible step unless marked autonomous with a reason; a lead agent can answer other asks | A person, or a phase's condition | A policy per tool: always, once, auto or never | Always, never, or only risky actions | Issue changes held below a confidence level | Approval prompts can go to a reviewer agent | Not described | Not described | Low-risk pull requests approved by policy | A project's Ship setting | Code Review never approves | A person, with a critic for plans nobody approved |
| Scheduling and priority | Schedule triggers and events; bands from blocker to later | Board phases; bands not described | Bands not described | A five-minute poll in the factory guide | Schedules, repository events and the API | Not described | Not described | Schedules, Slack, GitHub and webhooks | Bands not described | A schedule per thread, with a completion condition | GitHub events and cron | Suggested tasks |
| Memory | Files in git; an agent proposes, a person accepts | Observations condensed from message history | Recall before and capture after each turn | An index plus daily logs, opt in | Repository facts with citations | Not described | Notes saved by Devin, tagged by session | A generated repository wiki | Named notes per automation | Learned memory not described | Not described | In the export |
| Evaluation | Scorers, calibrated judges and replay benchmarks, run on demand | Eval runs with gates and tracked scorers | Eval files against a real agent | An experimental critic that scores a run while it works | Not described | Not described | Session Insights after a session | Agent Readiness for the repository | Resolution status per Bugbot finding | Not described | A verification step inside Code Review | A critic for plans |
| Cost control | Cost per task after the fact; caps only on measuring commands | A dollar limit before each model call, approximate | Not described | Not described | Budgets; the user budget is a hard stop | A token budget on a goal | Budgets that block new work | Model policy and analytics; no cap described | Not described | Orbs priced per hour | Turn limits and timeouts; a monthly limit for Code Review | Not described |
| Multi-machine | Several runners you own, sharing one event log | A sandbox per session | Durable sessions on Vercel or a Postgres store | Kubernetes | Not described | Cached cloud containers | Cloud machines and Outposts | Several computers with a shared folder | Team pools of self-hosted workers | An orb per thread | Not described | Not described |
