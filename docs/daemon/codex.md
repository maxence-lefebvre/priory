# Codex

A run of a `harness = "codex"` agent starts `codex --no-daemon`, or `codex resume <thread>
--no-daemon`, in the same tmux session, worktree and outbox as a Claude run. Codex chooses its own
session id, its thread, so Priory keeps its own UUID as the run's session. The pane's `@priory`
tag, the hooks and the tools carry that UUID. The pane sets `PRIORY_SESSION` to it, and the thread
arrives on `SessionStart` stdin. Every setting goes in as a `-c key=value` override, so Priory writes
no Codex file. `--no-daemon` keeps the run off the background server that a person's other Codex
sessions share, with its environment and cached hook trust.

Codex 0.170 fires `SessionStart` with the first turn, not at startup, so a Codex run cannot wait
for it before its first prompt. The daemon marks the task's oldest queued prompt `attempted` by the
run and passes it as Codex's last argument, after `--`. tmux refuses a command over about 16 KiB, so
the prompt goes through a file under `$PRIORY_HOME/prompts`, which a `sh` wrapper reads, removes and
appends. `UserPromptSubmit` confirms it as it does a paste. Every later prompt, such as an answer or
a nudge, is pasted. A Claude run still pastes its first prompt after `SessionStart`, which Claude
fires at startup, so its paste and confirmation stay as they are.

| Priory duty | Claude Code | Codex |
| ----------- | ----------- | ----- |
| The `priory` MCP server | `--mcp-config` | `-c mcp_servers.priory={command, args, env}`, with `default_tools_approval_mode = "approve"` in place of Claude's `mcp__priory` allow rule |
| The agent's `mcpServers`, and no other server | `--strict-mcp-config`, with the definitions copied into the `--mcp-config` file | the definitions stay in the person's own Codex config. Codex has no strict flag, so every other server of `$CODEX_HOME/config.toml` and the worktree's `.codex/config.toml` gets `-c mcp_servers.<name>.enabled=false` |
| Session id (R5.5) | `--session-id <UUID>` Priory chooses | the thread from the `SessionStart` hook's `session_id`, stored on the run; `priory runs` shows it |
| Resume | `--resume <session>` | `codex resume <thread>` |
| First prompt | pasted after `SessionStart` | the last argument, `-- <prompt>` |
| Idle signal for the outbox (R5.4) | `SessionStart`, `UserPromptSubmit`, `Stop` hooks in `--settings` | the same three hooks, as `-c hooks.<Event>`; Codex hooks take Claude's stdin and output shapes |
| End of a `/compact` | `SessionStart` with `source: compact` | `PostCompact`, since Codex 0.170 fires `SessionStart` with `source: compact` only at the next turn's start |
| Stop signal for health (R7.1) | `Stop` hook, and the pane for an exit | the same |
| Tokens, for traces, `budget`, spend caps and `compactAt` | the transcript's assistant messages | the session file's `token_count` events (see [trace.md](trace.md)) |
| `idleCompactAt` | resumes the session and types `/compact` after `SessionStart` | none: a resumed Codex session sends no `SessionStart` before its first prompt |
| A prompt shown anyway | `Notification` hook with `permission_prompt` | no equivalent: Codex has no notification hook for an approval prompt. A prompt that shows despite the deny hook stalls the turn until its step `timeout` blocks the task |
| Permission policy (R7.2) | `--permission-mode`, `allow` and `deny` rules, `PermissionRequest` deny hook | `sandbox_mode`, `approval_policy` and `approvals_reviewer` by `defaultMode`, below, and the same `PermissionRequest` deny hook |
| Built-in question tool (R6.4) | `PreToolUse` deny on `AskUserQuestion` | `PreToolUse` deny on `request_user_input`, which Codex offers only in plan collaboration mode, which Priory never sets |
| Tool guard | `PreToolUse` hook on `Bash`, `mcp__.*` and the file tools | the same hook on every tool. Codex 0.170 blocks a command or tool call that a `PreToolUse` hook denies; no test runs real Codex, so whether its shell tool and MCP calls reach the hook is unchecked |
| Skills and memories by path | the `PreToolUse` hook adds each file whose `paths` match a file tool's file (see [skills.md](skills.md#skills-and-memories-by-path)) | none yet: Codex has no file tool the hook reads |
| Model, effort, agent body and the factory's writing guide | `--model`, `--effort`, `--append-system-prompt` | `-c model`, `-c model_reasoning_effort`, `-c developer_instructions` |
| The factory's `instructions.md` | before the body, through `--append-system-prompt-file` | before the body, in `developer_instructions`, which `priory check` keeps under 8192 bytes ([skills.md](skills.md)) |
| The factory's skills | `--plugin-dir` | none yet |

