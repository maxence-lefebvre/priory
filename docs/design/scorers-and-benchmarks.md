# Scorers and benchmarks

This page explains how R10.7 could work: "Scorers grade finished runs. Benchmarks replay past
tasks under another model or configuration." M4 is done when "a configuration change is compared on
replayed tasks before it merges". It builds nothing. It holds the model, the first code slice with
its tests, the slices after it, and the open questions with a recommendation for each.

## What exists today

- The PRD lists `scorers/` and `benchmarks/` among the factory's files. Nothing reads them.
- `priory eval` compares two texts on a real model
  ([evaluating-a-prompt.md](../evaluating-a-prompt.md)). Each turn is a headless `claude -p` with no
  built-in tools, and a fake of Priory's MCP server that logs each call and answers `ok`. It counts
  how a turn ends: `result`, `ask` or `wait`. It takes a required `--budget`, and it never runs in
  CI. It measures a text, not a task: the agent has no repository and does no work.
- The factory end-to-end test plays a task through a real daemon in a scratch home, with a fake
  `claude`, `codex` and `gh`, and a bare Git origin per repository
  ([testing-a-factory.md](../testing-a-factory.md)). The world is fake and so is the agent: its
  turns are a list of tool calls. It measures a workflow, not a model.
- The state keeps what a task was asked: its request message and source event in the inbox, each
  prompt in the outbox, each ask with its answer, each result, and each step record with its tokens.
- A new task's worktree starts at `origin/HEAD` after a fetch. Priory does not store that commit.
- The outcome labels and `priory report risk` ([risk-estimate.md](risk-estimate.md)) say how past
  tasks fared. They cannot compare two configurations: their tasks differ and are months apart.

A benchmark joins the two halves that exist: the fake world of the end-to-end test, and the real
model of `priory eval`.

## The model

### A scorer

A scorer grades one finished run: a run whose turn ended with a result or an ask. It reads one of
three things and answers `pass` or `fail`. A `result` or `diff` scorer fails a run that has no
result: asking is not finishing. An `ask` scorer answers `not applicable` on a run that asked
nothing. A scorer that cannot run answers `error` with why.

| `on` | The scorer reads | Example question |
| ---- | ---------------- | ---------------- |
| `result` | The run's result: its outcome, summary, evidence and the other fields | Is the outcome `passed`? |
| `diff` | The result's `commit`, against the commit the task started from | Does the gate pass? Did it touch only `crates/`? |
| `ask` | The asks the run made, with their fields | Is the question one line, with what waits and the next action? |

A scorer is one file, `scorers/<name>.toml`, of one of four kinds. The first three are code and
come first. Use the fourth only for a question that code cannot answer.

```toml
# scorers/passed.toml
description = "The step ended as passed."
on = "result"
kind = "outcome"
expect = ["passed"]
```

```toml
# scorers/gate.toml
description = "The factory's gate passes on the change."
on = "diff"
kind = "command"
run = "scripts/score-gate.sh"   # in the factory folder; exit 0 pass, 1 fail, else error
timeout = "20m"
```

```toml
# scorers/short-ask.toml
description = "Each ask is short and says what waits."
on = "ask"
kind = "ask-shape"
expect = ["short", "whatWaits", "nextAction"]   # the shape `priory eval` already reads
```

A `diff` scorer never reads the task's live worktree: a later run may be editing it. Priory checks
the result's `commit` out into a scratch folder, and a `command` scorer runs there, with a cleared
environment and `PRIORY_SCORE_BASE`, `PRIORY_SCORE_HEAD` and `PRIORY_SCORE_RESULT`, the path of a
JSON file that holds the result. A result's `commit` is optional, so a `diff` scorer answers
`error: the result names no commit` for a result without one, and `error: no start commit` for a
task created before Priory stored it. One useful `command` scorer for a replayed task applies the
tests of the pull request that really merged and runs them: the task's own later history is its
reference.

**A model judge** is the fourth kind:

```toml
# scorers/does-what-was-asked.toml
description = "The change does what the request asks, and nothing else."
on = "diff"
kind = "judge"
model = "sonnet"
prompt = "does-what-was-asked.md"           # the judge's whole prompt, beside this file
evaluation = "does-what-was-asked.eval.toml" # labeled cases: a request, a diff and the right verdict
agreement = 0.8                              # the least share of labeled cases it must get right
```

