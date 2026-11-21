# Memories

A memory is one fact the factory keeps for every run, in `memories/<name>.md`. The file name,
without `.md`, is the memory's name. Every run gets an index of the memories, each name and
description, and reads a memory's file when its description fits the task. No tool writes a memory:
a memory changes only through a pull request to the factory. See [skills.md](../daemon/skills.md#memories)
for how a run gets them.

A memory's name has lowercase letters, digits and hyphens, and starts with a letter or a digit. The
file starts with frontmatter, fenced by `---` for YAML or `+++` for TOML. The Markdown after the
closing fence is the fact. The whole file is at most 4096 bytes.

`memories/` holds only memory files. A file whose name starts with `.`, such as `.gitkeep`, is
skipped.

## Frontmatter keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `name` | string | yes | | The memory's name, the same as its file's | `"release-day"` |
| `description` | string | yes | | When an agent should read the memory. The index shows it on one line | `"When releases go out."` |

Priory reads no other key, and refuses none.

## Example

<!-- example: factory/memories/release-day.md -->
```markdown
---
name: release-day
description: When releases go out. Read it before you plan or merge a change near a release.
---
Releases go out on Tuesdays. Merge nothing risky on a Monday afternoon.
```

## The index

A run gets this text after `instructions.md` and before the agent's own prompt, with one line per
memory in name order:

```text
# Memories

The factory's memories are in <folder>, one fact per file. Read a memory's file when its description fits your task. Never edit these files: a memory changes only through a pull request to the factory.

- release-day.md: When releases go out. Read it before you plan or merge a change near a release.
```

`<folder>` is `$PRIORY_HOME/kit/<hash>/memories`, where `<hash>` is a hash of the memories.

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `memories/ holds only <name>.md files, one memory each` | A file without `.md`, or a folder, under `memories/` |
| `a memory's file is named <name>.md, with lowercase letters, digits and hyphens` | A file name such as `Release_Day.md` |
| `a memory holds one fact in at most 4096 bytes: this file is <n> bytes` | The file is over 4096 bytes |
| `` a memory's frontmatter needs a `name` `` | No `name` |
| `` `name` is `<name>`, but the memory's file is memories/<file>.md `` | `name` differs from the file's name |
| `` a memory's frontmatter needs a `description`, which tells the agent when to read it `` | No `description`, or a blank one |
| `` a codex agent takes instructions.md, the memory index and its own prompt on its command line, as `developer_instructions`: together they are <n> bytes, over the limit of 8192 `` | Reported on a Codex agent's `agent.md`. The count writes the index's folder as `$PRIORY_HOME/kit/<hash>/memories` |
| `frontmatter is not closed by ---` | No closing fence. The same with `+++` |

Each line starts with the file and line, such as `memories/release-day.md:2:`. The name and
description errors point at line 1, except a wrong `name`, which points at its line. A file or
folder that is not a memory has no line.
