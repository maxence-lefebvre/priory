# The daemon

`priory daemon [DIR] [--once]` reads the herdr-bus output `factory.intake` as the consumer `priory`,
records each message in the inbox and routes it to a task. It checks the factory first, as
`priory check` does, and refuses to start while it has problems. `--once` routes what is pending
and exits; it starts no runs, since no daemon would stay to hear their hooks. Without it, the
daemon looks for new messages every second and runs agents, as described under Runs.

State is one SQLite file, named by `PRIORY_DB`. The daemon refuses to start without it, or with an
empty name, `:memory:` or a `file:` URI, which SQLite may not keep on disk.

## One message at a time

Each message is handled in one SQLite transaction: its inbox entry, any task it creates, and the
entry's outcome. The daemon acks the message on the bus only after that transaction commits. A
message delivered again, after a crash between the commit and the ack, adds nothing: the inbox
drops a message id it already holds. An error stops the daemon before the ack, so the next start
handles that message again.

## Routing

A task owns resources, stored in one table: its trigger key from its creation, its branch
`github:{repo}@priory/{id}` from its first run, and each pull request `github:{repo}#{n}` a wait of
its names. A message names resources too: a task request its `key`, a CLI request its subject, a
GitHub event its issue or pull request `github:{repo}#{n}` and, for a pull request event, its head
branch `github:{repo}@{head_branch}`. The oldest active task that owns any of them decides the route.
A task is active in any state except done, failed and dropped. So an agent that opens a pull request
from its branch gets that pull request's events, with no trigger involved.

| Message                                   | Outcome   | Task                                         |
| ----------------------------------------- | --------- | -------------------------------------------- |
| A schedule's `factory.task.requested`, with a task asleep at a model step owning its key | `woke` | that task, now `ready` |
| A schedule's `factory.task.requested`, with a task in another active state owning its key | `coalesced` | that task, unchanged |
| Any other, with an active task owning a resource it names | `kept` | that task                   |
| `factory.task.requested`, no active task  | `created` | a new task, `queued` at its workflow's start |
| `factory.task.requested`, malformed       | `ignored` | none                                         |
| Any other                                 | `ignored` | none                                         |

A task request for a workflow the factory lacks stops the daemon with an error, before its ack. The
request waits on the bus until the workflow is back. A malformed request is ignored instead, so that
it cannot block the output.

A source event comes before its task request, so it is recorded as `ignored`. The task keeps its
id in `source`.

A schedule's task request carries `scheduled` (see `docs/intake.md`). Only a schedule with a fixed
`key` sends a second one for the same key. One that finds the task asleep at a model step wakes it,
even when its wait does not match: its prompt is the trigger's `prompt`, then the firing's line,
which carries the firing time, then the events the task kept while it slept. Its open ask and wait
stay open: each new run of a task takes over its open asks and agent waits, so the next `Stop`
puts the task back to sleep on them. One that finds the task in any other state, such as running,
asleep at a human step, or asleep on the daemon's checks gate, is recorded as `coalesced` and is
never typed, so firings never stack.

A message kept for a `running` task also gets an outbox prompt, so the agent reads it after its
current turn. A message kept for an asleep task that satisfies its wait wakes it, as described under
Waits. A message kept for a task in any other state waits for the task's next run.

## Priory home

`PRIORY_HOME` names Priory's own directory. It defaults to `$XDG_STATE_HOME/priory-oss`, then
`~/.local/state/priory-oss`. It holds:

- `repos/<owner>/<name>`: a clone of `https://github.com/<owner>/<name>.git`, with no checkout.
  Git's own `url.<base>.insteadOf` setting can point it elsewhere, such as an SSH remote or a
  local mirror.
- `worktrees/<owner>/<name>/<task id>`: one worktree per task, on branch `priory/<task id>`, from
  the remote's default branch. The task's `worktree` names it only when Priory created it. A start
  reuses an existing worktree only when it belongs to Priory's clone and is on that branch. When
  the folder is gone but the branch is not, as for a revived task the sweep cleaned up or a retired
  task, the start adds the worktree back on that branch. A code step does the same before its
  action runs.
- `priory.sock`: the socket hooks report to. `signals.spool`: the signals sent while no daemon
  listened, which the next daemon applies first.
- `daemon.lock`: one daemon per home. A second one waits up to 5 seconds for the lock, then refuses to start.

## Runs

A run is one task step worked by one agent's `claude` process. When a step leads to another model
step with the same agent, the process carries on into a new run, so each run has at most one result.
A model step with another agent gets a new run and a new process, as described under Moving on. Each
second, the daemon goes through the `queued` and `ready` tasks whose step decides `model`, in queue
order, and starts a run for each one that fits in the room, as described under Queue and Room. It finds the
repository through the task's trigger.

The daemon creates the worktree, records the run and the task as `running`, then starts
the agent's harness in a detached tmux session named `priory-<task id>`, in the worktree. An agent
with `harness = "claude-code"` runs `claude`, as below. One with `harness = "codex"` runs `codex`,
as described under Codex. Each step starts its own agent's harness, so one workflow can mix both.
`claude` gets:

- `--session-id`: a UUID Priory chooses and stores, so a later run can resume the conversation. A
  task keeps one session per agent: when an earlier run of the step's agent reached `SessionStart`,
  the run gets `--resume <that id>` instead, in the same worktree. An agent with
  `session = "shared"` keeps one session across tasks instead, as described under Shared sessions.
- `--model` and `--effort`: the agent's, when it sets them.
- `--append-system-prompt`: the agent body, the Markdown after its frontmatter.
- `--permission-mode`: the agent's `permissions.defaultMode`, or `manual` when it sets none, whatever
  mode the user's own settings choose. In `manual` mode, every permission request reaches Priory's
  deny hook, described under Health. In `auto` mode, Claude's auto mode decides the run's permission
  prompts. `priory check` refuses `bypassPermissions`, and accepts `default` as Claude's older name
  for `manual`.
- `--settings`: hooks that run `priory hook SessionStart`, `UserPromptSubmit` and `Stop`, a
  permission for the `priory` MCP server's tools plus the agent's own `allow` and `deny` rules, in
  Claude Code's syntax, a `PreToolUse` hook that denies `AskUserQuestion` and tells the agent to
  call `ask` instead, and the permission hooks described under Health.
- `--mcp-config`: the `priory` MCP server, `priory mcp --session <id>`, described under Tools.

The same tmux command tags the new pane with a pane option `@priory` set to the harness session
id, so a pane is never left untagged. The daemon types only into that pane, even when a person
opens other panes in the session. Each second, it checks every live run's health, as described
under Health. A git or tmux error ends the run as `failed`, or blocks the task when no run exists yet.

tmux owns the agents, so the daemon can stop and start again without stopping them. When it starts,
it keeps every live run whose session still exists. It kills each tmux session whose tagged pane
carries the harness session of an ended run that no live run continues in that tmux session, as a
crash between ending a run and killing its session leaves.

A live run whose pane is gone when the daemon starts was lost with its tmux server, by a reboot or
a killed server, so the agent is not at fault. Unless its result, ask or wait ended the turn, as
under Health, the run ends `lost` and its task becomes `ready`, with the prompt `Priory restarted and
found tmux session <name> gone, as after a reboot, so this run was interrupted. Check the worktree,
then continue.` The next run resumes the session in the same worktree. This happens once: when the
task's previous run was lost too and this run stored no ask or wait, the task blocks with
`run <id> was lost again before it made progress: tmux session <name> is gone`. A prompt the lost
run had attempted stays `uncertain` and is never typed again.

### Queue

The queue holds the `queued` and `ready` tasks at a model step. It starts their runs in band order,
then in the order a person set inside the band, then oldest first. Code steps still run oldest first,
as described under Code steps.

The bands default to `blocker`, `now`, `today` and `later`, highest first. `factory.toml` can name
its own, highest first:

```toml
name = "demo"
bands = ["urgent", "normal", "someday"]
```

