# Benchmark cases

`priory bench capture` turns a past task into a benchmark case: a folder that a replay plays
again under another configuration. `priory bench replay` plays one case under one configuration.
`priory bench FILE --base REF --head REF` compares two commits of the factory on a benchmark's
cases. [The case reference](../reference/cases.md) lists a case's files and keys, and
[the benchmark reference](../reference/benchmarks.md) a benchmark file's.
[scorers-and-benchmarks.md](../design/scorers-and-benchmarks.md) explains why the comparison works
as it does.

## Capture

```sh
priory bench capture [DIR] TASK --into FOLDER
```

`DIR` is the factory folder, `.` by default. `FOLDER` must not exist: a case is captured once, and
a person reads it before it is committed. The command prints one line, such as:

```text
captured task 1 into benchmarks/cases/eng-412: 2 gh answer(s), 1 answer(s), partial, 1 gap
```

### What it reads

The capture reads the state and the transcripts on this machine. It never asks GitHub, whose
issue may have changed since the task ran.

- **The start.** The task's `base_sha`, the commit its worktree was created from.
- **The end.** The first step the task reached whose effect is `irreversible`, from its
  `factory.step.ended` records, or the step the task sits at now when that step is irreversible.
  Only runs and answers from before that step go into the case.
- **The request.** The first message the inbox routed to the task.
- **`gh` answers.** Each Bash call in the Claude Code transcript of each run before the end. A
  run's calls are the lines between where the previous run of the same transcript stopped reading
  and where this run stopped, so runs that share a session keep their own calls. A call is kept
  when it is one `gh` command alone, with no pipe, list, redirection, substitution or variable,
  and the transcript holds its whole output. Calls that run no `gh` are left out.
- **Answers.** The answers to the task's human steps. An ask the agent raised itself is left out:
  a replay ends there.
- **The reference.** The steps the task finished and its outcome labels.
- **What it spent.** The model, minutes and tokens of each model step before the end, from its
  `factory.step.ended` records. A replay's estimate prices them.

Each call or transcript it cannot keep is a gap, and a case with a gap is `partial`. A person
fills the gap by hand or leaves it. A Codex run is always a gap for now: the capture reads only
Claude Code's transcripts.

### Capture errors

| Error | Cause |
| ----- | ----- |
| `no task <id>` | The state has no such task |
| `task <id> has no start commit: it began before Priory stored one` | The task's worktree was created before Priory stored `base_sha` |
| `task <id> reached no irreversible step, so a replay of it would have no end` | The task never reached a step whose effect is `irreversible` |
| `<folder> already exists: capture into a new folder` | `--into` names a file or folder that exists |

## Replay

```sh
priory bench replay [DIR] CASE --budget USD [--turn-timeout DURATION]
```

`DIR` is the factory folder whose configuration the replay runs, `.` by default. `CASE` is a case
folder. `--budget` is required: the most the replay may spend, in dollars. `--turn-timeout` is
the longest one turn may run, `30m` by default, or the step's `timeout` when it is shorter. It
takes a duration such as `90s`, `20m` or `1h`.

The replay never starts a run for a step whose effect is `irreversible`. The task ends just
before it, and the scratch daemon blocks it there.

### The estimate

Before it starts, the replay prints an estimate:

```text
estimate: $0.4100 of a $5.0000 budget, plus at most $0.9000 if it ends inside a turn of 30m
```

The estimate prices each model step the case recorded with `prices.toml` of `DIR`, for the model
of that step's agent in `DIR`, or the model the case recorded when the agent sets none. The second
figure is the most one turn could cost past the budget: the case's costliest minute, times the turn
timeout. The replay refuses to start when the estimate is over the budget, and names the budget
that fits. It also refuses when `prices.toml` lacks one of the models: a cap in dollars needs every
price.

### What runs

The replay makes a scratch folder in `$TMPDIR` and prints its path. It holds:

- A copy of `DIR`, without `.git` and `runners/`, so the replay runs as a factory of one machine.
  Its `factory.toml` leaves out `log`, `telemetry`, `notify`, `serve`, `webhooks`, `slack`,
  `asks`, `handler`, `search` and `sweep`. Each name in `[asks.targets]` stays, as an empty
  folder of the scratch folder, so a workflow whose step names it still checks. A kit repository
  given as a relative path is made absolute.
- A bare origin of the case's repository, whose `main` is the case's start commit. The commit comes
  from Priory's clone in `$PRIORY_HOME/repos/<owner>/<name>`, so run one task in that repository on
  this machine first. Git's `insteadOf` serves it for `https://github.com/<owner>/<name>.git`, so a
  push lands there.
- A fake `gh`, first on `PATH`. It answers a call that has the same arguments as one of the case's
  `gh/` files with that file's `output`. Any other call fails with `gh: not recorded in this case`,
  and the replay counts it.
