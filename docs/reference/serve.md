# `serve` endpoints

The HTTP server's endpoints and the JSON they return. [serve.md](../daemon/serve.md) explains where
the server runs, its token and its limits.

Every endpoint takes `GET`, except the [writes](#writes), `POST /hooks/github` and `POST /hooks/linear`, which take
`POST`. Each one under `/api` needs `Authorization: Bearer <token>`; the web view's files do not.
Each listing returns the same JSON as
the CLI command with `--json`, with the state's `revision`, an integer, read in the same
transaction. A listing that is an object has a `revision` field. A listing that is a list comes as
`{"revision": N, "<name>": [...]}`, where `<name>` is `inbox` or `tasks`.
`/api/costs`, `/api/attention` and `/api/grades` are reports rather than listings: they have no `revision`.

`/api/asks` and `/api/tasks` leave out the rows that closed over a week ago, and add `older`, the
number they left out: see [Asks](#asks) and [Tasks](#tasks). `?all=1` returns every row, as the CLI
does, with every count in `older` at 0. Their rows match the CLI's JSON only with `?all=1`.

| Endpoint | CLI | Returns |
| -------- | --- | ------- |
| `/api/desk?person=NAME` | `priory desk --json --person NAME` | an object: [Desk](#desk) |
| `/api/inbox` | `priory inbox --json` | `inbox`: an array of [inbox messages](#inbox) |
| `/api/asks?band=BAND&risk=RISK&all=1` | `priory asks --json --band BAND --risk RISK` | an object: `asks`, an array of [asks](#asks), `bands` and `older` |
| `/api/tasks?all=1` | `priory tasks --json` | `tasks`: an array of [tasks](#tasks), and `older` |
| `/api/tasks/{id}` | `priory task ID --json` | one [task in detail](#task-in-detail) |
| `/api/queue` | `priory queue --json` | an object: [Queue](#queue) |
| `/api/views` | | an object: [Views](#views) |
| `/api/request` | | what the web view's Add prefills: `workflow` and `band`, the [`[request]`](factory.md#request) defaults, each `null` when unset; `workflows`, the factory's workflow names; and `bands`, its bands in order |
| `/api/links` | `priory links --json` | an object: [Links](#links) |
| `/api/flows` | `priory flows --json` | an object: [Flows](#flows) |
| `/api/reports/latest` | `priory report show --json` | the [report](#report) written last |
| `/api/reports/{task}` | `priory report show TASK --json` | the task's [report](#report) |
| `/api/costs?since=WINDOW&by=SPLIT` | `priory costs --json --since WINDOW --by SPLIT` | an object: [Costs](#costs) |
| `/api/grades?since=WINDOW` | `priory grades --json --since WINDOW` | an object: [Grades](#grades) |
| `/api/attention?since=WINDOW&by=SPLIT` | `priory report attention --json --since WINDOW --by SPLIT` | an object: [Attention](#attention) |
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
| `400` | The request does not parse, or its target does not start with `/`; `/api/events` lacks `since` or has a parameter out of range; `/api/desk` has a parameter other than `person`, or a broken escape; `/api/asks` has a parameter other than `band`, `risk` and `all=1`, a band the factory does not have, or a risk other than the five [below](#asks); `/api/tasks` has a parameter other than `all=1`; `/api/costs` has a parameter other than `since` and `by`, a broken escape, a `since` that cannot form a window (see [trace.md](../daemon/trace.md#costs)), or a `by` other than the five [below](#costs); `/api/grades` has a parameter other than `since`, a broken escape, or a `since` that does not parse; `/api/attention` has the same faults, with a `by` other than the three [below](#attention); a write's body is not the shape [below](#writes); for a webhook, it sends two `Content-Length`, or its payload lacks a field its event needs |
| `401` | The token is missing or wrong on a path under `/api`; for a webhook, the signature is |
| `403` | A `POST` carries an `Origin` whose host and port differ from its `Host`, such as a page on another site |
| `404` | No such path, task, report, ask or web view file |
| `405` | `POST` on a path that reads, or `GET` on a write. The `Allow` header names the method |
| `408` | The request sent no byte within 1 second, or took over 5 seconds in all |
| `409` | A write names an ask whose revision moved or that closed. The body adds the ask as `/api/asks` lists it. Or `/api/request` repeats a text already sent for the same linked issue. Or a cancel's `revision` is no longer the state's. Or a pick's report moved past its `revision`, or another choice took the pick |
| `411` | A `POST` has no `Content-Length`, or uses `Transfer-Encoding` |
| `413` | A `GET` has a body; a write's body is over 16 KiB; a webhook's is over 1 MiB |
| `415` | A write's `Content-Type` is not `application/json` |
| `422` | The daemon refused a write for another reason, such as an option a step does not take, or a request's unknown workflow or bad `repo`, or a push subscription it does not take. A request the bus could not take gets `422` too, with the bus's error |
| `431` | The request head is over 8 KiB or 32 headers |
| `500` | The state could not be read, a delivery could not be published, or Priory failed while answering. The server keeps serving |
| `503` | 4 requests already wait on `/api/events` and this one would wait too; or a write found no daemon on its socket, as under `priory serve` alone; or `/api/push/key` before a daemon made the key pair. It comes with `Retry-After: 1` |
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

## Writes

Each write sends the daemon the same line as its CLI verb, and the daemon applies it.
[serve.md](../daemon/serve.md#writes) explains why. Each needs the token, `Content-Type:
application/json` and a JSON body of at most 16 KiB with exactly the fields below.

| Endpoint | Body |
| -------- | ---- |
| `POST /api/asks/{id}/answer` | `revision`, an integer, and one of `option` or `text`, a string. Both become the answer: a step's ask takes only one of its options. `run`, a boolean, default `false`: `true` confirms an answer that runs the ask's command, as `priory answer --run` does. Without it, that answer gets `422` |
| `POST /api/asks/{id}/dismiss` | `revision`, an integer |
| `POST /api/snooze` | `kind` (`ask`, `blocked`, `failed` or `prompt`) and `id`, an integer, then one of: `for`, a duration such as `1h`; `until`, an RFC 3339 time; or `"clear": true` |
| `POST /api/request` | `text`, a string, then optionally `workflow`, `band`, `lane` and `repo`, strings, and `intervention`, `plan` or `hands-off`. What it leaves unset comes from `[request]` and the URLs in `text`, as for [`priory add`](../intake/cli.md#priory-add) |
| `POST /api/tasks/{id}/resume` | `{}` |
| `POST /api/tasks/{id}/revive` | optionally `step`, a string: the step to restart at, as `--step` |
| `POST /api/tasks/{id}/cancel` | optionally `revision`, an integer: the state's revision the client showed, as `--expected`. A newer state gets `409` |
| `POST /api/tasks/{id}/release` | optionally `revision`, an integer, as `--expected`. A newer state gets `409` |
| `POST /api/tasks/{id}/say` | `text`, a string: what the task's agent hears |
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

In `/api/asks/{id}`, `{id}` is the ask's `id` as `/api/asks` lists it: its number, or an outside
ask's name, and `revision` is the ask's `revision` the client showed. In `/api/tasks/{id}`, `{id}` is
the task's number.

```sh
curl -X POST http://127.0.0.1:7340/api/asks/42/answer -H "Authorization: Bearer $PRIORY_TOKEN" \
  -H 'Content-Type: application/json' -d '{"option": "merge", "revision": 3}'
```

| Reply | When |
| ----- | ---- |
| `200` `{"ok": "ask 42 answered"}` | The daemon applied it |
| `200` `{"key": "cli:2026-11-20-1", "id": 7}` | For `/api/request`: the daemon published the request and task `id` holds `key`. `id` is `null` when no task took the key |
| `409` `{"error", "ask"}` | The ask's revision is no longer the one sent, or it was answered, dismissed or withdrawn. `ask` is its current state |
| `409` `{"error"}` | A cancel's `revision` is no longer the state's. Or a pick's report is at another revision than the one sent, or the pick already has a choice: read the report again |
| `409` `{"error", "key", "id"}` | For `/api/request`: the same text already went to `key`, so nothing was added. `id` is the active task that holds `key`, or `null` |
| `404`, `422` `{"error"}` | The ask, task or pick is gone, or the daemon refused for another reason, such as a resume of a task that is not blocked, a say to a task with no run yet, or a comment on a stale report revision. `error` is the CLI's sentence |
| `503` `{"error"}` | No daemon listens on the socket, so nothing was sent. Reads keep working |
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
| `kind` | string | `ask`, `delivery`, `blocked`, `failed`, `prompt`, `command`, `held`, `sweep`, `memory`, `report`, `retired`, `queue` or `worktree`. See [desk.md](../daemon/desk.md) |
| `id` | integer or string | The ask, task, prompt, answer's action, hold, sweep, grant, report's inbox sequence number or queue place; a worktree's path |
| `task` | integer or null | The task |
| `key` | string or null | The task's key |
| `step` | string or null | The task's step |
| `lane` | string or null | The task's lane |
| `since` | string or null | When the item started waiting |
| `what` | string | What the item is about |
| `routed_to` | string or null | An `ask` item's lead, which answers it in the person's place, or null. Other kinds have no `routed_to` |
| `generation` | integer or null | The task's generation |
| `snoozed_until`, `snoozed_at`, `back_early` | | The item's [snooze](#snooze) |

## Inbox

Each message and report, oldest first: `seq`, `type`, `subject`, `task`, `outcome`, `time`, `line`
and `read`. A report has the type `priory.report`, its source as `subject`, the outcome `report`, its
line as `line`, and, once read, when it was read as `read`. Both are null on a message.

## Asks

Every ask, oldest first, in `asks`, and the factory's bands, highest first, in `bands`. With
`band`, only the asks of tasks in that band. `band` takes a comma-separated list and repeats, so
`?band=now,today` and `?band=now&band=today` are the same. A band the factory does not have gets
`400`, with the factory's bands in the error. With `risk`, only the asks that carry that risk:
`irreversible`, `red`, `yellow` or `green`, or `none` for an ask with no risk. `risk` takes a list
and repeats the same way, and an ask matches when it carries any of the risks asked. Another value
gets `400`. Both filters together keep the asks that match each. `priory asks --band BAND` and
`priory asks --risk RISK` filter the same way and refuse the same values.

Without `all=1`, `asks` holds the open asks and the asks answered or asked in the last 7 days. The
state keeps no time for a withdrawal, so a withdrawn ask counts from when it was asked. `older` is
an object: `answered` and `withdrawn`, the number of each left out, within `band` and `risk` when they are given.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | integer or string | The ask: its number, or the id an outside ask was given |
| `task`, `key`, `step`, `lane`, `band`, `repo` | | The task, its key, the ask's step, and the task's lane, band and repository. An outside ask has no task, so its `lane`, `band` and `repo` are null, and its `key` is the one it sent |
| `risk` | array of strings | What answering risks, empty for none: `irreversible` when the answer lets an irreversible step run before a person decides again, the rule `priory check --explain` uses, or when it is an agent's or a step's ask whose answer runs a [command](../daemon/tools.md#commands-an-answer-runs); then the task's grooming grade, `red`, `yellow` or `green`. An outside ask has none. The daemon records its factory's irreversible approvals when it starts |
| `run` | integer or null | The run that asked |
| `state` | string | `open`, `answered` or `withdrawn` |
| `withdrawn_reason` | string or null | Why a withdrawn ask closed, one of the [withdrawal reasons](../daemon/waits.md#why-an-ask-or-wait-was-withdrawn): `dismissed`, `sibling-fired`, `moved-on` or `revived` |
| `withdrawn_why` | string or null | `withdrawn_reason` in words: `dismissed`, `a wait of its run fired first`, `a later run called another tool` or `its task was revived` |
| `revision` | integer | The question's revision. `priory answer` and `priory dismiss` take it |
| `question` | string | The question |
| `options` | array of strings | The options |
| `recommendation`, `tradeoff` | string or null | What the agent recommends, and the trade-off |
| `title`, `text` | string or null | The ask's title and its Markdown body |
| `what_waits`, `next_action` | string or null | What waits on the answer, and what the agent does next |
| `command` | object, string or null | An agent's command: `argv`, `target`, `dir`, the absolute folder it runs in, and `runs`, the options that run it. See [tools.md](../daemon/tools.md#commands-an-answer-runs). An outside ask's command is the string it sent, which never runs |
| `ran` | object or null | The command its answer ran: `status` (`running`, `done`, `failed` or `uncertain`), `revision`, `error`, `stdout` and `stderr`. Null when no answer ran it |
| `coordinator`, `run_id`, `session` | string or null | Who asked from outside Priory: its coordinator, its run id and its session |
| `answer`, `answered_at` | string or null | The answer and when it came |
| `answered_by` | string or null | Who answered: `cli`, `serve`, or `agent:<lead>` for a lead's answer. Null when unanswered, or answered before Priory recorded it |
| `routed_to` | string or null | The lead the ask is routed to, which answers it in the person's place, or null for a person. See [tools.md](../daemon/tools.md#hands-off) |
| `delivery` | object or null | The latest delivery of an outside ask's answer or dismissal to its session: `state` (`queued`, `running`, `delivered` or `failed`), `revision`, `tries`, `error` and `since`. Null when nothing was delivered. See [asks.md](../daemon/asks.md) |
| `since` | string | When the agent asked |
| `revisions` | array | Every revision of the question, oldest first: `revision`, `question`, `options`, `recommendation`, `text`, `at`, when it was asked, and `answer` and `answered_at`, the answer that revision took, or null. An ask stored before Priory kept revisions lists only its current one, dated when the ask was first asked |
| `refused` | array | Every answer the daemon refused, oldest first: `revision`, the one the answer was sent for, `answer`, `by` (`cli` or `serve`), `why`, the CLI's error, and `at` |
| `snoozed_until`, `snoozed_at`, `back_early` | | The ask's [snooze](#snooze) |

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
`progress`, `conditions`, a held task's [conditions](#conditions) or null, `stack` and
`relations`. See Grades in [tools.md](../daemon/tools.md#grades).

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

## Task in detail

The fields of [Tasks](#tasks), plus `repo`, `branch`, `intervention` (`plan`, `hands-off` or null)
and `lead` (the agent that answers a `hands-off` task's asks, or null), and these arrays, oldest
first:

| Field | Each entry |
| ----- | ---------- |
| `runs` | `id`, `step`, `agent`, `tmux`, `session`, `started`, `ended`, `end`, `outcome` (its result's, or null), `tokens` (the tokens its step used, as the `factory.step.ended` trace record holds them, or null) |
| `steps` | A code step's action: `id`, `step`, `action`, `status`, `pull_request`, `commit_sha`, `error`, `created` |
| `asks` | An ask, as in [Asks](#asks) |
| `waits` | `id`, `run`, `kind`, `resource`, `until`, `target`, `created`, `fired`, `withdrawn` (1 once withdrawn, else null), `withdrawn_reason` (see [withdrawal reasons](../daemon/waits.md#why-an-ask-or-wait-was-withdrawn)) |
| `results` | `run`, `step`, `outcome`, `summary`, `evidence`, `commit_sha`, `reason`, `created` |
| `subscriptions` | `resource`, `created`: what the task subscribes to now. Empty once it fails or is dropped |

And these objects:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `brief` | object | `title` and `request`, the request's title and text, each null when it has none, and `result`, the latest result of a step named `groom`, or null |
| `plan` | object or null | The latest result of a step named `plan`. A result has `run`, `step`, `outcome`, `summary`, `evidence`, `commit_sha` and `created`. When it is null, `progress` holds the plan the agent published |
| `pull_request` | object or null | The latest pull request the task owns that has stored GitHub events: `repo`, `number`, `title`, `url`, `branch`, `base`, `head_sha`, `state` (`open`, `closed` or `merged`; a reopened pull request stays `closed`, since the intake publishes no reopen event), `review` (the latest review's state, or null) and `checks`, the latest conclusion of each check run on the head: `name`, `conclusion` and `at` |

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
| `kind` | `person`, `time`, `task`, `issue`, `merge` or `blockers` |
| `until` | A time condition's instant |
| `task` | A task condition's other task |
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
| `current`, `previous` | The two windows: `since` and `until`, then a [total](#cost-total) and `merges` |
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

The server reads the views when it starts.

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
| `/boot.js`, `/app.js`, `/asks.js`, `/add.js`, `/links.js`, `/reports.js`, `/stack.js`, `/desk.js`, `/costs.js`, `/flows.js` | `text/javascript; charset=utf-8` |
| `/app.css`, `/tokens/structure.css`, `/tokens/themes.css` | `text/css; charset=utf-8` |
| `/fonts/<file>.woff2` | `font/woff2` |
| `/sw.js`, the service worker | `text/javascript; charset=utf-8` |
| `/manifest.webmanifest` | `application/manifest+json` |
| `/favicon.ico`, `/favicon.svg`, `/apple-touch-icon.png`, `/icon-192.png`, `/icon-512.png`, `/icon-maskable-192.png`, `/icon-maskable-512.png` | the image's type |
| any other path outside `/hooks`, such as `/` or `/asks/42` | the page, `text/html; charset=utf-8` |

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
