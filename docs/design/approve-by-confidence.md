# Approve by confidence

This page explains how the daemon could answer a reversible ask with the agent's own
recommendation, when the agent states a confidence and a person set the level that is enough. It
builds nothing. It holds the model, the first code slice with its tests, the slices after it, and
the open questions with a recommendation for each.

The first slice is built, with every open question's recommendation taken:
[waits.md](../daemon/waits.md#answered-by-auto) describes it. It differs from this note in four
ways. A desk item of an ask carries `confidence` and `rationale` too, so the desk can show them
beside the recommendation. `priory asks` prints no new columns: the JSON carries the fields. A
batch's entries take their planning task's level, as they take its `intervention`. The daemon
drops the push its insert of an auto-answered ask queued, in the same transaction. The later slices
are not built.

The task page display is built too, from a reviewed design: the web view's task page lists every
answered ask with who answered it, and an auto answer's confidence, rationale and trade-off. The
Herdr Task panel shows the answerer and an auto answer's confidence and rationale.
[waits.md](../daemon/waits.md#answered-by-auto) describes both.

The name is the scout report's. The feature answers a question. It never grants an approval.

The idea comes from a read of other agent factories on 2026-11-22. GitHub Copilot rates each issue
change high, medium or low, and a repository's [automation
level](https://docs.github.com/en/copilot/concepts/agent-capabilities/agent-powered-workflows-and-operations/about-automation-rationale-and-approvals)
sets which ones apply without a person, each with a kept rationale. [Mastra
Factory](https://factory.mastra.ai/llms-full.txt) phases have "Approve only when" conditions.
[Cursor](https://cursor.com/docs/approval-agents) approves low-risk pull requests from policy files.

Principle 4 of the [PRD](../prd.md#principles) does not move: an irreversible step runs only after
a person approved that step, unless the factory marks it autonomous with a reason.

## What existed when this note was written

- The `ask` tool takes `question`, `options`, `recommendation` and `tradeoff`
  (`crates/priory-core/src/tools.rs:636`, `:657`, `:670`). It has no confidence and no reason for
  the recommendation beyond the trade-off ([tools.md](../reference/tools.md#askquestion-options-recommendation-tradeoff-confidence-rationale-title-text-whatwaits-nextaction-command)).
- A trigger's `intervention` has two values, `plan` and `hands-off`
  (`crates/priory-factory/src/factory.rs:872`, `:1074`). Unset, every ask goes to a person
  ([triggers.md](../reference/triggers.md#intervention-and-lead)).
- `hands-off` sends an ask to a lead agent. `lead_of` picks the asks a lead may answer
  (`crates/priory-core/src/tools.rs:2381`). It refuses an ask that runs a command, an ask at a step
  with `approves`, and an ask whose answer can let an irreversible step run before a person decides
  again (`crates/priory-factory/src/explain.rs:141`). Asks the daemon makes itself never reach it.
- A routed ask costs a lead task: a worktree, a run and its tokens (`tools.rs:2418`).
- An answer records who gave it in `answered_by`: `cli`, `serve` or `agent:<lead>`
  (`tools.rs:3102`).
- The risk estimate must never answer an ask or widen what a lead may answer
  ([risk-estimate.md](risk-estimate.md#what-it-must-never-be-used-for)).

## The model

### What the agent states

`ask` gains two optional fields:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `confidence` | `high`, `medium` or `low` | How sure the agent is that its recommendation is what the person would pick |
| `rationale` | one line, at most 200 characters | Why, in words a person can check later |

The tool refuses a `confidence` with no `rationale`, a rationale with a line break, and a
confidence on an ask whose `recommendation` is not one of its `options`. An ask without the fields
behaves as today.

### What a person sets

A trigger gains `auto_answer`, the lowest confidence the daemon may act on:

```toml
source = "github"
repo = "owner/name"
on = "issues.labeled"
workflow = "implement"
auto_answer = "high"          # or "medium"; unset: no ask is answered this way
```

`priory request --auto-answer` and `priory add --auto-answer` set it for one task. The task keeps
the level its trigger had when it was created, as it keeps `intervention`. `low` is refused: a
level that accepts every ask is `hands-off` with no reader.

### Which asks are eligible

An ask is eligible when all of these hold:

1. Its task has an `auto_answer` level, and the ask's `confidence` is at or above it.
2. A lead could answer it under `hands-off`: the same function decides, so the two lists cannot
   drift. That leaves out an ask whose answer can let an irreversible step run, a human step that
   `approves`, an ask that runs a command, and every ask the daemon makes itself.
3. An agent's `ask` call made it. A human step's ask has no agent behind its options and is never
   eligible.
4. It belongs to a task. An outside ask ([asks.md](../daemon/asks.md)) is never eligible.

Rule 2 is the one that keeps Principle 4. It is the workflow's shape that decides, read from the
file, and never the stated confidence.

### What the daemon does

In the transaction that stores an eligible ask, the daemon:

- stores the recommendation as the answer, with `answered_by = "auto"`;
- appends a `factory.ask.auto_answered` record to the trace with the task, step, ask, revision,
  option, confidence, rationale and level;
- queues the answer as the task's next prompt, as a person's answer would.

The turn still ends, since `ask` ends a turn. The task wakes at once, so it holds no process in
between only for the length of a tick.

An auto-answered ask never notifies. It shows in three places:

- `priory asks --all` and `/api/asks` carry `confidence`, `rationale` and `answered_by`.
- The desk's Since you looked lists it: `ask 14 answered "document --format" by auto (high): the
  flag duplicates --format`.
- The task page shows the ask with its rationale, as it shows an answered ask today.

An ask below the level goes to the desk, with its confidence and rationale beside the
recommendation. That alone helps a person answer faster.

### With `hands-off`

A task can have both. The daemon answers first when the ask is eligible. Otherwise the ask goes to
the lead, as today. A `low` ask skips the lead and goes to the person: the agent said it does not
know, and a second model's guess is not a person's answer. See open question 3.

### Why an agent's own word is enough here, and where it is not

A confidence is a claim by the party that wants to go on. Four things bound the damage:

- Only reversible work follows an auto-answer. The irreversible step still waits for a person, who
  reads the result and the list of auto-answered asks before approving.
- The level is a line in a trigger file, changed by a pull request.
- Every auto-answer leaves a record with its rationale.
- A person can overturn one (later slice 2), and the share overturned is the measure of whether
  the level is right.

The confidence never reaches a place that widens it: it is not a grade, not a merge risk, and no
input to queue order or routing.

## The first slice: fields, level and the auto-answer

This slice is built. See the top of this page for how it differs.

1. `ask` takes and stores `confidence` and `rationale`, and the listings and the task page's API
   carry them.
2. Triggers, `priory request` and `priory add` take `auto_answer`. `priory check` refuses `low` and
   an unknown value, with its line.
3. The daemon answers an eligible ask as above, and appends `factory.ask.auto_answered`
   ([trace-schema.md](../trace-schema.md) gains the record).
4. Since you looked lists auto-answered asks.
5. [tools.md](../daemon/tools.md), [triggers.md](../reference/triggers.md) and
   [events.md](../reference/events.md) gain the fields, the key and the record.

The slice changes no web view file. Since you looked is the CLI's `priory desk` and the API's
list, which the web view already draws line by line. A new control starts from a reviewed design,
as [CONTRIBUTING.md](../../CONTRIBUTING.md#ui) asks.

Tests, through the CLI and the MCP bridge as `crates/priory/tests/` does for asks:

- An ask with `confidence: high` on a task with `auto_answer = "high"` is answered by `auto` in
  the same tick, the task wakes with the recommendation, and the trace holds one
  `factory.ask.auto_answered`.
- The same ask at `medium` goes to the desk. With `auto_answer = "medium"` it is answered.
- An ask at a step whose answer leads to an irreversible step goes to the desk at any confidence
  and any level. So does a human step with `approves`, an ask with a `command`, a plan ask, a
  hold's ask and an approval asked again at a new commit.
- A property test over generated workflows: the eligible asks are a subset of the asks `lead_of`
  accepts.
- `priory check --explain` prints the same irreversible steps and the same paths with no human
  decision, with and without `auto_answer`.
- A confidence without a rationale, a rationale of two lines, and a recommendation that is not an
  option are refused with the reason.
- An ask with no confidence on a task with a level goes to the desk.
- A person's `priory answer` on an auto-answered ask is refused: the first answer stands.
- `auto_answer = "low"` fails `priory check` with the trigger's line.

## Later slices

1. `result` takes `confidence` and `rationale` on a grading step. Under `intervention = "plan"`, a
   green plan at or above the level moves on with no ask, when the grade leads to a reversible
   step. The plan and its rationale stay on the task page.
2. `priory overturn ASK TEXT`, a person verb in the `priory-person` guard. It appends
   `factory.ask.overturned` and sends the text to the task as `priory say` does.
3. `priory report asks`: per trigger and level, the auto-answered count, the overturned count and
   the share, with counts beside every rate.
4. An audit sample: one eligible ask in N goes to the desk anyway, with the confidence hidden
   until the person answers. The share where the person picked another option is the unbiased
   error rate. See open question 2.
5. The web and Herdr Desks show confidence and rationale on an open ask, from a reviewed design.

## Open questions

1. **Three levels or a number?** A number from 0 to 1 looks precise, and models are poor at it.
   Three words match Copilot's levels and the grades Priory already has. Recommendation: three
   levels. Add nothing finer until later slice 3 shows that `high` holds two different error rates.
2. **How does anyone learn that `high` is wrong?** Overturns count only the asks a person happened
   to read. Recommendation: ship the first slice with overturns planned, then the audit sample
   (later slice 4) before anyone sets `auto_answer = "medium"` on a busy trigger. `1 in 10` is a
   fair default.
3. **Should a `low` ask skip the lead?** The lead may know more than the task's agent, since its
   session spans tasks. It also answers with the same confidence a person cannot see.
   Recommendation: skip the lead on `low`. It costs a person one answer and saves a lead run on
   the asks most likely to be answered wrongly.
4. **Should the level live on the trigger or the workflow step?** A step knows which questions are
   cheap. A trigger knows how much its owner trusts the source. Recommendation: the trigger, like
   `intervention`, so one concept says how much a person steps in. Add a step key
   `auto_answer = false` only when a factory needs one step to always ask.
5. **Does the risk estimate calibrate the level?** The scout report suggests it. The estimate's own
   rules forbid it from answering asks. Recommendation: keep them apart. Later slice 3 reports its
   own rate, and a person changes the level by hand. No code changes a level from a number.
6. **Can an agent raise its confidence to avoid waiting?** Yes, and nothing in a prompt prevents
   it. Recommendation: accept it inside the bounds above, never tell the agent its task's level,
   and let the audit sample show it.