- A state, a log and a Priory home of its own, and this build's `priory daemon`. Its tmux server is
  its own too.

The daemon gets a cleared environment. It keeps `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TERM`,
`LANG`, `LC_ALL`, `LC_CTYPE`, `TZ`, `TMPDIR`, the harness's sign-in (`ANTHROPIC_API_KEY`,
`CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CONFIG_DIR`, `OPENAI_API_KEY`, `CODEX_HOME`), `PRIORY_EXE` and
`PRIORY_PROC`. Every other variable, such as `GH_TOKEN`, is dropped.

The replay publishes the case's request to the scratch log. The daemon creates the task, adds its
worktree from the scratch origin, and runs its agents with the real harness. A human step takes
the case's answers for that step, in order.

The replay is not a sandbox. The agent runs with your harness's sign-in, and a script it writes can
reach the network. Replay only cases from repositories and requests you trust.

### Endings

| Ending | When |
| ------ | ---- |
| `reached <step>` | The task got to the irreversible step, which never ran |
| `stopped` | The workflow reached `stop` or `done` before an irreversible step |
| `asked` | The agent asked, or a human step had no recorded answer left |
| `waited` | The task waited on anything but a time, or on a time for longer than the turn timeout |
| `timed out` | A turn ran past the turn timeout |
| `over budget` | What the replay spent passed the budget |
| `agent error: <why>` | The task failed or was blocked, the daemon exited, or the task never started within the turn timeout |
| `cost unknown: <why>` | A model step ended with no tokens, or with a model `prices.toml` does not price |

The replay reads what it spent from the scratch daemon's step tokens, and checks it as each turn
ends. Priory reads a turn's tokens when the turn ends, so a replay can pass its budget by the cost of
the turn in progress. On `timed out`, `over budget`, `cost unknown`, `asked` and `waited`, the replay
cancels the task. Then it stops the daemon and the tmux server, and removes the task's worktree and
the scratch folder, each by its exact path.

The scratch folder is `priory-replay-<pid>-<random>` in the temporary folder. When you stop the
replay with a signal, such as Ctrl-C, it cleans up nothing at once, and the agent may go on with its
turn. The next replay, `priory score`, a judge's calibration or the daemon's sweep cleans up
instead: for each such folder whose pid no longer runs, it kills the scratch tmux server through
that folder's own socket, then removes the folder.

It prints, for example:

```text
estimate: $0.4100 of a $5.0000 budget, plus at most $0.9000 if it ends inside a turn of 30m
scratch: /tmp/priory-replay-41022-9f2c4e1a7b3d4c5e8a6b0d1f2e3c4b5a
answer: approve-plan approved
ending: reached open-pr
spent: $0.4700 of $5.0000 in 612s
gh calls not recorded: 1
  gh pr view 7 --json state
```

On `over budget` and `cost unknown`, a line `Incomplete` comes before the ending: the replay
stopped short, so its spending and its ending do not measure the configuration.

### Replay errors

| Error | Cause |
| ----- | ----- |
| `give --budget USD, the most the replay may spend` | `--budget` is missing |
| `the estimate, $<x>, is over the $<y> budget: give --budget <z> or more` | The case's recorded steps cost more than the budget |
| `prices.toml has no price for <model>, the model of step <step>: a cap in dollars needs every price` | A model of the estimate has no price |
| `step <step> names no model, so no price applies: set its agent's model` | Neither the agent nor the case names the step's model |
| `the case records no model step's tokens, so nothing can be estimated: capture it again` | The case has no `spent`, as a case captured before Priory recorded it |
| `the case holds no tokens for step <step>, so nothing can be estimated: fill them in case.toml` | A `spent` entry has no `tokens` |
| `the configuration has no workflow <name>, which the case ran` | `DIR` lacks the case's workflow |
| `the case names no repository: set repo in case.toml` | `case.toml` has no `repo` |
| `the case holds no request: fill [request] in case.toml` | `case.toml` has no `request` |
| `no clone of <repo> in <path>: run one task in <repo> on this machine first` | Priory has not cloned the repository on this machine |
| `the clone <path> lacks the case's start commit <sha>: fetch it there` | The clone does not hold the start commit |

### Check a replay with a real agent

The tests replay with a fake `claude`, so they cost nothing. To check a replay with a real agent,
capture a small finished task and replay it with a low budget:

```sh
priory bench capture path/to/factory 12 --into /tmp/case-12
priory bench replay path/to/factory /tmp/case-12 --budget 2 --turn-timeout 10m
```

Read the ending, the spending against the estimate, and the `gh` calls not recorded.

## Compare

```sh
priory bench FILE --base REF --head REF --budget USD [--runs N] [--case NAME] [--turn-timeout DURATION] [--post OWNER/NAME#N]
```

