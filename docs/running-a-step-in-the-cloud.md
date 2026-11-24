# How to run a step in the cloud

This guide shows how to run one model step of a workflow in a Claude cloud session, so its builds
and tests run on Anthropic's machines instead of your desktop. It assumes a factory that already
runs a workflow with an `open-pr` step on a GitHub repository. [cloud.md](daemon/cloud.md) explains
how a cloud run works and what it gives up.

## Connect Claude to the repository

1. Sign in to Claude Code with your Claude account: `claude auth login`. A cloud session needs a
   Claude subscription, not an API key.
2. Give cloud sessions access to the repository. Install the Claude GitHub App on it from
   claude.ai/code, or run `/web-setup` in Claude Code to use your `gh` login.
3. Protect the repository's default branch, so a session cannot push to it. A session can push any
   other branch.
4. Check it by hand from your clone of the repository:

   ```sh
   claude --cloud "Push an empty commit to the branch claude/access-check. Open no pull request."
   ```

   It prints `Created cloud session` and a `View:` link. Wait until
   `git ls-remote origin claude/access-check` prints a commit. A session with no access to the
   repository still starts, from an uploaded copy with no `origin`, and its push fails. Then delete
   the branch, and archive the session on claude.ai/code.

## Give cloud sessions your instructions and skills

1. If the repository needs tools that a cloud session lacks, add a script to it that installs
   them, such as `scripts/cloud-setup.sh`, and name it in `factory.toml`:

   ```toml
   [repos."owner/name"]
   cloud_setup = "scripts/cloud-setup.sh"
   ```

   Keep secrets out of it: anyone who uses the environment can read it.
2. Print the setup script for the agent that runs in the cloud:

   ```sh
   priory cloud setup-script --agent implementer > cloud-setup.sh
   ```

   It writes the factory's instructions and that agent's skills under `~/.claude`, then runs each
   `cloud_setup` script. It refuses a script over 64 KB.
3. On claude.ai/code, add an environment, and paste the script into its setup script.
4. Make it your default environment: run `/remote-env` in Claude Code and pick it.

Print and paste the script again after the kit or the factory's skills change.

## Mark the step's agent

1. Give the step its own agent, with the cloud harness and no local settings:

   ```markdown
   +++
   description = "Implements one task in a cloud session."
   harness = "claude-cloud"
   +++
   You implement one issue.
   ```

2. Route the step's two outcomes, and keep the human step that approves `open-pr`:

   ```toml
   [[steps]]
   id = "implement"
   decides = "model"
   agent = "cloud-implementer"
   prompt = "Implement the issue and run the tests."
   effect = "reversible"
   timeout = "3h"
   next = { pushed = "approve-pr", failed = "stop" }
   ```

3. Let Priory trust the task's worktree for Claude, since `claude --cloud` shows Claude's trust
   dialog in a folder it does not trust. Set `trust_worktrees = true` in `factory.toml`, and name
   the repository in a trigger or in `[repos]`.
4. Run `priory check`. It names any key a cloud agent or step cannot take.

## See the run

- `priory runs` shows the cloud session's id in the `session` column. Its page is
  `https://claude.ai/code/<id>`.
- When the session pushes, the task moves to `approve-pr`, and its result holds the commit.
- A step that ends `failed` says why in its summary, with the session's link.
- Its tokens show as unknown in `priory costs`. Your Claude plan's usage page shows what it used.
