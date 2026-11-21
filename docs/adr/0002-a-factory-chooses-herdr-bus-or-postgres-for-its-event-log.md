# A factory chooses herdr-bus or Postgres for its event log

Date: 2026-11-21. Status: accepted.

## Context

[ADR 0001](0001-the-intake-and-the-daemon-reach-the-event-log-through-one-log-trait.md) put the
event log behind the `Log` trait, with herdr-bus's SQLite file as its one adapter. R9.4 in the PRD
asks for a second, Postgres, so that the log can travel between machines, and for a factory to
switch between the two.

## Decision

A factory names its log in `factory.toml`:

```toml
[log]
backend = "postgres"        # or "herdr-bus", the default
url_env = "PRIORY_LOG_URL"  # the default
```

`log::open` takes this table. Every process that opens the log, the intake, the daemon, the
webhooks and `priory add`, reads the same factory, so they all reach the same store.

The connection string stays in the environment variable `url_env` names, because it holds a
password. The file only names the variable.

The adapter uses the `postgres` crate, the synchronous client, with TLS through
`tokio-postgres-rustls` on the workspace's `rustls`. The client runs a private tokio runtime;
nothing in Priory becomes async. Both crates sit behind the `postgres` cargo feature, off by
default, so the default and Nix builds compile neither. `priory check` refuses
`backend = "postgres"` in a build without the feature.

Two tables hold the log: `priory_log_messages`, with a seq from a sequence and a unique index on
the output and the message id, and `priory_log_cursors`, one seq per consumer and output. The
contract's guarantees map onto them this way:

- A repeated id inserts nothing, and `publish` returns the seq the id has.
- Publishers take a transaction-scoped advisory lock, so seqs commit in order. Without it, a
  reader could see seq 8 before seq 7 commits, ack 8, and never read 7.
- `read` selects past the consumer's cursor, and `ack` moves the cursor forward only.
- The first connection creates the tables, under an advisory lock so that two processes starting
  at once do not race.
- When the server drops the connection, the next operation connects again and runs once more.
  Every operation is idempotent, so the second run repeats nothing.

`declare` adds nothing: herdr-bus's `Output` keeps its name private, and the tables hold every
output. Publishing on an undeclared output therefore works, where herdr-bus refuses it. Every
Priory publisher declares first, and an output with no messages reads empty.

## Consequences

- Moving a factory from herdr-bus to Postgres starts the log empty. Messages already in
  herdr-bus's file stay there, and the intake resumes each source as on its first start.
- One publisher writes at a time across every machine, to keep seqs in commit order.
- CI runs the contract suite against a Postgres service on every push. Locally it runs when
  `PRIORY_TEST_POSTGRES_URL` is set, and skips otherwise.
