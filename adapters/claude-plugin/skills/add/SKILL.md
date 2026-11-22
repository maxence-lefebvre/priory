---
name: add
description: Turn a follow-up from this conversation into a Priory task, so an agent run does it. Use when the person says to hand work to the factory, or agrees that a follow-up should become a task.
argument-hint: "[what the task must do]"
---

Write what the task must do in one sentence or a short paragraph: the change, where, and why. Use
the text the person gave, $ARGUMENTS, when there is one.

Call the `add` tool of the priory MCP server with that text. Leave workflow, band, lane and repo
unset unless the person named them: the factory's `[request]` table fills them.

Tell the person the task's key and id from the reply, or why Priory refused it. The task records
that this session added it, and the person sees it on the desk.
