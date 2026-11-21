# Batches: what is not built yet

This page explains the parts of batches that later slices build. It builds nothing.
[docs/daemon/batches.md](../daemon/batches.md) describes what works today: `priory batch add`, the
plan and its refusals, edits, splits, the go, `merged` and `waiting` moments, stacked entries, the
failure, revive and issue asks, the desk line and the HTTP API.

It is the design round for bowtie idea 10, trimmed to what is left. A **batch** is a set of issues
that one planning task turns into ordered tasks under one ask. An **entry** is one task of a batch.
A **stacked entry** is one with a `waiting` edge: it starts from its stacking parent's branch, and
its pull request is based on it.

## Stacked entries: what is left

- **Revalidate the parent at the start.** When the `waiting` moment is met, the daemon would record
  the parent's top pull request, branch and head on the entry, and revalidate them as the run
  starts. A parent branch that moved in between would refuse the start, and wake no one until a
  person answers an ask on the hold. Today the entry starts from wherever the parent's branch is,
  and only a branch that is gone refuses the start.
- **Tell the entry when the parent pushes again.** After feedback, the parent pushes new commits.
  The daemon would queue a line for the entry: `task 31 pushed priory/31 to <sha>; rebase onto it`.
  Today the entry hears only the parent's merge.
- **Skip the overlap hold for a stacking pair.** The overlap hold ([runs.md](../daemon/runs.md),
  Overlapping files) would skip an entry and its stacking parent, and only them. Otherwise it can
  hold the entry until its parent merges, since both change the same files. The overlap hold sees
  files only after a run, so it never replaces the plan's edges.
- **Handle the retarget before the merge.** GitHub may send the retarget of the entry's pull request
  before the close of the parent's. The daemon records the expected base when the close arrives;
  a retarget seen first, on an entry with several pull requests, could still ask a person.

## Web view

A later slice draws batches in the web view, after a person reviews its boards. `DESIGN.md` gains
these in that slice:

- **Ask detail**: for a go ask, a Plan section first in the reading part, with the lane graph, then
  Closes and Splits. Edit the plan is a ghost button beside Write an answer. Each entry row has a
  menu with Move up, Move down and Drop. A step ask takes no free answer and no dismiss, so the go
  ask shows neither.
- **Batch page** at `/batches/<id>`: a context line, the title in `--type-task`, the stage bar and
  its words, then the lane graph with each entry's state tile.
- **Lane graph**: one row per entry in start order, a lane per column, at most three lanes on a
  phone. A single `--text-3` line is `merged`; a double line is `waiting`. Each row also says it in
  words, so no reader depends on the line style. Nodes are numbered circles before the go and state
  tiles after it.
- **Desk**: the `batch` row under Also waiting, with a muted tile and the `layers` icon, since a
  batch is not a task state. It never ranks as the next thing. The web and Herdr desks draw it today.

The tile shows the daemon's task state, such as `held`; the stage bar shows the batch's word for it,
such as `planned`.

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
