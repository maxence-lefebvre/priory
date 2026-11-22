# How to use Priory from your own Claude Code session

This guide shows how to load the Priory plugin into your own Claude Code session, so it can read
your desk, ask you a question through the desk, answer the asks you hand it, and add a task. It
assumes a daemon runs on this
machine, and that `priory` and `claude` are on your `PATH`.
[The Claude Code plugin](adapters/claude-plugin.md) explains what the plugin does and which asks the
model may answer.

1. Export the daemon's home and state in the shell you start Claude Code from. Use the same values
   the daemon runs with:

   ```sh
   export PRIORY_HOME=~/.local/state/priory-oss
   export PRIORY_DB=~/.local/state/priory-oss/priory.db
   ```

   The server refuses to start without them, with
   `priory mcp --outside: set PRIORY_HOME and PRIORY_DB to the daemon's, so the server reaches its factory`.

2. Start Claude Code with the plugin folder: `adapters/claude-plugin` in a checkout, or the folder
   `priory-claude-plugin.tar.gz` unpacks to from a release:

   ```sh
   claude --plugin-dir ~/src/priory/adapters/claude-plugin
   ```

3. Check that the server is connected: type `/mcp`. The list shows `plugin:priory:priory`.

4. Ask what needs you:

   ```text
   /priory:desk
   ```

   The model lists the desk's items, most urgent first.

5. Answer an ask. Type `/priory:answer` with the ask's id:

   ```text
   /priory:answer cache.ship
   ```

   The model shows the ask and prints the line that answers it, such as
   `! priory answer --revision 1 cache.ship ship`. Check the line, then type it yourself. The model
   cannot run it: the plugin's hook denies it.

   To have the session answer for you instead, tell it, such as `answer cache.ship with ship`. The
   model calls the `answer` tool, and Priory records that your session decided, not you. Priory
   refuses an ask that runs a command, approves an irreversible step, or can let one run; the model
   then tells you to answer it in the desk.

6. Hand a follow-up to the factory:

   ```text
   /priory:add fix the typo in CONTRIBUTING.md
   ```

   The model replies with the new task's key and id. The task records that your session added it.

When the model is blocked on your decision, it uses the `ask` skill on its own. Its reply says
whether the answer will come back as a message, which needs an hc terminal and a factory with
[`[asks] deliver`](daemon/asks.md), or whether the model reads it with the `asks` tool once you
answer on the desk.
