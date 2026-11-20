+++
description = "Implements one task and opens its pull request."
harness = "claude-code"
model = "sonnet"
effort = "high"
runner = "any"
session = "per-task"           # or "shared": one conversation across tasks
memory = "6GB"                 # what a runner must have free to admit a run
disk = "20GB"                  # disk a run reserves above the runner's floor
mcpServers = ["github"]
permissions = { defaultMode = "auto", allow = ["Bash(cargo test:*)"], deny = ["Bash(git push:*)"] }
+++
# Implementation

Read the issue and its acceptance criteria, make the smallest correct change, and test it.
