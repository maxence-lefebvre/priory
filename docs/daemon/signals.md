# Signals

`priory hook <event>` reads Claude's hook JSON on stdin and sends one JSON line to the daemon over
`priory.sock`: `{"event": "Stop", "session": "<id>", "prompt": "<text>"}`. `prompt` comes only with
`UserPromptSubmit`. Every run's pane sets `PRIORY_SESSION` to the run's session. When the stdin
`session_id` differs from it, as in a Codex run, the hook sends `PRIORY_SESSION` as the session and
the stdin id as `thread`, which `SessionStart` stores. The daemon answers `ok` once the signal is
committed, and the hook waits up to 10 seconds for it. When no daemon takes the line, it goes to `signals.spool` instead. The next
daemon applies the spool before it removes it.

- `SessionStart`: the harness is up and its input is ready, so the run is idle. One whose `source`
  is `compact` changes nothing: Claude fires it when it compacts the conversation, mid-turn.
- `UserPromptSubmit`: the run is busy. Its attempted prompt is confirmed if the submitted text has
  the same words, once the `pasted_content` tag that Claude wraps a pasted prompt in is removed.
- `Stop`: the turn ended, so the run is idle.
- `PermissionPrompt`: Claude shows a permission prompt, with its `message`, as described under
  Health.

Each signal also carries the `transcript_path` of Claude's hook JSON, which Trace reads.

`UserPromptSubmit` also stamps the start of the run's turn, and `SessionStart` and `Stop` clear it.
The daemon never reads the terminal screen to decide that an agent is idle.

## Nudge

A turn that ends with no `wait`, `ask` or `result` gets one nudge. The `Stop` signal finds the run
with no result, open ask or open wait, and queues a prompt that names the three tools, then the
step's ending line from `tasks.ending`. The next tick types it like any other prompt.

Priory tells its own turns from a person's by the prompt it confirmed. `UserPromptSubmit` marks the
turn nudgeable only when the submitted text matches the prompt Priory attempted and that prompt is
not itself a nudge. A turn a person typed into the pane matches nothing, so it is never nudged.
A successful `wait`, `ask` or `result` call, `SessionStart` and `Stop` clear it, so a turn whose ask was answered or whose wait fired before `Stop` is not nudged.

So a nudge starts at most one turn, and a silent turn after it is not nudged again. The run is then
left to the health check, like any run that sends no exit. A prompt already queued when the turn
ends follows the nudge in the same prompt. The nudge is an outbox prompt rather than
the hook's `decision: block`: the daemon already confirms, retries and survives a restart for
outbox prompts, and the hook has no reply channel beyond `ok`.
