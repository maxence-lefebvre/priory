# The writing guide

This page explains how a factory's writing guide reaches every agent, and shows what it changes in
an ask. [factory.md](../reference/factory.md) lists the `writing` key.

An agent writes for a person who was away. The person saw none of the work, and reads the ask on a
phone between other things. Each agent's own prompt can say how to write, but then every agent
needs the same edit when the advice changes. The guide is one file for the whole factory.

## How it reaches the agent

Set `writing` in `factory.toml` to a file in the factory folder:

```toml
name = "triage"
writing = "WRITING.md"
```

When the daemon starts a run, it appends the file's text, after a blank line, to the agent's own
prompt. A Claude run gets the result in `--append-system-prompt`. A Codex run gets it in
`developer_instructions`, which is where Codex takes the agent's body (see [codex.md](codex.md)).
The text appears once. Every run of every agent gets it, including a run that resumes a session.

The guide goes through the harness's system prompt flag and not through the MCP server's
instructions, because Claude Code truncates those past about 2 KB.

Without the key, nothing changes: an agent gets its own prompt alone.

## Size

`priory check` refuses a `writing` file that does not exist. It warns, and still passes, when the
text is longer than 4096 bytes. The guide rides in every run's prompt, so a long one costs tokens
on every turn and crowds out the agent's own instructions. The starters' guides are about 1 KB.

## What the starters' guide changes

Eight starters ship the same `WRITING.md`: question first, then the recommendation, what waits and
what happens next, in plain words. `examples/writing.eval.toml` runs two cases that each end in an
ask, with and without the guide, on Haiku. Its `shape` setting counts an ask as `short` when its
question is one line of at most 25 words, and names the optional fields the ask filled. See
[evaluating-a-prompt.md](../evaluating-a-prompt.md) for how to run it.

Without the guide, one ask in six had a short question. With it, five of six did. Every ask in both
variants filled `whatWaits` and `nextAction`, because the `ask` tool's own schema asks for them, so
the guide's effect shows in the question. Three runs per case on a small model show a direction, not a rate.

<!-- priory eval writing.eval.toml -->
Model `haiku`, runs per case: 3, $0.0329 spent.

| Case | Variant | Expected | Hits | What the agent did |
| ---- | ------- | -------- | ---- | ------------------ |
| alias-decision | bare | ask short whatWaits nextAction | 1/3 | ask whatWaits nextAction 2, ask short whatWaits nextAction 1 |
| alias-decision | guided | ask short whatWaits nextAction | 3/3 | ask short whatWaits nextAction 3 |
| release-window | bare | ask short whatWaits nextAction | 0/3 | ask whatWaits nextAction 3 |
| release-window | guided | ask short whatWaits nextAction | 2/3 | ask short whatWaits nextAction 2, ask whatWaits nextAction 1 |
<!-- /priory eval -->
