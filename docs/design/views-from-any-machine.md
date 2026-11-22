# Views from any machine

This page explains how R8.5 could work: "Any machine of the factory can serve the views and act on
the factory's state." It builds nothing. It holds the model, the first code slice with its tests,
the slices after it, and the open questions with a recommendation for each.

## What existed when this note was written

- Each runner keeps only its own tasks, asks and runs in its own state file, `PRIORY_DB`
  ([runners.md](../daemon/runners.md)).
- `priory serve`, or the daemon's `[serve]`, reads that file read-only and sends every write to the
  local daemon's socket ([serve.md](../daemon/serve.md)). So a view on `desktop` shows and changes
  `desktop`'s tasks only.
- Two writes already cross machines through the log: `priory answer <runner>:<ask>` publishes
  `priory.ask.answered`, and `priory give` publishes a claim. The owner applies each when it reads
  it, through the same checks as a local write.
- Every runner stores every claim, hold and alive message in its `claims` and `leases` tables. So
  every runner already knows each active key's owner, its hold and when each runner was last alive.
- ADR 0003 and runners.md list the gap: "This runner's `priory asks` and desk list only its own
  asks." R8.5 closes it.

## The model

### Reads: each runner mirrors what the others publish

Each runner publishes, on the log, the parts of its state that a person acts on from anywhere. Every
runner records them in mirror tables of its state file. `serve` reads its own tables and the
mirrors in one transaction, as it reads today.

| What | Message | Mirror | Shown as |
| ---- | ------- | ------ | -------- |
| An ask, each time any part of it changes | `priory.ask.posted`: the ask as `priory asks --json` lists it, its state, its runner and a `version` | `remote_asks` | A desk and Asks row, `<runner>:<ask>` |
| The outcome of an answer sent from another machine | `priory.ask.applied`: the submission's id, and answered or refused with the reason | `remote_submissions` | "Sent to desktop", then "Answered" or the refusal |
| A task's owner and hold | the existing `priory.task.claimed` and `priory.task.held` | `claims` | A Tasks row with its runner and hold |
| A runner's last alive time | the existing `priory.runner.alive` | `leases` | "desktop, last seen 12 min ago" |

A posted ask carries what the owner's own view shows of it, including `revision`, `revisions`,
`refused` and whether it is open, answered, dismissed or withdrawn. One message type covers open,
changed and closed, so a reader never applies a close before the post it closes.

The question's `revision` cannot version the post. An answer closes the ask at the same revision,
and a refused answer is stored in its own table without a new revision. So the owner keeps a second
number per ask, `version`, raised by every change the post shows: a new revision, a close and a
refusal. The message id derives from the runner, the ask and the version, so a repeat adds nothing,
and a reader keeps the post with the highest version. The question's `revision` stays what R6.3
checks.

The owner stores each post in its `sends` table before it publishes it, as it does a claim, so a
crash publishes it at the next start.

A mirror row raises the reader's state revision, because the SQLite triggers that raise it cover
the new tables too. So `/api/events` and `priory wait` wake every view when another runner's ask
changes, with nothing new in the views.

Only asks cross in full. Tasks cross as their owner and hold only, from tables that exist already.
A task's detail, its runs and its terminal stay on its runner: the view links to that runner's own
web view for them, and R5.6's attach needs a shell on that machine anyway.

Two other ways were weighed:

| Way | For | Against |
| --- | --- | ------- |
| A serve asks the other runners' serves over the network | Always fresh; no new message | A cut-off runner's asks vanish from every view; each serve must listen beyond loopback; a page load waits on the slowest machine |
| One shared state database | One truth | Breaks R9.2: a runner must keep its tasks going with the others away. ADR 0003 kept each state file private |

The log already carries what a cut-off runner needs when it comes back, and every runner reads it.
Mirrors keep that: a runner away still shows its last asks everywhere, with how long ago it was last
seen.

### Writes: routed to the owner through the log