A judge is a model's opinion, so it ships only with its own measurement:

- `priory check` refuses a `judge` scorer with no `prompt`, no `evaluation`, or fewer than 10
  labeled cases, of which at least 3 `pass` and 3 `fail`.
- `priory score --calibrate <scorer> --budget USD` runs the judge on its labeled cases and stores
  the agreement with the hash of the prompt, the model and the cases. A person writes the labels.
- A judge whose stored agreement is missing, under its `agreement`, or from another prompt, model
  or cases answers `error: not calibrated`. So an edited judge prompt scores nothing until it is
  measured again.
- Every table that shows a judge's scores shows its agreement and its number of labeled cases.
- The judge runs as `priory eval` runs a turn: headless, with no tool, no settings and no MCP
  server. The request and the diff are untrusted text, and the prompt gives them as data. Its
  answer must be `pass` or `fail` on the first line, then one line of why. Any other answer is an
  `error`.
- `priory check` warns when a judge's model equals the model of an agent of the factory, since a
  model tends to prefer its own work. It is a warning: the agreement rule above is the gate.

Each score appends a `factory.run.scored` record to the trace, with the run, the scorer, the
verdict, the line of why, the factory revision, and for a judge its model and agreement. A score is
a record, so scoring a run again with a changed scorer adds a record and replaces none.

`priory score TASK [--scorer NAME]` scores the latest finished run of each step of a task, on
demand, while the repository still holds its commits. No scorer runs by itself after a run: a `command`
scorer can take as long as the gate, and a judge costs money.

### A benchmark

A benchmark is a set of cases and the scorers that grade them: `benchmarks/<name>.toml`.

```toml
description = "Twelve implement tasks merged in November."
workflow = "implement"
primary = "gate"                                             # the one scorer that gives the verdict
scorers = ["passed", "merged-tests", "short-ask"]            # shown beside it, with no verdict
cases = ["cases/eng-412", "cases/eng-431", "cases/app-57"]   # folders beside this file
```

A case is a past task, captured once into a folder by
`priory bench capture TASK --into benchmarks/cases/<name>`. The folder is committed with the
factory, so a person reads it before it is shared, and every later replay starts from the same
bytes:

| File | Holds | From |
| ---- | ----- | ---- |
| `case.toml` | The workflow, the repository, the start commit, the request's data, and the task's real path and labels as reference | The task, its inbox entry and its trace |
| `gh/` | The fake `gh`'s answers: each `gh` command the task's agent ran before the first irreversible step, with what it printed then | The harness transcripts of the task's runs |
| `answers.toml` | The answer a person gave at each human step before the first irreversible step | The asks |
| `merged.patch` | Optional: the diff that really merged, for a scorer that needs it | The task's merged pull requests |

Priory's own events hold selected fields of an issue, such as its title, and not its body or its
comments. What the agent read of the world is in its harness transcript, which records each command
and its output. So `capture` reads the `gh` answers there. It never asks today's GitHub, whose
issue may have been edited since. It keeps an answer only when the call was one `gh` command alone
and the transcript holds its whole output. A `gh` call inside a pipe, a script or a truncated
output is not kept. The case is `partial` when a call was not kept, or when the transcript is
gone, as a harness deletes old ones. A person fills the gaps by hand or leaves them, and the table
of a comparison names its partial cases.

**The start commit** is the one small addition to what Priory records: the daemon stores the commit
a task's worktree was created from, as `tasks.base_sha`. Without it, a task captured later can only
guess its start from `git merge-base`, which is wrong once the default branch was merged into the
task's branch. Every day without the column loses exact cases, so it belongs in the first slice.

**A replay** plays one case under one configuration, in a scratch folder, as the end-to-end test
does, with one change: the agent is real.