A new task's band is its request's `band`, else its trigger's, else its workflow's `band`, else the
last band. A request band the factory lacks, from a message published before a rename, counts as no
band. `priory check` refuses a trigger or workflow that names a band the factory lacks, with
its line, and `priory request --band` refuses one too. The daemon records the bands in the state
file when it starts, so the listings and `priory move` read them without the factory. A task whose
band a later `factory.toml` dropped comes after the last band. A task from before bands existed gets
the last band when the daemon starts.

`priory move <task> --band <band>` puts an active task in another band, after the tasks a person
ordered there. `priory move <task> --top`, `--before <other>` or `--after <other>` sets its place in
the person's order of its band, or of the other task's band, which it joins. `--band` combines with
`--top`, or with `--before` and `--after` when it names the other task's band. A move that sets a place
numbers every active task of the band in its current order, so tasks that arrive later go after them, oldest first.
`priory move` writes the state file directly: it needs no daemon, and the daemon reads the new order
on its next tick.

### Holds

`priory hold <task> --reason <text>` holds a `queued` or `ready` task, so it never starts until
its hold is released. It refuses a task in any other state: a running task already has a process,
which a hold would have to kill mid-turn, and an asleep task waits on its own ask or wait, whose wake
would race the hold. Hold such a task once it is back in the queue. `priory release <task>` lets a
held task go, back to the state it was held from. Both go to the daemon over `priory.sock`, as
`priory resume` does, so a hold and a run start never race.

A hold takes at most one condition, which releases it without a person:

| Flag | Released when |
| ---- | ------------- |
| none | a person runs `priory release` |
| `--until <time>` | the time passes: an RFC 3339 instant, or a duration such as `2 hours` from the hold, as for a time wait |
| `--after <task>` | the other task is done, failed or dropped |
| `--merge <pr>` | the pull request merges: `owner/name#42`, or `42` in the repository of the task's trigger |
| `--blockers` | the task's Linear issue has no open blocker left |

`--merge` takes `--if-closed release|drop|ask` for a pull request closed without a merge: `release`
lets the task go, `drop` drops it with the reason `<pr> closed without a merge`, and `ask`, the
default, puts an ask on the desk with the options `release` and `drop` while the task stays held.
`priory answer` takes either. A merge after the ask releases the task and answers the ask.

`--blockers` reads the `blockers` field of Linear events (see `docs/intake.md`). It is refused for
a task whose source event is not a Linear event, such as a GitHub issue or a CLI request. The newest
event about the issue that carries `blockers` decides: an empty list releases the task, and a null
one, which Linear's paging left unknown, does not. That event is a creation or state change of the
issue, or a `linear.issue.blockers` that one of its blockers published when it changed.

The daemon judges a hold when it is stored, then a time or task hold on each tick and a merge or
blockers hold on each message that names its pull request or issue, whatever the message's route.
A condition that already holds, such as a pull request merged before the hold, releases the task at
once. A task request with `hold` (see `docs/intake.md`) creates its task held the same way.

A held task has the reason `held: <reason>, <condition>`, which `priory tasks` and the desk show,
such as `held: needs the schema, until github:owner/repo#7 merges, and asks if it closes unmerged`.
Events for a held task are kept for its first run, as for a queued one, and a schedule's tick for it is
coalesced. `priory cancel` drops a held task and ends its hold.

Known limits:

- A pull request closed, then reopened and merged later, is judged on its newest close only. With
  `drop` or `ask`, the first close already acted.
- A hold names one condition; holding until two of them takes a person.

### Room

The daemon admits a run only when the machine has room for it, measured live:

| Measure | Read from | Admits when |
| ------- | --------- | ----------- |
| Memory | `MemAvailable` and `MemTotal` in `/proc/meminfo` | free memory, minus the memory promised to live runs and held by memory grants, minus the agent's `memory`, stays at or above `(1 - memory share) × MemTotal` |
| Disk | `df -Pk` on `PRIORY_HOME`, which reads `statvfs` | free disk, minus the disk promised to live runs, minus the agent's `disk`, stays above 10 % of the disk, or above 20 GB on a disk over 200 GB |
| CPU | the 1-minute load in `/proc/loadavg`, and the CPUs this process may use | the load stays below `cpu share × CPUs` |

The shares come from `agents` in `runners/<host>.toml`, where `<host>` is the name in
`/proc/sys/kernel/hostname`. Each part defaults to 90 %, as when the file or the field is missing.
So by default a run must leave 10 % of memory free, and the load must stay below 90 % of the CPUs.
`agents` has no disk part, so disk always keeps 10 % free, and at most 20 GB, so that a large disk
with tens of GB free still admits runs.

An agent declares its `memory` as a size such as `512MB` or `6GB`, where a GB is 1024³ bytes, as
in `/proc/meminfo`. An agent with no `memory` declares 4 GB: Claude Code takes about 0.5 GB, and an
agent runs builds and tests beside it. Declaring too much only delays a start; declaring too little
lets agents run the machine out of memory.

An agent may also declare `disk`, a size reserved above the disk floor. An agent with no `disk`
reserves none. A run that waits for disk it declared gets the reason `waits for room: 22.0 GB of
disk free, needs 8.0 GB and keeps 20.0 GB free`.

A live run promises its agent's `memory` and `disk` from its start until it ends, since no run
reports its own use yet. The room subtracts every promise and every held memory grant, described
under Memory gate, from the measured free memory and disk, so two runs admitted one after the other
cannot both take the same free room, and a run never starts into memory a build holds. The daemon
measures again before each
admission, since a start's clone and worktree take disk.

A task that does not fit stays `queued` or `ready`, and its reason says why it waits, such as
`waits for room: 3.1 GB of memory free, needs 6.0 GB and keeps 6.4 GB free`, `waits for room: 12.0
GB of disk free, keeps 20.0 GB free` or `waits for room: load 7.50 on 8 CPUs, keeps it under 7.20`.
`priory tasks` shows it.

The first task that waits for room holds back every task behind it in the queue, even a smaller one
that fits. Each of those gets the reason `waits behind task <id>, which waits for room`. So a higher
band only starts earlier: it never skips the room check, and a large task is never starved by
smaller ones that keep taking the room it waits for. The cost is idle room while it waits. A task
whose agent declares more memory than the runner's share of the machine, or more disk than the disk
holds above its floor, can never fit, so it holds back no one. A task that waits for a shared agent holds back no one either, as described under
Shared sessions.

While the head of the queue waits for room, the queue is stalled. The daemon records the stall in
the state file, so its start survives a restart, and `priory desk` starts with one banner line:
`stalled since 2026-11-19T10:00:00Z: short of disk by 10.0 GB; task 3 waits for room`. Another
task heading the queue for the same resource keeps the stall; a different resource starts a new one.
The stall ends when no task waits for room at the head. A task waiting because the room cannot be
measured holds the queue without a stall. The sweep scripts and the sweeper act on stalls, as
described under Sweep.

`PRIORY_PROC` names a directory to read in place of `/proc`. The tests use it, with a fake `df` on
`PATH`, to steer the measurements.

Known limits:

- A run's actual use already lowers the measured free memory, and its promise counts again on top,
  so the room errs on the side of starting fewer runs. A run that reports its own use would end the
  double count.
- An agent that grows past its declared `memory` can still crowd the machine. The memory gate
  covers the builds an agent wraps in it, and the agents slice, below, slows the rest down.
- Load is not declared per agent, so when load leaves no room, no task fits anyway.
- The measurements need Linux's `/proc`.

### Agents slice

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

### Memory gate

`priory memory acquire --gb N [--timeout D] -- <command>` runs a heavy command, such as a build or
a test suite, only once the machine has room for it. Several agents' builds then queue instead of
running the machine out of memory. N is a number of GB, such as `8` or `1.5`. D is a duration such
as `90s`, `15m` or `2h`.

1. The command asks the daemon over `priory.sock` for a grant of N GB. It sends its own pid and,
   inside a run, the run's `PRIORY_SESSION`, which names the grant's task.
2. The grant waits in one queue, oldest first. The daemon grants the oldest waiting grant when the
   measured free memory, minus the memory promised to live runs and every held grant, minus N,
   stays at or above `(1 - memory share) × MemTotal`, as the room check for runs does. A grant
   that does not fit holds back every grant behind it, even a smaller one that fits, so a large
   build is never starved.
