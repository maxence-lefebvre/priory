# Trace

The daemon appends one `factory.step.ended` record per step end, in the transaction that moves the
task on, as [the trace schema](../trace-schema.md) defines it. A model step ends with its run's
result, a human step with its answer, a code step with its action's status (the record's
`outcome`). A run that ends otherwise, such as blocked, lost or cancelled, ends no step and appends
no record. A repair appends `factory.task.repaired` instead (see Repair), and a reopen appends
`factory.task.reopened` (see [routing.md](routing.md#feedback-reopens-a-done-task)). A judgement that comes later,
such as a merge or a revert, appends `factory.step.labeled` (see Outcome labels). Each `priory gate` appends
`factory.gate.ran`, which a step's `gate-passed` proof reads (see Proofs in
[tools.md](tools.md#proofs)). Each score of `priory score` appends `factory.run.scored` (see
[score.md](score.md)), and each complete `priory score --calibrate` of a judge appends
`factory.scorer.calibrated` (see [score.md](score.md#calibration)). `priory report` reads only
`factory.step.ended`.

Records live in the `trace` table of the state file, one CloudEvents JSON object per row. R9.3 keeps
task state in Priory's own SQLite file and the event log in herdr-bus's. A bus write could not commit
with the state change it describes, so a crash between them would lose a record or write one twice.
A row commits with the move. An exporter publishes rows later, using `seq` as cursor (see
OpenTelemetry export).

`priory trace` prints every record, oldest first, one JSON object per line.

| Field | Priory's value |
| ----- | -------------- |
| `factory` | `name` in `factory.toml` |
| `factory_revision` | the last commit that changed the factory's folder; absent outside git |
| `task`, `run` | the task's and the run's ids; `run` only for a model step, the run that sent the result |
| `generation` | the task's generation when the step ended: 1, plus one per revive |
| `lane` | the task's lane when the step ended; absent for a task created before lanes |
| `machine` | the host name, which also names the runner |
| `started` | when the task entered the step, so it includes the time the task waited for a run |
| `harness`, `model`, `tokens` | for a model step: the agent's harness, the model the transcript names, else the agent's |

A model step's `tokens` come from Claude's session transcript, the file that `transcript_path` names. A run records the file
and its length at `SessionStart`, since a resumed session's file already holds earlier runs' turns.
At each `Stop`, the daemon reads the lines added since and adds each assistant message's usage to the
task's current step. Claude writes a message once per content block, each line with its usage, so a
message id counts once. A run the daemon blocks or finds lost is read as it ends, since a resume
carries on the same step. Entering a step starts its count again. No assistant message read means no
`tokens`. An unreadable transcript keeps its offset, so the next read counts its turns once.

Known limits:

- A run live when the daemon upgraded to this schema has no transcript recorded. Its first `Stop`
  only marks where reading starts, so that turn's tokens are lost, not counted with other runs'
  turns.
- A step using two models, such as an agent whose model changed between runs, records the last one.
  `--cost` prices all its tokens at that model's price.
- `factory_revision` names the last commit, even with uncommitted changes.

## OpenTelemetry export

The daemon can send its `factory.step.ended` records to an OTLP/HTTP endpoint as spans. It is off
unless `factory.toml` names the endpoint, and then nothing is sent:

```toml
[telemetry]
otlp = "http://127.0.0.1:4318"   # spans go to <otlp>/v1/traces as OTLP JSON
headers_env = "OTEL_EXPORTER_OTLP_HEADERS"   # optional: the variable holding the headers
```

A backend that wants a key reads it from a header. `headers_env` names the environment variable that
holds the headers, as `key1=value1,key2=value2` with URL-encoded values. The daemon reads it once
when it starts, refuses to start when it is unset, empty or malformed, and writes the header lines
to `otlp-headers` in its home, readable by its user only. `curl` reads them from that file, so no
value shows in the process list, the daemon's log or the trace.

A task is one trace, with a root span named `task <key>`, from the task's creation to its end. A
model step's run is a span named `invoke_agent <agent>` under the root, from the run's start to the
step's end, and the step is a span under it. A human or code step has no run, so its span is a child
of the root. A run that ended with no step, such as a cancelled, blocked or lost one, is a span of
the same name under the root, from its start to its end, with `priory.run.ended_as` naming how it
ended. An OTLP span is immutable once sent, so the root span, and the spans of runs that ended no
step, are sent when the task reaches `done`, `failed` or `dropped`, and not before. A task that a
revive or a reopen sends on starts a new root span, as its next generation, and the same trace. The step span starts when the task entered the step, so it includes the time the task
waited for a run and may start before its run. The ids derive from the record's `source` and the
task id and generation, run id or record id, so sending a record again gives the same ids.

| Attribute | Span | Value |
| --------- | ---- | ----- |
| `priory.task.id`, `priory.run.id`, `priory.step`, `priory.lane` | run, step | the record's `task`, `run`, `step` and `lane` |
| `priory.task.id`, `priory.key`, `priory.workflow`, `priory.lane`, `priory.band`, `priory.grade`, `priory.state`, `priory.generation` | root | the task's id, key, workflow, lane, band, grade, final state and generation when it ended |
| `priory.run.ended_as` | run of no step | how the run ended: `cancelled`, `lost` or `blocked` |
| `priory.outcome`, `priory.grade`, `priory.workflow`, `priory.key`, `priory.generation`, `priory.decider`, `priory.effect`, `priory.harness` | step | the record's fields of the same name |
| `gen_ai.request.model` | both | the record's `model` |
| `gen_ai.usage.input_tokens` | step | the record's `input`, `cache_read` and `cache_write` tokens together |
| `gen_ai.usage.output_tokens`, `gen_ai.usage.cache_read.input_tokens`, `gen_ai.usage.cache_creation.input_tokens` | step | the record's `output`, `cache_read` and `cache_write` tokens |
| `gen_ai.operation.name`, `gen_ai.agent.name` | run | `invoke_agent` and the run's agent |

The resource carries `service.name = priory`, `priory.factory` and `host.name`, the machine.

To send to a local ClickStack, Honeycomb or Grafana Cloud, see
[otel-recipes.md](otel-recipes.md).

Each tick, the daemon sends up to 200 records and 200 task ends past the endpoint's cursors in one request, through
`curl`, and moves on without waiting for it. `curl` gives up after `command_timeout`. The cursors, in
the state file's `exports` table (`seq` for records, `ended` for the `task_ends` table, which
the daemon fills when a task ends), move once the endpoint answers with a 2xx status, so a daemon
restart resends nothing it saw accepted. Any other answer, or no answer, is logged and the batch is
tried again 5 seconds later, except a 4xx other than 408 and 429: OTLP says such a request will never
be accepted, so the daemon logs it and skips the batch. A new endpoint starts from the first record.

Each harness process gets `OTEL_RESOURCE_ATTRIBUTES` with `priory.task.id` and `priory.run.id`,
after the daemon's own value when it has one. A process lives as long as its first run's agent keeps
the task, so a run carried into the next step keeps the run id of the run that started it. A harness that exports its own telemetry, such as Claude
Code with `CLAUDE_CODE_ENABLE_TELEMETRY`, then tags it with the same ids in the same backend. Priory
sets nothing else of the harness's telemetry.

Known limits:

- A daemon that stops after `curl` succeeded but before its next tick sends that batch again, with
  the same ids.
- `factory.task.repaired` and `factory.handler.called` records are not exported.
- A task that is still active has no root span yet, so its spans have a parent that arrives later,
  and a run that ended with no step has no span until its task ends.
- A task that ended before the daemon upgraded to this schema has no root span, so its run and step
  spans, when sent after the upgrade, name a parent that was never sent. A task revived before the
  upgrade attaches its earlier generations' runs without a step to its next root span.
- A root span's id uses the daemon's current factory name and host name, so renaming either
  between the steps and the end of a task leaves the root with another id than its children name.
- A task's root span names its lane, band and grade as they stand when it is sent.
- A partial success, a 2xx whose body names rejected spans, counts as accepted.

## Reports

`priory report [DIR] --workflow <name> [--since <window>] [--by lane]` prints, per workflow step, each
outcome with its count, the records that reached the step, and its rate:

```
step	outcome	count	reached	rate
review	approved	1	2	50.0%
review	changes	1	2	50.0%
```

A visit reaches its step when it ends there or while an active task sits at it, so outcome rates for
a step with tasks in progress add up to under 100 %. `--since` takes an RFC 3339 instant or a duration
such as `7 days` (the default), `36h` or `P2W`, counted back from now. Any other text, such as a bare
date or a clock time, is refused. `--by lane` splits counts by the task's current
lane and adds a leading `lane` column. It does not combine with `--cost`.

`--cost` prints each task's tokens and cost instead, per step, then in total. Prices come from
`prices.toml` in the factory folder `DIR` (default: the current folder). Each table is a model, named
as the records name it, with dollars per million tokens of each kind:

```toml
["claude-opus-5-5"]
input = 5.0
output = 25.0
cache_read = 0.5
cache_write = 6.25
```

A model with no price keeps its tokens. Its cost reads `price missing for <model>`, as does its task's
total. A model step with no `tokens` shows `-` and `tokens unknown`, and its task's total names it.
`priory check` refuses a price below zero or not finite. Prices apply at report time, so a changed
price changes past costs too. A task's handler calls show as one more row per model, with step
`handler`, and count in its total. Its summarizer call does too, with step `summarizer`.

## Costs

`priory costs [DIR] [--since <window>] [--by workflow|agent|model|machine|repo] [--json]` prints what
the factory spent in the current window against the window before it, then each day, then the
current window split by one dimension, then each scorer's verdicts by the same split:

```
period	since	until	cost	notes	merges
current	2026-11-15T23:00:00.000Z	2026-11-17T11:00:00.000Z	$2.000000	-	1
previous	2026-11-08T23:00:00.000Z	2026-11-15T23:00:00.000Z	$5.000000	-	1

day	cost	notes	merges
2026-11-14	$2.000000	-	0
2026-11-15	$3.000000	-	1
2026-11-16	$2.000000	-	1

workflow	cost	notes
implement	$2.000000	-

workflow	scorer	pass	fail	error	not_applicable
implement	gate	11	3	1	0
```

A `notes` cell names what has no cost, such as `price missing for m2: 1500 tokens` or
`tokens unknown for 1 records`.

It reads the same records as `report --cost`: model steps, handler calls and summarizer calls, and
prices them from the same `prices.toml` in `DIR` (default: the current folder).

- Without `--since`, the current window starts this Monday at 00:00 in the local timezone (`TZ`) and
  ends now, and the previous window is the full week before it.
- `--since` takes what `report --since` takes, and must start before now. The previous window then
  has the current window's length and ends where the current one starts.
- Days are local dates. A day appears once it has a record or a merge.
- `--by` defaults to `workflow`. `agent` is the run's agent, `handler` for a handler call and
  `summarizer` for a summarizer call. `repo` is the task's repository. A record with no value reads
  `-`.
- A model with no price keeps its tokens apart, as `price missing for <model>`. A row whose records
  were all unpriced or had no tokens has no cost, `-`, never `$0`. A model step with no `tokens`
  counts as `tokens unknown`.
- `merges` counts the pull requests whose stored `github.pull_request.closed` event says they
  merged, when a task owns the pull request or its head branch, as a worker's task owns its branch.
  A pull request counts once, on the local day of its first merged close. The split has no merges.

`--json` prints one object, which `GET /api/costs` also answers (see
[the serve reference](../reference/serve.md#costs)). The web view's Costs panel draws it. Its
`scores` counts, by the same split, each run's latest score by each scorer in the window. A score
takes its run's agent and its run's model. The text output prints the same counts in its last table.

Known limits:

- Prices apply at report time, as for `report --cost`.
- A merge counts only once its webhook or poll stored the closed event.
- A pull request a task only subscribed to does not count: its subscription ends when it closes.

## Outcome labels

The daemon appends a `factory.step.labeled` record when an event judges a task, in the transaction
that stores the event or the answer. It names the record of the step that graded the task, when one
did, the task's grade, and `happened`, when the event happened. A task gets one record per label
and source event, so two reviews that request changes write two. `priory grades` counts a task once
per label.

A task's pull requests are those of its stack (see [tools.md](tools.md)) and those whose head is a
branch it owns. A pull request that only names the task's key, as a review task's does, is not its.
A review whose author ends in `[bot]` does not count as a human review.

| Label | When |
| ----- | ---- |
| `approved-clean` | The last open pull request of the task merged, no human review requested changes on any of them, and each merged at the commit its first human review was written on, or had no human review |
| `changes-requested` | A human review requested changes on one of its pull requests, before or after the merge; or one of them merged at another commit than its first human review was written on, so a commit came after the review. That is checked when its last one merges, and again when a human review of a merged one arrives |
| `approval-rejected` | A person answered one of its human steps with an option that starts with `reject`, such as `rejected`, or that sends the task to `stop` |
| `closed-unmerged` | One of its pull requests closed without merging |
| `red-after-merge` | A check run on the merge commit of one of its merged pull requests completed with a conclusion other than `success`, `neutral` or `skipped` |
| `reverted` | A later pull request of the same repository merged, and its title or body says `This reverts commit <sha>` of the merge commit of one of the task's, as `git revert` writes, or says `Reverts owner/name#N`, as GitHub's revert button writes |
| `marked-wrong` | A person ran `priory label TASK wrong --evidence URL`. Its `happened` is the mark's `--happened`, else when the person ran it. `--clear` withdraws it with a `factory.step.unlabeled` record. See [label.md](../reference/label.md) |

The first four are final: a task with one of them has an outcome its grade can be compared with.
`priory grades` leaves out `marked-wrong`, which only `priory report risk` reads.
`red-after-merge` and `reverted` can come later, on top of `approved-clean`. Each also posts one
report from `priory`, which the desk ranks as an alarm (see [desk.md](desk.md#the-next-thing)):
`landed wrong: <repo>#<n> of task <id> is red on <base>: <check> <conclusion> <url>`, once per merge
commit however many of its checks fail, or `landed wrong: <repo>#<n> of task <id> was reverted by
#<m>: <url>`. A notification's title for it is `a merge landed wrong`. Events count by when
they happened, not when they arrived. A review's commit comes from its event's `commit`, or its
`head_sha` for a review stored before `commit` existed. A task with both `approved-clean` and
`changes-requested`, because a review requesting changes arrived after the merge, counts only as
`changes-requested`.

Known limits:

- The intake reads a merge commit's check runs for 7 days after the merge, until all of them
  completed (see [pull-requests.md](../intake/pull-requests.md#check-runs-at-a-merge)). A check that
  fails later than that, or starts after the others completed, does not label the task.
- A review stored before its event carried `commit` names the head at poll time, so a push between
  the review and the poll is missed.
- A task whose pull request closed unmerged and whose next one merged clean has both
  `closed-unmerged` and `approved-clean`.

## Grades

`priory grades [--since <window>] [--by lane] [--json]` compares each grade with its tasks' outcome
labels. It counts the tasks with a grade, created since `--since`, which takes what `report --since`
takes, or every graded task without it. A task counts under its grade now.

```
green one-shot: 50.0% (1 of 2), target 90%

grade	tasks	outcomes	approved-clean	changes-requested	approval-rejected	closed-unmerged	red-after-merge	reverted
red	0	0	0 (-)	0 (-)	0 (-)	0 (-)	0 (-)	0 (-)
yellow	1	1	1 (100.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)
green	2	2	1 (50.0%)	1 (50.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)	0 (0.0%)
```

The headline is green's one-shot rate: its `approved-clean` tasks over its tasks with a final
outcome, against the 90 % target. `outcomes` counts the tasks with a final outcome, and each label's
share is its count over them; `-` when there are none. A task still waiting for its pull request's
first review has no outcome yet.

`--by lane` splits each grade's row by the lane its tasks have now, with `-` for a task with none.
The table gains a first `lane` column, with one row per lane and grade. The headline still counts
green's tasks in every lane.

`--json` prints one object, which `GET /api/grades` also answers without `--by lane` (see
[the serve reference](../reference/serve.md#grades)).

## Risk

`priory report risk [DIR] [--workflow <name>] [--since <window>] [--by lane] [--json]` estimates
R10.6: of the tasks that ran an irreversible step, what share got an adverse outcome label within
30 days. It reads the factory in `DIR` for each irreversible step and the human steps that approve
it, and the `factory.step.ended` and `factory.step.labeled` records and pull requests of the state. Nothing
reads the estimate to answer an ask, skip a step, or order, route or prompt work.
[risk-estimate.md](../design/risk-estimate.md) explains the model and the choices behind it.

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

Each irreversible step of each workflow is a target. A task reached it at its first
`factory.step.ended` record of the step. The target's own row holds every task that reached it. A
conditioned row, such as `plan green`, holds the tasks whose last record of an earlier step before
the reach had that outcome.

A task is a trial of its rows when its reach lies in the window, is at least 30 days old, and the
task owned a pull request by the end of those 30 days. Its outcome is the labels whose `happened`
lies in the 30 days after the reach:

| Outcome | Labels |
| ------- | ------ |
| `landed wrong` | `red-after-merge` or `reverted`, or `marked-wrong` when a pull request of the task had merged by the mark's `happened` |
| `judged wrong` | Those, or `changes-requested`, `closed-unmerged`, `approval-rejected` or any `marked-wrong` |

A mark that `--clear` withdrew counts nowhere, whenever the clear was written: it says the mark was
a mistake, not that something happened. A mark never makes a task a trial: a task with no pull
request by day 30 stays `unjudged`, marked or not.

The header counts the tasks that reached the target in the window, then:

| Count | Meaning |
| ----- | ------- |
| `caught` | Tasks a human step that approves the target stopped in the window: its answer starts with `reject` or leads to `stop`. They are no trial |
| `pending` | Reach under 30 days old |
| `unjudged` | No pull request by the end of the 30 days, so no label can judge the task |
| `undated` | A label record with no `happened`, written before the field existed |
| `open` | Trials with a pull request still open on day 30 |
| `late` | Trials with an adverse label only after the 30 days. They stay out of the estimate |

Each row and outcome is a Beta(1, 1) prior updated by `k` adverse trials of `n`. The row prints
`k of n`, then the posterior's 5th percentile, median and 95th percentile. Read the 95th as the
posterior upper bound, under the assumptions below. A row under 20 trials prints `k of n, needs 20`
and no percentage. `--json` carries the interval at any `n`, with `"shown": false` under 20. The
30 days and the 20 trials are constants.

`--since` takes what `report --since` takes, and defaults to `180 days`. `--workflow` keeps one
workflow's targets.

`--by lane` splits each target by the lane its tasks have now, with `-` for a task with none. Each
lane gets its own block, headed `implement / open-pr, lane web:`, with its own counts, rows, adverse
trials and candidates. A lane appears when a task of the workflow has it. A lane's rows hold fewer
trials, so most read `needs 20`. No row borrows from its workflow's rate: each lane's estimate uses
the same Beta(1, 1) prior and its own trials only. `--json` then holds `"by": "lane"` and a `lane`
on each target. Without `--by lane`, `by` is null and a target has no `lane`.

The candidates table lists, under each target, the `fixes` relations whose fixed task reached the
target in the window and has no mark left. Priory records such a relation whenever a request links
a merged pull request ([relations.md](relations.md)), and an agent can record one too, so a relation
is not a label. A person reads each candidate and runs `priory label TASK wrong --evidence URL`, or
leaves it. A mark removes it from the table, and a clear puts it back. `--json` holds them in each
target's `candidates`, each with `task`, `fixed_by` and `by`.

The estimate rests on these assumptions:

1. Labels measure recorded judgements, not wrong work. A bug nobody reverted has no label, and a
   flaky check on a merge commit labels right work. The adverse trials table lists each one with its
   evidence.
2. Trials in a row share one rate. A prompt or model change breaks this, so the report prints the
   factory revisions and models a target spans.
3. Trials are independent. Tasks cut from one batch can fail together, which makes the interval too
   narrow.
4. 30 days hold what matters. `open` and `late` show what the horizon missed.
5. A task is one trial per row, however many steps it revisited or pull requests its stack holds.
6. The trials passed a person's gates. The rates say nothing of work with no gate.

The target's own row also shows in three places, over the default window:

- An ask whose answer lets an irreversible step run carries it as `risk_estimate` (see
  [serve.md](../reference/serve.md#asks)). `priory asks` prints its one History line.
- `priory check --explain` adds `caught`, `judged wrong` and `landed wrong` to its first two tables
  when it can read the state file `PRIORY_DB` names, and `-` when it cannot (see
  [workflows.md](../reference/workflows.md#irreversible-steps-and-their-gates)). `priory check`
  without `--explain` never reads the state.
- `priory flows --json` and `GET /api/flows` carry it as each irreversible step's `risk_estimate`.

Known limits:

- `GET /api/risk` comes in a later slice.
- An ask whose answer runs a command, and lets no irreversible step run, has no `risk_estimate`.

## Attention

`priory report attention [--since <window>] [--by workflow|lane|kind] [--json]` measures the PRD's
first success measure: minutes of a person's attention per merged pull request. It covers the
current window of `priory costs`: this week from Monday by default, or `--since`.

```
since	until	acts	person minutes	merges	per merge
2026-11-16T00:00:00.000Z	2026-11-17T11:00:00.000Z	4	16.0	2	8.0

asks	acted	open	median	p90	median present	p90 present
all	4	0	45.5	61.0	45.5	61.0

workflow	acts	person minutes	acted	open	median	p90	median present	p90 present
implement	2	6.0	2	0	60.5	61.0	60.5	61.0
review	2	10.0	2	0	31.0	31.0	15.5	31.0

task	key	workflow	lane	state	wall	working	waiting on person	waiting on world	queued
1	issue-1	implement	lane-a	done	240.0	110.0	30.0	90.0	10.0
```

Times are in minutes. A cell with no value reads `-`.

The report reads what the state already stores. It counts one person: every act is theirs.

- An act is a person's answer, dismissal, refused answer, snooze or unsnooze.
- The person was away during each gap of 2 hours or more between two consecutive acts.
- Person-minutes estimate attention: each act in the window costs the time since the previous act,
  up to 5 minutes. A burst of quick answers costs little, and an act after a break costs 5 minutes.
  `per merge` divides them by `merges`, counted as `priory costs` counts them, and reads `-` with no
  merge.
- A wait runs from when an ask's revision was asked to the person's first act on it: its answer,
  its dismissal, or a snooze before the next revision. `median` is the acted waits' median, and
  `p90` their nearest-rank 90th percentile. The `present` figures subtract the part of each wait that fell in
  an away gap. `open` counts revisions still waiting. A revision the agent replaced or withdrew
  before the person acted counts in neither.
- The asks counted are the revisions asked in the window. Acts count when they fall in it.
- `--by` splits acts and waits by the task's workflow (the default) or lane, or by the ask's kind:
  `step`, `agent` or `outside`. An act or ask with no value reads `-`, such as a task snooze under
  `kind`.

Each task whose current generation overlaps the window gets a row, from its creation to its end,
or to the window's end while it is active. Its wall time splits in this order of precedence, so an
interval hides the lower ones it overlaps:

1. Waiting on the person: each revision of an ask of the task, until its answer, its withdrawal or
   the next revision, and a hold on a person.
2. Working: a live run. It outranks a wait, since a run can go on after its wait is withdrawn.
3. Waiting on the world: a wait, such as on checks, a review or an event, until it fires, and any
   other hold.
4. Queued: the rest, such as waiting for a free slot.

An ask or a wait with no stored end ends at the task's next run. Else a withdrawn wait ends with
its own run, and the rest at the task's end. A task that ended before the state recorded task ends
ends at its last state change.

`--json` prints one object, which `GET /api/attention` also answers (see
[the serve reference](../reference/serve.md#attention)). The web view's Costs panel draws it
beside the costs.

Known limits:

- Every act counts as one person's, however many people use the factory.
- Person-minutes miss attention that leaves no act: reading a diff, a review on GitHub, a `say`.
- An ask dismissed before the state recorded withdrawals (schema 52) has no act, so its wait
  never ends.
- A blocked or failed task waiting on a person counts as queued: the state keeps no state history.
- A task's earlier generations count in no row.
