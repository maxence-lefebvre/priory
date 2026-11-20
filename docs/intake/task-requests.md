# Task requests

A `factory.task.requested` message asks the daemon for a task. Only this message creates a task.
Every other message only reaches a task that already exists.

Its fields and an example are in
[events.md](../reference/events.md#factorytaskrequested).

## When the intake publishes one

The intake publishes one task request per source event and matching trigger. A source event lists
the triggers it matched in its `triggers` field, and the intake publishes one request for each,
after the source event. So a consumer has already seen the source when it reads the request.

| Source | Matches as described in |
| ------ | ----------------------- |
| GitHub | the rule below; review requests as in [review-requests.md](review-requests.md#trigger) |
| Linear | [linear.md](linear.md#trigger) |
| Slack | [slack.md](slack.md#trigger) |
| Schedule | [schedule.md](schedule.md#when-it-fires) |
| `priory request` | always: it publishes one request with `trigger` set to `cli` (see [cli.md](cli.md)) |

A GitHub trigger matches an event when all of these hold:

1. Its `repo` is the event's repository.
2. Its `on` names the event's type without the `github.` prefix: `pull_request.opened` names
   `github.pull_request.opened`.
3. Its `filter`, if it has one, names the event's `label`.
4. The event is not from a baseline, unless the trigger sets `backfill = true` (see
   [github-polling.md](github-polling.md#baseline)).

GitHub triggers start from `issues.labeled`, `issues.opened`, `pull_request.labeled` or
`pull_request.opened`.

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

When someone adds `agent` to issue 15066, the intake publishes the `github.issues.labeled` event
with `"triggers": ["agent-label"]`, then this request:

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

## Keys and ids

| Source | Task key |
| ------ | -------- |
| GitHub issue or pull request | `github:owner/name#42`. GitHub numbers both from one sequence per repository |
| GitHub review request | `github:owner/name:pulls/42:review:<reviewer>` |
| Linear issue | `linear:ENG-7` |
| Slack message | `slack:<channel>:<ts>` |
| Schedule | `schedule:<key>`, or `schedule:<trigger>@<scheduled>` without a fixed key |
| `priory request` | `cli:<key>` |

The message id is the UUIDv5 of `factory.task.requested:{trigger}:{source}`, where `source` is the
source event's id.

## Fields the trigger sets

| Field | From |
| ----- | ---- |
| `workflow`, `band`, `lane` | the trigger; `band` and `lane` are null when it sets none |
| `team`, `project` | the Linear issue, for a Linear source only |
| `repo` | `priory request --repo`, or a review request's pull request; otherwise the daemon uses the trigger's `repo` |
| `hold` | the trigger's `hold`, or `priory request --hold` |
| `tldr` | `priory request --tldr`. The summarizer then skips the task (see [search.md](../daemon/search.md)) |

## `hold`

`hold` holds the new task before it can start, as `priory hold` does (see Holds in
[runs.md](../daemon/runs.md)). It has a `reason` and conditions, which release the task once all
of them are met. Its keys are in [triggers.md](../reference/triggers.md#hold).

| Condition | Met when |
| --------- | -------- |
| `until` | an RFC 3339 instant, or a duration such as `2 hours`, passes |
| `after` | another task, named by its id, is done, failed or dropped; or a Linear issue, such as `"ENG-7"`, is Done or Canceled. One value or a list |
| `merge` | a pull request, `owner/name#42` or `42` in the task's repository, merges. One value or a list. `if_closed` says what a close without a merge does: `release`, `drop` or `ask`, the default |
| `blockers = true` | the task's Linear issue has no open blocker left |

A trigger sets it as a table:

```toml
hold = { reason = "triage first", blockers = true }
hold = { reason = "after the schema", after = ["ENG-7", "ENG-9"], merge = "owner/name#42" }
```

A condition the daemon cannot use, such as `blockers` on a GitHub issue, leaves the task held for a
person, with the error in its reason.
