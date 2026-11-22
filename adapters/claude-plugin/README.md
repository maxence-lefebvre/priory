# Priory plugin for Claude Code

This folder is a Claude Code plugin. It lets your own Claude Code session read the Priory desk,
ask you a question through the desk, and add a task. It never answers an ask for you.

To install it, follow
[How to use Priory from your own Claude Code session](../../docs/using-priory-from-your-own-session.md).
[docs/adapters/claude-plugin.md](../../docs/adapters/claude-plugin.md) explains what each file does.

- `.claude-plugin/plugin.json`: the manifest. The plugin is named `priory`.
- `.mcp.json`: starts `priory mcp --outside`.
- `hooks/hooks.json`: runs `priory hook PreToolUse --outside` before each Bash call.
- `skills/`: the `desk`, `ask` and `add` skills, also `/priory:desk`, `/priory:ask` and `/priory:add`.
- `commands/answer.md`: `/priory:answer ASK`, which only you can run.
- `test/`: `node --test` tests for the files above.
