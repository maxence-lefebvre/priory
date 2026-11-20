# Reference

These pages list every key of the factory's files and every message the intake publishes. Each
config page gives a field table, a complete example and the errors `priory check` reports.

| Page | Covers |
| ---- | ------ |
| [factory.md](factory.md) | `factory.toml`: the factory's name and the daemon's settings |
| [workflows.md](workflows.md) | `workflows/<name>.toml`: steps, deciders, effects and outcomes |
| [agents.md](agents.md) | `agents/<name>/agent.md`: harness, model, session and permissions |
| [triggers.md](triggers.md) | `triggers/<name>.toml`: one section per source, and `hold` |
| [runners.md](runners.md) | `runners/<hostname>.toml`: platform and share of the machine |
| [events.md](events.md) | Every message type on `factory.intake`, with an example payload |

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
| `agents/*/agent.md` | `config/agent.json`, for tools that read frontmatter; editors do not |

A schema checks each file alone. Rules across files, such as a trigger naming a workflow that
exists, need `priory check`. Patterns for durations, sizes and shares accept the usual forms, such
as `15m`, `6GB` and `60%`. The loader also takes rare number forms such as `1e2%` and `+1s`, and
refuses extremes a pattern cannot see, such as a size under one byte.
