# Linear

The intake turns Linear activity into events: issues entering or leaving a saved view or a project,
issues created, state changes, edits, comments, and blockers that change.

## What it watches

| Stream | Reads | Started by |
| ------ | ----- | ---------- |
| `linear.view:<view>` | every issue of the view | a `linear.view` trigger naming the view |
| `linear.project:<project>` | every issue of the project | a `linear.project` trigger naming the project |
| `linear.project:<project>:<filter>` | every issue of the project that matches the filter, which Linear applies | a `linear.project` trigger with that `filter` |
| `linear.issues` | issues updated since the position, archived ones included, with their last 50 history entries and the issues they block | any Linear trigger, or an open wait or a subscription on a Linear issue |
| `linear.comments` | comments created since the position | the same as `linear.issues` |

The issues and comments streams cover the whole workspace, whatever the triggers start on. So a
task gets its issue's comments and state changes even when its trigger only watches a view. An open
wait or a subscription on a Linear issue starts them too (see [watches.md](watches.md)). A task
keyed on a Linear issue subscribes to it, so a comment there reopens the task once done (see
[routing.md](../daemon/routing.md#feedback-reopens-a-done-task)).

A signed webhook makes the issue and comment events arrive sooner than the next poll. See
[webhooks.md](webhooks.md#linear).

## Trigger

Three sources start tasks from Linear. Their keys are in [triggers.md](../reference/triggers.md#linear).

| `source` | `on` | `filter` |
| -------- | ---- | -------- |
| `linear.view` | `issue.entered`, `issue.left` | none: the view filters |
| `linear.project` | `issue.entered`, `issue.left` | `assignee`, `state` |
| `linear` | `issue.created`, `issue.state_changed`, `comment.created` | `team`, `project`, `label`, `state` |

For `linear`, `project` matches the project's name or its id, and `state` matches the state an issue
moved to, or the issue's state for the other two events. For `linear.project`, `state` matches state
types, such as `backlog`. All
sources take `backfill` and an optional `repo`, the repository their tasks work in. Without a
`repo`, the tasks block before their first run.

```toml
source = "linear.view"
view = "6b3c2f9a1d4e"          # a saved view: its id, or the slug id ending its URL
on = "issue.entered"
repo = "owner/name"
workflow = "implement"
key = "issue.identifier"
```

## Events

Their fields and examples are in [events.md](../reference/events.md#linear).

| Type | One message per |
| ---- | --------------- |
| `linear.issue.entered` | issue found in a view or project that was not in it at the previous poll. It answers `keep` to an open ask that its issue's leave raised (see [runs.md](../daemon/runs.md#holds)) |
| `linear.issue.left` | issue gone from a view or project that was in it at the previous poll. The daemon tells the issue's task, or asks a person whether to drop it when it has not run yet (see [runs.md](../daemon/runs.md#holds)) |
| `linear.view.changed` | poll that finds a view's filter changed since the previous poll |
| `linear.issue.created` | issue created |
| `linear.issue.state_changed` | history entry that moves an issue from one state to another |
| `linear.issue.updated` | history entry that edits the title, description, labels, assignee or priority of an issue a task owns or subscribes to, or moves it to another team |
| `linear.comment.created` | comment on an issue |
| `linear.issue.blockers` | issue that another issue blocks, each time the blocker changes |

The daemon routes a Linear event to the active task whose key is `linear:<issue>`.

### State changes

A `linear.issue.state_changed` carries the new state's name in `to` and its type in `to_type`:
`triage`, `backlog`, `unstarted`, `started`, `completed` or `canceled`. Linear's Duplicate state is
of type `canceled`. When the issue has a `duplicate` relation, `duplicate_of` names the issue it
duplicates. A webhook delivery carries `to_type` but no `duplicate_of`, since its payload names no
relations; the poll's copy of the same change is then dropped as a repeat.

The daemon hands a change to `canceled` or `completed` to the running task that owns the issue
during its turn (see [News during a turn](../daemon/routing.md#news-during-a-turn)). A change to
any other type, such as In Review, waits for the end of the turn.

### Edits

The issues stream reads each issue's last 50 history entries. An entry that changes the title,
description, labels, assignee or priority publishes one `linear.issue.updated`, which lists the
changed fields in `changed`. The stream publishes it only for an issue that a wait or a subscription
names (see [watches.md](watches.md)), and only for entries made at or after that watch's time. A
task keyed on the issue subscribes to it, so an edit wakes the task when it sleeps, as a comment does
(see [routing.md](../daemon/routing.md#what-a-kept-message-does)). An edit is not feedback: it
reopens no done task.

Linear's history records that a description changed, but not its old or new text. So the event
carries the first 200 characters of the description as the intake reads it, which a later edit may
already have changed. A webhook delivery publishes no edit: the next poll does.

### Team moves

Moving an issue to another team changes its identifier, such as `ENG-9` to `OPS-3`. Every later
event names the new one. Linear keeps the old identifiers in `previousIdentifiers`, and records the
move as a history entry with `fromTeam` and `toTeam`.

When the issues stream reads such an entry for an issue that a wait or a subscription names by an
old identifier, it publishes one `linear.issue.updated` with `changed: ["identifier"]`, the new
identifier in `issue` and the watched one in `previous_issue`. The daemon then moves the task's key,
what it owns and subscribes to, and its open waits and holds onto the new identifier (see
[routing.md](../daemon/routing.md#resources-what-a-task-owns)). The task hears the move as an event
of its own key, and later events of the issue reach it.

### Filter changes

A view's members change when its issues change, and when a person edits its filter. Each view
marker keeps the filter Linear reported. When a poll finds a different filter, the intake
re-baselines the view:

- It publishes no `linear.issue.entered` or `linear.issue.left`, so the change requests no task.
- It publishes one `linear.view.changed`, which lists the identifiers of the issues that entered
  and left.
- The daemon posts a report on the desk as `linear`, such as `view 6b3c2f9a1d4e changed its filter:
  2 issues entered and 1 left, with no task requested` (see [desk.md](../daemon/desk.md#reports)).

An issue that entered the view by changing during the same poll gets no task either.

A filter that changes while the intake reads the view's pages fails the view's poll, which publishes
nothing for it. The next poll reads the view again.

A project's filter changes only with its trigger file, so nothing like `linear.view.changed` is
published for it. The stream's name holds the project and the filter. Triggers with the same project
and filter share a stream, whatever the order of the filter's states. Each other filter has its own
members, so:

- An edited filter starts a new stream, with its own baseline. It publishes no `linear.issue.left`
  for the old members, and requests tasks only for a `backfill` trigger. That includes an issue whose
  earlier task is done, so set `backfill = false` before you edit the filter of a running factory.
- A filter put back resumes its old members, and publishes what entered and left since.

### Milestones

Each issue of a view or project carries `order`, its place in its project: its milestone's order and
target date, and its own order. It is null for an issue without a project. A change of any of them
publishes a new marker and no event. The daemon reads the marker and reorders the queue (see
[runs.md](../daemon/runs.md#milestone-order)).

### Blockers

Linear does not update an issue when its blocker closes. So for each updated issue, a second query
reads the issues it blocks, and the issues stream publishes one `linear.issue.blockers` per blocked
issue, with that issue's fields and open blockers.

| Event | Carries `blockers` |
| ----- | ------------------ |
| `linear.issue.created`, `linear.issue.state_changed`, `linear.issue.blockers` | yes |
| `linear.issue.updated`, `linear.comment.created`, view events | no |

A leaving view event can carry the issue's fields from an earlier poll. `priory hold --blockers`
reads `blockers` (see Holds in [runs.md](../daemon/runs.md)).

## Keys and ids

A trigger's `key`, when set, is `issue.identifier`, so the task key is `linear:ENG-7`.

Each message id is a UUIDv5 of a name (see [envelope.md](envelope.md)):

| Type | Name |
| ---- | ---- |
| `linear.issue.entered`, `linear.issue.left` | `{stream}:{position}:{issue id}:{type}`, with the stream's name and its new position |
| `linear.view.changed` | `linear.view:{view}:{position}:changed`, with the view stream's new position |
| `linear.issue.created` | `linear:issue:{issue id}:created` |
| `linear.issue.state_changed` | `linear:history:{history id}`; a webhook's is `linear:webhook:state_changed:{issue id}:{state id}:{createdAt}`, and the two match by content (see [webhooks.md](webhooks.md#linear)) |
| `linear.issue.updated` | `linear:history:{history id}:updated`; a team move's is `linear:history:{history id}:moved` |
| `linear.comment.created` | `linear:comment:{comment id}` |
| `linear.issue.blockers` | `linear:blockers:{blocked issue id}:{blocker issue id}:{blocker updatedAt}` |

## Positions and baselines

Unlike a GitHub stream, each Linear stream publishes a `factory.intake.baseline` marker after every
poll that moves it. The next poll starts from the newest marker. A missing marker means the stream
is still in its baseline.

| Stream | Position | The marker also holds |
| ------ | -------- | --------------------- |
| `linear.view:<view>` | a count of membership changes | `members`, the fields of each issue in the view, and `filter`, the view's filter |
| `linear.project:<project>`, with or without a filter | a count of membership changes | `members`, as above, and `filter`, the trigger's filter |
| `linear.issues` | the latest `updatedAt` seen | `seen`, the ids of the events at that instant |
| `linear.comments` | the latest `createdAt` seen | `seen`, as above |

```json
{
  "type": "factory.intake.baseline",
  "subject": "linear.comments",
  "data": {
    "stream": "linear.comments",
    "position": "2026-11-17T11:00:00.000Z",
    "seen": ["f0352bb3-8810-574d-940d-8d6c29e73f60"]
  }
}
```

The first poll is the baseline:

- A view or project publishes each of its issues as `linear.issue.entered` with `baseline: true`.
- The issues and comments streams read their 50 most recent nodes and publish them with
  `baseline: true`. An empty first page starts the stream from 1970.
- As for GitHub, only a `backfill` trigger requests a task from a baseline event.

A poll reads the events at its stream's position again, since two events can share a millisecond.
It skips those the marker lists in `seen`, so a repeated baseline event never requests a task.

### After a crash

A crash between an event and its stream's marker leaves events after the newest marker. On start,
the intake:

1. Rebuilds the marker that poll would have published, from those events, and publishes it.
2. Republishes the task requests of the newest of them, from its `triggers`. herdr-bus drops the
   repeats.
3. Resumes polling from the rebuilt marker, so it never publishes those events again with other
   triggers.

A view or project then reports an issue that left during the crash. A crash in a stream's first poll
leaves no marker, so the next poll is a baseline again. It reports no issue that left meanwhile.

## Calls and cost

| What | Value |
| ---- | ----- |
| Key | `LINEAR_API_KEY`. The intake refuses to start without it when the factory has a Linear trigger |
| Endpoint | `https://api.linear.app/graphql`, through `curl` |
| Timeout | 60 seconds per request; the next poll retries it |
| Queries per poll | one per view, one per project and filter, two for issues and comments, one per extra page of 100, and one per 50 issues that the updated issues block |
| Rate limit | Linear allows 2,500 requests an hour per user, across the user's keys. See [Complexity](#complexity) for its other limit |

`curl` reads the key from standard input, so it stays out of process lists. `--disable` makes curl
ignore `~/.curlrc`, so no trace option can log the key. The intake also replaces the key in any
error it prints.

A view or project that Linear refuses, such as one deleted, misspelled or not shared with the key's
user, skips only its own stream. The other streams publish, and the poll's error names the skipped
stream. Its members stay as they were, so nothing leaves it. The Linear source counts as failing
until the trigger is fixed, so its desk banner stays. A failed issues or comments query still ends
the poll, and both run first: an outage or a refused key costs one failed query, not one per stream.

### Complexity

[Linear's limits](https://linear.app/developers/rate-limiting) on an API key are 10,000 points per
query and 3,000,000 points an hour per user, whichever key asks. A property costs 0.1 point and an
object 1. A connection multiplies its children by its page size.

A page of 100 issues of a view or project costs about 2,950 points, computed from that formula.
It reads 20 labels per issue to stay there. At one poll a minute, the hourly budget holds 16 such
pages per poll, shared by all the factory's views and projects and everything else the user's keys
read. An unfiltered project also reads its completed issues on each poll, so give a `linear.project`
trigger a `state` filter, such as `["triage", "backlog", "unstarted", "started"]`.

## Limits

- A view event's `time` is when the intake noticed the change, at most one poll late.
- An issue that enters and leaves a view or project between two polls produces no event.
- A crash between a `linear.view.changed` and its marker re-baselines the view again on restart.
  An issue that entered the view meanwhile gets no task.
- A marker published before the intake kept filters has none. The first poll after an upgrade
  records the filter and treats the view as unchanged.
- Start-up recovery never dispatches a baseline event. A crash between a baseline event and the
  task request of a `backfill` trigger loses that request.
- A task's key names the issue identifier. A team move renames it only once the issues stream reads
  the move's history entry. A move past the last 50 history entries of the issue between two polls
  is missed, and the task no longer hears the issue.
- A view or project marker holds every issue of the stream and is published whenever one of them changes, so a
  large, busy view grows the output.
- A view or project event carries at most 20 of its issue's labels.
- A task keeps its last milestone order after its issue leaves the stream, or moves to a project that
  no trigger watches, since another stream can hold the same issue.
- An issue that a view or project event brought in carries no `blockers`. A task held with
  `hold = { blockers = true }`, such as a `backfill` one, waits until an event that carries them.
- A filter put back resumes the members it last had. Each issue that left meanwhile publishes
  `linear.issue.left`, which can ask a person about a task, and each one that entered requests a task.
- A state change past the last 50 history entries of an issue between two polls is missed.
- `blockers` reads 20 relations of an issue, and `linear.issue.blockers` follows 20 relations of
  each updated blocker. An issue with more is unknown. A blocker with more misses some.
- A removed blocking relation, unlike a closed blocker, publishes no event about the issue it
  blocked, since the blocker no longer names it. A hold on its blockers stays until another event
  about the issue says it has none, or a person releases it.
