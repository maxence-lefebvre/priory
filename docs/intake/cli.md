# `priory request` and `priory add`

`priory request` asks for a task by hand, without a trigger. It publishes a
`priory.task.requested.cli` source event, then one `factory.task.requested` for it (see
[task-requests.md](task-requests.md)).

```sh
priory request ./factory --workflow implement --key docs-1 --repo owner/repo "Fix the docs"
```

## Usage

```text
priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>]
               [--repo <owner/name>] [--tldr <text>] [--intervention plan|hands-off]
               [--auto-answer high|medium] [--relates <kind>:<task>]...
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
| `--intervention` | Who answers the task's asks, `plan` or `hands-off`; `intervention` in `triggers/cli.toml` when unset (see [triggers.md](../reference/triggers.md#intervention-and-lead)). Another value is a usage error |
| `--auto-answer` | The lowest confidence at which the daemon answers the task's asks with their recommendation, `high` or `medium`; `auto_answer` in `triggers/cli.toml` when unset (see [triggers.md](../reference/triggers.md#auto_answer)). Another value is a usage error |
| `--relates` | How the new task relates to an existing one, such as `builds-on:7`. The kind is `builds-on`, `fixes`, `takes-up` or `relates`. It repeats (see [relations.md](../daemon/relations.md)) |
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
| `--intervention hands-off`, and no `lead` in `triggers/cli.toml` | `--intervention hands-off needs a lead; set lead in triggers/cli.toml` |
| `--auto-answer low` | `--auto-answer low would answer every ask an agent makes: use high or medium` |

A run needs a repository, hence the last two rules. Priory clones it under its home, so a `repo`
holds only GitHub's characters: an owner of letters, digits and `-`, a name of letters, digits, `-`,
`_` and `.`. Neither part starts with `-`, and the name is not `.` or `..`. `triggers/cli.toml` is described in
[triggers.md](../reference/triggers.md#task-requests-triggersclitoml-and-hold).

## `priory add`

`priory add` is the quick form of `priory request`. It takes the workflow and band from the
factory's [`[request]`](../reference/factory.md#request) table, makes up the key, and prints it.
`POST /api/request` does the same (see [serve.md](../reference/serve.md#writes)), and so do the
web view's Add control and the Add action of the Herdr Desk and Queue panels.

```sh
priory add ./factory "Fix the docs"
# cli:2026-11-20-1
priory add ./factory "Review https://github.com/owner/repo/pull/42"
# github:owner/repo#42
```

```text
priory add [DIR] [--workflow <name>] [--band <band>] [--lane <lane>] [--repo <owner/name>]
           [--intervention plan|hands-off] [--auto-answer high|medium] [--relates <kind>:<task>]...
           [--] <text>
```

| Field | From, first that is set |
| ----- | ----------------------- |
| workflow | `--workflow`, then `[request] workflow` |
| band | `--band`, then `[request] band` |
| lane | `--lane` |
| intervention | `--intervention`, then `intervention` in `triggers/cli.toml` |
| auto-answer level | `--auto-answer`, then `auto_answer` in `triggers/cli.toml`. `POST /api/request` takes it as `auto_answer` |
| relations | `--relates`, as for `priory request`. `POST /api/request` takes them as `relates`, a list of `{ "kind", "task" }` |
| repo | `--repo`, then the repository the linked issues agree on, then `repo` in `triggers/cli.toml`. A GitHub URL's repository is in the URL. A Linear issue's is the `repo` of each Linear trigger whose `filter.team` is the issue's team key, such as `ENG`. Links that name two repositories set none |
| key | the issue or pull request when the text links to exactly one, as `github:owner/name#42` or `linear:ENG-7`; else `cli:<YYYY-MM-DD>-<n>` |

