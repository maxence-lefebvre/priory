# `serve` endpoints

The HTTP server's endpoints and the JSON they return. [serve.md](../daemon/serve.md) explains where
the server runs, its token and its limits.

Every endpoint but `POST /hooks/github` takes `GET`. Each one under `/api` needs
`Authorization: Bearer <token>`; the web view's files do not. Each listing returns the same JSON as
the CLI command with `--json`, with the state's `revision`, an integer, read in the same
transaction. A listing that is an object has a `revision` field. A listing that is a list comes as
`{"revision": N, "<name>": [...]}`, where `<name>` is `inbox`, `asks` or `tasks`.

| Endpoint | CLI | Returns |
| -------- | --- | ------- |
| `/api/desk` | `priory desk --json` | an object: [Desk](#desk) |
| `/api/inbox` | `priory inbox --json` | `inbox`: an array of [inbox messages](#inbox) |
| `/api/asks` | `priory asks --json` | `asks`: an array of [asks](#asks) |
| `/api/tasks` | `priory tasks --json` | `tasks`: an array of [tasks](#tasks) |
| `/api/tasks/{id}` | | one [task in detail](#task-in-detail) |
| `/api/queue` | `priory queue --json` | an object: [Queue](#queue) |
| `/api/views` | | an object: [Views](#views) |
| `/api/reports/latest` | `priory report show --json` | the [report](#report) written last |
| `/api/reports/{task}` | `priory report show TASK --json` | the task's [report](#report) |
| `/api/events?since=REV` | `priory wait --since REV` | `{"revision": N}` once it differs from `REV`. See [below](#apievents) |
| `POST /hooks/github` | | `202` for a signed GitHub delivery, with `[webhooks.github]`. See [below](#post-hooksgithub) |
| any other path | | the web view, with `web = true`: see [Web view files](#web-view-files) |

Times are RFC 3339 strings in UTC. A field with no value is `null`. Agent text, such as a question
or a reason, is untrusted: render it as text.

## Status codes

| Code | When |
| ---- | ---- |
| `200` | The endpoint answered |
| `202` | A signed webhook delivery was taken: published, or dropped on purpose |
| `400` | The request does not parse; `/api/events` lacks `since` or has a parameter out of range; for a webhook, it sends two `Content-Length`, or its payload lacks a field its event needs |
| `401` | The token is missing or wrong on a path under `/api`; for a webhook, the signature is |
| `404` | No such path, task, report or web view file |
| `405` | A method other than `GET`, on any path |
| `408` | The request took over 5 seconds |
| `411` | A webhook delivery has no `Content-Length`, or uses `Transfer-Encoding` |
| `413` | The request has a body; for a webhook, a body over 1 MiB |
| `431` | The request head is over 8 KiB or 32 headers |
| `500` | The state could not be read, or a delivery could not be published |
| `503` | 4 requests already wait on `/api/events` and this one would wait too. It comes with `Retry-After: 1` |

An error's body is `{"error": "<why>"}`.

## `/api/events`

Waits for the state to change. [serve.md](../daemon/serve.md#waiting-for-a-change) explains the
revision.

| Parameter | Meaning |
| --------- | ------- |
| `since` | Required. The revision the client last read |
| `timeout` | Seconds to hold the request without a change, from 0 to 25. 25 by default |

The reply is `200` with `{"revision": N}`: at once when `N` differs from `since`, else after the
first change, or after `timeout` with `N` equal to `since`. `priory wait --since REV [--timeout D]`
prints `N` the same way, and waits 25 seconds by default. `D` is a duration such as `90s` or `2m`.

## `POST /hooks/github`

Served only with [`[webhooks.github]`](factory.md#webhooksgithub). It takes no token: the
signature is its authentication. [webhooks.md](../intake/webhooks.md) explains what a delivery
publishes.

| Header | Meaning |
| ------ | ------- |
| `X-Hub-Signature-256` | `sha256=` and the hex HMAC-SHA256 of the raw body under the secret. Missing or wrong: `401`, and the body is never parsed |
| `X-GitHub-Event` | The event, such as `issues`. Missing: `400` |
| `X-GitHub-Delivery` | The delivery's GUID, copied into `delivery`. Missing: `400` |
| `Content-Length` | Required, at most 1 MiB. Over it: `413` before the body is read |

The reply is `202` with `{"outcome": "published"}`, or `{"outcome": "dropped"}` for a delivery
Priory does not take or already holds. A body that is not JSON, or lacks a field its event needs,
gets `400`.

## Desk

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `paused` | string or null | `paused since <time>: <reason>` while admissions are paused |
| `banner` | string or null | The stall banner while the queue is stalled |
| `counts.asleep` | integer | Tasks asleep on a wait |
| `counts.snoozed` | integer | Items snoozed now, which `items` leaves out |
| `items` | array | What waits on a person, then the queue, then worktrees that stay. `priory desk --all --json` keeps the snoozed items |

Each item:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `kind` | string | `ask`, `blocked`, `failed`, `prompt`, `held`, `sweep`, `memory`, `report`, `retired`, `queue` or `worktree`. See [desk.md](../daemon/desk.md) |
| `id` | integer or string | The ask, task, prompt, hold, sweep, grant, report's inbox sequence number or queue place; a worktree's path |
| `task` | integer or null | The task |
| `key` | string or null | The task's key |
| `step` | string or null | The task's step |
| `lane` | string or null | The task's lane |
| `since` | string or null | When the item started waiting |
| `what` | string | What the item is about |
| `generation` | integer or null | The task's generation |
| `snoozed_until`, `snoozed_at`, `back_early` | | The item's [snooze](#snooze) |

## Inbox

Each message and report, oldest first: `seq`, `type`, `subject`, `task`, `outcome`, `time`, `line`
and `read`. A report has the type `priory.report`, its source as `subject`, the outcome `report`, its
line as `line`, and, once read, when it was read as `read`. Both are null on a message.

## Asks

Every ask, oldest first.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | integer or string | The ask: its number, or the id an outside ask was given |
| `task`, `key`, `step`, `lane` | | The task, its key, the ask's step and the task's lane. An outside ask has no task, and its `key` is the one it sent |
| `run` | integer or null | The run that asked |
| `state` | string | `open`, `answered` or `withdrawn` |
| `withdrawn_why` | string or null | Why a withdrawn ask closed: `dismissed`, `a later run called another tool` or `revived as generation <n>` |
| `revision` | integer | The question's revision. `priory answer` and `priory dismiss` take it |
| `question` | string | The question |
| `options` | array of strings | The options |
| `recommendation`, `tradeoff` | string or null | What the agent recommends, and the trade-off |
| `title`, `text` | string or null | The ask's title and its Markdown body |
| `what_waits`, `next_action` | string or null | What waits on the answer, and what the agent does next |
| `command` | string or null | A command the asker attached. Priory stores it and runs nothing |
| `coordinator`, `run_id`, `session` | string or null | Who asked from outside Priory: its coordinator, its run id and its session |
| `answer`, `answered_at` | string or null | The answer and when it came |
| `since` | string | When the agent asked |
| `snoozed_until`, `snoozed_at`, `back_early` | | The ask's [snooze](#snooze) |

`priory asks` without `--json` prints only the open asks that are not snoozed, and `--all` adds the
snoozed ones.

## Snooze

An `ask`, `blocked`, `failed` or `prompt` desk item, and every ask, carry these fields. See
[desk.md](../daemon/desk.md#snooze).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `snoozed_until` | string or null | When the item comes back by itself, while it is snoozed |
| `snoozed_at` | string or null | When a person last snoozed it |
| `back_early` | object or null | `{ "at", "reason" }` once a change brought it back before its time. `reason` is `new revision`, `task state changed` or `new failure` |

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

## Views

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `factory` | string | The factory's name |
| `views` | object | Each view by name: the views Priory ships, each replaced by the factory's view of the same name, then the factory's other views. Each is the file's tree as JSON, such as `{ "layout": { "tabs": [{ "panel": "desk" }] } }`. See [views.md](views.md) |

The server reads the views when it starts.

## Web view files

With `web = true`, the server sends these without the token:

| Path | Type |
| ---- | ---- |
| `/boot.js`, `/app.js` | `text/javascript; charset=utf-8` |
| `/app.css`, `/tokens/structure.css`, `/tokens/themes.css` | `text/css; charset=utf-8` |
| `/fonts/<file>.woff2` | `font/woff2` |
| `/favicon.ico`, `/favicon.svg`, `/apple-touch-icon.png` | the image's type |
| any other path outside `/hooks`, such as `/` or `/asks/42` | the page, `text/html; charset=utf-8` |

An unknown file under `/fonts/` or `/tokens/`, or at the top level with a `.` in its name, such as
`/nothing.js`, gets `404`.

Every file carries `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src
'self'; font-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none';
frame-ancestors 'none'`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`. The
page loads nothing from another host.

## Report

The report a workflow wrote, in the shape of `docs/schemas/report.json`, with two more fields. See
[reports.md](../daemon/reports.md).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `task` | integer | The task that wrote it |
| `revision` | integer | Raised by each write and each pick; `priory report pick --expected` takes it |
| `date`, `since`, `greeting` | string | As written |
| `sections` | array | In the order written |
