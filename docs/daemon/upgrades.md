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
3. The certificate's key signed the statement, and the statement names the binary and its sha256.
4. Rekor's key signed an entry of this very signature, at a time inside the certificate's 10 minutes.

It does not check the Merkle proof of that entry, nor the certificate's timestamp from the
transparency log. When Sigstore rotates either key, each release is refused with a line on the desk
until a person installs one that holds the new key.

A check that fails changes nothing and shows on the desk once.

## A quiet moment

A verified release waits in `$PRIORY_HOME/upgrade/` until all of these hold:

- no `[upgrade.window]` is open,
- no memory grant is held or waits,
- the daemon runs no command, recipe, sweep script, handler, summary or delivery, and no shared
  session is being compacted, and
- no live run is inside a tool call: its transcript holds a result for each call of its last message.

A run whose transcript cannot be read counts as busy. After a day, the desk says what it waits for.
It never forces its way in.

The transcript can lag behind a call by a moment. A tool call the agent makes while the daemon is
down waits up to 30 seconds, as in an upgrade by hand. A call in flight at the stop can still fail.

## The rehearsal

Before anything stops, the new binary runs `priory check` on the factory, then
`priory upgrade probe` on copies of the state and the log. A release that fails either is refused.
When the copies' version or tables changed, the release migrates. It then needs no live run at all,
not only a quiet moment.

## The swap

The daemon starts a helper, a private copy of the old binary, in a transient user unit, and exits.
The helper, `priory upgrade resume`:

1. stops both services, and looks for a quiet moment again now that nothing can start. Without one,
   or when step 2 fails, it starts them again, and the daemon waits an hour;
2. copies both databases beside themselves with `VACUUM INTO`, and keeps the old binary as
   `priory.<old version>`, or with a longer name when that file exists;
3. renames the new binary over the old one, then runs `priory check` with it;
4. starts the services, waits up to 2 minutes for the daemon to answer its version on the socket,
   and watches for a minute that it stays the same process.

After a migration, only the daemon starts at step 4. It opens its databases and answers its version,
but reads no message, serves nothing and starts no run. Any other process that would open the state
or the log is refused with `an upgrade of priory is in progress: try again in a few minutes`. The
helper commits once the minute passed, and the daemon goes on and the intake starts.

Priory keeps the old binaries and backups of the last 3 upgrades, and removes older ones that it
made itself.

## Rollback

When a step fails from step 3 on, the helper stops the services, puts the old binary back, and
starts them. After a migration it puts both backups back first. Nothing else wrote to them, so
nothing is lost. A release that changed no schema leaves the databases alone.

The helper writes each phase to `$PRIORY_HOME/upgrade/journal.json` before it starts it. Systemd
starts a killed helper again. Before the swap it goes on, and after it, it rolls back. A service that
starts and finds a journal that no helper works on, as after a reboot, starts the helper and exits.

Each end writes one `factory.upgrade.installed` or `factory.upgrade.rolled_back` record, and one
report on the desk (see [the trace schema](../trace-schema.md#factoryupgradeinstalled-factoryupgraderolled_back-and-factoryupgraderefused)).
The helper leaves it in a file, outside the databases a rollback replaces, and the daemon that runs
records it once.

## Several runners

Several runners share a Postgres log, and Priory cannot tell whether a release changes its tables
without opening it. So `priory check` refuses `[upgrade]` with a Postgres log, and these machines
are upgraded by hand, one at a time. `upgrade = false` in a runner file leaves one machine out of a
factory on a herdr-bus log.

## Limits

- Once a migration is committed, going back is a person's choice: it loses what came after the backup.
- A release that a compromised release workflow built verifies like any other.
- A reboot in the middle of an upgrade, with a new binary that does not even start, leaves the
  services down.
- A `herdr-bus` client of another program that writes the log's file directly is not held back.
- Only systemd user services upgrade. The daemon never re-executes itself.
