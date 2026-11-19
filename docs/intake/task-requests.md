# `factory.task.requested`

One message per source event and trigger that matches it, the triggers its source lists in
`triggers`. `priory request` publishes one with `trigger` `cli` for its own source event. A trigger matches when its `repo` is the event's repository, its `on` names the event's
type (`pull_request.opened` names `github.pull_request.opened`), and its `filter`, if any, names the
event's `label`. A trigger can start from `issues.labeled`, `issues.opened`, `pull_request.labeled`
or `pull_request.opened`. A baseline event matches only a trigger
with `backfill = true`. Only this message creates a task.

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
| `repo`     | string         | Optional: the repository the task works in, `owner/name`. `priory request --repo` sets it; without it the daemon uses the trigger's `repo` |
| `hold`     | object         | Optional: files the task held. The trigger's `hold`, or `priory request --hold` |

`hold` holds the new task before it can start, as `priory hold` does (see Holds in
`docs/daemon/runs.md`). It has a `reason` and at most one condition: `until`, an RFC 3339 instant or a
duration such as `2 hours`; `after`, another task's id; `merge`, a pull request as `owner/name#42`
or `42` in the task's repository, with `if_closed` set to `release`, `drop` or `ask`, the
default; or `blockers: true`. A trigger sets it as a table, such as
`hold = { reason = "triage first", blockers = true }`. A condition the daemon cannot use, such as
`blockers` on a GitHub issue, leaves the task held for a person, with the error in its reason.

The intake publishes a task request after its source event, so a consumer has already seen the
source when it reads the request.

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