`defaultMode` maps to Codex's sandbox and approvals. `approvals_reviewer` is always set, so a user
config that routes approvals to Codex's reviewer does not decide a `manual` run's prompts:

| `defaultMode` | `sandbox_mode` | `approval_policy` | `approvals_reviewer` |
| ------------- | -------------- | ----------------- | -------------------- |
| `manual`, or none | `read-only` | `on-request` | `user`, so the deny hook answers |
| `acceptEdits` | `workspace-write` | `on-request` | `user` |
| `auto` | `workspace-write` | `on-request` | `auto_review`: Codex's reviewer decides, as Claude's auto mode does |
| `dontAsk` | `read-only` | `never` | `user` |

`priory check` refuses what cannot map, with its line: `defaultMode = "plan"` (Codex's CLI has no
plan permission mode) and any `allow` or `deny` rule (Codex has no per-tool rules).

Codex runs only the hooks a person trusted in its `/hooks` review, by hash, and asks whether to
trust a folder, as Claude does. Priory answers neither and never skips the review. A run that
shows the folder dialog blocks its task at once, as a Claude run does. With `trust_worktrees`,
Priory records the folder trust in its own profile instead (see
[Trusting the worktrees and the clone](sessions.md#with-trust_worktrees)). Hooks are
identical in every run, since each run's session reaches them as `PRIORY_SESSION`, so a person
trusts them once, with the clone. A Codex run that sends no `SessionStart` within its window
blocks its task with `codex showed no SessionStart within 60s; trust the clone and Priory's hooks once: cd
<clone path> && codex -c 'hooks.SessionStart=...' ..., accept the folder, type /hooks and trust
Priory's hooks, then exit`, then the pane's last lines as under [Trusting the worktrees and the clone](sessions.md#trusting-the-worktrees-and-the-clone). The command
carries Priory's hooks exactly as runs get them. A hook nobody trusted shows Codex's `Hooks need
review` dialog, which those lines show. The folder trust Priory records does not cover hooks, so a
run with it keeps this reason, after 180 seconds.

The hooks and the MCP server name Priory by `PRIORY_EXE` if set, else by the absolute path it was
invoked by, else by its resolved binary. Codex keys hook trust on that command, so the path must
survive upgrades. Under Nix, use the profile link, such as `~/.nix-profile/bin/priory`, or set
`PRIORY_EXE` to it, not a store path that changes on every deploy. A changed path changes the hooks, so a person trusts them again. So does a new hook, such as `PostCompact`.

Known limits:

- The tool guard (see [guard.md](guard.md)) added a `PreToolUse` hook, so Priory's Codex hooks
  changed. Every Codex clone needs Priory's hooks trusted again, once: a run in an untrusted clone
  blocks its task with the command to run, as above. `priory check --explain` lists the MCP calls
  of a Codex agent with MCP servers as reachable with no human decision, since Codex may run them
  without the hook.
- No test runs a real Codex. Whether `/hooks` lists and keeps trust for `-c` hooks, and whether
  `auto_review` runs before the deny hook, come from Codex's documentation, unchecked. That
  `SessionStart` comes with the first turn was seen with Codex 0.170.
- Priory invoked by a bare name, as from `PATH`, names its resolved binary; set `PRIORY_EXE` then.
- Priory turns off only the MCP servers it reads in `$CODEX_HOME/config.toml` and the worktree's
  `.codex/config.toml`. A server from a Codex profile, or from a config in a folder above the
  worktree, still starts. A name Priory found in the worktree's file starts only if Codex trusts
  that project. Neither the config reading nor `enabled = false` was checked against a real Codex.
- An agent whose harness changes mid-task starts a new session, since neither harness resumes the
  other's.
- Codex keeps its session files under `~/.codex/sessions`, by thread. Priory reads a run's token
  counts from the file its hooks name as `transcript_path` (see [trace.md](trace.md)). Whether
  real Codex names that file is unchecked: no test runs real Codex.
