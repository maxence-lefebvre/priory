# Outbox

Every prompt the daemon types goes through the outbox. A prompt moves through these states:

| State       | Meaning                                                                          |
| ----------- | -------------------------------------------------------------------------------- |
| `queued`    | Waits for the task's run to be idle                                              |
| `attempted` | Recorded, with the run marked busy, in the transaction before typing             |
| `confirmed` | Its `UserPromptSubmit` arrived                                                   |
| `uncertain` | Attempted before a daemon stop, never confirmed. Priory never types it again     |
| `withdrawn` | Queued when a person revived the task. Its new generation never gets it          |

A model step's prompt is the step's `prompt`, then the line `End this step by calling the result
tool of the priory MCP server with one of these outcomes: <the keys of next>.` A run's first prompt
is that, then one line per event: the task's source event and every kept event that no prompt
carried yet. Each line holds the type, subject and data.
A `ready` task's run has only those event lines, or none when no new events exist: the prompt that
moved or woke the task is already queued.

Once a task's run is idle, the daemon types its oldest queued prompt. It loads the text into a tmux
buffer from stdin and pastes it as one bracketed paste, so newlines do not submit and key names in
the text stay literal. Then it presses Enter.

A task with an attempted or uncertain prompt gets no further prompt while its text may still sit in
the agent's input. That holds while the run it went to is live. A run carried into the next step
takes over that prompt, since its process carries on. Once the run holding the prompt ended any
other way, its process is gone, and the task's next run gets its prompts.

A `UserPromptSubmit` with the same words, even one the daemon reads after a restart, confirms an
`uncertain` prompt of its run. Otherwise the prompt stays `uncertain` on the desk. The schema upgrade
that brought this rule hands an unconfirmed prompt on a `moved` run to the live run that carries on
its process, when no run ended any other way in between.

## Compact

The daemon types a queued `/compact` like any other prompt, once the run is idle, so an answer or a
nudge queued before it goes first. It is always typed alone. A prompt that a move, an answer or a
nudge queues after it still goes ahead of it, as above, but never takes it in. A new
run's first prompt is never a `/compact`: the task intro and Codex's launch argument skip it.

Typing it marks the run busy and starts its turn, so the step's `timeout` covers the compaction. The
compaction ends with Claude's `SessionStart` with `source: compact`, or Codex's `PostCompact`. Either
confirms the `/compact`, marks the run idle and queues a prompt ahead of the others: `Priory
compacted this conversation, as you asked. You are on task <id>, at step <step>. Carry on with the
step.`, then the step's ending line. That prompt starts a new turn, so the timeout restarts. The
transcript stays the same, so the next `Stop` counts the compaction's tokens with the turn's.
A compaction Priory did not type, as Claude's automatic one mid-turn, changes nothing.
