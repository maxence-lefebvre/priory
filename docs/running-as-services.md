# How to run Priory as services

This guide shows how to keep the intake and the daemon running as systemd user services on Linux,
from a first install to an upgrade. It assumes a factory that `priory check` accepts, a clone of the
repository, and `gh` logged in as the user the services run as.

`contrib/systemd/` holds two user units: `priory-daemon.service` runs `priory daemon` and
`priory-intake.service` runs `priory intake`. Both restart 5 seconds after a failure. They share one
environment file, `~/.config/priory/env`, so the factory folder and state paths are set once. Each
unit starts only once that file exists.

## 1. Install the units

Install the binary where the units look, `~/.cargo/bin/priory`. Then install the units, from your
clone:

```sh
cargo install --path crates/priory
mkdir -p ~/.config/systemd/user
cp contrib/systemd/*.service ~/.config/systemd/user/
systemctl --user daemon-reload
```

If home-manager, a package or a dotfiles tool already links the units into
`~/.config/systemd/user/`, skip the `cp`. Point `ExecStart` at the `priory` it installs, as
[Installing a release](installing-a-release.md) says.

## 2. Pick a state folder

Give the factory a state folder of its own, so its clones, worktrees and databases stay apart from
any other factory on the machine. Create it now: systemd creates no folder.

```sh
mkdir -p ~/.local/state/priory-acme
```

## 3. Write the environment file

`~/.config/priory/env` holds one `NAME=value` per line, with no quotes and no `export`. Systemd
reads each value literally: it expands neither `~` nor `$PATH`. Write every path out in full.

```sh
PRIORY_FACTORY=/home/me/factory
PRIORY_HOME=/home/me/.local/state/priory-acme
PRIORY_DB=/home/me/.local/state/priory-acme/priory.db
HERDR_BUS_DB=/home/me/.local/state/priory-acme/bus.db
PATH=/home/me/.local/bin:/home/me/.cargo/bin:/usr/local/bin:/usr/bin:/bin
PRIORY_TOKEN=<the web view's token>
```

- `PRIORY_HOME` holds the clones and worktrees (see [Priory home](daemon/home.md)). `PRIORY_DB`
  is the state database and `HERDR_BUS_DB` the message bus.
- User services skip your shell profile, so `PATH` is all they get. It must reach `claude`, `git`,
  `gh` and `tmux`. Print the folder of each with `dirname "$(command -v claude)"` and so on, and
  list every one.
- Keep secrets, such as `PRIORY_TOKEN` and the variables your factory's `token_env` and secrets
  name, in this file only. Never put them in a unit or in `factory.toml`.

Each unit has `ConditionPathExists` on this file. Until it exists, a home-manager or package switch
that links the units starts neither the daemon nor the intake, so write it when you are ready for
them to run. Then make it readable by you alone:

```sh
chmod 600 ~/.config/priory/env
```

Export the same `PRIORY_FACTORY`, `PRIORY_HOME`, `PRIORY_DB` and `HERDR_BUS_DB` in your shell
too, so `priory tasks`, `priory inbox` and the other commands read the services' state. Every
command that takes a factory folder reads `PRIORY_FACTORY` when you leave the folder out (see
[the `priory` commands](reference/cli.md)).

Check that the services see the same `PATH`:

```sh
env -i $(grep -v '^PRIORY_TOKEN=' ~/.config/priory/env | xargs) sh -c 'command -v claude git gh tmux'
```

It prints four paths. A missing line names the tool to add to `PATH`.

## 4. Trust the worktrees folder in Claude Code

