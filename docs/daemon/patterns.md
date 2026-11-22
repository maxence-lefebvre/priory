# Failure patterns

A failure pattern is a set of tasks that stopped at one step for the same reason. One failed task
is a `failed` row on the desk. Nine tasks that failed at `implement` this week for the same reason
are a pattern: a fault in the factory rather than in one task. Priory groups them so a person sees
the fault once, fixes the factory, and checks the fix on the tasks that showed it.

The design note, [failure-patterns.md](../design/failure-patterns.md), explains why it works this
way and what comes next.

## Finding patterns

```sh
priory patterns find [DIR] [--since <instant-or-duration>]
```

`DIR` is the factory folder, `.` by default. `--since` takes a duration such as `24h` or `7 days`,
or an RFC 3339 instant, and is `24h` by default. The command reads four kinds of evidence from the
window:

| Kind | Evidence | Step | Text |
| ---- | -------- | ---- | ---- |
| `failed` | A task that is `failed` now and changed state in the window | The step it sits at | Its reason |
| `blocked` | A task that is `blocked` now and changed state in the window | The step it sits at | Its reason |
| `repaired` | A `factory.task.repaired` record of the window | The record's step | Its cause |
| `marked-wrong` | A `priory label TASK wrong` mark of the window that no `--clear` withdrew | The step that graded the task, else the step it sits at | `marked wrong by a person` |

It groups the evidence by workflow, step and signature. The signature is the text's first line in
lower case, with each number and each hex word of 7 or more characters with a digit replaced by
`#`, runs of space collapsed, and at most 120 characters. So `cargo test failed: 3 tests failed in
41s` and `cargo test failed: 12 tests failed in 9s` are one pattern. A pattern's id is the first 8
hex digits of the SHA-256 of its workflow, step and signature, so the same failure next week joins
the same pattern.

For each piece of evidence that no earlier record holds, it appends one `factory.pattern.matched`
record to the trace ([trace-schema.md](../trace-schema.md#factorypatternmatched)). A pattern is the
records of its id, so it needs no table, and running the command twice over one window records
nothing new. A stopped task counts once per generation and reason. The text is redacted of
credential shapes before it is recorded.

It prints the count of new matches, then one line per pattern that gained one:

```text
2 new matches from 2026-11-21T03:00:00.000Z to 2026-11-22T03:00:00Z
3fa2b1c0	open	implement/implement	+2, 5 tasks	cargo test failed: 12 tests failed in 9s
```

### Each night

The [`dreaming`](../../examples/dreaming/README.md) starter runs it at 02:45 with a schedule
trigger and a workflow of one code step, which costs no model:

```toml
[[steps]]
id = "find"
decides = "code"
action = "run"
run = "priory patterns find"
effect = "read"
timeout = "2m"
next = { ok = "done", failed = "stop" }
```

A task that blocked and was resumed before the step ran is not found, since it is no longer
`blocked`. Run the step more often, such as every hour, to catch those.

## On the desk

The desk shows one `pattern` row for each open pattern of at least 2 tasks:

```text
implement/implement: 5 tasks stopped for one reason: cargo test failed: 12 tests failed in 9s; see it with `priory patterns show 3fa2b1c0`
```

It ranks with the rest, after every ask and to-do, and its title is `tasks stopped for one
reason`. It has no task, and no snooze. The web Desk and the Herdr Desk show it as a plain row.
See [desk.md](desk.md).

## Listing and showing

- `priory patterns [--json]` lists the patterns the desk shows. `--all` adds every other one: one
  of a single task, closed or dismissed. Columns: id, state, workflow, step, tasks, last, text.
- `priory patterns show ID [--json]` prints the pattern, then one line per match: task, key, kind,
  when it happened and its text.

A pattern's `state` is `open`, `closed` or `dismissed`. `tasks` counts distinct tasks, `last` is
when the latest one stopped, and `text` is the latest one's text.

## Closing and dismissing

- `priory patterns close [DIR] ID` closes a pattern once a fix merged. It leaves the desk. A task
  that stops after the close and matches it opens it again, with its whole count: the fix did not
  hold. A task found after the close that stopped before it leaves it closed.
- `priory patterns dismiss [DIR] ID` says it is no pattern. It never opens again, though `find`
  still records its matches.

Each appends a `factory.pattern.closed` record. Only a person runs them: the guard refuses both
inside a run, as for `priory label`.

| Error | Cause |
| ----- | ----- |
| `` no pattern <id>: list them with `priory patterns --all` `` | No match record names the id |
| `pattern <id> is already closed` | `close` on a closed pattern |
| `pattern <id> is already dismissed` | `close` or `dismiss` on a dismissed pattern |

## Capturing a pattern's tasks

```sh
priory patterns capture [DIR] ID --into FOLDER
```

It captures each task of the pattern into `FOLDER/task-<id>`, as
[`priory bench capture`](bench.md#capture) does, and prints one line per task: the capture's line,
or `skipped task <id>: <why>` for a task that cannot be captured or whose folder exists. A `failed`
or `blocked` task that reached no irreversible step is captured up to the step it stopped at. It
fails with `no task of pattern <id> could be captured` when it captured none. Only a person runs
it.

Add the folders to a benchmark's `cases`, then compare the fix on them with
`priory bench FILE --base main --head <fix>` ([bench.md](bench.md#compare)) before a person merges
it.

## Limits

- `find` reads the state file of the machine that runs it. In a factory of several runners, it
  covers that runner's tasks.
- A signature groups texts that differ only in numbers and hashes. Two reasons that name different
  files or branches are two patterns.
- Nothing proposes the fix yet. See the design note's later slices.