A write on a remote ask becomes a log message addressed to its owner, as `priory answer
<runner>:<ask>` does today. Each write keeps its own typed path at the owner:

- An answer publishes `priory.ask.answered`, which the owner applies through `answer_in_tx`, as it
  does now.
- A dismiss publishes a new `priory.ask.dismissed`, which the owner applies as the `tools::Dismiss`
  that `serve` builds for a local dismiss.

Each such message carries a submission id, its own message id. The owner publishes
`priory.ask.applied` with that id once it applied or refused it. Only that message tells the sender
what became of this answer: an ask that closes may have closed on someone else's answer, and an
ask that changed may not have read this one yet.

The reply changes. A local write answers `200` once applied. A remote write answers `202` with
`{"sent": "<runner>", "submission": "<id>"}` once published. The CLI prints the same id. Herdr
runs the CLI and never sees an HTTP status, so both hosts learn the outcome the same way: the ask's
row in `asks --json` and `/api/asks` lists its pending and applied submissions. A view keeps the
person's draft until its submission is applied, and shows "Sent to desktop" until then.

Task writes on another runner came in a later slice. `serve` exposes resume, revive, cancel,
release and say on a task, by the owner's task number, which no other runner knows: `claims` holds
the key, the runner and the request. A cancel that waits in the log could also apply after a
revive, to a task that is no longer the one the person saw. So a remote task write must name the
key and the task's generation, and the owner must refuse it when either moved.

### What "away" covers

Two cases differ:

- **The owner is away and the log is reachable.** An answer sent from another machine waits in the
  log, and the owner applies it when it reads the log again, which is R9.2's rule.
- **This machine cannot reach the log.** On herdr-bus, every runner shares one file, so this is a
  machine that lost the file. On Postgres, it is a cut-off machine. Its daemon stops, as
  runners.md says, and a remote answer cannot be published. The serve answers `503` and the view
  keeps the draft. Nothing is queued on the machine, so "Sent" always means "in the log".

A local outbox for answers given while cut off from the log would need its own ordering against
the owner's later posts. It is left out until a real partition shows the need.

### The revision guard across machines

Three checks, each at the place that can make it:

1. **At the serve, from the mirror.** An answer whose `revision` is not the mirrored ask's, or to an
   ask the mirror shows closed, gets `409` with the mirrored ask, as a local one does. This catches
   most stale answers at once, while the person still holds the draft.
2. **At the owner, when it applies.** The mirror can be behind, so the owner checks again, with
   R6.3's rule: the first answer to a revision stands, and a later one is refused and recorded.
   Only this check decides.
3. **Back to the sender.** `priory.ask.applied` names the submission and its outcome, and the next
   post shows the refusal under `refused`, so the person sees why on whatever machine they use.

### A machine that only serves

A machine with the factory and the log but no runner file, such as a laptop, has no daemon today:
with several runner files, a daemon without its own refuses to start. To serve, it needs a daemon
that reads the log into mirrors and runs nothing. That is `priory daemon --view-only`: it reads
`factory.intake` as its own consumer, claims nothing, publishes no alive message, and keeps only
mirrors. Its serve routes every answer and dismiss through the log.

## The first slice: every runner's asks on every desk

The asks are what a person must answer from anywhere, and the M3 done-when line is about answers.
So the first slice mirrors asks and routes answers, and nothing else:

