# Skills and instructions

A factory's `skills/` folder and `instructions.md` reach every run on every runner, through the
factory checkout each runner already has (R11.1). This page explains how each harness gets them and
how a run records what it got. The rest of R11.1, a kit repository and memories, is in
[the design note](../design/shared-skills-and-memories.md).

## The files

```text
instructions.md                    # optional: text before every agent's own prompt
skills/<name>/SKILL.md             # one folder per skill, with any files beside SKILL.md
```

`SKILL.md` starts with a YAML frontmatter, as Claude Code's skills do:

```markdown
---
name: review-comments
description: Writes the inline comments of a pull request review.
---
Say what is wrong in the first sentence, then why it matters.
```

`priory check` refuses a skill whose frontmatter has no `name` or no `description`, or whose `name`
differs from its folder, with the file and line 1. Its `ok` line counts the skills. The harness reads
the other frontmatter keys, so Priory passes them through.

They belong to the factory, not to a person. A run keeps reading the person's own `~/.claude` or
`~/.codex` as before: Priory adds the factory's files through launch flags, for that run only, and
writes nothing in either folder.

## A Claude Code run

Before the run starts, the daemon writes a plugin folder for the factory's skills under Priory home:
`$PRIORY_HOME/kit/<hash>/plugin/`, with `.claude-plugin/plugin.json` naming the plugin `priory` and
each skill under `skills/<name>/`, byte for byte. The hash covers the skills' names, paths and bytes,
so a later run with the same skills reuses the folder and writes nothing. The folder is written
under a temporary name and renamed into place, so a crash never leaves half a plugin.

The run gets `--plugin-dir <that folder>`. Claude Code loads it for that session only, and shows its
skills as `priory:<name>`.

With `instructions.md`, the run gets `--append-system-prompt-file` in place of
`--append-system-prompt`. The file holds the instructions, a blank line, then the agent's own prompt
and the [writing guide](writing.md). tmux refuses a command over about 16 KiB, so instructions of any
size reach Claude through the file, `$PRIORY_HOME/prompts/<session>.system.md`. The daemon writes it
again at each launch, so a resumed session gets the current text.

Without skills or instructions, a run's arguments are as before.

## A Codex run

A Codex run gets the instructions, a blank line, then the agent's own prompt in
`developer_instructions`, on its command line. `priory check` refuses a Codex agent whose prompt
and the instructions together pass 8192 bytes, which keeps the command under tmux's limit with
Priory's hooks and tools:

```text
agents/reviewer/agent.md:3: a codex agent takes instructions.md and its own prompt on its command line, as `developer_instructions`: together they are 40995 bytes, over the limit of 8192
```

Codex reads skills from its own home folder, so a Codex run gets no skills yet.

## What a run records

Each run records `kit_hash`, a hash of the skills and instructions it got, or none when it got
neither. A Claude run's hash covers both. A Codex run's covers the instructions only, so the two
differ in one factory. `priory runs` shows it in its `kit` column, and the `factory.step.ended`
trace record of a model step carries it (see [the trace schema](../trace-schema.md)).

Two runners whose factory checkouts differ give the same agent's runs different hashes. That is how a
person sees that a runner needs a pull.

A shared session compares its agent's prompt digest with the one it started with (see
[sessions.md](sessions.md)). With skills or instructions, that digest covers the hash too, so a
change to either reads as a change to the agent's prompt.

## Known limits

- Priory never removes an old plugin folder or a `.system.md` file. Both are small.
- No test runs a real Claude Code: that a run lists and uses `priory:<name>` is checked by hand.
- Nothing keeps the runners' factory checkouts at one commit: a person pulls each one.
