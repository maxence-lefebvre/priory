# `factory.toml`

`factory.toml` names the factory and holds the daemon's settings. Every factory needs one. It can
be `factory.yaml` or `factory.yml` instead, with the same keys.

Schema: [`config/factory.json`](../schemas/config/factory.json).

## Top-level keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `name` | string | yes | | The factory's name | `"example"` |
| `bands` | array of strings | no | `["blocker", "now", "today", "later"]` | The bands, highest first. See [runs.md](../daemon/runs.md) | `["urgent", "normal"]` |
| `handler` | string | no | none: no handler runs | The agent whose model reads a stalled run's screen. It needs `harness = "claude-code"`. See [health.md](../daemon/health.md) | `"reviewer"` |
| `stalled` | duration | no | `"10m"` | How long a run goes without a hook event or a pane change before it counts as stalled | `"15m"` |
| `command_timeout` | duration | no | `"5m"` | How long one `git`, `gh` or `tmux` command may run before Priory kills it. See [home.md](../daemon/home.md) | `"20m"` |
| `sweep` | table | no | the built-in worktree sweep alone | How the daemon frees room. See [`[sweep]`](#sweep) | |
| `search` | table | no | keyword search, no summarizer | How tasks are found. See [`[search]`](#search) | |
| `telemetry` | table | no | no export | Where trace records go. See [`[telemetry]`](#telemetry) | |

A duration is a whole number followed by `s`, `m` or `h`, such as `"90s"`, `"15m"` or `"2h"`.

## `[sweep]`

See [sweep.md](../daemon/sweep.md) for what the sweep removes and when the sweeper starts.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `every` | duration | no | `"10m"` | How often the built-in worktree sweep runs | `"30m"` |
| `scripts` | array of tables | no | `[]` | Scripts run when the resource they name stalls the queue | see below |
| `after` | duration | no | `"30m"` | How long the queue stays stalled after its scripts ran before the sweeper starts | `"1h"` |
| `agent` | string | no | none: no sweeper starts | The agent of the built-in `sweep` workflow | `"implementer"` |
| `repo` | string | no | `"maxence-lefebvre/priory"` | The repository the sweeper works in and files its issue on, `owner/name` | `"owner/name"` |

Each entry of `scripts`:

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `run` | string | yes | | A shell command, run by `sh -c` in `PRIORY_HOME` | `"cargo clean"` |
| `when` | `memory`, `disk` or `cpu` | yes | | The resource whose shortage runs it | `"disk"` |
| `timeout` | duration | no | `"5m"` | How long it may run before Priory kills it | `"1m"` |

## `[search]`

See [search.md](../daemon/search.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `tldr` | string | no | none: no summarizer runs | The agent whose model writes a new task's TL;DR. It needs `harness = "claude-code"` | `"reviewer"` |
| `embeddings` | boolean | no | `false` | Whether search compares embeddings as well as keywords | `true` |
| `model` | string | no | `"sentence-transformers/all-MiniLM-L6-v2"` | The embedding model Priory runs in process, a Hugging Face repository | |
| `command` | string | no | | A command that reads text on stdin and prints a JSON array of floats, instead of `model` | `"./embed.sh"` |

## `[telemetry]`

See [trace.md](../daemon/trace.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `otlp` | string | yes | | The OTLP/HTTP base URL. Spans go to its `/v1/traces` | `"http://127.0.0.1:4318"` |

## Example

<!-- example: factory/factory.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/factory.json
name = "example"
bands = ["blocker", "now", "today", "later"]   # the default bands, highest first
handler = "reviewer"                           # reads a stalled run's screen; claude-code only
stalled = "10m"                                # no hook event and no pane change for this long
command_timeout = "5m"                         # the limit on one git, gh or tmux command

[sweep]
every = "10m"                                  # the built-in worktree sweep
after = "30m"                                  # how long the queue stays stalled before the sweeper
agent = "implementer"                          # the sweeper agent
repo = "owner/name"                            # where the sweeper files its issue
scripts = [
  { run = "rm -rf \"$PRIORY_HOME/cache\"", when = "disk", timeout = "5m" },
]

[search]
tldr = "reviewer"                              # writes each new task's TL;DR; claude-code only
embeddings = true
model = "sentence-transformers/all-MiniLM-L6-v2"

[telemetry]
otlp = "http://127.0.0.1:4318"                 # spans go to its /v1/traces
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `missing: a factory needs factory.toml or factory.yaml` | The folder has neither file |
| `` unknown field `<key>`, expected one of ... `` | A key the tables above do not list |
| `"<text>" is not a duration such as 90s, 15m or 2h` | A duration that is not a whole number and `s`, `m` or `h` |
| `` `factory` is also defined in factory.toml `` | Both `factory.toml` and `factory.yaml` exist |
| `` `bands` needs at least one band, each named once `` | `bands` is empty or names a band twice |
| `` the sweeper agent `<name>` does not exist `` | `[sweep] agent` names no folder under `agents/` |
| `` the handler agent `<name>` does not exist `` | `handler` names no agent |
| `` the handler agent `<name>` needs harness = "claude-code" `` | `handler` names a Codex agent |
| `` the summarizer agent `<name>` does not exist `` | `[search] tldr` names no agent |
| `` the summarizer agent `<name>` needs harness = "claude-code" `` | `[search] tldr` names a Codex agent |
| `[search] names both a model and a command: keep one` | `[search]` sets `model` and `command` |
| `` `[sweep] repo` needs to be `owner/name` `` | `[sweep] repo` is not `owner/name` |

Each line starts with the file and line, such as `factory.toml:3:1:`. A file that does not parse
reports only its parse error, so fix those first.
