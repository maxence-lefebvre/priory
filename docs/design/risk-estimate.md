# The risk estimate

This page explains how R10.6 could work: "Estimate the chance that a wrong step outcome reaches an
irreversible step. Its labels (rejected approval, red CI after merge, revert), estimator and
assumptions are specified before it ships." It builds nothing. It holds the model, the first code
slice with its tests, the slices after it, and the open questions with a recommendation for each.

## What existed when this note was written

- Every step end appends a `factory.step.ended` record with its workflow, step, decider, effect and
  outcome ([trace-schema.md](../trace-schema.md)).
- Six outcome labels judge a task by what happened to its pull requests and human steps:
  `approved-clean`, `changes-requested`, `approval-rejected`, `closed-unmerged`, `red-after-merge`
  and `reverted` ([trace.md](../daemon/trace.md#outcome-labels)). Each is a `factory.step.labeled`
  record, written once per task and label: a second review that requests changes writes nothing.
  Its `time` is when Priory wrote it, not when the event happened.
- A person marks a task's work wrong with `priory label TASK wrong --evidence URL`, a
  `marked-wrong` label that `priory report risk` reads and `priory grades` leaves out
  ([label.md](../reference/label.md)).
- `priory grades` compares each grade with those labels, for the tasks that have a grade. It prints
  counts and shares, with no interval.
- `priory check --explain` lists each irreversible step with its gate, and every path that reaches
  one with no human decision ([workflows.md](../reference/workflows.md#irreversible-steps-and-their-gates)).
- An ask carries `risk`: `irreversible` when its answer lets an irreversible step run, then the
  task's merge risk verdict and its grade ([serve.md](../reference/serve.md#asks)). The first comes
  from the workflow's shape, and the other two are a model's opinion of one task. None is a
  measured rate.
- A relation `5 fixes 3` records that task 5 fixes what task 3 merged
  ([relations.md](../daemon/relations.md)). Priory records one by itself when a new task's request
  links a merged pull request, whether or not the new task corrects it.

## The model

### What is estimated

A step outcome is wrong when a later judgement contradicts the work it let through. Priory cannot
tell which step of a task was the wrong one: a label judges the task. So the estimate answers a
question a person can check against the records:

> Of the tasks that ran the irreversible step `t` of workflow `w`, what share got an adverse label
> within 30 days?

It is the rate of recorded adverse labels after an irreversible step. It is not the rate of wrong
work: assumption 1 says why. Each irreversible step of each workflow is a target, and each target
has rows:

- The target's own row holds every task that ran it, such as `implement / open-pr`. It is the
  history of that step's gate.
- A conditioned row holds the tasks that ended an earlier step `s` with outcome `o` before they
  ran the target, such as `plan / green`. It says how tasks that took that outcome fared. It never
  blames its step.

### The trial

A task reached the target `t` when it has a `factory.step.ended` record of `t`. Its first such
record counts, and its time is the reach. A task belongs to the conditioned row `s / o` when its
last record of `s` before the reach has outcome `o`.

A task is a trial of its rows when both of these hold:

1. Its reach is at least 30 days old, and inside the report's window.
2. It owned a pull request by the end of those 30 days.

The 30 days are a fixed horizon. A trial's outcome is what happened in the 30 days after its reach,
by the time each event happened, and nothing that happens later changes it. An event that Priory
stores late still counts on the day it happened. A mark a person clears counts nowhere, whenever
they clear it: the clear says the mark was a mistake, not that something happened. So every trial was watched for the
same time, a fast bad label and a slow clean one enter the count on the same day, and an old
trial has no more chances to turn bad than a new one. An adverse label that happened after the
horizon shows in a `late` count, beside the estimate and not in it.

A task whose reach is under 30 days old is `pending`. A task with no pull request by the horizon
has no label that can judge it after the step, so it is `unjudged` and never a trial, whatever a
person marked.
That covers an irreversible step that posts to Slack or labels an issue.

One task gives at most one trial per row, however many times it visited a step, was revived, or
how many pull requests its stack holds. Rows share tasks, so two rows are never independent, and
the report never multiplies or adds them.

### The labels

A trial has one of two adverse outcomes, or none:

| Outcome | The task got, within the horizon | Meaning |
| ------- | -------------------------------- | ------- |
| `landed wrong` | `red-after-merge` or `reverted`, or `marked-wrong` when a pull request of the task had merged by the mark's date | The harm passed every person and merged |
| `judged wrong` | Any of those, or `changes-requested`, `closed-unmerged`, or an `approval-rejected` whose human step ended after the reach | A person or an event contradicted the work after the irreversible step |

`landed wrong` is a part of `judged wrong`. Both use the same trials, so each row carries two
estimates. For `open-pr`, most of `judged wrong` is rework that a reviewer absorbed. For a merge
step, the two are nearly the same.

The labels need two small additions:

| Addition | Why | Slice |
| -------- | --- | ----- |
| `factory.step.labeled` gains `happened`, the time of the event that the label reads | The horizon counts by when an event happened. A record written before the field has no known date, so its task is `undated` and no trial: its write time would misplace a late event | First |
| One record per task, label and source event, in place of one per task and label. `priory grades` still counts a task once per label | A task with two irreversible steps needs the label that happened after each. Today a `changes-requested` before the second step hides one after it | First |
| `marked-wrong`: a person runs `priory label TASK wrong --evidence URL [--happened DATE]`, and `--clear` appends a `factory.step.unlabeled` record that withdraws it | Most bad merges are fixed forward, not reverted, and no event names them | Built, later slice 1 |

A `fixes` relation is not a label. Priory records one whenever a request links a merged pull
request, and an agent can write such a request. So `priory report risk` lists the `fixes` relations
on its tasks as candidates, and a person turns one into `marked-wrong` or leaves it. An agent must
not move a number that describes agents.

One more count sits beside each target, as context:

| Count | Meaning | From |
| ----- | ------- | ---- |
| `caught` | A person stopped the task at the target's gate | A human step that `approves` the target ended with an option that starts with `reject` or leads to `stop`. This is the `approval-rejected` label, read from the step records so its place on the path is known |

`caught` shows what the gate removes. The PRD names "rejected approval" among the labels: here it
is the evidence that the gate works, and the number to read before anyone argues for removing it.

### The estimator

Each row and outcome is a Beta-Binomial with no pooling:

- `n` trials, of which `k` had the outcome.
- The prior is Beta(1, 1): before any trial, every rate from 0 % to 100 % is equally likely. It
  weighs as much as two tasks, one adverse and one not.
- The posterior is Beta(1 + k, 1 + n - k).
- The report shows `k of n`, then the posterior's 5th percentile, median and 95th percentile. The
  two ends are a 90 % credible interval. The 95th percentile is the number to read, as "95 %
  posterior upper bound: 13.3 %". It is a bound under the assumptions below, not a guarantee.

| `k of n` | 5 % | median | 95 % |
| -------- | --- | ------ | ---- |
| 0 of 10 | 0.5 % | 6.1 % | 23.8 % |
| 0 of 20 | 0.2 % | 3.2 % | 13.3 % |
| 0 of 50 | 0.1 % | 1.3 % | 5.7 % |
| 2 of 41 | 2.0 % | 6.3 % | 14.2 % |
| 6 of 41 | 8.1 % | 15.8 % | 26.3 % |

The prior is deliberately not optimistic. A factory's true rate of bad merges is probably under
10 %, and a prior that said so would let a new workflow show a low risk with no evidence. With
Beta(1, 1), a row earns a low upper bound only by collecting trials.

### Assumptions

1. **The labels measure recorded judgements, not wrong work.** A bug nobody reverted has no label,
   so labels miss wrong work. A flaky check on the merge commit labels right work, so labels also
   blame it. The error runs both ways and its size is unknown. The estimate is the rate of adverse
   labels, and `priory report risk` lists each adverse trial with its evidence, so a person can see
   which ones they would not count.
2. **Trials in a row share one rate.** A prompt or model change breaks this: trials before it
   describe another agent. The window limits the damage, and the report prints how many factory
   revisions and models a row spans.
3. **Trials are independent, given that rate.** Tasks cut from one batch, or merged into one broken
   dependency, fail together. Then the interval is too narrow.
4. **30 days hold what matters.** A review, a merge and its first fallout fit in 30 days for most
   tasks. A pull request still open at the horizon counts as not judged wrong. The row shows those
   as `open`, and the `late` count shows what the horizon missed.
5. **A task is one trial.** A task whose stack holds three pull requests counts once, adverse when
   any of them was.
6. **The trials do not choose themselves.** They do, in part: a person approves and merges what
   looks right. So the rates describe work that passed that person's gates and review. They say
   nothing of what agents would do with no gate, and are no argument for removing one.

### The window

`priory report risk` reads the trials whose reach lies in the last 180 days, and `--since` changes
that. A shorter window follows the current prompts and models more closely and has fewer trials.
The default is long because a bad merge is rare: at 5 %, 60 trials hold three.

### How few trials, and what shows before

A row shows its percentages from 20 trials. Under 20 it reads:

```text
0 of 7, needs 20
```

The 20 is a presentation rule, not a statistical one. The interval is valid at any `n`, and
`--json` carries it at any `n` beside `"shown": false`. The rule exists because a percentage reads
as knowledge, and under 20 trials the 90 % interval is at least 13 points wide. Counts always show,
as the trace schema asks: "Report counts next to every rate, and leave a rate unknown when no
record supports it."

### Where it appears

**`priory report risk [--workflow W] [--since WINDOW] [--by lane] [--json]`**

```text
implement / open-pr: reached 58, caught 4 at its gate, pending 5, unjudged 0, undated 0, open 2, late 1
horizon 30 days, window 180 days, 3 factory revisions, 2 models

row          judged wrong  5%     median  95%     landed wrong  5%    median  95%
open-pr      17 of 53      22.8%  32.5%   43.4%   3 of 53       2.6%  6.8%    13.7%
plan green   13 of 41      21.4%  32.3%   44.6%   2 of 41       2.0%  6.3%    14.2%
plan yellow  4 of 12, needs 20                    1 of 12, needs 20

adverse trials
task  outcome       label     happened    evidence
31    landed wrong  reverted  2026-10-02  https://github.com/example/app/pull/88

candidates: `fixes` relations a person has not marked
task  fixed by  recorded by
27    44        priory
```

`--by lane` splits each row by the task's lane. A lane's row has fewer trials, so most read
`needs 20`. `GET /api/risk` answers the `--json` object.

**The ask's risk band.** An ask whose `risk` holds `irreversible` gains `risk_estimate`: the own
row of the irreversible step its answer lets run. It is one fixed row, chosen by the workflow's
shape and not by which row looks best. The ask shows one line under its risk:

```text
History: 17 of 53 tasks through this step were judged wrong within 30 days, 3 after the merge.
95 % posterior upper bounds: 43.4 % and 13.7 %.
```

Under 20 trials, the line gives the counts and `too few to estimate`. It adds no value to `risk`,
no color and no level. `priory asks --risk` filters as today.

**`priory check --explain`.** The first table, each irreversible step with its gate, gains
`caught`, `judged wrong` and `landed wrong` columns when the command finds the factory's state
file, and `-` when it does not. `priory check` without `--explain` never reads the state, so a
factory still lints with no history. The second table, the paths with no human decision, names the
same row for its irreversible step. That is where the number matters most: an `autonomous` step has
no gate and no `caught`.

### What it must never be used for

- **An automatic approval.** No code path reads the estimate to answer an ask, to skip a human
  step, or to let a lead agent answer an ask it could not answer before. The approval rule reads
  the workflow file only, and `priory check` gives the same verdict with or without history.
- **A reason for `autonomous`.** `autonomous` takes a sentence a person wrote. Priory never writes
  it, and `priory check` never accepts a number in its place.
- **Queue order, room or routing.** A low estimate starts no task earlier.
- **A prompt to an agent.** The estimate never reaches a run's prompt. An agent that knows its
  record is graded by labels has a reason to avoid labels, not errors.
- **A comparison of two configurations.** Two rows that differ by prompt or model have different
  trials, months apart. Benchmarks replay the same tasks for that
  ([scorers-and-benchmarks.md](scorers-and-benchmarks.md)).
- **A statement about one task.** The row describes past tasks of its kind. The ask's line says
  "tasks through this step", never "this change".

## The first slice: `priory report risk`

The smallest change that delivers the estimate reads the labels that exist. It adds one field and
no label.

1. `factory.step.labeled` gains `happened`, set from the event that the label reads, and Priory
   writes one record per task, label and source event. `priory grades` counts a task once per
   label, so its output does not change.
2. A `risk` module in `priory-core` beside `grades.rs`. It reads the `factory.step.ended` and
   `factory.step.labeled` rows of the `trace` table and the tasks' pull requests, and builds the
   targets, rows, trials and counts as [The trial](#the-trial) defines them.
3. The Beta quantiles. With whole counts and a Beta(1, 1) prior, both parameters are whole
   numbers, so the Beta's distribution function is a binomial tail sum, and a bisection finds a
   quantile. That is about 30 lines with no dependency. See open question 1 for the crate.
4. `priory report risk`, with `--workflow`, `--since` and `--json`, and the `needs 20` rule.
5. A section in [trace.md](../daemon/trace.md), after Grades, with the trial, the labels and the
   assumptions above, and `happened` in [trace-schema.md](../trace-schema.md).

Tests, through the CLI as `crates/priory/tests/grades.rs` does:

- A task that ended `plan: green`, ran `open-pr` 31 days ago and was reverted 10 days later is one
  `landed wrong` and `judged wrong` trial of `open-pr` and of `plan green`.
- The same task with a reach 29 days old is `pending` in both rows and in neither `n`, reverted or
  not.
- Two tasks with the same events, one reached 31 days ago and one 170 days ago, count the same.
- A revert that happened 31 days after the reach adds one to `late` and leaves the trial not
  adverse. A revert that happened on day 29 and that Priory stored on day 35 counts in the trial.
- A task with no pull request that ran an irreversible step is `unjudged`, and stays so when its
  first pull request opens on day 31.
- A task whose label record has no `happened` is `undated` and in no `n`.
- A task that got `changes-requested` before its second irreversible step and again after it is
  `judged wrong` for both targets. `priory grades` prints the same as before for it.
- A task that visited `plan` twice, `red` then `green`, is a trial of `plan green` only. A revived
  task is still one trial.
- A task of a workflow with two irreversible steps is a trial of each target, with each reach's
  own horizon.
- A task with a stack of two pull requests, one merged and reverted and one closed unmerged, is
  one `landed wrong` trial.
- A human step that approves `open-pr`, answered `rejected`, adds one to `caught` of `open-pr` and
  no trial. A human step after `open-pr`, answered `rejected`, makes the trial `judged wrong`.
- A pull request still open on day 30 counts in `n`, in `open`, and in no adverse outcome.
- A row with 19 trials prints `needs 20` and no percentage, and its JSON holds the interval with
  `"shown": false`. A row with 20 prints all three percentages.
- The quantile function returns the table above to one decimal, and a `proptest` checks that for
  any `k <= n` under 500 the three quantiles are ordered, lie in 0 to 1, and that the distribution
  function at each quantile gives back its probability within 1e-6.
- `--since` leaves out a trial whose reach is before the window.

## Later slices

1. Built. `priory label TASK wrong --evidence URL`, its `--clear`, the `marked-wrong` label, and the
   candidates table. The verb joins the `priory-person` guard, so no run can call it. Tests cover a
   mark on a task with no pull request, which stays `unjudged`, a mark on a task whose pull request
   never merged, which is `judged wrong` only, and a cleared mark.
   [label.md](../reference/label.md) and [trace.md](../daemon/trace.md#risk) describe it.
2. Built. The ask's `risk_estimate` and its History line in `priory asks`
   ([trace.md](../daemon/trace.md#risk), [serve.md](../reference/serve.md#asks)).
3. Built. The columns of `priory check --explain`, and `risk_estimate` in `priory flows --json` and
   `GET /api/flows` ([workflows.md](../reference/workflows.md#irreversible-steps-and-their-gates)).
4. Built. `--by lane`, with no partial pooling, as open question 2 recommends. Each lane's block
   has its own counts and rows on the flat prior. A test covers two lanes of one workflow, each with
   its own trials and adverse outcomes. [trace.md](../daemon/trace.md#risk) describes it.

## Open questions

1. **A crate for the Beta quantiles, or 30 lines?** `statrs` 0.19.1 (MIT, released 2026-08-11)
   gives the inverse Beta distribution function for any parameters. Its default features bring
   `nalgebra` and `rand`. Recommendation: write the 30 lines. A whole-number prior makes the exact
   sum possible, the first slice needs nothing else from the crate, and the `proptest` above pins
   it. Propose `statrs` in the slice that needs a prior that is not whole numbers.
2. **Should a thin row borrow from its workflow?** A lane's row or a rare outcome's row has few
   trials. Partial pooling would give each row a Beta prior centered on its workflow's rate, worth
   a fixed number of tasks, so a thin row leans on its siblings. It shows a number sooner. It also
   makes a row's number depend on other rows, which is harder to check by hand, and a new risky
   lane would look as safe as its workflow until its own trials outweigh the prior.
   Recommendation: not yet. Ship the flat prior, look at real rows after a month, and add pooling
   only if most rows a person reads say `needs 20`.
3. **One estimate or two?** `judged wrong` is the closest the labels come to R10.6 as written: a
   judgement contradicted the work after the irreversible step. `landed wrong` is the harm a person fears, and it is rare, so its rows stay
   wide for months. Recommendation: both, on the same trials, as above. One number would either
   hide bad merges under rework or drop the rework that R10.6 asks about.
4. **Does a model or prompt change reset a row?** Assumption 2 breaks at each change.
   Recommendation: no reset. Print the revisions and models a row spans, keep the 180-day window,
   and add `--by model` once the factory changed model with 20 trials on each side.
5. **Are 30 days and 20 trials right?** Both are judgements. A longer horizon catches more late
   reverts and delays every trial by as much. Recommendation: keep both as constants in the first
   slice, print them in the report, and read the `late` and `open` counts after a month. Move them
   to `factory.toml` only when a person asks for another value.
6. **What of irreversible calls outside irreversible steps?** A workflow's `guard.allow` lets every
   step make some guarded calls with no approval ([guard.md](../daemon/guard.md)). A call the guard
   allows leaves no record, so it is no reach. Recommendation: leave it out of R10.6. If it matters
   later, the guard appends a `factory.tool.allowed` record, and that record is a reach.
