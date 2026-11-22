# Review limit

A factory that opens pull requests faster than a person reviews them raises the attention each
merge costs, and its stale branches then cost rebases and tokens. The room check counts memory, disk
and CPU, not the reviewer. This note keeps the plan for holding starts on the person's review, from
the second scout report of other agent factories (idea 7). Sourcegraph Batch Changes inspired it:
its rollout windows set the rate at which changesets publish, and its bulk operations act on every
changeset that matches a filter.

## The first slice: hold new starts at the limit

Built. See [runs.md](../daemon/runs.md#review-limit).

1. `[queue] review_limit`, an integer of at least 1. Without it, nothing changes.
2. A pull request waits for a person while it is open and no agent works on its task: the task is
   not `queued`, `ready`, `running`, `failed` or `dropped`. Code decides this from the state file,
   with no model.
3. A queued task whose first run has not started waits while its batch, then its lane, has that many
   such pull requests. A woken or resumed task starts whatever the count, and so does a task of the
   first band. The waiting task holds back no other task.
4. The task's reason and one desk line per lane or batch say why, and `priory desk --json` carries
   the lines as `reviews`. No schema change: the lines come from the reasons the daemon writes.

## Later slices

1. A limit per lane in `lanes.toml`, and a weight per pull request, such as by its changed lines.
2. Bulk verbs on a batch: one action over several entries, such as a comment on each pull request
   or a re-queue of the failed entries.
3. Adopting a pull request a person opened as a batch entry, so the limit counts it.
4. A rollout window for a batch: at most N new pull requests per day, beside the review limit.
5. A preview of the difference between a batch's applied plan and an edited one, before the go.
6. The desk line on the web and Herdr Desks, from a design a person reviewed.
7. A notice when the count stays at the limit for a long time, such as a day.

## Open questions

1. **Should an approved pull request still count?** It waits for a person's merge, which is the
   person's irreversible step, so it counts. Recommendation: keep counting it, and revisit if
   factories merge through a merge queue that no person drives.
2. **Should a draft count?** The intake does not report drafts yet. Recommendation: count it, until
   an agent opens drafts on purpose.
