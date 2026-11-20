# Serve

Priory serves the factory's state over HTTP, as JSON under `/api`, for the web view and other
adapters. The server reads the state; it changes nothing. The keys are under
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

Every request but a webhook delivery needs `Authorization: Bearer <token>`. The token is the value
of the environment variable that `token_env` names, `PRIORY_TOKEN` by default. When that variable
is unset or empty, `priory serve` and `priory daemon` refuse to start: the server never runs open.
Any other request gets `401`. The server compares the token in a time that does not depend on how much of it matches,
and never logs it.

Generate a token once and keep it in the services' environment file, not in the factory:

```sh
echo "PRIORY_TOKEN=$(openssl rand -hex 32)" >> ~/.config/priory/env
```

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

The server holds at most 4 such requests at once, half its 8 workers, so a listing always finds a
free worker. Past that, `/api/events` answers `503` with `Retry-After: 1`. Worktrees on disk are
not in the state: a desk item for a worktree changes without a new revision.

## Limits

| Limit | Value | Past it |
| ----- | ----- | ------- |
| Request head: request line and headers | 8 KiB, 32 headers | `431` |
| Request body | none allowed, except 1 MiB on `POST /hooks/github` | `413` |
| Time to send the whole request, head and body | 5 seconds | `408` |
| Connections served at once | 8 | Others wait in the listen backlog |
| `/api/events` requests held at once | 4 | `503` |

Each connection carries one request and closes after the reply.

## Webhooks

With [`[webhooks.github]`](../reference/factory.md#webhooksgithub), the server also takes signed
GitHub deliveries at `POST /hooks/github`, with no token. [webhooks.md](../intake/webhooks.md)
explains them.

## The web view

With `web = true`, `/` and `/index.html` serve the web view. Until the web view is built, that is a
placeholder page. With `web = false`, they get `404` and only `/api` answers.