| Part | In a replay |
| ---- | ----------- |
| The factory | A checkout of the configuration under test |
| The daemon, the intake's messages, the workflow | Real, this build's, in a scratch Priory home |
| The harness and the model | Real: `claude` or `codex`, with the person's sign-in and nothing else of their settings |
| The repository | Real: a clone at the case's start commit. The agent edits, builds and tests for real |
| `origin` | A bare repository in the scratch folder, so a push goes nowhere |
| `gh` | The fake, answering from the case's `gh/`. A call the case does not hold fails with `not recorded in this case`, and is counted |
| Asks | A human step gets its answer from `answers.toml`. An ask the agent raises itself has no recorded answer: the replay ends there, as `asked` |
| Waits | A wait on a time is honored up to the step's timeout. Any other wait ends the replay, as `waited` |
| Other MCP servers | Not started. The case lists the servers the agent had, so a reader knows what the replay lacked |
| The environment | Cleared, with a scratch home and no token for GitHub, Linear or Slack |

**A replay ends at the first irreversible step, and never runs it.** That is the boundary of what a
fake world can answer honestly. The case holds what the world said before the change. What the
world would say about a new change, such as its checks or its review, was never recorded and
cannot be. So a replay covers the task from its request to the result that would have asked for
the irreversible step, and the scorers judge that result and its diff.

A replay's ending is one of `reached <step>` (it got to the irreversible step), `stopped` (the
workflow reached `stop`), `asked`, `waited`, `timed out`, `over budget`, `agent error` or
`cost unknown` (a turn whose tokens cannot be read). The
ending is a measurement too, and the table shows it.

The replay is not a sandbox. The agent runs on the person's machine with the person's model
sign-in, and a script it writes can reach the network. The cleared environment, the fake `gh`, the
scratch origin and the tool guard stop what an agent reaches for directly. So capture cases only
from repositories and requests you trust, as you would run their tasks.

### What differs between two runs

One thing: the factory's files. A comparison names two Git commits of the factory, `--base` and
`--head`. Since the factory is files in a repository, a prompt change, a model change and a
workflow change are all a diff between two commits, and no flag overrides a model.

Everything else is held the same for both sides: this build of `priory`, the case's bytes, the
start commit, the answers, and the scorers. The scorers and the benchmark file come from `--head`
for both sides. When the diff between the two commits touches `scorers/` or `benchmarks/`, the
table says so in its first line, since the change then moved the ruler too.

Runs go in pairs, one per side, case by case. The side that goes first alternates from one pair
to the next, so a warm cache or a drift in the model service favors neither side. A pair that the
budget cut is dropped whole, and the table counts the dropped pairs.

### How outcomes are compared

`priory bench FILE --base REF --head REF --budget USD [--runs N] [--case NAME]`

- **Run counts.** Each case runs `N` times per side, 3 by default. A model's run varies, so one run
  per side would compare two draws.
- **The unit is the case.** Runs of one case are not independent of each other, and two sides of
  one case share its difficulty. So for each scorer, each case gives one difference: the share of
  the head's runs that passed, less the base's.
- **Every replay counts for a `result` or `diff` scorer.** A `fail` and an `error` both count as
  not passed: a change that breaks the build makes a `command` scorer error, and leaving errors out
  would reward it. A replay that ended with no result, such as `asked`, `waited` or `timed out`,
  counts as not passed too, so a change cannot raise its share by asking more. Only an `ask` scorer
  leaves out the replays that asked nothing, and the table shows how many each side has. A judge
  that is `not calibrated` is left out whole, on both sides.
- **Broken rulers are found first.** Before the first replay, each `command` scorer runs once on
  each case's untouched start commit. A scorer that errors there cannot run on that case, and the
  case is dropped for both sides, with its name in the table. So an `error` during the comparison
  points at the change.
- **The table** shows, per scorer, each side's passes over its runs, its errors, the mean
  difference in points, and the cases the head won, tied and lost. Then per side the endings, the
  median cost and the median time of a run.
- **One verdict, from the primary scorer.** The benchmark file names one `primary` scorer, on
  `result` or `diff`, so it scores every replay. `priory check` refuses an `ask` scorer there. Its
  verdict is `better`, `worse` or `no clear difference`, from an exact sign test on the cases that
  did not tie: `better` or `worse` when the two-sided probability is under 0.05. With fewer than 6
  cases that did not tie, no split can get under 0.05, and the verdict reads `too few cases`. The
  test is deterministic, needs no dependency, and a person can check it on paper. It judges the
  direction of the cases' differences, not their size.
- **The other scorers are shown, not tested.** A verdict per scorer would be several tests, and a
  `better` would appear by chance among them. So they get their counts and no verdict, and a
  person reads `gate: better, merged-tests: +14 points, cost: +15 %` and decides.