3. The command polls the daemon every 250 ms. Once its grant is held, it runs the command with its
   own working directory, environment and terminal, and hands the grant to the command's process.
4. When the command exits, however it exits, the grant is released. `acquire` exits with the
   command's code, or 128 plus the signal that killed it.

The daemon removes a grant whose holder is gone, on each tick and on each gate message. The holder
is the `acquire` process while the grant waits, then the command's process. The daemon knows a
holder by its pid and its start time in `/proc/<pid>/stat`, so a zombie or a reused pid counts as
gone. So a killed `acquire` that waits frees its place within a second. A killed `acquire` whose
command still runs keeps its grant until that command ends.

`acquire` exits 125 and never runs the command when no daemon listens, when the daemon cannot
measure the room or refuses the grant, or when the grant still waits after `--timeout`. A grant of more than the runner's share of
the machine can never fit, so the daemon refuses it at once, as
`9.0 GB never fits: this runner's agents may take 7.2 GB`. A command that cannot start exits 127.

`priory memory` prints each grant in the order it was requested: id, task, state (`held` or
`waiting`), GB, command and age. A held grant's age counts from its grant, a waiting one's from its
request. `priory desk` shows each waiting grant as a `memory` row.

Every agent learns the gate from the MCP server's instructions, which ask it to wrap each heavy
command in `priory memory acquire --gb N -- <command>`, with the path of this `priory`.

Known limits:

- A grant counts on top of the memory its run already promised, so a build inside a run counts
  twice, as a run's own use does under Room. The gate errs on the side of waiting.
- Runs and grants queue apart: a run that fits starts even while a grant waits, and can take the
  room the grant waits for.
- The command's process holds the grant, not the processes it leaves behind: a daemonized child
  of a build runs on without a grant.
- A grant that arrives in the last poll before `--timeout` still runs its command, up to 250 ms late.
- A daemon that stops while a command waits fails that command with 125. A held grant survives a
  daemon restart in the state database.

### Shared sessions

An agent's `session` is `per-task`, the default, or `shared`. A `per-task` agent gets one session
per task, as above. A `shared` agent, such as a lead, keeps one conversation across every task it
serves. The state stores that session per agent. Once a run of it reached `SessionStart`, every
later run of the agent, for any task, resumes it with `--resume`. Until then, a new run starts a new
session and records it as the agent's.

A session runs one turn at a time, so a shared agent has at most one live run. A task that needs
the agent while another task's run of it is live stays `queued` or `ready`, with the reason
`waits for agent <name>: its shared session serves task <id>`, which `priory tasks` shows. It starts
once that run ends, by its result, ask, wait or a verdict. The room is checked after that, and a
task behind it that needs another agent can start ahead of it.

Each task keeps its own worktree and branch, and the resumed session starts in the task's worktree.
One worktree for the agent would put every task's commits on one branch and leave the sweep with no
task to judge it by. Each new run of a shared agent starts its task's first queued prompt with
`This is task <id> (<key>), from trigger <trigger>, in the worktree <path>. Earlier turns of this
conversation may belong to other tasks.` So the conversation can tell its tasks apart, even after a
crash cut a prompt short.

A session that an agent once shared stays shared: when the factory turns the agent back to
`per-task`, each task starts a new session rather than resuming the shared one.

Known limits:

- A long shared conversation is never compacted by Priory.
- Claude keeps a session's transcript under the directory it started in. Whether `--resume` finds
  it from another task's worktree depends on the Claude version, and no test runs a real Claude.
- Signals and tool calls name the harness session, not the process. The daemon kills a run's tmux
  session before it admits the next task, but a process that survived that kill could still report
  for the run that resumed its session.

### Trusting a clone

Claude asks whether to trust a folder the first time it runs there, before any hook fires. Priory
never answers that dialog and never writes Claude's settings. A worktree inherits the trust of the
repository it belongs to, so a person trusts each clone once, after Priory created it:

```sh
cd "$PRIORY_HOME/repos/<owner>/<name>" && claude   # accept the dialog, then /exit
```

A run that sends no `SessionStart` within 60 seconds of its start ends as `exited`. The daemon kills
its tmux session and blocks its task with the reason `claude showed no SessionStart within 60s;
trust the clone once: cd <clone path> && claude, accept, exit`, then `. Its pane ended with: ` and
the last 5 non-empty lines of its pane, joined by ` | `, so a person sees the dialog or warning
that stopped it. The daemon log gets the last 30. Its queued prompts stay queued, and a prompt it
was launched with goes back to the queue, so the task's next run gets them. A Codex run blocks
with the Codex instruction under Codex.

### Codex

A run of a `harness = "codex"` agent starts `codex --no-daemon`, or `codex resume <thread>
--no-daemon`, in the same tmux
session, worktree and outbox as a Claude run. Codex chooses its own session id, its thread, so
Priory keeps a UUID of its own as the run's session: the pane's `@priory` tag, the hooks and the
tools carry it. The pane sets `PRIORY_SESSION` to it, and the thread arrives on `SessionStart`
stdin. Every setting goes in as a `-c key=value` override, so Priory writes no
Codex file. `--no-daemon` keeps the run off the background server that a person's other Codex
sessions share, with its environment and its cached hook trust.

Codex 0.170 fires `SessionStart` with the first turn, not at startup, so a Codex run cannot wait
for it before its first prompt. The daemon marks the task's oldest queued prompt `attempted` by the
run and passes it as Codex's last argument, after `--`. tmux refuses a command over about 16 KiB,
so the prompt goes through a file under `$PRIORY_HOME/prompts`, which a `sh` wrapper reads, removes
and appends. `UserPromptSubmit` confirms it as it does a paste. Every later prompt, such as an answer or a nudge, is pasted. A Claude run still gets its
first prompt pasted after `SessionStart`, which Claude fires at startup, so its paste and its
confirmation stay as they are.

| Priory duty | Claude Code | Codex |
| ----------- | ----------- | ----- |
| The `priory` MCP server | `--mcp-config` | `-c mcp_servers.priory={command, args, env}`, with `default_tools_approval_mode = "approve"` in place of Claude's `mcp__priory` allow rule |
| Session id (R5.5) | `--session-id <UUID>` Priory chooses | the thread from the `SessionStart` hook's `session_id`, stored on the run; `priory runs` shows it |
| Resume | `--resume <session>` | `codex resume <thread>` |
| First prompt | pasted after `SessionStart` | the last argument, `-- <prompt>` |
| Idle signal for the outbox (R5.4) | `SessionStart`, `UserPromptSubmit`, `Stop` hooks in `--settings` | the same three hooks, as `-c hooks.<Event>`; Codex hooks take Claude's stdin and output shapes |
| Stop signal for health (R7.1) | `Stop` hook, and the pane for an exit | the same |
| A prompt shown anyway | `Notification` hook with `permission_prompt` | no equivalent: Codex has no notification hook for an approval prompt. A prompt that shows despite the deny hook stalls the turn until its step `timeout` blocks the task |
| Permission policy (R7.2) | `--permission-mode`, `allow` and `deny` rules, `PermissionRequest` deny hook | `sandbox_mode`, `approval_policy` and `approvals_reviewer` by `defaultMode`, below, and the same `PermissionRequest` deny hook |
| Built-in question tool (R6.4) | `PreToolUse` deny on `AskUserQuestion` | `PreToolUse` deny on `request_user_input`, which Codex offers only in plan collaboration mode, which Priory never sets |
| Model, effort, agent body | `--model`, `--effort`, `--append-system-prompt` | `-c model`, `-c model_reasoning_effort`, `-c developer_instructions` |

`defaultMode` maps to Codex's sandbox and approvals. `approvals_reviewer` is always set, so a user
config that routes approvals to Codex's reviewer does not decide a `manual` run's prompts:

| `defaultMode` | `sandbox_mode` | `approval_policy` | `approvals_reviewer` |
| ------------- | -------------- | ----------------- | -------------------- |
| `manual`, or none | `read-only` | `on-request` | `user`, so the deny hook answers |
| `acceptEdits` | `workspace-write` | `on-request` | `user` |
| `auto` | `workspace-write` | `on-request` | `auto_review`: Codex's reviewer decides, as Claude's auto mode does |
| `dontAsk` | `read-only` | `never` | `user` |

