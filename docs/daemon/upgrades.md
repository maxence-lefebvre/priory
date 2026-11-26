# Upgrades

With `[upgrade]` in `factory.toml`, the daemon installs newer releases of Priory by itself. Without
it, the daemon looks nothing up. To turn it on, follow
[How to upgrade Priory automatically](../upgrading-automatically.md).

## Finding and verifying a release

Every `every`, the daemon reads the list of releases from GitHub with `curl`, with no token and over
HTTPS alone. It takes the newest version of its `channel`, never a draft, a pre-release, an older
version or a version it refused. Then it downloads the binary for this machine and checks three
things:

- its sha256 is the one in the `.sha256` beside it,
- GitHub holds a build provenance attestation for that sha256, and
- the attestation verifies.

The attestation is a Sigstore bundle. Priory checks it offline, with two keys in its source and no
`gh`:

1. Fulcio's intermediate key signed the certificate, which GitHub Actions asked for.
2. The certificate names `.github/workflows/release.yml` of this repository, at the tag `v<version>`.
   It also holds this repository's and its owner's numbers, which a rename does not change, the event
   `push`, and a GitHub-hosted runner.
3. The certificate's key signed the statement, and the statement names the binary and its sha256.
4. Rekor's key signed an entry of this very signature, at a time inside the certificate's 10 minutes.

It does not check the Merkle proof of that entry, nor the certificate's timestamp from the
transparency log. When Sigstore rotates either key, each release is refused with a line on the desk
until a person installs one that holds the new key.

The running binary must verify the same way, or the daemon would replace a build someone made
themselves. A check that fails changes nothing and shows on the desk once.

The right to push a `v*` tag to the repository is enough to make a release that passes all of this:
the workflow that runs is the one at the tag. Protect the tags, not only the default branch.

## A quiet moment

A verified release waits in `$PRIORY_HOME/upgrade/`, which only the daemon's user may enter and
which a jail hides, until all of these hold:

- it is outside `[upgrade.not_during]`,
- priory is not paused, and
- the daemon runs no command, recipe, sweep script, handler, summary or delivery, and no shared
  session is being compacted.

A restart would end each of those `interrupted`, which loses its work. A live run, a tool call or
a memory grant does not hold the upgrade back: the run's agent stays in tmux, and its tool calls,
hooks and `priory memory` wait for the next daemon. After a day, the desk says what it waits for.
It never forces its way in.

An edit of a key that only a restart applies, such as `[serve]`, puts the upgrade off until the
daemon restarted with it: nothing shows yet that a daemon starts with it.

