# Scores

`priory score` grades a task's finished runs with the factory's scorers, on demand. A scorer is a
file in `scorers/` ([the scorer reference](../reference/scorers.md)). No scorer runs by itself
after a run.

```sh
priory score [DIR] TASK [--scorer NAME] [--json]
```

`DIR` is the factory folder, `.` by default. The command scores the latest finished run of each
step of the task: the last run of that step whose turn ended with a result or an ask. `--scorer`
runs one scorer instead of all of them. It prints one row per run and scorer:

```text
run	step	scorer	verdict	reason
1	plan	passed	fail	outcome ready, not passed
1	plan	short-ask	not applicable	the run asked nothing
2	implement	passed	pass	outcome passed
2	implement	short-ask	not applicable	the run asked nothing
```

A verdict is `pass`, `fail`, `not applicable` or `error`, and `reason` is its line of why. `--json`
prints `{"scores": [...]}`, each entry with the same five fields. The command fails when the task
does not exist, has no finished run, or the factory has no scorer of that name.

## A `command` scorer

A `command` scorer reads the result's `commit`. It never reads the task's live worktree, since a
later run may be editing it. For each run it scores, Priory:

1. Makes a new folder under `PRIORY_HOME/scores/`.
2. Clones Priory's clone of the task's repository into it with `git clone --shared`, and checks out
   the result's `commit` there. The commit still checks out when no branch names it any more.
3. Writes the result as JSON to a file in that folder.
4. Execs the scorer's `run`, with no argument and no shell, in the checkout. Its environment is
   cleared and holds only `PRIORY_SCORE_BASE`, `PRIORY_SCORE_HEAD` and `PRIORY_SCORE_RESULT`. It
   runs in its own process group.
5. Waits up to the scorer's `timeout`, then kills the process group.
6. Removes the folder by its exact path, whatever happened.

Exit 0 scores `pass`, exit 1 `fail`, and any other ending `error`. The reason is how it ended,
such as `exit 1` or `timed out after 1200s`, then the last line it printed, if any.

The command runs the scorers one after another in the foreground: a scorer that takes 20 minutes
holds `priory score` for 20 minutes.

## Records

Each score appends one `factory.run.scored` record to the trace, as [the trace
schema](../trace-schema.md#factoryrunscored) defines it. A score is a record: scoring a run again,
with the same scorer or a changed one, appends another record and replaces none.
