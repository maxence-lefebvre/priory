# How to run a step in the cloud

This guide shows how to run one model step of a workflow in a Claude cloud session, so its builds
and tests run on Anthropic's machines instead of your desktop. It assumes a factory that already
runs a workflow with an `open-pr` step on a GitHub repository. [cloud.md](daemon/cloud.md) explains
how a cloud run works and what it gives up.

## Connect Claude to the repository

1. Sign in to Claude Code with your Claude account: `claude auth login`. A cloud session needs a
   Claude subscription, not an API key.
2. Install the Claude GitHub App on the repository from claude.ai/code, so a session clones it from
   GitHub. `/web-setup` alone gives a session your `gh` login's access, and Claude Code still uploads
   the clone it starts from: see [Secrets](daemon/cloud.md#secrets) for what that clone holds.
3. Protect the repository's default branch, so a session cannot push to it. A session can push any
   other branch.
4. Check it by hand from a new clone, since without the App the command uploads every branch of the
   clone it runs in:

   ```sh
   git clone --single-branch https://github.com/owner/name.git access-check && cd access-check
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

   `effect` is `reversible` or `irreversible`, since a start pushes the task's branch. On a public
   repository, the commits earlier steps made become public at that push, before a person approves
   `open-pr`. Put a human step before the cloud step if a person must see them first.

3. Let Priory trust the folder `claude --cloud` runs in, `cloud/owner/name` under Priory's home,
   since the command shows Claude's trust dialog in a folder it does not trust. Set
   `trust_worktrees = true` in `factory.toml`, and name the repository in a trigger or in `[repos]`.
4. Make sure no setup recipe leaves untracked files that the repository does not ignore: a worktree
   with changes starts no session.
5. To change how many cloud runs a runner has at once, 4 when unset, set `max` under `[cloud]`.
6. Run `priory check`. It names any key a cloud agent or step cannot take.

## See the run

- `priory runs` shows the cloud session's id in the `session` column. Its page is
  `https://claude.ai/code/<id>`.
- When the session pushes, the task moves to `approve-pr`, and its result holds the commit.
- A step that ends `failed` says why in its summary, with the session's link.
- Its tokens show as unknown in `priory costs`. Your Claude plan's usage page shows what it used.