Claude Code asks whether to trust a folder on its first run there. Priory never answers that
dialog, and which trusted folder covers a worktree depends on the Claude Code version (see
[Trusting the worktrees and the clone](daemon/sessions.md#trusting-the-worktrees-and-the-clone)).
So trust both folders, once. Trust the worktrees folder before the first run:

```sh
mkdir -p ~/.local/state/priory-acme/worktrees
cd ~/.local/state/priory-acme/worktrees && claude   # accept the dialog, then /exit
```

Trust each clone after Priory made it, in step 8.

## 5. Serve the web view over HTTPS

To get the web view on a phone, as an installed app with push notifications, the browser needs
HTTPS. Keep the daemon on loopback and let Tailscale terminate HTTPS in front of it. In
`factory.toml`:

```toml
[serve]
listen = "127.0.0.1:7340"
url = "https://runner.example.ts.net"
```

Then, once:

```sh
tailscale serve --bg 7340
```

[How to use the web view](using-the-web-view.md#3-reach-it-from-a-phone) covers the phone's side:
install the view and turn on notifications.

## 6. Start them

If you copied the units in step 1, enable them, so they start at login and now:

```sh
systemctl --user enable --now priory-intake priory-daemon
```

If a manager linked them, it enabled them too. Start them without `enable`:

```sh
systemctl --user start priory-intake priory-daemon
```

| Action  | Command                                                  |
| ------- | -------------------------------------------------------- |
| Status  | `systemctl --user status priory-intake priory-daemon`    |
| Logs    | `journalctl --user -u priory-daemon -u priory-intake -f` |
| Restart | `systemctl --user restart priory-daemon`                 |
| Stop    | `systemctl --user stop priory-daemon priory-intake`      |

Check: `systemctl --user status` shows both `active (running)`. A unit that shows
`start condition unmet` has no environment file at `~/.config/priory/env`.

## 7. Keep them up after you log out

Without lingering, user services stop when your last session ends. On an always-on machine, run this
once. It changes the machine, so you run it yourself:

```sh
loginctl enable-linger "$USER"
```

Check: `loginctl show-user "$USER" --property=Linger` prints `Linger=yes`.

## 8. Read the first poll's backlog

The intake's first poll of each stream is a baseline: it records the recent events it finds, and
requests no task for them (see [Baseline](intake/github-polling.md#baseline)). On a busy repository,
that is thousands of check runs. Do not read the whole of `priory inbox --json`. Note the last seq
once the first poll ended, then read only what came after it, or search:

```sh
priory inbox | tail -n 1                 # the seq is the first column
priory inbox --since 4210
priory inbox | grep 'check_run' | wc -l
```

Once Priory cloned a repository under `$PRIORY_HOME/repos/`, trust the clone in Claude Code:

```sh
cd ~/.local/state/priory-acme/repos/<owner>/<name> && claude   # accept the dialog, then /exit
```

A task whose run met the dialog is blocked with a reason that names both folders. After you trusted
them, `priory resume TASK` starts it again.

## Upgrade the binary

To let the daemon do this by itself, follow
[How to upgrade Priory automatically](upgrading-automatically.md).

A newer release migrates the state database forward when it opens it. An older release then
refuses it with `state schema N is newer than this priory knows (M)`. So back up both databases
before every upgrade, to roll back:

```sh
systemctl --user stop priory-daemon priory-intake
sqlite3 "$PRIORY_DB" ".backup '$PRIORY_DB.before-upgrade'"
sqlite3 "$HERDR_BUS_DB" ".backup '$HERDR_BUS_DB.before-upgrade'"
cargo install --path crates/priory     # or install the release, as installing-a-release.md says
systemctl --user start priory-intake priory-daemon
```

A live run keeps the `priory mcp` server its session started, on the old binary. That server only
relays calls to the daemon's socket, so it keeps working on a migrated state. A tool call it makes
while the daemon restarts waits up to 30 seconds for the new daemon. A call in flight when the
daemon stops can still fail. A session started on 1.2.0 or earlier keeps a server that opens
the state itself, so install over one only when `priory runs` shows no live run.

Check: `priory --version` prints the new version and `systemctl --user status` shows both units
running. To roll back, stop both units, install the older binary and copy each backup over its
database.

## What a stop does to agents

The daemon unit sets `KillMode=process`. The default, `control-group`, would also kill the tmux server
the daemon started on a stop or restart. tmux stays in the unit's cgroup, so every agent dies with it.
`process` signals only the daemon, so agents keep running and the new daemon reconnects to them.
Systemd logs `Unit process ... (tmux: server) remains running after unit stopped`, which is expected.
`systemctl --user stop` therefore leaves agents running. End one with
`tmux kill-session -t =priory-<task id>` when you mean to.