A tool call the agent makes while the daemon is down waits up to 30 seconds, as in an upgrade by
hand. A call in flight at the stop is answered first, after the phase of the tick in progress
([home.md](home.md#socket-spool-and-lock)). The tool guard does not wait: a guarded command the agent
starts in the gap is denied, and the agent sees why.

## The rehearsal

Before anything stops, the old binary and then the new one run `priory check` on the factory, and
`priory upgrade probe` on copies of the state and the log. A release that fails where the old binary
passes is refused. When the old one fails too, the fault is this machine's and the upgrade is put
off.
When the copies' version or tables changed, the release migrates. A migration that only adds leaves
the oldest schema that may still open the state in its floor (see
[Additive migrations](#additive-migrations)). When the running binary is at least that old, the
release installs like one that does not migrate. Otherwise it needs no live run at all, not only a
quiet moment.

## The swap

The daemon starts a helper, a private copy of the old binary, in a transient user unit. It runs the
copy's `--version` first, then waits up to 10 seconds until the helper holds its lock. Without that,
the daemon removes the journal, puts the upgrade off and goes on. With it, the daemon starts nothing
more until the helper stops it.

The helper, `priory upgrade resume`, first gets ready while both services run:

1. it copies the new binary beside the old one, and checks that copy's sha256 and attestation, the
   old binary's attestation, and `priory check` with both. The old binary then runs
   `priory upgrade startable` with the environment of the daemon's unit, as its next start would read
   it: a missing token or secret or a taken address puts the upgrade off;
2. it gives the old binary two more names, `priory.<old version>` and one for a rollback, and
   reserves 64 KiB for a rollback's last records;
3. it looks for a quiet moment again. For a release that migrates, no other process may have either
   database open, such as a standalone `priory serve`, whose webhooks a rollback would lose;
4. it records which of the two units run. It later starts those alone, never one a person stopped.
   It also records whether the intake ran for as long as it would be watched.

A failure so far stops nothing. All of this has 30 seconds, after which the helper puts the upgrade
off and the daemon, which waits for it, works on at most 10 seconds later. Then it:

5. stops both services, looks for a quiet moment once more, and copies both databases beside
   themselves with `VACUUM INTO`. When this fails, it starts them again;
6. renames the new binary over the old one, then runs `priory check` with it;
7. starts the services, waits up to 2 minutes for the daemon to answer its version on the socket,
   and watches for a minute that it stays the same process. It watches the intake too, unless that
   was in a crash loop before the stop.

After a migration, only the daemon starts at step 7. It opens its databases and answers its version,
but reads no message, serves nothing, dispatches nothing and starts no run. Any other process that
would open the state or the log is refused with
`an upgrade of priory is in progress: try again in a few minutes`. The helper commits once the minute
passed, and the daemon goes on and the intake starts.

Priory keeps the old binaries and backups of the last 3 upgrades, and removes older ones that it
made itself.

## Additive migrations

Each migration after schema 123 declares its kind in `KINDS` in `state.rs`, and one without a
declaration does not compile. An additive one creates tables, indexes and triggers on its own tables,
and adds columns that are null or have a default. It refers to no old table, and its release writes
no value into an old column that the release before it would misread. A test refuses a declared
additive migration that does more, and one measures the write lock each holds on a 30,000 row inbox.

The daemon keeps the oldest schema that may open the state in the file's `application_id`: the
schema of its last migration that is not additive. Any binary at least that old opens the newer
file and leaves it as it is. One of 1.3.8 or before knows no floor and refuses it.

So an additive release starts the intake with the daemon, holds nothing back, and keeps the state on
a rollback: the old binary opens it with what the new one wrote.

## Rollback

When a step fails from step 6 on, the helper stops the services, puts the old binary back, and
starts them. It first moves back the backup of each database whose own schema the release changes,
unless the change is additive, and leaves the other as it is. A rollback only renames and unlinks, so a full disk does not stop it.
Where the disk does not take the journal either, the helper removes the journal and starts the
services without it.

The helper writes each phase to `$PRIORY_HOME/upgrade/journal.json` before it starts it. Systemd
starts a killed helper again. Before the swap it goes on, and after it, it rolls back and puts the
upgrade off. A service that starts and finds a journal that no helper works on, as after a reboot,
starts the helper and exits. When that helper does not start and nothing was changed yet, the
service removes the journal and runs.

## Refused or put off

A release is refused for good when the fault is its own: it fails `priory check` where the old binary
passes, it says another version, or its daemon ends by itself. When the old daemon does not answer
after the rollback either, the fault is this machine's: the refusal is cleared, and the desk shows the
put-off once a daemon runs.

Everything else puts the upgrade off: a factory the old binary refuses too, a missing tool, a step
that ran out of its time, a daemon that is up but does not answer in time, an almost full disk or quota, an intake that ends, a
killed helper or daemon, a busy moment, or a service a person stopped meanwhile. It waits 1 hour, then 2, 4, 8
and 16, then 24 each time, also across a restart. So a release whose daemon hangs is tried again
once a day until a newer one comes. Each try stops the services, and the trace holds each reason.

`priory upgrade retry VERSION` forgets a refusal or a wait.

Each end writes one `factory.upgrade.installed`, `factory.upgrade.rolled_back` or
`factory.upgrade.put_off` record, and one report on the desk (see [the trace schema](../trace-schema.md#factoryupgradeinstalled-factoryupgraderolled_back-factoryupgradeput_off-and-factoryupgraderefused)).
The helper leaves it in a file, outside the databases a rollback replaces, and the daemon that runs
records it once.

## Several runners

Several runners share a Postgres log, and Priory cannot tell whether a release changes its tables
without opening it. So `priory check` refuses `[upgrade]` with a Postgres log, and these machines
are upgraded by hand, one at a time. `upgrade = false` in a runner file leaves one machine out of a
factory on a herdr-bus log.

## Limits

- Once a migration is committed, going back is a person's choice: it loses what came after the backup.
- A rollback compares tables, indexes, triggers and the schema's number. What a release writes into
  unchanged tables before it fails stays.
- Both versions check the live factory, each twice on a failure. Edits that hit both checks of the
  new version and neither of the old one refuse it, until `priory upgrade retry`.
- After a migration, what the daemon does only after the commit, such as its server's start, is not
  watched.
- The binary's folder must be on a filesystem with hard links.
- Only this user's processes are seen to hold a database, and one that opens it after the last look
  is not.
- A service that SIGTERM ended by any other way than `systemctl stop` counts as stopped by a person.
- A run outside a jail is the same user, so it can write to `$PRIORY_HOME/upgrade/`. The attestation
  is checked again on the copy that is installed, but not against a write after that.
- A release that a compromised release workflow built verifies like any other.
- A reboot in the middle of an upgrade, with a new binary that does not even start, leaves the
  services down.
- A `herdr-bus` client of another program that writes the log's file directly is not held back.
- A quota shows only where `fallocate` is on the `PATH` and works, or where the failure names it.
- The check of the daemon's environment needs systemd 249 or later for the variables its unit sets
  with `Environment=`, and reads their values from the running daemon.
- Only systemd user services upgrade. The daemon never re-executes itself.
