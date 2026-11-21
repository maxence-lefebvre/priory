# Skills, instructions and memories

A factory's `skills/` folder, `instructions.md` and `memories/` folder reach every run on every
runner, through the factory checkout each runner already has (R11.1). This page explains how each
harness gets them and how a run records what it got. The rest of R11.1, a kit repository and
memories that agents write, is in [the design note](../design/shared-skills-and-memories.md).

## The files

```text
instructions.md                    # optional: text before every agent's own prompt
skills/<name>/SKILL.md             # one folder per skill, with any files beside SKILL.md
memories/<name>.md                 # one fact per file, with a description
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

Without skills, instructions or memories, a run's arguments are as before.

## A Codex run

A Codex run gets the instructions, a blank line, then the agent's own prompt in
`developer_instructions`, on its command line. `priory check` refuses a Codex agent whose prompt
and the instructions together pass 8192 bytes, which keeps the command under tmux's limit with
Priory's hooks and tools:

```text
agents/reviewer/agent.md:3: a codex agent takes instructions.md and its own prompt on its command line, as `developer_instructions`: together they are 40995 bytes, over the limit of 8192
```

Codex reads skills from its own home folder, so a Codex run gets no skills yet.

## Memories

A memory is one fact in `memories/<name>.md`, with a `name` and a `description` in its frontmatter
(see [the reference](../reference/memories.md)). `priory check` refuses a file that is not
`<name>.md`, a memory over 4096 bytes, and a memory without a `name` or a `description` or whose
`name` differs from its file, each with the file and line. Its `ok` line counts the memories.

Before a run starts, the daemon copies the memories, byte for byte, into a snapshot under Priory
home: `$PRIORY_HOME/kit/<hash>/memories/`, named by a hash of the memories. A later run with the same
memories reuses the folder. A changed memory gets a new folder, and an earlier run's folder stays as
it was. The run gets an index after the instructions and before the agent's own prompt: the
snapshot's folder, then one line per memory with its file name and description. The agent reads a
memory's file when its description fits its task, so a long memory costs nothing until it is
needed.

A Claude Code run gets the index through `--append-system-prompt-file`, as instructions do, and
`--add-dir <snapshot>`, so it reads the files without a permission prompt. A Codex run gets the
index in `developer_instructions`, and its sandbox reads any file. `priory check` counts the index
toward a Codex agent's 8192-byte limit.

No tool writes a memory, and the snapshot is a copy: an agent that edits it changes nothing a later
run reads. A memory changes only through a pull request to the factory, which a person reviews.

## What a run records

Each run records `kit_hash`, a hash of the skills, instructions and memories it got, or none when it
got none of them. A Claude run's hash covers all three. A Codex run's covers the instructions and
memories only, so the two differ in a factory with skills. `priory runs` shows it in its `kit` column, and the `factory.step.ended`
trace record of a model step carries it (see [the trace schema](../trace-schema.md)).

Two runners whose factory checkouts differ give the same agent's runs different hashes. That is how a
person sees that a runner needs a pull.

A shared session compares its agent's prompt digest with the one it started with (see
[sessions.md](sessions.md)). With skills, instructions or memories, that digest covers the hash too, so a
change to any of them reads as a change to the agent's prompt.

## Known limits

- Priory never removes an old plugin folder, memory snapshot or `.system.md` file. They are small.
- No test runs a real Claude Code: that a run lists and uses `priory:<name>` is checked by hand.
- Nothing keeps the runners' factory checkouts at one commit: a person pulls each one.
