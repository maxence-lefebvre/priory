# Intent and no-op cases on a trigger

This page explains why a trigger names the outcome it exists for and the cases that must cost no
attention, and what reads them. It holds the model, the first slice, and the slices after it.

The first slice is built. [triggers.md](../reference/triggers.md#intent-and-noop) lists the keys,
[score.md](../daemon/score.md#an-intent-scorer) the scorer, and
[trace.md](../daemon/trace.md#no-op-tasks) the report. The later slices are not built.

## Why

Attention per merge is Priory's first measure (see [the PRD](../prd.md#success-measures)), and the
cheapest minute to save is the ask that should never have been raised. A trigger has a filter and a
`pre` script, which decide on an event's fields, not on its meaning. A labeled issue that turns out
to be a question still starts a task that plans, asks and maybe opens a pull request.

GitHub Agentic Workflows makes `intent:` a frontmatter field that must stay valid if the
implementation changes, with no-op conditions: the inverse cases that must not create attention or
writes. Its evaluations are yes or no questions answered `YES`, `NO` or `UNKNOWN`, where a failed
request is not evaluated, never `UNKNOWN`, and missing usage is unavailable, not zero.

## The model

- A trigger takes `intent`, one sentence on one line, and `noop`, a list of cases. A task copies
  both from its trigger when it is created, so editing the trigger changes no running task.
- The task's first prompt carries them after its source event, and asks the agent to end with a
  result, no ask and no write, when a case holds. The agent decides at runtime; Priory states the
  rule.
- The triage decision model reads them in its state, beside the feedback item.
- An `intent` scorer asks a decision model one question about the intent and one per case, and
  grades the task's latest finished run. A case that holds passes only when the task asked and
  wrote nothing.
- `priory report attention` counts, per trigger, the ended tasks that asked and wrote nothing.
  That count needs no model: code decides what code can decide. It also counts the tasks that
  asked or wrote although an intent score found a case, and the person-minutes spent on them.

Nothing here skips a person's approval. A no-op ends a task with no write, so no irreversible step
runs.

## Later slices

1. Calibrate the intent scorer against a person's labeled cases, as a judge is, before its
   verdicts count in the report.
2. Weights: a case that costs more attention, such as a pull request, weighs more than an ask.
3. Bulk verbs: score every ended task of a trigger in one command, so the report needs no
   `priory score` per task.
4. Budget notices: tell the person when a trigger's `should have` minutes pass a set share of the
   week's attention.
5. Show the intent and the cases on the web and Herdr task pages, from a design a person reviewed.
