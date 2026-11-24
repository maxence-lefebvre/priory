# Sandboxes

A workflow can start each of its runs' agents in a sandbox: a bubblewrap jail around the task's
worktree. A jailed agent sees its worktree and the few files it needs, and not the rest of the
person's files or the daemon's variables. Other workflows' runs start as before. To turn a sandbox
on, follow [How to jail a workflow's agents](../jailing-a-workflow.md). The design and the slices
after this one are in [the design note](../design/sandboxes.md).

## A workflow turns a sandbox on

`factory.toml` declares each sandbox by name in
[`[sandboxes.<name>]`](../reference/factory.md#sandboxesname), and a workflow names one with
[`sandbox`](../reference/workflows.md#top-level-keys):

```toml
# factory.toml
[sandboxes.jail]
kind = "bwrap"
read = ["~/.cargo", "~/.rustup", "~/.gitconfig"]
write = ["~/.cache/sccache"]
env = ["RUSTC_WRAPPER", "SCCACHE_DIR"]
```

```toml
# workflows/implement.toml
sandbox = "jail"
```

Every model step of the workflow starts its agent in the jail, debate turns included. A shared
agent's run follows the workflow of the task it serves, so one conversation can run jailed for one
task and not for another. A lead's or an investigator's task follows the workflow of the task it serves,
since its prompt holds that task's agent's words: the lead that answers a jailed task's ask, and
the investigator of its `consult`, start in the same jail with the same binds. A lead or an
investigator of an unjailed task starts as before.

A helper in the jail sees its own worktree, not the person's files. An investigator's MCP servers
start inside it too, so a server that reads files under the home, such as its login or its package
cache, needs them in the sandbox's `read` or `write`. Name a login read-only in `read`. Code steps, recipes, `record` commands, a human step's `command` and the
compaction of an idle shared session stay on the host: they are the factory's own commands, not the
agent's.

## What a jailed run sees

Every path is bound at the same path inside as outside. A path a hook sends, a path a person
trusted and a path the sweep reads in `/proc` then mean the same file on both sides.

| Path | Inside the jail |
| ---- | --------------- |
| `/` except `/home`, `/root`, `/tmp` and `/run/user` | read-only |
| `/home`, `/root`, `/tmp`, `/run/user` and `$HOME` | a new empty folder in memory, with only the binds below |
| `/dev` and `/proc` | the host's devices, and the jail's own processes |
| the task's worktree | read-write |
| the clone's git folder under `repos/` | read-write, since a commit writes there |
| the clone's `config`, `config.worktree`, `commondir` and `hooks`, its `worktrees` folder, and the links from the task's worktree to them | read-only, except the task's own folder in `worktrees` |
| Claude Code's home: `CLAUDE_CONFIG_DIR`, else `~/.claude` and `~/.claude.json` | read-only: the login, settings, plugins and trusted folders |
| its `projects` folder | read-write: the transcripts |
| its `session-env` and `shell-snapshots` folders | a new empty folder in memory |
| Codex's home: `CODEX_HOME`, else `~/.codex` | read-write: the login, sessions and trusted folders. Priory's trust profile in it, `priory-trust.config.toml`, is read-only |
| the folder of the state file `PRIORY_DB` names, and its link's target folder | a new empty folder in memory, with only the binds below it, even where a bind above names the same folder |
| the run's MCP configuration and system prompt files, Priory's binary, the factory's skills and memories folders | read-only |
| the harness's program, when it lives under a hidden folder such as `~/.local/bin` | its folder read-only, and its link |
| for a run that starts under direnv ([sessions.md](sessions.md#with-trust_worktrees)): the run's `direnv/<session>/` folder in Priory home, the person's `direnvrc` and `lib` it links to, and the `direnv` program as the harness's | read-only |
| the daemon's socket folder | read-write |
| `spools/<session>/` in Priory home, the run's own spool | read-write |
| the sandbox's `read` and `write` paths | as declared, skipped when missing |

A write anywhere else lands in the jail's own folders in memory, and is gone when the run ends.

The read-only paths are the ones a process on the host runs code from. Git runs the hooks, the
`core.sshCommand`, the filters and the credential helpers that the clone's `config` names. A
worktree's `.git` file and any git folder's `commondir` lead git to that `config`, so the jail
writes the clone's `commondir` as `.` and its `config.worktree` files empty when they are missing,
then binds them. A file that exists keeps its content.
Claude Code runs the hooks, plugins, MCP servers and scripts that its settings and
`~/.claude.json` name. Its shell sources the scripts in `session-env` and `shell-snapshots`, so a
run writes its own in memory. The daemon's git also runs with `core.hooksPath=/dev/null` and
`core.fsmonitor=false` in the clones under `repos/` and their worktrees, jailed or not.
Outside a jail an agent can write the clone's `config`, and its `core.sshCommand`, filters and
credential helpers can then run commands as the daemon.

The pane's command is the [agents slice's](agents-slice.md) scope, then `bwrap` with these binds
and `--die-with-parent`, `--new-session` and `--unshare-pid`, then a shell that clears the
variables, then the harness as before. The scope comes first, so the jail and every process it
starts stay in `priory-agents.slice`. The network is shared.

## What a jailed run keeps

The shell in the jail runs `env -i` with only these variables, each when it is set:

- `PATH`, `LANG`, `TERM`, `HOME` and `XDG_RUNTIME_DIR`;
- `PRIORY_HOME`, `PRIORY_SESSION`, `PRIORY_SPOOL` and the variables Priory gives the run, such as
  `PRIORY_GUARD`, `PRIORY_PRIVATE` and `OTEL_RESOURCE_ATTRIBUTES`;
- `CLAUDE_CONFIG_DIR` or `CODEX_HOME`, which move the harness's own home;
- `DIRENV_CONFIG`, for a run that starts under direnv;
- the sandbox's `env`.

So a jailed run holds none of the daemon's other variables: not a webhook secret and not a cloud
key. The run's MCP configuration sets `PRIORY_DB` for `priory mcp`, but the jail empties the state
file's folder, so the file is not there. A state file directly in `/` blocks the jailed task, since
the jail cannot hide it. `priory check` refuses an `env` that names a variable the factory reads a
secret from, such as `PRIORY_TOKEN`.

## Hooks, tools and the spool

Hooks and the `priory` MCP server run `priory` inside the jail, and reach the daemon over its
socket, whose folder is bound in. The tool guard, the signals and the tool calls work as for an
unjailed run.

A hook that finds no daemon appends its line to a spool. A jailed run cannot see `signals.spool`,
so `PRIORY_SPOOL` names its own spool, `spools/<session>/signals.spool` in Priory home. The next
daemon applies `signals.spool`, then each run's spool (see [signals.md](signals.md)).

## When the runner cannot jail

When any workflow turns a sandbox on, the daemon checks once at start that
`bwrap --ro-bind / / true` exits 0, as it checks `systemd-run` for the slice. On a runner where it
fails, a jailed task never starts unjailed. It blocks with:

```
workflow implement needs sandbox jail, and this runner cannot start bubblewrap: <bwrap's error>
```

On a runner other than Linux, the error is `bubblewrap needs Linux`. The desk shows the task, and
tasks of other workflows run. A person can install bubblewrap, restart the daemon and resume the
task, give it to another runner with `priory give`, or remove `sandbox` from the workflow.

`priory check` refuses a workflow that turns a sandbox on when `bwrap` is not on `PATH`. The
daemon does not: it blocks each jailed task, so a runner without bubblewrap still runs the others.

## What a run records

`priory runs` shows the sandbox of each run in its last column. Each model step's
[`factory.step.ended`](../trace-schema.md#factorystepended) record carries `sandbox`, and
`session_start_ms`, the time from the run's launch to its `SessionStart`, for jailed and unjailed
runs alike. The state file stores the sandbox in `runs.sandbox`, added by schema 107.

## What the jail does not stop

A jailed run reads and writes only its worktree, its clone, its harness's own home and the paths
the factory names, and holds only the variables it names. It does not stop:

- **The network.** The run reaches any host. It cannot read the person's keys, but it can send out
  what it can read: the worktree and the harness's login.
- **The socket.** The daemon's socket is bound in, and a line written to it by hand is a call no
  guard reads, as [guard.md](guard.md#how-a-command-is-read) says.
- **The clone.** The run can write other tasks' refs and objects in the shared clone.
- **The harness's home.** The run can read its harness's login, and read and write every session's
  transcript in it.
- **Codex's home.** A Codex run can write `config.toml`, which names the commands Codex runs.
- **The worktree.** Code steps, and a person who opens the worktree, run what the run wrote there,
  including its `.claude/settings.local.json`.

Known limits:

- `--new-session` detaches the agent from the pane's terminal, so a terminal signal such as a
  resize does not reach it. Attach and `priory screen` still show its pane.
- Only Linux gets a jail.
- The tests run the jail with a fake `claude`, not with Claude Code or Codex.
- Claude Code cannot save its settings, its plugin caches, `~/.claude.json` or a refreshed login
  inside the jail. It runs on the login it started with.
- The run's git cannot change the clone's config, so `git push -u`, `git remote add` and
  `git branch --set-upstream-to` fail inside the jail.
