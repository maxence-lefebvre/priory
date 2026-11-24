# How to upgrade Priory automatically

This guide shows how to let the daemon install newer releases by itself. It assumes the daemon and
the intake run as systemd user services (see
[How to run Priory as services](running-as-services.md)) from a release binary, on Linux x86_64 or
aarch64, with `curl` on their `PATH`.

It is off until you turn it on. Keep upgrading by hand on macOS, without systemd, with a Nix
install, with a build that has the `model` feature, or with a Postgres log, so on several runners
that share one.

## 1. Check the install

Both units must run one binary by its own path, not a link, in a folder the user can write:

```sh
systemctl --user show --value --property=ExecStart priory-daemon priory-intake
```

`PRIORY_DB` and `HERDR_BUS_DB` must be set to absolute paths in the units' environment file.

## 2. Turn it on

In `factory.toml`:

```toml
[upgrade]
channel = "patch"

[upgrade.window]
from = "09:00"
until = "19:00"
days = ["mon", "tue", "wed", "thu", "fri"]
timezone = "Europe/Paris"
```

`patch` stays on the running major and minor version. `minor` stays on the major version. The
daemon never upgrades inside the window. Leave the window out to allow any time.
[`[upgrade]`](reference/factory.md#upgrade) lists every key. The daemon follows the edit without a
restart.

To leave one machine out, set `upgrade = false` in its
[runner file](reference/runners.md#keys).

Check: `priory check` passes, and within an hour the desk shows no line that starts with
`priory cannot upgrade itself here`.

## 3. Read what it did

Each upgrade and each rollback writes one report on the desk and one record in the trace:

```sh
priory desk
priory trace | grep factory.upgrade
journalctl --user -u 'priory-upgrade-*'
```

A version that failed is never tried again. Priory takes the next one. To try it again yourself,
remove it from `refused` in `$PRIORY_HOME/upgrade/state.json` while the daemon is stopped.

## Go back by hand

An upgrade that changed the schema is not rolled back once it was committed, since that would lose
what happened after it. The `factory.upgrade.installed` record names the old binary in `kept` and
the two copies in `backups`:

```sh
systemctl --user stop priory-daemon priory-intake
cp <kept> ~/.cargo/bin/priory.new && mv ~/.cargo/bin/priory.new ~/.cargo/bin/priory
rm -f "$PRIORY_DB"-wal "$PRIORY_DB"-shm "$HERDR_BUS_DB"-wal "$HERDR_BUS_DB"-shm
cp <state backup> "$PRIORY_DB"
cp <log backup> "$HERDR_BUS_DB"
systemctl --user start priory-intake priory-daemon
```

Skip the three database lines when `schema_from` equals `schema_to` and the release changed no
table of the log. Set the channel aside first, or the daemon installs the release again.

How the daemon verifies, waits and rolls back is in [upgrades.md](daemon/upgrades.md).