`FILE` is a benchmark file, `benchmarks/<name>.toml` in a factory folder inside a Git repository.
`--base` and `--head` name two commits of that repository, such as `main` and a branch. `--budget`
is required: the most the comparison may spend, in dollars. `--runs` is the number of pairs each
case gets, 3 by default. `--case` runs only the case whose folder has that name. `--turn-timeout`
is as for a replay. `--post` names the pull request the comparison is for, such as
`owner/factory#12`: see [Post to a pull request](#post-to-a-pull-request).

The only thing that differs between the two sides is the factory's files at each commit. The
benchmark file, its cases and its scorers come from `--head` for both sides.

### What it does

1. It writes the files of each commit into a scratch folder in `$TMPDIR`, and refuses a commit
   whose factory fails `priory check`.
2. It reads each `judge` scorer's calibration from this machine's trace, as `priory score` does
   ([score.md](score.md#judges)). A judge that is not calibrated is left out whole, on both sides,
   and the table names it. It refuses to start when the primary scorer is such a judge.
3. It prints an estimate: each case's recorded steps priced for each side's models, plus each
   judge's calls priced at what one call cost when it was calibrated, one per recorded model step,
   times `--runs`. It refuses to start when the estimate is over the budget, and names the `--runs`
   that would fit.
4. Each `command` scorer of the benchmark runs once on each case's untouched start commit, from
   Priory's clone on this machine. A case where one errors is dropped for both sides, and the
   table names it. So an `error` during the comparison points at the change.
5. It replays each case in pairs: one replay of the base, one of the head, as
   [Replay](#replay) describes. The side that goes first alternates from one pair to the next, so
   a warm cache or a drifting model service favors neither side. It prints one line per replay,
   such as `eng-412 pair 1: head reached open-pr, $0.4100`.
6. Once a replay ends, and before its scratch folder goes, the benchmark's scorers grade it. A
   `result` or `diff` scorer takes its verdict on the replay's last finished run. An `ask` scorer
   takes the worst of its verdicts on the runs that asked: an error, then a fail, then a pass. A
   judge's calls count toward the budget, and the replay's printed cost includes them. The judges
   get only what the comparison and that replay left of the budget. After a replay that ended
   `over budget` or `cost unknown` they get nothing, so they call no model.
7. It prints the table, and appends a `factory.bench.compared` record to this machine's trace
   ([the trace schema](../trace-schema.md#factorybenchcompared)).

Before each pair, the comparison checks what it spent. Once the budget is spent, no later pair
starts. A pair whose replay ends `over budget` or `cost unknown` is dropped whole, and no later
pair starts. So is a pair where a judge ran out of budget, or spent past it. Either way the
comparison is incomplete.

### The table

```text
benchmark implement-november, base 999daf0, head 4c1e2b7, comparison 1 of these commits
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

The first scorer is the benchmark's `primary`. For each scorer, each side shows its passes over
the replays that count, and its errors:

- For a `result` or `diff` scorer, every replay counts. A `fail` and an `error` are both not
  passed. A replay that ended with no result, such as `asked`, `waited` or `timed out`, is not
  passed either, so a change cannot raise its share by asking more.
- For an `ask` scorer, only the replays that asked count.

Each case gives one difference per scorer: the share of the head's replays that passed, less the
base's. `mean diff` is the mean of those differences, in points. `won`, `tied` and `lost` count
the cases where the difference is above, at or below zero. A case where a side has no replay that
counts is left out of all four.

The second table counts each side's [endings](#endings), and shows the median cost and the median
time of a replay.

### The verdict

Only the primary scorer gets a verdict, from an exact sign test on the cases it did not tie:

| Verdict | When |
| ------- | ---- |
| `better` | The two-sided probability of a split at least as uneven is under 0.05, and the head won more cases |
| `worse` | The same, and the head lost more cases |
| `no clear difference` | The probability is 0.05 or more |
| `too few cases` | Fewer than 6 cases did not tie: no split of 5 or fewer can reach 0.05 |

A tie carries no information for the sign test, so a change that helps a little on every case can
read `no clear difference`. The mean difference is there for that.

The table gives no verdict in two cases:

- **Incomplete.** A pair was dropped, so the first line starts with `Incomplete` and says why.
- **A repeat.** An earlier complete comparison of the same two commits and benchmark on this
  machine gave the verdict. The header says `a repeat`, and the verdict column reads `repeat`.
  Running again until a `better` shows up would find one by chance.

The header also says when the diff between the two commits touches `scorers/` or `benchmarks/`,
since the change then moved the ruler too, and it names the cases whose capture left gaps. Each
judge of the benchmark gets one line above the table, with its agreement:

```text
judge does-what-was-asked: opus, agreed on 9/10 labeled cases
left out on both sides: tone: not calibrated: it agreed on 6/10 labeled cases, under 80%
```

### Compare errors

| Error | Cause |
| ----- | ----- |
| `give --budget USD, the most the comparison may spend` | `--budget` is missing |
| `<file> is not in a factory's benchmarks/ folder` | `FILE` is not `benchmarks/<name>.toml` |
| `<dir> is not in a Git repository` | The factory folder is not in a Git repository |
| `no commit <ref> in <repository>` | `--base` or `--head` names no commit |
| `the base commit <sha> fails priory check: <problem>` | The factory at a commit has a problem. The first is named |
| `the head commit <sha> has no benchmark <name>` | The head commit lacks `FILE` |
| `the benchmark <name> has no case <case>` | `--case` names no case of the benchmark |
| `the case <path>: <why>` | A case folder is missing or does not load |
| `cannot fetch <ref> from the origin of <repository>: <why>` | With `--post`, the origin lacks `--base` or `--head`, or cannot be reached |
| `<base> and <head> have no merge base` | With `--post`, the two commits share no history |
| `the comparison ran, but its comment was not posted: <why>` | With `--post`, `gh pr comment` failed. The table is printed and recorded |
| `the comparison ran, but its status was not set: <why>` | With `--post`, `gh api` failed to set the status. The comment is posted |
| `the estimate, $<x>, is over the $<y> budget: give --runs <n>, or --budget <z> or more` | The estimate is over the budget, and fewer runs would fit |
| `the primary scorer <name> is a judge whose verdict does not count yet: not calibrated: <why>` | The primary scorer is a judge that is not calibrated |

The [replay errors](#replay-errors) apply to each replay, and stop the comparison.

### Post to a pull request

With `--post OWNER/NAME#N`, the comparison is for pull request `N` of `OWNER/NAME`:

1. Before it resolves the commits, it fetches `--base` and `--head` from the `origin` of the
   factory's repository, so a pull request's head that this clone lacks is found. `--head` names a
   commit, and the comparison runs from the merge base of `--base` and `--head`, so the changes the
   base branch got since are not counted as the pull request's.
2. The header counts the comparisons of this pull request on this machine: `comparison 2 of this
   pull request`. A repeat is still one of the same two commits and benchmark.
3. After it prints the table, it writes it as one comment on the pull request with
   `gh pr comment`, and prints `posted: <url>`.
4. It sets the commit status `priory/benchmark` on the head commit with `gh api`, and prints
   `status: priory/benchmark <state> on <commit>`:

| State | When | Description |
| ----- | ---- | ----------- |
| `success` | The comparison is complete and the primary scorer does not read `worse` | `<primary>: <verdict>`, such as `gate: better` |
| `failure` | The primary scorer reads `worse` | `<primary>: worse` |
| `failure` | The comparison is incomplete | `Incomplete: <why>` |

A repeat writes its comment and leaves the status as it is, and prints `status: left as it is,
since this comparison is a repeat`.

The status is bound to the head commit, so a push after the comparison leaves the new head with no
status. A repository that wants a comparison before every merge of the paths it cares about makes
`priory/benchmark` a required status there. The comparison never merges, and its status never
approves a pull request.

### Before a change merges

The starter [`examples/benchmark/`](../../examples/benchmark/README.md) compares a pull request on
the factory's own repository when someone adds the label `benchmark`:

1. Its trigger starts a task on `pull_request.labeled` with the label `benchmark`.
2. Its workflow is one human step, `compare`. Its ask names the pull request and the head commit of
   the labeled event, such as `Compare pull request 12 at <full commit> on the implement benchmark,
   for at most $40?`, with the options `run` and `skip`. The task has no branch and no worktree, and no
   agent runs.
3. Answering `run` with `priory answer --run` runs the step's command once, in the factory folder:
   `priory bench benchmarks/implement.toml --base main --head <commit> --budget 40 --post
   owner/factory#12`. The step's `timeout` bounds it. A failed command, such as an estimate over
   the budget, asks the step again with its exit.

A label is text a bot or an agent can add. The answer is a person's, and no run can give one, so
a label alone runs nothing. The command takes the commit the ask named, so a push after the answer
changes nothing of what runs. A pull request from a fork, or one whose event does not name its head
repository, is dropped with the reason: its prompts and scripts would run with your sign-in (see
[workflows.md](../reference/workflows.md#fields-of-the-tasks-event)). To compare again, remove the
label and add it again: a new task asks again.

### Check a comparison with a real agent

The tests compare with a fake `claude`, so they cost nothing. To check a comparison with a real
agent, make a benchmark of one small case, change one agent's prompt on a branch, and compare with
one run and a low budget:

```sh
priory bench path/to/factory/benchmarks/smoke.toml --base main --head my-branch \
  --budget 4 --runs 1 --turn-timeout 10m
```

Read the estimate against what was spent, each replay's ending, and the table.
