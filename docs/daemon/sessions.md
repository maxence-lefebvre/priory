# Sessions

## Shared sessions

An agent's `session` is `per-task` (the default) or `shared`. A `per-task` agent gets one session
per task, as above. A `shared` agent, such as a lead, keeps one conversation across every task it
serves. The state stores it per agent. After a run reaches `SessionStart`, every later run of the
agent, for any task, resumes it with `--resume`. Until then, a new run starts a new session and records
it as the agent's.

A session runs one turn at a time, so a shared agent has at most one live run. A task needing the
agent while another task's run of it is live stays `queued` or `ready`, with the reason
`waits for agent <name>: its shared session serves task <id>`, which `priory tasks` shows. It starts
when that run ends by its result, ask, wait or a verdict. Then the room is checked, and a task
behind it that needs another agent can start ahead of it.

Each task keeps its own worktree and branch, and the resumed session starts in the task's worktree.
One worktree for the agent would put every task's commits on one branch and leave the sweep no task
to judge by. Each new run of a shared agent starts its task's first queued prompt with
`This is task <id> (<key>), from trigger <trigger>, in the worktree <path>. Earlier turns of this
conversation may belong to other tasks.` That lets the conversation tell its tasks apart, even after
a crash cut a prompt short.

A session an agent once shared stays shared. When the factory turns the agent back to `per-task`,
each task starts a new session instead of resuming the shared one.

Known limits:

- A long conversation is compacted by Priory only when its agent sets `compactAt` or
  `idleCompactAt`, or calls the `compact` tool. See Compacting a long session and Compacting an
  idle session.
- Claude stores a session's transcript under its starting directory. Whether `--resume` finds it
  from another task's worktree depends on the Claude version. No test runs a real Claude.
- Signals and tool calls name the harness session, not the process. The daemon kills a run's tmux
  session before admitting the next task, but a process that survived the kill could still report
  for the run that resumed its session.

## Compacting a long session

An agent's `compactAt` is a context size in tokens, such as `150000`. When a turn ends with the
conversation's context at or past it, the daemon queues a `/compact`, as the `compact` tool does. It
types it once the run is idle, then the same continuation line. [outbox.md](outbox.md) describes
that path under Compact. Unset, Priory never compacts on its own.

The context size is a count, not a fraction of the model's window: Priory does not know each
model's window.

The daemon reads the size at the turn's `Stop` hook, from the session transcript that the hook names.
It reads it at a resumed run's `SessionStart` too, so a session past `compactAt` is compacted after
its new task's first turn.
It is the input, cached input, cache writes and output of the last message the model wrote. An
error message Claude writes itself, marked `<synthetic>`, does not count. So the size lags by
one turn: a turn that passes `compactAt` is compacted after it ends, never during it. Claude can
still compact a turn itself if it fills the window first.

It compacts once per crossing. After the daemon queues a `/compact`, it queues no other until a
turn's context reads below `compactAt` again. Every run of the session shares that state, so a
compaction that leaves the context above `compactAt` is not repeated, even after the task sleeps
and wakes.

While the daemon's `/compact` is queued, a turn's ask or wait does not settle: the task sleeps on it
at the first `Stop` after the compaction and its continuation. So a lead that ends every turn with
an ask or a wait is still compacted before it sleeps. A task whose answer's command still runs is
neither compacted nor kept awake: it waits for the command, and a later turn compacts. A turn whose
context reads below `compactAt` before the `/compact` is typed withdraws it. A `/compact` the agent
asks for with the tool keeps the tool's own rules.

A turn that stores a result is compacted only if the next step carries on in the same process. A
result that moves the task to another conversation leaves the crossing to the session's next run,
which checks at its `SessionStart`. Its first `result` is then refused until the compaction, as for
the `compact` tool.

A Codex agent takes `compactAt` too. Its context size is the input and output of the last model
call in Codex's session file (see [trace.md](trace.md)), and its compaction ends at Codex's
`PostCompact` hook (see [codex.md](codex.md)).

Known limit: an answer that reaches a run kept awake for its `/compact` can start its command
before the compaction, so the continuation may come while the command runs.

