# Serve

Priory serves the factory's state over HTTP, as JSON under `/api`, for the web view and other
adapters. The server reads the state, and changes it only by sending the daemon the same lines as
the CLI, as [Writes](#writes) explains. The keys are under
[`[serve]`](../reference/factory.md#serve), and the endpoints in
[serve.md](../reference/serve.md).

## Where it runs

With a `[serve]` table in `factory.toml`, `priory daemon` starts the server next to its own loop.
Without one, nothing listens. `priory daemon --once` never serves.

`priory serve` runs the same server on its own, until it is killed:

```sh
priory serve [DIR] [--listen ADDR]
```

`DIR` is the factory folder, `.` by default. It needs a `[serve]` table. `--listen` replaces
`listen` for this run and follows the same loopback rule. Run it while no daemon serves, or give it
another port. Once it listens, it prints `priory: serving on http://ADDR` to stderr; with port `0`,
`ADDR` names the port the system chose.

Each server thread opens its own read-only connection to `PRIORY_DB`, so a request never waits on
the daemon's loop. The server never creates or migrates the state file: `priory serve` refuses to
start until a `priory daemon` of the same version has opened it once.

## The token

Every request under `/api` needs `Authorization: Bearer <token>`; a webhook delivery and the web
view's files do not. The token is the value of the environment variable that `token_env` names,
`PRIORY_TOKEN` by default. When that variable is unset or empty, `priory serve` and `priory daemon`
refuse to start: the server never runs open. An `/api` request without the token gets `401`. The
server compares the token in a time that does not depend on how much of it matches,
and never logs it.

Generate a token once and keep it in the services' environment file, not in the factory:

```sh
echo "PRIORY_TOKEN=$(openssl rand -hex 32)" >> ~/.config/priory/env
```

## Writes

Eight endpoints change the state: answer an ask, dismiss one, snooze a desk item, add a task, and
resume, revive, cancel or say to a task. Each is the twin of a CLI verb: `priory answer`, `priory
dismiss`, `priory snooze`, `priory add`, `priory resume`, `priory revive`, `priory cancel` and
`priory say`. All but the add build the same line
the verb builds and sends it to the daemon on its socket, so the daemon applies it in the same
transaction, with the same checks and the same refusals. The server thread never opens the state
for writing. An add goes to the daemon on its socket too, but the daemon publishes it on the bus as
`priory add` does, then takes it in at once, so the reply names the new task. It is not one
transaction: a failure between the two messages leaves only the source event. [serve.md](../reference/serve.md#writes) lists the
bodies and status codes.

A write carries the revision of the ask the client showed, as `priory answer --revision` does. The
first answer to that revision stands. When the daemon refuses, the server reads the ask again: a
revision that moved, or an ask that closed, gets `409` with the ask, so a view can redraw it and
keep the person's draft. A cancel may carry the state's revision the client showed, as `priory
cancel --expected` does: a newer state gets `409`, and nothing is dropped.

`priory serve` without a daemon has no socket to send to: its writes get `503`, and its reads keep
working.

A browser cannot be made to write from another site. The token travels only in the `Authorization`
header, never in a cookie, so a form or script on another page has nothing to send: the browser
attaches no credential by itself, and the server answers no CORS preflight that would let a page
set the header. The server also refuses a `POST` whose `Origin` names another host with `403`.
That check alone does not stop DNS rebinding, where a hostile name resolves to this machine and its
origin then matches `Host`. The token still stops it: the web view keeps the token in the local
storage of its own origin, which a rebound page cannot read. Keep it that way: never add cookie
auth.

## Beyond this machine

`listen` must be a loopback address, such as `127.0.0.1` or `::1`, unless `[serve]` sets
`public = true`. `priory check` refuses any other address without it. Exposing the state to a
network is the factory owner's choice for each machine. The server speaks plain HTTP: put it behind
a TLS proxy before it leaves the machine.

## Waiting for a change

The state carries a revision, a number that every committed change to the tasks, asks, runs, waits,
holds, inbox, reports, snoozes and the pause raises. SQLite triggers raise it in the same transaction
as the change, so no write path can miss it. Each listing carries the revision it read, read in one
transaction with its data.

A view keeps the revision of its last read and waits for the next one instead of polling:

- Over HTTP, `GET /api/events?since=REV` answers at once when the state's revision differs from
  `REV`. Otherwise it holds the request until a change, or for 25 seconds, and answers with the
  revision either way. The view reads its listings again when the revision changed.
- From the CLI, `priory wait --since REV [--timeout D]` does the same and prints the revision.

Each read costs what the listings hold, so `/api/asks` and `/api/tasks` keep to the open rows and
those closed in the last 7 days, and say how many they left out. The web view asks for every row
with `?all=1` only once the person scrolls to the older ones, or opens an older ask by its URL.

The server holds at most 4 such requests at once, half its 8 workers, so waiting clients leave 4
workers to the other requests. A fifth request that would wait gets `503` with `Retry-After: 1`; one
whose answer is ready gets it at once.

The revision changes with what is stored, not with the clock or the disk. Four things change a
listing without a new revision: a snoozed item comes back at its `snoozed_until`, an ended task
leaves the queue's `ended` list after a day, a closed ask or task leaves the default `/api/asks` or
`/api/tasks` after 7 days, and a desk item for a worktree follows the disk. A view
that shows them re-reads at those times as well.

The wait answers at once when the revision differs from `REV`, lower included, so a client that
kept a revision from a replaced state file does not hang.

## Limits

| Limit | Value | Past it |
| ----- | ----- | ------- |
| Request head: request line and headers | 8 KiB, 32 headers | `431` |
| Request body | none on a `GET`; 16 KiB on a write; 1 MiB on `POST /hooks/github` | `413` |
| Time to send the first byte of the request | 1 second | `408` |
| Time to send the whole request, head and body | 5 seconds | `408` |
| Time for each write of the reply to go out | 5 seconds | The connection closes |
| Connections served at once | 8 | Others wait in the listen backlog |
| `/api/events` requests held at once | 4 | `503` |

Each connection carries one request and closes after the reply.

## Webhooks

With [`[webhooks.github]`](../reference/factory.md#webhooksgithub), the server also takes signed
GitHub deliveries at `POST /hooks/github`, with no token. [webhooks.md](../intake/webhooks.md)
explains them.

## The web view

With `web = true`, every `GET` outside `/api` and `/hooks` serves the web view: its page, scripts, styles, fonts
and icons, all built into the binary. They carry no state, so they need no token. The page asks
for the token once, keeps it in the browser's local storage, and sends it on every `/api` request;
a `401` brings the sign-in back. A strict Content-Security-Policy lets the page load only its own
files, and the page shows agent text as text. The view's layout comes from the factory's
[views](../reference/views.md). Its Add control reads the defaults from `GET /api/request` and queues the task through `POST /api/request`, as `priory add` does. Its Reports panel reads `GET /api/reports/latest`, and picks and comments through the [report writes](../reference/serve.md#writes), as the `priory report` verbs do. With `web = false`, every path outside `/api` gets `404`.
