# `factory.task.requested`

A `factory.task.requested` message asks the daemon for a task. Only this message creates a task.
Every other message only reaches a task that already exists.

## When the intake publishes one

The intake publishes one task request per source event and matching trigger. A source event lists
the triggers it matched in its `triggers` field, and the intake publishes one request for each.

A GitHub trigger matches an event when all of these hold:

- its `repo` is the event's repository;
- its `on` names the event's type, without the `github.` prefix: `pull_request.opened` names
  `github.pull_request.opened`;
- its `filter`, if it has one, names the event's `label`;
- the event is not from a baseline, unless the trigger sets `backfill = true` (see Baseline in
  [github-polling.md](github-polling.md)).

GitHub triggers can start from `issues.labeled`, `issues.opened`, `pull_request.labeled` or
`pull_request.opened`. Linear, Slack and schedule triggers match as their own pages describe.

`priory request` publishes one task request with `trigger` set to `cli`, for its own source event
(see [cli.md](cli.md)).

The intake publishes a task request after its source event. So a consumer has already seen the
source when it reads the request.

## An example

This trigger, `triggers/agent-label.toml`, starts the `implement` workflow when someone adds the
label `agent` to an issue of `cli/cli`:

```toml
source = "github"
repo = "cli/cli"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
key = "issue.number"
band = "today"
```

When someone adds `agent` to issue 15066, the intake publishes the `github.issues.labeled` event (see
[github-issues.md](github-issues.md)), with `"triggers": ["agent-label"]`. Then it publishes this
request:

```json
{
  "id": "8826a421-4031-5495-a6ed-dd2af83c6842",
  "type": "factory.task.requested",
  "subject": "github:cli/cli#15066",
  "time": "2026-11-17T15:18:55Z",
  "data": {
    "workflow": "implement",
    "key": "github:cli/cli#15066",
    "band": "today",
    "lane": null,
    "trigger": "agent-label",
    "source": "2674a560-8df6-5f05-b644-5ac1b1df8158"
  },
  "links": []
}
```

## Fields

| Field      | Type           | Meaning                                                      |
| ---------- | -------------- | ------------------------------------------------------------ |
| `workflow` | string         | The workflow the task starts                                 |
| `key`      | string         | Which events are the same work: `github:owner/name#42`, the issue or pull request number. GitHub numbers both from one sequence per repository. `linear:ENG-7` for a Linear issue. A schedule's key is under `schedule.fired` |
| `band`     | string or null | The trigger's `band`, or null when it sets none              |
| `lane`     | string or null | The trigger's `lane`, or null when `lanes.toml` derives it   |
| `team`     | string, absent or null | The Linear issue's team key. Absent when the source is not a Linear event |
| `project`  | string, absent or null | The Linear issue's project name, or null when it has none. Absent when the source is not a Linear event |
| `trigger`  | string         | The trigger's name: its file name without the extension      |
| `source`   | UUID string    | The `id` of the source event's message, on the same output   |
| `repo`     | string, absent or null | The repository the task works in, `owner/name`. `priory request --repo` sets it; without it the daemon uses the trigger's `repo` |
| `hold`     | object         | Optional: files the task held. The trigger's `hold`, or `priory request --hold` |
| `tldr`     | string         | Optional: the task's TL;DR, from `priory request --tldr`. The summarizer then skips the task (see [search.md](../daemon/search.md)) |

## `hold`

`hold` holds the new task before it can start, as `priory hold` does (see Holds in
[runs.md](../daemon/runs.md)). It has a `reason` and at most one condition:

| Condition | Releases the task when |
| --------- | ---------------------- |
| `until` | an RFC 3339 instant, or a duration such as `2 hours`, passes |
| `after` | another task, named by its id, is done, failed or dropped |
| `merge` | a pull request, as `owner/name#42` or `42` in the task's repository, merges. `if_closed` says what a close without a merge does: `release`, `drop` or `ask`, the default |
| `blockers: true` | the task's Linear issue has no open blocker left |

A trigger sets it as a table, such as:

```toml
hold = { reason = "triage first", blockers = true }
```

A condition the daemon cannot use, such as `blockers` on a GitHub issue, leaves the task held for a
person, with the error in its reason.
