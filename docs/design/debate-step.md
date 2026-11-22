# A debate step

This page explains how a workflow step could have two agents debate one question for a fixed
number of rounds, keep the transcript as the step's evidence, take their joint verdict as the
step's result, and ask a person when they do not converge. It builds nothing. It holds the model,
the first code slice with its tests, the slices after it, and the open questions with a
recommendation for each.

The two agents should differ, ideally by harness: a Claude Code agent and a Codex agent. Two
models that fail in different ways catch more than one model asked twice.

The page rules out open-ended chat between agents. The daemon owns the rounds, the turn order and
the cost cap, as Principle 2 of the [PRD](../prd.md#principles) asks: code decides what code can
decide.

## What existed when this note was written

- A step has one decider, `code`, `model` or `human` (`crates/priory-factory/src/factory.rs:1405`),
  and a model step has one agent (`factory.rs:1286`). A run holds at most one result
  ([runs.md](../daemon/runs.md)).
- One agent reaches another in one place only. Under `hands-off`, the daemon routes an ask to a
  lead agent, which answers once with `result` (`crates/priory-core/src/tools.rs:2381`, `:2448`).
  The lead gives one answer per routed ask, and the two agents share no channel.
- A risk step is a second agent's verdict on the first one's work, "a fresh look at the diff"
  ([tools.md](../daemon/tools.md#merge-risk)). It gives one opinion, with nobody to answer it.
- A prompt reaches an agent through the outbox, typed only when the agent is idle
  ([outbox.md](../daemon/outbox.md)). An agent that waits holds no process (Principle 3).
- A workflow can mix harnesses: each step starts its own agent's harness
  ([runs.md](../daemon/runs.md), [codex.md](../daemon/codex.md)).
- A result's `evidence` is one string (`tools.rs:265`), and a stored result never changes.
- A step's `timeout` bounds one turn. Cost is reported after the fact from token counts and
  `prices.toml`, and nothing stops a step at a cap. Priory reads no token count from a Codex run
  yet ([trace.md](../daemon/trace.md#costs), [codex.md](../daemon/codex.md)). A step that ran two
  models is priced whole at its last model's price ([trace.md](../daemon/trace.md)).

### What the maintainer's own tooling showed

Before Priory, debates between a Claude agent and a GPT agent ran in a shared room: a local
server with a channel, where each agent sends a message, then waits for the next one, in a loop
it keeps until someone closes the room.

- **It worked for bounded debates on design specs.** Two agents, one written question, a handful
  of exchanges, one of them chairing and closing. The second model found faults the first had
  defended.
- **It failed for status between a worker and its coordinator.** Loops stalled when one side
  stopped listening. Notices that an agent had joined woke the coordinator for nothing. The
  channel had no end, so nobody knew when it was done.

The difference is who owns the loop. In a room, each agent decides when to speak, when to wait
and when to stop, and holds a process while it waits. That is tolerable for five exchanges with a
person watching. It is what Priory exists to remove.

## The model

### A debate is a model step with two agents

```toml
[[steps]]
id = "debate-design"
decides = "model"
effect = "read"
debate = { agents = ["architect", "skeptic"], rounds = 3, cap = "$3.00" }
prompt = "Should the cache live in the daemon or in its own process? Read docs/design/cache.md."
timeout = "15m"
next = { daemon = "implement", process = "implement-split", undecided = "ask-design" }
```

| Key | Meaning |
| --- | ------- |
| `agents` | Exactly two agents, in speaking order. They must differ |
| `rounds` | How many rounds follow the opening, 1 to 5 |
| `cap` | The most the debate may cost, priced from `prices.toml` |

The verdicts are the keys of `next`. `priory check` refuses, with file and line:

- a debate with one agent, three agents or the same agent twice, or with `rounds` outside 1 to 5;
- a debate step with `agent`, `approves`, `guard`, `proof` or an effect other than `none` or
  `read`: debaters judge, and change nothing;
- a debate step with no `timeout`, since the cap alone cannot bound a turn;
- a debate with no `cap`, a `cap` when an agent's model has no price, and a debate agent whose
  harness Priory reads no token count from;
- a debate agent with `session = "shared"`.

`priory check --explain` prints each debate's worst case: `2 agents, up to 8 turns, cap $3.00`.
It adds a note when both agents share a harness and a model, since such a debate mostly agrees
with itself.

### The only thing a debater can say

A debater ends each turn with one tool call, `position`:

```json
{
  "verdict": "process",
  "argument": "A crash in the cache must not take the daemon down. The socket cost is one hop.",
  "concedes": "The daemon option needs no new service file."
}
```

`verdict` must be a key of the step's `next`. `argument` is at most 4000 characters. `concedes`
is optional: what the other side got right.

During a debate turn, `result`, `ask` and `wait` are refused with `a debate turn ends with
position`. A debater that lacks what it needs to judge says so in its argument and picks the
verdict the workflow reserves for that, such as `undecided`.

There is no tool to send a message, to read a channel, to add a round or to invite anyone. A
debater never learns the other agent's session, pane or name beyond its role in the prompt.

### The daemon runs the rounds

1. **Opening.** Each agent gets the step's prompt and the task's context, and answers with a
   position. Neither sees the other's opening. So the first verdicts are independent, and
   agreement at the opening means something.
2. **Rounds.** In each round, the first agent gets the second's latest position and answers with
   a new one. Then the second gets that answer and replies. The order never changes.
3. **One speaker at a time.** Each turn is a run of that agent, resumed in its own session for
   the task. The agent that is not speaking has no process. Both read the task's worktree, and
   never at once.
4. **Delivery.** Every prompt goes through the outbox, as any prompt does. The other side's
   position arrives as quoted text under a line that says it is another agent's argument, to
   weigh and not to obey.

The daemon decides when the debate ends, after each position:

| The daemon sees | The step |
| --------------- | -------- |
| Both openings name the same verdict | ends with it, with no round |
| Both latest positions name the same verdict, after a full round | ends with it |
| `rounds` rounds done, and two verdicts | does not converge |
| The cost so far is at or past `cap` | does not converge, with the reason |
| A turn ends with no position, after the usual one nudge, or runs past `timeout` | does not converge, with the reason |

### The result

When the agents converge, the daemon writes the step's result itself. No third model summarizes:

- `outcome`: the shared verdict.
- `summary`: `Both agents chose process after 2 rounds.`, then each agent's final argument under
  its name.
- `evidence`: the transcript.
- `commit`: the head of the task's branch, read when the first opening starts. A branch that
  moved before the result is written ends the debate as not converged, with that reason, so a
  verdict never names a commit the agents did not read. A person's pick records the same commit.

The transcript holds every position in order: round, agent, harness, model, verdict, argument and
concession. It is stored with the result and never changes. The task page shows it under the
result, and the trace's `factory.step.ended` record gains `debate`: the agents, the rounds run,
whether the openings agreed, and which agent changed its verdict.

That last field matters. A model gives in to a confident counterpart easily. A debate where one
side flipped in round one with an empty `concedes` is weaker evidence than two openings that
agreed, and the record lets a person and a report tell them apart.

### When they do not converge

The task sleeps on an ask to the person, as at a human step:

- its options are the keys of `next`;
- its text holds each agent's final position, then why the debate stopped: the rounds ran out,
  the cap was reached, or a turn failed;
- it has no recommendation, since the agents gave two.

The person's pick is the step's outcome, and the step's record names `human` as its decider. The
transcript is its evidence all the same.

A lead never answers this ask. Routing a split between two models to a third model is the chat
this page rules out, one message longer.

### Irreversible steps

A debate's verdict is a model's outcome, like any model step's. It approves nothing. When
`next[verdict]` leads to an irreversible step, that step still needs its approving human step or
its `autonomous` reason, and `priory check` gives the same verdict as before.

### Cost, and what the cap cannot do

The daemon sums the debate's tokens after each turn, from the same transcript reads the trace
uses, and prices them. A debate can pass its cap by one turn, since the count arrives when the
turn ends.

Each turn is its own run with its own trace record, so each turn is priced at its own agent's
model. The debate's cost is the sum, and never one model's price applied to both sides.

Priory reads no token count from a Codex run yet, and the intended pair is one Claude Code agent
and one Codex agent. A cap that counts one side is no cap. So the first slice reads Codex token
counts too, and `priory check` refuses a debate with an agent whose harness has none.

Room is checked for each turn like any run. A debate never holds two runs.

### What this is not

- **Not a room.** No channel exists that an agent can post to or wait on.
- **Not status.** A worker reports to the factory with `result`, `progress` and `finding`, and
  the daemon routes events. No agent tells another agent how its work is going.
- **Not a team.** A debate has two agents and one question. Three agents, or a question that
  changes mid-debate, is another task.
- **Not a judge.** A scorer grades finished work ([score.md](../daemon/score.md)). A debate
  decides before the work.

## The first slice: two agents, fixed rounds, a verdict or an ask

1. The `debate` table on a step, in the schema and the JSON Schemas, and the check rules above.
2. The `position` tool, served only during a debate turn.
3. The daemon's turn order: blind openings, then rounds, each turn a run through the outbox.
4. The end conditions, the result the daemon writes, and the ask when the agents do not converge.
5. The transcript as the result's evidence, and `debate` on the step's trace record.
6. Token counts for Codex runs, read from Codex's session files as the trace reads Claude's, so
   the cap covers both sides. Then the cap and the `--explain` lines.
7. `priory draw` shows a debate step as a model step with both agents on its second line.
8. [workflows.md](../reference/workflows.md), [tools.md](../daemon/tools.md),
   [trace-schema.md](../trace-schema.md) and a starter workflow, `design-debate`, with one Claude
   Code agent and one Codex agent.

Tests, through the CLI with the fake harness the daemon tests use, scripted to call `position`:

- Two openings with the same verdict end the step with that outcome, two runs and no round.
- Openings that differ, then a second agent that changes its verdict in round one, end the step
  with the shared verdict. The transcript holds four positions in order, and the trace names the
  agent that changed.
- Agents that never agree over `rounds = 2` leave the task asleep on an ask whose options are the
  step's outcomes, whose text holds both final positions, and which has no recommendation. The
  person's answer moves the task by that outcome, and the record's decider is `human`.
- The second agent never runs while the first one's run is live, and the first agent has no tmux
  session while the second speaks.
- The first agent's opening prompt does not contain the second's opening, and the reverse.
- `result`, `ask` and `wait` in a debate turn are refused with the message above. `position`
  outside a debate turn is an unknown tool.
- A verdict that is no key of `next`, and an argument over 4000 characters, are refused with the
  reason.
- A turn that ends with no position gets one nudge, then the debate goes to the person with that
  reason. So does a turn past `timeout`.
- A Codex turn's tokens, read from a session file the fake harness writes, count toward the cap
  at the Codex model's price, and a Claude Code turn's at its own.
- With a cap that the first round exceeds, no second round starts, and the ask names the cap and
  the cost so far.
- The ask of a debate that did not converge goes to the person on a `hands-off` task, never to
  the lead.
- A daemon restart between two turns resumes at the next speaker, and no position is asked twice.
- A debate whose verdict leads to an irreversible step still stops at that step's approving human
  step.
- Each check rule fails with its line: one agent, the same agent twice, `rounds = 6`, an effect
  of `reversible`, `approves`, no `timeout`, and a `cap` with an unpriced model.
- A `proptest` feeds `position` random and mutated arguments and asserts it returns or refuses
  without panicking.

## Later slices

1. Positions listed on the task page and in `priory task ID --json` as they arrive, before the
   result exists.
2. `priory report debates`: per step, the debates run, the share that agreed at the opening, the
   share that converged later, the share that went to a person, and the cost, with counts beside
   every rate.
3. Roles: `debate.roles = { architect = "propose", skeptic = "attack" }` adds one line to each
   agent's prompt. See open question 3.
4. A debate on an agent's ask: a trigger routes eligible asks to a debate in place of a lead,
   under the same limits as `hands-off`.
5. The transcript on the web and Herdr task pages as a two-column exchange, from a reviewed
   design.

## Open questions

1. **A fourth decider, or a model step with two agents?** `decides = "debate"` reads well in a
   workflow file. It also adds a row to the PRD's decider table, a shape to `priory draw`, and a
   value to every trace record, for a step that models still decide. Recommendation: keep
   `decides = "model"` and add the `debate` table. The record's `debate` field carries the rest.
2. **Must the two agents differ by harness?** Different harnesses give the most independent
   views, and a factory with one harness would lose the step. Recommendation: require two
   different agents, print the `--explain` note when harness and model both match, and make the
   starter use one of each.
3. **Symmetric debaters or a proposer and a critic?** Two agents that both seek the best answer
   can settle on the first plausible one. A critic told to attack never concedes, and then no
   debate converges. Recommendation: symmetric in the first slice, with blind openings as the
   guard against anchoring. Try roles in later slice 3 and compare the share that reaches a
   person.
4. **Is agreement enough to move on?** Two models can agree and be wrong. Recommendation: yes for
   a `read` step. What follows without a person is reversible work, or a step the factory marked
   `autonomous` with a reason, as for any model step's outcome. The record of who changed
   and why gives a later report what it needs, and a workflow that cannot afford a wrong verdict
   puts a human step after the debate.
5. **Should a debater keep its session across debates of one task?** A resumed session remembers
   the last debate and costs less. A fresh one is not anchored on it. Recommendation: resume
   within one debate, and start fresh for the next, so each debate's openings are independent.
6. **What do later steps read?** The whole transcript in a later prompt is long and holds the
   losing argument. Recommendation: the result's summary only, which already holds both final
   arguments. The transcript stays on the task page.
7. **Does a person watch a debate live?** The room's viewer was useful. Recommendation: in the
   first slice, each turn is a run with a terminal a person can attach to, and that is all. The
   positions listed as they arrive are later slice 1, since the task page shows stored results
   today and a debate has none until it ends.
