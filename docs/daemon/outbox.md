# Outbox

Every prompt the daemon types into an agent goes through the outbox, a table in the state file. The
daemon stores each prompt before it types it. So a daemon restart never loses a prompt and never
types one twice.

## Prompt states

A prompt moves through these states:

| State       | Meaning                                                                          |
| ----------- | -------------------------------------------------------------------------------- |
| `queued`    | Waits for the task's run to be idle                                              |
| `attempted` | Recorded, with the run marked busy, in the transaction before typing             |
| `confirmed` | Its `UserPromptSubmit` arrived                                                   |
| `uncertain` | Typed but never confirmed: the daemon stopped, two Enters brought no submit, or `hold --now` ended its run |
| `held`      | A person's `say` during a debate. The step after the debate reads it             |
| `withdrawn` | Queued or held when a person revived the task. Its new generation never gets it  |

A prompt usually goes `queued`, then `attempted`, then `confirmed`. The `uncertain` state covers a
daemon that stopped between typing and confirming, a prompt whose second Enter brought no
`UserPromptSubmit`, and a prompt typed into a run that `hold --now` ended. Priory cannot tell whether the agent received the text, so it shows the prompt
on the desk for a person to judge, instead of typing it again.
`priory outbox` lists every prompt with its state.

## What a prompt holds

A model step's prompt is the step's `prompt`, then a line that tells the agent how to end the step:

```
End this step by calling the result tool of the priory MCP server with one of these outcomes: <the keys of next>.
```

A run's first prompt is that, then one line per event: the task's source event, and every kept
event that no prompt carried yet. Each event line holds the event's type, subject and data. For
example, the first prompt of a task that `priory request` created looks like this:

```
Implement the request and run the tests.
End this step by calling the result tool of the priory MCP server with one of these outcomes: failed, passed.
priory.task.requested.cli cli:docs-1 {"person":"max","repo":"owner/name","text":"Fix the typo in the README","time":"2026-11-17T18:00:00Z"}
```

When a task enters another step, the daemon removes the old step's ending line from the prompts
still queued, such as a person's `say`, so a prompt names only the new step's outcomes. It removes a
queued nudge too, since the nudge asks for the old step's outcomes.

A `ready` task's run starts with only those event lines, or with nothing when no new events exist.
The prompt that moved or woke the task is already queued.

## Typing a prompt

Once a task's run is idle, and at least 1 second after the run's `SessionStart`, the daemon types
its oldest queued prompt:

1. It loads the text into a tmux buffer from stdin.
2. It pastes the buffer as one bracketed paste. So newlines in the text do not submit it, and key
   names in the text stay literal.
3. 0.3 seconds later, it presses Enter as a key press of its own.
4. If no `UserPromptSubmit` arrives within 5 seconds, it presses Enter once more. It never pastes
   the text again.
5. If none arrives within 10 seconds of the second Enter, the prompt becomes `uncertain` and shows
   on the desk.

Claude fires `SessionStart` before its input reads keys. Keys typed before then wait, and Claude
reads them in one burst as a paste, Enter included, so the prompt sits in its input unsent. The
wait after `SessionStart` and the separate Enter keep the Enter apart from the paste. The second
Enter covers an Enter the agent still missed. An Enter on an empty input submits nothing, so it
does no harm when the first one worked and the hook was slow.

A `/compact` gets one Enter and no second one, since its end is a `SessionStart` that can take
minutes.

A task with an `attempted` or `uncertain` prompt gets no further prompt while that text may still
sit in the agent's input. That rule holds while the run the prompt went to is live:

- A run carried into the next step takes over that prompt, since its process carries on.
- Once the run holding the prompt ended any other way, its process is gone, and the task's next run
  gets its prompts.

A run whose keyboard a person holds gets no prompt while the grant is open: `say` queues, and the
daemon types at the run's next idle point after the grant ends. A prompt pasted before the grant
gets no Enter while it is open, so Priory never submits what the person typed, and is not marked
`uncertain` meanwhile. Its wait starts over, so its Enter goes once the grant ends. See
[A run's keyboard](serve.md#a-runs-keyboard).

A `UserPromptSubmit` with the same words confirms an `uncertain` prompt of its run, even one the
daemon reads after a restart. Otherwise the prompt stays `uncertain` on the desk.

The schema upgrade that brought this rule hands an unconfirmed prompt on a `moved` run to the live
run that carries on its process, when no run ended any other way in between.

## Compact

An agent asks for a compaction with the `compact` tool (see [tools.md](../reference/tools.md#compactfocus)), which queues a
`/compact` prompt. The daemon queues a bare `/compact` itself when a turn ends past the agent's
`compactAt` (see [sessions.md](sessions.md#compacting-a-long-session)). Then the run's ask or wait
settles at the next `Stop`, after the compaction, so the task sleeps only once it is typed. The daemon types a queued `/compact` like any other prompt, once the run is
idle, so an answer or a nudge queued before it goes first.

A `/compact` is always typed alone:

- A prompt that a move, an answer or a nudge queues after it still goes ahead of it, but never
  takes it in.
- A new run's first prompt is never a `/compact`. The task intro and Codex's launch argument skip
  it.

Typing it marks the run busy and starts its turn, so the step's `timeout` covers the compaction.
The compaction ends with Claude's `SessionStart` with `source: compact`, or Codex's `PostCompact`.
Either one:

1. confirms the `/compact`,
2. marks the run idle,
3. queues this prompt ahead of the others, followed by the step's ending line:
   `Priory compacted this conversation, as you asked. You are on task <id>, at step <step>. Carry on
   with the step.`

That prompt starts a new turn, so the timeout restarts.

The compaction keeps the transcript, Claude's or Codex's session file, and Priory's read offset in
it, so the next `Stop` counts tokens from where the last one stopped.

A compaction that Priory did not type, such as Claude's automatic one mid-turn, changes nothing.
