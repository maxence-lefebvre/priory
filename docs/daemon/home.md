# Priory home

Priory home is the folder where Priory keeps its clones and worktrees. The daemon's socket is
named by it but lives elsewhere (see [Socket, spool and lock](#socket-spool-and-lock)).
`PRIORY_HOME` names it. When `PRIORY_HOME` is unset or empty, Priory uses
`$XDG_STATE_HOME/priory-oss`, else `~/.local/state/priory-oss`.

The state database is not in Priory home. `PRIORY_DB` names it (see [README.md](README.md)).

## What it holds

For a factory that works in `owner/name`, Priory home looks like this:

```
$PRIORY_HOME/
  daemon.lock                  one daemon per home
  signals.spool                signals sent while no daemon listened
  repos/owner/name/            Priory's clone of owner/name, with no checkout
  partial/owner/name/          a clone in progress, renamed into repos/ once complete
  worktrees/owner/name/3/      the worktree of task 3, on branch priory/3
  prompts/                     Codex first prompts (see codex.md)
  handlers/                    where stall handler calls run (see health.md)
  summaries/                   where summarizer calls run (see search.md)
  models/                      the embedding model's files (see search.md)
  push/                        the Web Push key pair and the requests in flight (see serve.md)
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

Hooks report to the daemon's socket (see [signals.md](signals.md)). The MCP tools and commands such
as `priory answer` use it too.

The socket is not in Priory home. A Unix socket path of more than 108 bytes cannot be bound, and
`PRIORY_HOME` can be as long as a worktree path. So the socket is at a short path that every client
derives from `PRIORY_HOME` the same way:

- `$XDG_RUNTIME_DIR/priory/<hash>.sock` when `XDG_RUNTIME_DIR` is set and not empty, and that path
  is under 108 bytes.
- `/tmp/priory-<uid>/<hash>.sock` otherwise.

`<hash>` is 16 hex digits of the FNV-1a hash of the canonical path of `PRIORY_HOME`, so two homes
never share a socket. The daemon makes the folder with mode `0700`. It refuses to start when the
folder exists but another user owns it, it is not a folder, or its mode lets anyone else in; the
error names the folder. Fix it with `chmod 700`, or remove it. A socket left by a dead daemon is
replaced at the next start.

The daemon passes its `XDG_RUNTIME_DIR` to the agents' panes and MCP servers, so a tmux server
started elsewhere does not change it. A command you run yourself must see the same `PRIORY_HOME` and
`XDG_RUNTIME_DIR` as the daemon, or it looks at another socket and reports
`no priory daemon listens on <path>`. The path is in that error. The daemon reads one connection at a time, and drops one that sends no
full line within 2 seconds, so a silent client cannot stop the others.

`signals.spool` holds the signals sent while no daemon listened. The next daemon applies them first.

`daemon.lock` allows one daemon per home. A second daemon waits up to 5 seconds for the lock, then
refuses to start.

### Push keys

The daemon makes `push/`, readable by its owner only, with the VAPID key pair that signs Web Push
notifications. `vapid.key` holds the private key, mode `0600`, written once and whole; `vapid.pub`
holds the public key that `GET /api/push/key` serves, written again from it on every start. Delete both files
to make a new pair: every browser then has to turn notifications on again. The private key never
leaves the file: it is not logged, printed or sent. While a notification goes out, `push/` also
holds its request's header and body files, which the daemon removes once `curl` ends.

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