`priory check` refuses what cannot map, with its line: `defaultMode = "plan"`, since Codex's CLI has
no plan permission mode, and any `allow` or `deny` rule, since Codex has no per-tool rules to put
them in.

Codex runs only the hooks a person trusted in its `/hooks` review, by their hash, and asks whether
to trust a folder as Claude does. Priory answers neither and never skips the review. Its hooks are
the same in every run, since the run's session reaches them as `PRIORY_SESSION`, so a person trusts
them once, with the clone. A Codex run that sends no `SessionStart` within 60 seconds blocks its
task with `codex showed no SessionStart within 60s; trust the clone and Priory's hooks once: cd
<clone path> && codex -c 'hooks.SessionStart=...' ..., accept the folder, type /hooks and trust
Priory's hooks, then exit`, then the pane's last lines as under Trusting a clone. The command
carries Priory's hooks exactly as runs get them. A hook nobody trusted shows Codex's `Hooks need
review` dialog, which those lines show.

The hooks and the MCP server name Priory by `PRIORY_EXE` when it is set, else by the absolute path
Priory was invoked by, else by its resolved binary. Codex keys hook trust on the command, so that
path must survive upgrades: under Nix, run Priory by its profile link, such as
`~/.nix-profile/bin/priory`, or set `PRIORY_EXE` to it, rather than a store path that changes on
every deploy. Changing that path changes the hooks, so a person trusts them again.

Known limits:

- No test runs a real Codex. Whether `/hooks` lists and keeps trust for `-c` hooks and whether
  `auto_review` runs before the deny hook are read from Codex's documentation, not checked. That
  `SessionStart` comes with the first turn was seen with Codex 0.170.
- Priory invoked by a bare name, as from `PATH`, names its resolved binary; set `PRIORY_EXE` then.
- An agent whose harness changes during a task starts a new session, since neither harness resumes
  the other's.
- Codex keeps its transcripts under `~/.codex/sessions`, by thread. The traces slice reads token use
  from there; Priory reads nothing from them yet.

## Signals

`priory hook <event>` reads Claude's hook JSON on stdin and sends one JSON line to the daemon over
`priory.sock`: `{"event": "Stop", "session": "<id>", "prompt": "<text>"}`. `prompt` comes only with
`UserPromptSubmit`. Every run's pane sets `PRIORY_SESSION` to the run's session. When the stdin
`session_id` differs from it, as in a Codex run, the hook sends `PRIORY_SESSION` as the session and
the stdin id as `thread`, which `SessionStart` stores. The daemon answers `ok` once the signal is
committed, and the hook waits up to 10 seconds for it. When no daemon takes the line, it goes to `signals.spool` instead. The next
daemon applies the spool before it removes it.

- `SessionStart`: the harness is up and its input is ready, so the run is idle.
- `UserPromptSubmit`: the run is busy. Its attempted prompt is confirmed if the submitted text has
  the same words, once the `pasted_content` tag that Claude wraps a pasted prompt in is removed.
- `Stop`: the turn ended, so the run is idle.
- `PermissionPrompt`: Claude shows a permission prompt, with its `message`, as described under
  Health.

Each signal also carries the `transcript_path` of Claude's hook JSON, which Trace reads.

`UserPromptSubmit` also stamps the start of the run's turn, and `SessionStart` and `Stop` clear it.
The daemon never reads the terminal screen to decide that an agent is idle.

### Nudge

A turn that ends with no `wait`, `ask` or `result` gets one nudge. The `Stop` signal finds the run
with no result, open ask or open wait, and queues a prompt that names the three tools, then the
step's ending line from `tasks.ending`. The next tick types it like any other prompt.

Priory tells its own turns from a person's by the prompt it confirmed. `UserPromptSubmit` marks the
turn nudgeable only when the submitted text matches the prompt Priory attempted and that prompt is
not itself a nudge. A turn a person typed into the pane matches nothing, so it is never nudged.
A successful `wait`, `ask` or `result` call, `SessionStart` and `Stop` clear it, so a turn whose ask was answered or whose wait fired before `Stop` is not nudged.

So a nudge starts at most one turn, and a silent turn after it is not nudged again. The run is then
left to the health check, like any run that sends no exit. A prompt already queued when the turn
ends follows the nudge in the same prompt. The nudge is an outbox prompt rather than
the hook's `decision: block`: the daemon already confirms, retries and survives a restart for
outbox prompts, and the hook has no reply channel beyond `ok`.

## Health

Each second, the daemon gives every live run one of these verdicts, in this order:

| Verdict | Detected by | Run ends | Task | Reason |
| ------- | ----------- | -------- | ---- | ------ |
| Unexpected exit | the tagged pane is gone, or dead under `remain-on-exit`, and no result, ask or wait ended the turn | `exited` | `failed`, then repaired: see Repair | `run <id> exited: claude no longer runs in tmux session <name>` |
| No `SessionStart` | 60 seconds with no `SessionStart`, as under Trusting a clone | `exited` | `blocked` | `claude showed no SessionStart within 60s; ...` |
| Permission prompt | a `PermissionPrompt` signal | `permission` | `blocked` | `run <id> stopped on a permission prompt: <message>` |
| Timeout | a turn older than its step's `timeout` | `timeout` | `blocked` | `run <id> timed out: its turn ran past <n>s` |

A verdict blocks the task when a person must act: the trust dialog, a permission the agent lacks,
or a turn that may be legitimately long. A person reads the reason on the desk and either resumes
the task or cancels it. An unexpected exit fails the task instead, since the daemon can often tell
why the agent died and act on it, as under Repair. An exit before the run's `SessionStart` still
blocks the task: a harness that never came up dies the same way when restarted. The daemon commits
the verdict, then kills the run's tmux session, so a crash in between leaves an ended run, which the
next start cleans up.

### Failed or blocked

These ends make a task `failed`:

- An unexpected exit after the run's `SessionStart`, whatever its cause. Its repair decides whether
  it stays failed.
- A second unexpected exit of the run its repair restarted, before that run stored a result, ask or
  wait.

Everything else that stops a task needs a person and makes it `blocked`: no `SessionStart`, an exit
before `SessionStart`, a permission prompt, a timeout, a run lost twice, a code step's action that
is not built, a step with no known agent, a trigger with no repo, and a git or tmux error at the
start of a run. `open-pr`'s own failure is not an end: it is the `failed` outcome, which the
workflow routes like any other.

### Repair

When an unexpected exit fails a task, the daemon makes one repair attempt. It classifies the cause
from the room, measured as for admission, after the run's promise is released:

| Cause | Found when | Action | Restarts when |
| ----- | ---------- | ------ | ------------- |
| `out of memory (<room>)` | the room has no memory for the agent | `wait for room` | the room check admits the run again |
| `disk full (<room>)` | the room has no disk | `sweep`: a sweep runs at once, and the room is measured again | the disk is no longer short |
| `the harness crashed` | the room has space | `restart` | at once |

A cause that is gone, or that the room check will confirm gone before the run starts, makes the task
`ready` with the prompt `Priory repaired this task after: <reason>. Cause: <cause>. Action:
<action>. Check the worktree, then continue.` ahead of its other prompts. The next run resumes the
session of the step's agent, as a resume does. A disk still short after the sweep leaves the task
`failed` with the reason `<reason>. Cause: <cause>. Action: sweep. The cause persists, so Priory
did not restart the agent.`

A task gets one repair per run: when the run a repair restarted exits too, before it stored a result,
ask or wait, the task stays `failed` with `<reason>; it failed again after its repair, so Priory did
not restart the agent`. Once a run stores a result, ask or wait, a later exit gets a repair again,
and so does the first exit after a revive. `priory revive` restarts a failed task either way.

Each repair appends a `factory.task.repaired` record to the trace, with its cause, action and
outcome, `ready` or `failed`, as [the trace schema](trace-schema.md) defines it.

Known limits:

