# `priory.task.requested.cli`

`priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--hold <reason> [--until <time> | --after <task> | --merge <pr> [--if-closed release|drop|ask] | --blockers]] [--] <text>`
publishes this source event, then one `factory.task.requested` for it. `--hold` files the task held,
with the condition that follows it (see `hold` below). It loads the factory as `priory check` does
and refuses an unknown workflow, publishing nothing. `DIR` defaults to `.`. A flag value cannot start with `--`; put `--` before text that does.

The type has no `github.` prefix because no GitHub event produced it. It ends in `.cli` because a
later source, such as Linear, adds its own `priory.task.requested.<source>`. It is a request, not a
repository fact, so it names the person and text, not a GitHub id.

| Field    | Type           | Meaning                                                    |
| -------- | -------------- | ---------------------------------------------------------- |
| `text`   | string         | The request, as typed                                      |
| `person` | string or null | `$USER`; null when it is unset                             |
| `time`   | RFC 3339       | When the command ran                                       |
| `repo`   | string or null | `--repo`, or null                                          |

The message's `subject` is the task key, `cli:<key>`, and its `time` equals `data.time`. The following
task request has `trigger` `cli`, `source` set to this message's `id`, `band`, `lane` and `repo`
from the flags (null when absent), and the same key. A run needs a repository, so without
`--repo` the request refuses to publish unless `triggers/cli.toml` sets `repo`:
`the request names no repo; pass --repo owner/name, or set repo in triggers/cli.toml`.

```json
{
  "id": "b770d2d3-3057-5e09-bfca-a0bb555bafc8",
  "type": "priory.task.requested.cli",
  "subject": "cli:docs-1",
  "time": "2026-11-17T18:00:00Z",
  "data": {
    "text": "Fix the docs",
    "person": "max",
    "time": "2026-11-17T18:00:00Z",
    "repo": "owner/repo"
  },
  "links": []
}
```

```json
{
  "id": "1d4a502b-b1b3-54da-8165-013fe15a8d3a",
  "type": "factory.task.requested",
  "subject": "cli:docs-1",
  "time": "2026-11-17T18:00:00Z",
  "data": {
    "workflow": "implement",
    "key": "cli:docs-1",
    "band": "today",
    "lane": null,
    "trigger": "cli",
    "source": "b770d2d3-3057-5e09-bfca-a0bb555bafc8",
    "repo": "owner/repo"
  },
  "links": []
}
```

Running the same command again, with the same key and text, adds nothing. The ids come from those
two values (see Message ids), so herdr-bus drops both messages. Another text under the same key is
a new request for the same task.
