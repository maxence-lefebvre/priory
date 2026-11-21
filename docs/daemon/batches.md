# Batches

A batch is a set of issues that one planning task turns into ordered tasks under one ask. A person
hands the issues over with `priory batch add`. The planning task's agent plans them, a person answers
one go, and the daemon creates one task per entry. Each entry starts once the entries it needs have
merged.

This page describes `merged` moments, edits of a plan, split issues, the desk line, the HTTP API and
the web view. Stacked entries (the `waiting` moment) come later.

## A batch's life

1. `priory batch add ./factory ENG-40 ENG-41 ENG-43` publishes a request for a planning task on the
   factory's [`[batch] workflow`](../reference/factory.md#batch), keyed `batch:<YYYY-MM-DD>-<n>`
   (see [cli.md](../intake/cli.md#priory-batch-add)). The daemon creates the task and its batch,
   `planning`.
2. The planning agent reads the issues and the code, then sends its plan with `result(batch)`. The
   daemon checks the plan and stores it as plan revision 1.
3. The human step after it asks the go. Its question reads `Start batch 1: 2 tasks from 3 issues,
   and close 1?`, and its text lists the entries in order, each with its edges, files and brief,
   then the closes. The batch is
   `asked`.
4. Before the go, a person can edit the plan with `priory batch edit` (see Editing a plan). Each edit
   makes a new plan revision and asks the go again.
5. `go` marks the batch `going` in the transaction that stores the answer. The `start-batch` code
   step then creates the entries' tasks. `drop` ends the planning task at `stop`, the batch
   `dropped`, and creates no task.
6. In `examples/batches/`, a `close` model step then closes the issues the plan lists as done.
7. The batch is `ended` once its planning task and every entry ended.

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
| `issue <issue> is in two entries; give them parts` | One issue in two entries, not each with a `part` |
| `issue <issue> has part <n> twice` | Two entries with the same part of one issue |
| `entry <e> waits on <other>, which the plan does not have` | A dangling edge |
| `the plan has a loop: <e>, <f>, <e>` | A cycle |
| `entry <e> waits on <f>, which comes after it; put <f> first` | An edge to a later entry, since the entries' order is their start order |
| `` moment `<m>` is not merged or waiting `` | An unknown moment |
| `` entry <e> waits on <other> at `waiting`, which Priory does not build yet; use merged `` | A stacked entry, not built yet |
| `<issue> already has task <id> (<state>)` | An active task holds the issue. Leave it out of the plan; never close it |
| `<issue> is not one of the batch's issues` | An issue the person did not hand over |
| `issue <issue> is both an entry and a close` | The plan both works and closes an issue |
| `entry id <e> is used twice` | Duplicate entry ids |
| ``task <id> plans no batch: only `priory batch add` starts one`` | A `batch` from a task that no batch request created |
| `batch <id> went already; its plan is fixed` | A `batch` after the go |

## Split issues

An issue can be split across several entries, each with its own `part`, a number:

```json
{ "id": "e2", "issue": "ENG-41", "part": 1, "title": "Export invoices as CSV", "brief": "..." },
{ "id": "e3", "issue": "ENG-41", "part": 2, "title": "Export invoices as PDF", "brief": "...",
  "after": [{ "entry": "e2", "moment": "merged" }] }
```

The plan may say why in `splits`: `[{ "issue": "ENG-41", "why": "..." }]`. At the go, each part's
task is keyed `<issue>.<part>`, such as `linear:ENG-41.1`, and owns that key's events. An event on
the issue itself, such as a comment on `linear:ENG-41`, reaches the oldest part still active, and
only that part; once it ends, the next. A task request on the issue joins that part instead of
starting a second task, as it joins a task keyed by the issue. A task that holds the issue or a part
key since the plan fails the start, as for an unsplit issue.

## Editing a plan

```sh
priory batch edit BATCH --expected REVISION --drop ENTRY [--with-successors]
priory batch edit BATCH --expected REVISION --move ENTRY (--top | --before OTHER | --after OTHER)
priory batch edit BATCH --expected REVISION --moment ENTRY OTHER MOMENT
```

`BATCH` is a batch id or its key. `REVISION` is the plan revision the person saw, as `priory batch
show` prints it. One edit changes the plan while its go ask is open:

- `--drop` takes an entry out of the plan. An issue that no entry or close names any more leaves the
  batch. With `--with-successors`, it drops every entry that starts after it too, directly or not.
- `--move` changes the entry's place in the start order.
- `--moment` sets the moment of the edge from `ENTRY` to `OTHER`. Until stacked entries exist, the
  plan checks refuse every moment but `merged`.

The edit stores the next plan revision, and asks the go again as the ask's next revision, with the
new plan in its question and text: `batch 1 is at plan revision 2`. A go sent on the earlier
revision is refused, as `priory answer` refuses any answer to an old revision. The edited plan
passes the same checks as a planner's plan.

`priory batch edit` refuses, and changes nothing:

| Error | Cause |
| ----- | ----- |
| `the plan of batch <id> is at revision <current>, not <REVISION>` | A stale `--expected` |
| `batch <id> is <state>; only an asked batch takes edits` | Before the plan, after the go, or dropped |
| `no entry <e> in batch <id>` | An unknown entry |
| `entry <e> has successors <f> and <g>; drop them first or add --with-successors` | A drop of an entry others start after |
| `entry <e> starts after <other>; move it after <other>` | A move of `<e>` before an entry it starts after |
| `entry <f> starts after <e>; move <e> before <f>` | A move of `<e>` after an entry that starts after it |
| `entry <e> cannot move next to itself` | `--before` or `--after` the entry itself |
| `entry <e> does not wait on <other>` | A `--moment` on an edge the plan does not have |
| a plan check's error | An edit whose plan the checks above refuse |

`POST /api/batches/{id}/edit` is its twin (see [serve.md](../reference/serve.md#writes)).

## The go and the start

The go is bound to the plan revision its ask showed. A go on a plan whose revision moved since is
refused: `the plan of batch <id> is at revision <n>, not <m>; answer its new ask`.

`start-batch` creates, in one transaction with its own action record:

1. One task per entry, in plan order, keyed by its issue, or by its part key for a split issue. It runs the batch's entries workflow, in
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

## On the desk

A going batch is one `batch` row under Also waiting, in place of its entries' held rows: its title,
its counts, and which entries start next. An entry held on a condition with an open ask keeps its own
row. See [desk.md](desk.md#batches).

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
      "files": ["migrations/"],
      "task": 2, "key": "linear:ENG-40", "state": "queued", "stage": "queued", "after": [] },
    { "entry": "e2", "position": 2, "issue": "ENG-41", "part": null, "title": "Export invoices as CSV",
      "files": [], "task": 3, "key": "linear:ENG-41", "state": "held", "stage": "planned",
      "after": [{ "entry": "e1", "task": 2, "moment": "merged", "met": null }] }
  ],
  "closes": [{ "issue": "ENG-43", "why": "Fixed by #198", "evidence": "https://github.com/acme/shop/pull/198", "closed": null }],
  "splits": []
}
```

The list has `revision` and `batches`, each batch without `entries`, `closes` and `splits`. `GET /api/batches`
and `GET /api/batches/{id}` return the same JSON (see [serve.md](../reference/serve.md)).

`state` is `planning`, `asked`, `going`, `ended` or `dropped`. An entry's `stage` is the batch's
word for its task: `planned` (no task yet, or held), `queued`, `blocked`, `started`, `merged`,
`failed` or `dropped`. `waiting` stays 0 until stacked entries exist. `position` is the entry's place
in the plan. `closed` is when the `close` step's result reported the issue closed, by an evidence
line that starts with the issue and the word `closed`, such as `ENG-43: closed`. `files` is the
planner's guess at the files an entry changes, and `splits` the plan's `splits`, each an issue and
why the plan splits it.

## In the web view

The batch page, `/batches/<id>`, shows the batch's key, state, lane, when it went and by whom, and
its plan revision. Under the title, a stage bar and its words count the entries per stage, and the
issues closed so far. Then the lane graph: one row per entry in start order, each with its issue,
its task, its state tile and state, its title, when it starts in words, and its task's reason or
step. A single line between two rows is `merged`; a double line is `waiting`. The desk's `batch`
row opens the page.

A go ask opens with a Plan section, then Closes and Splits, before Why. The plan is the same lane
graph with numbered circles in place of tiles, and each entry's files. While the batch is `asked`,
each row has a menu: Move up, Move down and Drop, each sent as `POST /api/batches/{id}/edit` on the
plan revision the page drew. A Drop of an entry that others start after asks for a second tap,
naming them, and then drops them too. Each edit asks the go again, so the ask moves to its next
revision.