- The cause is read after the agent died, when its own memory is already free. An agent the kernel
  killed for memory often reads as `the harness crashed`, and still restarts once.
- A harness that crashes for a reason the room cannot see, such as a bad binary after its first
  turn, gets one restart before it stays failed.
- Waiting for room has no deadline: a task waits as any task in the queue does.
- The daemon ends the run, then repairs in a second transaction, since the sweep runs outside any
  transaction. A daemon that stops in between leaves the task `blocked` with the exit's reason, as
  before repair existed.
- `ready` means the task waits to start its agent, not that the agent started: a cancel or a room
  that never frees can still stop it.
- A sweeper task never waits for room, so its memory repair starts it at once.
- A repair's run lost with its tmux server is replaced by a run that gets a repair of its own.
- A room that cannot be measured confirms no freed disk, but a crash with an unreadable room reads
  as `the harness crashed`, and the admission then waits until the room reads again.

A run whose agent is gone after its result, ask or wait ended the turn is not an unexpected exit.
It settles as its `Stop` would have, as under Moving on, except that a next model step always gets
a new run, since no process carries on.

A model step's `timeout` is a duration such as `90s`, `15m` or `2h`, and defaults to `3h`. A run
whose agent exits after a wait already fired, or after its ask was answered, but before its `Stop`,
counts as an unexpected exit; `priory resume` recovers it. It
bounds one turn, from `UserPromptSubmit` to `Stop`, not the step.

Priory never approves a permission prompt. Each run's settings carry a `PermissionRequest` hook
that denies every request and tells the agent: `Priory runs this session unattended and never
approves a permission prompt, so it denied this one. Go on without it. If the task cannot go on
without it, call the ask or result tool of the priory MCP server.` What the agent does next is its
own: the desk hears of it only through that ask, or a verdict if the run then stalls. A
`Notification` hook with the matcher `permission_prompt` runs `priory hook PermissionPrompt`, so a
prompt that shows anyway, when the deny hook failed, ends the run on the next tick.

## Desk

`priory desk` prints the stall banner first while the queue is stalled (see Room), then what waits on a person, oldest first: since, kind, id, task, lane, what and the
task's generation.

| Kind | Id | What | Since |
| ---- | -- | ---- | ----- |
| `ask` | the ask | its question, then its options | it was asked |
| `blocked` | the task | its reason | the task last changed state |
| `failed` | the task | its reason, then `revive: priory revive <task>` | the task last changed state |
| `prompt` | the outbox prompt | its text, for an `uncertain` prompt | it was attempted |
| `held` | the hold | the task's reason: why it is held, and until what | it was held |
| `memory` | the memory grant | `waits for <N> GB: <command>` | it was requested |
| `queue` | the task's place in the queue | its band, then its reason when it has one | the task last changed state |
| `sweep` | the script's run | its command, then `running`, or its status and output, for the open stall | it started |
| `retired` | the task | `its worktree was removed to free disk; it resurrects from <branch> when its wait fires` | it retired |
| `worktree` | the worktree's path | why it stays: see Sweep | `-` |

The lane column is the task's lane, and `-` for a `worktree` row.

It lists only asks and prompts of active tasks, holds of held tasks, and waiting memory grants. After them come the `queue` rows, in start order,
and last one `worktree` row for every worktree the sweep keeps. `priory asks` still prints every open ask.

`priory resume <task>` makes a blocked task `ready`. It queues the prompt `A person resumed this
task after: <reason>. Check the worktree, then continue.` ahead of the task's other queued prompts,
so its next run resumes the session of the step's agent with it. When the task's latest run never
reached `SessionStart`, or it has none, the reason is about the launch, not the agent's work, so the
prompt is only `A person resumed this task. Check the worktree, then continue.` A resume line that
no run took yet is replaced, not repeated, and the line that ends the step is added only when no
queued prompt carries it already. `priory cancel <task>` drops an
active task with the reason `cancelled from the desk`, ends its live run as `cancelled` and kills
its tmux session. Both go to the daemon over `priory.sock`, as `priory answer` does, and are refused
for a task in any other state. An answer to an ask of a done, failed or dropped task is refused too.

### Revive

`priory revive <task> [--step <step>]` restarts a failed or dropped task as its next generation. A
task starts at generation 1. The revive goes to the daemon over `priory.sock` and, in one
transaction:

1. Withdraws every open ask and wait of the task, recording the new generation on each.
2. Raises the task's generation by one.
3. Enters the step the task ended on, or `<step>`, as a move into it would (see Moving on): a model
   step makes the task `ready` with its prompt queued, a human step asks again, and a code step
   makes the task `ready`.
4. At a model step, queues ahead of that prompt: `A person revived this task as generation <n>
   after it was <state>: <reason>. The asks and waits of its earlier generation are withdrawn. Check
   the worktree, then continue.`

It replies `task <id> is <state> at <step>, generation <n>`. It refuses a task in any other state, a
step the task's workflow lacks, and a task whose key another active task holds, naming that task:
a factory holds at most one active task per key.

The task keeps its id, key, branch, worktree and sessions. Its next run resumes the session of the
step's agent, like a resume. The agent keeps what it learned in the earlier generation, and the
revive prompt tells it that the asks and waits it remembers are gone. A new session would lose that
context, and the worktree already holds the earlier generation's work. When the sweep removed the
worktree, the next run adds it back from the branch, as under Priory home. A revive straight into a
code step, such as `open-pr`, adds it back before the action runs.

A withdrawn ask or wait stays in the state file but counts as closed. An answer to a withdrawn ask
is refused with `ask <id> was withdrawn when task <task> was revived as generation <n>`. An event
for a withdrawn wait fires nothing: the task keeps it, as it keeps any event that matches no wait,
and the next run reads it. `priory asks`, `priory waits` and the desk leave withdrawn ones out. A
new run never takes over a withdrawn ask or wait.

A failed task is one its repair could not restart, as under Repair.

## Sweep

The daemon removes the worktrees of ended tasks. It sweeps when it starts, when a stall short of
disk starts, and every 10 minutes after, or every `[sweep] every`, not on every tick. A sweep fetches `origin` into each clone with a worktree and refreshes
`origin/HEAD`, then looks at each folder `worktrees/<owner>/<name>/<id>`. A clone whose fetch
failed is skipped until the next sweep.

It removes a folder, and its branch `priory/<id>`, only when every check holds:

- Task `<id>` is `done`, `failed` or `dropped`, and its recorded `worktree` is this folder.
- It is a worktree of Priory's clone `repos/<owner>/<name>`, on branch `priory/<id>`.
- Nothing is uncommitted or untracked, whatever `status.showUntrackedFiles` says.
- No process has its working directory inside it (read from `/proc`).
- The branch tip is in `origin/HEAD`, or it is the head commit of a merged pull request for the
  branch (a `github.pull_request.closed` event with `merged` true). Otherwise it stays as
  `unpushed commits` when a commit is on no remote branch, else as `not merged`.

The sweep runs `git worktree remove`, then `git branch -d`, both without force, so git refuses
what the checks missed. A squash-merged branch is not in the default branch, so `git branch -d`
refuses it: the worktree goes and the branch stays, and no sweep looks at that branch again. A
failed worktree removal is logged and retried at the next sweep. A git command that cannot run
keeps the worktree, as `cannot check: <error>`.

`priory desk` runs the same checks without fetching or removing, and lists every folder that stays
with its reason: `no task <id>`, `task <id> still <state>`, `not a worktree of <clone>`,
`not recorded as the worktree of task <id>`, `not on branch priory/<id>`, `uncommitted changes`, `untracked files`, `a process runs in it`,
`unpushed commits` or `not merged`. It reads the refs of the last fetch.

### Retiring asleep tasks

After each sweep, when free disk is at or under the room's disk floor (see Room), the daemon retires
asleep tasks, the longest asleep first, until the floor is met again. Retiring runs
`git worktree remove` without force: the worktree goes, and the branch `priory/<id>` and the task's
sessions stay. A task retires only when its worktree is Priory's, on its branch, has nothing
uncommitted or untracked, and no process runs in it. The daemon marks the task before it removes the worktree, so a
crash in between leaves a mark on a folder that still exists, which the next sweep checks again.

