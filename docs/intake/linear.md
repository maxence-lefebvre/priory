# Linear

The intake turns Linear activity into events: issues entering or leaving a saved view, issues
created, state changes, edits, comments, and blockers that change.

## What it watches

| Stream | Reads | Started by |
| ------ | ----- | ---------- |
| `linear.view:<view>` | every issue of the view | a `linear.view` trigger naming the view |
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

Two sources start tasks from Linear. Their keys are in [triggers.md](../reference/triggers.md#linear).

| `source` | `on` | `filter` |
| -------- | ---- | -------- |
| `linear.view` | `issue.entered`, `issue.left` | none: the view filters |
| `linear` | `issue.created`, `issue.state_changed`, `comment.created` | `team`, `project`, `label`, `state` |

`project` matches the project's name or its id. `state` matches the state an issue moved to, or the
issue's state for the other two events. Both
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
| `linear.issue.entered` | issue found in a view that was not in it at the previous poll |
| `linear.issue.left` | issue gone from a view that was in it at the previous poll |
| `linear.view.changed` | poll that finds a view's filter changed since the previous poll |
| `linear.issue.created` | issue created |
| `linear.issue.state_changed` | history entry that moves an issue from one state to another |
| `linear.issue.updated` | history entry that edits the title, description, labels, assignee or priority of an issue a task owns or subscribes to |
| `linear.comment.created` | comment on an issue |
| `linear.issue.blockers` | issue that another issue blocks, each time the blocker changes |

The daemon routes a Linear event to the active task whose key is `linear:<issue>`.

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

A filter that changes while the intake reads the view's pages fails the poll, which publishes
nothing. The next poll reads the view again.

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
| `linear.issue.entered`, `linear.issue.left` | `linear.view:{view}:{position}:{issue id}:{type}`, with the view stream's new position |
| `linear.view.changed` | `linear.view:{view}:{position}:changed`, with the view stream's new position |
| `linear.issue.created` | `linear:issue:{issue id}:created` |
| `linear.issue.state_changed` | `linear:history:{history id}`; a webhook's is `linear:webhook:state_changed:{issue id}:{state id}:{createdAt}`, and the two match by content (see [webhooks.md](webhooks.md#linear)) |
| `linear.issue.updated` | `linear:history:{history id}:updated` |
| `linear.comment.created` | `linear:comment:{comment id}` |
| `linear.issue.blockers` | `linear:blockers:{blocked issue id}:{blocker issue id}:{blocker updatedAt}` |

## Positions and baselines

Unlike a GitHub stream, each Linear stream publishes a `factory.intake.baseline` marker after every
poll that moves it. The next poll starts from the newest marker. A missing marker means the stream
is still in its baseline.

| Stream | Position | The marker also holds |
| ------ | -------- | --------------------- |
| `linear.view:<view>` | a count of membership changes | `members`, the fields of each issue in the view, and `filter`, the view's filter |
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

- A view publishes each of its issues as `linear.issue.entered` with `baseline: true`.
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

A view then reports an issue that left during the crash.

## Calls and cost

| What | Value |
| ---- | ----- |
| Key | `LINEAR_API_KEY`. The intake refuses to start without it when the factory has a Linear trigger |
| Endpoint | `https://api.linear.app/graphql`, through `curl` |
| Timeout | 60 seconds per request; the next poll retries it |
| Queries per poll | one per view, two for issues and comments, one per extra page of 100, and one per 50 issues that the updated issues block |
| Rate limit | Linear allows 1,500 requests an hour per key |

`curl` reads the key from standard input, so it stays out of process lists. `--disable` makes curl
ignore `~/.curlrc`, so no trace option can log the key. The intake also replaces the key in any
error it prints.

## Limits

- A view event's `time` is when the intake noticed the change, at most one poll late.
- An issue that enters and leaves a view between two polls produces no event.
- A crash between a `linear.view.changed` and its marker re-baselines the view again on restart.
  An issue that entered the view meanwhile gets no task.
- A marker published before the intake kept filters has none. The first poll after an upgrade
  records the filter and treats the view as unchanged.
- Start-up recovery never dispatches a baseline event. A crash between a baseline event and the
  task request of a `backfill` trigger loses that request.
- A task's key names the issue identifier. Moving the issue to another team changes its identifier,
  so later events of that issue no longer reach the task.
- A view marker holds every issue of the view and is published whenever one of them changes, so a
  large, busy view grows the output.
- A state change past the last 50 history entries of an issue between two polls is missed.
- `blockers` reads 20 relations of an issue, and `linear.issue.blockers` follows 20 relations of
  each updated blocker. An issue with more is unknown. A blocker with more misses some.
- A removed blocking relation, unlike a closed blocker, publishes no event about the issue it
  blocked, since the blocker no longer names it. A hold on its blockers stays until another event
  about the issue says it has none, or a person releases it.
