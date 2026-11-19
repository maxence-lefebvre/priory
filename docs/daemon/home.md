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
  start re-adds the worktree on that branch. A code step does the same before its action runs.
- `priory.sock`: the socket hooks report to. `signals.spool`: signals sent while no daemon
  listened, which the next daemon applies first.
- `daemon.lock`: one daemon per home. A second waits up to 5 seconds for the lock, then refuses
  to start.
