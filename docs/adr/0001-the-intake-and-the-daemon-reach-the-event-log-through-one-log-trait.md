# The intake and the daemon reach the event log through one Log trait

Date: 2026-11-21. Status: accepted.

## Context

The intake publishes every source event on the `factory.intake` output, and the daemon reads it
as a consumer with a cursor. Until now both called herdr-bus directly. R9.4 in the PRD brings a
second store, Postgres, so that the log can travel between machines. Principle 6 says a boundary
becomes swappable in the milestone that brings its second adapter, and M3 is that milestone.

## Decision

`priory_intake::log::Log` is the boundary. It has six methods, the operations Priory uses today
and no others:

| Method | Used by |
| ------ | ------- |
| `declare` | the intake, the webhook and `priory request`, before they publish |
| `publish` | every publisher. A message whose id the output holds already adds nothing |
| `message` | `priory request`, to read back what it published |
| `messages` | the intake's start, paging back newest first to resume each source |
| `read` | the daemon, reading past its cursor as a named consumer, oldest first |
| `ack` | the daemon, after the message's state change commits |

`log::open()` returns the machine's log, today herdr-bus's SQLite file. `herdr_bus::Bus`
implements the trait.

The trait keeps herdr-bus's message types: `Message`, the CloudEvents-shaped envelope that
[envelope.md](../intake/envelope.md) describes, `Seq` and `Stored`. The envelope is the contract
between the intake and its consumers, so it stays the same whatever stores it.

A delivery names its output and seq, not a herdr-bus connection. `ack` takes the consumer and the
delivery, so another store needs no notion of a connection. The herdr-bus adapter looks the
connection up again on each ack, one more small write. Herdr-bus pulls every connection of a
consumer at once, so the adapter keeps only the deliveries of the output's own connection. When
the consumer's other connections, such as a tap made with herdr-bus's CLI, fill a whole read, the
read fails and names them instead of returning nothing.

Reading an output nobody declared yet returns nothing. The daemon can then start before the
intake ever ran, as it could before.

Herdr-bus requests, with a deadline, a take and a reply, stay out of the trait: Priory does not
use them yet.

One contract test suite, `crates/priory-intake/tests/log.rs`, runs against herdr-bus and against
an in-memory log kept in the tests. The in-memory log proves the suite asks for nothing herdr-bus
alone does. A new adapter joins the suite with one line.

## Consequences

- A Postgres adapter implements six methods and passes the suite. Choosing it per factory is the
  second half of R9.4.
- The trait returns boxed errors. The one error the daemon matched on, an undeclared output, is
  now part of the contract instead.
- The webhook and intake lock file, `bus.changes.lock`, still sits beside herdr-bus's file. It
  orders two writers on one machine and is not a log operation.
