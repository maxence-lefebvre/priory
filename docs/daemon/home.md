# Priory home

`PRIORY_HOME` names Priory's directory. Default: `$XDG_STATE_HOME/priory-oss`, else
`~/.local/state/priory-oss`. It holds:

- `repos/<owner>/<name>`: a clone of `https://github.com/<owner>/<name>.git`, with no checkout.
  Git's own `url.<base>.insteadOf` setting can point it elsewhere, such as an SSH remote or a
  local mirror.
- `worktrees/<owner>/<name>/<task id>`: one worktree per task, on branch `priory/<task id>`, from
  the remote's default branch. The task's `worktree` names it only when Priory created it. A start
  reuses a worktree only when it belongs to Priory's clone and is on that branch. If the folder is
  gone but the branch remains, as for a revived task the sweep cleaned up or a retired task, the
  start re-adds the worktree on that branch. A code step does the same before its action runs. It
  runs `git worktree prune` first, since git refuses to re-add a folder a person deleted while the
  clone still lists it.
- `priory.sock`: the socket hooks report to. `signals.spool`: signals sent while no daemon
  listened, which the next daemon applies first.
- Every `git`, `gh` and `tmux` command runs in its own process group. After 5 minutes, or after
  `command_timeout` in `factory.toml`, Priory sends KILL to that whole group: a helper that ignores
  TERM could keep the output open. Set `command_timeout = "20m"` for a first clone of
  a large repository. The daemon has one thread, so a hung fetch would stop every hook, tool call
  and answer. A timed-out command fails like any other, with `timed out after <n>s`.
- `daemon.lock`: one daemon per home. A second waits up to 5 seconds for the lock, then refuses
  to start.