Its commits need not be pushed: the branch stays in Priory's clone, on the same disk, and no sweep
deletes the branch of an active task. Requiring a push would keep most asleep tasks, since an agent
pushes only at `open-pr`, and pushing is an irreversible step Priory does not take on its own.

A retired task stays `asleep`, with its `retired` time set. It keeps every wait, ask and event, so
routing, holds and waits treat it as any asleep task, and R4.1's states stay as they are. When its
wait fires or its ask is answered, it becomes `ready`, as R3.2 says. Its next run adds the worktree
back from the branch, at the same path, so `claude --resume` finds the session it stored for that
folder. Starting the run, or a code step's action, clears `retired`. An answer that approves a step
while the task is retired reads the branch's commit from Priory's clone.

A code step skips the room check, except when it must add a worktree back: while free disk is at or
under the floor, the task stays `ready` with the reason `waits for room to add back its worktree`,
or `waits for room: <error>` when the disk cannot be measured,
and the code steps behind it wait too.

A task that cannot retire gets the reason `cannot retire: <why>`, with the same reasons as the
sweep's, such as `untracked files`. The daemon sets it, and logs it, only when it changes, so it is
reported once. A wake clears it.

Retiring runs only at a sweep: when the daemon starts, when a disk stall starts, and every `[sweep]
every`. A disk that runs short with nothing waiting for room waits for the next sweep.

### Sweep scripts and the sweeper

`factory.toml` can add a `[sweep]` table:

```toml
[sweep]
every = "10m"                 # the built-in worktree sweep's cadence
scripts = [
  { run = "docker builder prune -f", when = "disk", timeout = "5m" },
]
after = "30m"                 # how long the queue stays stalled after its scripts
agent = "sweeper"             # no agent, no sweeper
repo = "maxence-lefebvre/priory"  # where the sweeper works and files its issue; this is the default
```

When a stall starts, the daemon runs every script whose `when` names its resource: `memory`, `disk`
or `cpu`. Each runs once per stall, through `sh -c` in `PRIORY_HOME`, in its own process group. A
script past its `timeout`, 5 minutes by default, is killed with its children. The daemon keeps the
last 2000 bytes of its output and its status, `exit <code>` or `timed out after <n>s`, and the desk
lists each script of the open stall as a `sweep` row. When a script ends, children it left in the
background are killed too. A script that fails or cannot start never stops the daemon. A restarted
daemon kills the scripts its predecessor left running, after checking each process group's start
time in `/proc` so that a reused pid is left alone, and records them as `interrupted by a daemon
restart` with the tail of their output.

Once the stall's scripts have ended, or at its start when none matches, the stall waits `after`, 30
minutes by default. If it still holds and `agent` names an agent, the daemon creates a task with the
key `priory:sweep` and the trigger `priory/sweep` on the `sweep` workflow, in the highest band and
the lane `priory/sweep`. A stall gets at most one sweeper. A key has at most one active task, so while
the sweeper of an earlier stall is still active, a new stall waits for it to end before it gets its
own. A sweeper task skips the room check: it frees the room the queue waits for.
Its source event, `priory.queue.stalled`, carries the stall, `PRIORY_HOME` and each script's
command, status and output, so its first prompt names them.

Unless the factory defines its own `sweep` workflow, the sweeper runs this built-in one:

1. `clean`, a model step with the sweeper agent. It removes only what Priory or the scripts manage
   and never a worktree with unpushed, uncommitted or untracked work. It is `irreversible` and
   `autonomous`: it deletes files, and waiting for a person would keep the queue stalled. It ends
   `cleaned`, with a summary whose first line titles an issue, or `nothing`, which ends the task.
2. `approve-issue`, a human step with the options `open` and `skip`, which approves `open-issue`.
3. `open-issue`, a code step that runs `gh issue create -R <repo>` with the latest result's summary
   and evidence, as `open-pr` does. The body ends with `Filed by Priory for event <id>`, the sweeper's
   source event. Before it creates one, the step looks for an issue in any state whose body holds
   that line, which a crash after its creation leaves, and reuses it. It is irreversible and needs that approval: it posts an agent's
   text to a repository other people read, and a sweeper runs rarely, so the ask costs little.

`priory check` refuses a `[sweep]` field it does not know, a malformed duration or resource, and an
`agent` the factory lacks, and a `repo` that is not `owner/name`, with its line.

Known limits:

- The sweeper's worktree is a clone of `repo`, which takes some disk on a disk stall.
- What the sweeper may remove is set by its prompt, not enforced on paths.
- A script's log file grows until it ends; only its tail is read.
- A crash between starting a script and recording its pid leaves that script unbounded.
- A restarted daemon cannot kill the background children of a lost script whose shell already
  exited, since it cannot tell that group from a process that reused the shell's pid.
- A stall that alternates between two resources starts a new stall, and new scripts, at each change.

## Tools

`priory mcp --session <id>` is a stdio MCP server. It answers `initialize`, `ping`, `tools/list`
and `tools/call`, and sends each tool call to the daemon over `priory.sock` as
`{"tool": ..., "session": ..., "arguments": ...}`. The daemon applies it in one transaction and
replies with the tool's text or error. With no daemon listening, the call fails and nothing is
stored.

- `result(outcome, summary, evidence?, commit?)` stores the run's result, which nothing can change
  or delete. The outcome must be a key of the step's `next`; otherwise the error lists the valid
  outcomes. An outcome that leads to `done` needs `commit` when the task owns a pull request,
  as described under Done and checks. A `commit`, like a checks wait's `head_sha`, must be a full
  SHA, since GitHub names pull request heads by theirs: an abbreviated one is refused, and one in
  capitals is stored in lower case.
- `ask(question, options, recommendation, tradeoff)` stores an ask at revision 1.

- `wait(on)` stores a wait, described under Waits.

A run has at most one of a result, an open ask and an open wait: any later call is refused.

### Moving on

When the run's turn ends, its `Stop` moves the task by its result to `next[outcome]`:

| Target     | Task                                                 | Run                       |
| ---------- | ---------------------------------------------------- | ------------------------- |
| model step, same agent | at that step, `running`, with its prompt queued | `moved`, into a new run |
| model step, other agent or irreversible | at that step, `ready`, with its prompt queued | `moved` |
| human step | at that step, `asleep`, with an ask of its `options` | `asleep`                  |
| code step  | at that step, `ready`: see Code steps                | `moved`                   |
| `done`     | `done`, or `asleep` on its checks: see Done and checks | `done`, or `asleep`       |
| `stop`     | `dropped`                                            | `dropped`                 |

A prompt that a move or an answer queues goes ahead of the prompts already queued for the task, and
carries them after it, so an event kept during the last turn never reaches the agent before its new
instructions.

A `ready` task's next run starts the step's agent, with its own system prompt, model and settings,
and resumes that agent's session for the task, or starts a new one when the agent has not run for
it yet.

A run with an open ask or wait and no result ends `asleep`, and its task sleeps. The daemon kills a run's
tmux session when the run ends, after the hook has its reply. The worktree stays until a sweep removes it.

### Code steps

A code step runs its `action` in the daemon, with no agent and no run. Each loop, the daemon takes
the oldest `queued` or `ready` task at a code step and runs its action outside any transaction. Then
one transaction records the action run and moves the task by `next[status]`, as in the table under
Moving on. An action that is not built yet blocks the task with `action <name> is not built yet`.

`open-pr` is the one built-in action:

1. It pushes the commit of the task's branch, the one its approval covers, to that branch on the
   clone's `origin`.
2. It reuses the pull request already open from that branch, if `gh pr list` finds one.
3. Otherwise it runs `gh pr create -R <repo> --base <default branch> --head <branch>`. The title is
   the first line of the latest result's `summary`, and the body is the summary, then its `evidence`.

Its status is `opened` or `failed`. A failure records the error, which a human step reached by
`next.failed` shows in its question. On `opened`, the task owns the pull request, so its events
route to the task. A pull request that another active task owns fails instead, with that task named.
When the task then reaches `done` and its latest result names no `commit`, the checks wait of Done
and checks pins the commit that `open-pr` pushed.

