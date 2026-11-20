# Priory home

Priory home is the folder where Priory keeps its clones, worktrees and the daemon's socket.
`PRIORY_HOME` names it. When `PRIORY_HOME` is unset or empty, Priory uses
`$XDG_STATE_HOME/priory-oss`, else `~/.local/state/priory-oss`.

The state database is not in Priory home. `PRIORY_DB` names it (see [README.md](README.md)).

## What it holds

For a factory that works in `owner/name`, Priory home looks like this:

```
$PRIORY_HOME/
  daemon.lock                  one daemon per home
  priory.sock                  the socket that hooks, tools and commands talk to
  signals.spool                signals sent while no daemon listened
  repos/owner/name/            Priory's clone of owner/name, with no checkout
  worktrees/owner/name/3/      the worktree of task 3, on branch priory/3
  prompts/                     Codex first prompts (see codex.md)
  handlers/                    where stall handler calls run (see health.md)
  summaries/                   where summarizer calls run (see search.md)
  models/                      the embedding model's files (see search.md)
```

### Clones

`repos/<owner>/<name>` is a clone of `https://github.com/<owner>/<name>.git`, made with no checkout.
Priory clones each repository once and adds one worktree per task from it.

To clone from somewhere else, such as an SSH remote or a local mirror, use git's own
`url.<base>.insteadOf` setting. For example:

```sh
git config --global url."git@github.com:".insteadOf "https://github.com/"
```

### Worktrees

`worktrees/<owner>/<name>/<task id>` is the task's worktree. It is on the branch
`priory/<task id>`, made from the remote's default branch. The task's `worktree` field names the
folder only when Priory created it.

A start reuses a worktree only when it belongs to Priory's clone and is on that branch.

The folder can be gone while the branch remains. That happens to a revived task whose worktree the
sweep removed, and to a retired task. The next start then adds the worktree back on that branch. A
code step does the same before its action runs. Before it adds the worktree back, Priory runs
`git worktree prune`, because git refuses to re-add a folder that a person deleted while the clone
still lists it.

### Socket, spool and lock

Hooks report to `priory.sock` (see [signals.md](signals.md)). The MCP tools and commands such as
`priory answer` use it too.

`signals.spool` holds the signals sent while no daemon listened. The next daemon applies them first.

`daemon.lock` allows one daemon per home. A second daemon waits up to 5 seconds for the lock, then
refuses to start.

## Command timeout

Every `git`, `gh` and `tmux` command that Priory runs gets its own process group. After 5 minutes,
Priory sends KILL to that whole group. Killing the group, not only the command, matters because a
helper that ignores TERM could keep the command's output open.

The daemon has one thread, so a hung fetch would stop every hook, tool call and answer. The timeout
prevents that. A timed-out command fails like any other, with `timed out after <n>s`.

`command_timeout` in `factory.toml` changes the limit. Raise it for a first clone of a large
repository:

```toml
name = "example"
command_timeout = "20m"
```
