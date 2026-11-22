# Failure patterns

This page explains how R10.8 works: "A failure pattern groups the tasks that failed, blocked, were
repaired or were marked wrong at one step for the same reason." It holds the model, the first slice,
which is built, the slices after it, and the open questions with a recommendation for each.

## What existed when this note was written

- A failed or blocked task is one desk row, with its reason ([desk.md](../daemon/desk.md)).
- A repair appends a `factory.task.repaired` record with its cause, and `priory label TASK wrong`
  appends a `factory.step.labeled` record ([trace-schema.md](../trace-schema.md)).
- The dream turns a day into memories, which helps the next run of the same kind
  ([dreaming.md](dreaming.md)). It does not say "step `implement` failed 9 times this week for the
  same reason".
- `priory bench capture`, `replay` and `bench --base --head` compare two commits of the factory on
  past tasks ([scorers-and-benchmarks.md](scorers-and-benchmarks.md)). Capture refused a task that
  reached no irreversible step, which is where most failures stop.

So nothing tied a factory change to the failures it was meant to end. LangSmith Engine turns
production traces into tracked issues with their contributing traces, keeps each issue current as
new traces match, and replays an issue's traces against a fix. GitHub Agentic Workflows documents
the same first half as `gh aw logs` with failure clustering.

## The model

1. **Evidence.** A task that is `failed` or `blocked`, a repair, or a person's mark. Each names a
   workflow, a step and a text: the reason, the cause, or `marked wrong by a person`.
2. **A signature.** The text's first line, lower case, with numbers and hashes as `#`. Two reasons
   that differ in a count, a duration or a commit are one signature. Code decides it, so a pattern
   is the same tomorrow as today, and costs nothing.
3. **A pattern.** The evidence of one workflow, step and signature, with an id from their hash.
   Each piece of evidence is a `factory.pattern.matched` trace record, appended once. So a pattern
   needs no table, its count only grows, and the same failure next week joins it.
4. **The desk.** An open pattern of at least 2 tasks is one desk row. One task is already its own
   `failed` or `blocked` row.
5. **What a person does with it.** Close it once a fix merged, and the next match opens it again.
   Dismiss it as no pattern, and it never shows again. Capture its tasks as benchmark cases, and
   compare the fix on them with `priory bench` before it merges.

A nightly code step runs `priory patterns find` over the last 24 hours, so the work costs no model.

## The first slice

Built. [patterns.md](../daemon/patterns.md) describes it.

1. `priory patterns find [DIR] [--since W]` reads the window's failed and blocked tasks, repairs
   and marks, and appends a `factory.pattern.matched` record for each new piece of evidence.
2. `priory patterns [--all] [--json]` and `show ID` list them. The desk shows a `pattern` row per
   open pattern of at least 2 tasks.
3. `priory patterns close` and `dismiss` append a `factory.pattern.closed` record. Both are person
   verbs, as is `capture`.
4. `priory patterns capture [DIR] ID --into FOLDER` captures each task as a case. `priory bench
   capture` now takes a `failed` or `blocked` task that reached no irreversible step, up to the step
   it stopped at, so a failure can be replayed under the fix.
5. The `dreaming` starter runs `find` at 02:45 with a code step.

Tests: grouping by step and signature, a second `find` that records nothing, a new task that grows
the count, the window, repairs and marks, a cleared mark, a close and the match that reopens it, a
match found after a close that stopped before it, a dismiss, the person verbs, the capture of a
failed task, and the capture of a pattern. A `proptest` feeds the signature random text.

It needs no migration: every write is a trace record.

## Later slices

1. A model step that proposes the fix: from a pattern, a task whose agent reads the pattern's
   matches and opens a pull request on the factory, labeled `benchmark`, so the existing
   comparison workflow runs on the captured cases.
2. A web and Herdr panel for patterns, with Close, Dismiss and Capture, from a design a person
   reviewed. Today they show the row as plain text.
3. Record a stop when it happens, in the state, so a task that blocked and was resumed within the
   day is found.
4. A factory key for the least tasks a pattern needs on the desk, now 2.
5. Learn from what a person does: a dismissed signature could lower the weight of similar ones.

## Open questions

1. **Should a model group the texts?** It would join reasons that code cannot, such as two
   phrasings of one fault. It also costs a call a night and moves a pattern's id with the model's
   mood. Recommendation: code only, until patterns that should be one show up in `--all`.
2. **Should a pattern span workflows?** A fault in a shared agent shows at steps of several
   workflows. Recommendation: no. The fix lands in one workflow's step or agent, and the step
   names where to look.
