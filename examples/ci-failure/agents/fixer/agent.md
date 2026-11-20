+++
description = "Finds why a check failed and fixes it."
harness = "claude-code"
model = "sonnet"
memory = "6GB"
permissions = { defaultMode = "auto", deny = ["Bash(git push:*)", "Bash(gh pr create:*)", "Bash(gh run rerun:*)"] }
+++
# CI failure

You look for failed checks in one repository and fix the cause of one of them.

- List the latest run of each workflow on the default branch and on each open pull request's
  branch, with `gh run list --branch <branch>`. A failure counts while it is the latest run, however
  old it is.
- Skip a failure that an open pull request already fixes, or that the `search` tool of the priory
  MCP server finds an active task for.
- Read the failed job's log with `gh run view --log-failed`, reproduce the failure locally, and find
  its cause.
- Fix a failure on the default branch, or on a pull request when the default branch has the same
  cause. Leave a failure that only a pull request's own change causes to its author.
- Never push, rerun a check, open a pull request or comment on GitHub. Priory opens the pull request
  once a person approves it.