A daemon that stops after `gh pr create` and before recording it runs the action again at its next
start, once the approval still holds: the push repeats and the open pull request is reused. If
someone closed that pull request meanwhile, a second one opens.

#### Approvals

An irreversible step, code or model, runs only when an answer approved it at the task's current
commit, unless it sets `autonomous`. A step's ask approves each step in its `approves` that
`next[option]` leads to: the daemon records the approval with the commit of the task's branch at the
answer. A run never carries on into an irreversible model step, so each one starts a new run. Before
the step runs, or before that run starts, the daemon compares the approval's commit with the
branch's commit now:

| Approval                       | Effect                                                              |
| ------------------------------ | ------------------------------------------------------------------- |
| at the same commit             | the step runs                                                       |
| at another commit              | the ask gets a new revision, with a line naming both commits, and the task sleeps at that human step. The step's queued prompt is taken back, so the next run never reads it |
| none                           | the task blocks: `step <id> is irreversible and no answer approved it` |

A branch with no commit to compare blocks the task too. `priory check` refuses an irreversible step
that a path reaches with no approving human step, so the `none` row needs a factory changed while
the task ran.

`priory actions` prints each action run: id, task, step, action, status, pull request, pushed commit
and error.

### Asks and answers

`priory asks` prints each open ask: id, task, revision, question and options.
`priory answer <ask> <option-or-text> --revision <n>` sends the answer to the daemon. The first
answer to the ask's revision stands. An answer to another revision, a second answer, or, for a
step's ask, a text that is not one of its options, is refused.

- A step's ask moves the task by `next[option]`, as in the table. A model step makes the task
  `ready` with that step's prompt queued.
- An agent's ask queues the answer as the task's next prompt. An asleep task becomes `ready`. A
  task whose run has not stopped yet stays `running`, and the answer is typed after its `Stop`.

A `ready` task's next run resumes the session of the current step's agent and types its queued
prompts.

### Waits

`wait(on)` names one condition in `on.type`. Each condition fires the wait on:

| `type`         | Fields                  | Fires on                                                                |
| -------------- | ----------------------- | ----------------------------------------------------------------------- |
| `pull_request` | `repo`, `pull_request`  | `github.pull_request.closed` for it, merged or not                      |
| `checks`       | `repo`, `pull_request`, optional `head_sha` | `github.check_run.completed` for it, with `pending` 0, on `head_sha` if given |
| `review`       | `repo`, `pull_request`  | `github.pull_request_review.submitted` for it                           |
| `time`         | `until`                 | the daemon's loop, once `until` has passed                              |
| `task`         | `task`                  | the daemon's loop, once that task is done, failed or dropped            |
| `linear_comment` | `issue`, such as `ENG-7` | `linear.comment.created` on that issue                               |
| `linear_state` | `issue`, `state`, such as `Done` | `linear.issue.state_changed` on that issue, with `to` equal to `state` |
| `slack_reply`  | `channel`, `thread_ts`  | `slack.thread.replied` in that thread, which the intake never publishes for its own bot |
| `disk`         | `gb`, a number above 0  | the daemon's loop, once the room's disk reading shows `gb` GB free (a GB is 1024³ bytes) |

```json
{ "on": { "type": "linear_state", "issue": "ENG-7", "state": "Done" } }
{ "on": { "type": "slack_reply", "channel": "C0123", "thread_ts": "1763500000.000100" } }
```

A Linear or Slack condition makes the task own `linear:<issue>` or `slack:<channel>:<thread_ts>`, as
a pull request condition owns its pull request, and is refused when another active task owns it. It
stores the time of the agent's last prompt, and only an event at or after that time fires it: an
event's `time`, to the second. The wait also takes the events on its resource from that time on
that the daemon ignored because no task owned it, so one of them can fire it at once. The intake watches the issue or thread from then on, as
`docs/intake.md` describes, so a reply posted after the prompt and before the wait fires the wait at
the next poll.

`until` is an RFC 3339 instant, or a duration such as `30 seconds`, `5 minutes` or `2 hours`: an
SQLite date modifier, which Priory adds to the time of the call. An unknown type or a missing field
is refused, and nothing is stored. A pull request condition makes the task own the pull request; it
is refused when another active task owns it already.

The intake publishes one `github.check_run.completed` per check run, and only once it completed, so
the daemon cannot count the runs it has not seen. The intake reads every check run on the head
commit in the same poll, so each check event carries `pending`: how many of them had not completed
then. "Every check run has completed" is a check event with `pending` 0. Two runs that complete
between two polls both carry 0: the first fires the wait, and the second is kept. A check run that
GitHub has not created yet is not counted. A pull request with no check runs never fires a checks
wait. A checks wait without `head_sha` takes a check event on any head, so a late event for an older
head can fire it. More than 100 check runs and statuses on a commit keep `pending` above 0, so such a
pull request never fires a checks wait.

When the wait is stored, the daemon looks at the events the task kept since the agent's last prompt:
those not typed yet, and on which no wait fired. The first one that satisfies the wait fires it at
once, and the time and task conditions are checked at once too. The run then does not sleep: the
event is typed after its `Stop`. A disk wait is checked at the next tick: each tick measures the
disk with the room's reading, `df` on `PRIORY_HOME`, only while a disk wait is open.

A wait that fires queues one prompt ahead of the task's other queued prompts: the event's line, or
for time, task and disk, `wait <id> fired: <until> passed`, `wait <id> fired: task <id> is <state>`
or `wait <id> fired: <N> GB of disk free`, then
a line for each event the task kept while it slept. An asleep task becomes `ready`, and its next run
resumes the session of its step's agent with that prompt. An event that does not satisfy the wait so
stays kept until the wake.

A check event also carries `failing`: how many of the completed check runs on the head commit had a
conclusion other than `success`, `neutral` or `skipped` in the same poll.

`priory waits` prints each open wait: id, task, run, kind, what it waits on, such as `50.0 GB free`
for a disk wait, and when it was stored.

### Done and checks

A task owns a pull request when its `open-pr` opened it, when one of its waits named it, when an
event it kept comes from a pull request on its branch, or when a pull request event created it. An
opened one comes first, then a waited one, then the latest from its branch. When such a task reaches
`done`, by a result, an answer or an action, it is not done yet. The daemon stores a checks wait of
its own on that pull request, pinned to the `commit` of the task's latest result, else to the commit
that `open-pr` pushed after that result. The task sleeps with no process, at the step of that result,
so the agent that made it fixes what the checks find. A task with neither commit is blocked
instead.

The daemon judges this wait on every event the inbox kept or ignored about the pull request,
including events the agent already read and those an earlier task kept. It judges
when it stores the wait and at each new event:

| Condition                                                                           | Effect                                 |
| ----------------------------------------------------------------------------------- | -------------------------------------- |
| The newest event names another head and came after the result                    | wakes the agent: `wait <id> fired: the head of <pull request> moved to <sha>, away from your result's commit <commit>` |
| The newest event names another head and came before the result                   | the task sleeps on, since a later event tells whether the head is the commit |
| Else, the newest check at the commit is red, or counts `failing` and a red check at the commit was seen | wakes the agent with that red check: `wait <id> fired: check <name> at <commit> concluded <conclusion> <url>` |
| Else, the newest check at the commit has `pending` 0 and `failing` 0             | marks the task done and starts no run  |
| Else, no check at the commit, and the newest `github.pull_request.check_runs` snapshot at it has `runs` 0 | marks the task done and starts no run  |
| Otherwise                                                                           | the task sleeps on                     |

Each wake line is followed by the event's own line, unless a queued prompt already carries it. The woken agent can fix the failure and send a
new result, which stores a new checks wait at its new commit. Checks of the old commit that finish
before that result are history, so they do not count as a moved head. One that the intake reports
after the result does, since the daemon cannot tell it from a head moved back.

The `result` tool refuses an outcome that leads to `done` without `commit` when the task owns a pull
request.

Known limits:

- Priory reads no branch protection, so every check run reported for the head counts as required.
  Commit statuses are not check runs and do not count.
- A head with no check runs finishes the task only once the intake publishes its snapshot: at least
  5 minutes after the last update of the pull request, and later when polls fail. A check run
  that appears after the task finished does not count.
