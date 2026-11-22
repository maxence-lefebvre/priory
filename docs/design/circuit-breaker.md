# A circuit breaker on a repeated failure

This page explains how the daemon could stop a fix loop that makes no progress: a step's gate, a
red check or a repair that fails the same way as the attempt before. It holds the model, the slice
that is built, the slices after it, and the open questions with a recommendation for each.

The first slice is built: [A fix that fails the same way](../daemon/waits.md#a-fix-that-fails-the-same-way)
describes it. It covers the red checks of the done gate, the `ci-fix-exhausted` reason. The later
slices are not built.

## Why

A spend cap ends a loop late, after the money is gone, and it cannot tell a loop from honest work
([spend.md](../daemon/spend.md)). A repeated failure can: a fix whose checks fail with the same
signature as the commit before it changed nothing the checks can see. That is a fact code can
check, so the daemon decides it, as Principle 2 of the [PRD](../prd.md#principles) asks. What to do
next is not, so a person decides.

Other factories do the same. GitHub Agentic Workflows' loop guide says to "attempt bounded fix
retries, stop on repeated identical signature, pause with structured reason". Cline's SDK stops a
session with `finishReason: "mistake_limit"` after consecutive recoverable mistakes. Kiro Crew's
Task Runner retries "up to 3 attempts, then re-plan".

## What existed when this note was written

- The checks gate wakes the agent on every red check at its result's commit, with no bound
  (`judge` in `crates/priory-core/src/tools.rs`, [waits.md](../daemon/waits.md#done-and-checks)).
  A result that names a commit whose checks already failed wakes the agent again at once.
- A repair restarts a run once, then leaves the task failed ([health.md](../daemon/health.md#repair)).
- The stall handler makes at most 3 calls per run ([health.md](../daemon/health.md#stalls-and-the-handler)).
- `priory gate` sends the daemon only its commit and exit code, not its output
  ([tools.md](../daemon/tools.md#proofs)).
- A check event carries a check's name and conclusion, not its error text
  ([events.md](../reference/events.md#githubcheck_runcompleted)).
- The daemon already asks a person about a stack, with a named agent ask that ends the task's gates
  (`ask_stack` in `tools.rs`).

## The model

A failure has a signature. A loop is two consecutive attempts with the same signature. On a loop,
the daemon does not wake the agent: it ends the task's gates and puts one agent ask on the desk,
named after a typed reason:

| Reason | Loop | Signature |
| ------ | ---- | --------- |
| `ci-fix-exhausted` | the done gate's red checks | the red checks' names and conclusions |
| `stuck` | a failing `priory gate` | the exit code and the first error line |
| `ci-timeout` | a check that never finishes | none: the time the gate has waited |

The options are `try again` and `stop`. An answer ends the streak, so each answer buys one attempt.

## Later slices

1. `stuck`: `priory gate` keeps the first error line of its output, the trace record carries it,
   and two failed gate runs in a row with the same line and exit code ask a person. The run is
   mid-turn, so the ask holds the task at the turn's end. See open question 1.
2. `ci-timeout`: a gate whose checks stay pending past a factory duration asks a person.
3. The first error line of a check: the intake reads the check run's output, so a fix that changes
   which test fails inside one job no longer matches.
4. A `factory.loop.stopped` trace record with the reason and both signatures, so a report can count
   loops by workflow.
5. Budget notices: tell the agent how much of its step's budget is left, so it wraps up instead of
   being cut off.

## Open questions

1. **Can the daemon stop a `priory gate` loop inside a turn?** It cannot end a turn early (see
   [urgent wakes](urgent-wakes.md#open-questions)). Recommendation: `priory gate` exits with a line
   that tells the agent a person decides next, and the ask's open state makes the run end asleep at
   its `Stop`.
2. **Should a factory turn the breaker off or raise its count?** Recommendation: not until a month
   of asks shows false trips. A count of 2 matches the loop guide.
