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
- `pre_review` in `factory.toml` names one judge that reads an approval step's lessons before the
  step asks a person (see [Pre-review](#pre-review)).

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
| The grading crashed | Refuses it, recorded as scorer `gate` with `error: the gate's grading crashed` |
| A judge whose calibration does not pass | Holds the result on an ask (see [A judge that lost its calibration](#a-judge-that-lost-its-calibration)), unless another scorer refuses it. Its `error: not calibrated: <why>` is recorded |

A judge gates only while its latest calibration, for its current prompt, model and labeled cases,
is at or above its `agreement` ([Calibration](#calibration)). One gated call may spend five times
what one call cost on average when the judge was calibrated, at least $0.01.

When no scorer refuses and every judge counts, the daemon stores the result, checking it again
first, and the task moves on as its `Stop` would have moved it.

### A judge that lost its calibration

Editing a judge's prompt or cases, or changing its model, leaves it not calibrated. A gate does not
pass a result that no calibrated judge read, and a judge that is not calibrated cannot refuse one.
So when no scorer refuses the result and a judge did not count, the daemon keeps the result held
and asks a person one question, on the Desk like any ask:

```text
The gate of step implement holds outcome passed of task 1: does-what-was-asked is not calibrated: <why>. Calibrate it with priory score --calibrate, then answer grade again, or answer let it through to store the result this once.
```

Its title is `gate held: <scorer> is not calibrated: <why>`, and its options are:

- `grade again`: the scorers grade the held result again. A judge calibrated since then counts.
  One that is still not calibrated asks again.
- `let it through`: the daemon stores the result as if the gate passed, this once. The next gated
  result asks again until the judge is calibrated.

While the ask is open, the run stays live and idle as during grading, and no scorer runs. A run
that ends while the ask is open drops the result, as for any gate, and withdraws the ask.

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

## Pre-review

A pre-review keeps a person from rejecting the same mistake twice. A lesson is a memory whose
[`step`](../reference/memories.md#frontmatter-keys) names an approval step: a human step with
`approves`, as `<workflow>/<step>`. A factory writes lessons in `memories/`, and the daemon
proposes one each time a person rejects an approval with a note.

### Where lessons come from

When a person answers an approval step's ask as `OPTION: NOTE`, and the option stops the task or
sends it back to a model step, the daemon writes a memory candidate scoped to that step, with the
note as its text (see [memories.md](../reference/memories.md#lessons)). For example:

```sh
priory answer 12 'rejected: the pull request has no test'
```

The candidate waits for a person, as an agent's `remember` does (see
[skills.md](skills.md#accepting-a-candidate)). The rejected task's later runs see it at once.
Other tasks see it once a person accepts it, which keeps a careless note out of every run.

### What the pre-review reads

When a model step sends a result whose outcome leads to an approval step, the daemon holds the
result for the `pre_review` judge, as for any gate, when all of these hold:

- `factory.toml` names `pre_review`;
- the task sees at least one lesson for that approval step: the kit's, the memories repository's,
  the factory's, or a candidate the task sees, as a run gets them;
- no pre-review refused a result of this run before.

The judge reads the request, the result and, on `diff`, the diff, as any judge does. After them,
between markers of their own, it reads the lessons, one per line as `<name>: <text>`. Only the
pre-review reads them, not the step's other gate scorers. The judge's prompt says what a repeat
is. A labeled case gives its own `lessons`, so calibration measures the same question (see
[scorers.md](../reference/scorers.md#labeled-cases)). Like any judge in a gate, it counts only
while its calibration passes. A pre-review that is not calibrated raises no ask, unlike other gate
judges: its `error: not calibrated: <why>` is recorded, and the result goes on to the approval step,
where a person reads it anyway.

### What it does

A pass lets the result through, and the approval step asks the person as before. A refusal from
the pre-review alone stores nothing, and the agent reads:

```text
Before step approve-pr asks a person, the pre-review found outcome passed repeats a lesson. asked: It adds no test.. The lessons kept for that step:
- lesson-tests: Every pull request needs a test.
Fix it, then send the result again: it then goes to the person.
```

The pre-review sends a result back once per run. The agent's next result skips it, and the approval
step asks the person. A refusal from the pre-review never blocks the task, and the pre-review never
approves anything. When another scorer of the same gate also refuses, the gate's own rule applies.

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
