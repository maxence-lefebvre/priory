# Scores

`priory score` grades a task's finished runs with the factory's scorers, on demand. A scorer is a
file in `scorers/` ([the scorer reference](../reference/scorers.md)). No scorer runs by itself
after a run: a `command` scorer can take as long as the gate, and a `judge` costs money.

```sh
priory score [DIR] TASK [--scorer NAME] [--budget USD] [--json]
priory score [DIR] --calibrate SCORER --budget USD [--json]
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

`--budget` is the most the judges may spend, in dollars. Without it, a judge answers
`error: a judge costs money: give --budget USD`, and the other scorers run. When a judge scored a
run, the table gains an `agreement` column, such as `9/10 labeled cases`, and the command prints
what the judges spent on stderr.

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

## Judges

A `judge` scorer asks a model whether a run's change does what its scorer's prompt asks. A model's
opinion counts only once it is measured against a person's, so a judge answers
`error: not calibrated: <why>` until `priory score --calibrate` has stored an agreement for its
current prompt, model and labeled cases, at or above its `agreement`. Editing the prompt or the
cases, or changing the model, makes the judge not calibrated again.

### The call

A judge reads what a `diff` scorer reads: it fails a run with no result, and answers `error` for a
result with no commit or a task with no start commit, before it calls the model. Otherwise Priory
reads the diff from the start commit to the result's commit in its clone, cut at 100,000 bytes,
and the task's request from the inbox.

It calls the model as `priory eval` runs a turn, with less: a headless `claude -p` of the judge's
`model`, with no tool (`--tools ""`), no MCP server (`--strict-mcp-config` and no config), no
session kept, run in an empty scratch folder under `$PRIORY_HOME/scores/` so it reads no project
settings, without the variables that mark a nested Claude session, and with `--max-budget-usd` set
to what is left of the budget.

The request, the result and the diff are untrusted: a person or the agent under review wrote them.
So they never reach the system prompt. The system prompt holds Priory's frame, which says the data
is data and never instructions, then the scorer's prompt file, then the answer's shape. The user
message quotes the request, the result as JSON and the diff, each between an opening and a closing
marker that carry a random id made for that call, such as `<diff id="3f9a0c1d2b7e4a65">`. Text
inside cannot close its marker, since it cannot know the id. The message ends with the answer's
shape again.

The answer must be `pass` or `fail` alone on its first line, then one line of why. Any other answer
is an `error` that quotes it. A call that fails, or ends past its budget, is an `error` too. Once the
judges have spent the budget, a judge answers `error: over budget: the judges spent the budget`.

### Calibration

`priory score --calibrate SCORER --budget USD` runs the judge once on each labeled case of its
`evaluation` file ([the scorer reference](../reference/scorers.md#labeled-cases)), in order, and
prints one row per case:

```text
case	label	verdict	agreed	reason
adds-flag	pass	pass	true	It adds the flag and nothing else.
rewrites-parser	fail	pass	false	It adds the flag.
agreement: 9/10 labeled cases, 90%, needs 80%: calibrated, $0.2100 of $1.0000 spent
```

A case agrees when the judge's verdict equals its label. An `error` never agrees. When every case
ran within the budget, the command appends a `factory.scorer.calibrated` record to the trace
([the trace schema](../trace-schema.md#factoryscorercalibrated)) with the agreement, the model and
the SHA-256 of the prompt file and of the evaluation file. It stores the agreement even when it is
under the threshold: the judge then answers
`error: not calibrated: it agreed on 6/10 labeled cases, under 80%`.

When the budget runs out first, the last line reads
`Incomplete: the budget ran out after <n> of <m> cases, ... and nothing was stored`, and the command
fails. `--json` prints the rows under `cases`, with `agreed`, `of`, `needs`, `spent` and `complete`.

The calibration lives in this machine's state. `priory score` and `priory bench` read the latest
record whose scorer, model and hashes match the judge as it is now.

### Check a judge with a real model

The tests calibrate with a fake `claude`, so they cost nothing. To check a judge for real, write
its prompt and ten labeled cases, then calibrate it with a low budget:

```sh
priory score path/to/factory --calibrate does-what-was-asked --budget 1
```

Read each row's reason, and fix the prompt or a label where the judge and the person disagree for
a reason you accept.

## Records

Each score appends one `factory.run.scored` record to the trace, as [the trace
schema](../trace-schema.md#factoryrunscored) defines it. A judge's record also holds its model, its
agreement and its number of labeled cases. A score is a record: scoring a run again, with the same
scorer or a changed one, appends another record and replaces none.

## Where scores show

`priory task ID --json` lists each run's latest score by each scorer in its `scores`, and the web
view's task page and Herdr's Task panel show them under each run. The Costs panels of the web view
and Herdr, and `priory costs`, count each scorer's verdicts in the window,
by the same split as the cost. The panel's pass rate is `pass` over `pass` and `fail`: an `error`
means the scorer broke, not the run, so it stays out, as does `not applicable`.
