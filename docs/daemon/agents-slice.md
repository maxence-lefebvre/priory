# Agents slice

On a host with a systemd user manager, each run gets its own scope inside one user slice,
`priory-agents.slice`. The run's tmux pane runs `systemd-run --user --scope --quiet --collect
--expand-environment=no --slice=priory-agents.slice -- <agent>`. A scope execs the agent in place,
so the pane's process is still the agent. Health and `priory runs` find it as before, and the
agent's children, such as builds and tests, stay in its scope.

`--expand-environment=no` keeps `${NAME}` in the agent's body, prompt and settings as written. A
later systemd expands it in a scope's command by default. systemd 254 added the option, so the
daemon reads `systemd-run --version` and omits it before 254, where no expansion happens.

When the daemon starts, it caps the slice from the runner's `agents` share, 90 % by default:

```sh
systemctl --user set-property --runtime priory.slice CPUWeight=20
systemctl --user set-property --runtime priory-agents.slice CPUQuota=<cpu share × CPUs × 100>% MemoryHigh=<memory share>%
```

- `CPUWeight=20`: a dash in a slice name means nesting, so `priory-agents.slice` sits inside
  `priory.slice`. CPU weights split the CPU between siblings, so the weight goes on `priory.slice`,
  beside the user's `app.slice` (default weight 100). Agents yield the CPU to the user's apps when
  both want it.
- `CPUQuota` caps the agents together at their share of the CPUs, even on an idle machine. The daemon
  counts the machine's online CPUs in `/proc/stat`, so a quota or an affinity on the daemon itself
  never lowers the agents' quota.
- `MemoryHigh` is the memory share of `MemTotal`. Past it, the kernel reclaims the agents' memory and
  throttles them, so they slow down. Priory sets no hard limit such as `MemoryMax`; a machine-wide
  out-of-memory kill can still end an agent.

`--runtime` keeps the limits under `$XDG_RUNTIME_DIR/systemd/user.control/`. They need no
`daemon-reload`, which a drop-in under `~/.config/systemd/user/` would need, and they vanish at
reboot, so each daemon start writes the current `agents` share again. A runner file change takes
effect at the next daemon start. Priory touches no unit but its two slices. `systemctl --user revert
priory.slice priory-agents.slice` removes the limits.

The daemon detects systemd once, by those calls. If one fails, as on a host with no systemd user
manager, the daemon logs `priory daemon: no systemd user manager, so runs start outside
priory-agents.slice and only the room check caps them: <error>`, and starts runs as before.

## Which daemon sets the limits

One daemon sets a slice's limits. Another daemon starts its runs in the slice and leaves the limits
alone. It logs `priory daemon: left the limits of priory-agents.slice alone: <why>`, once per reason,
and its desk's banner says the same for as long as it holds. It leaves them alone when:

- It runs inside the slice itself, as a daemon that an agent starts does. The daemon reads its own
  cgroup in `/proc/self/cgroup`.
- The daemon of another home took the slice and still runs. The first daemon to set the limits writes
  its Priory home to `$XDG_RUNTIME_DIR/priory/<slice>.owner`. Once no daemon holds that home's
  `daemon.lock`, its owner is gone and another daemon takes the slice.
- `XDG_RUNTIME_DIR` is not set, so no owner can be recorded.
- `XDG_RUNTIME_DIR` is not `/run/user/<uid>`, and `DBUS_SESSION_BUS_ADDRESS` names a bus outside it.
  `systemctl` would then reach the real user manager, while the owner file sat in the other folder.

A daemon checks all this again at each sweep. One that was refused sets its limits at the first sweep
that lets it, with no restart.

`PRIORY_SLICE` names another slice, so two Priory daemons on one user manager, such as a test
daemon beside a real one, cap their agents apart. Its weight goes on the slice named by the part
before its first dash, or on the slice itself with no dash.

## Restoring the limits

At each [sweep](sweep.md), the daemon that set the limits reads the slice's `cpu.max` and
`memory.high`. When the CPU quota differs, or the memory limit differs by more than 1 % of
`MemTotal`, it sets its limits again. It logs `another process set priory-agents.slice to
CPUQuota=90% MemoryHigh=90%; the sweep restored its limits`, once per restore. The desk's banner
ends with the last restore and its time for 24 hours, which the daemon keeps in
`$PRIORY_HOME/slice.json`. A slice in which nothing runs has no cgroup to read, so the sweep sets
its limits again and says nothing. Where the kernel has no `memory.high`, it compares the CPU quota alone.

The sweep reverts a `systemctl set-property` on the slice. To change the limits, change the shares
of `agents` in `runners/<host>.toml` (see [room.md](room.md)) and restart the daemon.

A run that shows no `SessionStart` in time in a throttled slice blocks its task with that cause. See
[sessions.md](sessions.md#trusting-the-worktrees-and-the-clone).

Known limits:

- Only Linux with systemd and cgroup v2 gets the slice. The room check is the only cap elsewhere.
- A run started before a daemon restart keeps its scope. The restart writes the slice's limits
  again, so the running agents get the new share too.
- The sweep runs every 10 minutes by default, so a changed limit can hold that long.
- The sweep reads no `CPUWeight`. It sets the weight again with the other limits.
- Throttling is read from the slice's `cpu.stat`. A kernel without its throttling counters names none.
- A daemon that restarts can lose the slice to a refused daemon whose sweep runs in between. It takes
  the slice back once that daemon stops.
- A process that an agent starts through a tmux server outside the slice runs in that server's cgroup.
  A daemon there does not see that an agent started it, so only a running owner refuses it.
- A user manager that stops after the daemon started fails each new run's `systemd-run`. Its pane
  exits and health blocks the task as an unexpected exit.
