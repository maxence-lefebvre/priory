# Signals

`priory hook <event>` reads Claude's hook JSON on stdin and sends one JSON line to the daemon over
`priory.sock`: `{"event": "Stop", "session": "<id>", "prompt": "<text>"}`. `prompt` comes only with
`UserPromptSubmit`. Every run's pane sets `PRIORY_SESSION` to the run's session. When the stdin
`session_id` differs from it, as in a Codex run, the hook sends `PRIORY_SESSION` as the session and
the stdin id as `thread`, which `SessionStart` stores. The daemon answers `ok` once the signal is
committed; the hook waits up to 10 seconds. If no daemon takes the line, it goes to
`signals.spool`. The next daemon first marks every attempted prompt `uncertain`, then applies the
spool and removes it. So a spooled `UserPromptSubmit` no longer confirms the prompt it submitted.

- `SessionStart`: the harness is up and its input is ready, so the run is idle. One whose `source`
  is `compact` ends a `/compact` Priory typed (see Compact in Outbox), and otherwise changes nothing:
  Claude also fires it when it compacts the conversation on its own, mid-turn.
- `PostCompact`: Codex's end of a `/compact`. Codex 0.170 fires `SessionStart` with `source:
  compact` only with the next turn, so only Codex runs register this hook.
- `UserPromptSubmit`: the run is busy. Its attempted prompt is confirmed if the submitted text has
  the same words, once the `pasted_content` tag Claude wraps a pasted prompt in is removed.
- `Stop`: the turn ended, so the run is idle.
- `PermissionPrompt`: Claude shows a permission prompt with its `message` (see Health).

Each signal also carries the `transcript_path` of Claude's hook JSON, which Trace reads.

`UserPromptSubmit` also stamps the start of the run's turn. `SessionStart` and `Stop` clear it.
The daemon never reads the terminal screen to decide an agent is idle.

## Nudge

A turn that ends with no `wait`, `ask` or `result` gets one nudge. The `Stop` signal finds the run
with no result, open ask or open wait, and queues a prompt naming the three tools, then the step's
ending line from `tasks.ending`. The next tick types it like any other prompt.

Priory tells its own turns from a person's by the prompt it confirmed. `UserPromptSubmit` marks the
turn nudgeable only when the submitted text matches the prompt Priory attempted and that prompt is
not a nudge itself. A turn a person typed into the pane matches nothing, so it is never nudged.
A successful `wait`, `ask` or `result` call, `SessionStart` and `Stop` clear it. So a turn whose
ask was answered, or whose wait fired before `Stop`, is not nudged.

So a nudge starts at most one turn. A silent turn after it is not nudged again, and the health check
then handles the run, as for any run that sends no exit. A prompt already queued when the turn ends
follows the nudge in the same prompt. The nudge is an outbox prompt, not the hook's
`decision: block`. The daemon already confirms, retries and survives a restart for outbox prompts.
The hook has no reply channel beyond `ok`.
