# GitHub issues

The intake publishes four issue events: a label added, an issue opened, an issue closed, and a
comment on an issue. Other issue events, such as `mentioned`, are not published.

## What it watches

| Event | Read from | Stream |
| ----- | --------- | ------ |
| label added | the repository's `issues/events`, newest first | `github.issues.labeled` |
| issue opened | the repository's `issues` list, newest first by creation | `github.issues.opened` |
| issue closed | the repository's `issues/events`, newest first | `github.issues.labeled` |
| comment by a person | the repository's `issues/comments`, newest 100 | `github.issues.labeled` |

GitHub's issue events list has no `opened` event, so `opened` comes from the issues list. Polling,
baselines and API cost are in [github-polling.md](github-polling.md).

A comment on a pull request's conversation is in the same `issues/comments` list. The intake skips it
there, since the pull request's own query publishes it (see [pull-requests.md](pull-requests.md)).
It skips a bot's comment too. Comments and closes publish together, oldest first, so a comment
written after a close never arrives before it.

The close and the comments start no task: no trigger takes them. They reach the active task that
owns the issue, such as one whose key is `github:owner/name#42` (see
[routing.md](../daemon/routing.md)).

## Trigger

A `github` trigger names the repository and one event. Its keys are in
[triggers.md](../reference/triggers.md#github).

| `on` | `filter` | `key` |
| ---- | -------- | ----- |
| `issues.labeled` | `label`, optional | `issue.number` |
| `issues.opened` | none | `issue.number` |

```toml
source = "github"
repo = "owner/name"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
```

[task-requests.md](task-requests.md) gives the full matching rule.

## Events

Their fields and examples are in [events.md](../reference/events.md#github).

| Type | One message per |
| ---- | --------------- |
| `github.issues.labeled` | label added to an issue |
| `github.issues.opened` | issue opened |
| `github.issues.closed` | issue closed |
| `github.issues.comment` | comment on an issue, by a person |

Each is published once, with a link to the issue.

## Keys and ids

| What | Value |
| ---- | ----- |
| Task key | `github:owner/name#42` |
| Subject | `owner/name#42` |
| `github.issues.labeled` id | the UUIDv5 of `github:{repo}:issues/events:{github_id}` |
| `github.issues.opened` id | the UUIDv5 of `github:{repo}:issues:{github_id}:opened` |
| `github.issues.closed` id | the UUIDv5 of `github:{repo}:issues/events:{github_id}` |
| `github.issues.comment` id | the UUIDv5 of `github:{repo}:issues/comments:{github_id}` |

## Limits

- A webhook delivery of an issue's close or comment publishes nothing. The next poll publishes it.
- The comments list reads the newest 100 conversation comments per poll, issues and pull requests
  together. Over 100 between two successful polls, the older ones are missed.
- An `opened` event's `title` is the title at poll time, which may differ from the title the issue
  was opened with.
- The other limits of GitHub polling are in [github-polling.md](github-polling.md#limits).
