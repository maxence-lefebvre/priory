# Signals

The daemon learns what an agent is doing from its harness's hooks, not from its screen. Each run's
settings register hooks that call `priory hook <event>`. Each call sends one signal to the daemon.
The daemon never reads the terminal screen to decide an agent is idle.

## How a hook reaches the daemon

`priory hook <event>` reads Claude's hook JSON on stdin. It sends one JSON line to the daemon over
the daemon's socket (see [Priory home](home.md#socket-spool-and-lock)), such as:

```json
{"event": "UserPromptSubmit", "session": "6f1c0b9e-0d2a-4f43-9a51-3c2e8e1b7d10", "prompt": "Implement the request and run the tests.", "transcript": "/home/me/.claude/projects/.../6f1c0b9e.jsonl"}
```

The line carries:

- `event`: the hook's name, from the command line.
- `session`: the run's session. Every run's pane sets `PRIORY_SESSION` to it.
- `prompt`: the submitted text. Only `UserPromptSubmit` carries it.
- `message`: the permission prompt's text, for `PermissionPrompt`.
- `transcript`: the `transcript_path` of Claude's hook JSON, which [trace.md](trace.md) reads for
  token counts.
- `source`: why `SessionStart` fired, such as `startup` or `compact`.
- `thread`: the harness's own session id, when it differs from `PRIORY_SESSION`.

A Codex run chooses its own session id, its thread. So when the stdin `session_id` differs from
`PRIORY_SESSION`, the hook sends `PRIORY_SESSION` as `session` and the stdin id as `thread`.
`SessionStart` stores the thread on the run (see [codex.md](codex.md)).

The daemon answers `ok` once it has committed the signal. The hook waits up to 10 seconds for that
answer. If no daemon accepts the line, or the daemon closes the connection without an answer, the
hook appends the line to `signals.spool` in Priory home. A jailed run sees only its own spool,
`spools/<session>/signals.spool`, which `PRIORY_SPOOL` names (see [sandboxes.md](sandboxes.md)).

The next daemon to start reads the spool before anything else:

1. It marks every `attempted` prompt `uncertain` (see [outbox.md](outbox.md)).
2. It applies each spooled signal, then removes the spool. It reads `signals.spool` first, then
   each jailed run's spool.

So a spooled `UserPromptSubmit` no longer confirms the prompt it submitted. That prompt stays
`uncertain` on the desk.

## The events

| Event | Meaning | Effect on the run |
| ----- | ------- | ----------------- |
| `SessionStart` | The harness is up. Its input may read keys a moment later | idle |
| `UserPromptSubmit` | A prompt was submitted | busy; may confirm the attempted prompt |
| `Stop` | The turn ended | idle; the task moves on by the run's result, ask or wait |
| `PostCompact` | Codex finished a `/compact` | idle |
| `PermissionPrompt` | Claude shows a permission prompt, with its `message` | ends the run (see [health.md](health.md)) |

Some events need more detail:

- `SessionStart` with `source` set to `compact` ends a `/compact` that Priory typed (see Compact in
  [outbox.md](outbox.md)). Otherwise it changes nothing, because Claude also fires it when it
  compacts the conversation on its own, mid-turn.
- `PostCompact` is Codex's end of a `/compact`. Codex 0.170 fires `SessionStart` with
  `source: compact` only with the next turn, so only Codex runs register this hook.
- `UserPromptSubmit` confirms the run's attempted prompt if the submitted text has the same words.
  Before comparing, the daemon removes the `pasted_content` tag that Claude wraps around a pasted
  prompt.

`UserPromptSubmit` also stamps the start of the run's turn. `SessionStart` and `Stop` clear it. The
step's `timeout` counts from that stamp.

`PreToolUse` sends no signal. It is the tool guard's hook, which asks the daemon about an
irreversible call and denies it unless a person approved it (see [guard.md](guard.md)).

## A turn, signal by signal

Here is one turn of a Claude run, from launch to the end of a step:

1. The daemon starts `claude` in tmux. Claude fires `SessionStart`. The run is idle.
2. A second later, the daemon pastes the step's prompt, then presses Enter on its own (see
   [Typing a prompt](outbox.md#typing-a-prompt)). Claude fires `UserPromptSubmit` with the text. The words match the attempted prompt, so the prompt is confirmed and the run is busy.
3. The agent works, then calls the `result` tool.
4. The turn ends. Claude fires `Stop`. The run is idle, and the task moves to the step that
   `next[outcome]` names (see Moving on in [tools.md](tools.md)).

## Nudge

A turn that ends with no `wait`, `ask` or `result` gets one nudge. When the `Stop` signal finds the
run with no result, no open ask and no open wait, the daemon queues a prompt that names the three
tools, followed by the step's ending line. The step's ending line is the one that lists its
outcomes, such as `End this step by calling the result tool of the priory MCP server with one of
these outcomes: failed, passed.` The next tick types the nudge like any other prompt.

Priory only nudges its own turns. It tells them apart from a person's turns by the prompt it
confirmed:

- `UserPromptSubmit` marks the turn nudgeable only when the submitted text matches the prompt
  Priory attempted, and that prompt is not a nudge itself.
- A turn that a person typed into the pane matches nothing, so it is never nudged.
- A successful `wait`, `ask` or `result` call clears the mark, and so do `SessionStart` and `Stop`.
  So a turn whose ask was answered, or whose wait fired before `Stop`, is not nudged.

So a nudge starts at most one turn. A silent turn after a nudge is not nudged again. The health
check then handles the run, as for any run that sends no exit. If a prompt is already queued when
the turn ends, it follows the nudge in the same prompt. A nudge still queued when the task enters
another step, as when a merge moves it on, is dropped: the new step's prompt says how to end it.

The nudge is an outbox prompt, not the hook's `decision: block`. The daemon already confirms,
retries and survives a restart for outbox prompts, while the hook has no reply channel beyond `ok`.

### The nudge's evaluation

[`nudge.eval.toml`](../../crates/priory-core/src/nudge.eval.toml) measures the nudge's text, kept
in [`nudge.txt`](../../crates/priory-core/src/nudge.txt), against the text it replaced. The
previous text named the three tools in one sentence, and Haiku answered a question meant for a
person with a guessed `result` instead of `ask`. The shipped text lists `ask` first and forbids a
guessed outcome. Each case
starts with a step prompt that lacks its ending line, so the agent ends that turn in text, as a
silent turn does. The nudge follows, with the step's ending line. A run hits when the agent then
calls the tool the case expects. [evaluating-a-prompt.md](../evaluating-a-prompt.md) says how to
run it again.

<!-- priory eval nudge.eval.toml -->
Model `haiku`, runs per case: 5, $0.0711 spent.

| Case | Variant | Expected | Hits | What the agent did |
| ---- | ------- | -------- | ---- | ------------------ |
| finished | previous | result passed | 5/5 | result passed 5 |
| finished | shipped | result passed | 5/5 | result passed 5 |
| needs-a-person | previous | ask | 0/5 | result done 5 |
| needs-a-person | shipped | ask | 5/5 | ask 5 |
| waiting-on-checks | previous | wait checks | 5/5 | wait checks 5 |
| waiting-on-checks | shipped | wait checks | 5/5 | wait checks 5 |
<!-- /priory eval -->
