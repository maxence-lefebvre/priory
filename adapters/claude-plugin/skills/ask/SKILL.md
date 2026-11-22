---
name: ask
description: Ask the person a question through the Priory desk when the work is blocked on their decision and the question should outlive this conversation. Use instead of waiting in the chat when the person may be away.
---

Use this when the work cannot go on without the person's decision: a design choice, a scope
change, something irreversible.

1. Pick an id for the question, such as `cache.ship`: letters, digits, `.`, `_` or `-`. Asking under
   the same id again revises the question.
2. Call the `ask` tool of the priory MCP server with the id, the question with the context the
   person needs, the options you expect, your recommendation and why, and the trade-off of each
   option.
3. Read its reply. It says whether the answer will arrive here as a message, or whether you must
   read it with the `asks` tool.
4. Go on with the work that does not depend on the answer. Never wait in a loop. When the reply
   said to read the answer with `asks`, check once when the person comes back to this session.

Never answer the question yourself, and never run a command that answers it.
