# Reports and picks

A workflow writes a report for a person: ordered sections of rows, such as Bonjour's morning
dossier. A row can carry a pick, a choice the person makes. Priory keeps the latest report of each
task, with a revision.

This report is not `priory report --workflow`, which sums trace records ([trace.md](trace.md)), nor
a report line that `priory post` puts on the desk ([desk.md](desk.md)).

## Write a report

An agent step calls the `report` tool ([tools.md](tools.md)). A code step or a script runs:

```sh
priory report put TASK FILE
priory report put TASK -        # the report on stdin
```

Both check the shape first and refuse a bad one with the reason, storing nothing. Each write
replaces the task's report and raises its revision by one. A write keeps the choice already made on
each pick, by pick id, whatever `chosen` it carries.

`docs/schemas/report.json` is the JSON Schema of the shape. In short:

```json
{
  "date": "2026-11-20",
  "since": "2026-11-19T18:00:00+01:00",
  "greeting": "Bonjour, Max",
  "sections": [
    {
      "id": "sentry",
      "title": "Sentry",
      "count": 3,
      "rows": [
        {
          "id": "fnd-2409",
          "key": "FND-2409",
          "text": "Release build pauses on a DWARF breakpoint",
          "meta": "41 events since 18:00",
          "state": null,
          "owner": null,
          "time": null,
          "pick": {
            "id": "2026-11-20_sentry_fnd-2409",
            "options": ["Take", "Later", "Drop"],
            "recommended": "Take",
            "chosen": null,
            "applied": null
          },
          "comments": []
        }
      ]
    }
  ]
}
```

Priory refuses a report when:

- a field is missing, unknown or of the wrong type;
- `date` is not a date, or `since` is not an RFC 3339 instant;
- two sections share an id, or two picks share an id;
- a pick has no options, or its `recommended` or a `requests` key is not one of its options;
- a row's `comments` is not empty. Comments on rows come in a later version.

## Read a report

```sh
priory report show [TASK] [--json]
```

It prints the task's report, or without `TASK` the report written last. `--json` prints the report
with two more fields, `task` and `revision`, as `GET /api/reports/latest` and
`GET /api/reports/{task}` return it ([serve endpoints](../reference/serve.md)).

## Pick an option

```sh
priory report pick PICK OPTION --expected REVISION
```

The daemon finds the report that holds the pick and records `OPTION` as its `chosen`. It refuses
the pick when:

- the report is at another revision than `REVISION`, as an answer to a stale ask is refused;
- the pick already has a choice: the first one stands;
- `OPTION` is not one of the pick's options.

A recorded pick raises the report's revision. Then the daemon wakes the report's task with one
line, as its next prompt:

```text
The person picked Take on 2026-11-20_sentry_fnd-2409 in your report: apply it, then write the report again with the pick's applied filled.
```

The workflow applies the choice and writes the report again with `applied` set. A pick on the report
of a done, failed or dropped task is refused, since nothing would apply it.

## Request a task from a pick

A pick's `requests` maps an option to a task request, with the fields of `priory request`
([docs/intake/cli.md](../intake/cli.md)):

```json
"requests": {
  "Take": { "workflow": "fix", "key": "FND-2409", "text": "Fix the DWARF pause", "repo": "owner/repo" }
}
```

`workflow`, `key` and `text` are required; `band`, `lane`, `repo`, `tldr` and `hold` are optional.
Picking that option publishes the request, as `priory request` does, and sets `applied` to
`requested task cli:<key>`. It wakes no task.
