# How to run Priory as services

This guide shows how to keep the intake and the daemon running as systemd user services. It assumes
a factory that `priory check` accepts, and a clone of the repository.

`contrib/systemd/` holds two user units: `priory-daemon.service` runs `priory daemon` and
`priory-intake.service` runs `priory intake`. Both restart 5 seconds after a failure. They share one
environment file, so the factory folder and state paths are set once.

## 1. Install the units

Install the binary where the units look, `~/.cargo/bin/priory`. Then install the units and the
environment file, from your clone:

```sh
cargo install --path crates/priory
mkdir -p ~/.config/systemd/user ~/.config/priory
cp contrib/systemd/*.service ~/.config/systemd/user/
$EDITOR ~/.config/priory/env
systemctl --user daemon-reload
```

## 2. Write the environment file

`~/.config/priory/env` holds one `NAME=value` per line, with no quotes and no `export`:

```sh
PRIORY_FACTORY=/home/me/factory
PRIORY_DB=/home/me/.local/state/priory-oss/priory.db
HERDR_BUS_DB=/home/me/.local/state/priory-oss/bus.db
PATH=/home/me/.local/bin:/usr/local/bin:/usr/bin:/bin
```

`PRIORY_HOME` is optional (see [Priory home](daemon/home.md)). User services skip your shell profile,
so `PATH` must reach `claude`, `git`, `gh` and `tmux`. Systemd does not create the parent folders of
`PRIORY_DB` and `HERDR_BUS_DB`; create them first.

## 3. Start them

| Action        | Command                                                         |
| ------------- | --------------------------------------------------------------- |
| Start now and at login | `systemctl --user enable --now priory-intake priory-daemon` |
| Status        | `systemctl --user status priory-intake priory-daemon`           |
| Logs          | `journalctl --user -u priory-daemon -u priory-intake -f`        |
| Restart       | `systemctl --user restart priory-daemon`                        |
| Stop          | `systemctl --user stop priory-daemon priory-intake`             |

## 4. Keep them up after you log out

Without lingering, user services stop when your last session ends. On an always-on machine, run this
once. It changes the machine, so you run it yourself:

```sh
loginctl enable-linger "$USER"
```

## What a stop does to agents

The daemon unit sets `KillMode=process`. The default, `control-group`, would also kill the tmux server
the daemon started on a stop or restart. tmux stays in the unit's cgroup, so every agent dies with it.
`process` signals only the daemon, so agents keep running and the new daemon reconnects to them.
Systemd logs `Unit process ... (tmux: server) remains running after unit stopped`, which is expected.
`systemctl --user stop` therefore leaves agents running. End one with
`tmux kill-session -t =priory-<task id>` when you mean to.
