# `serve` endpoints

The HTTP server's endpoints and the JSON they return. [serve.md](../daemon/serve.md) explains where
the server runs, its token and its limits.

Every endpoint takes `GET` and needs `Authorization: Bearer <token>`. Each listing returns the same
JSON as the CLI command with `--json`.

| Endpoint | CLI | Returns |
| -------- | --- | ------- |
| `/api/desk` | `priory desk --json` | an object: [Desk](#desk) |
| `/api/inbox` | `priory inbox --json` | an array of [inbox messages](#inbox) |
| `/api/asks` | `priory asks --json` | an array of [asks](#asks) |
| `/api/tasks` | `priory tasks --json` | an array of [tasks](#tasks) |
| `/api/tasks/{id}` | | one [task in detail](#task-in-detail) |
| `/api/queue` | `priory queue --json` | an object: [Queue](#queue) |
| `/`, `/index.html` | | the web view, with `web = true` |

Times are RFC 3339 strings in UTC. A field with no value is `null`. Agent text, such as a question
or a reason, is untrusted: render it as text.

## Status codes

| Code | When |
| ---- | ---- |
| `200` | The endpoint answered |
| `400` | The request does not parse |
| `401` | The token is missing or wrong |
| `404` | No such path, or no such task |
| `405` | A method other than `GET`, on any path |
| `408` | The request head took over 5 seconds |
| `413` | The request has a body |
| `431` | The request head is over 8 KiB or 32 headers |
| `500` | The state could not be read |

An error's body is `{"error": "<why>"}`.

## Desk

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `banner` | string or null | The stall banner while the queue is stalled |
| `counts.asleep` | integer | Tasks asleep on a wait |
| `items` | array | What waits on a person, then the queue, then worktrees that stay |

Each item:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `kind` | string | `ask`, `blocked`, `failed`, `prompt`, `held`, `sweep`, `memory`, `retired`, `queue` or `worktree`. See [desk.md](../daemon/desk.md) |
| `id` | integer or string | The ask, task, prompt, hold, sweep, grant or queue place; a worktree's path |
| `task` | integer or null | The task |
| `key` | string or null | The task's key |
| `step` | string or null | The task's step |
| `lane` | string or null | The task's lane |
| `since` | string or null | When the item started waiting |
| `what` | string | What the item is about |
| `generation` | integer or null | The task's generation |

## Inbox

Each message and report, oldest first: `seq`, `type`, `subject`, `task`, `outcome`, `time`, `line`
and `read`. A report has the type `priory.report`, its source as `subject`, the outcome `report`, its
line as `line`, and, once read, when it was read as `read`. Both are null on a message.

## Asks

Every ask, oldest first.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | integer | The ask |
| `task`, `key`, `step`, `lane` | | The task, its key, the ask's step and the task's lane |
| `run` | integer or null | The run that asked |
| `state` | string | `open`, `answered` or `withdrawn` |
| `revision` | integer | The question's revision. `priory answer` needs it |
| `question` | string | The question |
| `options` | array of strings | The options |
| `recommendation`, `tradeoff` | string or null | What the agent recommends, and the trade-off |
| `answer`, `answered_at` | string or null | The answer and when it came |
| `since` | string | When the agent asked |

`priory asks` without `--json` prints only the open asks.

## Tasks

Every task in start order: `id`, `key`, `workflow`, `step`, `state`, `band`, `lane`, `reason`,
`position` (its place in the queue from 1, or null), `generation`, `retired`, `tldr`, `since`
(when the task last changed state), `grade` (`red`, `yellow`, `green` or null) and `grade_reason`.
See Grades in [tools.md](../daemon/tools.md#grades).

## Task in detail

The fields of [Tasks](#tasks), plus `repo` and `branch`, and these arrays, oldest first:

| Field | Each entry |
| ----- | ---------- |
| `runs` | `id`, `step`, `agent`, `tmux`, `session`, `started`, `ended`, `end` |
| `steps` | A code step's action: `id`, `step`, `action`, `status`, `pull_request`, `commit_sha`, `error`, `created` |
| `asks` | An ask, as in [Asks](#asks) |
| `waits` | `id`, `run`, `kind`, `resource`, `until`, `target`, `created`, `fired`, `withdrawn` |
| `results` | `run`, `step`, `outcome`, `summary`, `evidence`, `commit_sha`, `reason`, `created` |

## Queue

| Field | Each entry |
| ----- | ---------- |
| `waiting` | A queued or ready task in start order, then each held task by when it was held: `position` and `band_position` (its place in the queue and in its band, null for a held task), `id`, `key`, `workflow`, `step`, `state`, `band`, `lane`, `reason`, `since`, and for a held task its hold: `hold` (`person`, `time`, `task`, `merge` or `blockers`), `hold_until`, `hold_target`, `hold_resource` |
| `running` | A live run: `run`, `task`, `key`, `step`, `agent`, `started` |
| `ended` | A task done, failed or dropped in the last day, latest first: `id`, `key`, `workflow`, `step`, `state`, `reason`, `ended` |