- **The first complete comparison of a commit gives its verdict.** Running again until a `better`
  shows up is another way to find one by chance. So a later comparison of the same two commits and
  benchmark is marked `repeat`: it posts its table, carries no verdict and leaves the status as it
  is. Each table carries its number, such as `comparison 2 of this pull request`.

```text
benchmark implement-november, base 999daf0, head 4c1e2b7, comparison 1 of this pull request
12 cases, 3 pairs each, 0 pairs dropped, $31.20 of $40

scorer        base   errors  head   errors  mean diff  won  tied  lost  verdict
gate          26/36  1       33/36  0       +19.4      6    6     0     better
passed        30/36  0       33/36  0       +8.3       3    8     1
merged-tests  19/36  0       24/36  0       +13.9      5    5     2
short-ask     1/2    0       2/2    0       +50.0      1    1     0

side  reached open-pr  stopped  asked  timed out  median cost  median time
base  30               3        2      1          $0.41        11 min
head  33               1        2      0          $0.47        12 min
```

What the verdict assumes, so a person can disagree:

1. The cases stand for the tasks the factory will get. They are past tasks a person chose, so they
   lean toward what was captured: merged, well-described work.
2. Cases are independent of each other. Two cases cut from one batch of issues are not.
3. A tie carries no information. The sign test drops ties, so a change that helps a little on
   every run of every case can read `no clear difference`. The mean difference is there for that.
4. The primary scorer was chosen before the comparison. It lives in the benchmark file, and a
   change to that file shows in the table's first line.
5. A replay measures the task up to its first irreversible step. A change that makes pull
   requests harder to review, or checks slower, shows nowhere.

### The cost cap

`--budget` is required, as for `priory eval`. Before the first run, the command prints an estimate:
each case's recorded tokens up to its first irreversible step, priced for each side's own model
from `prices.toml`, times the runs, plus the judges' calls. It refuses to start when the estimate
is over the budget, and names the `--runs` or `--case` that would fit. It also refuses when
`prices.toml` lacks a model of either side or of a judge: a cap in dollars needs every price.

It then checks what it spent before each pair and after each turn, from the scratch daemon's step
tokens and the judges' reported costs. A run in progress when the budget ends is cancelled and ends
as `over budget`, and no later pair starts. The table's first line then says `Incomplete`, and it
prints no verdict.

The cap is not hard. An interactive run has no spending limit inside one turn, unlike the headless
turns of `priory eval`, and Priory reads a turn's tokens when the turn ends. So the comparison can
pass its budget by the cost of the turn in progress. A workflow step may set no `timeout`, so a
replay ends every turn after 30 minutes, or the step's `timeout` when it is shorter, and
`--turn-timeout` changes that. The estimate names the largest overrun that one turn could cost at
that length, and the table prints what was really spent. A turn whose tokens cannot be read ends the comparison as
`Incomplete`, since its cost is unknown.

### Before a change merges

The M4 line is "a configuration change is compared on replayed tasks before it merges". The path
uses a trigger and a workflow of the factory itself, and it runs only when a person asks.

1. A person opens a pull request on the factory's repository that changes an agent's prompt or
   model, or a workflow.
2. The person adds the label `benchmark` to it. A trigger starts a task, bound to the head commit
   the event names:

   ```toml
   # triggers/benchmark.toml
   source = "github"
   repo = "owner/factory"
   on = "pull_request.labeled"
   filter = { label = "benchmark" }
   workflow = "benchmark"
   key = "pull_request.number"
   ```

3. The workflow is one human step. Its ask reads `Compare pull request 12 at 4c1e2b7 on 12 cases?
   Estimated $31 of a $40 budget, plus at most one turn`, with the options `run` and `skip`. A
   label is text that a bot or an agent can add, and an answer on the desk is a person's: no run
   can give one.
