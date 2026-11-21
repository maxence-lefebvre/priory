# How to add a second machine

This guide shows how to run one factory on two machines that share one event log. It assumes a
clone of this repository with its [build prerequisites](../GETTING_STARTED.md#build-from-source),
Docker on this machine, and ssh key access to the second machine. Read
[runners.md](daemon/runners.md) first for how the two runners share tasks.

The log is a Postgres database on this machine, bound to `127.0.0.1`. The second machine reaches
it through a reverse ssh tunnel that this machine starts, so nothing listens beyond `127.0.0.1` on
either machine.

## What the second machine needs

- A user account with `git`, `tmux` and `jq`. Priory needs no root.
- A clone of this repository, over https, and its Rust toolchain. The script below uses
  `~/priory-roger/priory`, or the path under the home that `SECOND_MACHINE_CLONE` names, and checks
  out `SECOND_MACHINE_REF`, `origin/main` by default, there. Give it a clone of its own. Without root, install `rustup`
  and `gcc` into the user's Nix profile, `nix profile add nixpkgs#rustup nixpkgs#gcc`, or install
  `rustup` into `~/.rustup`. `rust-toolchain.toml` picks the version on the first build.
- `priory` built with the Postgres log: `cargo build -p priory --features postgres`.
- A runner file named after its hostname, as `hostname` prints it, such as `runners/key-drum.toml`.
- For real runs: the agents' harnesses signed in, such as `claude`, and `gh` signed in. The fake
  run below needs neither.

Copy no token, ssh key or `.env` to it. The only secret it holds is the log's connection string.

## 1. Try it with fake agents

`scripts/second-machine.sh` sets up a scratch factory with one runner per machine, Postgres, the
tunnel and both daemons, with the fake `claude` and `gh` of `crates/priory/tests/fake/`. It prints
each command unless you pass `--run`. Replace `me@second` with your ssh target.

1. Read what it would do:

   ```sh
   scripts/second-machine.sh up me@second
   ```

2. Start it:

   ```sh
   scripts/second-machine.sh --run up me@second
   ```

   It stops when either machine has under 20 GB free, or when `~/priory-second-machine` exists.
   It prints both runner names and the log's port.

3. Ask for a task from this machine:

   ```sh
   scripts/second-machine.sh --run priory me@second local \
     request --workflow work --key demo-1 --repo demo/two-machines "Demo"
   ```

   The first claim on the log wins. This machine's runner usually makes it, since the log is
   local. If the second runner wins, the task runs there at once: ask again with another key. This
   machine's runner file lets its agents take 1% of its memory, so its task stays queued and never
   starts. `priory tasks` shows it with `waits for room`. On a machine with more than 100 GB of
   memory, 1% fits the 1 GB agent: raise `memory` in `agents/worker/agent.md`.

4. Give it to the second machine, named by its hostname:

   ```sh
   scripts/second-machine.sh --run priory me@second local give cli:demo-1 key-drum
   ```

   The second runner runs the fake agent, and the task asks at its `approve` step.
   `scripts/second-machine.sh --run priory me@second remote asks` prints the ask's id and revision.

5. Answer it from this machine:

   ```sh
   scripts/second-machine.sh --run priory me@second local answer key-drum:1 approved --revision 1
   ```

   The task is done on the second machine. A second answer to the same revision is refused.

6. Stop everything and remove both scratch folders and the container:

   ```sh
   scripts/second-machine.sh --run down me@second
   ```

`tunnel-down`, `tunnel-up`, `stop local|remote` and `start local|remote` let you cut the link or
stop a runner on purpose.

## 2. Run your own factory on two machines

1. Add one runner file per machine to the factory, each named by its hostname. Keep the same
   factory folder on both machines, such as one Git repository cloned on each.
2. Set the log in `factory.toml`:

   ```toml
   [log]
   backend = "postgres"
   ```

3. Start Postgres on this machine, bound to loopback, with a generated password:

   ```sh
   docker run -d --name priory-log --env-file postgres.env -p 127.0.0.1:PORT:5432 postgres:17-alpine
   ```

   Pick a random high `PORT`. `postgres.env` holds `POSTGRES_USER`, `POSTGRES_DB` and a
   `POSTGRES_PASSWORD` from `openssl rand -hex 24`. Keep it readable by you only.
4. Start the tunnel from this machine, and check that `ss -ltn` on the second machine shows it on
   `127.0.0.1` only. An sshd with `GatewayPorts yes` binds every address:

   ```sh
   ssh -f -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=2 \
     -R 127.0.0.1:PORT:127.0.0.1:PORT me@second
   ```

5. On each machine, set `PRIORY_LOG_URL` to
   `postgresql://priory:PASSWORD@127.0.0.1:PORT/priory?sslmode=disable` in the daemon's
   environment, from a file only you can read. The tunnel encrypts the link, so the plain
   connection inside it stays on loopback.
6. Start this machine's daemon first, then the second machine's, each with
   `priory daemon <factory folder>`.

## Limits

These come from a run between two real machines; the run's timings are in the pull request that
added this guide.

- Every user of the second machine can reach `127.0.0.1:PORT` there, since loopback is shared.
  Only the password keeps them out, so generate it.
- When the tunnel closes or hangs, the second daemon keeps its started tasks going and shows
  `the log is unreachable since <time>` on its desk, as
  [runners.md](daemon/runners.md#a-runner-cut-off-from-the-log) says. It catches up once the
  tunnel is back: a request it missed stays with the runner that claimed it. A hung tunnel costs
  the daemon the `[log]` `timeout` on each try, at most once a minute.
- A cut longer than 30 minutes ends the second runner's lease, and its never-started tasks move.
- This machine holds the log. When it is down, both runners stop.
- A runner's tasks move only 30 minutes after its last alive message, and only those that never
  started. Use `priory give` to move one sooner.
