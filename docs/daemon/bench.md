# Benchmark cases

`priory bench capture` turns a past task into a benchmark case: a folder that a replay plays
again under another configuration. `priory bench replay` plays one case under one configuration.
[The case reference](../reference/cases.md) lists a case's files and keys.
[scorers-and-benchmarks.md](../design/scorers-and-benchmarks.md) explains the benchmark it
belongs to. The comparison of two configurations is not built yet.

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
  `asks`, `handler`, `search` and `sweep`. A kit repository given as a relative path is made
  absolute.
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

When you stop the replay with a signal, it cleans up nothing: stop its daemon and the scratch
tmux server, then remove the scratch folder it named.

It prints, for example:

```text
estimate: $0.4100 of a $5.0000 budget, plus at most $0.9000 if it ends inside a turn of 30m
scratch: /tmp/priory-replay-41022
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

### Check it with a real agent

The tests replay with a fake `claude`, so they cost nothing. To check a replay with a real agent,
capture a small finished task and replay it with a low budget:

```sh
priory bench capture path/to/factory 12 --into /tmp/case-12
priory bench replay path/to/factory /tmp/case-12 --budget 2 --turn-timeout 10m
```

Read the ending, the spending against the estimate, and the `gh` calls not recorded.
