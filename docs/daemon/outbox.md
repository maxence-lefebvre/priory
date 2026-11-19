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
