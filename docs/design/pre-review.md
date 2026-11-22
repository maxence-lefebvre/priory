# Pre-review against past rejections

A person who rejects an approval says why. Without a record of it, the next task makes the same
mistake, and the same person spends another minute to reject it again. This note keeps the plan for
applying those reasons at the gate, before an approval ask reaches the desk, from the second scout
report of other agent factories (idea 9).

Three tools inspired it, from their public pages on 2026-11-22. CrewAI's
`@human_feedback(learn=True)` extracts lessons from a person's feedback, and recalls them to improve
the next output before the person sees it. LangSmith Engine learns from the reason given when an
issue is dismissed. Kiro Crew's reviewer is an independent session that reads the `git diff`, not
the worker's summary.

Principle 4 of the [PRD](../prd.md#principles) does not move: a pre-review only sends a result
back. An irreversible step still runs only after a person approved it.

## The first slice: lessons and the pre-review gate

Built. See [score.md](../daemon/score.md#pre-review).

1. A memory's `step`, `<workflow>/<step>`, makes it a lesson for an approval step: a human step
   with `approves`. `priory check` refuses a `step` that names no such step.
2. A person's answer `OPTION: NOTE` at an approval step, whose option stops the task or sends it
   back to a model step, proposes a lesson: a memory candidate with the note as its text. It waits
   for the existing accept. The candidate carries `step` on the log, and the state file stores it.
3. `pre_review` in `factory.toml` names a judge. A result whose outcome leads to an approval step
   with lessons is held for it, as a gate holds one. The judge reads the lessons after the request,
   the result and, on `diff`, the diff. Labeled cases take `lessons` too, so calibration measures
   the same question.
4. A refusal from the pre-review sends the result back once per run, with the lessons. The next
   result reaches the person. It never blocks the task.

## Later slices

1. Lessons from a review with changes requested on the task's pull request, not only from an ask.
2. A model that turns a note into a general lesson before the person accepts it, and merges a new
   lesson into a close one, so the list stays short.
3. A count of the asks a pre-review saved, and of results it sent back that a person approved
   unchanged anyway, per step, in `priory report`.
4. The desk shows that a result went back for a lesson, from a reviewed design.

## Open questions

1. Should a lesson apply across workflows that share a step id? Recommendation: no. A step id such
   as `approve-pr` means different things in different workflows, and a person can copy a lesson.
2. Should the rejected task's own lesson apply before a person accepts it? Recommendation: yes, as
   every candidate reaches its own task's later runs. The person wrote the note for that task.
