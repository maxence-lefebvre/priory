# GitHub issues

The intake publishes two issue events: a label added, and an issue opened. Other issue events, such
as `closed` or `mentioned`, are not published.

## What it watches

| Event | Read from | Stream |
| ----- | --------- | ------ |
| label added | the repository's `issues/events`, newest first | `github.issues.labeled` |
| issue opened | the repository's `issues` list, newest first by creation | `github.issues.opened` |

GitHub's issue events list has no `opened` event, so `opened` comes from the issues list. Polling,
baselines and API cost are in [github-polling.md](github-polling.md).

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

Each is published once, with a link to the issue.

## Keys and ids

| What | Value |
| ---- | ----- |
| Task key | `github:owner/name#42` |
| Subject | `owner/name#42` |
| `github.issues.labeled` id | the UUIDv5 of `github:{repo}:issues/events:{github_id}` |
| `github.issues.opened` id | the UUIDv5 of `github:{repo}:issues:{github_id}:opened` |

## Limits

- An `opened` event's `title` is the title at poll time, which may differ from the title the issue
  was opened with.
- The other limits of GitHub polling are in [github-polling.md](github-polling.md#limits).
