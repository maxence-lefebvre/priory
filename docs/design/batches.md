# Batches: what is not built yet

This page explains the parts of batches that later slices build. It builds nothing.
[docs/daemon/batches.md](../daemon/batches.md) describes what works today: `priory batch add`, the
plan and its refusals, edits, splits, the go, `merged` and `waiting` moments, stacked entries, the
failure, revive and issue asks, the desk line, the HTTP API and the web view.

It is the design round for bowtie idea 10, trimmed to what is left. A **batch** is a set of issues
that one planning task turns into ordered tasks under one ask. An **entry** is one task of a batch.
A **stacked entry** is one with a `waiting` edge: it starts from its stacking parent's branch, and
its pull request is based on it.

## Stacked entries: what is left

- **Revalidate the parent at the start.** Built: see
  [daemon/batches.md](../daemon/batches.md#the-waiting-condition-and-stacked-entries).
- **Tell the entry when the parent pushes again.** Built: see
  [daemon/batches.md](../daemon/batches.md#the-waiting-condition-and-stacked-entries).
- **Skip the overlap hold for a stacking pair.** The overlap hold ([runs.md](../daemon/runs.md),
  Overlapping files) would skip an entry and its stacking parent, and only them. Otherwise it can
  hold the entry until its parent merges, since both change the same files. The overlap hold sees
  files only after a run, so it never replaces the plan's edges.
- **Handle the retarget before the merge.** GitHub may send the retarget of the entry's pull request
  before the close of the parent's. The daemon records the expected base when the close arrives;
  a retarget seen first, on an entry with several pull requests, could still ask a person.

## Open questions

Each with a recommendation, taken for anything reversible.

1. **Add an issue to a going batch?** Start a second batch. A later slice can let `priory batch add
   --after TASK` make the new batch's roots wait on a task of the first.
2. **Cancel a going batch in one verb?** `priory cancel` per task works. Add `priory batch cancel`
   if the desk line needs a Cancel button.
3. **Several runners?** Batches stay refused in a factory of several runners until a slice adds
   claims: a local transaction cannot stop another runner from claiming a request on the same issue.
4. **Cap a chain of stacked entries?** `max_prs` caps one task's own stack. The plan adds no cap
   across tasks; add one if long chains of stacked entries prove hard to review.
