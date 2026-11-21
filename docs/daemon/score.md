# Scores

`priory score` grades a task's finished runs with the factory's scorers, on demand. A scorer is a
file in `scorers/` ([the scorer reference](../reference/scorers.md)). No scorer runs by itself
after a run.

```sh
priory score [DIR] TASK [--scorer NAME] [--json]
```

`DIR` is the factory folder, `.` by default. The command scores the latest finished run of each
step of the task: the last run of that step whose turn ended with a result or an ask. `--scorer`
runs one scorer instead of all of them. It prints one row per run and scorer:

```text
run	step	scorer	verdict	reason
1	plan	passed	fail	outcome ready, not passed
1	plan	short-ask	not applicable	the run asked nothing
2	implement	passed	pass	outcome passed
2	implement	short-ask	not applicable	the run asked nothing
```

A verdict is `pass`, `fail`, `not applicable` or `error`, and `reason` is its line of why. `--json`
prints `{"scores": [...]}`, each entry with the same five fields. The command fails when the task
does not exist, has no finished run, or the factory has no scorer of that name.

## A `command` scorer

A `command` scorer does not run yet: it answers `error: command scorers do not run yet`.

## Records

Each score appends one `factory.run.scored` record to the trace, as [the trace
schema](../trace-schema.md#factoryrunscored) defines it. A score is a record: scoring a run again,
with the same scorer or a changed one, appends another record and replaces none.
