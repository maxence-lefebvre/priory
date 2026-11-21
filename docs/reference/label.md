# Marks

`priory label` records a person's judgement that a task's work was wrong, for `priory report risk`
to count. [trace.md](../daemon/trace.md#risk) says how the report reads it, and
[risk-estimate.md](../design/risk-estimate.md) why only a person writes it.

## `priory label`

```sh
priory label [DIR] TASK wrong --evidence URL [--happened DATE]
priory label [DIR] TASK wrong --clear
```

| Argument or flag | Default | Meaning |
| ---------------- | ------- | ------- |
| `DIR` | `.` | The factory folder. Its name and revision go in the record |
| `TASK` | required | The task's id |
| `wrong` | required | The judgement. It is the only one |
| `--evidence URL` | required to mark | An `http://` or `https://` URL that shows the work was wrong, such as the pull request that fixed it forward |
| `--happened DATE` | now | When the wrong work showed. An RFC 3339 instant, or a date such as `2026-10-02`, read as midnight UTC. It may not be after now |
| `--clear` | | Withdraws every mark of the task. It takes neither `--evidence` nor `--happened` |

A mark appends a `factory.step.labeled` record with label `marked-wrong`, `evidence` the URL and
`happened` the date. The same mark twice writes one record. It prints:

```text
task 27 marked wrong: https://github.com/example/app/pull/91
```

`--clear` appends a `factory.step.unlabeled` record that withdraws the marks before it
([trace-schema.md](../trace-schema.md#factorystepunlabeled)). A mark after it counts again. It prints:

```text
task 27 is no longer marked wrong
```

Both write the state file directly, with or without a running daemon. Only a person runs it: inside
a run's shell it exits 2, and the `priory-person` guard denies it to every agent
([guard.md](../daemon/guard.md)).

## Errors

The command exits 1 with one of these errors and records nothing:

| Error | Cause |
| ----- | ----- |
| `no task <id>` | The task does not exist |
| `--evidence "<text>" is not a URL: name the issue, pull request or commit that shows the task was wrong` | The evidence does not start with `http://` or `https://` |
| `--happened "<text>" is neither an RFC 3339 instant nor a date such as 2026-10-02` | The date does not parse |
| `--happened <date> is after now` | The date is in the future |
| `task <id> is not marked wrong` | `--clear` on a task with no mark left to withdraw |

It exits 2 and prints the usage when `wrong` is missing, when neither `--evidence` nor `--clear` is
given, or when `--clear` comes with `--evidence` or `--happened`.
