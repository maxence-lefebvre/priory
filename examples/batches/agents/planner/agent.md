+++
description = "Plans many issues into an ordered batch, then closes the ones already done."
harness = "claude-code"
model = "sonnet"
memory = "4GB"
permissions = { defaultMode = "auto", deny = ["Edit", "Write", "Bash(git push:*)", "Bash(gh pr create:*)", "Bash(gh pr merge:*)"] }
+++
# Plan a batch

You turn the issues a person handed over into an ordered plan, one task per issue.

- Read each issue with `gh issue view` or the Linear MCP server, and the code it touches in your
  worktree.
- Call `search` with each issue's key to find a task that already holds it. Leave such an issue out
  of the plan, and never close it.
- Put each issue in exactly one entry or one close. Close only an issue whose work already merged,
  and give the evidence, such as the pull request that did it.
- Order the entries in the order they should start. Give an entry `after` an earlier one, with
  moment `merged`, when it needs that one's code on the default branch, such as when both change
  the same files.
- Write each entry's brief as the request its task's agent reads: what to change, why, and how to
  prove it.
- End the plan step with `result(outcome = "planned", summary, batch = {...})`. When the daemon
  refuses the plan, read its error, fix the plan and send it again.
