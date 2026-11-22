# Reference

These pages list every key of the factory's files and every message the intake publishes. Each
config page gives a field table, a complete example and the errors `priory check` reports.

| Page | Covers |
| ---- | ------ |
| [cli.md](cli.md) | Every `priory` command, with its flags and the page that describes it |
| [factory.md](factory.md) | `factory.toml`: the factory's name and the daemon's settings |
| [workflows.md](workflows.md) | `workflows/<name>.toml`: steps, deciders, effects, outcomes and `priory check --explain` |
| [agents.md](agents.md) | `agents/<name>/agent.md`: harness, model, session and permissions |
| [triggers.md](triggers.md) | `triggers/<name>.toml`: one section per source, and `hold` |
| [runners.md](runners.md) | `runners/<hostname>.toml`: platform and share of the machine |
| [views.md](views.md) | `views/<name>.toml`: the web view's layouts |
| [memories.md](memories.md) | `memories/<name>.md`: the facts every run gets, and the index of them |
| [scorers.md](scorers.md) | `scorers/<name>.toml`: the questions that grade a finished run |
| [benchmarks.md](benchmarks.md) | `benchmarks/<name>.toml`: the cases a comparison replays and its primary scorer |
| [cases.md](cases.md) | A benchmark case folder: `case.toml`, `gh/` and `answers.toml`, and its gaps |
| [serve.md](serve.md) | `priory serve` and the JSON of its endpoints and of the listings' `--json` |
| [events.md](events.md) | Every message type on `factory.intake`, with an example payload |
| [e2e-scenarios.md](e2e-scenarios.md) | The scenario file of the end-to-end factory test |
| [prompt-evals.md](prompt-evals.md) | `priory eval`, its evaluation file and the fake MCP server |
| [label.md](label.md) | `priory label`: a person's `marked-wrong` mark, its flags and errors |
| [trace-schema.md](../trace-schema.md) | Every trace record, with its fields |

The examples are files of a complete factory in [`examples/factory/`](examples/factory/), and the
example payloads are in [`examples/events/`](examples/events/). A test runs `priory check` on that
factory and validates each payload against its schema, so the examples stay correct.

## JSON Schemas

The code generates one JSON Schema per config file in [`docs/schemas/config/`](../schemas/config/)
and one per message type in [`docs/schemas/events/`](../schemas/events/). A test fails when a
committed schema differs from the code. After changing a config or event type, regenerate them:

```sh
PRIORY_UPDATE_SCHEMAS=1 cargo test -p priory --test schemas
```

## Validation in your editor

Point a file at its schema with a first-line comment, and an editor that reads it validates and
completes the keys as you type.

A TOML file, for [Taplo](https://taplo.tamasfe.dev/) or the Even Better TOML extension:

```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
```

A YAML file, for the [YAML language server](https://github.com/redhat-developer/yaml-language-server)
or the Red Hat YAML extension:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
```

| File | Schema |
| ---- | ------ |
| `factory.toml` | `config/factory.json` |
| `workflows/*.toml` | `config/workflow.json` |
| `triggers/*.toml` | `config/trigger.json` |
| `runners/*.toml` | `config/runner.json` |
| `views/*.toml` | `config/view.json` |
| `scorers/*.toml` | `config/scorer.json` |
| `agents/*/agent.md` | `config/agent.json`, for tools that read frontmatter; editors do not |
| `memories/*.md` | `config/memory.json`, for tools that read frontmatter; editors do not |
| `benchmarks/*.toml` | `config/benchmark.json` |
| A case folder's `case.toml`, `gh/<n>.toml` and `answers.toml` | `config/case.json`, `config/case-gh.json`, `config/case-answers.json` |
| A judge scorer's labeled cases | `config/judge-cases.json` |
| A report a workflow writes | `report.json`, beside the two folders (see [reports.md](../daemon/reports.md)) |

A schema checks each file alone. Rules across files, such as a trigger naming a workflow that
exists, need `priory check`. Patterns for durations, sizes and shares accept the usual forms, such
as `15m`, `6GB` and `60%`. The loader also takes rare number forms such as `1e2%` and `+1s`, and
refuses extremes a pattern cannot see, such as a size under one byte.
