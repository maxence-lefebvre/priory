# Priory plugin for Claude Code

This folder is a Claude Code plugin. It lets your own Claude Code session read the Priory desk,
ask you a question through the desk, and add a task. It answers or dismisses an ask only when you
tell it to, only an ask a lead could answer, and Priory records your session, not you, as the
decider.

To install it, follow
[How to use Priory from your own Claude Code session](../../docs/using-priory-from-your-own-session.md).
[docs/adapters/claude-plugin.md](../../docs/adapters/claude-plugin.md) explains what each file does.

- `.claude-plugin/plugin.json`: the manifest. The plugin is named `priory`.
- `.mcp.json`: starts `priory mcp --outside`.
- `hooks/hooks.json`: runs `priory hook PreToolUse --outside` before each Bash call, and
  the `Stop`, `SessionStart` and `UserPromptSubmit` hooks the `compact` tool waits for.
- `skills/`: the `desk`, `ask`, `add` and `compact` skills, also `/priory:desk`, `/priory:ask`,
  `/priory:add` and `/priory:compact`.
- `commands/answer.md`: `/priory:answer ASK`, which only you can run.
- `test/`: `node --test` tests for the files above.
