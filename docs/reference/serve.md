# `serve` endpoints

The HTTP server's endpoints and the JSON they return. [serve.md](../daemon/serve.md) explains where
the server runs, its token and its limits.

Every endpoint takes `GET`, except the [writes](#writes), `POST /hooks/github` and `POST /hooks/linear`, which take
`POST`. Each one under `/api` needs `Authorization: Bearer <token>`; the web view's files do not.
Each listing returns the same JSON as
the CLI command with `--json`, with the state's `revision`, an integer, read in the same
transaction. A listing that is an object has a `revision` field. A listing that is a list comes as
`{"revision": N, "<name>": [...]}`, where `<name>` is `inbox` or `tasks`. `/api/runs` comes as
`{"revision": N, "runs": [...], "older": N}`.
`/api/costs`, `/api/attention` and `/api/grades` are reports rather than listings: they have no `revision`.

`/api/asks`, `/api/tasks` and `/api/runs` leave out the rows that closed over a week ago, and add
`older`, the number they left out: see [Asks](#asks), [Tasks](#tasks) and [Runs](#runs). `?all=1` returns every row, as the CLI
does, with every count in `older` at 0. Their rows match the CLI's JSON only with `?all=1`.

| Endpoint | CLI | Returns |
| -------- | --- | ------- |
| `/api/desk?person=NAME` | `priory desk --json --person NAME` | an object: [Desk](#desk) |
| `/api/inbox` | `priory inbox --json` | `inbox`: an array of [inbox messages](#inbox) |
| `/api/inbox?unrouted=1` | `priory inbox --unrouted --json` | `unrouted`: the [unrouted groups](#unrouted), and `outsider`: the comments from outside the repository |
| `/api/asks?band=BAND&risk=RISK&all=1` | `priory asks --json --band BAND --risk RISK` | an object: `asks`, an array of [asks](#asks), `bands` and `older` |
| `/api/tasks?all=1` | `priory tasks --json` | `tasks`: an array of [tasks](#tasks), and `older` |
| `/api/tasks?pr=RESOURCE` | | `tasks`: the active tasks that hold a pull request. See [Tasks that hold a pull request](#tasks-that-hold-a-pull-request) |
| `/api/tasks/{id}` | `priory task ID --json` | one [task in detail](#task-in-detail) |
| `/api/runs?all=1` | `priory runs --json` | `runs`: an array of [runs](#runs), newest first, and `older` |
| `/api/queue` | `priory queue --json` | an object: [Queue](#queue) |
| `/api/batches` | `priory batch show --json` | `batches`: every batch, newest first, without its entries and closes. See [batches.md](../daemon/batches.md#priory-batch-show) |
| `/api/batches/{id}` | `priory batch show ID --json` | one batch with its entries, closes and splits. `ID` is the batch's id or its key |
| `/api/views` | `priory views --json` | an object: [Views](#views) |
| `/api/request` | | what the web view's Add prefills: `workflow` and `band`, the [`[request]`](factory.md#request) defaults, each `null` when unset; `workflows`, the factory's workflow names; and `bands`, its bands in order |
| `/api/links` | `priory links --json` | an object: [Links](#links) |
| `/api/flows` | `priory flows --json` | an object: [Flows](#flows) |
| `/api/reports` | `priory report list --json` | `reports`: each task's report as `task`, `date`, `greeting` and `put`, newest first |
| `/api/reports/latest` | `priory report show --json` | the [report](#report) written last |
| `/api/reports/{task}` | `priory report show TASK --json` | the task's [report](#report) |
| `/api/costs?since=WINDOW&by=SPLIT` | `priory costs --json --since WINDOW --by SPLIT` | an object: [Costs](#costs) |
| `/api/grades?since=WINDOW` | `priory grades --json --since WINDOW` | an object: [Grades](#grades) |
| `/api/attention?since=WINDOW&by=SPLIT` | `priory report attention --json --since WINDOW --by SPLIT` | an object: [Attention](#attention) |
| `/api/runs/{id}/screen?have=HASH&history=N` | `priory screen --run ID --json` | the run's screen in colour, after up to 2,000 rows of scrollback, with secrets hidden. See [Run screen](#run-screen) |
| `/api/events?since=REV` | `priory wait --since REV` | `{"revision": N}` once it differs from `REV`. See [below](#apievents) |
| `/api/push/key` | | `{"key": "<base64url>"}`, the daemon's VAPID public key. See [Notifications](#notifications) |
| `POST /api/asks/{id}/answer` | `priory answer ID TEXT --revision N` | `{"ok": "<what the CLI prints>"}`. See [Writes](#writes) |
| `POST /api/asks/{id}/dismiss` | `priory dismiss --expected N ID` | as above |
| `POST /api/snooze` | `priory snooze KIND ID (--for D \| --until T \| --clear)` | as above |
| `POST /api/request` | `priory add TEXT` | `{"key": "<task key>", "id": <task id>}`. See [Writes](#writes) |
| `POST /api/tasks/{id}/resume` | `priory resume ID` | `{"ok": "<what the CLI prints>"}`. See [Writes](#writes) |
| `POST /api/tasks/{id}/revive` | `priory revive ID [--step STEP]` | as above |
| `POST /api/tasks/{id}/cancel` | `priory cancel ID [--expected N]` | as above |
| `POST /api/tasks/{id}/release` | `priory release ID [--expected N]` | as above |
| `POST /api/inbox/{seq}/read` | `priory inbox read SEQ` | as above |
| `POST /api/tasks/{id}/say` | `priory say ID TEXT` | as above |
| `POST /api/tasks/{id}/move` | `priory move ID --expected N [--band BAND] [--top \| --before TASK \| --after TASK]` | as above |
| `POST /api/batches/{id}/edit` | `priory batch edit ID --expected N ...` | as above |
| `POST /api/reports/pick` | `priory report pick PICK OPTION --expected N` | `{"ok": "<what the CLI prints>"}` |
| `POST /api/reports/comment` | `priory report comment ROW --expected N TEXT` | as above |
| `POST /api/reports/reply` | `priory report reply COMMENT TEXT` | as above |
| `POST /api/reports/resolve` | `priory report resolve COMMENT` | as above |
| `POST /api/reports/reopen` | `priory report reopen COMMENT` | as above |
| `POST /hooks/github` | | `202` for a signed GitHub delivery, with `[webhooks.github]`. See [below](#post-hooksgithub) |
| `POST /hooks/linear` | | `200` for a signed Linear delivery, with `[webhooks.linear]`. See [below](#post-hookslinear) |
| any other path | | the web view, with `web = true`: see [Web view files](#web-view-files) |

Times are RFC 3339 strings in UTC. A field with no value is `null`. Agent text, such as a question
or a reason, is untrusted: render it as text.

## Status codes

| Code | When |
| ---- | ---- |
| `200` | The endpoint answered |
| `202` | A signed webhook delivery was taken: published, or dropped on purpose |
| `400` | The request does not parse, or its target does not start with `/`; `/api/events` lacks `since` or has a parameter out of range; `/api/desk` has a parameter other than `person`, or a broken escape; `/api/asks` has a parameter other than `band`, `risk` and `all=1`, a band the factory does not have, or a risk other than the eight [below](#asks); `/api/tasks` has a parameter other than `all=1`, or a `pr` that is not a pull request such as `github:owner/repo#7`; `/api/runs` has a parameter other than `all=1`; a run screen's `history` is not a whole number; `/api/costs` has a parameter other than `since` and `by`, a broken escape, a `since` that cannot form a window (see [trace.md](../daemon/trace.md#costs)), or a `by` other than the five [below](#costs); `/api/grades` has a parameter other than `since`, a broken escape, or a `since` that does not parse; `/api/attention` has the same faults, with a `by` other than the three [below](#attention); a write's body is not the shape [below](#writes); for a webhook, it sends two `Content-Length`, or its payload lacks a field its event needs |
| `401` | The token is missing or wrong on a path under `/api`; for a webhook, the signature is |
| `403` | A `POST` carries an `Origin` whose host and port differ from its `Host`, such as a page on another site, and that is not one of [`[serve] extension_origins`](factory.md#serve) |
| `404` | No such path, task, report, ask, batch or web view file; a run screen whose run ended, whose pane this server cannot find or read within 2 seconds, or that holds a secret redaction cannot hide; any run screen with `[serve] terminal = "off"` |
| `405` | `POST` on a path that reads, or `GET` on a write. The `Allow` header names the method |
| `408` | The request sent no byte within 1 second, or took over 5 seconds in all |
| `409` | A write names an ask whose revision moved or that closed. The body adds the ask as `/api/asks` lists it. Or `/api/request` repeats a text already sent for the same linked issue. Or a cancel's or a move's `revision` is no longer the state's. Or a pick's report moved past its `revision`, or the pick already holds that choice or an applied one. Or a batch edit's `revision` is no longer the plan's revision |
| `411` | A `POST` has no `Content-Length`, or uses `Transfer-Encoding` |
| `413` | A `GET` has a body; a write's body is over 16 KiB; a webhook's is over 1 MiB |
| `415` | A write's `Content-Type` is not `application/json` |
| `422` | The daemon refused a write for another reason, such as an option a step does not take, or a request's unknown workflow or bad `repo`, or a push subscription it does not take. A request the bus could not take gets `422` too, with the bus's error |
| `431` | The request head is over 8 KiB or 32 headers |
| `500` | The state could not be read, a delivery could not be published, or Priory failed while answering. The server keeps serving |
| `503` | 4 requests already wait on `/api/events` and this one would wait too; or a write found no daemon on its socket, as under `priory serve` alone; or `/api/push/key` before a daemon made the key pair; or 2 run screens are already being read. It comes with `Retry-After: 1` |
| `504` | A write reached the daemon, which did not reply within 10 seconds. It may have applied it |

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

## Run screen

`/api/runs/{id}/screen` returns what the run's agent pane shows now, read with tmux's
`capture-pane -e`. [serve.md](../daemon/serve.md#a-runs-screen) explains what it hides and why. The
reply carries `Cache-Control: no-store`.

| Parameter | Meaning |
| --------- | ------- |
| `have` | The `hash` the client holds. When it equals the current one, the reply is `{"hash": "<HASH>", "same": true, "scrollback": N}` |
| `history` | Rows of scrollback to send above the screen, 0 by default. The server sends at most 2,000, however many are asked. A value that is not a whole number gets `400` |

| Field | Meaning |
| ----- | ------- |
| `run`, `task` | The run and its task |
| `width`, `height` | The pane's size, in columns and rows |
| `cursor` | `{"x", "y"}`, on the screen from the top left at 0, or `null` when the cursor is hidden |
| `lines` | `earlier` rows of scrollback, then the screen's `height` rows, blank rows kept and trailing spaces trimmed, with each secret replaced by `[redacted]`. Each row keeps tmux's SGR colour codes (`ESC [ … m`); the style carries from one row to the next. tmux's hyperlink codes are dropped |
| `earlier` | How many rows of scrollback `lines` starts with: `history`, or fewer when tmux keeps fewer |
| `cut` | How many top rows the server left out to keep `lines` under 256 KiB of JSON, 0 on a usual screen |
| `redacted` | How many secrets the server replaced |
| `hash` | 16 hex digits, a hash of the fields above after redaction. A hidden value never changes it |
| `scrollback` | How many rows of scrollback tmux keeps for the pane. The hash leaves it out |

`priory screen (TASK | --run RUN) [--json]` prints the same screen, with no scrollback, as rows of
text or, with `--json`, as this object. Its rows keep their colours on a terminal and drop them
when the output goes to a file or a pipe. It hides the values of the default secret variables,
such as `PRIORY_TOKEN`, from its own environment, and the secret shapes. Like the endpoint, it
gives tmux 2 seconds.

## Writes

Each write sends the daemon the same line as its CLI verb, and the daemon applies it.
[serve.md](../daemon/serve.md#writes) explains why. Each needs the token, `Content-Type:
application/json` and a JSON body of at most 16 KiB with exactly the fields below.

| Endpoint | Body |
| -------- | ---- |
| `POST /api/asks/{id}/answer` | `revision`, an integer, and one of `option` or `text`, a string. Both become the answer: a step's ask takes only one of its options, or `OPTION: NOTE` (see [waits.md](../daemon/waits.md#asks-and-answers)). `run`, a boolean, default `false`: `true` confirms an answer that runs the ask's command, as `priory answer --run` does. Without it, that answer gets `422` |
| `POST /api/asks/{id}/dismiss` | `revision`, an integer |
| `POST /api/snooze` | `kind` (`ask`, `blocked`, `failed` or `prompt`) and `id`, an integer, then one of: `for`, a duration such as `1h`; `until`, an RFC 3339 time; or `"clear": true` |
| `POST /api/request` | `text`, a string, then optionally `workflow`, `band`, `lane` and `repo`, strings, and `intervention`, `plan` or `hands-off`. What it leaves unset comes from `[request]` and the URLs in `text`, as for [`priory add`](../intake/cli.md#priory-add) |
| `POST /api/tasks/{id}/resume` | `{}` |
| `POST /api/tasks/{id}/revive` | optionally `step`, a string: the step to restart at, as `--step` |
| `POST /api/tasks/{id}/cancel` | optionally `revision`, an integer: the state's revision the client showed, as `--expected`. A newer state gets `409` |
| `POST /api/tasks/{id}/release` | optionally `revision`, an integer, as `--expected`. A newer state gets `409` |
| `POST /api/tasks/{id}/say` | `text`, a string: what the task's agent hears |
| `POST /api/tasks/{id}/move` | `revision`, an integer: the state's revision the client showed, as `--expected`. Then `band`, one of the factory's bands, a place, or both. The place is one of `"top": true`, `before` or `after`, a task's id, as `--top`, `--before` and `--after`. A body with two places, or with neither a band nor a place, gets `400`, a newer state `409`, and a band the factory lacks or a task that is not active `422` |
| `POST /api/subscribe` | `task`, an integer, and `resource`, a string such as `github:owner/name#7`, `linear:ENG-7` or `slack:C0123:1763500000.000100`, as `priory subscribe TASK RESOURCE`. Optionally `revision`, an integer, as `--expected`: a newer state gets `409`. A resource it cannot name gets `400`, a missing task `404`, and a resource another active task owns `422`, naming that task |
| `POST /api/batches/{id}/edit` | `revision`, the plan revision the client showed, then one edit: `drop`, an entry id, with optionally `"with_successors": true`; or `move`, an entry id, with one of `"top": true`, `before` or `after`, an entry id; or `moment`, an object of `entry`, `other` and `moment`, which sets the moment of the edge from `entry` to `other`. A body with no edit, two edits, or a field its edit does not take gets `400`, a plan whose revision moved `409`, and an edit the plan refuses `422`. See [batches.md](../daemon/batches.md#editing-a-plan) |
| `POST /api/inbox/{seq}/read` | `{}`: marks the report read, so its row leaves the desk. A sequence number that is not a report gets `422` |
| `POST /api/reports/pick` | `pick`, the pick's id, `option`, one of its options, and `revision`, the report's revision the client showed |
| `POST /api/reports/comment` | `row`, the row's id, `text`, a string, and `revision`, the report's revision the client showed. A comment verb takes `text` only with `comment` and `reply`, and `revision` only with `comment` |
| `POST /api/reports/reply` | `comment`, an integer, and `text`, a string |
| `POST /api/reports/resolve`, `POST /api/reports/reopen` | `comment`, an integer |
| `POST /api/push/subscribe` | the browser's `PushSubscription.toJSON()`: `endpoint`, a string, `keys`, an object of `p256dh` and `auth`, base64url strings, and optionally `expirationTime`, which is ignored. See [Notifications](#notifications) |
| `POST /api/push/unsubscribe` | `endpoint`, a string |
| `POST /api/todos` | `text`, one line, then optionally `band`, one of the factory's bands, and `due`, an RFC 3339 time, as `priory todo add`. The reply's `ok` is the new to-do's id |
| `POST /api/todos/{id}/done` | `{}`, as `priory todo done`. A to-do already done gets `422` |
| `POST /api/seen` | `at`, an RFC 3339 time: the visit to the desk to record for `me`, as `priory seen --at` records it, to the second |

In `/api/asks/{id}`, `{id}` is the ask's `id` as `/api/asks` lists it: its number, an outside
ask's name, or `<runner>:<ask>` for another runner's ask, and `revision` is the ask's `revision` the
client showed.

A write to another runner's ask is not applied here. The server checks it against that runner's last
post: an ask the post shows closed or at another revision gets `409` with the ask, and an answer that
runs the ask's command without `run: true` gets `422`, whose error says `Runs on <runner>`. Otherwise
it publishes the write on the log for that runner and replies `202`. The runner applies it when it
reads the log, even after time away, and the ask's `submissions` then show the outcome. See
[runners.md](../daemon/runners.md#every-runners-asks). In `/api/tasks/{id}`, `{id}` is
the task's number.

```sh
curl -X POST http://127.0.0.1:7340/api/asks/42/answer -H "Authorization: Bearer $PRIORY_TOKEN" \
  -H 'Content-Type: application/json' -d '{"option": "merge", "revision": 3}'
```

| Reply | When |
| ----- | ---- |
| `200` `{"ok": "ask 42 answered"}` | The daemon applied it |
| `202` `{"sent": "xps13", "submission": "<id>"}` | For another runner's ask: the write is on the log for that runner, which applies it when it reads it. `submission` names it in the ask's `submissions` |
| `200` `{"key": "cli:2026-11-20-1", "id": 7}` | For `/api/request`: the daemon published the request and task `id` holds `key`. `id` is `null` when no task took the key |
| `409` `{"error", "ask"}` | The ask's revision is no longer the one sent, or it was answered, dismissed or withdrawn. `ask` is its current state |
| `409` `{"error"}` | A cancel's or a move's `revision` is no longer the state's. Or a pick's report is at another revision than the one sent, or the pick already holds that choice or an applied one: read the report again |
| `409` `{"error", "key", "id"}` | For `/api/request`: the same text already went to `key`, so nothing was added. `id` is the active task that holds `key`, or `null` |
| `404`, `422` `{"error"}` | The ask, task or pick is gone, or the daemon refused for another reason, such as a resume of a task that is not blocked, a say to a task with no run yet, or a comment on a stale report revision. `error` is the CLI's sentence |
| `503` `{"error"}` | No daemon listens on the socket, or the log cannot be reached for another runner's ask, so nothing was sent. Reads keep working |
| `504` `{"error"}` | The daemon took the line but did not reply within 10 seconds. It may have applied it: read the state before you retry |

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

## `POST /hooks/linear`

Served only with [`[webhooks.linear]`](factory.md#webhookslinear). It takes no token: the
signature is its authentication. [webhooks.md](../intake/webhooks.md#linear) explains what a
delivery publishes.

| Header | Meaning |
| ------ | ------- |
| `Linear-Signature` | The hex HMAC-SHA256 of the raw body under the signing secret, with no prefix. Missing or wrong: `401`, and the body is never parsed |
| `Linear-Delivery` | The delivery's UUID, copied into `delivery`. Optional |
| `Content-Length` | Required, at most 1 MiB. Over it: `413` before the body is read |

The body's `webhookTimestamp`, in milliseconds, must be within 60 seconds of now, as Linear
documents: else `401`. Linear retries any reply but `200`, so the reply is `200` with
`{"outcome": "published"}`, or `{"outcome": "dropped"}` for a delivery Priory does not take or
already holds. A body that is not JSON, lacks `webhookTimestamp`, or lacks a field its event needs
gets `400`.

## Desk

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `paused` | string or null | `paused since <time>: <reason>` while admissions are paused |
| `banner` | string or null | The stall banner while the queue is stalled |
| `intake` | string or null | The intake banner: `the intake has not polled since <time>: ...` when no source polled in the last 5 minutes, else `the intake fails to poll <source> since <time>: <error>` for each failing source, joined by `; `. Null while every source polls. See [desk.md](../daemon/desk.md#intake-banner) |
| `counts.asleep` | integer | Tasks asleep on a wait |
| `counts.snoozed` | integer | Items snoozed now, which `items` leaves out |
| `counts.unrouted` | integer | [Unrouted](#unrouted) events from the 24 hours before now (`PRIORY_NOW` when set). See [desk.md](../daemon/desk.md#unrouted-events) |
| `counts.outsider` | integer | The comments and reviews from outside the repository in the 24 hours before now (`PRIORY_NOW` when set). See [desk.md](../daemon/desk.md#unrouted-events) |
| `items` | array | What waits on a person, then the queue, then worktrees that stay. `priory desk --all --json` keeps the snoozed items |
| `next` | object or null | The item that ranks first, as in `ranked`; null when nothing waits on a person |
| `ranked` | array | What waits on a person, best first, without the snoozed items: each a desk item with `why`, why it ranks there, and `title`, the line a notification carries. See [desk.md](../daemon/desk.md#the-next-thing) |
| `last_visit` | string or null | The person's last visit, which `priory seen` and `POST /api/seen` record |
| `since_you_looked` | array | The events from the second of `last_visit` on, newest first, at most 50: each with `time`, `task`, `key` and `what`. See [desk.md](../daemon/desk.md#since-you-looked) |

`/api/desk?person=NAME` reads the visit of `NAME`, percent-encoded, as `priory desk --json --person
NAME` does. Without it, the person is `me`. Any other parameter, or a broken escape, gets `400`.

Each item:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `kind` | string | `ask`, `delivery`, `blocked`, `failed`, `prompt`, `command`, `held`, `batch`, `sweep`, `memory`, `report`, `retired`, `queue` or `worktree`. See [desk.md](../daemon/desk.md) |
| `id` | integer or string | The ask, task, prompt, answer's action, hold, batch, sweep, grant, report's inbox sequence number or queue place; a worktree's path |
| `task` | integer or null | The task; for a `batch`, its planning task |
| `key` | string or null | The task's key |
| `step` | string or null | The task's step |
| `lane` | string or null | The task's lane |
| `since` | string or null | When the item started waiting |
| `what` | string | What the item is about |
| `routed_to` | string or null | An `ask` item's lead, which answers it in the person's place, or null. Other kinds have no `routed_to` |
| `generation` | integer or null | The task's generation |
| `snoozed_until`, `snoozed_at`, `back_early` | | The item's [snooze](#snooze) |
| `runner`, `last_seen` | | An `ask` item's runner, in a factory of several, and for another runner's ask when that runner was last alive. Another runner's ask has `<runner>:<ask>` as `id`, a null `task` and no snooze, and ranks with the local asks |

## Inbox

Each message and report, oldest first: `seq`, `type`, `subject`, `task`, `outcome`, `time`, `line`
and `read`. A report has the type `priory.report`, its source as `subject`, the outcome `report`, its
line as `line`, and, once read, when it was read as `read`. Both are null on a message.

## Unrouted

The events that reached no task and changed nothing (see
[routing.md](../daemon/routing.md#events-that-reach-no-task)), as an object `unrouted` with two
arrays. `priory inbox --unrouted --since SEQ` keeps the events after sequence number `SEQ`.

`owned` groups the events whose first resource a task owned or subscribed to, in any state, one
group per resource, the group with the newest event first:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `resource` | string | The resource, such as `github:owner/name#7` |
| `task` | object | The newest task that owned it: `id`, `key`, `state`, `workflow` |
| `events` | array | Its events, newest first: `seq`, `type`, `subject`, `time`, and `line`, the first line of its body, text or title, or null |
| `follow_up` | string or null | The command that routes its next event, each argument shell-quoted: `priory revive ID` for a failed or dropped task, `priory add --workflow WORKFLOW 'TEXT'` with the task's workflow and `request_text` for a done one, else `priory subscribe ID RESOURCE`. Null for a done task's resource that `request_text` cannot name, such as a Slack thread |
| `request_text` | string or null | For a GitHub or Linear resource, the text of a new task keyed by it: `Follow up on URL after event SEQ`, the issue's URL, which `priory add` turns into the resource as key, and the newest event's sequence number, so each follow-up is a new request |

`outsider`, beside `unrouted`, lists the comments and reviews from someone outside the repository,
newest first: `seq`, `type`, `subject`, `time` and `actor`. No task takes them, so they come with
no command.

`other` counts the rest, the events no task ever owned, per source (the resource's prefix, such as
`github` or `slack`), most first: `source` and `events`.

Without `--json`, `priory inbox --unrouted` prints each owned group as its resource, event count
and task, one line per event, then its follow-up, and last one line such as `No task ever owned: 2
github events, 1 linear event.` Then it prints the outsider events as a group of their own,
`from outside the repository: N events`, one line per event with its author.

## Asks

Every ask, oldest first, in `asks`, and the factory's bands, highest first, in `bands`. With
`band`, only the asks of tasks in that band. `band` takes a comma-separated list and repeats, so
`?band=now,today` and `?band=now&band=today` are the same. A band the factory does not have gets
`400`, with the factory's bands in the error. With `risk`, only the asks that carry that risk:
`irreversible`, `high`, `medium`, `low`, `red`, `yellow` or `green`, or `none` for an ask with no risk. `risk` takes a list
and repeats the same way, and an ask matches when it carries any of the risks asked. Another value
gets `400`. Both filters together keep the asks that match each. `priory asks --band BAND` and
`priory asks --risk RISK` filter the same way and refuse the same values.

Without `all=1`, `asks` holds the open asks and the asks answered or withdrawn in the last 7 days.
An ask withdrawn before Priory kept `withdrawn_at`, with no withdrawal on record, counts from when it
was asked. `older` is
an object: `answered` and `withdrawn`, the number of each left out, within `band` and `risk` when they are given.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | integer or string | The ask: its number, or the id an outside ask was given |
| `task`, `key`, `step`, `lane`, `band`, `repo` | | The task, its key, the ask's step, and the task's lane, band and repository. An outside ask has no task, so its `lane`, `band` and `repo` are null, and its `key` is the one it sent |
| `risk` | array of strings | What answering risks, empty for none: `irreversible` when the answer lets an irreversible step run before a person decides again, the rule `priory check --explain` uses, or when it is an agent's or a step's ask whose answer runs a [command](../daemon/tools.md#commands-an-answer-runs); then the task's [merge risk](../daemon/tools.md#merge-risk) verdict, `low`, `medium` or `high`; then the task's grooming grade, `red`, `yellow` or `green`. An outside ask has none. The daemon records its factory's irreversible approvals when it starts |
| `merge_risk` | object or null | The task's latest [merge risk](../daemon/tools.md#merge-risk): `verdict`, then the lines `ask`, `blast_radius`, `verification`, `rollback` and `detection`. Null before a risk step ends |
| `run` | integer or null | The run that asked |
| `state` | string | `open`, `answered` or `withdrawn` |
| `withdrawn_reason` | string or null | Why a withdrawn ask closed, one of the [withdrawal reasons](../daemon/waits.md#why-an-ask-or-wait-was-withdrawn): `dismissed`, `sibling-fired`, `moved-on`, `revived` or `predecessor-revived` |
| `withdrawn_at` | string or null | When the ask was withdrawn, in UTC. Null for an ask withdrawn before Priory kept the time, with no withdrawal on record |
| `withdrawn_why` | string or null | `withdrawn_reason` in words: `dismissed`, `a wait of its run fired first`, `a later run called another tool`, `its task was revived` or `the task it waits on was revived` |
| `revision` | integer | The question's revision. `priory answer` and `priory dismiss` take it |
| `question` | string | The question |
| `options` | array of strings | The options |
| `free_text` | boolean | Whether the ask takes free text and a dismissal. False for a step's ask and a hold's ask, which take only their options |
| `recommendation`, `tradeoff` | string or null | What the agent recommends, and the trade-off |
| `title`, `text` | string or null | The ask's title and its Markdown body. A human step's ask has the task's [merge risk](../daemon/tools.md#merge-risk) as its body, when it has one |
| `what_waits`, `next_action` | string or null | What waits on the answer, and what the agent does next |
| `command` | object, string or null | An agent's command: `argv`, `target`, `dir`, the absolute folder it runs in, and `runs`, the options that run it. See [tools.md](../daemon/tools.md#commands-an-answer-runs). An outside ask's command is the string it sent, which never runs |
| `ran` | object or null | The command its answer ran: `status` (`running`, `done`, `failed` or `uncertain`), `revision`, `error`, `stdout` and `stderr`. Null when no answer ran it |
| `coordinator`, `run_id`, `session` | string or null | Who asked from outside Priory: its coordinator, its run id and its session |
| `answer`, `answered_at` | string or null | The answer and when it came |
| `answered_by` | string or null | Who answered: `cli`, `serve`, or `agent:<lead>` for a lead's answer. Null when unanswered, or answered before Priory recorded it |
| `routed_to` | string or null | The lead the ask is routed to, which answers it in the person's place, or null for a person. See [tools.md](../daemon/tools.md#hands-off) |
| `delivery` | object or null | The latest delivery of an outside ask's answer or dismissal to its session: `state` (`queued`, `running`, `delivered` or `failed`), `revision`, `tries`, `error`, `since` and `cleared`, when a person cleared a failed one from the desk with `priory deliver --clear`. Null when nothing was delivered. See [asks.md](../daemon/asks.md) |
| `since` | string | When the agent asked |
| `revisions` | array | Every revision of the question, oldest first: `revision`, `question`, `options`, `recommendation`, `text`, `at`, when it was asked, and `answer` and `answered_at`, the answer that revision took, or null. An ask stored before Priory kept revisions lists only its current one, dated when the ask was first asked |
| `refused` | array | Every answer the daemon refused, oldest first: `revision`, the one the answer was sent for, `answer`, `by` (`cli` or `serve`), `why`, the CLI's error, and `at` |
| `snoozed_until`, `snoozed_at`, `back_early` | | The ask's [snooze](#snooze) |
| `runner` | string, absent with one runner | In a factory of several runners, the runner that owns the ask |
| `last_seen` | string or null | Another runner's ask only: when its runner was last alive, from its last `priory.runner.alive` |
| `submissions` | array | Another runner's ask only: every answer and dismiss sent to it from any machine, oldest first: `submission`, `verb` (`answer` or `dismiss`), `revision`, `text`, `sent`, then `outcome` (`answered`, `dismissed` or `refused`), `why` and `applied`, null until its runner applied it |

In a factory of several runners, `asks` also lists every other runner's asks from their last
[`priory.ask.posted`](events.md#prioryaskposted), with `<runner>:<ask>` as `id`. Their `task` is the
task's number on that runner, and they carry no snooze.

`priory asks` without `--json` prints only the open asks that are not snoozed, and `--all` adds the
snoozed ones.

## Snooze

An `ask`, `blocked`, `failed` or `prompt` desk item, and every ask, carry these fields. Only an ask
of a task can be snoozed: an outside ask's fields stay null. See [desk.md](../daemon/desk.md#snooze).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `snoozed_until` | string or null | When the item comes back by itself, while it is snoozed |
| `snoozed_at` | string or null | When a person last snoozed it |
| `back_early` | object or null | `{ "at", "reason" }` once a change brought it back before its time. `reason` is `new revision`, `task state changed` or `new failure` |

## Tasks

Every task in start order. Without `all=1`, the tasks still going and the tasks that ended (`done`,
`failed` or `dropped`) in the last 7 days; `older`, an integer, is the number left out. Each task
has `id`, `key`, `workflow`, `step`, `state`, `band`, `lane`, `reason`, `position` (its place in
the queue from 1, or null), `generation`, `retired`, `tldr`, `since`
(when the task last changed state), `grade` (`red`, `yellow`, `green` or null), `grade_reason`,
`progress`, `conditions`, a held task's [conditions](#conditions) or null, `title` (its TL;DR, else
the first line of its request, cut to 120 characters), `batch` (the id of the batch it is an entry
or the planning task of, or null), `stack` and `relations`. See Grades in [tools.md](../daemon/tools.md#grades).

`relations` lists every relation the task is in, on either side, oldest first (see
[relations.md](../daemon/relations.md)). Each entry:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `task` | integer | The task the relation reads from, as in `5 fixes 3` |
| `kind` | string | `builds-on`, `fixes`, `takes-up` or `relates` |
| `other` | integer | The task it relates to |
| `by` | string | Who recorded it: a person's `$USER`, `agent`, `request` or `priory` |
| `created` | RFC 3339 | When it was recorded |

`stack` lists the pull requests the task opened or claimed, bottom first, each based on the one
before it (see [Stacks in routing.md](../daemon/routing.md#stacks)). It is empty when the task has
none. Each entry:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `pull_request` | string | The resource, such as `github:owner/name#58` |
| `number` | integer | The pull request's number |
| `branch` | string | Its head branch |
| `base` | string | Its base branch: the trunk for the bottom one, the branch below for the others |
| `state` | string | `open`, `merged` or `closed`, from the intake's close events |

`progress` is null until the task's agent or a person publishes one. Then it is an object with any
of `goal`, `acceptanceCriteria` (each `{ "id", "description", "required" }`), `latestChange` and
`nextAction`. See the `progress` tool in [tools.md](../daemon/tools.md).

## Runs

The runs still live and those started in the last 7 days, newest first. Without `all=1`, `older`,
an integer, is the number of ended runs left out. `priory runs --json` prints every run, as `?all=1` does;
`priory runs` prints them as a table, without the task, result and trace fields. Each run:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `run` | integer | The run's id |
| `task` | integer | Its task's id |
| `key` | string | Its task's key |
| `step` | string | The step it ran |
| `agent` | string or null | The agent it ran |
| `started` | RFC 3339 | When it started |
| `ended` | RFC 3339 or null | When it ended; null while it is live |
| `end` | string or null | How it ended, such as `done`, `moved`, `blocked`, `lost` or `exited`. See [runs.md](../daemon/runs.md) |
| `outcome` | string or null | Its result's outcome, such as `passed` or `approved`, or null without a result |
| `model` | string or null | The model its `factory.step.ended` event recorded |
| `tokens` | object or null | That event's `input`, `output`, `cache_read` and `cache_write` token counts |

## Tasks that hold a pull request

`/api/tasks?pr=RESOURCE` returns the active tasks that hold one pull request, as `{"revision": N,
"tasks": [...]}`. `RESOURCE` is the pull request's resource, percent-encoded, such as
`github%3Aowner%2Frepo%237` for `github:owner/repo#7`. The owner and repository match in any case,
as GitHub's do, so `github:Owner/Repo#7` finds the same tasks. The GitHub extension reads it to find the
task to show, and to know when no task holds the pull request at all. A task can hold it in several
ways at once (see [Resources in routing.md](../daemon/routing.md#resources-what-a-task-owns)).

Each task has `id`, `key`, `workflow`, `step`, `state` and `holds`, the ways it holds the pull
request, in this order:

| `holds` | The task |
| ------- | -------- |
| `key` | has the pull request as its key |
| `stack` | opened or claimed it: it is in the task's [stack](#tasks) |
| `wait` | waits, or waited, on it |
| `subscription` | subscribes to it now |
| `review` | has its review key, `github:owner/repo:pulls/N:review:REVIEWER` |
| `owns` | none of the above, but still owns it, so its events reach the task, such as a pull request its stack later dropped |

The tasks that hold it by `key` or `stack`, which own its branch, come first, then the others, each
group by id. An empty `tasks` means no active task holds the pull request.

## Task in detail

The fields of [Tasks](#tasks), plus `repo`, `branch`, `base_sha` (the commit the task's worktree was
first created from, or null for a task with no worktree yet or one created before Priory kept it), `intervention` (`plan`, `hands-off` or null)
`lead` (the agent that answers a `hands-off` task's asks, or null), `merge_risk` (as in
[Asks](#asks)), `plan_goal`, `plan_testing`, `plan_brief` and `verification` (what the latest result that gave
them sent as its `goal`, `testing`, `brief` and `verification`, else null; see [`result`](../daemon/tools.md#resultoutcome-summary-evidence-commit-reason-branches-risk-goal-testing-brief-verification-feedback-batch)),
and these arrays, oldest first unless the table says otherwise:

| Field | Each entry |
| ----- | ---------- |
| `runs` | `id`, `step`, `agent`, `tmux`, `session`, `started`, `ended`, `end`, `outcome` (its result's, or null), `tokens` (the tokens its step used, as the `factory.step.ended` trace record holds them, or null) |
| `steps` | A code step's action: `id`, `step`, `action`, `status`, `pull_request`, `commit_sha`, `error`, `created` |
| `asks` | An ask, as in [Asks](#asks) |
| `waits` | `id`, `run`, `kind`, `resource`, `until`, `target`, `created`, `fired`, `withdrawn` (1 once withdrawn, else null), `withdrawn_reason` (see [withdrawal reasons](../daemon/waits.md#why-an-ask-or-wait-was-withdrawn)), `withdrawn_at` (when, in UTC; null for a wait withdrawn before Priory kept the time) |
| `results` | `run`, `step`, `outcome`, `summary`, `evidence`, `commit_sha`, `reason`, `merge_risk` (a risk step's verdict and lines, else null), `created` |
| `subscriptions` | `resource`, `created`: what the task subscribes to now. Empty once it fails or is dropped |
| `inbox` | Every event routed to the task, newest first: its source event, the events routing gave it, and those another task took that fired one of its waits or that its subscription delivered to it. Each has `seq`, `type`, `subject`, `time`, `task` (the task routing gave it, or null), `outcome` (as [routing](../daemon/routing.md#outcomes) recorded it, or for an event a subscription delivered beside another task, `woke` or `kept` for this task), `prompt` (the state of this task's prompt that carries the event: `queued`, `attempted`, `uncertain`, `confirmed`, `withdrawn`, or null when none does) and `wait` (the id of this task's wait it fired, or null). See [The task's inbox](../daemon/routing.md#the-tasks-inbox) |
| `triage` | `item` (a feedback item's URL), `class` (`trivial`, `nothing-to-do` or `persons-call`), `run` (the run whose result sorted it), `created`: how the agent sorted each review feedback item, in the order it first did. See [Review feedback](../daemon/tools.md#review-feedback) |

And these objects:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `brief` | object | `title` and `request`, the request's title and text, each null when it has none, and `result`, the latest result of a step named `groom`, or null |
| `plan` | object or null | The latest result of a step named `plan`. A result has `run`, `step`, `outcome`, `summary`, `evidence`, `commit_sha` and `created`. When it is null, `progress` holds the plan the agent published |
| `pull_request` | object or null | The latest pull request the task owns that has stored GitHub events: `repo`, `number`, `title`, `url`, `branch`, `base`, `head_sha` (each from the newest event that carries it), `state` (`open`, `closed` or `merged`; a reopened pull request stays `closed`, since the intake publishes no reopen event), `review` (the latest review's state, or null) and `checks`, the latest conclusion of each check run on the head: `name`, `conclusion` and `at` |

## Queue

| Field | Each entry |
| ----- | ---------- |
| `bands` | The name of one of the factory's bands, highest first, as the daemon last recorded them. A band that holds no task is listed too |
| `workflows` | The name of one of the factory's workflows, sorted. `priory queue --json` without `DIR` lists none |
| `up_next` | One of the next three runs, soonest first: `at`, `kind`, `task`, `key`, `title`, `trigger` and `why`. See [Up next](#up-next) |
| `waiting` | A queued or ready task in start order, then each held task by when it was held: `position` and `band_position` (its place in the queue and in its band, null for a held task), `id`, `key`, `title`, `workflow`, `step`, `state`, `band`, `lane`, `reason`, `since`, and `conditions` (see [Conditions](#conditions)), null for a task that is not held |
| `asleep` | An asleep task, longest asleep first: `id`, `key`, `title`, `workflow`, `step`, `band`, `lane`, `since` (when it fell asleep), `retired`, and `waits`, each open wait as `kind`, `on` (what it waits on, as `priory waits` shows it) and `since` (when it was stored). A task asleep only on an ask has no waits |
| `running` | A live run: `run`, `task`, `key`, `title`, `step`, `agent`, `started` |
| `ended` | A task done, failed or dropped in the last day, latest first: `id`, `key`, `title`, `workflow`, `step`, `state`, `reason`, `ended` |

A row's `title` is its task's TL;DR when one is set, else the first line of its request text, cut
to 120 characters. It is null for a schedule firing that starts a new task.

### Up next

`/api/queue` reads the schedules of the factory `serve` runs. `priory queue DIR --json` reads those
of the factory in `DIR`, and `priory queue --json` lists no schedule firings.

| `kind` | `at` | `why` |
| ------ | ---- | ----- |
| `queued` | Now: it starts once there is room | `first in the queue`, `a band above it`, or `queue position <n>` |
| `held` | The end of its time hold, once time is all it still waits for | `held until <time>` |
| `scheduled` | The trigger's [next firing](../intake/schedule.md#next-firing) | `schedule <trigger> fires` |

`task` and `key` name the task, and are null for a firing that starts a new task. `trigger` names
the schedule trigger of a `scheduled` entry, and is null otherwise.

- A schedule without a fixed `key` starts a task at each firing, so up next can list several of
  its firings.
- A schedule with a fixed `key` lists only its next firing. Its later firings coalesce into that
  task.
- A firing for a live task that is not asleep coalesces into it, so up next leaves it out.
- Up next leaves out a schedule with a `hold`, and held tasks that wait for more than time. They
  wait for their release, so they have no start time.

### Conditions

A held task lists each condition of its hold, in the order it was given, under `conditions`.
`priory tasks --json` and `/api/tasks/{id}` carry the same field.

| Field | Meaning |
| ----- | ------- |
| `kind` | `person`, `time`, `task`, `issue`, `merge`, `blockers`, or a batch entry's `merged` or `waiting` (see [batches.md](../daemon/batches.md)) |
| `until` | A time condition's instant |
| `task` | The other task of a task, `merged` or `waiting` condition |
| `resource` | `linear:<issue>` for an issue or blockers condition, `github:<owner/name>#<number>` for a merge |
| `met` | When the condition was met, or null |
| `met_as` | How it was met, such as `task 9 is done` or `ENG-7 is Done`, or null |


## Costs

`/api/costs` prices from the `prices.toml` of the factory `serve` runs, read when it starts. Both
parameters are optional: `since` takes an RFC 3339 instant or a duration such as `30 days`, and `by`
one of `workflow` (the default), `agent`, `model`, `machine` and `repo`. See
[trace.md](../daemon/trace.md#costs) for the windows and what each dimension reads.

| Field | Value |
| ----- | ----- |
| `by` | The split's dimension |
| `current`, `previous` | The two windows: `since` and `until`, `since_day`, the local date `since` falls on in the daemon's timezone, as `days` are keyed, then a [total](#cost-total) and `merges` |
| `days` | One entry per local day of either window with a record or a merge, oldest first: `day` (`YYYY-MM-DD`), a [total](#cost-total) and `merges` |
| `split` | One entry per value of the current window's records, sorted: the value under the dimension's name, such as `"model": "claude-opus-5-5"`, and a [total](#cost-total) |

### Cost total

| Field | Value |
| ----- | ----- |
| `cost_usd` | The dollars of the priced tokens. Null when no record was priced and some were unpriced or had no tokens |
| `tokens` | Every counted token: `input`, `output`, `cache_read` and `cache_write` |
| `unpriced` | Each model with no price: `model` and its `tokens` |
| `unknown` | The records with no `tokens`: model steps, handler calls and summarizer calls |

## Grades

`/api/grades` takes an optional `since`, an RFC 3339 instant or a duration such as `30 days`, and
counts the tasks created since then; without it, every graded task. See
[trace.md](../daemon/trace.md#grades) for what each label means.

| Field | Value |
| ----- | ----- |
| `since` | The window's start as an instant, or null |
| `one_shot` | Green's headline: `clean`, its `approved-clean` tasks; `outcomes`, its tasks with a final outcome; `rate`, `clean` over `outcomes`, or null with none; and `target`, `0.9` |
| `grades` | One entry per grade, `red`, `yellow` then `green`: `grade`; `tasks`, its tasks; `outcomes`, those with a final outcome; and `labels`, an object with each of the six labels as a key, each `{"count": N, "share": S}`, where `share` is `count` over `outcomes`, or null with none |

## Attention

Both parameters are optional: `since` takes what `/api/costs` takes, and `by` one of `workflow` (the
default), `lane` and `kind`. See [trace.md](../daemon/trace.md#attention) for what counts as an act,
a wait and away time. Minutes are numbers, not rounded.

| Field | Value |
| ----- | ----- |
| `since`, `until` | The window |
| `by` | The split's dimension |
| `acts` | The person's acts in the window |
| `person_minutes` | The attention those acts cost |
| `merges` | The merged pull requests in the window, as `/api/costs` counts them |
| `person_minutes_per_merge` | `person_minutes` divided by `merges`, or null with no merge |
| `waits` | The [waits](#waits) of the asks asked in the window |
| `split` | One entry per value of the acts and waits, sorted: the value under the dimension's name, such as `"lane": "lane-a"`, then `acts`, `person_minutes` and `waits` |
| `tasks` | One entry per task, in id order: `id`, `key`, `workflow`, `lane`, `state`, `since` (its creation), `until` (its end, or null while active), then `wall_minutes` split into `working_minutes`, `person_wait_minutes`, `world_wait_minutes` and `queued_minutes` |

### Waits

| Field | Value |
| ----- | ----- |
| `acted` | The ask revisions a person acted on |
| `open` | The ask revisions still waiting |
| `median_minutes`, `p90_minutes` | The acted waits' median and nearest-rank 90th percentile; null with none |
| `median_present_minutes`, `p90_present_minutes` | The same, without the time the person was away |

## Views

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `factory` | string | The factory's name |
| `views` | object | Each view by name: the views Priory ships, each replaced by the factory's view of the same name, then the factory's other views. Each is the file's tree as JSON, such as `{ "layout": { "tabs": [{ "panel": "desk" }] } }`. See [views.md](views.md) |

The server reads the views when it starts. `priory views [DIR] --json` prints the same object from
the factory's files, with no server running.

## Links

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `links` | array of objects | The factory's [autolinks](factory.md#links), in the order a view applies them: each `pattern`, a regex, and `url`, its URL template with `$1`-style groups and `{repo}` |

```json
{ "links": [{ "pattern": "\\B#(\\d+)\\b", "url": "https://github.com/{repo}/issues/$1" }] }
```

The server reads the links when it starts. `priory links [DIR]` without `--json` prints them as
`pattern` and `url` columns.

## Flows

The same object as `priory check --explain --json` (see
[workflows.md](workflows.md#irreversible-steps-and-their-gates)), with `workflows` added for the
web view's Flows panel. The server reads it when it starts. `priory flows [DIR] --json` prints the
same object, for the Herdr adapter's Flows panel.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `irreversible` | array of objects | Each irreversible step: `workflow`, `step`, `approved_by` (the human steps that approve it) and `autonomous` (its reason, or `null`) |
| `paths` | array of objects | Each path from `start` to an irreversible step that follows no `human` edge: `workflow` and `path`, an array of step ids |
| `tools` | array of objects | Each guarded call an agent can make with no human decision: `agent`, `guard`, `workflow` and `why` |
| `proofs` | array of objects | Each outcome the daemon verifies before it accepts it: `workflow`, `step`, `outcome` and `proof` |
| `workflows` | array of objects | Each workflow by name: `name`, `start` and `steps` |

A step has `id`, `decides`, `effect`, `kind`, `edges` and, when the step sets them, `agent`,
`options`, `approves` and `autonomous`, and, on an irreversible step, `approved_by`: the human steps
that approve it, as in `irreversible`. `kind` is how a drawing shows the step:

| `kind` | The step |
| ------ | -------- |
| `read` | has effect `read` |
| `reversible` | has effect `reversible` |
| `irreversible` | has effect `irreversible` |
| `ask` | is a human step with no `approves` |
| `answer` | is a human step that `approves` a step |
| `verdict` | has effect `none` and is not a human step |

Each of a step's `edges`, in outcome order, has `outcome`, `to` (a step id, `done` or `stop`) and
`by`: `code`, `model`, `human` or `asleep`.

```json
{
  "irreversible": [{ "workflow": "implement", "step": "open-pr", "approved_by": ["approve-pr"], "autonomous": null }],
  "paths": [],
  "tools": [],
  "proofs": [],
  "workflows": [{
    "name": "implement",
    "start": "open-pr",
    "steps": [{
      "id": "open-pr", "decides": "code", "effect": "irreversible", "kind": "irreversible", "approved_by": ["approve-pr"],
      "edges": [{ "outcome": "opened", "to": "done", "by": "code" }, { "outcome": "failed", "to": "stop", "by": "code" }]
    }]
  }]
}
```

## Web view files

With `web = true`, the server sends these without the token:

| Path | Type |
| ---- | ---- |
| `/boot.js`, `/app.js`, `/asks.js`, `/add.js`, `/links.js`, `/reports.js`, `/stack.js`, `/desk.js`, `/costs.js`, `/inbox.js`, `/flows.js`, `/batches.js` | `text/javascript; charset=utf-8` |
| `/app.css`, `/tokens/structure.css`, `/tokens/themes.css` | `text/css; charset=utf-8` |
| `/fonts/<file>.woff2` | `font/woff2` |
| `/sw.js`, the service worker | `text/javascript; charset=utf-8` |
| `/manifest.webmanifest` | `application/manifest+json` |
| `/favicon.ico`, `/favicon.svg`, `/apple-touch-icon.png`, `/icon-192.png`, `/icon-512.png`, `/icon-maskable-192.png`, `/icon-maskable-512.png` | the image's type |
| any other path outside `/hooks`, such as `/`, `/asks/42` or `/batches/4` | the page, `text/html; charset=utf-8` |

An unknown file under `/fonts/` or `/tokens/`, or at the top level with a `.` in its name, such as
`/nothing.js`, gets `404`.

Every file carries `Content-Security-Policy: default-src 'none'; script-src 'self'; style-src
'self'; font-src 'self'; img-src 'self'; connect-src 'self'; manifest-src 'self'; worker-src
'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`. The
page loads nothing from another host.

## Notifications

[serve.md](../daemon/serve.md#notifications) explains when the daemon sends a notification.

`GET /api/push/key` returns `{"key": "<base64url>"}`: the uncompressed P-256 public key, 65 bytes,
that a browser passes to `PushManager.subscribe` as `applicationServerKey`. Before a daemon made the
key pair, it returns `503`.

`POST /api/push/subscribe` stores the browser's subscription, or replaces the keys of one with the
same `endpoint`. The daemon refuses it with `422` when:

- `endpoint` is not an `https://` URL, with no port and no user info, on one of the push
  services browsers use: `push.apple.com`, `fcm.googleapis.com`, `push.services.mozilla.com` or
  `notify.windows.com`, or a name under one of them;
- `keys.p256dh` is not a base64url P-256 public key, or `keys.auth` is not a base64url 16 byte
  secret;
- 20 other subscriptions exist.

`POST /api/push/unsubscribe` drops the subscription with that `endpoint`, and answers `200` even
when none had it. Both reply `{"ok": "subscribed"}` or `{"ok": "unsubscribed"}`.

Each notification is a Web Push message encrypted as RFC 8291 says (`Content-Encoding:
aes128gcm`), signed with VAPID (RFC 8292), with `TTL: 86400` and `Urgency: high`. Its plaintext is
JSON with two fields and nothing else:

| Field | For a new ask | For an alarm |
| ----- | ------------- | ------------ |
| `title` | the ask's title, cut to 120 characters; without one, `task <key> asks at <step>`, or `ask <name>` for an outside ask | `task <key> <state>`, such as `task gh:o/r#42 blocked` |
| `url` | `/asks/<id>` | `/tasks/<id>` |

## Report

The report a workflow wrote, in the shape of `docs/schemas/report.json`, with two more fields. See
[reports.md](../daemon/reports.md).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `task` | integer | The task that wrote it |
| `revision` | integer | Raised by each write, pick and comment verb; `priory report pick --expected` and `priory report comment --expected` take it |
| `date`, `since`, `greeting` | string | As written |
| `sections` | array | In the order written. Each row's `comments` holds its threads, as [reports.md](../daemon/reports.md#comment-on-a-row) shows |