- A head pushed again after it had a snapshot with `runs` 0 reuses that snapshot, so a result at
  that head can finish the task within the grace period.
- An agent's own checks wait does not fire on `github.pull_request.check_runs`. Only the done gate
  takes it.
- A check event from an intake older than `failing` cannot mark a task done.

## Outbox

Every prompt the daemon types goes through the outbox. A prompt moves through these states:

| State       | Meaning                                                                          |
| ----------- | -------------------------------------------------------------------------------- |
| `queued`    | Waits for the task's run to be idle                                              |
| `attempted` | Recorded, with the run marked busy, in the transaction before typing             |
| `confirmed` | Its `UserPromptSubmit` arrived                                                   |
| `uncertain` | Attempted before a daemon stop, never confirmed. Priory never types it again     |

A model step's prompt is the step's `prompt`, then the line `End this step by calling the result
tool of the priory MCP server with one of these outcomes: <the keys of next>.` A run's first prompt
is that, then one line per event: the task's source event and every event it kept that no prompt
carried yet. Each line holds the type, the subject and the data.
A `ready` task's run has only those event lines, and none when there are no new events: the prompt
that moved or woke the task is already queued.

The daemon types the oldest queued prompt of a task once its run is idle. It loads the text into a
tmux buffer from stdin and pastes it as one bracketed paste, so newlines do not submit and no text
is read as a key name. Then it presses Enter. A task with an attempted or uncertain prompt gets no
further prompt while its text may still sit in the agent's input: while the run it went to is live,
or moved on into another run. Once that run ended any other way, its process is gone, and the task's
next run gets its prompts. The prompt itself stays `uncertain` on the desk.

## Trace

Each time a step ends, the daemon appends one `factory.step.ended` record, as
[the trace schema](trace-schema.md) defines it, in the transaction that moves the task on. A model
step ends with its run's result, a human step with its answer, and a code step with its action's
status, which is the record's `outcome`. A run that ends any other way, such as blocked, lost or
cancelled, ends no step and appends no record. A repair appends a `factory.task.repaired` record
instead, as under Repair. `priory report` reads only `factory.step.ended` records.

The records live in the `trace` table of the state file, one CloudEvents JSON object per row. R9.3
keeps the task state in Priory's own SQLite file and the event log in herdr-bus's. A record written
to the bus could not commit with the state change it describes, so a crash between the two would
lose a record or write one twice. A row in the state file commits with the move. An exporter can
publish the rows to the bus or to OpenTelemetry later, with the row's `seq` as its cursor and the
record's `id` to drop a duplicate.

`priory trace` prints every record, oldest first, one JSON object per line.

| Field | Priory's value |
| ----- | -------------- |
| `factory` | `name` in `factory.toml` |
| `factory_revision` | the last commit that changed the factory's folder; absent outside git |
| `task`, `run` | the task's and the run's ids; `run` only for a model step, the run that sent the result |
| `generation` | the task's generation when the step ended: 1, plus one per revive |
| `lane` | absent until lanes land |
| `machine` | the host name, which also names the runner |
| `started` | when the task entered the step, so it includes the time the task waited for a run |
| `harness`, `model`, `tokens` | for a model step: the agent's harness, the model the transcript names, else the agent's |

A model step's `tokens` come from Claude's session transcript, the file that `transcript_path` names.
A run records the file and its length at `SessionStart`, since a resumed session's file already
holds earlier runs' turns. At each `Stop`, the daemon reads the lines added since and adds the usage
of each assistant message to the task's current step. Claude writes a message once per content block,
each line with the message's usage, so a message id counts once. A run the daemon blocks or finds
lost is read as it ends, since a resume carries on the same step. Entering a step starts its count
again. A step with no assistant message read has no `tokens`. A transcript that cannot be read keeps
its offset, so the next read counts its turns once.

Known limits:

- A run that was live when the daemon upgraded to this schema has no transcript recorded. Its first
  `Stop` only marks where its reading starts, so that turn's tokens are lost rather than counted
  with other runs' turns.
- A step that used two models, as when an agent's model changed between its runs, records the last
  one, and `--cost` prices all its tokens at that model's price.
- `factory.step.labeled` is not written yet.
- `factory_revision` names the last commit, even when the factory has uncommitted changes.

### Reports

`priory report [DIR] --workflow <name> [--since <window>] [--by lane]` prints, for each step of the workflow,
each outcome with its count, the records that reached the step and its rate:

```
step	outcome	count	reached	rate
review	approved	1	2	50.0%
review	changes	1	2	50.0%
```

A visit reaches its step when it ends there, or while an active task is still at it, so the
outcome rates of a step with tasks in progress add up to less than 100 %.
`--since` takes an RFC 3339 instant or a duration such as `7 days`, the default, back from now.
`--by lane` counts each lane apart, by the lane its task has now, and adds a leading `lane` column. It
does not combine with `--cost`.

`--cost` prints each task's tokens and cost instead, per step, then in total. The prices come from
`prices.toml` in the factory folder `DIR`, which defaults to the current folder. Each table is a model,
as the records name it, with dollars per million tokens of each kind:

```toml
["claude-opus-5-5"]
input = 5.0
output = 25.0
cache_read = 0.5
cache_write = 6.25
```

A model with no price keeps its tokens, and its cost reads `price missing for <model>`, as does its
task's total. A model step with no `tokens` shows `-` and `tokens unknown`, and its task's total names
it. `priory check` refuses a price below zero or not finite. Prices apply when the report runs, so a changed price changes past costs too.

## Running as services

`contrib/systemd/` holds two user units: `priory-daemon.service` runs `priory daemon` and
`priory-intake.service` runs `priory intake`. Both restart on failure after 5 seconds. They share one
environment file, so a person sets the factory folder and the state paths once.

Install the binary where the units look for it, `~/.cargo/bin/priory`, then the units and the file:

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

`PRIORY_HOME` is optional and defaults as under Priory home. A user service does not read your shell
profile, so `PATH` must reach `claude`, `git`, `gh` and `tmux`. Systemd does not create the parent
folders of `PRIORY_DB` and `HERDR_BUS_DB`; create them first.

| Action        | Command                                                         |
| ------------- | --------------------------------------------------------------- |
| Start now and at login | `systemctl --user enable --now priory-intake priory-daemon` |
| Status        | `systemctl --user status priory-intake priory-daemon`           |
| Logs          | `journalctl --user -u priory-daemon -u priory-intake -f`        |
| Restart       | `systemctl --user restart priory-daemon`                        |
| Stop          | `systemctl --user stop priory-daemon priory-intake`             |

Without lingering, user services stop when your last session ends. On an always-on machine, run
this once, which changes the machine and so is yours to run:

```sh
loginctl enable-linger "$USER"
```

The daemon unit sets `KillMode=process`. With systemd's default, `control-group`, stopping or
restarting the unit would also kill the tmux server the daemon started, since tmux stays in the
unit's cgroup, and with it every agent. `process` signals only the daemon, so the agents keep
running and the new daemon reconnects to them. Systemd logs `Unit process ... (tmux: server) remains
running after unit stopped`, which is expected. `systemctl --user stop` therefore leaves agents
running; end one with `tmux kill-session -t =priory-<task id>` when you mean to.

## Listings

`priory inbox` prints each entry, oldest first: seq, type, subject, task and outcome.
`priory tasks` prints each task in start order: id, key, workflow, step, state, band, lane, reason,
position, the task's place in the queue from 1 when it is `queued` or `ready`, generation, and when
it retired, for a retired task (see Retiring asleep tasks).
`priory tasks --lane L` prints only the tasks in lane `L`.
`priory runs` prints each run: id, task, step, tmux session, harness session (a Codex run's
thread), started, ended and how it ended. `priory outbox` prints each prompt: id, task, run, state and text, with newlines
shown as `\n`. `priory asks` and `priory waits` are described under Tools, `priory memory` under Memory gate, `priory actions` under Code steps,
and `priory desk` under Desk. They read `PRIORY_DB` and print `-` for an empty cell, tab-separated after a header
line.