A GitHub URL counts when it names an issue or pull request, such as
`https://github.com/owner/name/issues/42`. A Linear URL counts when it names an issue, such as
`https://linear.app/acme/issue/ENG-7/title`. An issue key joins the task that already has it, if
one is active. That task hears the text as [`priory say`](../daemon/tools.md) sends it: an asleep
task at a model step wakes, and a running one reads it after its turn. A task that waits on a
step's ask shows the text on that ask, and its next agent run reads it. A task whose agent has not
started yet keeps the text, and reads it in its first prompt. The joined task keeps its own
workflow, band and lane: `--workflow`, `--band` and `--lane` apply only to a new task.
`POST /api/request` says which of these happened in its reply's `outcome`.

The date is today's in the machine's time zone, and `n` is the first number no other `priory add`
took that day. Two adds that run at once get different keys: the key alone names the source event,
and the first one published owns it. A `priory request --key` or a report pick under the same
`cli:` key is not counted, so avoid keys of that form there.

It refuses what `priory request` refuses, and also:

| Refused | Message |
| ------- | ------- |
| Neither `--workflow` nor `[request]` | `the request names no workflow; pass --workflow, or set workflow in factory.toml's [request] table` |
| The same text again for the same linked issue | `<key> already got this exact request, so nothing was added; reword the text to send it again` |

The second refusal holds after the task ended too: the bus keeps the first request's id.

## `priory batch add`

`priory batch add` publishes one request for a batch's planning task, on the factory's
[`[batch]`](../reference/factory.md#batch) workflow, and prints its key. The request carries the
issues as task keys, and the workflow each entry's task will run. See
[batches.md](../daemon/batches.md) for what the planning task does with them.

```sh
priory batch add ./factory ENG-40 ENG-41 owner/repo#42 --repo owner/repo
# batch:2026-11-21-1
```

```text
priory batch add [DIR] ISSUE... [--repo <owner/name>] [--entries <workflow>] [--band <band>]
                 [--lane <lane>]
```

`DIR` counts as the factory folder only when it is a folder; else it is the first issue, and the
folder is `.`. An `ISSUE` is `ENG-7`, `owner/name#42`, or a GitHub or Linear URL as `priory add`
reads it. The same issue twice counts once.

| Field | From, first that is set |
| ----- | ----------------------- |
| workflow | `[batch] workflow` |
| entries' workflow | `--entries`, then `[batch] entries`, then `[request] workflow` |
| band, lane | `--band`, `--lane` |
| repo | `--repo`, then the repository the issues agree on, then `repo` in `triggers/cli.toml` |
| key | `batch:<YYYY-MM-DD>-<n>`, numbered as `priory add` numbers `cli:` keys |

It refuses what `priory add` refuses, and also:

| Refused | Message |
| ------- | ------- |
| No `[batch]` table | `factory.toml has no [batch] workflow; set one to plan batches` |
| A factory of several runners | `batches need one runner; this factory has <n>` |
| An issue in no form above | `` `<issue>` is not an issue: give ENG-7, owner/name#42 or a URL `` |
| No entries' workflow | `the batch names no entries workflow; pass --entries, or set entries in factory.toml's [batch] table` |

## Events

The fields and examples are in [events.md](../reference/events.md#priorytaskrequestedcli).

| Message | Data |
| ------- | ---- |
| `priory.task.requested.cli` | `text`, `person` (`$USER`, or null), `time` and `repo` |
| `factory.task.requested` | `trigger` `cli`, `source` this event's id, the same key, and `band`, `lane` and `repo` from the flags, null when absent, and `intervention` and `auto_answer` when a flag sets them. The daemon adds `session` for a task that the `add` tool of [`priory mcp --outside`](../daemon/tools.md#the-outside-server) asked for |

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
| Event id from `priory add`, generated key | the UUIDv5 of `priory.task.requested.cli:add:{key}` |
| Event id from `priory add`, issue key | as `priory request`'s, with the issue key in place of `cli:<key>` |
| Event id from a report pick | the UUIDv5 of `priory.task.requested.cli:pick:{plen}:{pick}:{len}:{key}:{text}`, with `plen` the pick id's byte length (see [reports.md](../daemon/reports.md)) |

Running the same command again, with the same key and text, adds nothing: herdr-bus drops both
messages. Another text under the same key is a new request for the same task.