4. The step carries a `command` that `run` runs once, as a runbook step does
   ([workflows.md](../reference/workflows.md#runbooks)):
   `priory bench benchmarks/implement.toml --base <merge base> --head 4c1e2b7 --budget 40 --post`.
   The answer is bound to that command, so what the ask shows is what runs. Open question 8 says
   why this is not an approved code step.
5. `--post` writes the table as one comment on the pull request, and sets a commit status
   `priory/benchmark` on the head commit: `success` when the comparison is complete and the primary scorer
   does not read `worse`, `failure` otherwise. A repository that wants the comparison before every such
   merge makes that status required for the paths it cares about.

The status is bound to the head commit. A push after the comparison gives a head with no status,
so a stale table cannot pass for the new commit. To compare again, the person removes the label
and adds it again.

What keeps it from running by itself:

- No trigger event fires on a push to a pull request, and `priory bench` runs from no other place
  in Priory. One label and one answer are one comparison. `bench` joins the `priory-person` guard,
  as `eval` did, so no run starts one.
- The budget is in the workflow file, reviewed like any change.
- The command passes no room check, but each replay's run does: the scratch daemon admits it
  against the machine's live free memory, as the real one would.
- The spending and the comment come from a person's answer, at most once, with the uncertain
  outcome of an answer's command if Priory stops mid-run.
- The comparison runs the head's prompts and scripts with the person's sign-in. So the task is
  dropped, with the reason, unless the event's `head_repo` is present and equals `repo`: a pull
  request from a fork, or one whose head repository the event does not name, is never compared.
  The ask names the commit, and `--head` takes that commit, so a push after the answer changes
  nothing of what runs.
- A person can also run `priory bench` by hand and paste the table, as `priory eval` asks today.

The comparison informs the merge. It never merges, and its status never approves a pull request.

## The first slice: the start commit, and code scorers on demand

The smallest change that is useful alone, and that every later slice needs:

1. The daemon stores `tasks.base_sha` when it creates a task's worktree: the commit `origin/HEAD`
   named then. `priory task --json` shows it.
2. `priory-factory` loads `scorers/<name>.toml` of kinds `outcome`, `command` and `ask-shape`, and
   publishes its schema. `priory check` reports an unknown kind, an `expect` that does not fit the
   kind, and a `command` scorer whose `run` is missing, each with its file and line.
3. `priory score TASK [--scorer NAME] [--json]` scores the latest finished run of each step, prints
   one row per run and scorer, and appends `factory.run.scored` records. A `diff` scorer runs in a
   scratch checkout of the result's commit.
4. The record's schema in [trace-schema.md](../trace-schema.md), and a reference page for scorer
   files.

Tests:

- A new task's `base_sha` is the origin's head when its worktree was created, and stays after a
  later commit lands on the origin. A revived task keeps it.
- `priory check` on a scorer with `kind = "judge"` says the kind is not supported yet, with the
  file and line, and an example factory gains one scorer of each kind, so the test that checks
  every example covers the loader.
- An `outcome` scorer passes a run whose result is in `expect` and fails another.
- A `command` scorer that exits 0, 1 and 2 scores `pass`, `fail` and `error`. It sees the three
  variables and no variable of the caller's environment. One past its `timeout` is killed and
  scores `error`.
- A `diff` scorer on an earlier result reads that result's commit while a later run has changed
  the task's worktree and branch. A result with no `commit`, and a task with no start commit, each
  score `error` with the reason.
- A run that ended with a well-formed ask and no result scores `pass` on an `ask-shape` scorer and
  `fail` on an `outcome` scorer. A run with a result and no ask scores `not applicable` on the
  `ask-shape` scorer.
- An `ask-shape` scorer agrees with `priory eval`'s `shape` on the same asks: both call one
  function.
- Scoring a run twice appends two records and changes none.
- A `proptest` feeds the scorer loader random and mutated TOML, and it returns or refuses without
  a panic, as CONTRIBUTING asks for a parser of outside text.

## Later slices

1. Built. `priory bench capture` ([bench.md](../daemon/bench.md),
   [cases.md](../reference/cases.md)). It writes no `merged.patch` yet, and reads only Claude
   Code's transcripts: a Codex run is a gap. The case folder, from a task with a `base_sha`. Tests capture a task of
   the end-to-end harness and compare the folder byte for byte, refuse a task with no start commit
   or no irreversible step, and mark a task whose transcript is gone as `partial`. The transcript
   reader is a parser of outside text, so it gets a `proptest`.
2. Built. `priory bench replay` ([bench.md](../daemon/bench.md#replay)): the replay of one case
   under one configuration, with the endings, `--budget` and the estimate. The capture now
   records each model step's model, minutes and tokens as the case's `spent`, which the estimate
   prices. The scratch daemon blocks a task before an irreversible step. A model step that ends
   with no tokens ends the replay as `cost unknown`. Its tests use the fake `claude`, so they cost
   nothing, and cover a budget that ends inside a turn and a turn with no tokens. The real-agent
   check is by hand, as the operator rules ask.
3. Built. `priory bench` with `--base` and `--head` ([bench.md](../daemon/bench.md#compare),
   [benchmarks.md](../reference/benchmarks.md)): the pairs, the table and the sign test. The
   comparison is a pure function over per-case fixtures, which also print the table above. Its
   tests cover ties, a dropped pair, a repeat, and a base that passes 2 of 3 against a head that
   passes 1 and then errors twice, or asks twice, which the head must lose both times. A repeat is
   found from the `factory.bench.compared` records of the machine that runs the comparison, and
   the header reads `of these commits` until `--post` names a pull request.
4. `--post`, the `benchmark` example factory with its trigger and workflow. Tests start from a
   fresh task with no branch, and cover a label from a bot, which still waits for the person's
   answer, a fork, an event with no `head_repo`, and a head that moved after the answer. This slice closes M4's done-when line.
5. The `judge` kind, `priory score --calibrate` and the agreement rule.
6. Built. Scores in the web view's task page and Costs panel ([score.md](../daemon/score.md#where-scores-show)).
   Herdr's Task and Costs panels do not show them yet.

## Open questions

1. **Should a scorer run by itself when a step ends?** A workflow step could name scorers, and the
   daemon would score each of its results. It gives live scores with no command. It also puts a
   slow command or a paid judge on every run, and a score nobody reads is waste.
   Recommendation: not in M4. Score on demand and in benchmarks, then add a per-step `score = [...]`
   for `outcome` and `ask-shape` scorers only, which cost nothing.
2. **May a score ever decide a step's outcome?** A `proof` already lets the daemon check an outcome
   before it accepts it, with `gate-passed`. A scorer could become a proof. Recommendation: no. A
   proof is a rule the factory enforces, a score is a measurement, and an agent that is graded by
   the thing that gates it learns the grader. Keep them two concepts with two names.
3. **How does a replay serve the agent's other MCP servers?** The case holds no answer of Linear,
   Slack or a web search, so a replay starts none of them, and a task that leaned on one replays
   poorly. Recommendation: accept it for M4 and show the lost servers in the table. Later, record
   an MCP server's answers through a proxy during the live task, which also gives the guard a view
   of those calls. The same recording would replace the transcript as the source of `gh/`, and end
   partial cases.
4. **Should a replay run in a container with no network?** It would turn "not a sandbox" into a
   real boundary, at the price of a container runtime, the model's endpoint allowed through, and
   each repository's toolchain inside. Recommendation: not in M4. Cases come from the factory's own
   repositories, where the same agents already run unattended. Revisit before anyone shares cases
   between factories.
5. **Is the sign test the right test?** A paired bootstrap over cases would use the size of each
   difference, and give an interval. It needs a random seed to repeat, and with 12 cases its
   interval is unstable. Recommendation: the sign test for the verdict, the mean difference beside
   it, and no bootstrap until benchmarks hold 30 cases or more.
6. **How many cases and runs?** Under 6 untied cases there is no verdict, and 3 runs per side
   resolve a case's pass share only to thirds. Recommendation: say in the how-to guide to capture
   at least 12 cases per workflow, mixed between tasks that merged clean and tasks that were sent
   back, and keep 3 runs. Spend more budget on cases before runs: cases are the unit.
7. **Should every scorer get a verdict?** A Holm correction across the scorers would allow it. It
   needs more cases to reach any verdict, and it is harder to check by hand. Recommendation: one
   primary scorer per comparison, and the others as counts.
8. **How does a person's answer start the comparison?** An approved irreversible code step is the
   usual shape, but an approval binds the commit of the task's branch, and this task has no branch:
   it blocks. A human step's `command` fits: the answer runs it once and is bound to it. Two things
   are unknown until the slice tries them: whether such a command may run for an hour, and how it
   gets the pull request's number and head commit from the task's source event. Recommendation:
   use the `command`, and extend it in that slice with what it lacks, with a test from a fresh
   task. Fall back to a code step with `autonomous` only if the command cannot be made to fit.
