# Benchmark cases

`priory bench capture` turns a past task into a benchmark case: a folder that a replay will play
again under another configuration. [The case reference](../reference/cases.md) lists its files and
keys. [scorers-and-benchmarks.md](../design/scorers-and-benchmarks.md) explains the benchmark it
belongs to. The replay and the comparison are not built yet.

```sh
priory bench capture [DIR] TASK --into FOLDER
```

`DIR` is the factory folder, `.` by default. `FOLDER` must not exist: a case is captured once, and
a person reads it before it is committed. The command prints one line, such as:

```text
captured task 1 into benchmarks/cases/eng-412: 2 gh answer(s), 1 answer(s), partial, 1 gap
```

## What it reads

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

Each call or transcript it cannot keep is a gap, and a case with a gap is `partial`. A person
fills the gap by hand or leaves it. A Codex run is always a gap for now: the capture reads only
Claude Code's transcripts.

## Errors

| Error | Cause |
| ----- | ----- |
| `no task <id>` | The state has no such task |
| `task <id> has no start commit: it began before Priory stored one` | The task's worktree was created before Priory stored `base_sha` |
| `task <id> reached no irreversible step, so a replay of it would have no end` | The task never reached a step whose effect is `irreversible` |
| `<folder> already exists: capture into a new folder` | `--into` names a file or folder that exists |
