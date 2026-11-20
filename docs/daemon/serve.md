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

Every request under `/api` needs `Authorization: Bearer <token>`. The token is the value of the
environment variable that `token_env` names, `PRIORY_TOKEN` by default. When that variable is unset
or empty, `priory serve` and `priory daemon` refuse to start: the server never runs open. An `/api`
request without the token gets `401`. The server compares the token in a time that does not depend on how much of it matches,
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

## Limits

| Limit | Value | Past it |
| ----- | ----- | ------- |
| Request head: request line and headers | 8 KiB, 32 headers | `431` |
| Request body | none allowed | `413` |
| Time to send the whole request head | 5 seconds | `408` |
| Connections served at once | 8 | Others wait in the listen backlog |

Each connection carries one request and closes after the reply.

## The web view

With `web = true`, every path outside `/api` serves the web view: its page, scripts, styles, fonts
and icons, all built into the binary. They carry no state, so they need no token. The page asks
for the token once, keeps it in the browser's local storage, and sends it on every `/api` request;
a `401` brings the sign-in back. A strict Content-Security-Policy lets the page load only its own
files, and the page shows agent text as text. The view's layout comes from the factory's
[views](../reference/views.md). With `web = false`, every path outside `/api` gets `404`.
