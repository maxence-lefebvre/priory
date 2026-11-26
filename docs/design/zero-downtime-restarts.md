# Zero-downtime restarts

This page explains what an upgrade of the `priory` binary does to a running factory today, and how
Priory could swap its binary and restart its processes without breaking a live run. The measurements
below are from before the first slice. [Later slices](#later-slices) says which slices are built.

## Why

Installing 1.1.0 moved the state from schema 100 to 106. The live Bonjour run kept the `priory mcp`
server its Claude Code session started, which was still the 1.0 binary. From then on, every call of
its `report` tool failed with `state schema 106 is newer than this priory knows (100)`, and the run
could not write its report. Until this ships, the rule is to install only when `priory runs` shows no
live run. A factory that always has a live run can then never upgrade.

## What broke before the first slice

The measurements ran on a scratch factory with the fake `claude` from `crates/priory/tests/fake/`,
its own tmux server, state, Priory home and log, and the web server on `127.0.0.1`. Binary A is
`main` at schema 107. Binary B adds one migration that creates a table, schema 108. Each pass:

1. starts the daemon and the intake on A, installed at one path;
2. opens three tasks: one asleep on an ask, one asleep on a `time` wait due during the gap, and one
   live run held in its turn just before a tool call;
3. starts a `priory mcp` server for the live run and keeps it, as Claude Code keeps one per session;
4. freezes the daemon, lets the held call reach its socket, then stops the daemon and the intake as
   systemd does: `SIGTERM` to the main process only;
5. installs B over the same path, waits, then starts both again. The gap was 81 seconds without a
   schema change and 98 seconds with one.

A third pass installs B while the daemon on A keeps running, then runs one `priory tasks`.

| What | Same schema | Schema 107 to 108 |
| ---- | ----------- | ----------------- |
| A live run and its tmux session | Survives. The new daemon keeps the run, since its pane lives | Same |
| A hook during the gap (the run's `Stop`) | Spooled to `signals.spool`, applied when the daemon starts | Same |
| A tool call sent to a frozen daemon, which then stops | Fails: `the daemon did not reply: Connection reset by peer`. Not applied, and nothing retries it | Same |
| A daemon tool (`progress`) on the kept MCP server, during the gap | Fails: `no priory daemon listens on ...` | Same |
| The same call after the restart | Works | Works |
| A local tool (`tldr`) on the kept MCP server | Works throughout, even with no daemon | Fails from the first command of B that opens the state, before any restart, and for the rest of the session |
| An ask open on the desk | Survives. Answered after the restart, the next run resumes the session with the answer | Same |
| A `time` wait due during the gap | Fires on the first tick after the start, 51 seconds late | Same, 71 seconds late |
| A schedule trigger every minute | The one missed slot is published on the restart | Of the two missed slots, only the later one is published |
| An HTTP client waiting on `/api/events` | Connection refused through the gap, answered 2 seconds after the start | Same, 1 second after the start |
| A CLI read of B before the restart | Works | Works, and migrates the state to 108 |
| B installed while daemon A runs | Not run | `priory tasks` on B migrates the state under daemon A. Daemon A and its web server keep working, because this migration only adds a table. The kept MCP server on A fails its local tools |

Not measured: a tool call that the daemon applied but whose reply was lost, a GitHub or Linear poll
across the gap, the Herdr panels, a real browser and push notifications. Webhooks go to the same
server as the web view, so they are refused through the same gap. GitHub and Linear are polled
anyway. Sentry, Honeycomb, Intercom, CloudWatch and Figma are not, so their deliveries in the gap
are lost unless the sender retries.

The code explains the table:

- `priory mcp` answers `report`, `tldr`, `search` and `relate` itself, from the state file
  (`sections::tool`, `search::tool`, `relate::tool`). Each opens it with `state::open`, which refuses
  a schema newer than the binary knows. Every other tool goes to the daemon's socket.
- `state::open` migrates the file forward, so the first process of a new binary that calls it
  migrates the state, whatever else still runs on the old one. The daemon calls it before it takes
  its lock in `runtime::listen`.
- The daemon handles no signal. `SIGTERM` ends it at once, so a line it accepted gets no reply.
- The daemon's socket takes one connection at a time and replies before it accepts the next.
- A hook whose line the daemon took but did not acknowledge within 10 seconds gives up, and does
  not spool it, so whether it was applied is unknown. A hook that opened the spool before the
  daemon renamed it, and writes after the daemon read it, is lost. `runtime.rs` marks that race as
  a shortcut.

## Where the docs and the code disagree

- The PRD's Architecture says the agents' MCP tools reach the daemon "through a stdio bridge on a
  local socket". Four of them open the state file instead ([tools.md](../reference/tools.md) says so
  for `tldr`, `search` and `report`).
- [running-as-services.md](../running-as-services.md#upgrade-the-binary) says a newer release
  migrates the state "when it opens it", and its steps stop both units before the install. It does
  not say that any process of the new binary migrates it, even with the old daemon running, nor that
  a live run's MCP server keeps the old binary and breaks.
- [runs.md](../daemon/runs.md) says the daemon "can stop and start again without stopping" the
  agents. That holds for the agents, but their tool calls fail through the gap, unlike hooks, which
  spool. No page says so.
- The brief points at runs.md for "interrupted by a daemon restart". In runs.md that reason belongs
  to recipes; [sweep.md](../daemon/sweep.md) uses it for stall scripts, and the code for code steps
  and handler calls. A live agent run is never ended that way: it is kept, or ended `lost` when its
  pane is gone.
- [health.md](../daemon/health.md) and [outbox.md](../daemon/outbox.md) match the code on restarts.
  Neither covers an upgrade.

## The model

An upgrade swaps one file and restarts two services. tmux already owns the agents, and the daemon
already adopts a run whose pane lives (R5.2). What breaks is everything that talks to the daemon or
the state across the swap. The design keeps two rules: only the daemon migrates the state, and every
client of the daemon waits briefly for it instead of failing.

### MCP servers survive a binary swap

Claude Code starts a run's `priory mcp` once, at the session's start, and keeps it until the
session ends. A resumed session starts a new one. Codex starts its servers the same way. Priory
cannot count on either to restart a server that exits mid-session, so the server must never exit on
its own, and a fix inside it lasts only for sessions that start after the fix ships.

| Option | Cost | What it rules out | Constraint |
| ------ | ---- | ----------------- | ---------- |
| **A thin proxy.** The four local tools go to the daemon's socket, as the others do. The server never opens the state | Small: the daemon gains four handlers. A slow `search` must not hold the socket, which takes one connection at a time today | `tldr` and `search` while no daemon runs, which the wait below restores | The line format on the socket becomes a contract between builds |
| Re-exec by path. On a schema error, the server `exec`s the installed `priory mcp`, keeping its stdio and pid | Medium: the new process must start already initialized and replay the failed call | Nothing, but it hides version skew instead of removing it | The installed path must stay stable, which `runtime::exe` already prefers |
| A schema window. An old binary opens a newer state when each migration since is additive | Medium: every migration declares whether it is additive, and the old binary must trust that | Renames and drops without a two-release dance | Covers only the state, not changes in the socket's lines |

Recommendation: the proxy. It matches the PRD, and it removes the only process that opens the
state, outlives an upgrade and cannot be restarted with it.

A tool call must also outlive the gap:

- When the socket refuses the connection, the line was never sent. The server retries every
  200 ms for up to 30 seconds, then fails as today. The daemon of this small factory answered 1 to
  2 seconds after it started.
- When the connection resets after the line was sent, the line may or may not have been applied.
  Each line carries a call id, a UUID, so ids from different clients never collide. The daemon stores the reply in the transaction that applies the line,
  and keeps it for an hour. The server sends the line again with the same id within that hour, and
  gets the stored reply instead of a second apply.

### The daemon hands over without ending runs

The adoption already works: the measurements kept the live run, its tmux session and its spooled
hook. Three gaps remain: a line in flight at the stop, the time the socket and the port are closed,
and the two ways a hook's line is lost.

| Option | Cost | What it rules out | Constraint |
| ------ | ---- | ----------------- | ---------- |
| **A graceful stop.** On `SIGTERM`, the daemon stops accepting, replies to each line it accepted, commits, and exits | Small | Nothing | systemd sends `SIGKILL` after `TimeoutStopSec`, 90 seconds by default, so the stop must finish first |
| **Clients that wait.** The MCP retry above, and the same wait in the verbs that call the daemon | Small, one helper in `runtime::call` | Nothing | A person's verb waits up to 30 seconds instead of failing at once |
| **Hooks that keep their line.** A hook spools its line when no acknowledgement arrives, with the same call id, so the daemon applies it once. The spool moves into the state, or into one file per line, so no rename races a write | Small to medium | Nothing | A hook runs inside the harness's own hook timeout |
| systemd socket activation. A `.socket` unit owns the hook socket and the serve port, so the kernel queues connections through the restart | Medium: the daemon takes `LISTEN_FDS`, and the units name a socket path that today hashes the home folder | Hosts without a systemd user manager, such as macOS, keep the gap | Needs the units installed from `contrib/systemd/` |
| Re-exec in place, passing the listening descriptors to the new binary | High: a second start path, and a failed `exec` leaves no daemon | Nothing, but it saves 1 to 2 seconds over a restart | systemd keeps tracking the same pid, so it works under the unit |

Recommendation: the graceful stop, clients that wait, and hooks that keep their line. They close
the gap for agents and verbs on every host. They do not keep a webhook from an unpolled source whose
sender does not retry; socket activation or re-exec in place would (see Open questions). Re-exec in
place costs the most for that.

### The intake loses no event

The intake reads back what it already published to the log, so a restart resumes each poll where it
stood ([github-polling.md](../intake/github-polling.md#failures)). A schedule publishes only its
latest missed slot on purpose ([schedule.md](../intake/schedule.md#when-it-fires)). The
measurements confirm the schedule only. A GitHub or Linear poll across the gap still needs a
measurement before this design claims it. A webhook sent while the server is down is lost for the
sources that are never polled; that gap is the daemon's gap above.

### Clients reconnect

The web view already retries `/api/events` every 5 seconds and reads again on success. Push
notifications stay queued in the state ([serve.md](../daemon/serve.md#notifications)). The Herdr
panels call `priory wait`, which polls the state's revision through `state::open`, not the socket.
It works through a daemon restart. A panel calls it again each time it returns, within 60 seconds,
and a call already open does not recheck the schema. So once the new daemon migrated, the next call
runs on the new binary and the new schema, and a call that refuses a schema in between is retried
by the panel's next call. A client must also tell "the daemon restarts" from "the daemon is
gone": the web view should show its offline state after 30 seconds without an answer, not at once.

### Migrations

Any process of a new binary that called `state::open` migrated the state, as the third pass
showed. Now only the daemon does, as it starts:

- The daemon takes its lock before it opens the state, so a second daemon cannot migrate under the
  first.
- Every other process creates the schema of an empty file. A process that writes the state waits up to
  30 seconds for the daemon to migrate an older one, as after an install, once: it prints a line when it
  starts waiting, and later opens refuse at once with `the state is at schema N and this priory uses M`.
  A read-only open refuses at once. An act on another runner goes unchecked without waiting. Nothing
  migrates under a running old daemon.
- The lock is per `PRIORY_HOME`, not per state file, so two homes must not share one database.
- The file's `application_id` holds its floor: the oldest schema whose binaries may open it. A binary
  opens a newer file when its own schema is at least the floor. 0 lets in that schema alone.
- A standalone `priory serve` keeps connections open, as the daemon does. `priory upgrade` restarts
  it after the daemon, and it refuses to start on a schema it does not know.
- A migration still runs in one transaction. Under WAL, readers keep reading through it, and other
  writers wait up to their 5 second busy timeout. A migration that would take longer, such as a
  backfill over every trace record, splits in two: a quick schema change at start, then a backfill
  in batches from the daemon's loop, with the code reading both shapes until it ends.

### The `priory upgrade` verb

`priory upgrade PATH` takes a downloaded binary. `priory upgrade --release vX.Y.Z` downloads one as
`install.sh` does and checks its `.sha256` and attestation. Then it:

1. **verifies**: runs `PATH --version` and `PATH check FACTORY`, and refuses a version older than the
   installed one;
2. **backs up**: `.backup` of the state and, for a SQLite log, of the log, beside each file with the
   old version in its name;
3. **rehearses**: copies the backup, migrates the copy with the new binary, prints how long it took,
   and removes the copy, so the backup stays at the old schema;
4. **installs**: copies the new binary next to the installed one, then renames it over the installed
   path in one step, keeping the old file as `priory.previous`;
5. **hands over**: `systemctl --user restart priory-daemon priory-intake` when the units exist,
   then a standalone `priory serve` unit if one runs, otherwise it prints the restarts to run; the
   new daemon migrates as it starts;
6. **checks**: waits for the socket to answer, then compares `priory runs` before and after. Every
   run that was live is still live, or the verb says which one is not and how to roll back.

`priory upgrade --rollback` stops both units and a standalone `priory serve`, puts `priory.previous`
and the backups back, and starts them again. It loses every change since the backup, such as an answer or a stored result, so it
lists them from the trace and asks before it proceeds.

## The first slice: no MCP server opens the state

Built. [tools.md](../daemon/tools.md#the-mcp-server) describes it.

- The daemon gains the `report`, `tldr`, `search` and `relate` lines, answered with the code that the
  MCP server runs today.
- The socket serves each connection on its own thread, and `search` runs off the daemon's loop, so
  a slow `search` holds up only other searches.
- `priory mcp` sends these four to the socket, as every other tool, and opens no state.
- The MCP servers retry a refused connection for up to 30 seconds, then give the reply its whole
  wait. Hooks and verbs still fail at once with no daemon.
- Tests: a kept MCP server answers all four tools after the state moved to a newer schema under it;
  a call made while no daemon listens succeeds once one starts within the window; a tool call is
  answered while a `search` waits on its embedder.
- Docs: tools.md stops saying that `priory mcp` answers from the state file, and
  running-as-services.md says that a run's MCP server survives a schema change, while a call in
  flight at the stop can still fail until the later slices.

An MCP server started after this slice ships survives later schema changes. A session started
before keeps its old server, so the first install of this slice still follows the lead's rule.

## Later slices

Each is useful alone, in this order:

1. **The graceful stop.** `SIGTERM` lets the daemon finish the lines it accepted before it exits. Built:
   [home.md](../daemon/home.md#socket-spool-and-lock) says how. The verbs wait for a restart too.
2. **Call ids.** A line resent after a reset is applied once, from the MCP server or a hook.
3. **Hooks that keep their line**, with the spool race closed. The race is closed: a hook appends under a
   lock on the spool, and starts over when the daemon has taken the file.
4. **Only the daemon migrates**, with its lock taken first. Built.
5. **`priory upgrade`.** Verify, back up, rehearse, install, hand over, check, and `--rollback`. The
   daemon does this by itself with `[upgrade]` ([upgrades.md](../daemon/upgrades.md)), on a herdr-bus
   log and one machine. The verb for a person, and one runner after another on a Postgres log, are left.
   There, a runner would wait until each runner before it by name committed the version, and stop at
   the first rollback.
6. **Split migrations.** A long backfill runs in batches from the daemon's loop.
7. **Socket activation**, if the open question below says webhooks need it.

## Open questions

- **Do webhooks for unpolled sources justify socket activation?** A restart took 1 to 2 seconds here,
  plus its migration. Sentry and the other unpolled sources lose a delivery only when it lands
  then. Priory cannot count what it refused. Recommendation: not yet. Read the senders' delivery logs
  after a few upgrades, and build it if one shows a lost delivery.
- **Does the first slice ship in a release that also bumps the schema?** If it does, sessions live at
  the install break once more. Recommendation: ship it in a release with no migration, so the next
  release with one finds every new session on the proxy.
