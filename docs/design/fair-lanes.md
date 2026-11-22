# Fair lanes

This page explains why the queue takes turns between lanes inside a band, and why a lane or a
trigger caps its running tasks. It holds the slices after the first one.

Inside a band, the queue starts the oldest task first. A batch of 40 entries in one lane then holds
every other lane's `today` work behind it. [urgent-wakes.md](urgent-wakes.md) treats starvation
between bands. This note treats starvation inside one.

Temporal's Task Queue Fairness gives each fairness key its own virtual queue and cycles through
them round-robin, with a weight per key. Inngest's `concurrency` takes a `key`, so one account's
steps do not use another account's slots. Priory's lane is the natural key: every task has one.

## The first slice

Built. [runs.md](../daemon/runs.md#turns-between-lanes) describes it:

- `[queue] fair = "lane"` orders the tasks of a band that no person ordered: the oldest of each
  lane first, then the lane whose last run started longest ago.
- `max_running` on a lane in [`lanes.toml`](../reference/lanes.md), or on a trigger, makes the
  lane's or trigger's other tasks wait once that many of its tasks have a live run.

Three choices the first slice made:

1. **No turn state.** The order derives from the state file each tick: each task's rank among the
   queued tasks of its lane, then the lane's latest run id. A restart or a second reader computes
   the same order, and `priory tasks` shows it. Run ids, not start times, break ties, since two
   runs can start in the same second.
2. **A person's order wins.** The tasks a person placed keep their place. A move numbers the whole
   band, so after one move a band is in a person's order until new tasks arrive.
3. **A cap holds every new start.** A task a person ordered, or a woken task, waits at the cap
   too. The cap is often there for an outside limit, such as an API's rate, that a person's order
   does not lift. A person raises the cap in a pull request.

## Later slices

1. A `weight` per lane in `lanes.toml`, so a lane gets several turns for each turn of another.
2. `fair = "trigger"`, for a factory whose lanes do not match its sources.
3. A desk line when a lane or trigger sits at its cap with tasks waiting, as the spend caps have.
4. Bulk verbs to move or hold a lane's queued tasks at once.
5. Inngest's priority as seconds a run jumps ahead is `[queue] age` from
   [urgent-wakes.md](urgent-wakes.md#later-slices), seen from the other side. Build it there.
