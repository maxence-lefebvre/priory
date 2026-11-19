# Linear

## Linear events

A trigger with `source = "linear.view"` names a saved view by its id, or by the slug id that ends its
URL, and starts on `issue.entered` or `issue.left`. A trigger with `source = "linear"` starts on
`issue.created`, `issue.state_changed` or `comment.created`, and may filter on `team` (a team key),
`label` (one label of the issue) and `state` (the state an issue moved to, or the issue's state for
the other two). Both take `backfill`. Their `key`, when set, is `issue.identifier`. Either may name
the `repo`, `owner/name`, that its tasks work in; without one, its tasks block before their first run.

| Type                         | One message per                                                  |
| ---------------------------- | ---------------------------------------------------------------- |
| `linear.issue.entered`       | issue found in a view that was not in it at the previous poll     |
| `linear.issue.left`          | issue gone from a view that was in it at the previous poll        |
| `linear.issue.created`       | issue created                                                    |
| `linear.issue.state_changed` | entry of an issue's history that moves it from one state to another |
| `linear.comment.created`     | comment on an issue                                              |
| `linear.issue.blockers`      | issue that another issue blocks, each time the blocker changes   |

The intake polls issues and comments whenever the factory has a Linear trigger, whatever the
triggers start on, so that a task receives the comments and state changes of its issue (R3). The
daemon routes a Linear event to the active task whose key is `linear:<issue>`.

Linear does not update an issue when one of its blockers closes, so the issues stream publishes
`linear.issue.blockers` for it instead. For each updated issue, it reads the issues that issue blocks
in a second query, by id, and publishes one event per blocked issue, with the blocked issue's fields
and open blockers. `linear.issue.created` and `linear.issue.state_changed` carry `blockers` too. View
and comment events do not: a view event that leaves can carry the issue's fields from an earlier poll.
`priory hold --blockers` reads them, as `docs/daemon/runs.md` describes under Holds.

| Field       | Type             | Meaning                                                         |
| ----------- | ---------------- | --------------------------------------------------------------- |
| `issue`     | string           | The issue identifier, such as `ENG-7`                           |
| `linear_id` | string           | Linear's id of what the event is about: the issue for a view event or `created`, the history entry for `state_changed`, the comment for `comment.created` |
| `title`     | string           | The issue title when the intake saw the event                   |
| `url`       | string           | The issue's page, or the comment's for `comment.created`        |
| `team`      | string           | The issue's team key                                            |
| `project`   | string or null   | The issue's project name; null when the issue has no project    |
| `state`     | string           | The issue's state when the intake saw the event                 |
| `labels`    | array of strings | The issue's labels when the intake saw the event                |
| `blockers`  | array of strings or null | The identifiers of the issues that block it and are neither completed nor canceled. Null when the issue has more than 20 relations, since one past the first page could block it. `created`, `state_changed` and `blockers` only |
| `blocker`   | string           | The issue whose change published the event. `blockers` only     |
| `view`      | string           | The view, as the trigger names it. View events only             |
| `from`, `to`| string           | The states the issue moved from and to. `state_changed` only     |
| `body`      | string           | The comment, in Markdown. `comment.created` only                |
| `actor`     | string or null   | Linear display name: who created the issue, moved it or commented. Absent on view events |
| `time`      | RFC 3339         | When the issue was created, moved or commented; for a view event, when the intake polled |
| `baseline`  | boolean          | True when the event came from the first poll of its stream (see Baseline) |
| `triggers`  | array of strings | As for GitHub events                                            |

```json
{
  "id": "f0352bb3-8810-574d-940d-8d6c29e73f60",
  "type": "linear.comment.created",
  "subject": "ENG-7",
  "time": "2026-11-17T11:00:00Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "6f1c1c9e-2b7a-4d39-9a43-2d6a3e1b8f10",
    "title": "Fix the thing",
    "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-6f1c1c9e",
    "team": "ENG",
    "project": "Desktop",
    "state": "In Progress",
    "labels": ["agent"],
    "body": "Please also cover the empty case.",
    "actor": "max",
    "time": "2026-11-17T11:00:00.000Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-6f1c1c9e" }]
}
```

## Linear streams

Linear has one stream per view that a trigger names, `linear.view:<view>`, and two streams for the
whole workspace, `linear.issues` and `linear.comments`. Unlike a GitHub stream, a Linear stream
publishes a `factory.intake.baseline` message after every poll that moves it, and starts the next
poll from the newest one. A missing marker means the stream is still in its baseline.

| Stream             | Reads                                                   | Position                     |
| ------------------ | ------------------------------------------------------- | ---------------------------- |
| `linear.view:<view>` | every issue of the view                               | a count of membership changes; the marker also holds `members`, the fields of each issue in the view |
| `linear.issues`    | issues updated at or after the position, archived ones included, with their last 50 history entries and the issues they block | the latest `updatedAt` seen; the marker also holds `seen`, the ids of the events at that instant |
| `linear.comments`  | comments created at or after the position               | the latest `createdAt` seen, and `seen` as above |

The first poll of a view publishes each of its issues as `linear.issue.entered` with `baseline:
true`. The first poll of the other two reads their 50 most recent nodes, and publishes what they
hold with `baseline: true`. As for GitHub, a baseline event requests a task only for a `backfill`
trigger. When that first page is empty, the stream starts from 1970.

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

A poll reads again the events at its stream's position, because two events can share a millisecond.
It skips those the marker lists in `seen`, so a baseline event read again never requests a task.

A crash between an event and its stream's marker leaves events after the newest marker. On start,
the intake rebuilds the marker that poll would have published from those events, and publishes it.
It also publishes again the task requests of the newest of them, from its `triggers`, as the GitHub
source does, and herdr-bus drops the repeats. Polling then goes on from the rebuilt marker, so it
never publishes those events again with other triggers, and a view reports an issue that left
during the crash.

Limits:

- A view event's `time` is when the intake noticed the change, at most one poll late.
- An issue that enters and leaves a view between two polls produces no event.
- As for GitHub, start-up recovery never dispatches a baseline event, so a crash between a baseline
  event and the task request of a `backfill` trigger loses that request.
- A task's key names the issue identifier. Moving the issue to another team changes its
  identifier, so later events of that issue no longer reach the task.
- A view marker holds every issue of the view and is published whenever one of them changes, so a
  large, busy view grows the output.
- A state change past the last 50 entries of an issue's history between two polls is missed.
- `blockers` reads 20 relations of an issue, and `linear.issue.blockers` follows 20 relations of
  each updated blocker, so an issue with more is unknown, and a blocker with more misses some.
- A blocking relation that is removed, rather than its blocker closing, publishes no event about the
  issue it blocked: the blocker no longer names it. A hold on its blockers stays until another event
  about the issue says it has none, or a person releases it.

## Linear calls

The intake reads the API key from the environment variable `LINEAR_API_KEY`, and refuses to start
without it when the factory has a Linear trigger. It posts each GraphQL query to
`https://api.linear.app/graphql` with `curl`, which reads the key from its standard input, so the
key never appears in a process list. `--disable` makes curl ignore `~/.curlrc`, so no trace option
can log the key, and the intake replaces the key in any error it prints. A request times out after
60 seconds, and the next poll retries it. A poll makes one query per view and two for
issues and comments, plus one per extra page of 100, and one for each 50 issues that the updated
issues block. Linear allows 1,500 requests an hour per key.
