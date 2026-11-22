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
| `harness` | `claude-code` or `codex` | yes | | The coding agent CLI that runs it. See [codex.md](../daemon/codex.md) | `"claude-code"` |
| `model` | string | no | the harness's default | The harness's model name | `"sonnet"` |
| `effort` | string | no | the harness's default | The harness's reasoning effort | `"high"` |
| `runner` | string | no | | Reserved for pinning the agent's runs to one runner, by Tailscale hostname, or `any`. `priory check` accepts it and nothing reads it yet: a run starts on the runner whose daemon owns its task | `"any"` |
| `session` | `per-task` or `shared` | no | `"per-task"` | `per-task`: one conversation per task. `shared`: one conversation across tasks, such as a lead. See [sessions.md](../daemon/sessions.md) | `"shared"` |
| `compactAt` | integer | no | | The context size, in tokens, past which the daemon compacts the conversation between turns. Claude Code only. See [sessions.md](../daemon/sessions.md#compacting-a-long-session) | `150000` |
| `idleCompactAt` | integer | no | | The context size, in tokens, past which the daemon compacts a shared session idle for 55 minutes, before its prompt cache expires. Claude Code and `session = "shared"` only. See [sessions.md](../daemon/sessions.md#compacting-an-idle-session) | `60000` |
| `onChange` | `tell` or `fresh` | no | `"tell"` | What a resumed shared session gets when the agent changed since it started. `tell`: a line naming what changed. `fresh`: a new session. `session = "shared"` only. See [sessions.md](../daemon/sessions.md#when-the-agent-changes) | `"fresh"` |
| `memory` | size | no | | What a runner must have free to admit a run. See [memory-gate.md](../daemon/memory-gate.md) | `"6GB"` |
| `disk` | size | no | | Disk a run reserves above the runner's floor | `"20GB"` |
| `mcpServers` | array of strings | no | `[]` | The MCP servers the run gets beside `priory`, and no others, by their names in the person's own harness config. See [mcpServers](#mcpservers) | `["github"]` |
| `connectorSignIn` | boolean | no | `false` | Lets the run call an MCP server's `authenticate` and `complete_authentication` tools. Off by default: an unattended run that signs in clears the stored token every later run needs. See [guard.md](../daemon/guard.md#connector-sign-in) | `true` |
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
| `` a codex agent cannot take `compactAt`: Priory reads no Codex tokens, so it never knows the context size `` | A Codex agent with `compactAt` |
| `` a codex agent cannot take `idleCompactAt`: Priory reads no Codex tokens, so it never knows the context size `` | A Codex agent with `idleCompactAt` |
| `` `idleCompactAt` needs `session = "shared"`: only a shared session is resumed across tasks `` | `idleCompactAt` on a `per-task` agent. The same for `onChange = "fresh"` |
| `invalid value: integer `0`, expected a nonzero u64` | `compactAt = 0` or `idleCompactAt = 0`. A negative or non-integer value gets serde's matching message |

Other files name agents too: a workflow's model steps, and `handler`, `[search] tldr` and
`[sweep] agent` in `factory.toml`. Their pages list those errors.
