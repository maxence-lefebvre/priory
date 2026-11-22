# Benchmarks

A benchmark is the set of cases a comparison replays and the scorers that grade them. Each file
`benchmarks/<name>.toml`, `.yaml` or `.yml` holds one, named by its file. `priory bench` compares
two commits of the factory on it ([bench.md](../daemon/bench.md#compare)).

Schema: [`config/benchmark.json`](../schemas/config/benchmark.json).

## Keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `description` | string | yes | | What the cases are, in one line | `"Twelve implement tasks merged in November."` |
| `workflow` | string | yes | | The workflow the cases ran | `"implement"` |
| `primary` | string | yes | | The scorer whose verdict the comparison gives. It must read `result` or `diff`, so it scores every replay | `"gate"` |
| `scorers` | list of strings | no | `[]` | Other scorers, shown beside the primary one with no verdict | `["passed", "short-ask"]` |
| `cases` | list of paths | yes | | The case folders, each relative to the benchmark file's folder ([cases.md](cases.md)) | `["cases/eng-412"]` |

`priory check` does not read the case folders. `priory bench` loads them from the head commit and
fails on one that is missing.

## Example

<!-- example: factory/benchmarks/implement.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/benchmark.json
description = "Twelve implement tasks merged in November."
workflow = "implement"
primary = "gate"                    # the one scorer that gives the verdict
scorers = ["passed", "short-ask"]   # shown beside it, with no verdict
cases = ["cases/eng-412", "cases/eng-431", "cases/app-57"]   # beside this file
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` no workflow `<name>` in workflows/ `` | `workflow` names no workflow |
| `` no scorer `<name>` in scorers/ `` | `primary` or an entry of `scorers` names no scorer |
| `` the primary scorer `<name>` reads asks, which not every replay makes: use one on result or diff `` | `primary` names a scorer with `on = "ask"` |
| `` a benchmark needs `cases`: folders that priory bench capture wrote `` | `cases` is empty |
| `` the case <path> is not inside the benchmarks folder `` | A case path is absolute or climbs out with `..` |
| `` unknown field `<key>`, expected one of ... `` | A key the table above does not list |
