# Batches: what is not built yet

This page explains the parts of batches that later slices build. It builds nothing.
[docs/daemon/batches.md](../daemon/batches.md) describes what works today: `priory batch add`, the
plan and its refusals, edits, splits, the go, `merged` moments, the failure, revive and issue asks,
the desk line, the HTTP API and the web view.

It is the design round for bowtie idea 10, trimmed to what is left. A **batch** is a set of issues
that one planning task turns into ordered tasks under one ask. An **entry** is one task of a batch.

## Start moments

Each edge of a plan reads `ENTRY after OTHER at MOMENT`:

| Moment | Met when | Use |
| ------ | -------- | --- |
| `merged` | Built. Every pull request of the other task's stack merged, or the other task entered an `after = "merged"` step, or it is done owning no pull request | The entry needs the other's code on the default branch |
| `waiting` | The other task owns an open pull request and sleeps on the gates that `done` or an `after = "merged"` step stores: its work is finished, and it waits on review, checks or the merge. `merged` also meets it | The entry stacks on the other's branch: it starts from that branch and its pull request is based on it |

`done` alone never meets `merged`, since a task with one pull request is `done` at green checks
while its pull request is still open. Today a plan with a `waiting` edge is refused with
`` entry <e> waits on <other> at `waiting`, which Priory does not build yet; use merged ``.

## Stacked entries (the `waiting` moment)

A `waiting` edge makes a stack across two tasks. It differs from a task's own stack in one way: the
stack spans two tasks.

- An entry has at most one `waiting` predecessor, its stacking parent. A plan with two is refused:
  `entry <e> has <n> waiting predecessors; give it one stacking parent`.
- The go records the parent as the entry's `tasks.base_task`.
- When the `waiting` moment is met, the daemon records the parent's top pull request, branch and
  head on the entry, and revalidates them as the run starts. A parent branch that moved or vanished
  in between refuses the start, and wakes no one until a person answers an ask on the hold.
- The entry starts its worktree from the parent's branch, such as `priory/31`, instead of the
  default branch. Its `open-pr` bases its bottom pull request on that branch. Its own stack still
  fits its own workflow's `max_prs`, which keeps its meaning: a cap on one task's own stack.
- When the parent pushes again, after feedback, the daemon queues a line for the entry: `task 31
  pushed priory/31 to <sha>; rebase onto it`.
- When the parent's pull request merges, the entry gets the line a task's own stack gets: `#231
  merged; rebase #234 onto main`. The retarget of the entry's bottom pull request onto the merged
  one's base is expected, so it asks no person, unlike the retarget ask of
  [waits.md](../daemon/waits.md).
- The overlap hold ([runs.md](../daemon/runs.md), Overlapping files) skips an entry and its stacking
  parent, and only them. Otherwise it would hold the entry until its parent merges. The overlap hold
  sees files only after a run, so it never replaces the plan's edges.

Once stacked entries ship, the planner picks `waiting` for entries that change the same files when
the chain fits `max_prs`, since each link then saves a review cycle of waiting. Until then it picks
`merged`.

The batch's `waiting` stage, in `priory batch show` and its counts, then counts entries whose
`waiting` moment is met and whose `merged` one is not.

## Open questions

Each with a recommendation, taken for anything reversible.

1. **Add an issue to a going batch?** Start a second batch. A later slice can let `priory batch add
   --after TASK` make the new batch's roots wait on a task of the first.
2. **Cancel a going batch in one verb?** `priory cancel` per task works. Add `priory batch cancel`
   if the desk line needs a Cancel button.
3. **Several runners?** Batches stay refused in a factory of several runners until a slice adds
   claims: a local transaction cannot stop another runner from claiming a request on the same issue.
