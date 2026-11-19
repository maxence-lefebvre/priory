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
| Session id (R5.5) | `--session-id <UUID>` Priory chooses | the thread from the `SessionStart` hook's `session_id`, stored on the run; `priory runs` shows it |
| Resume | `--resume <session>` | `codex resume <thread>` |
| First prompt | pasted after `SessionStart` | the last argument, `-- <prompt>` |
| Idle signal for the outbox (R5.4) | `SessionStart`, `UserPromptSubmit`, `Stop` hooks in `--settings` | the same three hooks, as `-c hooks.<Event>`; Codex hooks take Claude's stdin and output shapes |
| Stop signal for health (R7.1) | `Stop` hook, and the pane for an exit | the same |
| A prompt shown anyway | `Notification` hook with `permission_prompt` | no equivalent: Codex has no notification hook for an approval prompt. A prompt that shows despite the deny hook stalls the turn until its step `timeout` blocks the task |
| Permission policy (R7.2) | `--permission-mode`, `allow` and `deny` rules, `PermissionRequest` deny hook | `sandbox_mode`, `approval_policy` and `approvals_reviewer` by `defaultMode`, below, and the same `PermissionRequest` deny hook |
| Built-in question tool (R6.4) | `PreToolUse` deny on `AskUserQuestion` | `PreToolUse` deny on `request_user_input`, which Codex offers only in plan collaboration mode, which Priory never sets |
| Model, effort, agent body | `--model`, `--effort`, `--append-system-prompt` | `-c model`, `-c model_reasoning_effort`, `-c developer_instructions` |

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
trust a folder, as Claude does. Priory answers neither and never skips the review. Hooks are
identical in every run, since each run's session reaches them as `PRIORY_SESSION`, so a person
trusts them once, with the clone. A Codex run that sends no `SessionStart` within 60 seconds blocks
its task with `codex showed no SessionStart within 60s; trust the clone and Priory's hooks once: cd
<clone path> && codex -c 'hooks.SessionStart=...' ..., accept the folder, type /hooks and trust
Priory's hooks, then exit`, then the pane's last lines as under Trusting a clone. The command
carries Priory's hooks exactly as runs get them. A hook nobody trusted shows Codex's `Hooks need
review` dialog, which those lines show.

The hooks and the MCP server name Priory by `PRIORY_EXE` if set, else by the absolute path it was
invoked by, else by its resolved binary. Codex keys hook trust on that command, so the path must
survive upgrades. Under Nix, use the profile link, such as `~/.nix-profile/bin/priory`, or set
`PRIORY_EXE` to it, not a store path that changes on every deploy. A changed path changes the hooks, so a person trusts them again.

Known limits:

- No test runs a real Codex. Whether `/hooks` lists and keeps trust for `-c` hooks, and whether
  `auto_review` runs before the deny hook, come from Codex's documentation, unchecked. That
  `SessionStart` comes with the first turn was seen with Codex 0.170.
- Priory invoked by a bare name, as from `PATH`, names its resolved binary; set `PRIORY_EXE` then.
- An agent whose harness changes mid-task starts a new session, since neither harness resumes the
  other's.
- Codex keeps its transcripts under `~/.codex/sessions`, by thread. The traces slice reads token use
  from there; Priory reads nothing from them yet.
