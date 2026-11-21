# Batches

A batch is a set of issues that one planning task turns into ordered tasks under one ask. A person
hands the issues over with `priory batch add`. The planning task's agent plans them, a person answers
one go, and the daemon creates one task per entry. Each entry starts once the entries it needs have
merged.

This page describes the first slice: `merged` moments only, from the CLI. Edits of a plan, split
issues, stacked entries and the web view come later.

## A batch's life

1. `priory batch add ./factory ENG-40 ENG-41 ENG-43` publishes a request for a planning task on the
   factory's [`[batch] workflow`](../reference/factory.md#batch), keyed `batch:<YYYY-MM-DD>-<n>`
   (see [cli.md](../intake/cli.md#priory-batch-add)). The daemon creates the task and its batch,
   `planning`.
2. The planning agent reads the issues and the code, then sends its plan with `result(batch)`. The
   daemon checks the plan and stores it as plan revision 1.
3. The human step after it asks the go. Its question reads `Start batch 1: 2 tasks from 3 issues,
   and close 1?`, and its text lists the entries, their order and edges, and the closes. The batch is
   `asked`.
4. `go` marks the batch `going` in the transaction that stores the answer. The `start-batch` code
   step then creates the entries' tasks. `drop` ends the planning task at `stop`, the batch
   `dropped`, and creates no task.
5. In `examples/batches/`, a `close` model step then closes the issues the plan lists as done.
6. The batch is `ended` once its planning task and every entry ended.

The go approves the batch: its tasks, their order and the closes. It approves no entry's pull
request: each entry still meets its own workflow's gates.

A factory of several runners refuses batches, at `priory batch add` and at `start-batch`:
`batches need one runner; this factory has <n>`. One runner's transaction cannot stop another
runner from claiming a request on an entry's issue.

## The plan

The planning agent sends the plan in `result`'s `batch` field. Every issue of the batch is in one
entry or one close. The entries' order is their start order.

```json
{
  "title": "Invoice exports",
  "entries": [
    { "id": "e1", "issue": "ENG-40", "title": "Add the invoices table", "brief": "...",
      "files": ["migrations/"], "after": [] },
    { "id": "e2", "issue": "ENG-41", "title": "Export invoices as CSV", "brief": "...",
      "after": [{ "entry": "e1", "moment": "merged", "why": "it reads the new table" }] }
  ],
  "close": [{ "issue": "ENG-43", "why": "Fixed by #198", "evidence": "https://github.com/acme/shop/pull/198" }]
}
```

An issue compares by its key, `linear:ENG-7` or `github:owner/name#42`, whichever way the plan
writes it. The daemon refuses the result, and the agent reads why:

| Error | Cause |
| ----- | ----- |
| `issue <issue> of the batch is in no entry and no close` | An issue left out |
| `issue <issue> is in two entries; give them parts` | One issue in two entries |
| `entry <e> waits on <other>, which the plan does not have` | A dangling edge |
| `the plan has a loop: <e>, <f>, <e>` | A cycle |
| `` moment `<m>` is not merged or waiting `` | An unknown moment |
| `` entry <e> waits on <other> at `waiting`, which Priory does not build yet; use merged `` | A stacked entry, not built yet |
| `entry <e> is part <n> of <issue>; Priory does not split issues yet` | A split issue, not built yet |
| `<issue> already has task <id> (<state>)` | An active task holds the issue. Leave it out of the plan; never close it |
| `<issue> is not one of the batch's issues` | An issue the person did not hand over |
| `issue <issue> is both an entry and a close` | The plan both works and closes an issue |
| `entry id <e> is used twice` | Duplicate entry ids |
| ``task <id> plans no batch: only `priory batch add` starts one`` | A `batch` from a task that no batch request created |

## The go and the start

The go is bound to the plan revision its ask showed. A go on a plan whose revision moved since is
refused: `the plan of batch <id> is at revision <n>, not <m>; answer its new ask`.

`start-batch` creates, in one transaction with its own action record:

1. One task per entry, in plan order, keyed by its issue. It runs the batch's entries workflow, in
   the planning task's band, lane and repository. Its request text is the entry's brief, from a
   `priory.batch.entry` source event in the inbox.
2. One `merged` hold condition per edge, on the other entry's task. An entry with no edge is
   `queued`.
3. One `builds-on` relation per edge, by `priory`, which `priory tasks --graph` draws.
4. Positions in the band after its other active tasks, in plan order, as `priory move` numbers them.

An issue that an active task took since the plan fails the whole start, and nothing is created:
`linear:ENG-41 has task 51 (running) since the plan; drop the entry or the batch`. The task goes
back to the go, whose question starts with that error, and the batch is `asked` again. A batch whose
tasks exist starts nothing again.

## The `merged` condition

An entry's `merged` condition on another task is met when:

- every pull request the other task owns has merged, or
- the other task entered a step with `after = "merged"`, or
- the other task is `done` and owns no pull request.

`done` alone never meets it: a task with one pull request is `done` once its checks pass, while its
pull request is still open (see [waits.md](waits.md)).

When the other task fails or is dropped, or one of its pull requests closes without a merge, the
condition gets its own ask on the desk, with `release` and `drop`: `Task 34 (linear:ENG-41) of
batch 4 dropped, and task 36 waits on it. Release the task, or drop it?`. `release` meets that one
condition, so the entry still waits for its other conditions. `drop` drops that entry, and the
entries that wait on it get their own asks in turn.

A person can move, hold, release or cancel an entry like any task. The batch reads its tasks' states
and keeps no copy.

## `priory batch show`

```sh
priory batch show [BATCH] [--json]
```

With no `BATCH`, it lists every batch, newest first. `BATCH` is a batch id or its planning task's
key, and shows that batch, its entries and its closes. `--json` prints:

```json
{
  "revision": 812,
  "id": 1, "key": "batch:2026-11-21-1", "task": 1, "title": "Invoice exports",
  "state": "going", "repo": "acme/shop", "band": "today", "lane": "billing/exports",
  "plan_revision": 1, "ask": 1, "went": "2026-11-21T09:12:00Z", "went_by": "cli",
  "counts": { "planned": 1, "queued": 1, "blocked": 0, "started": 0, "waiting": 0, "merged": 0, "failed": 0, "dropped": 0 },
  "entries": [
    { "entry": "e1", "position": 1, "issue": "ENG-40", "part": null, "title": "Add the invoices table",
      "task": 2, "key": "linear:ENG-40", "state": "queued", "stage": "queued", "after": [] },
    { "entry": "e2", "position": 2, "issue": "ENG-41", "part": null, "title": "Export invoices as CSV",
      "task": 3, "key": "linear:ENG-41", "state": "held", "stage": "planned",
      "after": [{ "entry": "e1", "task": 2, "moment": "merged", "met": null }] }
  ],
  "closes": [{ "issue": "ENG-43", "why": "Fixed by #198", "evidence": "https://github.com/acme/shop/pull/198", "closed": null }]
}
```

The list has `revision` and `batches`, each batch without `entries` and `closes`.

`state` is `planning`, `asked`, `going`, `ended` or `dropped`. An entry's `stage` is the batch's
word for its task: `planned` (no task yet, or held), `queued`, `blocked`, `started`, `merged`,
`failed` or `dropped`. `waiting` stays 0 until stacked entries exist. `position` is the entry's place
in the plan. `closed` is when the `close` step's result reported the issue closed, by an evidence
line that names it and says `closed`.
