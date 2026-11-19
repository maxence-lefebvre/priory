# Services and listings

## Running as services

`contrib/systemd/` holds two user units: `priory-daemon.service` runs `priory daemon` and
`priory-intake.service` runs `priory intake`. Both restart 5 seconds after a failure. They share one
environment file, so the factory folder and state paths are set once.

Install the binary where the units look, `~/.cargo/bin/priory`. Then install the units and the file:

```sh
cargo install --path crates/priory
mkdir -p ~/.config/systemd/user ~/.config/priory
cp contrib/systemd/*.service ~/.config/systemd/user/
$EDITOR ~/.config/priory/env
systemctl --user daemon-reload
```

`~/.config/priory/env` holds one `NAME=value` per line, with no quotes and no `export`:

```sh
PRIORY_FACTORY=/home/me/factory
PRIORY_DB=/home/me/.local/state/priory-oss/priory.db
HERDR_BUS_DB=/home/me/.local/state/priory-oss/bus.db
PATH=/home/me/.local/bin:/usr/local/bin:/usr/bin:/bin
```

`PRIORY_HOME` is optional and defaults as under Priory home. User services skip your shell profile,
so `PATH` must reach `claude`, `git`, `gh` and `tmux`. Systemd does not create the parent folders of
`PRIORY_DB` and `HERDR_BUS_DB`; create them first.

| Action        | Command                                                         |
| ------------- | --------------------------------------------------------------- |
| Start now and at login | `systemctl --user enable --now priory-intake priory-daemon` |
| Status        | `systemctl --user status priory-intake priory-daemon`           |
| Logs          | `journalctl --user -u priory-daemon -u priory-intake -f`        |
| Restart       | `systemctl --user restart priory-daemon`                        |
| Stop          | `systemctl --user stop priory-daemon priory-intake`             |

Without lingering, user services stop when your last session ends. On an always-on machine, run this
once. It changes the machine, so you run it yourself:

```sh
loginctl enable-linger "$USER"
```

The daemon unit sets `KillMode=process`. The default, `control-group`, would also kill the tmux server
the daemon started on a stop or restart. tmux stays in the unit's cgroup, so every agent dies with it.
`process` signals only the daemon, so agents keep running and the new daemon reconnects to them.
Systemd logs `Unit process ... (tmux: server) remains running after unit stopped`, which is expected.
`systemctl --user stop` therefore leaves agents running. End one with
`tmux kill-session -t =priory-<task id>` when you mean to.

## Listings

`priory inbox` prints each entry, oldest first: seq, type, subject, task and outcome.
`priory tasks` prints each task in start order: id, key, workflow, step, state, band, lane, reason,
position, the task's queue place from 1 when `queued` or `ready`, generation, and retirement time for a
retired task (see Retiring asleep tasks).
`priory tasks --lane L` prints only the tasks in lane `L`.
`priory runs` prints each run: id, task, step, tmux session, harness session (a Codex run's thread),
started, ended and how it ended. `priory outbox` prints each prompt: id, task, run, state and text,
with newlines shown as `\n`. `priory asks` and `priory waits` are described under Tools, `priory memory`
under Memory gate, `priory actions` under Code steps, and `priory desk` under Desk. They read
`PRIORY_DB` and print `-` for an empty cell, tab-separated after a header line.
