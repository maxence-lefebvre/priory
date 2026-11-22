# Scorers

A scorer grades one finished run: a run whose turn ended with a result or an ask. A `held` scorer
grades a merged task instead, once a delay after its merge has passed. Each file
`scorers/<name>.toml`, `.yaml` or `.yml` holds one, named by its file. A scorer answers `pass`,
`fail`, `not applicable` or `error`. `priory score` runs them on demand, except a `held` scorer. A step's
[`gate`](workflows.md#step-keys) and the factory's [`critic`](factory.md#top-level-keys) run them on
a result before the daemon stores it (see [score.md](../daemon/score.md#gates)). The daemon runs
each `held` scorer by itself (see [score.md](../daemon/score.md#held-scorers)).

Schema: [`config/scorer.json`](../schemas/config/scorer.json).

## Keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `description` | string | yes | | What the scorer checks, in one line | `"The step ended as passed."` |
| `on` | `result`, `diff`, `ask` or `merge` | yes | | What the scorer reads. Each kind reads one: see below | `"result"` |
| `kind` | `outcome`, `command`, `ask-shape`, `judge` or `held` | yes | | How the scorer answers | `"outcome"` |
| `expect` | list of strings | for `outcome` and `ask-shape` | | The outcomes that pass, or the parts of an ask's shape every ask must show | `["passed"]` |
| `run` | path | for `command` | | An executable in the factory folder | `"scripts/score-gate.sh"` |
| `timeout` | duration | no | `"20m"` | For `command`: how long it may run before Priory kills it | `"5m"` |
| `model` | string | for `judge` | | The model that grades, as `claude --model` takes it | `"opus"` |
| `prompt` | path | for `judge` | | A file beside the scorer that holds the judge's question | `"does-what-was-asked.md"` |
| `evaluation` | path | for `judge` | | A file beside the scorer that holds its labeled cases, named `*.eval.toml` | `"does-what-was-asked.eval.toml"` |
| `agreement` | number above 0, at most 1 | no | `0.8` | For `judge`: the least share of labeled cases its calibration must get right before its verdict counts | `0.8` |
| `after` | `merged + <n><unit>`, with unit `m`, `h` or `d` | for `held` | | When the daemon grades the task: this long after its last pull request merged | `"merged + 7d"` |
| `fails` | list of `changes-requested`, `red-after-merge`, `reverted`, `marked-wrong`, `fixed` | for `held` | | The signals that fail the task when they happened by the moment `after` names | `["reverted", "fixed"]` |

## Kinds

| `kind` | `on` | Passes when |
| ------ | ---- | ----------- |
| `outcome` | `result` | The result's outcome is one of `expect` |
| `command` | `diff` | `run` exits 0 in a scratch checkout of the result's commit. Exit 1 fails, and any other ending is an error |
| `ask-shape` | `ask` | Every ask the run made shows each part `expect` names |
| `held` | `merge` | None of `fails` happened by the moment `after` names. The first four are [outcome labels](../daemon/trace.md#outcome-labels), judged by when they happened. `fixed` is a later task's `fixes` [relation](../daemon/relations.md) to the task, judged by when it was recorded |
| `judge` | `diff` or `result` | `model` answers `pass` to the question in `prompt`, about the request, the result and, on `diff`, the diff. On `result` it reads no diff and needs no commit, as a plan's critic does. Its verdict counts only once calibrated: see [score.md](../daemon/score.md#judges) |

An ask's shape is the one `priory eval` reads ([prompt-evals.md](prompt-evals.md)): `short` when
its question is one line of at most 25 words, `whatWaits` and `nextAction` when it filled them.

A `result` or `diff` scorer fails a run that ended with an ask: asking is not finishing. An `ask`
scorer answers `not applicable` on a run that asked nothing.

A `command` scorer runs with a cleared environment and these variables:

| Variable | Holds |
| -------- | ----- |
| `PRIORY_SCORE_BASE` | The commit the task started from, its `base_sha` |
| `PRIORY_SCORE_HEAD` | The result's `commit` |
| `PRIORY_SCORE_RESULT` | The path of a JSON file that holds the result |

It answers `error: the result names no commit` for a result without a `commit`, and
`error: no start commit` for a task created before Priory stored its start commit.

A `judge` scorer answers `error: not calibrated: <why>` until `priory score --calibrate` stores
an agreement at or above its `agreement` for its current `model`, prompt and labeled cases. It
needs `priory score --budget`. A file in `scorers/` whose name ends in `.eval.toml` is a judge's
labeled cases, not a scorer.

## Labeled cases

A judge's `evaluation` file, `scorers/<name>.eval.toml`, holds the cases a person labeled. It needs
at least 10 cases, of which at least 3 `pass` and 3 `fail`.

Schema: [`config/judge-cases.json`](../schemas/config/judge-cases.json).

| Key of `[[cases]]` | Type | Required | Meaning |
| ------------------ | ---- | -------- | ------- |
| `name` | string | yes | A name, unique in the file |
| `request` | string | yes | What the task was asked |
| `result` | table | no | The run's result, such as `outcome` and `summary` |
| `diff` | string | no | The change, as `git diff` prints it |
| `verdict` | `pass` or `fail` | yes | The right verdict, which a person gives |
| `why` | string | no | Why the person gave it, for a reader |

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

<!-- example: factory/scorers/held-a-week.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/scorer.json
description = "The change held for a week after its merge: no revert and no later task that fixes it."
on = "merge"
kind = "held"
after = "merged + 7d"
fails = ["reverted", "red-after-merge", "fixed"]
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

<!-- example: factory/scorers/does-what-was-asked.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/scorer.json
description = "The change does what the request asks, and nothing else."
on = "diff"
kind = "judge"
model = "opus"                                  # not the model of an agent it grades
prompt = "does-what-was-asked.md"               # the judge's question, beside this file
evaluation = "does-what-was-asked.eval.toml"    # labeled cases, beside this file
agreement = 0.8                                 # the least share of labeled cases it must get right
```

<!-- example: factory/scorers/does-what-was-asked.md -->
```md
Does the change do what the request asks, and nothing else?

Pass a change that does all the request asks, with tests where the request names behaviour. Fail a
change that leaves part of the request undone, or that also changes code the request does not need.
A result's summary is the agent's claim: check it against the diff.
```

<!-- example: factory/scorers/does-what-was-asked.eval.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/judge-cases.json
[[cases]]
name = "adds-flag"
request = "Add a --quiet flag to the CLI that hides progress lines."
result = { outcome = "passed", summary = "Added --quiet." }
diff = """
+    "--quiet" => quiet = true,
"""
verdict = "pass"

[[cases]]
name = "adds-flag-and-test"
request = "Add a --quiet flag to the CLI that hides progress lines."
result = { outcome = "passed", summary = "Added --quiet with a test." }
diff = """
+    "--quiet" => quiet = true,
+fn quiet_hides_progress() {
"""
verdict = "pass"

[[cases]]
name = "fixes-typo"
request = "Fix the typo \"recieve\" in the README."
result = { outcome = "passed", summary = "Fixed the typo." }
diff = """
-We recieve events.
+We receive events.
"""
verdict = "pass"

[[cases]]
name = "bumps-timeout"
request = "Raise the fetch timeout from 30 to 60 seconds."
result = { outcome = "passed", summary = "Raised it to 60." }
diff = """
-const FETCH: u64 = 30;
+const FETCH: u64 = 60;
"""
verdict = "pass"

[[cases]]
name = "renames-option"
request = "Rename the option `colour` to `color`, keeping `colour` as an alias."
result = { outcome = "passed", summary = "Renamed, with an alias." }
diff = """
-    #[serde(rename = "colour")]
+    #[serde(alias = "colour")]
"""
verdict = "pass"

[[cases]]
name = "flag-and-rewrite"
request = "Add a --quiet flag to the CLI that hides progress lines."
result = { outcome = "passed", summary = "Added --quiet and tidied the parser." }
diff = """
+    "--quiet" => quiet = true,
-fn parse(args: &[String]) {
+fn parse_arguments(input: Vec<String>) {
"""
verdict = "fail"

[[cases]]
name = "half-done"
request = "Add --quiet and --verbose flags to the CLI."
result = { outcome = "passed", summary = "Added both flags." }
diff = """
+    "--quiet" => quiet = true,
"""
verdict = "fail"

[[cases]]
name = "wrong-file"
request = "Fix the typo \"recieve\" in the README."
result = { outcome = "passed", summary = "Fixed the typo." }
diff = """
-We recieve events.
+We receive events.
--- a/CHANGELOG.md
+Fixed typos.
"""
verdict = "fail"

[[cases]]
name = "wrong-value"
request = "Raise the fetch timeout from 30 to 60 seconds."
result = { outcome = "passed", summary = "Raised it to 60." }
diff = """
-const FETCH: u64 = 30;
+const FETCH: u64 = 90;
"""
verdict = "fail"

[[cases]]
name = "drops-alias"
request = "Rename the option `colour` to `color`, keeping `colour` as an alias."
result = { outcome = "passed", summary = "Renamed it." }
diff = """
-    #[serde(rename = "colour")]
+    #[serde(rename = "color")]
"""
verdict = "fail"
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` unknown variant `<kind>`, expected one of `outcome`, `command`, `ask-shape`, `judge` `` | A kind not listed above |
| `` kind `<kind>` reads on = "<on>", not "<other>" `` | An `on` that does not fit the kind |
| `` kind `judge` reads on = "result" or "diff", not "ask" `` | A `judge` scorer on `ask` |
| `` an `outcome` scorer needs `expect`: the outcomes that pass `` | An `outcome` scorer with no `expect` |
| `` an `ask-shape` scorer needs `expect`: some of short, whatWaits, nextAction `` | An `ask-shape` scorer with no `expect` |
| `` `<part>` is not part of an ask's shape: use short, whatWaits, nextAction `` | An `ask-shape` `expect` that names something else |
| `` a `command` scorer takes no `expect`: its exit code is its answer `` | A `command` scorer with `expect` |
| `` a `command` scorer needs `run`: an executable in the factory folder `` | A `command` scorer with no `run` |
| `` `run` names <path>, which is missing from the factory folder `` | `run` names no file |
| `` `run` names <path>, which is not executable: chmod +x it `` | `run` names a file without its execute bit |
| `` `run` names <path>, which is not in the factory folder `` | `run` is absolute or climbs out with `..` |
| `` `<key>` is for kind `command`, not `<kind>` `` | `run` or `timeout` on another kind |
| `` `<key>` is for kind `judge`, not `<kind>` `` | `model`, `prompt`, `evaluation` or `agreement` on another kind |
| `` a `judge` scorer takes no `expect`: its answer is pass or fail `` | A `judge` scorer with `expect` |
| `` a `judge` scorer needs `model`: the model that grades `` | A `judge` scorer with no `model` |
| `` a `judge` scorer needs `prompt`: a file beside it with the judge's whole prompt `` | A `judge` scorer with no `prompt` |
| `` a `judge` scorer needs `evaluation`: a file beside it with its labeled cases `` | A `judge` scorer with no `evaluation` |
| `` `<key>` names <path>, which is missing from scorers/ `` | `prompt` or `evaluation` names no file |
| `` `<key>` names <path>, which is not beside the scorer `` | `prompt` or `evaluation` is absolute or climbs out with `..` |
| `` `evaluation` <path>: a judge needs at least 10 labeled cases, 3 of them pass and 3 fail: this file has <n> (<p> pass, <f> fail) `` | Too few labeled cases, or too few of one verdict |
| `` `evaluation` <path>: two cases named <name> `` | Two labeled cases share a name |
| `` `agreement` is <x>: give a share above 0 and at most 1, such as 0.8 `` | An `agreement` out of range |

`priory check` also warns, and passes, when a judge's `model` is the model of an agent of the
factory, since a model tends to prefer its own work:
`` warning: scorers/<name>.toml:<line>: the judge's model `<model>` is also the model of agent `<agent>`: a model tends to prefer its own work ``.
| `` unknown field `<key>`, expected one of ... `` | A key the table above does not list |
