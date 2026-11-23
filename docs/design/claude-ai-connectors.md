# claude.ai connectors

A person connects an MCP server once in claude.ai, under Settings, Connectors, and it follows their
Claude login to every machine and cloud session. Slack refused the local MCP server a factory used,
so a run needs the person's connectors. Priory starts every Claude Code run with
`--strict-mcp-config`, and under that flag Claude Code loads no connector. This note keeps how a
run gets the connectors its agent names, and no other.

## What Claude Code does

Measured with Claude Code 2.1.330, and read in its code:

1. `claude -p --strict-mcp-config --mcp-config <a file with no servers>` loads no tool named
   `mcp__claude_ai_*`. The code agrees: it fetches the account's connectors only when strict mode is
   off, no managed config holds the servers exclusively, and it is not `--bare`. No flag, setting or
   variable turns them on under strict mode. `disableClaudeAiConnectors` and
   `ENABLE_CLAUDEAI_MCP_SERVERS=false` only turn them off.
2. A connector's server is `claude.ai <display name>`, and a second of the same name gets ` (2)`.
   Its tools are `mcp__claude_ai_<name>__<tool>`, after Claude Code turns each run of other
   characters into one `_`.
3. A permission rule names one server exactly: `mcp__claude_ai_*` matches no server. Only `mcp__*`
   reaches every server, and it would deny Priory's own.
4. `claude mcp get "claude.ai <name>"` exits 0 for a connector of the account and 1 with
   `No MCP server named` for none. The account's list is fetched each time; no file caches it.

## The first slice: `connectors`, its denials and its check

Built. See [agents.md](../reference/agents.md#connectors).

1. An agent key `connectors`, a list of names as claude.ai shows them. It is its own key, not a
   prefix in `mcpServers`: its definition is in the account, not in a file Priory copies, it changes
   how the run starts, and Codex cannot reach it. `priory check` refuses an empty, repeated or
   prefixed name, two names that give one tool prefix, and the key on a Codex agent.
2. A run whose agent names a connector starts without `--strict-mcp-config`. Its `--settings` deny
   each other server Priory can list: the person's user and local scopes, the worktree's `.mcp.json`,
   the machine's `managed-mcp.json` and `claudeAiMcpEverConnected`. A property test checks that the
   denials cover every one of them and none the run names.
3. The tool guard denies a tool of any server the run does not name, from `PRIORY_MCP_SERVERS`, so
   a connector Priory cannot list and a plugin's server are denied too.
4. Before the run starts, the daemon checks each connector with `claude mcp get` in the background,
   and blocks the task when the account lacks it or holds it twice. A found connector counts for 10
   minutes.
5. An investigator names a connector only with a `read_only` sentence under
   `[mcp."claude.ai <name>"]` and an `allow` rule for one of its tools. `connectorSignIn` keeps its
   meaning: an unattended run never signs in.

No schema change.

## Later slices

1. Keep the servers a connector run does not name from starting, once Claude Code has a flag or
   setting that does it. Today they start and only their tools are denied.
2. Read a plugin's servers from the person's enabled plugins, so a deny rule hides their tools from
   the model as well as the guard.
3. Show each connector's last check on the runner's health page.
