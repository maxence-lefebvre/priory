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
| `gate` | string | no | none: no step can name the `gate-passed` proof | The project's check, such as its tests. `priory gate` runs it with `sh -c` in a task's worktree. See Proofs in [tools.md](../daemon/tools.md#proofs) | `"cargo test"` |
| `critic` | string | no | none: a plan no person reads moves on unread | A [scorer](scorers.md) on `result` that grades a plan no person reads: a `red`, `yellow` or `green` outcome that leads to a model or code step, of a task without `intervention = "plan"`. It can only refuse the plan. See Gates in [score.md](../daemon/score.md#gates) | `"plan-is-sound"` |
| `pre_review` | string | no | none: an approval ask reaches the desk unreviewed | A judge [scorer](scorers.md) that reads the lessons kept for an approval step before that step asks a person. A result that repeats one goes back to the agent once, with the lessons. It never approves anything. See Pre-review in [score.md](../daemon/score.md#pre-review) | `"repeats-a-lesson"` |
| `writing` | path | no | none: agents get only their own prompt | A file, relative to the factory folder, whose text Priory appends to the system prompt of every agent it starts. `priory check` refuses a missing file and warns past 4096 bytes. See [writing.md](../daemon/writing.md) | `"WRITING.md"` |
| `sweep` | table | no | the built-in worktree sweep alone | How the daemon frees room. See [`[sweep]`](#sweep) | |
| `search` | table | no | keyword search, no summarizer | How tasks are found. See [`[search]`](#search) | |
| `log` | table | no | herdr-bus's file on this machine | Where the event log lives. See [`[log]`](#log) | |
| `telemetry` | table | no | no export | Where trace records go. See [`[telemetry]`](#telemetry) | |
| `notify` | table | no | no desktop notification | Notifications besides Web Push. See [`[notify]`](#notify) | |
| `serve` | table | no | nothing listens | The HTTP server over the factory's state. See [`[serve]`](#serve) | |
| `webhooks` | table | no | no webhook is taken | The signed webhooks the server takes. Needs `[serve]`. See [`[webhooks.github]`](#webhooksgithub), [`[webhooks.linear]`](#webhookslinear), [`[webhooks.sentry]`](#webhookssentry), [`[webhooks.honeycomb]`](#webhookshoneycomb), [`[webhooks.intercom]`](#webhooksintercom), [`[webhooks.cloudwatch]`](#webhookscloudwatch) and [`[webhooks.figma]`](#webhooksfigma) | |
| `slack` | table | no | no Slack shortcut | The Slack message shortcut. See [`[slack.shortcut]`](#slackshortcut) | |
| `asks` | table | no | answers are only stored | What happens to the answer of an ask from outside Priory. See [`[asks]`](#asks) | |
| `links` | table of strings | no | the built-in `#n` link | Autolinks, each a regex and a URL template. See [`[links]`](#links) | |
| `request` | table | no | `priory add` needs `--workflow` | The defaults of `priory add` and `POST /api/request`. See [`[request]`](#request) | |
| `batch` | table | no | `priory batch add` is refused | How `priory batch add` plans batches. See [`[batch]`](#batch) | |
| `caps` | table | no | no cap | Spend caps in dollars for every task and each lane. See [`[caps]`](#caps) | |
| `decisions` | table | no | no decision model | A decision model for each decision point. See [`[decisions.triage]`](#decisionstriage) | |
| `consult` | table | no | no agent consults | Who may consult which investigator, and the bounds of one consult. See [`[consult]`](#consult) | |
| `mcp` | table of tables | no | | A person's statement about each MCP server an investigator names. See [`[mcp.<name>]`](#mcpname) | |
| `guard` | table | no | the built-in guards alone | Irreversible commands and MCP tools the tool guard denies on top of the built-in ones. See [`[guard]`](#guard) | |
| `repos` | table of tables | no | no recipe runs | The setup, resume and teardown recipes of each repository. See [`[repos]`](#repos) | |

A duration is a whole number followed by `s`, `m` or `h`, such as `"90s"`, `"15m"` or `"2h"`.

## `[queue]`

See [runs.md](../daemon/runs.md#overlapping-files) for how the overlap hold works,
[room.md](../daemon/room.md#urgent-wakes) for how `preempt` stops a run, and
[runs.md](../daemon/runs.md#review-limit) for how `review_limit` holds new starts.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `overlap` | boolean | no | `true` | Whether a task waits for an earlier active task whose branch changes the same files | `false` |
| `preempt` | list of bands | no | `[]` | The bands whose task at the head of the queue, waiting for memory or disk, may stop runs of a lower band at the end of their turn | `["blocker"]` |
| `review_limit` | integer, at least 1 | no | no limit | How many pull requests of a lane, or of a batch, may wait for a person before its new tasks wait to start | `5` |

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

## `[repos]`

Each key is a GitHub `owner/name`, and its table holds that repository's recipes. Every key is
optional. See [Recipes](../daemon/runs.md#recipes) for when each runs and what a failure does.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `setup` | recipe | no | none | Runs after the daemon adds a task's new worktree, before its agent starts | `{ argv = ["npm", "ci"] }` |
| `resume` | recipe | no | none | Runs after the daemon adds back a worktree that a retirement removed, before the agent starts | `{ argv = ["npm", "ci", "--prefer-offline"] }` |
| `teardown` | recipe | no | none | Runs before the sweep or a retirement removes the worktree | `{ shell = "docker compose down" }` |

Each recipe is a table with exactly one of `argv` and `shell`:

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `argv` | array of strings | one of the two | | The program and its arguments. Priory execs them as they are, with no shell | `["cargo", "fetch"]` |
| `shell` | string | one of the two | | A shell command, run with `sh -c` | `"make deps && make build"` |
| `timeout` | duration | no | `command_timeout` | How long it may run before Priory kills it with its process group | `"15m"` |

```toml
[repos."owner/name"]
setup = { argv = ["npm", "ci"], timeout = "15m" }
resume = { argv = ["npm", "ci", "--prefer-offline"] }
teardown = { shell = "docker compose down --remove-orphans" }
```

## `[kit]`

A kit repository whose skills, instructions and memories every run gets, before the factory's own.
See [skills.md](../daemon/skills.md#a-kit-repository) for when each runner fetches it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `repo` | string | yes | | The repository, as `git fetch` takes it: a URL or a path. A relative path starts at the factory folder | `"git@github.com:owner/abbey.git"` |
| `ref` | string | no | `"HEAD"`: the default branch | The branch, tag or commit to fetch | `"main"` |
| `skills` | path | no | `"skills"` | The skills folder inside the repository | `"skills"` |
| `instructions` | path | no | `"instructions.md"` | The instructions file inside the repository | `"claude/CLAUDE.md"` |
| `memories` | path | no | `"memories"` | The memories folder inside the repository | `"memories"` |
| `every` | duration | no | `"5m"` | How often each runner fetches the ref again | `"10m"` |

A missing folder or file gives nothing. Each path is relative and stays inside the repository.

Without `[memories]`, an `open-memories-pr` code step writes accepted memories to the `memories`
folder, on a branch based on `ref`, and opens a pull request onto it (see
[skills.md](../daemon/skills.md#accepted-memories-move-into-the-kit)).

## `[memories]`

A memories repository whose memories every run gets, after the kit's and before the factory's own.
It keeps memories apart from the kit's skills, so their frequent commits stay out of the skills'
history. See [skills.md](../daemon/skills.md#a-memories-repository).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `repo` | string | yes | | The repository, as `git fetch` takes it: a URL or a path. A relative path starts at the factory folder | `"git@github.com:owner/scriptorium.git"` |
| `ref` | string | no | `"HEAD"`: the default branch | The branch, tag or commit to fetch | `"main"` |
| `path` | path | no | `"memories"` | The memories folder inside the repository | `"facts"` |
| `every` | duration | no | `"5m"` | How often each runner fetches the ref again | `"10m"` |

A missing folder gives nothing. The path is relative and stays inside the repository. The kit's own
`memories` folder still counts.

With `[memories]`, an `open-memories-pr` code step writes accepted memories to `path` in this
repository, on a branch based on `ref`, and opens a pull request onto it, in place of the kit.

## `[search]`

See [search.md](../daemon/search.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `tldr` | string | no | none: no summarizer runs | The agent whose model writes a new task's TL;DR. It needs `harness = "claude-code"` | `"reviewer"` |
| `embeddings` | boolean | no | `false` | Whether search compares embeddings as well as keywords | `true` |
| `model` | string | no | `"sentence-transformers/all-MiniLM-L6-v2"` | The embedding model Priory runs in process, a Hugging Face repository | |
| `command` | string | no | | A command that reads text on stdin and prints a JSON array of floats, instead of `model` | `"./embed.sh"` |

## `[log]`

Where the intake publishes its events and the daemon reads them. See
[routing.md](../daemon/routing.md) and
[ADR 0002](../adr/0002-a-factory-chooses-herdr-bus-or-postgres-for-its-event-log.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `backend` | `"herdr-bus"` or `"postgres"` | no | `"herdr-bus"` | The store. `"herdr-bus"` is the SQLite file `HERDR_BUS_DB` names. `"postgres"` is a database several machines share, and needs a `priory` built with `--features postgres` | `"postgres"` |
| `url_env` | string | no | `"PRIORY_LOG_URL"` | The environment variable that holds the Postgres connection string, a libpq URL such as `postgresql://user:password@host/db`. The file never holds the string itself | `"PRIORY_LOG_URL"` |
| `timeout` | duration | no | `"10s"` | How long one Postgres call waits on the database before the log counts as unreachable. See [runners.md](../daemon/runners.md#a-runner-cut-off-from-the-log) | `"30s"` |

Priory creates the tables `priory_log_messages` and `priory_log_cursors` on first connect. It uses
TLS when the server offers it and the machine's root certificates trust it; add `sslmode=require`
to the URL to refuse a plain connection.

On Postgres, `timeout` bounds the connect, sets the server's `statement_timeout`, and sets
`tcp_user_timeout` on the socket. TCP keepalives probe a quiet connection after 10 seconds, every
10 seconds, 3 times. A call that gets no answer within `timeout`, such as over an ssh tunnel that
hangs, is given up on.

## `[telemetry]`

See [trace.md](../daemon/trace.md) and, for backends, [How to send traces to an OpenTelemetry backend](../sending-traces-to-opentelemetry.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `otlp` | string | yes | | The OTLP/HTTP base URL. Spans go to its `/v1/traces` | `"http://127.0.0.1:4318"` |
| `headers_env` | string | no | no headers | The environment variable that holds the headers each request carries, as `key1=value1,key2=value2` with URL-encoded values. The daemon refuses to start when it is unset, empty or malformed. `priory check` refuses a value that is not a variable name | `"OTEL_EXPORTER_OTLP_HEADERS"` |

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
| `extension_origins` | array of strings | no | `[]` | The browser extensions whose pages may write, each an exact origin: `chrome-extension://` and the extension's 32-letter id, such as the [GitHub extension](../adapters/github.md)'s. They still send the token. `priory check` refuses any other value | `["chrome-extension://ekngedcpijdeijlmffkeehilmafoikkm"]` |
| `terminal` | string | no | `"watch"` | What a token holder may do with a run's terminal. `"watch"` serves its screen at [`/api/runs/{id}/screen`](serve.md#run-screen); `"type"` also lets a token holder take its [keyboard](serve.md#keyboard) and type into it, past the tool guard; `"off"` serves none | `"off"` |
| `redact_env` | array of strings | no | `[]` | More environment variables whose values the screen hides, besides the token and the factory's other secrets. Evidence is also searched for their values ([evidence.md](../daemon/evidence.md#never-secrets)) | `["OPENAI_API_KEY"]` |
| `url` | string | no | none | The address a person opens the server at, `http://` or `https://` and a host. The Evidence section of a pull request body links each item to the task page under it. Without it, the section holds captions alone. `priory check` refuses another shape with `` `[serve] url` "<url>" is not an address: write it as https://host:port `` | `"https://runner.example.ts.net:7340"` |

The screen is redacted on a best effort basis: whoever holds the token may read what an agent's
screen shows. See [serve.md](../daemon/serve.md#a-runs-screen) for what it hides.

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

## `[webhooks.sentry]`

See [sentry.md](../intake/sentry.md) for what a delivery publishes, and
[serve.md](serve.md#post-hookssentry) for the endpoint. With this table, the server takes Sentry
deliveries at `POST /hooks/sentry`. A [`sentry` trigger](triggers.md#sentry) needs it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"SENTRY_CLIENT_SECRET"` | The environment variable that holds the Sentry integration's Client Secret. Never the secret itself | `"PRIORY_SENTRY_SECRET"` |

`priory serve` and `priory daemon` refuse to start while that variable is unset or empty.

## `[webhooks.cloudwatch]`

See [cloudwatch.md](../intake/cloudwatch.md) for what a delivery publishes and how to set up
EventBridge, and [serve.md](serve.md#post-hookscloudwatch) for the endpoint. With this table, the
server takes CloudWatch alarm state changes from an EventBridge API destination at
`POST /hooks/cloudwatch`. A [`cloudwatch` trigger](triggers.md#cloudwatch) needs it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"CLOUDWATCH_WEBHOOK_SECRET"` | The environment variable that holds the value of the EventBridge connection's API key, `X-Priory-Webhook-Token`. Never the value itself | `"PRIORY_CLOUDWATCH_SECRET"` |

`priory serve` and `priory daemon` refuse to start while that variable is unset or empty.

## `[webhooks.honeycomb]`

See [honeycomb.md](../intake/honeycomb.md) for the payload template and what a delivery publishes,
and [serve.md](serve.md#post-hookshoneycomb) for the endpoint. With this table, the server takes
Honeycomb deliveries at `POST /hooks/honeycomb`. A [`honeycomb` trigger](triggers.md#honeycomb)
needs it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"HONEYCOMB_WEBHOOK_SECRET"` | The environment variable that holds the Honeycomb webhook recipient's Shared Secret. Never the secret itself | `"PRIORY_HONEYCOMB_SECRET"` |

`priory serve` and `priory daemon` refuse to start while that variable is unset or empty.

## `[webhooks.figma]`

See [figma.md](../intake/figma.md) for what the passcode proves, how to create the webhook and what
a delivery publishes, and [serve.md](serve.md#post-hooksfigma) for the endpoint. With this table,
the server takes Figma deliveries at `POST /hooks/figma`. A [`figma` trigger](triggers.md#figma)
needs it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"FIGMA_WEBHOOK_PASSCODE"` | The environment variable that holds the passcode the Figma webhook was created with. Never the passcode itself | `"PRIORY_FIGMA_PASSCODE"` |

`priory serve` and `priory daemon` refuse to start while that variable is unset or empty.

## `[webhooks.intercom]`

See [intercom.md](../intake/intercom.md) for what a notification publishes, and
[serve.md](serve.md#post-hooksintercom) for the endpoint. With this table, the server takes Intercom
notifications at `POST /hooks/intercom` and answers `HEAD` there. An
[`intercom` trigger](triggers.md#intercom) needs it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `secret_env` | string | no | `"INTERCOM_CLIENT_SECRET"` | The environment variable that holds the Intercom app's client secret. Never the secret itself | `"PRIORY_INTERCOM_SECRET"` |
| `workspace` | string | yes | | The workspace id whose notifications Priory takes, a notification's `app_id`. Notifications from other workspaces get `202` and publish nothing | `"a86dr8yl"` |

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
and [tools.md](tools.md#commands-an-answer-runs) for the commands an answer runs.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `deliver` | string | no | none: an answer is only stored | The command that delivers an outside ask's answer or dismissal to the session that asked, relative to the factory folder. `priory check` refuses one that is not an executable file | `"adapters/herdr/deliver"` |
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

## `[batch]`

What `priory batch add` starts: the planning workflow, and the workflow of each entry's task. See
[batches.md](../daemon/batches.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `workflow` | string | yes | | The planning task's workflow, by file name. It needs a human step right after a model step: the go | `"plan-batch"` |
| `entries` | string | no | `[request] workflow` | The workflow each entry's task runs | `"implement"` |

`priory check` reports:

| Problem | Message |
| ------- | ------- |
| An unknown planning workflow | `` `[batch] workflow` `<name>` does not exist `` |
| No human step after a model step | `` `[batch] workflow` `<name>` needs a human step after a model step: the go that starts the batch `` |
| An unknown entries' workflow | `` `[batch] entries` `<name>` does not exist `` |

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

## `[caps]`

Spend caps in dollars, priced from `prices.toml`. See [spend.md](../daemon/spend.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `task` | number | no | no cap | Dollars one task may spend. Past it, the task is held with an ask to raise the cap or stop | `5.0` |
| `day` | number | no | no cap | Dollars the factory may spend in a local day. Past it, a running task is held with an ask, and new runs wait | `50.0` |
| `week` | number | no | no cap | Dollars the factory may spend in a week from Monday 00:00 local time, as `day` | `200.0` |
| `lanes` | table of tables | no | no lane cap | Each lane's `task`, `day` and `week` caps, by its `<team>/<theme>` name. A lane's `day` or `week` cap only holds new runs | `{ "core/intake" = { day = 10.0 } }` |

Each cap is a number above zero. A run can overshoot a cap by one turn. A factory with a cap needs
`prices.toml` beside `factory.toml`, which prices each model the transcripts name (see
[trace.md](../daemon/trace.md#costs)):

<!-- example: factory/prices.toml -->
```toml
# Dollars per million tokens of each kind, by the model name the transcript gives.
["claude-sonnet-5-5"]
input = 3.0
output = 15.0
cache_read = 0.3
cache_write = 3.75

["claude-haiku-5-5"]
input = 1.0
output = 5.0
cache_read = 0.1
cache_write = 1.25
```

## `[consult]`

Who may consult which investigator, and the bounds of one consult. See
[tools.md](../daemon/tools.md#consulting-an-investigator) and
[investigators](agents.md#investigators).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `deadline` | duration | no | `"15m"` | How long from the `consult` call to its report, the wait for room included. Past it, the daemon ends the investigator's task, queued or running | `"10m"` |
| `cap` | number | no | `0.5` | Dollars one consult may cost. The daemon prices the investigator's tokens each second and ends its run once they cost this much | `0.25` |
| `per_task` | integer | no | `5` | How many consults one task may make | `3` |
| `allow` | table of arrays | no | nobody consults | Each asking agent's investigators, by the asking agent's name. An agent with no line gets no `consult` tool | `{ implementer = ["sentry"] }` |

```toml
[consult]
deadline = "15m"
cap = 0.50
per_task = 5

[consult.allow]
implementer = ["sentry", "honeycomb"]
triage = ["sentry"]
```

## `[mcp.<name>]`

A person's statement about one MCP server of their own harness config, by the name agents give it
in `mcpServers`. An investigator may name a server only with a `read_only` sentence.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `read_only` | string | no | | Where the service enforces that the server only reads. Priory cannot check it; `priory check --explain` prints it for review | `"The token has the org:read and event:read scopes only."` |

## `[decisions.triage]`

A decision model that sorts a person's feedback on a task's pull request or issue before the
feedback wakes the task. Its answers count only after `priory calibrate triage` measured it on a
person's labeled cases. See [decisions.md](../daemon/decisions.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `run` | string | yes | | The command, run with `sh -c` in the factory's folder. It reads the question as JSON on stdin and prints the answer as JSON | `"python3 jev.py"` |
| `key_env` | string | yes | | The environment variable that holds the model's key. Never the key itself. The command sees it, `PATH` and `HOME`, and no other variable | `"TYPESAFE_API_KEY"` |
| `evaluation` | string | yes | | The file of labeled cases, relative to the factory's folder. See [Labeled cases](#labeled-triage-cases) | `"triage.eval.toml"` |
| `agreement` | number | no | `0.9` | The least share of labeled cases the model must sort as the person did, from 0.5 to 1 | `0.9` |
| `threshold` | number | no | `0.9` | The least probability of an answer that Priory acts on, from 0.5 to 1 | `0.95` |
| `timeout` | string | no | `"5s"` | How long one call may run, at most `30s` | `"3s"` |
| `daily_calls` | integer | no | `200` | The most calls the daemon makes in a local day | `500` |

### Labeled triage cases

The `evaluation` file holds `[[cases]]`, at least 10, with at least 3 of each class:

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `name` | string | yes | A name, unique in the file | `"thanks"` |
| `kind` | string | no | The event type; `github.pull_request.comment` when unset | `"github.pull_request_review_comment"` |
| `body` | string | yes | What the person wrote | `"Thanks, looks good."` |
| `label` | string | yes | `trivial`, `nothing-to-do` or `persons-call` | `"nothing-to-do"` |
| `why` | string | no | Why the person gave that class, for a reader | |

## Example

<!-- example: factory/factory.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/factory.json
name = "example"
bands = ["blocker", "now", "today", "later"]   # the default bands, highest first
handler = "reviewer"                           # reads a stalled run's screen; claude-code only
stalled = "10m"                                # no hook event and no pane change for this long
command_timeout = "5m"                         # the limit on one git, gh or tmux command
gate = "cargo test"                            # what `priory gate` runs in a worktree

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
headers_env = "OTEL_EXPORTER_OTLP_HEADERS"     # the variable holding the headers, such as a key

[serve]
listen = "127.0.0.1:7340"                      # loopback only, unless public = true
token_env = "PRIORY_TOKEN"                     # the variable holding the token
web = true                                     # false serves only /api

[webhooks.github]
secret_env = "PRIORY_GITHUB_WEBHOOK_SECRET"    # the variable holding the webhook's secret

[webhooks.linear]
secret_env = "PRIORY_LINEAR_WEBHOOK_SECRET"    # the variable holding Linear's signing secret

[webhooks.sentry]
secret_env = "SENTRY_CLIENT_SECRET"            # the variable holding the Sentry integration's Client Secret

[webhooks.cloudwatch]
secret_env = "CLOUDWATCH_WEBHOOK_SECRET"       # the variable holding the EventBridge connection's API key value

[webhooks.honeycomb]
secret_env = "HONEYCOMB_WEBHOOK_SECRET"        # the variable holding the Honeycomb recipient's Shared Secret

[webhooks.figma]
secret_env = "FIGMA_WEBHOOK_PASSCODE"          # the variable holding the Figma webhook's passcode

[webhooks.intercom]
secret_env = "INTERCOM_CLIENT_SECRET"          # the variable holding the Intercom app's client secret
workspace = "a86dr8yl"                         # the one workspace whose notifications it takes

[asks]
deliver = "bin/deliver"                        # delivers outside answers, from the factory folder
targets = { ops = "../ops" }                   # where an answer can run an ask's command

[request]
workflow = "implement"                         # what priory add starts without --workflow
band = "today"                                 # its band without --band

[caps]
task = 5.0                                     # dollars one task may spend
week = 200.0                                   # dollars the factory may spend from Monday

[caps.lanes."core/intake"]
day = 10.0                                     # dollars the lane may spend in a day
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `missing: a factory needs factory.toml or factory.yaml` | The folder has neither file |
| `` `writing` names <file>, which does not exist `` | `writing` points at a file that is not in the factory folder (a file that cannot be read says `cannot be read` and why) |
| `` `gate` is empty `` | `gate` is blank, and `sh -c ""` exits 0, so it would pass every proof |
| `` the critic `<name>` is no scorer in scorers/ `` | `critic` names a scorer the factory lacks |
| `` the critic `<name>` reads on = "<on>": a critic reads a plan, which names no commit, so use one on result `` | `critic` names a scorer on `diff` or `ask` |
| `` the pre-review `<name>` is no scorer in scorers/ `` | `pre_review` names a scorer the factory lacks |
| `` the pre-review `<name>` is of kind <kind>: a pre-review is a judge, which reads the lessons `` | `pre_review` names a scorer that is not a `judge` |
| `` `[guard]` command `<name>` has the name of another guard `` | A `[guard]` command takes a built-in guard's name or another command's |
| `` `[guard]` command `<name>` needs a program in `run` `` | A `[guard]` command with an empty `run` |
| `` unknown field `<key>`, expected one of ... `` | A key the tables above do not list |
| `"<text>" is not a duration such as 90s, 15m or 2h` | A duration that is not a whole number and `s`, `m` or `h` |
| `` `factory` is also defined in factory.toml `` | Both `factory.toml` and `factory.yaml` exist |
| `` `bands` needs at least one band, each named once `` | `bands` is empty or names a band twice |
| `` `preempt` names band `<band>`, which is not one of <bands> `` | `[queue] preempt` names a band the factory lacks |
| `` `preempt` names band `<band>`, the last band, which has no band below it `` | `[queue] preempt` names the last band, which has no lower band to stop |
| ``invalid value: integer `0`, expected a nonzero u32`` | `[queue] review_limit` is 0 |
| `` `[kit]` `<key>` is empty `` | `[kit] repo` or `ref` is blank |
| `` `[kit]` `<key>` is `<value>`: it cannot start with `-` `` | `repo` or `ref` starts with `-`, which git would read as an option |
| `` `[kit]` `<key>` is `<path>`: a kit path is relative and stays inside the repository `` | `skills`, `instructions` or `memories` is empty, absolute, or holds `.` or `..` |
| `` `[memories]` `<key>` is empty `` | `[memories] repo` or `ref` is blank |
| `` `[memories]` `<key>` is `<value>`: it cannot start with `-` `` | `repo` or `ref` starts with `-`, which git would read as an option |
| `` `[memories]` `path` is `<path>`: a memories path is relative and stays inside the repository `` | `path` is empty, absolute, or holds `.` or `..` |
| `` the sweeper agent `<name>` does not exist `` | `[sweep] agent` names no folder under `agents/` |
| `` `[asks] deliver`: <path> is not a file in the factory folder `` | `deliver` names no file under the factory folder |
| `` `[asks] deliver`: <path> is not executable: chmod +x it `` | The `deliver` file has no execute bit |
| `` [log] backend = "postgres" needs a priory built with `--features postgres` `` | `[log] backend = "postgres"` in a build without the Postgres adapter |
| `` `[log] url_env` names the environment variable that holds the connection string, such as PRIORY_LOG_URL, never the string itself `` | `url_env` is not a variable name, such as a pasted URL |
| `` `headers_env` names the environment variable that holds the headers, such as OTEL_EXPORTER_OTLP_HEADERS, never the headers themselves `` | `[telemetry] headers_env` is not a variable name, such as pasted headers |
| `` the handler agent `<name>` does not exist `` | `handler` names no agent |
| `` the handler agent `<name>` needs harness = "claude-code" `` | `handler` names a Codex agent |
| `` the summarizer agent `<name>` does not exist `` | `[search] tldr` names no agent |
| `` the summarizer agent `<name>` needs harness = "claude-code" `` | `[search] tldr` names a Codex agent |
| `[search] names both a model and a command: keep one` | `[search]` sets `model` and `command` |
| `` `[repos]` key `<key>` needs to be `owner/name` `` | A `[repos]` key that is not a GitHub `owner/name` |
| `` `[repos."<repo>"] <recipe>` needs one of `argv` or `shell` `` | A recipe with neither key, or with both (`, not both`) |
| `` `[repos."<repo>"] <recipe>` needs a program in `argv` `` | An empty `argv`, or an empty program |
| `` `[repos."<repo>"] <recipe>` needs a command in `shell` `` | A blank `shell` |
| `` `[repos."<repo>"] <recipe>` runs `<command>`, which the guard `<guard>` holds irreversible: a recipe runs with no person's approval `` | A recipe that runs a command the [tool guard](../daemon/guard.md) denies, such as `gh pr merge` |
| `` `[sweep] repo` needs to be `owner/name` `` | `[sweep] repo` is not a GitHub `owner/name`, such as a path or a name of `..` |
| `` `[serve] listen` <address> is beyond this machine: set `public = true` to serve it `` | `listen` is not a loopback address and `public` is not set |
| `` `[serve] extension_origins` <origin> is not a Chromium extension's origin, `chrome-extension://` and its 32-letter id `` | An entry of `extension_origins` that is not `chrome-extension://` and 32 letters |
| `` `[webhooks]` needs a `[serve]` table to listen on `` | `[webhooks]` is set without `[serve]` |
| `` `[slack.shortcut] users` is empty, so nobody could use the shortcut `` | `users = []` |
| `` `[slack.shortcut] users` `<id>` is not a Slack user id such as U0123ABCD `` | A `users` entry that is not `U` or `W` then capital letters and digits, such as a display name |
| `` `[links]` `<regex>` is not a regex: <reason> `` | A `[links]` key the regex crate refuses |
| `` `[links]` `<regex>` names group $<n>, but the regex has <m> groups `` | A template names a group past the regex's last one |
| `` `[links]` `<regex>` links to `<url>`: a link starts with `https://` `` | A template that is not an `https://` URL |
| `` `[links]` `<regex>` links to `<url>`: a link's host is fixed, with no group or `{repo}` in it `` | A template whose host is empty or holds anything but letters, digits, `.`, `-` and a `:port`, such as `$1` or `{repo}`, so the linked text could pick the site |
| `` `[request] workflow` `<name>` does not exist `` | `[request] workflow` names no file under `workflows/` |
| `` `[request] band` `<band>` is not one of <bands> `` | `[request] band` names a band the factory lacks |
| `<n> is not a spend cap: give dollars above zero` | A `[caps]` value of zero or less, or not finite |
| `` `[caps.lanes]` `<lane>` is not a lane such as `team/theme` `` | A `[caps.lanes]` key that is not `<team>/<theme>` |
| `` `[decisions.triage] key_env` names the environment variable that holds the model's key, such as TYPESAFE_API_KEY, never the key itself `` | `key_env` is not a variable name, such as a pasted key |
| `` `[decisions.triage] <key>` is <n>: give a share from 0.5 to 1, such as 0.9 `` | `agreement` or `threshold` is under 0.5 or over 1 |
| `` `[decisions.triage] timeout` is <n>s: give at most 30s `` | `timeout` is over 30 seconds |
| `` `[decisions.triage] daily_calls` is 0: give at least 1 `` | `daily_calls = 0` |
| `` `[decisions.triage] evaluation` <file>: <why> `` | The labeled cases cannot be read, or hold fewer than 10 cases or fewer than 3 of a class |
| `` `[caps]` needs prices.toml, which turns the steps' tokens into dollars `` | `[caps]` in a factory without `prices.toml` |
| `<rate> is not a price: give dollars per million tokens, zero or more` | A price in `prices.toml` that is negative or not a finite number |
| `` `[consult.allow]` names `<name>`, which is no agent `` | An asking agent with no folder under `agents/` |
| `` `[consult.allow]` names investigator `<name>` as an asker: an investigator cannot consult, so a consult is one hop `` | An investigator as a key of `[consult.allow]` |
| `` `[consult.allow]` lets `<asker>` consult `<name>`, which is no agent `` | A target with no folder under `agents/` |
| `` `[consult.allow]` lets `<asker>` consult `<name>`, which is no investigator: set `investigator = true` on it `` | A target that is no investigator |
| `invalid socket address syntax` | `listen` is not an IP address and port, such as `127.0.0.1:7340` |

Each line starts with the file and line, such as `factory.toml:3:1:`. A file that does not parse
reports only its parse error, so fix those first.

In a factory with a spend cap and `prices.toml`, `priory check` also warns, and passes, for the
agent of a model step whose spend no cap counts:
`` warning: agents/<name>/agent.md:<line>: the spend caps count nothing for agent `<name>`: <why> ``,
where `<why>` is `` prices.toml has no price for its model `<model>` `` or
`Priory reads no Codex tokens`. A Claude Code alias such as `sonnet` never warns: the transcript
names the full model, which `priory check` cannot know.
