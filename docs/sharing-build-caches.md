# How to share build caches between runs

This guide shows how to give a runner's agents a build cache, such as sccache or a Nix binary
cache. It assumes a factory that already runs on the machine, and a daemon started as
[services.md](daemon/services.md) describes.

Each task builds in its own worktree, so each one starts with an empty build folder. A cache saves
that time. Priory does not install, start or configure one: a cache belongs to the machine, as
R11.2 of the [PRD](prd.md) says. You set it up once on each runner, and every run there uses it.

## Where a run gets its environment

A model step's agent runs in a tmux pane, and takes the environment of the tmux server. The daemon
starts that server with its own environment when none runs on its socket. A code step's `run`
command takes the daemon's environment directly. So a variable you set for the daemon reaches every
run on that machine.

Priory adds only its own variables, such as `PRIORY_HOME` and `PRIORY_SESSION`. It adds no cache
variable, and no file in the factory sets one.

## Set what each machine needs

Do these steps on each runner. The factory folder stays the same on every machine.

1. Install the cache tool on the machine, such as `cargo install sccache` or
   `nix profile add nixpkgs#sccache`.

2. Add the cache's variables to the daemon's environment file, `~/.config/priory/env`. For
   sccache:

   ```sh
   RUSTC_WRAPPER=/home/me/.cargo/bin/sccache
   SCCACHE_DIR=/home/me/.cache/sccache
   SCCACHE_CACHE_SIZE=20G
   ```

   Use the absolute path in `RUSTC_WRAPPER`. The service's `PATH` may not reach the tool.

3. For a Nix binary cache, set `substituters` and `trusted-public-keys` in the machine's Nix
   configuration, `/etc/nix/nix.conf` or `~/.config/nix/nix.conf`. Nix fetches from the cache
   itself, so the daemon's environment needs nothing.

4. Restart the daemon so it reads the file:

   ```sh
   systemctl --user restart priory-daemon
   ```

   Runs that started before the restart keep their old environment.

| What | Where you set it | Example |
| ---- | ---------------- | ------- |
| sccache on this machine | `~/.config/priory/env` | `RUSTC_WRAPPER`, `SCCACHE_DIR`, `SCCACHE_CACHE_SIZE` |
| A shared sccache store, such as S3 or Redis | `~/.config/priory/env`, with its credentials | `SCCACHE_BUCKET`, `SCCACHE_REDIS_ENDPOINT` |
| A Nix binary cache | `nix.conf` on the machine | `substituters`, `trusted-public-keys` |
| A cache's credentials | The machine only, never the factory folder | `~/.aws/credentials`, Nix's `netrc-file` |

## Check that runs use it

Look at the tmux server's environment, with the daemon's `TMUX_TMPDIR` if it sets one:

```sh
tmux show-environment -g RUSTC_WRAPPER
```

A server that was running before the daemon started keeps its own environment, and prints
`unknown variable`. Set the variable there for the panes started after it:

```sh
tmux set-environment -g RUSTC_WRAPPER /home/me/.cargo/bin/sccache
```

After a task builds, `sccache --show-stats` shows the cache hits.

## What to expect

- Dependencies hit the cache across tasks. The repository's own crates mostly do not, since each
  worktree has its own path, and Cargo's incremental builds are never cached.
- Every agent on the machine can read the daemon's variables. Give a shared cache a credential that
  can only reach that cache.
- The factory folder is in git and every runner reads it. Keep cache settings and credentials out
  of it.
