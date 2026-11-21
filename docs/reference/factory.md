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
| `notify` | table | no | no desktop notification | Notifications besides Web Push. See [`[notify]`](#notify) | |
| `serve` | table | no | nothing listens | The HTTP server over the factory's state. See [`[serve]`](#serve) | |
| `webhooks` | table | no | no webhook is taken | The signed webhooks the server takes. Needs `[serve]`. See [`[webhooks.github]`](#webhooksgithub) and [`[webhooks.linear]`](#webhookslinear) | |
| `slack` | table | no | no Slack shortcut | The Slack message shortcut. See [`[slack.shortcut]`](#slackshortcut) | |
| `asks` | table | no | answers are only stored | What happens to the answer of an ask from outside Priory. See [`[asks]`](#asks) | |
| `links` | table of strings | no | the built-in `#n` link | Autolinks, each a regex and a URL template. See [`[links]`](#links) | |
| `request` | table | no | `priory add` needs `--workflow` | The defaults of `priory add` and `POST /api/request`. See [`[request]`](#request) | |
| `guard` | table | no | the built-in guards alone | Irreversible commands and MCP tools the tool guard denies on top of the built-in ones. See [`[guard]`](#guard) | |

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

## `[notify]`

See [serve.md](../daemon/serve.md#desktop-notifications).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `desktop` | boolean | no | `false` | Whether the daemon runs `notify-send` with the title of each new next item on the desk that is an ask or an alarm | `true` |

## `[serve]`

See [serve.md](../daemon/serve.md) for what the server does, and [serve.md](serve.md) for its
endpoints. With no `[serve]` table, nothing listens.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `listen` | string | no | `"127.0.0.1:7340"` | The IP address and port to listen on. A host name is refused | `"127.0.0.1:8080"` |
| `token_env` | string | no | `"PRIORY_TOKEN"` | The environment variable that holds the token clients send. Never the token itself | `"PRIORY_WEB_TOKEN"` |
| `web` | boolean | no | `true` | Serve the web view's files as well as `/api`. `false` serves only `/api` | `false` |
| `public` | boolean | no | `false` | Allow a `listen` address beyond this machine | `true` |

`public = true` exposes the factory's state to the network the address reaches. Whether a machine
may do that is the factory owner's choice, per machine. The server speaks plain HTTP, with no TLS.

## `[webhooks.github]`

See [webhooks.md](../intake/webhooks.md) for what a delivery publishes, and [serve.md](serve.md#post-hooksgithub)
for the endpoint. With this table, the server takes GitHub deliveries at `POST /hooks/github`.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"PRIORY_GITHUB_WEBHOOK_SECRET"` | The environment variable that holds the webhook's secret. Never the secret itself | `"GITHUB_WEBHOOK_SECRET"` |

`priory serve` and `priory daemon` refuse to start while that variable is unset or empty.

## `[webhooks.linear]`

See [webhooks.md](../intake/webhooks.md#linear) for what a delivery publishes, and
[serve.md](serve.md#post-hookslinear) for the endpoint. With this table, the server takes Linear
deliveries at `POST /hooks/linear`.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"PRIORY_LINEAR_WEBHOOK_SECRET"` | The environment variable that holds the webhook's signing secret. Never the secret itself | `"LINEAR_WEBHOOK_SECRET"` |

`priory serve` and `priory daemon` refuse to start while that variable is unset or empty.

## `[slack.shortcut]`

See [slack.md](../intake/slack.md#message-shortcut) for what the shortcut does and how to set up
its Slack app. With this table, `priory intake` holds a Socket Mode connection to Slack.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `token_env` | string | no | `"SLACK_APP_TOKEN"` | The environment variable that holds the app-level token (`xapp-`). Never the token itself | `"PRIORY_SLACK_APP_TOKEN"` |
| `users` | array of strings | yes | | The Slack user ids allowed to queue a task. Slack's profile menu shows yours under "Copy member ID" | `["U0123ABCD"]` |

`priory intake` refuses to start while that variable or `SLACK_BOT_TOKEN` is unset or empty.

## `[asks]`

See [asks.md](../daemon/asks.md) for when the daemon runs the `deliver` command and how it retries,
and [tools.md](../daemon/tools.md#commands-an-answer-runs) for the commands an answer runs.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `deliver` | string | no | none: an answer is only stored | The command that delivers an outside ask's answer or dismissal to the session that asked, relative to the factory folder | `"adapters/herdr/deliver"` |
| `targets` | table of strings | no | none: only `worktree` | The folders, by name, where an answer can run an agent's ask command besides the task's worktree, relative to the factory folder or absolute | `{ ops = "../ops" }` |

## `[links]`

Autolinks turn keys in text into links when the web view shows it. Each key is a regex, in the
[regex crate's syntax](https://docs.rs/regex/latest/regex/#syntax). Each value is a URL template:
`$1` is what the regex's first group matched, `$0` the whole match, and `{repo}` the task's
repository, `owner/name`. They go in the path or query, never in the host. Priory percent-encodes each group's text before it goes in the URL. The
text Priory stores never changes.

```toml
[links]
'\b([A-Z]{2,5}-\d+)\b' = "https://linear.app/acme/issue/$1"
'\B#(\d+)\b' = "https://github.com/acme/app/issues/$1"
```

| Rule | Detail |
| ---- | ------ |
| Built-in link | `'\B#(\d+)\b' = "https://github.com/{repo}/issues/$1"`: `#42` links to issue or pull request 42 in the task's repository. Text with no repository, such as an outside ask's, gets no link from it |
| Replacing a built-in link | An entry with the built-in link's exact regex replaces it |
| Order | The factory's entries in file order, then each built-in link no entry replaced. Where two regexes match, the match that starts first wins, then the rule listed first |
| Linear | No source names a Linear workspace, so Linear keys get no built-in link. Add the first line of the example above, with your workspace's name in place of `acme`. It links keys such as `ENG-123` |
| Where links show | The web view's ask (its question and its text), its task page (its title, reason, brief and plan), its desk rows, reports included, its task rows, and the keys and text of its Reports panel's rows. The [Herdr adapter](../adapters/herdr.md)'s panels link the same text. `priory links --json` and `GET /api/links` list the rules. CLI text output stays plain |

The web view runs each regex as a JavaScript regex with the `u` flag. A regex that JavaScript
refuses, such as one with an inline flag like `(?i)`, links nothing there; `priory check` does not
catch that. `\d`, `\w` and `\b` match only ASCII in JavaScript and any script in Rust, so write
`[0-9]` or `[A-Za-z]` when it matters. The regex runs in the reader's browser, so keep it simple:
a regex with nested repeats, such as `(a+)+`, can stall the page on long text.

## `[request]`

What `priory add` and `POST /api/request` use when a request names no workflow or band. See
[cli.md](../intake/cli.md#priory-add).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `workflow` | string | yes | | The workflow a request starts when it names none, by file name | `"groom"` |
| `band` | string | no | none, as `priory request` without `--band` | The band of a request that names none | `"today"` |

## `[guard]`

The tool guard denies an irreversible call that no person approved (see
[guard.md](../daemon/guard.md), which lists the built-in guards). `[guard]` adds guards. A workflow or an
irreversible step lets its agent make a guarded call with `guard.allow` (see
[workflows.md](workflows.md#top-level-keys)).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `commands` | array of tables | no | `[]` | Shell commands, each a guard of its own name | see below |
| `tools` | array of strings | no | `[]` | MCP tools, each a guard named as written: a full name, or a prefix ending in `*`. `guard.allow` names the pattern, such as `mcp__stripe__*` | `["mcp__stripe__create_refund", "mcp__pagerduty__*"]` |

Each entry of `commands`:

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `name` | string | yes | | The guard's name, which no other guard has | `"fly-deploy"` |
| `run` | array of strings | yes | | The program, then the first words after its options | `["fly", "deploy"]` |
| `flags` | array of strings | no | `[]` | When set, the command matches only with one of these options | `["--prod"]` |

```toml
[guard]
commands = [{ name = "fly-deploy", run = ["fly", "deploy"] }]
tools = ["mcp__stripe__*"]
```

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

[serve]
listen = "127.0.0.1:7340"                      # loopback only, unless public = true
token_env = "PRIORY_TOKEN"                     # the variable holding the token
web = true                                     # false serves only /api

[webhooks.github]
secret_env = "PRIORY_GITHUB_WEBHOOK_SECRET"    # the variable holding the webhook's secret

[webhooks.linear]
secret_env = "PRIORY_LINEAR_WEBHOOK_SECRET"    # the variable holding Linear's signing secret

[asks]
deliver = "bin/deliver"                        # delivers outside answers, from the factory folder
targets = { ops = "../ops" }                   # where an answer can run an ask's command

[request]
workflow = "implement"                         # what priory add starts without --workflow
band = "today"                                 # its band without --band
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `missing: a factory needs factory.toml or factory.yaml` | The folder has neither file |
| `` `[guard]` command `<name>` has the name of another guard `` | A `[guard]` command takes a built-in guard's name or another command's |
| `` `[guard]` command `<name>` needs a program in `run` `` | A `[guard]` command with an empty `run` |
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
| `` `[sweep] repo` needs to be `owner/name` `` | `[sweep] repo` is not a GitHub `owner/name`, such as a path or a name of `..` |
| `` `[serve] listen` <address> is beyond this machine: set `public = true` to serve it `` | `listen` is not a loopback address and `public` is not set |
| `` `[webhooks]` needs a `[serve]` table to listen on `` | `[webhooks]` is set without `[serve]` |
| `` `[slack.shortcut] users` is empty, so nobody could use the shortcut `` | `users = []` |
| `` `[slack.shortcut] users` `<id>` is not a Slack user id such as U0123ABCD `` | A `users` entry that is not `U` or `W` then capital letters and digits, such as a display name |
| `` `[links]` `<regex>` is not a regex: <reason> `` | A `[links]` key the regex crate refuses |
| `` `[links]` `<regex>` names group $<n>, but the regex has <m> groups `` | A template names a group past the regex's last one |
| `` `[links]` `<regex>` links to `<url>`: a link starts with `https://` `` | A template that is not an `https://` URL |
| `` `[links]` `<regex>` links to `<url>`: a link's host is fixed, with no group or `{repo}` in it `` | A template whose host is empty or holds anything but letters, digits, `.`, `-` and a `:port`, such as `$1` or `{repo}`, so the linked text could pick the site |
| `` `[request] workflow` `<name>` does not exist `` | `[request] workflow` names no file under `workflows/` |
| `` `[request] band` `<band>` is not one of <bands> `` | `[request] band` names a band the factory lacks |
| `invalid socket address syntax` | `listen` is not an IP address and port, such as `127.0.0.1:7340` |

Each line starts with the file and line, such as `factory.toml:3:1:`. A file that does not parse
reports only its parse error, so fix those first.
