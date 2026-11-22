# Scores

`priory score` grades a task's finished runs with the factory's scorers, on demand. A scorer is a
file in `scorers/` ([the scorer reference](../reference/scorers.md)). No scorer runs by itself
after a run: a `command` scorer can take as long as the gate.

```sh
priory score [DIR] TASK [--scorer NAME] [--json]
```

`DIR` is the factory folder, `.` by default. The command scores the latest finished run of each
step of the task: the last run of that step whose turn ended with a result or an ask. `--scorer`
runs one scorer instead of all of them. It prints one row per run and scorer:

```text
run	step	scorer	verdict	reason
1	plan	gate	pass	the tests pass
1	plan	passed	fail	outcome ready, not passed
2	implement	gate	error	the result names no commit
2	implement	passed	pass	outcome passed
```

A verdict is `pass`, `fail`, `not applicable` or `error`, and `reason` is its line of why. `--json`
prints `{"scores": [...]}`, each entry with the same five fields. The command fails when the task
does not exist, has no finished run, or the factory has no scorer of that name.

## A `command` scorer's checkout

A `command` scorer never reads the task's worktree, since a later run may be editing it. The
command adds a detached worktree of the result's `commit` from Priory's clone, under
`$PRIORY_HOME/scores/`, runs the scorer there and removes it. So a scorer of an earlier run sees
that run's commit, whatever the branch holds now. The commit must still be in the clone.

The agent writes a result's `commit`, so the command checks it before git reads it: a value that
is not 7 to 64 hexadecimal characters answers `error: the result's commit "<value>" is not a
commit hash`. The scorer's `run` must resolve inside the factory folder. One that leads outside,
as through a symbolic link, answers `error` and never runs.

The scorer runs in its own process group, with a cleared environment that keeps `PATH` and `HOME`
and adds `PRIORY_SCORE_BASE`, `PRIORY_SCORE_HEAD` and `PRIORY_SCORE_RESULT`. Past its `timeout`,
Priory kills the group and the scorer answers `error: timed out after <n>s`. Exit 0 passes and
exit 1 fails, with the last line of the scorer's output as the reason. Any other exit is an error.

## Records

Each score appends one `factory.run.scored` record to the trace, as [the trace
schema](../trace-schema.md#factoryrunscored) defines it. A score is a record: scoring a run again,
with the same scorer or a changed one, appends another record and replaces none.

## Where scores show

`priory task ID --json` and the web view's task page list each run's latest score by each scorer,
under the run. The Costs panel and `priory costs --json` count each scorer's verdicts in the window,
by the same split as the cost. The panel's pass rate is `pass` over `pass` and `fail`: an `error`
means the scorer broke, not the run, so it stays out, as does `not applicable`.
