# Agents

An agent is who runs a model step: a harness, a model and its permissions. Each agent is a folder
`agents/<name>/` holding `agent.md`. The folder name is the agent's name.

`agent.md` starts with frontmatter, fenced by `+++` for TOML or `---` for YAML. The Markdown after
the closing fence is the agent's instructions.

Schema: [`config/agent.json`](../schemas/config/agent.json). Editors do not validate frontmatter
against it, but `priory check` applies the same rules.

## Frontmatter keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `description` | string | yes | | One line on what the agent does | `"Implements one task."` |
| `harness` | `claude-code`, `codex` or `claude-cloud` | yes | | The coding agent CLI that runs it. `claude-cloud` runs each step in a Claude cloud session. See [codex.md](../daemon/codex.md) and [cloud.md](../daemon/cloud.md) | `"claude-code"` |
| `model` | string | no | the harness's default | The harness's model name | `"sonnet"` |
| `effort` | string | no | the harness's default | The harness's reasoning effort | `"high"` |
| `runner` | string | no | | Reserved for pinning the agent's runs to one runner, by Tailscale hostname, or `any`. `priory check` accepts it and nothing reads it yet: a run starts on the runner whose daemon owns its task | `"any"` |
| `session` | `per-task` or `shared` | no | `"per-task"` | `per-task`: one conversation per task. `shared`: one conversation across tasks, such as a lead. See [sessions.md](../daemon/sessions.md) | `"shared"` |
| `compactAt` | integer | no | | The context size, in tokens, past which the daemon compacts the conversation between turns. See [sessions.md](../daemon/sessions.md#compacting-a-long-session) | `150000` |
| `idleCompactAt` | integer | no | | The context size, in tokens, past which the daemon compacts a shared session idle for 55 minutes, before its prompt cache expires. Claude Code and `session = "shared"` only. See [sessions.md](../daemon/sessions.md#compacting-an-idle-session) | `60000` |
| `onChange` | `tell` or `fresh` | no | `"tell"` | What a resumed shared session gets when the agent changed since it started. `tell`: a line naming what changed. `fresh`: a new session. `session = "shared"` only. See [sessions.md](../daemon/sessions.md#when-the-agent-changes) | `"fresh"` |
| `memory` | size | no | | What a runner must have free to admit a run. See [memory-gate.md](../daemon/memory-gate.md) | `"6GB"` |
| `disk` | size | no | | Disk a run reserves above the runner's floor | `"20GB"` |
| `mcpServers` | array of strings | no | `[]` | The MCP servers the run gets beside `priory`, and no others, by their names in the person's own harness config. See [mcpServers](#mcpservers) | `["github"]` |
| `connectors` | array of strings | no | `[]` | The claude.ai connectors the run gets, by the name claude.ai shows, and no other connector or server of the person's. Claude Code only. See [connectors](#connectors) | `["Linear", "Slack"]` |
| `skills` | array of strings | no | every skill | The skills the run gets, by name, from the factory's `skills/` or the kit's. `[]` gives none. See [skills.md](../daemon/skills.md#skills-per-agent) | `["review-comments"]` |
| `connectorSignIn` | boolean | no | `false` | Lets the run call an MCP server's `authenticate` and `complete_authentication` tools. Off by default: an unattended run that signs in clears the stored token every later run needs. See [guard.md](../daemon/guard.md#connector-sign-in) | `true` |
| `investigator` | boolean | no | `false` | Makes the agent an investigator: a read-only agent that answers other agents' consults, one question each, in a fresh run. See [Investigators](#investigators) | `true` |
| `permissions` | table | no | | Claude Code's own `permissions` settings, so rules copy across. See below | |

A size is a number followed by `KB`, `MB`, `GB` or `TB`, such as `"512MB"` or `"1.5TB"`. Units are
binary, as in `/proc/meminfo`.

### `permissions`

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `defaultMode` | `manual`, `acceptEdits`, `plan`, `auto` or `dontAsk` | no | `manual` | The run's `--permission-mode`. `default` reads as `manual`. `bypassPermissions` is refused | `"auto"` |
| `allow` | array of strings | no | `[]` | Claude Code permission rules the run allows | `["Bash(cargo test:*)"]` |
| `deny` | array of strings | no | `[]` | Claude Code permission rules the run denies | `["Bash(git push:*)"]` |

A Codex agent takes `defaultMode` only, and not `plan`: Codex has no per-tool rules.

A `deny` rule matches the command's text, so it misses wrapped forms such as `gh -R owner/repo pr
merge` or `bash -c '...'`. Priory's tool guard denies irreversible calls whatever their wrapper,
in every mode (see [guard.md](../daemon/guard.md)).

### `mcpServers`

A run gets the `priory` server and the servers its agent names, and no server of the person's
config that its agent leaves out. The daemon finds each name in the person's own config when the
run starts, on the runner that starts it, so a definition and its tokens never enter the factory
or the trace.

| Harness | Where a name is defined | How the run is limited |
| ------- | ----------------------- | ---------------------- |
| Claude Code | `mcpServers` of `$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`: the user scope, as `claude mcp add --scope user` writes it | `--strict-mcp-config`, and the definitions in the `--mcp-config` file. See [runs.md](../daemon/runs.md) |
| Codex | `mcp_servers` of `$CODEX_HOME/config.toml`, else `~/.codex/config.toml`, or of the worktree's `.codex/config.toml` | `-c mcp_servers.<name>.enabled=false` for each other server of those files. See [codex.md](../daemon/codex.md) |

`priory check` does not read these files, since each runner has its own. The daemon refuses the
run instead, and blocks its task with one of the errors below.

| Message | Cause |
| ------- | ----- |
| `agent <agent> names MCP server <name>, which the mcpServers of <path> do not define; add it with claude mcp add --scope user` | A Claude Code agent names a server the person's user scope lacks. A server of the local or project scope does not count |
| `agent <agent> names MCP server <name>, which neither <user config> nor <worktree>/.codex/config.toml defines under mcp_servers` | A Codex agent names a server neither file defines |
| `agent <agent> names MCP server priory, which Priory serves to every run itself; remove it from mcpServers` | An agent names `priory` |
| `agent <agent> cannot read its MCP servers: <path> is not valid JSON at line <n> column <m>` | The Claude Code config does not parse. The message never quotes the file, which can hold tokens |
| `agent <agent> cannot read its MCP servers: <path> is not valid TOML at line <n>` | A Codex config does not parse. A file that cannot be read gives `<path>: <error>` after the colon |

### `connectors`

A claude.ai connector is an MCP server a person connects once in claude.ai, under Settings,
Connectors. It follows their Claude login to every machine, so no runner's config defines it.
Claude Code names its server `claude.ai <name>` and its tools `mcp__claude_ai_<name>__<tool>`, with
each run of characters other than letters, digits, `-` and `_` turned into one `_`. So `Google Drive`
gives `mcp__claude_ai_Google_Drive__<tool>`.

```toml
connectors = ["Linear", "Slack"]
```

A connector is its own key rather than a name in `mcpServers`, for three reasons. Its definition
lives in the person's claude.ai account, not in a config file Priory copies. Claude Code loads no
connector under `--strict-mcp-config`, so a run with one starts differently (see
[runs.md](../daemon/runs.md)). And a Codex run cannot reach one.

Without `--strict-mcp-config`, Claude Code loads every server of the person's config and every
connector of their account. So a run whose agent names a connector also gets:

| Limit | What it covers |
| ----- | -------------- |
| A deny rule `mcp__<server>` in its `--settings` for each server it does not name | The `mcpServers` and each project's `mcpServers` of the person's Claude Code config, the worktree's `.mcp.json`, the machine's `managed-mcp.json`, and the connectors listed under `claudeAiMcpEverConnected` in the config: those that once connected on this machine |
| The tool guard's denial of any other MCP tool (see [guard.md](../daemon/guard.md#servers-a-connector-run-does-not-name)) | A connector Priory cannot list, and a plugin's server |

A server it does not name still starts, since Claude Code has no flag that keeps one out once strict
mode is off. Its tools are denied.

`priory check` refuses a name it can tell is wrong. Whether the person's account has the connector,
only the runner can tell: before the run starts, the daemon runs `claude mcp get "claude.ai <name>"`
for each connector, in the worktree, in the background. The task waits with the reason
`checks its connectors with claude mcp get` meanwhile. A connector found counts as found for 10
minutes, so most runs start with no check. The daemon also asks for `claude.ai <name> (2)`, the name
Claude Code gives a second connector of the same name. It never keeps what `claude mcp get` prints.

| Message | Cause |
| ------- | ----- |
| `agent <agent> names connector <name>, which the claude.ai account that claude signs in with on this runner does not have: connect it in claude.ai under Settings, Connectors, then resume the task` | `claude mcp get` finds no such server |
| `agent <agent> names connector <name>, which the claude.ai account holds twice, as claude.ai <name> and claude.ai <name> (2): rename one in claude.ai, then resume the task` | The account has two connectors of that name |
| `agent <agent> cannot check its connector <name>: claude mcp get "claude.ai <name>" <error>` | `claude mcp get` ended with another status, did not answer within 60 seconds, or could not start |

Each of them blocks the task, and `priory resume` checks again. A connector that needs a sign-in
offers an `authenticate` tool, which the guard denies unless the agent sets `connectorSignIn` (see
[guard.md](../daemon/guard.md#connector-sign-in)).

### Investigators

An investigator answers a `consult` from a run of another agent (see
[tools.md](../daemon/tools.md#consulting-an-investigator)). It only reads, so `priory check`
refuses one that could write:

| Rule | Why |
| ---- | --- |
| `harness = "claude-code"` | Only Claude Code takes per-tool rules |
| `permissions.defaultMode = "dontAsk"` | Claude Code denies every call that would prompt, so the run can call only what `allow` names, and reads in its worktree |
| No `allow` rule names `Edit`, `MultiEdit`, `Write`, `NotebookEdit` or `Bash` | Each of them writes |
| No `allow` rule names `Agent` or `Task` | Each starts a subagent, which can carry its own permission mode |
| No `allow` rule holds a `*` or names a whole MCP server such as `mcp__sentry` | Each MCP tool is written out by its exact name, so a tool the server adds later is denied until a person adds it |
| Each of its `mcpServers` has a `read_only` sentence under [`[mcp.<name>]`](factory.md#mcpname) | The person states where the service enforces that the server only reads. Priory cannot check it |
| Each of its `connectors` has a `read_only` sentence under `[mcp."claude.ai <name>"]`, and an `allow` rule that names one of its tools | The same statement, for a connector. A connector loads beside every other one of the account, so the run may call only the connector tools `allow` names |
| No `session = "shared"` and no `connectorSignIn` | Each consult starts a fresh session, and an unattended run never signs in |
| A `model` that `prices.toml` prices | The daemon prices the run's tokens each second against `[consult] cap` |
| No workflow step and no trigger's `lead` names it | It only answers consults |

```toml
+++
description = "Reads Sentry issues, events and releases, and answers questions about them."
harness = "claude-code"
model = "claude-haiku-5-5"
investigator = true
mcpServers = ["sentry"]
permissions = { defaultMode = "dontAsk", allow = ["mcp__sentry__search_issues", "mcp__sentry__get_issue_details", "Read", "Grep"] }
+++
You read Sentry and answer one question.
```

`priory check --explain` prints each investigator's `allow` rules and each server's `read_only`
sentence, so a person can review both.

The daemon also starts an investigator's `claude` with two flags that no factory file sets:

| Flag | Effect |
| ---- | ------ |
| `--setting-sources ""` | Claude Code loads no user, project or local settings file. Only Priory's `--settings` apply, so an `allow` rule in `~/.claude/settings.json` or in the repository's `.claude/settings.json` never reaches the run. A key the person keeps there, such as `env` or `apiKeyHelper`, does not reach it either |
| `--disallowedTools Edit,MultiEdit,Write,NotebookEdit,Bash,Agent,Task` | Claude Code denies these tools even if a rule allows them |

## Examples

TOML frontmatter:

<!-- example: factory/agents/implementer/agent.md -->
```markdown
+++
description = "Implements one task and opens its pull request."
harness = "claude-code"
model = "sonnet"
effort = "high"
runner = "any"
session = "per-task"           # or "shared": one conversation across tasks
compactAt = 150000             # compact between turns once the context reaches it
memory = "6GB"                 # what a runner must have free to admit a run
disk = "20GB"                  # disk a run reserves above the runner's floor
mcpServers = ["github"]
permissions = { defaultMode = "auto", allow = ["Bash(cargo test:*)"], deny = ["Bash(git push:*)"] }
+++
# Implementation

Read the issue and its acceptance criteria, make the smallest correct change, and test it.
```

YAML frontmatter:

<!-- example: factory/agents/reviewer/agent.md -->
```markdown
---
description: Reads stalled runs and writes task TL;DRs.
harness: claude-code
model: haiku
permissions:
  defaultMode: manual          # bypassPermissions is refused
  allow: ["Read"]
---
# Review

Answer in one short paragraph.
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `frontmatter is not closed by +++` | No closing fence. The same with `---` |
| `` unknown field `<key>`, expected one of ... `` | A key the tables above do not list. Keys are camelCase |
| `` missing field `<key>` `` | No `description` or `harness` |
| `"<text>" is not a size such as 512MB or 6GB` | A bad `memory` or `disk` |
| `` `bypassPermissions` is not allowed: Priory never skips permission checks `` | `defaultMode = "bypassPermissions"` |
| `` a codex agent cannot take `permissions.defaultMode = "plan"`: use manual, acceptEdits, auto or dontAsk `` | A Codex agent in plan mode |
| `` a codex agent cannot take `permissions.allow`: Codex has no per-tool rules, so its sandbox and approvals follow `defaultMode` alone `` | A Codex agent with `allow`. The same for `deny` |
| `` a codex agent cannot take `idleCompactAt`: a resumed Codex session sends no `SessionStart` before its first prompt, so Priory never knows when to type `/compact` `` | A Codex agent with `idleCompactAt` |
| `` a codex agent takes its own prompt on its command line, as `developer_instructions`: it is <n> bytes, over the limit of 8192 `` | A Codex agent's prompt and the writing guide pass 8192 bytes. With `instructions.md`, memories or skills, the message names each part it counts and says `together they are` (see [skills.md](../daemon/skills.md#a-codex-run)) |
| `` `idleCompactAt` needs `session = "shared"`: only a shared session is resumed across tasks `` | `idleCompactAt` on a `per-task` agent. The same for `onChange = "fresh"` |
| `` `skills` names `<name>`, which is not a skill of this factory: <names> `` | A name in `skills` with no folder in `skills/`, in a factory without `[kit]`. With `[kit]`, the kit may hold it, and `priory check` accepts it |
| `` `skills` names `<name>` twice `` | A name repeated in `skills` |
| `` `skills` holds an empty name `` | `""` in `skills` |
| `` a codex agent takes its own prompt on its command line, as `developer_instructions`: it is <n> bytes, over the limit of 8192 `` | A Codex agent whose prompt and writing guide pass 8192 bytes. With instructions, memories or skills the run gets, the message names them too. See [skills.md](../daemon/skills.md#a-codex-run) |
| `` investigator `<name>` needs harness = "claude-code": only Claude Code takes per-tool rules `` | A Codex investigator |
| `` investigator `<name>` needs permissions.defaultMode = "dontAsk", so every tool it may call is named in `allow` `` | An investigator in another mode |
| `` investigator `<name>` may not allow `<rule>`: it names a tool that writes `` | An `allow` rule of `Edit`, `MultiEdit`, `Write`, `NotebookEdit` or `Bash` |
| `` investigator `<name>` may not allow `<rule>`: it starts a subagent, which can carry its own permission mode `` | An `allow` rule of `Agent` or `Task`, such as `Agent(Explore)` |
| `` investigator `<name>` may not allow `<rule>`: it holds a `*`: write out each tool by its exact name `` | A pattern in `allow`, such as `mcp__sentry__get_*` |
| `` investigator `<name>` may not allow `<rule>`: it names a whole MCP server: write out each of its tools by its exact name `` | A rule such as `mcp__sentry` |
| `` investigator `<name>` names MCP server `<server>`, which needs a `read_only` sentence under [mcp.<server>] in factory.toml: where the service enforces that it only reads `` | A server with no `read_only` sentence |
| `` investigator `<name>` cannot take session = "shared": each consult starts a fresh session, so no task's data reaches another's report `` | A shared investigator |
| `` investigator `<name>` cannot take `connectorSignIn`: an unattended investigator never signs in `` | `connectorSignIn = true` on an investigator |
| `` investigator `<name>` names connector `<connector>`, which needs a `read_only` sentence under [mcp."claude.ai <connector>"] in factory.toml: where the service enforces that it only reads `` | A connector with no `read_only` sentence |
| `` investigator `<name>` names connector `<connector>`, but its `allow` names none of the connector's tools: write each as mcp__claude_ai_<connector>__<tool> `` | A connector none of whose tools `allow` names |
| `` `connectors` holds an empty name `` | `""` in `connectors` |
| `` `connectors` names `<name>` twice `` | A name repeated in `connectors` |
| `` `connectors` names `claude.ai <name>`: write the connector's name as claude.ai shows it, `<name>`, since Claude Code adds `claude.ai ` itself `` | A name with Claude Code's prefix |
| `` `connectors` names `<a>` and `<b>`, whose tools Claude Code names alike, mcp__claude_ai_<server>__<tool>: keep one `` | Two names that give one tool prefix, such as `Google Drive` and `Google_Drive` |
| `` a codex agent cannot take `connectors`: claude.ai connectors reach Claude Code alone `` | A Codex agent with `connectors` |
| `` harness = "claude-cloud" cannot take `<key>`: <why> `` | A cloud agent with `model`, `effort`, `session = "shared"`, `mcpServers`, `connectors`, `permissions`, `memory`, `disk` or `compactAt`. See [cloud.md](../daemon/cloud.md) |
| `` investigator `<name>` needs a `model` with a price in prices.toml, so its cap can be enforced `` | No `model`. With a model `prices.toml` lacks: `` runs model `<model>`, which prices.toml does not price, so its cap cannot be enforced `` |
| `invalid value: integer `0`, expected a nonzero u64` | `compactAt = 0` or `idleCompactAt = 0`. A negative or non-integer value gets serde's matching message |

Other files name agents too: a workflow's model steps, a trigger's `lead`, and `handler`,
`[search] tldr`, `[sweep] agent` and `[consult.allow]` in `factory.toml`. Their pages list those
errors. A workflow step or a `lead` that names an investigator gets `` agent `<name>` is an
investigator, which only answers consults ``.
