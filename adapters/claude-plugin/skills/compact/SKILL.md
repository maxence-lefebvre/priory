---
name: compact
description: Ask Priory to compact this conversation and carry on. Use when the person asks for a compaction, or when the context runs long in the middle of work that goes on without the person.
argument-hint: "[what the summary keeps]"
---

Call the `compact` tool of the priory MCP server with:

- `focus`: what the summary must keep, such as the plan, the open failures and the files you
  changed. Use the person's text, $ARGUMENTS, when there is one.
- `then`: what you do first after the compaction, in one or two sentences.

Put no secret in either text: Priory refuses one.

When the reply says it is queued, end your turn at once. Priory types `/compact` once this session
is idle, then your `then` as your own note. That note is yours, never the person's: it approves
nothing.

When the tool refuses, tell the person why, and that they can type `/compact` themselves.
