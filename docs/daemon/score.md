# Scores

`priory score` grades a task's finished runs with the factory's scorers, on demand. A scorer is a
file in `scorers/` ([the scorer reference](../reference/scorers.md)). A scorer runs by itself only
where the factory names it as a gate: a step's `gate` and the factory's `critic` grade a result
before the daemon stores it (see [Gates](#gates)). No other scorer runs after a run: a `command`
scorer can take as long as the tests, and a `judge` costs money.

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

A judge on `diff` reads what a `diff` scorer reads: it fails a run with no result, and answers
`error` for a result with no commit or a task with no start commit, before it calls the model.
Otherwise Priory reads the diff from the start commit to the result's commit in its clone, and the
task's request from the inbox. A diff over 100,000 bytes is `error` (`the diff is over 100000
bytes`), with no call: a judge that read only part of a diff could pass what it never saw. A judge
on `result` reads the request and the result alone, so it needs no commit: a plan's critic is one.
Its system prompt then names no diff, and its message holds no diff block.

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

## Gates

A gate grades a result before the daemon stores it. A failing scorer refuses the result, and a
passing one only lets the task go on as it would without the gate. A gate approves nothing: an
irreversible step still needs a person's approval, and a `plan` task's grade still waits for a
person.

Two keys name gates:

- A model step's `gate` lists scorers by outcome
  ([workflows.md](../reference/workflows.md#step-keys)), such as
  `gate = { passed = ["tests", "does-what-was-asked"] }`.
- `critic` in `factory.toml` names one scorer on `result`
  ([factory.md](../reference/factory.md#top-level-keys)). It grades a plan no person reads: a
  `red`, `yellow` or `green` outcome that leads to a model or code step, of a task whose
  `intervention` is not `plan`. Under `plan`, and for a grade that leads to a human step, a person
  reads the plan, so the critic does not run.

### A held result

When the agent sends a result whose outcome a gate grades, the daemon checks it as it checks any
result, then holds it and stores nothing. The agent reads:

```text
result held: its gate grades outcome passed when this turn ends, and the task moves to done unless the gate refuses it
```

The scorers run on a thread of their own, so a slow `command` scorer never holds the daemon's
loop. They read the held result as `priory score` reads a stored one. While they grade, the run
stays live and idle, the task stays `running`, and:

- no prompt is typed into the run. A person's `say` or an event waits for the verdict;
- a `result`, `ask`, `wait` or `compact` call is refused with
  `run <id> holds outcome <outcome> while its gate grades it: end your turn, and Priory sends the verdict`;
- the stall handler leaves the run alone.

A held result counts as sent for the step's [`budget`](health.md#budget): when the turn ends, an
overrun stands, and a cost that cannot be read fails the task, as for a stored result.

### The verdict

| Scorer answers | In a gate |
| -------------- | --------- |
| `pass` or `not applicable` | Lets the result through |
| `fail` | Refuses it |
| `error` | Refuses it: a scorer that cannot grade the change cannot let it through |
| A judge whose calibration does not pass | Does not count. Its `error: not calibrated: <why>` is recorded, and the gate goes on without it |

A judge gates only while its latest calibration, for its current prompt, model and labeled cases,
is at or above its `agreement` ([Calibration](#calibration)). One gated call may spend five times
what one call cost on average when the judge was calibrated, at least $0.01.

When no scorer refuses, the daemon stores the result, checking it again first, and the task moves
on as its `Stop` would have moved it.

When a scorer refuses, the result is never stored. The first refusal in a run reaches the agent as
its next prompt, followed by how to end the step:

```text
The gate refused outcome passed. tests: two tests fail. Fix it, then send the result again. A second refusal in this run goes to a person.
```

The agent fixes it and sends a result again, which the gate grades again. A second refusal in the
same run blocks the task with `the gate refused outcome <outcome> twice: <reasons>`, and ends the
run as `refused`. The desk shows it, and a person resumes, revives or cancels the task
([desk.md](desk.md)). A resumed task starts a new run, whose first refusal reaches the agent
again.

A run that ends while its gate grades, such as one cancelled, revived, stopped for an urgent wake or
whose agent exited, drops the verdict: its scores are recorded, and nothing else changes. After a
daemon restart, a gate still grading starts its scorers again.

Each score of a gate appends a `factory.run.scored` record with `gate` set to the outcome it graded
([Records](#records)).

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