## Compacting an idle session

A shared session waits between tasks with no process. Anthropic's prompt cache keeps a
conversation for 5 minutes, or for an hour with its longer lifetime (Anthropic's prompt caching
docs, `ttl: "1h"`). Priory assumes the hour. A task
that resumes the session after it reads the whole context again at full price. An agent's `idleCompactAt` is a context
size in tokens, such as `60000`. When its shared session has been idle for 55 minutes and its
context is at or past `idleCompactAt`, the daemon compacts it while the cache still holds it. Unset,
Priory never compacts an idle session.

The 55 minutes count from the last hook of any run of the session. They leave five minutes of the
hour for the launch, the resume and the `/compact`. A session idle past the hour, as after a daemon
stop, is left alone: its cache is gone, so a compaction would read the context at full price too.

The daemon reads the context size from the session's transcript, as for `compactAt`. Then it:

1. resumes the session in a tmux session named `priory-idle-<agent>`, in the worktree of its last
   run, with the same flags as the agent's runs, so the resume can reuse the cached conversation,
2. types `/compact` once Claude's `SessionStart` arrives, unless the hour since the session's last
   hook has passed: then the cache may be gone, and it kills the tmux session instead,
3. kills the tmux session at the `SessionStart` with `source: compact`.

It decides once per idle stretch. It skips a session below `idleCompactAt`, one whose last
worktree is gone, and one whose agent changed since the session started (see When the agent
changes), since a resume with other flags may not reuse the cache. A run of the session that starts
after the decision makes its next idle stretch count again.

It never compacts a session with a live run, so it never compacts a busy one. While the compaction
runs, a task that needs the agent stays `queued` or `ready` with the reason
`waits for agent <name>: its shared session is being compacted`. A compaction that has not ended
after 10 minutes is killed. A daemon restart kills the compactions it finds live.

`priory check` refuses `idleCompactAt` on a Codex agent and on an agent whose `session` is not
`shared`. A resumed Codex session sends no `SessionStart` before its first prompt, and an idle
compaction types no prompt, so the daemon would never know when to type `/compact`.

Known limits:

- Priory does not read which cache lifetime Claude used. A session cached for 5 minutes, as with
  some API keys, is cold by the time the daemon compacts it.
- Priory does not check that the resume reused the cache, and counts no tokens of an idle
  compaction toward any task.
- A compaction starts without the room check, so it uses memory that the queue does not count.
- When the launch of a resumed run fails, the line about the change stays in that task's prompt,
  and another task that resumes the session first does not get it.

## When the agent changes

Each run of an agent gets the agent's current instructions, model and permissions as flags. A
shared session resumed days later still carries earlier turns that followed the old ones. So the
daemon records what a shared session started with: a digest of the agent's prompt, its tools
(`mcpServers`, `permissions.allow`, `permissions.deny` and `connectorSignIn`), its model and effort, and its flags
(`permissions.defaultMode`).

When a task resumes the session and a digest differs, the task's first prompt starts with one line
that names the parts that changed, before the line that names the task:

```
Your agent definition changed since this conversation started: its prompt, model.
```

The daemon then records the new digests, so the next task hears nothing. An unchanged agent adds
nothing to the prompt.

An agent with `onChange = "fresh"` starts a new shared session instead of resuming the old one.
`priory check` refuses `onChange = "fresh"` on an agent whose `session` is not `shared`.

A session recorded before Priory kept digests has none: it is resumed as unchanged, and its next
run records them.

## When a resume never starts

A Claude Code run that resumes a session can show the conversation and an idle prompt without
running Priory's hooks. Claude 2.1.330 did this on some large sessions: it kept the session id and
wrote to the same transcript, but sent no `SessionStart`. Resuming the same session again does the
same, while a new session starts. So the daemon does not block such a run.

