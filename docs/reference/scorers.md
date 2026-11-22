# Scorers

A scorer grades one finished run: a run whose turn ended with a result or an ask. Each file
`scorers/<name>.toml`, `.yaml` or `.yml` holds one, named by its file. A scorer answers `pass`,
`fail`, `not applicable` or `error`. `priory score` runs them on demand, and no scorer runs by
itself after a run (see [score.md](../daemon/score.md)).

Schema: [`config/scorer.json`](../schemas/config/scorer.json).

## Keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `description` | string | yes | | What the scorer checks, in one line | `"The step ended as passed."` |
| `on` | `result`, `diff` or `ask` | yes | | What the scorer reads. Each kind reads one: see below | `"result"` |
| `kind` | `outcome`, `command` or `ask-shape` | yes | | How the scorer answers | `"outcome"` |
| `expect` | list of strings | for `outcome` and `ask-shape` | | The outcomes that pass, or the parts of an ask's shape every ask must show | `["passed"]` |
| `run` | path | for `command` | | An executable in the factory folder | `"scripts/score-gate.sh"` |
| `timeout` | duration | no | `"20m"` | For `command`: how long it may run before Priory kills it | `"5m"` |

## Kinds

| `kind` | `on` | Passes when |
| ------ | ---- | ----------- |
| `outcome` | `result` | The result's outcome is one of `expect` |
| `command` | `diff` | Not run yet: every run scores `error` (see [score.md](../daemon/score.md#a-command-scorer)). Once built, `run` exits 0 in a scratch checkout of the result's commit. Exit 1 fails, and any other ending is an error |
| `ask-shape` | `ask` | Every ask the run made shows each part `expect` names |

An ask's shape is the one `priory eval` reads ([prompt-evals.md](prompt-evals.md)): `short` when
its question is one line of at most 25 words, `whatWaits` and `nextAction` when it filled them.

A `result` or `diff` scorer fails a run that ended with an ask: asking is not finishing. An `ask`
scorer answers `not applicable` on a run that asked nothing.

`command` scorers do not run yet. Once built, a `command` scorer runs with a cleared environment
and these variables:

| Variable | Holds |
| -------- | ----- |
| `PRIORY_SCORE_BASE` | The commit the task started from, its `base_sha` |
| `PRIORY_SCORE_HEAD` | The result's `commit` |
| `PRIORY_SCORE_RESULT` | The path of a JSON file that holds the result |

It answers `error: the result names no commit` for a result without a `commit`, and
`error: no start commit` for a task created before Priory stored its start commit.

The `judge` kind, a model that grades a change, is not built yet. See
[scorers-and-benchmarks.md](../design/scorers-and-benchmarks.md).

## Example

<!-- example: factory/scorers/passed.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/scorer.json
description = "The step ended as passed."
on = "result"
kind = "outcome"
expect = ["passed"]
```

<!-- example: factory/scorers/gate.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/scorer.json
description = "The tests pass on the change."
on = "diff"
kind = "command"
run = "scripts/score-gate.sh"  # in the factory folder; exit 0 pass, 1 fail, else error
timeout = "20m"
```

<!-- example: factory/scripts/score-gate.sh -->
```sh
#!/bin/sh
# Runs in a scratch checkout of the result's commit, with PRIORY_SCORE_BASE, PRIORY_SCORE_HEAD
# and PRIORY_SCORE_RESULT set. Exit 0 passes, 1 fails, anything else is an error.
cargo test --quiet || exit 1
```

<!-- example: factory/scorers/short-ask.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/scorer.json
description = "Each ask is short and says what waits."
on = "ask"
kind = "ask-shape"
expect = ["short", "whatWaits", "nextAction"]
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` kind `judge` is not supported yet: use outcome, command or ask-shape `` | A `judge` scorer |
| `` unknown variant `<kind>`, expected one of `outcome`, `command`, `ask-shape` `` | A kind not listed above |
| `` kind `<kind>` reads on = "<on>", not "<other>" `` | An `on` that does not fit the kind |
| `` an `outcome` scorer needs `expect`: the outcomes that pass `` | An `outcome` scorer with no `expect` |
| `` an `ask-shape` scorer needs `expect`: some of short, whatWaits, nextAction `` | An `ask-shape` scorer with no `expect` |
| `` `<part>` is not part of an ask's shape: use short, whatWaits, nextAction `` | An `ask-shape` `expect` that names something else |
| `` a `command` scorer takes no `expect`: its exit code is its answer `` | A `command` scorer with `expect` |
| `` a `command` scorer needs `run`: an executable in the factory folder `` | A `command` scorer with no `run` |
| `` `run` names <path>, which is missing from the factory folder `` | `run` names no file |
| `` `run` names <path>, which is not executable: chmod +x it `` | `run` names a file without its execute bit |
| `` `run` names <path>, which is not in the factory folder `` | `run` is absolute or climbs out with `..` |
| `` `<key>` is for kind `command`, not `<kind>` `` | `run` or `timeout` on another kind |
| `` unknown field `<key>`, expected one of ... `` | A key the table above does not list |
