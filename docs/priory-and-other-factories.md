# Priory and other agent factories

This page compares Priory with ten other products that run coding agents for a team, and one that is
shutting down. It says what each one does that Priory does not, and the reverse. Every claim about
another product links the vendor's own page it comes from. Those pages were read on 2026-11-22, and
products change fast, so check the linked page before you rely on a claim. No product was run for
this page: "has a cap" means its docs say so.

## What Priory is for

Priory runs coding agents on machines you own and carries each task through a workflow you wrote,
usually to a pull request a person merges. Its daemon, not an agent, handles waits, crashes and
questions, so agents keep moving while nobody watches.

It suits an engineer, alone or in a small team, who runs agents on one to three machines they own
and wants to answer them from one place. It does not suit a team that wants a hosted service,
Kubernetes, one factory shared by dozens of people, or a chat interface: the
[product requirements](prd.md#non-goals) list these as non-goals.

## Where Priory lacks something

Five gaps came out of the reading. Each is something several products below have and Priory does
not.

1. **Spend caps at run time.** Priory prices a task after the fact, from token counts and the
   factory's `prices.toml`. Only its measuring commands, `priory score`, `priory bench` and
   `priory eval`, take a dollar budget. Nothing stops a factory task at a cap.
2. **Graduated approval.** A trigger's [`intervention`](concepts.md) gives three choices: a person
   approves plans, a lead agent answers asks, or neither. Nothing approves by a stated confidence or
   risk level.
3. **Proof for the reviewer.** A task's result carries text. Priory records no video, screenshot or
   live preview of the change.
4. **A learning loop.** A memory exists only when an agent calls `remember` and a person accepts
   it. Priory does not mine review comments or finished sessions, and a memory does not record
   where it came from or age.
5. **The run's environment.** A run starts in a fresh git worktree with the owner's credentials.
   A factory cannot declare setup, resume or teardown commands, a token per agent, or network rules.

## Mastra and Mastra Factory

Mastra is a framework for agents and workflows. Mastra Factory is its product for coding
work, and the closest product to Priory on this page.

What Mastra does that Priory does not:

- `TokenCostControl` checks a dollar limit before each model call, over a time window, and its own
  docs say the limit is approximate
  ([TokenCostControl](https://mastra.ai/reference/processors/token-cost-control)).
- Evals can act as gates that block a run, apart from tracked scorers, with a verdict of `passed`,
  `scored` or `failed` ([gates and verdicts](https://mastra.ai/docs/evals/gates-and-verdicts)).
- Observational Memory turns long message histories into dated observations and condenses them
  ([observational memory](https://mastra.ai/docs/memory/observational-memory)).
- Mastra Factory phases can start and approve work on their own, each limited by a condition. Each
  session runs in a sandbox, hosted or local, that pauses and resumes with its files, and has an
  app preview link ([Mastra Factory](https://factory.mastra.ai/llms-full.txt)).

What Priory does that Mastra does not, in the pages read: priority bands for its queue, and a
check, before anything runs, of which steps a person approves.

## Vercel AI SDK and eve

The AI SDK is Vercel's library for building agents. Its approval pages were read only through a
search summary, so this section covers eve. eve is Vercel's open-source
agent framework under the Apache-2.0 license, in public preview since 2026-06-17
([eve announcement](https://vercel.com/changelog/introducing-eve-an-open-source-agent-framework)).

What eve does that Priory does not:

- A tool's approval can be `always()`, `once()`, `auto()`, `never()` or a policy. A turn that
  waits on an approval is parked and holds no compute ([tools](https://eve.dev/docs/tools)).
- The framework calls a memory's `recall` before each turn and `capture` after it, and recalled
  text enters as user messages, never as system instructions ([memory](https://eve.dev/docs/memory)).
- Eval files send messages to a real agent server, assert on tool calls and can stub a tool's
  result ([evals](https://eve.dev/docs/evals)).
- A session is a durable workflow checkpointed per step, on Vercel or self-hosted on a local or
  Postgres store ([execution model](https://eve.dev/docs/concepts/execution-model-and-durability)).

What Priory does that eve does not, in the pages read: a queue with priority bands, and cost per
task. The eve pages read mention session timeouts and no other cost control.

## OpenHands

OpenHands is an agent platform with an SDK, a cloud service and a self-hosted deployment.

What OpenHands does that Priory does not:

- Its software factory guide gives the triage, developer and reviewer automations a token each,
  so the reviewer can post findings but cannot push code
  ([software factory](https://docs.openhands.dev/openhands/usage/use-cases/software-factory)).
- Path-triggered rules are injected whenever the agent reads, edits or creates a matching file
  ([path-triggered rules](https://docs.openhands.dev/overview/skills/path)).
- A critic scores a run from 0 to 1 while it works and drives refinement
  ([critic](https://docs.openhands.dev/sdk/guides/critic)).
- `ConfirmRisky` asks for confirmation only on actions a security analyzer rates risky
  ([security and confirmation](https://docs.openhands.dev/sdk/guides/security)).

What Priory does that OpenHands does not, in the pages read: priority bands, and a waiting agent
that wakes on the event it named, such as a pull request's checks. The software factory guide
polls every five minutes instead.

## GitHub Copilot cloud agent

Copilot cloud agent, formerly called the coding agent, is GitHub's coding agent. It runs on GitHub
Actions runners and delivers its work as pull requests.

What Copilot does that Priory does not:

- Budgets apply at user, organization, cost center and enterprise level, and a user budget is
  always a hard stop
  ([budgets](https://docs.github.com/en/copilot/concepts/billing-and-usage/organizations-and-enterprises/budgets)).
- Automations can rate each issue change high, medium or low, hold low-confidence changes as
  suggestions, and keep each change's rationale
  ([rationale, confidence and approvals](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-automation-rationale-and-approvals)).
- Copilot Memory stores repository facts with citations to the code and checks them against the
  current branch ([Copilot Memory](https://docs.github.com/en/copilot/concepts/prompting-and-context/copilot-memory)).
- Agentic workflows are Markdown files compiled to GitHub Actions
  ([agentic workflows](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-github-agentic-workflows)).

What Priory does that Copilot does not, in the pages read: runs on machines you own, priority
bands, and scorers and replay benchmarks.

## OpenAI Codex cloud tasks

Codex is OpenAI's coding agent. Its docs call the older web experience "Codex Cloud (Legacy)" and
plan to deprecate it ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).

What Codex does that Priory does not:

- A cloud task takes `--attempts` to run several attempts and pick the best one
  ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).
- `approvals_reviewer = "auto_review"` sends approval prompts to a reviewer agent that checks for
  exfiltration, credential probing and destructive actions
  ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).
- A thread goal has a `tokenBudget` ([Codex docs](https://developers.openai.com/codex/llms-full.txt)).
- A cloud environment runs a setup script with internet access, an optional maintenance script on
  a cached container, and keeps the agent's internet off by default
  ([cloud environments](https://developers.openai.com/codex/cloud/environments)).

What Priory does that Codex cloud does not, in the pages read: runs on machines you own, priority
bands, and scorers and replay benchmarks.

## Devin

Devin is Cognition's coding agent, run as a service.

What Devin does that Priory does not:

- Budgets for shared pools and user limits block new work on every surface once reached
  ([usage policies](https://docs.devin.ai/enterprise/features/usage-policies)).
- Devin saves corrections and decisions to a memory drive by itself, tags each note with the
  session it came from, and "Dreaming" keeps the notes organized
  ([memory](https://docs.devin.ai/product-guides/memory)).
- Testing mode records a video of Devin testing the change, as proof for the reviewer
  ([testing and recordings](https://docs.devin.ai/work-with-devin/testing-and-recordings)).
- Security profiles bundle network, MCP, git and GitHub CLI limits and bind to an automation
  ([security profiles](https://docs.devin.ai/product-guides/security-profiles)).
- Outposts run sessions on machines you own over an outbound-only connection, while the agent loop
  stays in Cognition's cloud ([Outposts](https://docs.devin.ai/cloud/outposts/overview)).

What Priory does that Devin does not, in the pages read: the agent loop runs on your machines too,
and scorers and replay benchmarks.

## Factory

Factory sells coding agents called droids, with automations and long projects called Missions.

What Factory does that Priory does not:

- Agent Readiness scores a repository on five levels of what an agent needs, and `/readiness-fix`
  fixes the gaps ([Agent Readiness](https://docs.factory.com/agent-readiness/overview)).
- Droid Control records videos to verify claims about the change's behaviour
  ([Droid Control](https://docs.factory.com/automations/droid-control)).
- Missions plan a large project into features and milestones
  ([Missions planning](https://docs.factory.com/missions/planning)).
- Workstreams put an orchestrator over a team of agents
  ([Workstreams](https://docs.factory.com/workstreams/overview)).

What Priory does that Factory does not, in the pages read: priority bands, and replay benchmarks
that compare a configuration change on past tasks. Neither caps spend: Factory's cost page gives
advice and an analytics API
([cost and productivity](https://docs.factory.com/agent-effectiveness/cost-and-productivity)).

## Cursor cloud agents

Cursor's cloud agents, formerly called background agents, run Cursor's agent on isolated machines
([cloud agents](https://cursor.com/docs/cloud-agent)).

What Cursor does that Priory does not:

- Each automation has a memory tool, named notes outside the working tree, on by default
  ([automations](https://cursor.com/docs/cloud-agent/automations)).
- Bugbot reviews pull requests and can fix what it finds ([Bugbot](https://cursor.com/docs/bugbot)).
- PR Routing and Approval approves low-risk pull requests and assigns reviewers only with evidence,
  such as CODEOWNERS or a recent commit ([PR Routing and Approval](https://cursor.com/docs/approval-agents)).
- Self-hosted workers and team pools run the tools on your machines while Cursor keeps the agent
  loop ([self-hosted machines](https://cursor.com/docs/cloud-agent/self-hosted),
  [team pools](https://cursor.com/docs/cloud-agent/self-hosted/pool)).

What Priory does that Cursor does not, in the pages read: the agent loop runs on your machines too,
priority bands, and scorers and replay benchmarks.

## Amp

Amp is Sourcegraph's coding agent. Orbs are the machines its threads run on.

What Amp does that Priory does not:

- A schedule lives on a thread, once or repeating, with a completion condition that clears it
  ([automations and webhooks](https://ampcode.com/docs/orbs/automations)).
- An orb is a cloud machine per thread that sleeps and resumes. Self-hosted orbs connect over
  outbound HTTPS only ([self-hosted orbs](https://ampcode.com/docs/orbs/self-hosted)).
- A project's "Ship Behavior" sets what the Ship button does, by default a push to trunk after the
  tests pass ([shipping changes](https://ampcode.com/docs/orbs/shipping)).
- A checklist says how to make a codebase work well with agents
  ([optimizing your codebase](https://ampcode.com/docs/orbs/optimizing)).

What Priory does that Amp does not, in the pages read: learned memory, priority bands, and scorers
and replay benchmarks.

## Claude Code GitHub Actions and Code Review

Claude Code runs in GitHub Actions on any GitHub event or an `@claude` mention
([GitHub Actions](https://code.claude.com/docs/en/github-actions)). Code Review is Anthropic's
review service for pull requests.

What they do that Priory does not:

- Code Review runs several agents, verifies each finding to drop false positives, and posts the
  findings ranked by severity. It never approves or blocks
  ([Code Review](https://code.claude.com/docs/en/code-review)).
- Code Review has a monthly spend limit and shows the average cost per review per repository
  ([Code Review costs](https://code.claude.com/docs/en/code-review-costs)).
- A run is limited with `--max-turns` and workflow timeouts
  ([GitHub Actions](https://code.claude.com/docs/en/github-actions)).

What Priory does that they do not, in the pages read: memory learned between runs, priority bands,
and an agent that sleeps between events instead of a new Actions run.

## Google Jules

Google is shutting Jules down on 2026-12-31, and it has been read-only since 2026-11-11
([turndown notice](https://jules.google/docs/changelog/2026-10-29)). It is listed for its ideas.

- A Planning Critic reviewed plans nobody approved, and Google reports 9.5 % fewer task failures
  ([Planning Critic](https://jules.google/docs/changelog/2026-01-26-1)).
- Suggested Tasks scanned a repository for `#TODO`s and proposed plans
  ([suggested tasks](https://jules.google/docs/suggested-tasks)).

## Side by side

Each cell sums up the sections above. A cell that says "none found" means the pages read describe
none, not that the product has none.

| Theme | Priory | Mastra and Mastra Factory | AI SDK and eve | OpenHands | Copilot cloud agent | Codex cloud | Devin | Factory | Cursor cloud agents | Amp | Claude Code Actions | Jules |
| ----- | ------ | ------------------------- | -------------- | --------- | ------------------- | ----------- | ----- | ------- | ------------------- | --- | ------------------- | ----- |
| Where it runs | Machines you own, one tmux session and git worktree per task | Mastra's platform, other clouds, or local; Factory sandboxes per session | Vercel, or self-hosted | Cloud, Kubernetes, or a container per conversation | GitHub Actions runners | OpenAI's cloud environments | Cognition's cloud; your machines through Outposts | Factory's cloud or your machines | Cursor's machines; your machines through self-hosted pools | Amp's cloud orbs, or self-hosted orbs | GitHub-hosted or self-hosted runners | Google's cloud, until 2026-12-31 |
| Who approves | A person for each irreversible step; a lead agent can answer other asks | A person, or a phase's condition | A policy per tool: always, once, auto or never | Always, never, or only risky actions | A person through the pull request; changes held below a confidence level | Approval rules, or a reviewer agent | A person through plans and pull requests | Autonomy levels and plan approval | Bugbot reviews; low-risk pull requests approved by policy | The Ship button's project setting | A person; Code Review never approves | A person approves the plan; a critic checks plans nobody approves |
| Scheduling and priority | Schedule triggers and events; bands from blocker to later | Schedules and board phases; no bands found | One file per cron schedule; no queue found | Cron and webhooks | Schedules, repository events and the API | Automations not read | Schedules and chat or tracker events | Schedules, Slack, GitHub and webhooks | Schedules and events from GitHub, Slack, Linear and webhooks | A schedule per thread, with a completion condition | GitHub events and Actions cron | Scheduled and suggested tasks |
| Memory | Files in git; an agent proposes, a person accepts | Message history, working memory and observations | Recall before and capture after each turn | A memory index plus daily logs, opt in | Repository facts with citations | Opt-in memories, details not read | Notes saved by Devin, tagged by session | A generated repository wiki | Named notes per automation | Guidance files only | `CLAUDE.md` and `REVIEW.md`; nothing learned | Existed; docs not read |
| Evaluation | Scorers, calibrated judges and replay benchmarks, run on demand | Scorers, datasets, and gates that block | Eval files against a real agent | A critic that scores a run while it works | None found | None found | Session Insights after a session | Agent Readiness for the repository | Bugbot's resolution status per finding | None found | A verification step inside Code Review | A critic for plans |
| Cost control | Cost per task after the fact; caps only on measuring commands | A dollar limit before each model call | Session timeouts only | Budget settings in the cloud API; cost metrics in the SDK | Budgets; the user budget is a hard stop | A token budget on a thread goal | Budgets that block new work | Advice and an analytics API; no cap found | Usage per agent; no cap found | Orb sizes priced per hour | Turn limits, timeouts; a monthly limit for Code Review | Task quotas by plan |
| Multi-machine | Several runners you own, sharing one event log | Several deployment targets | Durable sessions on Vercel or a Postgres store | Kubernetes and warm pools | GitHub's runners | Cached cloud containers | Cloud machines and Outposts | Several Droid Computers with a shared folder | Team pools of self-hosted workers | An orb per thread, satellites on macOS | GitHub's runners | Google's cloud |
