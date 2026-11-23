# How to jail a workflow's agents

This guide shows how to start a workflow's agents in a bubblewrap jail, so they see their worktree
and the paths you name, and not the rest of your files or the daemon's variables. It assumes a
factory that already runs on a Linux machine. [sandboxes.md](daemon/sandboxes.md) explains what a
jailed run sees and keeps.

## Install bubblewrap on each runner

1. Install the `bubblewrap` package, which provides `bwrap`:

   ```sh
   sudo apt-get install bubblewrap     # Debian and Ubuntu
   sudo dnf install bubblewrap         # Fedora
   nix profile add nixpkgs#bubblewrap  # Nix
   ```

2. Check that it can start a jail as your user:

   ```sh
   bwrap --ro-bind / / true && echo ok
   ```

   If it prints `setting up uid map: Permission denied` or a similar error, the distribution
   restricts unprivileged user namespaces. On Ubuntu 24.04 and later, AppArmor does: the
   `bubblewrap` package's AppArmor profile allows `/usr/bin/bwrap`, so use that binary rather than
   one from elsewhere. On other systems, check that `sysctl kernel.unprivileged_userns_clone` is 1.

A runner without bubblewrap still runs every task of an unjailed workflow. It blocks each jailed
task, and you can give one to another runner with `priory give`.

## Declare a sandbox

1. Add a sandbox to `factory.toml`. Name each folder the agents need beyond the worktree, and each
   variable of the daemon they keep:

   ```toml
   [sandboxes.jail]
   kind = "bwrap"
   read = ["~/.cargo", "~/.rustup", "~/.gitconfig"]
   write = ["~/.cache/sccache"]
   env = ["RUSTC_WRAPPER", "SCCACHE_DIR"]
   ```

   Your home is empty in the jail, so a tool installed under it needs its folder in `read`, and a
   commit needs `~/.gitconfig` for your name and email. A build cache goes in `write`. The keys are
   in [`[sandboxes.<name>]`](reference/factory.md#sandboxesname).

2. Turn it on in the workflow:

   ```toml
   # workflows/implement.toml
   sandbox = "jail"
   ```

3. Check the factory:

   ```sh
   priory check
   priory check --explain
   ```

   `priory check` refuses a jailed workflow on a machine without `bwrap`. `--explain` ends with a
   table of each workflow, its sandbox and its binds.

4. Restart the daemon, which checks once at start that it can jail.

## Check a run

Once a task of the workflow runs, `priory runs` shows `jail` in the run's last column. Watch the
agent with `priory attach` or `priory screen` as for any run.

If the task blocks with `this runner cannot start bubblewrap`, fix the error it names, restart the
daemon and resume the task with `priory resume <task>`.

If the agent fails on a missing file or command, add its folder to `read` or `write`, or its
variable to `env`, and resume the task.