When a run that resumed a session sends no `SessionStart` within its window (see
[By hand](#by-hand)), its pane shows Claude's input box and its slice was not throttled, the daemon:

1. ends the run as `unresumed` and kills its tmux session,
2. repairs the task with the action `new session` (see [health.md](health.md#repair)), which
   appends a `factory.task.repaired` record whose `cause` names the old session's transcript, and
3. makes the task `ready` with this prompt first, before the prompts the task already queued:
   `Priory repaired this task after: claude showed no SessionStart within 60s when Priory resumed
   session <id>. Cause: it did not start the resumed session; its transcript is <path>, so read its
   end for where you were. Action: new session. Check the worktree, then continue.`

The task's next run starts a new session, and later runs resume that one: a session with an
`unresumed` run is never resumed again, by a debate either. For a `shared` agent, the new session becomes the
agent's shared session, as for a first run. The desk lists the repair under Since you looked as
`factory.task.repaired <step>: ready, new session after <cause>`, and the daemon log names the run.

A resume whose pane shows no input box, or in a throttled slice, blocks its task instead: it may only
be slow. A new session that also sends no `SessionStart` within its window blocks its task as described
under Trusting the worktrees and the clone. A resume that follows a repair and stored no result,
ask or wait fails the task instead, as a second failure after a repair does.

Known limits:

- A resume that would have started just past its window is replaced too. The new session reads the
  old transcript at full price instead of reusing its prompt cache.
- The new session has only the old transcript and the task's prompts to go on. It reads the
  transcript itself, and a long one costs tokens.
- A Codex run is not covered: Priory does not recognize Codex's prompt, and a resumed Codex session
  sends no `SessionStart` before its first prompt (see [codex.md](codex.md)).
- Priory does not know why Claude skipped the hooks.

## Attaching to a run

`priory attach TASK` attaches your terminal to the tmux session of the task's live run, so you see
the agent's screen. `priory attach --run RUN` names one run instead. It reads the state file, finds
the run's tmux session on the tmux server your `TMUX` and `TMUX_TMPDIR` name, and checks that a live
pane there carries the run's `@priory` tag. Then it runs
`tmux -S <socket> attach-session -t '=priory-<task id>'` in your terminal, with that server's
socket. Detach with tmux's
detach key, `C-b d` by default. tmux keeps owning the session, so the agent goes on after you
detach.

- `--read-only` adds tmux's `-f read-only,ignore-size`, which `-r` abbreviates: you watch without
  typing into the agent. Your terminal sizes the agent's window only while no client on the tmux
  server can type. Such a client does not protect the run from its
  [budget](health.md#budget), a [spend cap](spend.md) or an [urgent wake](room.md#what-is-never-stopped).
- `--print` prints the attach command instead of running it, so a view host can run it in a pane,
  such as `hc run -- <that command>` in a Herdr pane.

Without `--read-only`, attach first sends the daemon one line, which appends a
`factory.terminal.attach_issued` record with the task, the run and whether it printed or ran the
command. It records that someone asked for a command that can type, not that a client attached nor
what it typed: a printed command can run later or never, and tmux does not report a client's keys.
Without a daemon it says `priory attach: not recorded: no priory daemon listens; attaching anyway`
on stderr and attaches.

A task with no live run, a run that ended, or an unknown id exits with 1 and names the state, such
as `priory attach: task 1 is dropped, with no live run` or `run 3 ended as cancelled`. A session
missing from your tmux server, or one whose panes carry no live tag of the run, exits with 1 and
asks you to run attach with the daemon's `TMUX` and `TMUX_TMPDIR`, so you never attach to another
server's session of the same name.

The daemon sets a run's idle flag and turn from hooks, tool calls and its own typing of a prompt.
It reads no tmux client for them, so attaching and detaching change no run's idle flag, turn or prompt. A client that can type can still type into the
agent, as a person at the pane would: the daemon then sees that turn's hooks like any other.

`priory screen [DIR] TASK` prints the same pane's screen once, and exits. `DIR` names the factory
whose secret variables it hides. `--run RUN` names one run, `--history N` adds rows of scrollback, and
`--json` prints the object [`/api/runs/{id}/screen`](../reference/serve.md#run-screen) returns.
It refuses the same runs, with the same sentences as attach.

Known limits:

- tmux refuses to attach from inside another tmux client unless you unset `TMUX` first.
- An attach without `--read-only` resizes the agent's window to your terminal.

## Opening a run when it starts

A model step with `attach = true` opens each of its runs where a person can watch it, through the
factory's `[runs] attach` command. Priory never calls a view host itself. The command is the view
host's adapter, such as [`adapters/herdr/attach`](../adapters/herdr.md#opening-runs-in-herdr).
Debate steps count as model steps, so each debate turn's run gets the command too.

After the daemon starts a run's harness in tmux, it runs the command once, in the factory folder.
The command gets the task id and the run id as its arguments, and as `PRIORY_TASK` and
`PRIORY_RUN` in its environment, with the run's tmux session as `PRIORY_TMUX` and the daemon's
executable as `PRIORY_EXE`. It also gets the daemon's own environment, so `PRIORY_DB`,
`PRIORY_HOME` and `TMUX_TMPDIR` name the daemon's state and tmux server.

The daemon starts the command, then goes on with its loop. It never holds or fails the run:

- A command that exits with a status other than 0, or that runs past 30 seconds, gets a
  `factory.run.attach_failed` record with the reason and the tail of its output. The daemon kills
  it at 30 seconds.
- A command that cannot start, such as a file that was removed, gets the same record with the
  error.
- A run whose harness did not start runs no command.

Each new run of the step runs the command once. Each new run also starts a new tmux session, so a
terminal attached to the old run has nothing left to show. A wait inside a run starts no new run,
so it runs no command.

Known limits:

- A daemon that restarts while a command runs does not check that command's exit status, stop it
  at 30 seconds or clean up its log, `attach/<run>.log` in Priory home.
- The command opens a view, and nothing closes it. The view host's terminal for an ended run stays
  until a person closes it.

## Trusting the worktrees and the clone

Claude and Codex ask whether to trust a folder on their first run there, before any hook fires.
Priory never answers that dialog.

### With `trust_worktrees`

When the factory sets `trust_worktrees = true` (see [factory.md](../reference/factory.md#top-level-keys)),
the daemon records the trust itself before each run starts, so no run stops on the dialog. It
records it only for the run's worktree, `worktrees/<owner>/<name>/<n>`, and that worktree's clone,
`repos/<owner>/<name>`, both under Priory home. The repository must be one the factory names: a
trigger's `repo`, the sweeper's `repo` or a `[repos]` key. The daemon resolves both paths through
symlinks and `..` first. It writes nothing for a path that resolves outside those folders, such as
a worktree or clone that a symlink leads elsewhere.

- **Claude Code** has no flag or setting that skips the dialog. The daemon sets
  `projects["<path>"].hasTrustDialogAccepted` to `true` for the worktree and the clone in Claude's
  config, `$CLAUDE_CONFIG_DIR/.claude.json` or else `~/.claude.json`. It changes no other value,
  but it writes the file back with its keys in sorted order.
  Claude rewrites that file while sessions run, so the daemon writes a copy, and renames the copy
  over the file only when the file's inode, size and modification time are still the ones it read.
  When they changed, it reads the file again and redoes its change. After the rename, it reads the
  file back. It gives up after 3 tries. It writes nothing when both keys are already `true`, and
  refuses a config that is missing or is not a JSON object. When the config is a symlink, the
  daemon edits the file it points to and keeps the link.
- **Codex** ignores a `-c projects."<path>".trust_level` override, but reads a profile. The daemon
  sets `projects."<clone>".trust_level = "trusted"` in its own profile,
  `$CODEX_HOME/priory-trust.config.toml`, and starts the run with `-p priory-trust`. Codex keys
  a worktree's trust on its clone. The daemon never edits `config.toml`. It refuses the grant, and
  starts the run without `-p`, when `config.toml` marks the clone `untrusted`, sets a default
  `profile` that `-p` would replace, or cannot be read or parsed. It refuses a profile that is a
  symlink: a jailed Codex run can write `CODEX_HOME`, and a link there would make the daemon
  rewrite a file outside the jail. A refusal names a TOML error by its line number only, since
  the parser's message quotes the line, which can hold a token. The daemon creates the profile
  empty before its first grant, so a jailed Codex run always gets it read-only and cannot replace
  it. It also refuses a profile that is not a regular file, such as a FIFO. When the daemon cannot
  read a config back after a dialog, it blocks the task without the restart. Codex still runs only the hooks a person trusted with
  `/hooks`, so that step stays manual.
- **direnv** refuses a folder's `.envrc` until a person runs `direnv allow`. When `direnv` is on the
  daemon's PATH and the worktree has an `.envrc`, the daemon checks its content, as below, and writes
  a direnv config folder of its own for the run, `direnv/<session>/` in Priory home. It holds:

  - `direnv.toml`: the person's own (from `DIRENV_CONFIG`, else `$XDG_CONFIG_HOME/direnv`, else
    `~/.config/direnv`) without its `[whitelist]`, which skips direnv's content check.
  - `data/direnv/allow/`: direnv's allow file for the worktree's `.envrc` with the checked content.
    direnv matches the path it is given, and Priory home may go through a symlink, so the path as
    the daemon spells it and its real path each get one.
  - `envrc`: the checked bytes.
  - `direnvrc`: Priory's own. It runs the person's `direnvrc`, or else their `~/.direnvrc`, then makes
    direnv run `envrc` where it would read the worktree's `.envrc` a second time. It does that in the
    daemon's load alone, so an agent's own `direnv exec` runs the `.envrc` of the folder it names.
    `BASH_SOURCE` in the `.envrc` names the copy, `envrc` in the run's folder, and `$PWD` the
    worktree.
  - `direnvrc.person` and `lib`: links to the real paths of the person's, so a home-manager link into
    `/nix/store` still resolves in a jail and libraries such as nix-direnv still load.

  The run's harness then starts as `direnv exec <worktree> <harness>`, with `DIRENV_CONFIG` naming
  that folder, so every command the agent runs gets the `.envrc` environment, such as a nix flake's
  toolchain. direnv alone gets that `data` folder as its `XDG_DATA_HOME`: the `.envrc`, the person's
  `direnvrc` and the harness see the old one. The daemon never edits the person's `direnv.toml` or
  direnv's allow folder. It refuses the grant, and starts the run without direnv, when the person's
  `direnv.toml` cannot be read or parsed, when the person denied the `.envrc` with `direnv deny`, when
  `direnv/` in Priory home is a symlink, or when the `.envrc` fails the check below. The task's reason
  then says `started without its .envrc:` and why. The daemon deletes the folder once the run ended.
  Without `direnv` or without an `.envrc`, nothing changes. An agent's own `direnv exec . <command>`
  needs the person's own `direnv allow`, and sees `XDG_DATA_HOME` unset when the task's `.envrc`
  exports one.

  A worktree outlives its run. So without a check, an `.envrc` that one run's agent wrote would run
  at the start of the next, outside the tool guard and the harness's permissions. The daemon grants
  direnv only when the worktree's `.envrc` is a regular file with the same content as the one on the
  commit the task started from. direnv refuses an `.envrc` that changed after the check, and the run
  starts without it. direnv reads the file once to hash it and again to run it: a change between the
  two has no effect, since direnv runs the checked copy.
  That commit is the default branch's head, which the daemon reads from GitHub outside the clone:
  every worktree shares the clone's refs and objects, and an agent can write both. The daemon hashes
  the commit and its tree again before it compares, so a forged object does not pass.
  A stacked task is checked against the commit its bottom task started from, since its own base
  is another agent's branch. Failing that, the `.envrc` must match the one on the repository's default
  branch. The daemon reads that branch's head from GitHub, fetches the commit when the clone lacks it,
  and waits for each at most 10s. It hashes that commit and its tree again too.

  A run starts without its `.envrc`, although no agent wrote it, in these cases:

  - The `.envrc` is a symlink, or is over 1 MiB.
  - The `.envrc` is untracked, such as one that a setup recipe writes. The daemon does not ask GitHub
    for it. An untracked copy of the file on the commit the task started from passes.
  - The task changed the `.envrc`, or merged a branch that did. Its later runs start without it until
    the change is on the default branch.
  - The `.envrc` is an older honest version, from neither the commit the task started from nor the
    default branch, as after a reset to an earlier commit. A rebase on the default branch fixes it.
  - Git writes the file with other content than it stores, as `core.autocrlf` or a filter does.
  - A git command of the check fails. The reason then says `could not be checked` with the command and
    its exit code, such as `git hash-object: exit 139` for a crash, and the daemon's log has its error.

  The check has limits:

  - It covers `.envrc` alone. What the `.envrc` loads runs as the worktree has it, so an agent's edit
    of it runs at the next start: `flake.nix` and `flake.lock`, the caches in `.direnv/`, `.env`, a
    sourced script, a parent's `.envrc` through `source_up`, and programs in a `PATH_add` folder.
  - A process outside a jail that runs as the same user can write the run's `direnv/<session>/` folder
    itself, as it can write any file of the person's.
  - The copy depends on direnv's `source_env` function. A direnv that loads an `.envrc` without it still
    refuses a change before its hash, and runs one made after it.
  - The daemon's git reads the clone's config, which an unjailed agent can write. Its
    `core.sshCommand`, filters and credential helpers can run commands as the daemon, at any start.
  - A task that started before the daemon stored a base for this check has none, and neither has one
    that started while `trust_worktrees` was off, when the daemon does not ask GitHub. Their `.envrc`
    must match the default branch.

  The variables an `.envrc` exports
  reach the harness's environment as they are, outside the daemon's secret redaction, so keep
  secrets out of a repository's `.envrc`.

  The harness starts only after the `.envrc` loads: a `use flake` took 12s on a fresh worktree
  and under 1s after. direnv gets 600s to load it, and a run whose `.envrc` is still loading then
  blocks with a reason that names direnv. Once direnv hands over, the harness gets its usual window,
  60s or 180s with recorded trust, to reach `SessionStart`. When
  direnv itself fails before the harness starts, for example because the `.envrc` exits non-zero
  or `nix` is not on PATH in a jail, the harness starts without the `.envrc`. The daemon records
  direnv's error as a `failed` `factory.run.trusted` record and in the task's reason, which a
  later block reason keeps. A harness that exits on its own under direnv is not started again.

Each path gets a `factory.run.trusted` record in the trace, whose `outcome` is `granted`, `present`
or `refused` (see [trace-schema.md](../trace-schema.md#factoryruntrusted)).

A trusted folder lets the repository's own settings run, as a person's trust does: Claude's
`.claude/settings.json` hooks, `env` and `permissions.allow` rules, `CLAUDE.md` and skills, and
Codex's `.codex/config.toml`. The daemon does not compare those files with the commit the task
started from, as it does an `.envrc`: hooks that an agent wrote into `.claude/settings.json` run in
the next run on the same worktree, or on a branch stacked on it, outside the tool guard.
Those settings run beside Priory's guard, and a hostile one may
weaken it, for example with Claude's `disableAllHooks`. So turning the key on vouches for every
repository the factory names. A run without claude.ai connectors still gets `--strict-mcp-config`,
which keeps `.mcp.json` servers out. A run with connectors goes without that flag, and its guard
denies the tools of every server its agent does not name. Investigators load no project settings.

When a harness shows its dialog although the run's grant was `granted` or `present`, the daemon
reads the harness's config again. When the grant is gone, the harness rewrote its config from an
older copy. The daemon then blocks the task with the reason `<harness> asked whether to trust
<worktree>: it overwrote the trust Priory recorded before it read it`, and starts the run again
once, with the grant written again. A second loss fails the task. When the grant is still there,
the version most likely keys trust differently. The daemon then blocks the task at once with the reason
`<harness> asked whether to trust <worktree> although Priory recorded its trust; this version of
the harness keys trust differently. Trust the folder by hand, or turn trust_worktrees off`, and
writes a `factory.run.trusted` record with `outcome = "missed"` and the harness's `--version`
line. A run whose grant was refused gets the manual reason below instead. To check a new version
by hand, start it in a temporary HOME holding a copy of the grant, in a worktree of a clone, and
look for the dialog.

### By hand

Without `trust_worktrees`, Priory writes no harness config. Which trusted folder covers a task's
worktree depends on the Claude Code version. Tested with a worktree under `worktrees/` of a clone
under `repos/`:

| Claude Code | Trusting `repos/<owner>/<name>` | Trusting `worktrees/` |
| ----------- | ------------------------------- | --------------------- |
| 2.1.42 | shows the dialog | covers the worktree |
| 2.1.198 | covers the worktree | covers the worktree |
| 2.1.310 and 2.1.330 | covers the worktree | shows the dialog |

So a person trusts both, once: the worktrees folder before the first run, and each clone after
Priory created it.

```sh
cd "$PRIORY_HOME/worktrees" && claude              # accept the dialog, then /exit
cd "$PRIORY_HOME/repos/<owner>/<name>" && claude   # accept the dialog, then /exit
```

Each second, the daemon reads the screen of every run that has sent no `SessionStart`. When it
shows Claude's trust dialog, the daemon ends the run as `exited`, kills its tmux session and blocks
its task at once with the reason `claude asked whether to trust <worktree path>; Priory never
answers it. Trust the worktrees folder and the clone once: cd <worktrees path> && claude, accept,
exit, then cd <clone path> && claude, accept, exit`. The desk shows the blocked task with that
reason. After both folders are trusted, `priory resume` starts the task again.

A run that sends no `SessionStart` within its window, for another reason, also ends as `exited`.
The window is 60 seconds from its start, or 180 when Priory recorded the run's trust. Under direnv,
the `.envrc` gets 600 seconds to load, and the window counts from there. The daemon kills its tmux session and blocks its task with the reason `claude showed
no SessionStart within 60s; trust the worktrees folder and the clone once: ...`, the same commands,
`. Resume the task to try again`, then `. Its pane ended with: ` and the last 5 non-empty lines of its pane, joined by ` | `, so a
person sees the dialog or warning that stopped it. The daemon log gets the last 30. A line the
pane wrapped counts as one line, so a secret that wraps is hidden whole. Both hide secrets as the
screen does (see [serve.md](serve.md#a-runs-screen)), and show `[redacted]` alone when one still
shows.

The reason names a cause the daemon can see, the first of these that applies:

- Since the run started, the CPU quota of the [agents slice](agents-slice.md) kept back at least 10 %
  of the CPU its agents wanted: `priory-agents.slice was throttled: its quota kept back <n>% of the CPU
  its agents wanted since the run started, under CPUQuota=<quota>`. The share is the growth of
  `throttled_usec` in the slice's `cpu.stat`, over its growth plus that of `usage_usec`.
- Claude's pane shows its input box: `its prompt is ready and asks for no trust, so its SessionStart
  hook is late or never reached the daemon`.
- Priory recorded trust for each path of a Claude run, as `granted` or `present` (see
  [With trust_worktrees](#with-trust_worktrees)): `it started too slowly`.

At Claude's input box, or when Priory recorded a Claude run's trust, the cause replaces the commands,
so a person does not accept a trust that is not missing. Recorded trust also adds `although Priory
recorded its trust` after the window: `claude showed no SessionStart within 180s although Priory
recorded its trust; it started too slowly`. Elsewhere, as at Codex's `Hooks need review`, the
throttling follows the commands after `; `.

The daemon never restarts such a run by itself, since a restart changes nothing that made it slow.
The exception is under [When a resume never starts](#when-a-resume-never-starts).
`priory resume` starts it again.

The daemon's `runs` metrics group counts both as starts that ended in `error`, and shows a start
still waiting for `SessionStart` by its age. See [metrics.md](metrics.md#the-runs-group).

In both cases, its queued prompts stay queued. A prompt it was launched with returns to the queue,
so the task's next run gets them. A Codex run blocks with the Codex instruction under Codex.

Priory does not read Claude's trust store to warn ahead of a run. The store is Claude's private
file, and the rule that reads it changed twice in the versions above, so a check could call a
folder trusted while Claude still shows the dialog.
