# Agents slice

On a host with a systemd user manager, every run starts in its own scope inside one user slice,
`priory-agents.slice`. The run's tmux pane runs `systemd-run --user --scope --quiet --collect
--expand-environment=no --slice=priory-agents.slice -- <agent>`. A scope execs the agent in place, so
the pane's process is still the agent. Health and `priory runs` find it as before, and everything the
agent starts, such as builds and tests, stays in its scope.

`--expand-environment=no` keeps `${NAME}` in the agent's body, prompt and settings as written: a
later systemd expands it in a scope's command by default. systemd 254 brought the option, so the
daemon reads `systemd-run --version` and leaves it out before 254, where no expansion happens.

When the daemon starts, it caps the slice from the runner's `agents` share, 90 % by default:

```sh
systemctl --user set-property --runtime priory.slice CPUWeight=20
systemctl --user set-property --runtime priory-agents.slice CPUQuota=<cpu share × CPUs × 100>% MemoryHigh=<memory share>%
```

- `CPUWeight=20`: a dash in a slice name means nesting, so `priory-agents.slice` sits inside
  `priory.slice`, and CPU weights split the CPU between siblings. The weight therefore goes on
  `priory.slice`, beside the user's `app.slice` and its default weight of 100, so agents yield the
  CPU to the user's apps when both want it.
- `CPUQuota` caps the agents together at their share of the CPUs, even on an idle machine.
- `MemoryHigh` is the memory share of `MemTotal`. Past it, the kernel reclaims the agents' memory and
  throttles them, so they slow down. Priory sets no hard limit such as `MemoryMax`; a machine-wide
  out-of-memory kill can still end an agent.

`--runtime` keeps the limits under `$XDG_RUNTIME_DIR/systemd/user.control/`. They need no
`daemon-reload`, which a drop-in under `~/.config/systemd/user/` would, and they vanish at reboot,
so each daemon start writes the current `agents` share again. A runner file change takes effect at
the next daemon start. Priory touches no unit but its two slices. `systemctl --user revert
priory.slice priory-agents.slice` removes the limits.

The daemon detects systemd once, by those calls. When one fails, as on a host with no systemd user
manager, the daemon logs `priory daemon: no systemd user manager, so runs start outside
priory-agents.slice and only the room check caps them: <error>`, and starts runs as before.

`PRIORY_SLICE` names another slice, so that two Priory daemons on one user manager, such as a test
daemon beside a real one, cap their agents apart. Its weight goes on the slice named by the part
before its first dash, or on the slice itself when it has no dash.

Known limits:

- Only Linux with systemd and cgroup v2 gets the slice. The room check is the only cap elsewhere.
- A run started before a daemon restart keeps its scope, and the restart writes the slice's limits
  again, so the running agents get the new share too.
- A user manager that stops after the daemon started fails each new run's `systemd-run`. Its pane
  exits and health blocks the task as an unexpected exit.