1. A migration adds `remote_asks` (runner, ask, version, the ask's JSON) and `remote_submissions`
   (submission, runner, ask, revision, outcome). The revision triggers cover both. Each owner's
   asks gain a `version`.
2. On its tick, a daemon with several runners posts each ask whose version moved past the one it
   last posted, through `sends`. When it applies an answer or dismiss from the log, it publishes
   `priory.ask.applied` through `sends` in the same transaction. Schemas `priory.ask.posted.json`,
   `priory.ask.applied.json` and `priory.ask.dismissed.json` join `docs/schemas/events/`, with rows
   in [events.md](../reference/events.md). `priory.ask.answered` gains `submission`.
3. Every runner records other runners' posts in `remote_asks`, keeping the highest version, and
   their outcomes in `remote_submissions`.
4. `priory asks --json`, `priory desk --json`, `/api/asks` and `/api/desk` add each mirrored open ask
   with `"runner": "<name>"`, the id `<runner>:<ask>` and its `submissions`. Local rows get
   `"runner"` too.
5. `priory answer` and `priory dismiss` on `<runner>:<ask>`, and their two endpoints, check the
   mirror's revision, then publish, and print or reply the submission id. The web view and the Herdr
   Asks panel show the runner's name on each row, keep the draft, and show "Sent to <runner>" until
   the submission is applied.
6. runners.md and ADR 0003's consequences drop the "only its own asks" limit.

Tests, on two daemons that share one herdr-bus file, as the runner tests already do:

- Runner `a` opens an ask; runner `b`'s `/api/asks` lists `a:<ask>` with `runner: "a"`, and its
  `/api/events` returns a new revision.
- An answer posted to `b`'s serve gets `202` with a submission; `a` applies it; `b`'s listing then
  shows the ask closed and the submission answered.
- An answer with a stale revision gets `409` at `b`, and nothing is published.
- Two answers to one revision, sent from `b` and `c`: the first in log order stands, and the other's
  submission shows refused on every runner.
- `a` refuses an answer without revising the question: the version moves, and `b`'s next read shows
  the refusal.
- `a` stops; an answer sent from `b` is applied when `a` starts again, once, and its submission
  reaches `b`.
- A daemon restarted between storing a post or an outcome and publishing it publishes it once.
- Reading the whole log again from the start leaves `remote_asks` and `remote_submissions` as they
  were.
- A factory with one runner file publishes no post, as ADR 0003 keeps it.

## Later slices

1. Built: tasks of every runner in `/api/tasks`, `priory tasks` and the web Tasks panel, from
   `claims` and `leases`, each with its runner, hold and the owner's last alive time. No new
   message. See [runners.md](../daemon/runners.md#every-runners-tasks).
2. Built: resume, revive, cancel, release and say on another runner's task, each naming the key
   and the generation, refused by the owner when either moved, with `priory.task.applied` as for
   answers. The owner publishes the generation in `priory.task.held`. The web and Herdr Tasks
   panels open a remote row in place with the actions its hold allows. See
   [runners.md](../daemon/runners.md#acting-on-another-runners-task).
3. Built: `priory daemon --view-only`, for a machine with no runner file. It reads the log as
   `priory.view.<hostname>`, keeps the mirrors, serves them, and sends only answers, dismisses and
   task actions through the log. See [runners.md](../daemon/runners.md#a-machine-that-only-views).
4. A task's detail and runs across machines, if linking to the owner's web view proves too thin.

## Open questions

1. **Does the desk mix every runner's asks, or group them by runner?** Recommendation: mix them in
   the desk's usual order, with the runner's name on each row and a filter by runner. A person
   answers asks, not machines.
2. **What does a view show for a runner that is away?** Recommendation: keep its asks, mark the
   group "desktop last seen 40 min ago" from `leases`, and still accept answers, which apply when it
   comes back. Hiding them would lose answers the person could give now.
3. **Should a remote answer that runs a command (R6.8) cross machines?** The command runs on the
   owner, which the person cannot see from here. Recommendation: allow it, with `run: true` as
   today, and show the owner's name in the confirm step, "Runs on desktop".
4. **Do asks cost too much log space?** One post per ask change, and an ask changes a few times.
   Recommendation: no limit in the first slice. Measure the log's growth on Max's factory for a
   week before adding one.
5. **Should `serve` on a runner be reachable from the other machines?** Not needed: each machine
   serves its own mirror. Recommendation: keep `listen` on loopback, and keep `tailscale serve` as
   the way a phone reaches one machine.
