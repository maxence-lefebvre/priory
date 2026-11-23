# Sandboxes

This page explains how a run could start its agent inside a sandbox, so the agent sees the task's
worktree and the few files it needs, and not the rest of the person's machine. It compares five
ways to build the sandbox, then holds the model, the first code slice with its tests, the slices
after it, and the open questions with a recommendation for each.

The first slice is built: see [sandboxes.md](../daemon/sandboxes.md). Max approved bubblewrap
(LGPL-2.1) as a runtime requirement on 2026-11-23, for factories that turn a sandbox on only.
Leads and investigators of a jailed task start in its jail too. The later slices are not built.

The idea is Max's, from 2026-11-22: run a task's agent in a sandbox instead of a git worktree on
the host. This note keeps the worktree and puts the sandbox around it. The comparison below shows
why: every sandbox that replaces the worktree also loses attach, the screen, the sweep or the
evidence, and the first slice should lose none of them.

[Mastra Factory](https://github.com/mastra-ai/mastra/blob/main/mastracode/factory/README.md) runs
its agents in sandboxes and measures them. Its metrics page,
[Configure: metrics](https://factory.mastra.ai/configure/metrics), has a Sandboxes section. This
note could not open that page from its session, so it read the Factory README, which points to it.
Mastra measures, per sandbox provider: how long a sandbox takes to be ready, including its
repository setup; how many are starting and how long the oldest start has waited; start failures
by cause; sandboxes recreated and retired; the time of each setup phase and command; and how many
are active. For sessions, it measures the time from a message to the model's first output. A pull
request merged on 2026-11-18 renamed its session start metrics to sandbox ones
([mastra#30862](https://github.com/mastra-ai/mastra/pull/30862)). Later slice 5 takes the idea of
measuring a sandbox's start and failures, in Priory's own trace and words.

## What existed when this note was written

- A run starts with `tmux new-session -d -c <worktree>`, with `PRIORY_HOME`, `PRIORY_SESSION`,
  `XDG_RUNTIME_DIR` and the run's extra variables (`crates/priory-runtime/src/runtime.rs:869`,
  `:880`, `:899`). The pane's command is a prefix, `launch.scope`, then `env -u` of twelve
  variables, then the harness (`runtime.rs:922`). The prefix is the agents slice's
  `systemd-run --user --scope ...` when the host has a systemd user manager
  (`crates/priory-core/src/daemon.rs:221`, [agents-slice.md](../daemon/agents-slice.md)). So a
  wrapper that execs the agent in place already has a slot.
- The worktree is `$PRIORY_HOME/worktrees/<owner>/<name>/<task id>`, added from Priory's clone
  under `repos/` ([home.md](../daemon/home.md#worktrees)). A commit in the worktree writes objects
  and refs into the clone's git folder, so a run must be able to write the clone too.
- Hooks run `priory hook <event>`, which writes one line to the daemon's socket at
  `$XDG_RUNTIME_DIR/priory/<hash>.sock`, or appends it to `$PRIORY_HOME/signals.spool` when no
  daemon listens ([signals.md](../daemon/signals.md#how-a-hook-reaches-the-daemon)). The tool guard
  is a `PreToolUse` hook that asks the daemon over the same socket and denies when it cannot
  ([guard.md](../daemon/guard.md#the-hook-and-its-deadline)). The `priory` MCP server, `priory mcp
  --session <id>`, sends each tool call over it too ([tools.md](../daemon/tools.md#the-mcp-server)).
- A hook line carries the transcript's path, and the daemon reads that file on the host for tokens,
  budgets and compaction (`daemon.rs:2408`, [trace.md](../daemon/trace.md)). So a path a hook sends
  must mean the same file to the daemon.
- The MCP configuration is a file under `$PRIORY_HOME/prompts/`, readable by its owner only. It
  holds the agent's `mcpServers`, copied from the person's Claude config, tokens included
  ([runs.md](../daemon/runs.md)).
- Claude asks once per folder whether to trust it, and Priory never answers
  ([sessions.md](../daemon/sessions.md#trusting-the-worktrees-and-the-clone)). The answer lives in
  the person's Claude config, by path.
- `priory attach` and `priory screen` talk to the run's tmux session on the host
  ([sessions.md](../daemon/sessions.md#attaching-to-a-run)). Killing that session ends the run.
- The sweep keeps a worktree while "a process runs in it", read from `/proc` on the host, and runs
  the repository's `teardown` recipe before it removes one ([sweep.md](../daemon/sweep.md)).
  Recipes run on the host, in the worktree ([runs.md](../daemon/runs.md#recipes)).
- A step's `record` command runs on the host in a detached checkout of the result's commit
  ([evidence.md](../daemon/evidence.md#a-steps-record-command)). `open-pr` pushes from the daemon
  with its own `gh`, so an agent never needs a token that can push for that step.
- A run can read every variable of the daemon and every file of the person's user: `~/.ssh`, the
  `gh` login, cloud credentials. The PRD lists the risk, "Agents run with the user's credentials"
  ([prd.md](../prd.md#risks)). The guard says it "is not a sandbox", and lists what it does not see
  ([guard.md](../daemon/guard.md#how-a-command-is-read)).
- [agent-credentials.md](agent-credentials.md) scopes the variables a run gets. Its later slice 2
  asks for the wall that hides `~/.ssh` and the `gh` login, and a network policy. Its open
  question 2 asks for a second design note after a spike. This is that note, without the spike:
  the first slice below starts with it.
- A Codex run already maps `defaultMode` to Codex's own `sandbox_mode`
  ([codex.md](../daemon/codex.md)).
- Each runner file declares `platform.os`, `linux`, `macos` or `windows`
  ([runners.md](../reference/runners.md)).

## Five ways to build the sandbox

The evidence below comes from each project's own pages. Where this note's session could not open a
page, the claim is marked *unread*: it comes from a search summary of that page, and the first
slice that relies on it checks it first.

### The harness's own sandbox

Claude Code can run its Bash commands in a sandbox
([Claude Code sandboxing](https://code.claude.com/docs/en/sandboxing)). It uses Seatbelt on macOS,
and bubblewrap with `socat` on Linux. Writes are limited to the working directory, a temporary
folder and paths a person adds. Reads are allowed almost everywhere unless denied. Network traffic
goes through a proxy on the host that checks each host name against `allowedDomains`, which starts
empty. The page names its limits: only TCP is covered, TLS is not inspected, and Docker does not
work inside it. It is built on Anthropic's
[sandbox-runtime](https://github.com/anthropic-experimental/sandbox-runtime), Apache-2.0, labelled a
research preview.

- **Start time:** none to speak of: the harness starts as today, and each command starts in its
  own sandbox.
- **What the agent keeps:** everything. Claude itself, its file tools, its MCP servers and its
  hooks run outside the sandbox. Only Bash commands are inside.
- **Network and secrets per workflow:** a network allowlist per run, through `--settings`, which
  Priory already writes. Reads must be denied path by path, so `~/.ssh` is hidden only if the
  factory names it.
- **Guard and hooks:** unchanged, since hooks run outside.
- **Cost and license:** free; part of the harness.
- **What breaks:** nothing in the worktree model. What it leaves open is the point of this note:
  Claude's own `Read` tool, its MCP servers and the processes they start are not inside. Codex has
  its own sandbox with other settings, so a factory of both harnesses gets two policies.

### A namespace jail: bubblewrap

[bubblewrap](https://github.com/containers/bubblewrap) is "a low-level unprivileged sandboxing
tool", the one Flatpak uses. It builds a new mount namespace on a tmpfs root, with unprivileged
user namespaces; its setuid mode was removed. Its
[man page](https://github.com/containers/bubblewrap/blob/main/bwrap.xml) gives `--bind` and
`--ro-bind` for host paths, `--unshare-net` for a new network namespace, `--die-with-parent` and
`--new-session`. It is Linux only, since it relies on Linux namespaces. Its repository shows
LGPL-2.1. Landlock is a kernel feature that lets a process restrict its own file and port access
([kernel docs](https://docs.kernel.org/userspace-api/landlock.html), *unread*); it is a possible
second layer, not needed for a first slice.

- **Start time:** bubblewrap's pages state none. It runs no image and no daemon: it makes
  namespaces and mounts, then execs. The first slice records the time from launch to
  `SessionStart` for each run, jailed or not, so the cost is measured, not assumed.
- **What the agent keeps:** whatever is bound in. With every bind at the same path inside as
  outside, the agent keeps its Claude login (`~/.claude` and `~/.claude.json`), the person's MCP
  servers and their programs under `/usr` or `/nix`, and its trusted folders. It loses the `gh`
  login, `~/.ssh` and every other file of the home that is not bound.
- **Network and secrets per workflow:** files per workflow, by its binds. The network is shared
  or cut whole: `--unshare-net` leaves no route to the model's API. sandbox-runtime shows the way
  between: cut the network, and bind in the unix sockets of a host proxy that allows listed hosts
  only. Later slice 1 takes that.
- **Guard and hooks:** the socket's folder and the spool are bound in at their paths. The hook,
  the guard and the MCP bridge work as today, with no code change.
- **Cost and license:** free. One package on each runner: `bubblewrap`, LGPL-2.1, a program
  Priory runs as it runs `tmux` and `systemd-run`, linked into nothing.
- **What breaks:** nothing the same-path rule keeps. The pane runs `bwrap ... claude`, so tmux
  owns it on the host: attach and the screen are unchanged. `--die-with-parent` ends the jail
  when the pane dies, so killing the tmux session still ends the run. Recipes and `record` stay on
  the host. Two things need a test: that the sweep's `/proc` check still sees a jailed process's
  folder, and that the agents slice still holds a jail started inside its scope.

### A rootless container: Podman or Docker

Rootless Docker needs `newuidmap`, `newgidmap` and at least 65,536 subordinate ids
([Docker rootless](https://github.com/docker/docs/blob/main/content/manuals/engine/security/rootless/_index.md)),
and its CPU and memory limits need cgroup v2 and systemd
([tips](https://github.com/docker/docs/blob/main/content/manuals/engine/security/rootless/tips.md)).
Rootless Podman needs the same ids and a user-mode network tool, pasta by default
([Podman rootless tutorial](https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md)).
`--userns=keep-id` maps the person's uid to the same uid inside, so files in a bound worktree keep
their owner ([userns](https://github.com/containers/podman/blob/main/docs/source/markdown/options/userns.container.md)).
`--network none` leaves only a loopback device
([Docker](https://github.com/docker/docs/blob/main/content/manuals/engine/network/drivers/none.md),
[Podman](https://github.com/containers/podman/blob/main/docs/source/markdown/options/network.md)).
On macOS, Podman "requires a virtual machine"
([podman-machine](https://github.com/containers/podman/blob/main/docs/source/markdown/podman-machine.1.md)),
and so does Docker Desktop (*unread*). Podman is Apache-2.0 and needs no daemon or setuid binary
([podman](https://github.com/containers/podman)). The Docker Engine's upstream, Moby, is Apache 2.0.
Docker Desktop is free for small businesses and paid for larger ones
([license](https://docs.docker.com/subscription/desktop-license/), *unread*).

- **Start time:** neither project states one.
- **What the agent keeps:** what the image holds and what is bound in. Claude, `gh`, `node`,
  every MCP server's program and the repository's toolchain must be in the image, so each
  factory maintains an image per repository. The login is bound in as with bubblewrap.
- **Network and secrets per workflow:** a container per run gives `--network none` or a
  network of its own, and variables by name. A host allowlist still needs a proxy.
- **Guard and hooks:** on Linux, the socket folder bound in at its path works. On macOS the
  container is in a VM, and a host unix socket does not cross into it; Docker Desktop special-cases
  its own socket only (*unread*). So on macOS the hooks would need a TCP bridge.
- **Cost and license:** free on Linux with Podman. Docker Desktop costs a large company money.
- **What breaks:** the pane runs `podman run -it`. tmux still owns that client, so attach and the
  screen work, but killing the session kills the client and maybe not the container: the kill
  path must remove the container by name. The same-path rule must hold for the transcript and the
  sweep, or each breaks. Podman's own cgroup handling may place the container outside
  `priory-agents.slice`. In return a container can enforce the agent's `memory` as a hard limit,
  which the room today only promises.

### A microVM: Firecracker

Firecracker boots a guest to `/sbin/init` in 125 ms or less, with a minimal kernel and the serial
console off, and adds 5 MiB or less of memory for a 1-vCPU guest
([specification](https://github.com/firecracker-microvm/firecracker/blob/main/SPECIFICATION.md)).
It needs KVM, can give the guest a vsock device, and ships a jailer
([firecracker](https://github.com/firecracker-microvm/firecracker)). Apache 2.0. Its tested hosts
are bare-metal EC2 machines. With no KVM on macOS, it is Linux only. Kata Containers wraps such VMs
as containers ([kata](https://github.com/kata-containers/kata-containers), Apache 2.0). gVisor runs
containers on a user-space kernel, `runsc` ([gvisor](https://github.com/google/gvisor), Apache-2.0).

- **Start time:** the fastest kernel boot of the five, before a root file system that holds
  Claude, Node and the toolchain is built and mounted.
- **What the agent keeps:** nothing by default. There is no shared home: the login, the MCP
  configuration and the worktree must be copied in or served over a virtual disk.
- **Network and secrets per workflow:** the strongest wall: a tap device per VM and nothing else.
- **Guard and hooks:** a unix socket does not cross a VM. A relay in the guest would carry hook
  lines over vsock to the host socket, and the transcript would have to come back the same way.
- **Cost and license:** free software, on a host with `/dev/kvm`. A laptop VM or a cloud VM without
  nested virtualisation has none.
- **What breaks:** the worktree model. The worktree lives in the guest, so the sweep's checks,
  the overlap hold's diff, `record` and `open-pr` need its commits copied back first. The screen
  needs a console into the guest.

### A hosted sandbox service

[e2b](https://github.com/e2b-dev/infra) runs each sandbox as a Firecracker microVM, and its
infrastructure is Apache-2.0 and can be hosted on one KVM machine. Its pages give per-second
prices per vCPU and per GiB, pause and resume, and egress rules (*unread*:
[pricing](https://e2b.dev/pricing), [network](https://e2b.dev/docs/network/internet-access)).
[Daytona](https://github.com/daytonaio/daytona) claims sandboxes that start "in under 90ms", is
AGPL-3.0, and its README says since June 2026 that the repository is no longer maintained.
[Modal](https://modal.com/docs/guide/sandbox-networking) builds its sandboxes on gVisor, can block
all networking or allow listed address ranges, and passes secrets as variables or injects them into
HTTPS headers from outside the sandbox (*unread*; its license is closed).

- **Start time:** sub-second by each vendor's claim.
- **What the agent keeps:** nothing on the person's machine. The Claude login must travel to the
  vendor, which puts a credential the person signed in with on a third party's machine.
- **Network and secrets per workflow:** the best tools of the five: egress rules and secrets per
  sandbox, set by API.
- **Guard and hooks:** the daemon is on a laptop that binds only 127.0.0.1. A hook in the vendor's
  machine cannot reach it, so every hook and tool call would need a tunnel, or the daemon would have
  to poll the sandbox. Both change the PRD's architecture, "Agents outlive the daemon. Their MCP
  tools reach it through a stdio bridge on a local socket".
- **Cost and license:** a bill per second. As a scale, at e2b's listed rates (*unread*), a 2-vCPU,
  4-GiB sandbox for an hour costs about $0.17, before the model's tokens. The PRD's non-goals
  exclude "a hosted service or billing" for Priory itself, not a factory's use of one.
- **What breaks:** everything the microVM breaks, plus offline: Principle 5 says a machine that goes
  away loses nothing, and a laptop that sleeps loses its tunnel. [cloud-runs.md](cloud-runs.md)
  already chose, for work that runs elsewhere, to give up the hooks and judge only the pull request.
  A hosted sandbox would be that model again, with a vendor in place of Anthropic.

### Side by side

| | Harness's own | bubblewrap | Rootless container | Firecracker | Hosted |
| - | - | - | - | - | - |
| Start time | none added | not stated; measured by the first slice | not stated | 125 ms boot, plus the root file system | sub-second, by claim |
| Hides `~/.ssh` and `gh` login | only paths named | yes, by default | yes | yes | yes |
| Covers Claude's own tools and MCP servers | no | yes | yes | yes | yes |
| Network allowlist | built in | with a host proxy | with a host proxy | own tap device | built in |
| Hooks reach the daemon | unchanged | bind the socket | Linux only | relay over vsock | tunnel |
| Attach, screen, sweep, evidence | unchanged | unchanged | kill path and paths to watch | copy back | copy back |
| macOS | yes | no | through a VM | no | yes |
| License | the harness's | LGPL-2.1 | Apache 2.0 | Apache 2.0 | vendor's |
| Cost | none | none | none, or Docker Desktop | a KVM host | per second |

## The model

### A sandbox has a name, and a workflow turns it on

`factory.toml` declares each sandbox:

```toml
[sandboxes.jail]
kind = "bwrap"
read = ["~/.cargo", "~/.rustup"]          # bound read-only, at the same path
write = ["~/.cache/sccache"]              # bound read-write, at the same path
env = ["RUSTC_WRAPPER", "SCCACHE_DIR"]    # the daemon's variables the run keeps
```

A workflow turns it on for every model step it runs, debate steps included:

```toml
# workflows/implement.toml
sandbox = "jail"
```

A person turns it on per workflow because a workflow is where the factory already states what work
may do: its steps' effects, its `guard.allow`. A review workflow can be jailed first, while an
implementer that needs a wide toolchain is not. A shared agent's runs follow the workflow of the
task they serve, so one conversation can run jailed for one task and not for another.

Code steps, recipes, `record` commands and a human step's `command` stay on the host. They are
the factory's own commands, approved by the factory's pull request
([runs.md](../daemon/runs.md#recipes)), and never the agent's.

### What a jailed run sees

The rule that keeps everything else working: every path is bound at the same path inside as
outside. A path a hook sends, a path a person trusted and a path the sweep reads in `/proc` then
mean the same file on both sides.

| Path | Inside the jail |
| ---- | --------------- |
| `/` except `/home`, `/root`, `/tmp` and `/run/user` | read-only |
| `/tmp` | a new tmpfs |
| `$HOME` | a new tmpfs, with only the binds below |
| the task's worktree | read-write |
| the clone's git folder under `repos/` | read-write, since a commit writes there |
| `~/.claude` and `~/.claude.json`, or the harness's own home for Codex | read-write: the login, sessions, transcripts and trusted folders. A later change made the clone's config and hooks and Claude's home read-only: see [sandboxes.md](../daemon/sandboxes.md#what-a-jailed-run-sees) |
| the run's MCP configuration file and Priory's binary | read-only |
| the daemon's socket folder and `signals.spool` | read-write |
| the sandbox's `read` and `write` paths | as declared |

The jail adds `--die-with-parent`, `--new-session` and a new PID namespace, and shares the network.
The run's variables start from the short list of
[agent-credentials.md](agent-credentials.md#a-reviewer-token-that-cannot-push), `PATH`, `LANG`,
`TERM`, `HOME`, Priory's own variables and the sandbox's `env`, plus the agent's declared secrets
once that note's slice exists. So a jailed run holds neither the person's files nor the daemon's
variables, only what the factory named.

The pane's command becomes the agents slice's prefix, then `bwrap` with those arguments, then the
harness as today. The scope comes first, so the jail and everything it starts stay in
`priory-agents.slice`.

### When the runner cannot jail

The daemon checks once at start that `bwrap --ro-bind / / true` exits 0, as it checks
`systemd-run` for the slice. A jailed task on a runner where it fails never starts unjailed. Its
task blocks with `workflow implement needs sandbox jail, and this runner cannot start bubblewrap:
<stderr>`, and the desk shows it. On a macOS runner the reason says `bubblewrap needs Linux`. A
person can install the package and resume, give the task to another runner with `priory give`, or
remove `sandbox` from the workflow.

`priory check` refuses, with file and line:

- a workflow's `sandbox` that `[sandboxes]` does not declare;
- a `kind` other than `bwrap`;
- a `read` or `write` path that is not absolute or under `~`, or that is `~` itself;
- an `env` name that the factory's `secret_envs` holds, such as the server's token variable.

`priory check --explain` lists each workflow with its sandbox and its binds.

### What the jail does not stop

The claim is exact: a jailed run reads and writes only the worktree, the clone, the harness's own
home and the paths the factory names, and holds only the variables it names. It does not claim:

- **The network.** The run reaches any host. It cannot read the person's keys, but it can send out
  what it can read: the worktree and the Claude login. Later slice 1 closes this.
- **The socket.** The daemon's socket is bound in, and a line written to it with `nc` is still a
  call no guard reads, as [guard.md](../daemon/guard.md#how-a-command-is-read) says. Later slice 2
  closes this.
- **The clone.** The run can write other tasks' refs in the shared clone. Open question 3. Its
  config, hooks and worktree links are read-only, so the daemon's git on the host runs nothing
  the run wrote.
- **The Claude login.** The run can read its own harness's credentials, which every option but a
  proxy that injects them shares.

## The first slice: a bubblewrap jail per workflow

This slice is built. It differs from the plan below in four ways:

- A jailed run's hooks spool to their own `spools/<session>/signals.spool`, which `PRIORY_SPOOL`
  names, since a bound `signals.spool` would lose its lines once a daemon renamed it away.
- The variables are cleared by a shell inside the jail, so the variables a run keeps take the
  pane's values.
- `priory check` refuses a jailed workflow on a machine without `bwrap`, at Max's request. The
  daemon still blocks each jailed task instead, so a runner without bubblewrap runs the others.
- CI does not install bubblewrap. The tests that need it are ignored by default and fail, not
  skip, when it is missing; the other tests use a fake `bwrap`.

1. `[sandboxes.<name>]` in `factory.toml` and `sandbox` on a workflow, in the schema and the JSON
   Schemas, and the check rules above.
2. The jail's arguments, built in `priory-runtime` beside the slice's scope, and passed as part of
   the prefix of `launch`.
3. The short list of variables for a jailed run.
4. The start check and the blocked task on a runner that cannot jail.
5. `priory runs` shows `jail` for a jailed run. Each run's `factory.step.ended` record
   ([trace-schema.md](../trace-schema.md#factorystepended)) gains `sandbox`, and the time from
   launch to `SessionStart`, jailed or not.
6. A page `docs/daemon/sandboxes.md`, its link in the daemon's README, the keys in
   [factory.md](../reference/factory.md) and [workflows.md](../reference/workflows.md), and a
   how-to guide, "How to jail a workflow's agents", with the package to install.

Tests use the existing `Setup` and fake `claude`, through the CLI, and wait on state, never a fixed
sleep. CI installs `bubblewrap`. A test that needs it fails, and does not skip, when it is missing.

- A run of a jailed workflow sends `SessionStart`, its tool calls reach the daemon, and its result
  moves the task, as an unjailed run does.
- The fake `claude` of a jailed run cannot read a file the test put under `$HOME/.ssh`, can write
  in its worktree, can commit, and cannot write next to the worktree.
- It does not see a variable of the daemon that the sandbox's `env` does not name, nor the server's
  token variable, and sees one it names.
- The transcript path its `Stop` hook sends is read by the daemon, and the task's tokens count.
- A guarded call in a jailed run is denied with the guard's reason and leaves a
  `factory.tool.denied` record. With the daemon stopped, a hook's line lands in the spool and the
  next daemon applies it.
- `priory screen` shows the jailed run's pane. Killing the run's tmux session leaves no process
  of the jail.
- While a jailed process sits in the worktree, the sweep keeps it with `a process runs in it`.
- A run of a workflow without `sandbox` gets the same arguments as before this slice.
- With `bwrap` replaced on `PATH` by a script that exits 1, the jailed task blocks with the reason
  above, no unjailed run starts, and a task of another workflow runs.
- Each check rule fails with its line.
- `priory check` passes on every starter in `examples/`.

## Later slices

1. **A network allowlist.** A sandbox gains `network = ["api.anthropic.com", "github.com"]`. The
   jail cuts the network with `--unshare-net` and binds in the unix sockets of a proxy the daemon
   runs on the host, which allows those hosts only, as
   [sandbox-runtime](https://github.com/anthropic-experimental/sandbox-runtime) does. Each refused
   host leaves a trace record. This is the network half of
   [agent-credentials.md](agent-credentials.md)'s later slice 2.
2. **A socket per run.** The daemon binds one socket per jailed run, which takes only that
   session's hooks and tool calls, and binds it in in place of the shared one. A line that names
   another session, or a person's verb, is refused there.
3. **The harness's own sandbox, on macOS.** A sandbox with `kind = "harness"` writes the harness's
   sandbox settings: Claude's `sandbox` settings with the same paths and hosts, and Codex's
   `sandbox_mode`. It covers only commands, and the how-to guide says so, but it is the one kind
   that works on a macOS runner.
4. **A container kind.** `kind = "podman"` with an `image`, for a repository whose toolchain the
   host lacks, with `--userns=keep-id`, the same-path binds, a container named after the run that
   the kill path removes, and the agent's `memory` as a hard limit.
5. **Sandbox measures.** `priory report` gains each sandbox's start times, its failures to start
   by reason, and its jailed runs that ended `failed` or `blocked` beside unjailed ones, so a
   person sees what a jail costs.
6. **A sandbox for recipes and `run` actions**, once a factory asks to jail its own commands.

Firecracker and hosted services stay out of this list. Each replaces the worktree, and with it the
hooks, the screen and the sweep. A factory that wants a run elsewhere has
[cloud-runs.md](cloud-runs.md), which already says what Priory gives up for it.

## Open questions

1. **bubblewrap, the harness's own sandbox, or both first?** The harness's sandbox costs no package
   and works on macOS, but leaves Claude's file tools and MCP servers outside, and changes with
   each harness version, the PRD's "Harness changes" risk. Recommendation: bubblewrap first, since
   it is the only option that makes the credentials note's wall true for every process of the run.
   The harness's sandbox is later slice 3. bubblewrap is a new runtime requirement, not a crate:
   LGPL-2.1, maintained under the `containers` organisation. Confirm its last release before the
   slice, and approve it as a dependency.
2. **Do user namespaces work where runners run?** Some distributions restrict unprivileged user
   namespaces. Recommendation: the slice's first commit runs `bwrap --ro-bind / / true` on the
   author's runners and on CI, and writes the answer here. Answer: it passed in the container
   that built the slice, bubblewrap 0.9.0 on Ubuntu, run as root. It was not run on Max's runners
   or on CI, which does not install bubblewrap. Where it fails, the block reason names
   the distribution's setting, and the how-to guide says how to allow bubblewrap.
3. **Should a jailed run get its own clone?** The shared clone lets a run write other tasks' refs.
   A clone per task costs disk and a fetch. Recommendation: not in the first slice. Measure whether
   any jailed run touches a ref outside its branch, with the reflog, before paying for it.
4. **Must the binds be at the same path?** A container's `/workspace` reads better. Recommendation:
   yes, always. The transcript path, Claude's trusted folders, the sweep's `/proc` check and the
   screen's paths all assume it, and a rule with no exception is one nobody forgets.
5. **Should a jailed run keep Claude's whole home?** `~/.claude` holds every session's transcript,
   so a jailed run can read other tasks' conversations. Recommendation: yes in the first slice,
   since resume and trust need it. Revisit with a `CLAUDE_CONFIG_DIR` per factory if a factory's
   tasks must not see each other.
6. **Is this the second adapter of the runtime boundary?** No: a jail is a prefix, as the slice is,
   and the harness trait is untouched. Recommendation: no adapter trait until a container kind
   brings a second way to start and stop the agent.
7. **Which claims did this note not read?** The pages of e2b, Modal, Daytona's pricing, Docker
   Desktop's license and Mastra's metrics page could not be opened from the session that wrote it.
   The recommendation does not rest on any of them. Recommendation: read them before a later slice
   cites one.
