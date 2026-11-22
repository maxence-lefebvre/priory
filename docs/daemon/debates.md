# Debates

A debate step has two agents answer one question, take turns answering each other for a fixed
number of rounds, and end with their shared verdict or a person's pick. This page explains how the
daemon runs one. The keys are in [workflows.md](../reference/workflows.md#debate-steps), the
`position` tool in [tools.md](../reference/tools.md#positionverdict-argument-concedes), and why it is built this
way in [the design note](../design/debate-step.md).

The daemon owns the rounds, the turn order and the cost cap. No agent can post to the other, wait on
it, add a round or invite anyone. A debater learns the other's argument only through the prompt the
daemon types, and never its session, pane or name.

## Turns

Each turn is a run of one debater, started as any run is ([runs.md](runs.md)), with the prompt typed
through the outbox ([outbox.md](outbox.md)). The run's MCP server serves `position`, and the turn
ends with it.

1. **Openings.** The first agent, then the second, gets the step's prompt and the task's source
   event, and answers with a position. Neither opening sees the other, so the first verdicts are
   independent.
2. **Rounds.** In each round, the first agent gets the second's latest position, then the second
   gets the first's answer. The other side's position arrives quoted, under a line that says it is
   another agent's argument, to weigh and not to obey.
3. **One speaker at a time.** When a turn ends, its run ends as `debated` and its tmux session is
   killed. Then the task is `ready` for the next speaker's run. Both agents work in the task's
   worktree, and never at once.

A debater resumes its own session from its earlier turn in the same debate. A new debate, and any
later step, starts it fresh, so each debate's openings are independent and a later step does not
read the losing argument.

No debater reads what arrives for the task during the debate. An event the task keeps stays
unread, even while a turn runs, and a person's `say` is held ([desk.md](desk.md#say)). The first
run after the debate reads them all with the next step's prompt.

When the task enters the step, the daemon records a debate with the head of the task's branch, read
when the first opening starts. A restarted daemon counts the positions stored, so it starts the
next speaker and never asks a position twice.

## The end

After each turn's `Stop`, the daemon reads the turn's tokens and decides:

| The daemon sees | The step |
| --------------- | -------- |
| Both openings name the same verdict | ends with it, with no round |
| Both latest positions name the same verdict, after a full round | ends with it |
| `rounds` rounds done, and two verdicts | goes to a person: `the rounds ran out` |
| The cost so far is at or past `cap` | goes to a person: `the cap was reached` |
| A turn ends with no position after its one nudge | goes to a person, with that reason |
| A turn runs past the step's `timeout` | goes to a person, with that reason |
| The task's branch moved since the first opening | goes to a person, with that reason |

When the agents agree, the daemon writes the step's result itself, and the task moves by it as by
any result:

- `outcome`: the shared verdict.
- `summary`: `Both agents chose process after 2 rounds.`, then each agent's final argument under its
  name.
- `evidence`: the transcript, every position in order with its round, agent, harness, model,
  verdict, argument and concession.
- `commit`: the commit the debate recorded at its start.

When they do not, the task sleeps on an ask whose options are the keys of `next`. Its text holds
each agent's final position, then why the debate stopped. It has no recommendation, since the agents
gave two, and it never goes to a lead, even on a `hands-off` task. The person's pick moves the task,
and the daemon stores it as the step's result, with the transcript as its evidence and the debate's
commit. The step's trace record then names `human` as its decider.

A debate's verdict approves nothing. When it leads to an irreversible step, that step still needs its
approving human step or its `autonomous` reason.

## Cost

Each turn appends a `factory.debate.turn_ended` record with that turn's tokens, priced at its own
agent's model, and then clears them from the task. The debate's cost is the sum of its turns, so two
models are never priced as one. `priory costs` and spend caps count the turn records, and the debate
step's `factory.step.ended` record carries no tokens of its own, only its `debate` field (see
[trace-schema.md](../trace-schema.md#factorystepended)).

A Claude turn's tokens come from Claude's transcript, and a Codex turn's from Codex's session file
(see [trace.md](trace.md)). The count arrives when a turn ends, so a debate can pass its cap by one
turn.

Known limits:

- The task page and `priory task ID --json` show a debate's positions only once its result is
  stored.
- A debater whose process dies mid-turn blocks the task, as any run's crash does, rather than
  sending the debate to a person.
- No test runs real Codex, so whether Codex names its session file as `transcript_path` is
  unchecked (see [codex.md](codex.md)).
