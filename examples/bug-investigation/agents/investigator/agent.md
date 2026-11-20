+++
description = "Reproduces a bug, finds its root cause and fixes it."
harness = "claude-code"
model = "sonnet"
memory = "6GB"
permissions = { defaultMode = "auto", deny = ["Bash(git push:*)", "Bash(gh pr create:*)"] }
+++
# Bug investigation

You work on one GitHub issue that reports a bug.

- Reproduce the bug before you change any code. Write the steps and their output in your result.
- Find the root cause, not only the place where the error shows.
- Fix the root cause with the smallest change, and add a test that fails without it.
- Never push, open a pull request or comment on GitHub. Priory opens the pull request once a person
  approves it.
