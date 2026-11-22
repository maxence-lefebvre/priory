# Memories

A memory is one fact the factory keeps for every run, in `memories/<name>.md`. The file name,
without `.md`, is the memory's name. Every run gets an index of the memories, each name and
description, and reads a memory's file when its description fits the task. An agent also writes
memories with the `remember` and `forget` tools, as candidates a person accepts (see
[Candidates](#candidates)). See [skills.md](../daemon/skills.md#memories) for how a run gets them.

A memory's name has lowercase letters, digits and hyphens, and starts with a letter or a digit. The
file starts with frontmatter, fenced by `---` for YAML or `+++` for TOML. The Markdown after the
closing fence is the fact. The whole file is at most 4096 bytes.

Schema: [`config/memory.json`](../schemas/config/memory.json). Editors do not validate frontmatter
against it, but `priory check` applies the same rules.

`memories/` holds only memory files. A file whose name starts with `.`, such as `.gitkeep`, is
skipped.

## Frontmatter keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `name` | string | yes | | The memory's name, the same as its file's | `"release-day"` |
| `description` | string | yes | | When an agent should read the memory. The index shows it on one line | `"When releases go out."` |

Priory ignores any other key.

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

The factory's memories are in <folder>, one fact per file. Read a memory's file when its description fits your task. Never edit these files: save a fact with the remember tool, and remove a wrong one with forget.

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
| `` a codex agent takes instructions.md, the memory index and its own prompt on its command line, as `developer_instructions`: together they are <n> bytes, over the limit of 8192 `` | Reported on a Codex agent's `agent.md`. The count writes the index's folder as `$PRIORY_HOME/kit/<hash>/memories`. With skills, the message names the skill index after the memory index, and the count writes its folder as `$PRIORY_HOME/kit/<hash>/plugin/skills` |
| `frontmatter is not closed by ---` | No closing fence. The same with `+++` |

Each line starts with the file and line, such as `memories/release-day.md:2:`. The name and
description errors point at line 1, except a wrong `name`, which points at its line. A file or
folder that is not a memory has no line.

## Candidates

Each `remember` or `forget` call is one candidate, known by the id of its
[`priory.memory.written`](events.md#priorymemorywritten) message. A candidate's memory is a file like
the ones above, with its `description` quoted as a JSON string:

```markdown
---
name: release-day
description: "When releases go out."
---
Releases go out on Tuesdays.
```

A run gets the kit's memories, then the factory's, then the candidates it sees, each in log order: a
person's accept puts a candidate at the accept's place, and a candidate of the run's own task sits
at its write. A forget removes the memory of its name. The tools are in
[tools.md](../daemon/tools.md#remembername-description-text-forgetname-and-memoryname).

| State | Meaning | Which runs get it |
| ----- | ------- | ----------------- |
| `sending` | Stored on this runner, not read back from the log yet | Its own task's |
| `pending` | On the log, and waits for a person | Its own task's |
| `accepted` | A person accepted it | Every task's, on every runner |
| `dropped` | A person dropped it | None |
| `retired` | A person accepted it, then dropped it | None: the kit's or the factory's memory of its name, if any, applies again |

### `priory memories`

```sh
priory memories [DIR] [--all] [--json]
priory memories (accept | drop) [DIR] CANDIDATE
```

The first lists the `pending` candidates this runner read, oldest first, or with `--all` every
candidate: id, name, kind (`remember` or `forget`), state, task key, runner (`-` in a factory of one
runner) and when it was written. `--json` prints `{ "candidates": [...] }`, each with `id`, `name`,
`description`, `text` and `forget`, `task`, `runner`, `written`, `decided` and `state`. Given the
factory's folder `DIR`, each also carries `replaces`, whether runs get a memory of its name now, and
`current_text`, that memory's text, or null. Those memories are the kit's at the commit runs get,
the factory's, then the accepted candidates. `replaces` is null, unknown, for a name the factory and
the accepted candidates lack while the factory's kit has no commit checked out on this machine, as
on a machine that only views. `/api/desk` marks its candidates the same way.

`accept` and `drop` publish [`priory.memory.accepted`](events.md#priorymemoryaccepted) or
[`priory.memory.dropped`](events.md#priorymemorydropped) on the log of the factory in `DIR`, `.` by
default, and print `<name> accepted: every runner applies it as it reads the log`. `drop` of an
`accepted` candidate retires it: it publishes `priory.memory.dropped` with `retire: true` and prints
`<name> retired: every runner applies it as it reads the log`.
`POST /api/memories/{id}/accept` and `POST /api/memories/{id}/drop` are their twins (see
[serve.md](serve.md#writes)). They check this
runner's state file first and refuse:

| Message | Cause |
| ------- | ----- |
| `no memory candidate <id>` | This runner never read that id |
| `candidate <id> is not on the log yet: try again once its runner reaches the log` | It is `sending` |
| `candidate <id> is accepted` | `accept` of a candidate a person already accepted |
| `candidate <id> is dropped`, `candidate <id> is retired` | A person already dropped or retired it |
| `the log is unreachable since <time>: try again when it is back` | This runner is cut off from the log |

A candidate takes one accept or drop. The first in log order stands, and every runner records a
later one as `refused`. After an accept, only a retire applies. A retired candidate stays out of
every run and out of the daily pull request to the kit, so a fix to the kit's memory of its name
reaches runs again. An open pull request that still holds the retired text is not changed: close it.

### Errors from the tools

| Message | Cause |
| ------- | ----- |
| `a memory's name has lowercase letters, digits and hyphens, and starts with a letter or a digit` | A name such as `Release_Day` |
| `a memory needs a description, which tells an agent when to read it` | A blank `description` |
| `a memory needs a text: the fact itself` | A blank `text` |
| `a memory holds one fact in at most 4096 bytes: this one is <n> bytes` | The candidate's file, as above, is over 4096 bytes |
| `the text looks like it holds a credential: a memory reaches every later run, so keep secrets out of it` | The name, description or text matches a GitHub, Anthropic, OpenAI, AWS, Slack or Linear key, or a PEM private key. A secret with no known shape, such as a webhook secret, passes |
| `no memory <name> reaches your task: your instructions' index lists those that do` | `memory` names a memory the caller's task does not see |

A runner applies the same rules to each `priory.memory.written` it reads, since anyone who can
publish on the log can write one.
