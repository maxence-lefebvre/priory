# `priory request`

`priory request` asks for a task by hand, without a trigger. It publishes a
`priory.task.requested.cli` source event, then one `factory.task.requested` for it (see
[task-requests.md](task-requests.md)).

```sh
priory request ./factory --workflow implement --key docs-1 --repo owner/repo "Fix the docs"
```

## Usage

```text
priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>]
               [--repo <owner/name>] [--tldr <text>]
               [--hold <reason> [--until <time>] [--after <task-or-issue>]... [--merge <pr>]...
                               [--if-closed release|drop|ask] [--blockers]]
               [--] <text>
```

| Argument | Meaning |
| -------- | ------- |
| `DIR` | The factory folder; `.` by default |
| `--workflow` | The workflow the task starts. Required |
| `--key` | The task key, without its `cli:` prefix. Required |
| `--band`, `--lane` | The task's band and lane |
| `--repo` | The repository the task works in, `owner/name` |
| `--tldr` | The task's TL;DR, so the summarizer skips it (see [search.md](../daemon/search.md)) |
| `--hold` | Files the task held, with the conditions that follow it (see [`hold`](task-requests.md#hold)). `--after` and `--merge` repeat |
| `text` | The request |

A flag value cannot start with `--`. Put `--` before text that does.

## What it refuses

It loads the factory as `priory check` does, and publishes nothing when it refuses.

| Refused | Message |
| ------- | ------- |
| An unknown workflow | `` unknown workflow `<name>` `` |
| A band the factory lacks | `` band `<band>` is not one of <bands> `` |
| No `--repo`, and no `repo` in `triggers/cli.toml` | `the request names no repo; pass --repo owner/name, or set repo in triggers/cli.toml` |
| A `--repo` that is not a GitHub `owner/name` | `repo "<repo>" is not a GitHub owner/name` |

A run needs a repository, hence the last two rules. Priory clones it under its home, so a `repo`
holds only GitHub's characters: an owner of letters, digits and `-`, a name of letters, digits, `-`,
`_` and `.`. Neither part starts with `-`, and the name is not `.` or `..`. `triggers/cli.toml` is described in
[triggers.md](../reference/triggers.md#task-requests-triggersclitoml-and-hold).

## Events

The fields and examples are in [events.md](../reference/events.md#priorytaskrequestedcli).

| Message | Data |
| ------- | ---- |
| `priory.task.requested.cli` | `text`, `person` (`$USER`, or null), `time` and `repo` |
| `factory.task.requested` | `trigger` `cli`, `source` this event's id, the same key, and `band`, `lane` and `repo` from the flags, null when absent |

The type has no `github.` prefix because no GitHub event produced it. It ends in `.cli` because a
later source, such as Linear, adds its own `priory.task.requested.<source>`. It is a request, not a
repository fact, so it names the person and text, not a GitHub id.

## Keys and ids

| What | Value |
| ---- | ----- |
| Task key and subject | `cli:<key>` |
| Event `time` | when the command ran, equal to `data.time` |
| Event id | the UUIDv5 of `priory.task.requested.cli:{len}:{key}:{text}`, with the `cli:` key and `len` its byte length, so a `:` in the key or text cannot make two requests collide |
| Task request id | the UUIDv5 of `factory.task.requested:cli:{source}` |

Running the same command again, with the same key and text, adds nothing: herdr-bus drops both
messages. Another text under the same key is a new request for the same task.
