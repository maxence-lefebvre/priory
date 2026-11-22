# Skills, instructions and memories

A factory's `skills/` folder, `instructions.md` and `memories/` folder reach every run on every
runner, through the factory checkout each runner already has (R11.1). A kit repository that
`[kit]` names adds its own, and a memories repository that `[memories]` names adds memories. Each
runner fetches both. This page explains how each harness gets
them, how a run records what it got, and how memories that agents write reach other runs once a
person accepts them. The rest of R11.1 is in [the design note](../design/shared-skills-and-memories.md).

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
differs from its folder, with the file and line 1. Its `ok` line counts the skills. Priory also reads
`paths` (see [Skills and memories by path](#skills-and-memories-by-path)). The harness reads the
other frontmatter keys, so Priory passes them through.

They belong to the factory, not to a person. A run keeps reading the person's own `~/.claude` or
`~/.codex` as before: Priory adds the factory's files through launch flags, for that run only, and
writes nothing in either folder.

## A Claude Code run

Before the run starts, the daemon writes a plugin folder for the factory's skills under Priory home:
`$PRIORY_HOME/kit/<hash>/plugin/`, with `.claude-plugin/plugin.json` naming the plugin `priory` and
each skill under `skills/<name>/`, byte for byte. The hash covers the skills' names, paths and bytes,
so a later run with the same skills reuses the folder and writes nothing. The folder is written
under a temporary name, made read-only and renamed into place, so a crash never leaves half a
plugin.

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

Codex reads skills only from its own home folder, `~/.agents/skills`, the repository's
`.agents/skills` and its installed plugins. No `-c` override adds a folder, so a Codex run gets an
index of the skills instead. The daemon writes the same read-only plugin folder as for a Claude Code
run, and the run gets, after the instructions and the memory index, the folder's `skills/` path,
then one line per skill with its `SKILL.md` path and its description:

```text
# Skills

The factory's skills are in /home/max/.local/state/priory/kit/3f2a…/plugin/skills, one folder per skill. Read a skill's SKILL.md when its description fits your task, and follow it. Never edit these files.

- release-notes/SKILL.md: Writes release notes from merged pull requests.
```

Codex's sandbox reads any file, so the run reads a skill's files where they are. Codex does not
list these skills in its own skill picker: the agent finds them through the index only.
`priory check` counts the index toward the 8192-byte limit.

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

An agent also writes memories, with the `remember` and `forget` tools, as the next section says.
A memory in git changes only through a pull request, which a person reviews.

## Skills and memories by path

An index works only when the agent picks the right line from it, and a long kit makes it miss more
often. So a skill or memory can name the files it is about, with `paths` in its frontmatter: globs
relative to the root of the task's repository (see
[the reference](../reference/memories.md#path-globs)).

```markdown
---
name: migrations
description: Writes a schema migration.
paths:
  - crates/*/src/state.rs
  - "migrations/**/*.sql"
---
Take the next free version, and never edit a migration that shipped.
```

When a Claude Code run reads, edits or creates a file a glob matches, Priory adds that skill's
`SKILL.md` or that memory's file to the run's context, before the tool runs. Code decides, so
nothing relies on the model picking the line. A run gets each file once, however many matching files
it touches. The skill or memory stays in the index and in the plugin as before.

`priory check` refuses a glob the hook cannot match, with the file and the line of `paths` (see
[the errors](../reference/memories.md#errors-in-path-globs)).

How it works:

1. Before the run starts, the daemon writes the skills and memories with `paths`, each with its
   globs and its file's text, to `$PRIORY_HOME/kit/<hash>/paths/paths.json`, read-only like the
   plugin folder. The run's pane gets `PRIORY_PATHS`, that file, and `PRIORY_WORKTREE`, the task's
   worktree.
2. Claude runs `priory hook PreToolUse` before every `Read`, `Edit`, `Write`, `MultiEdit` and
   `NotebookEdit` call too, as well as `Bash` and MCP calls for the [tool guard](guard.md). The hook
   takes the file the call names, relative to the worktree, and matches it against each glob. A
   file outside the worktree matches nothing.
3. On a match, the hook asks the daemon which matching files the run has not got yet, and waits up
   to 1 second. The daemon records each new one against the run in its state file and replies with
   them.
4. The hook prints each new file's text as the call's `additionalContext`, after a line that names
   the skill or memory, its file, and the path and glob that matched. The call then runs as before:
   the hook never denies a file tool's call.

A daemon that does not answer counts every match as new, so a run then gets a file again at each
match rather than never. A lookup that fails adds nothing. A resumed session is a new run, so it
gets each file again at its first match.

Limits:

- A Codex run gets no file by path yet: its hook sees no file tool, and Codex reads files through
  its shell.
- The hook sees only the file a file tool names. A `cat` or `sed` in `Bash`, or a search with
  `Grep` or `Glob`, brings in nothing.
- A memory an agent writes with `remember` has no `paths`.

## Memories agents write

A memory one run learns must reach runs on every runner, and a poisoned memory must not. So an
agent's memory travels on the log, as a candidate, and a person accepts it before other tasks get
it.

### Candidates

Each `remember(name, description, text)` or `forget(name)` call is one candidate (see
[tools.md](tools.md#remembername-description-text-forgetname-and-memoryname)). The daemon stores it
in its state file, then publishes it as `priory.memory.written` through its queue, as it does a
claim, so a runner cut off from the log publishes it once the link is back. The message's id is the
candidate's id. A later write of the same name is a new candidate, never an edit. A forget is a
candidate too: a tombstone for its name.

`remember` refuses an exact repeat: a name, description and text equal to the memory the task
sees under that name, to a candidate of that name that waits for a person, or to one a person
dropped. The reply says which, and a dropped one's names the drop's date. A changed text under the
same name is a correction and is stored. The runner that serves the call applies this rule, and
only to its own writes: two runners can write the same text before either reads the other, so a
candidate read from the log is recorded as before (see
[memories.md](../reference/memories.md#errors-from-the-tools)).

Every runner records each candidate as it reads the log, in log order, in a factory of one runner
too. A [view-only machine](runners.md#a-machine-that-only-views) records them as well, and a person
there can accept or drop one. A runner refuses a candidate that breaks the rules of `remember`, since anyone who can publish
on the log can write one.

### Which run gets which candidate

A run gets the kit's memories, then the memories repository's, then the factory's, then, in log
order:

- each accepted candidate, at the place of its accept, and
- each candidate its own task wrote on this runner and no person dropped, at the place of its
  write. One not read back from the log yet comes last.

A forget removes the memory of its name, whether the kit, the memories repository, the factory or a
candidate held it. So:

- The task that wrote a candidate gets it at its next run, such as the run an answer resumes.
- Another task gets it only once a person accepts it, on any runner, since every runner reads the
  accept.
- Accepting a candidate never accepts a later write of the same name. That write waits for its own
  accept, so unreviewed text never inherits one.
- Between two accepted candidates of one name, the later accept in log order wins, and every runner
  agrees, since they read one order.

The memories a run gets go into its snapshot, `$PRIORY_HOME/kit/<hash>/memories/`, and its index,
as above. A candidate's file holds its name, its description and its text (see
[the reference](../reference/memories.md#candidates)). An accepted candidate changes the run's
`kit_hash`, so a shared session reads it as a change to its agent's prompt. The `memory(name)` tool
replies with the file the caller's task sees at the time of the call, so it can show a memory
accepted after the run started.

### Accepting a candidate

The desk shows one `memories` item while any candidate waits, which lists every one (see
[desk.md](desk.md#memory-candidates)). A person reads them with `priory memories`, then accepts or
drops each by its id:

```sh
priory memories
priory memories accept 0196f1d0-4a5b-7c6d-8e9f-0a1b2c3d4e5f
priory memories drop 0196f1d0-9a8b-7c6d-8e9f-0a1b2c3d4e5f
```

The web Desk and the Herdr Desk offer the same Accept and Drop, beside each candidate's full text
and the text it replaces (see [desk.md](desk.md#memory-candidates)).

Each publishes its verdict on the log, and every runner applies it as it reads it. A drop also
takes the candidate from its own task's later runs. A candidate takes one verdict: the first in log
order stands. The one exception is a retire: `priory memories drop`, or Drop through the endpoint,
of an accepted candidate takes it out of every later run and out of the daily pull request, so a
wrong accept is not kept for good.

### Accepted memories move into the kit

Accepted candidates live in the log and in each state file. A daily pull request moves them into
the memories repository, or into the kit repository when `[memories]` is unset, where a person
merges them. The steps below say "the kit" for whichever of the two it is. A schedule trigger starts it, and a code step with
the `open-memories-pr` action opens it:

```toml
# triggers/kit-memories.toml
source = "schedule"
repo = "owner/abbey"           # the GitHub owner/name of the memories repository or the kit
at = "06:00"
key = "kit-memories"
workflow = "memories"
```

```toml
# workflows/memories.toml
start = "open"

[[steps]]
id = "open"
decides = "code"
action = "open-memories-pr"
effect = "irreversible"
autonomous = "a person reviews the pull request before any memory merges"
next = { opened = "done", unchanged = "done", failed = "stop" }
```

Each firing is a task request, so the runners claim its key as they claim any other, and one
runner runs the step (see [runners.md](runners.md#claims)). That runner:

1. Fetches the kit's `ref` into its mirror under Priory home.
2. Takes, for each name, the accepted candidate whose accept came last in log order. A pending,
   dropped or retired candidate never goes in.
3. On top of the ref's commit, writes each accepted memory as `<memories>/<name>.md`, byte for byte
   the file runs get, and removes the file of each accepted forget. `<memories>` is the memories
   repository's `path`, or the kit's `memories` folder; `memories` by default.
4. When that changes no file, ends `unchanged` and pushes nothing. So once the pull request merges,
   the next firing proposes nothing.
5. Otherwise pushes the commit to the kit's `priory/memories` branch with git's own credentials,
   replacing what the branch held.
6. Opens a pull request from `priory/memories` onto the kit's `ref`, or onto the default branch when
   it sets no `ref`, with `gh pr create -R <repo>`. When one is open already, it edits that
   one's title and body with `gh pr edit`.

The body lists each memory the commit adds, changes or removes, with its candidate's id, task,
runner and accept time. The commit is authored by `Priory <priory@localhost>` and dated at the last
accept, so the same memories on the same ref make the same commit.

The step needs push rights to the kit repository and a signed-in `gh` on whichever runner wins
the claim. A runner without them ends the step `failed`, with git's or gh's error.

### Limits

- An accepted candidate stays in the state file once the kit holds it, and still overrides the
  kit's file of its name. So a later edit of that file in git reaches no run, and the next pull
  request proposes the candidate's text again, until a person retires the candidate with
  `priory memories drop`. A retire does not change a pull request already open: close it.
- `priory/memories` is Priory's own branch: each firing rebuilds it from the kit's ref, so a commit
  pushed to it is lost.
- The kit's `ref` must be a branch, or unset, for the pull request to open onto it.
- Write access to the log equals a person's accept, as it does an answer (see
  [runners.md](runners.md#every-runner-reads-the-whole-log)).
- The credential check matches common key shapes only. It does not compare a memory with the
  factory's own secret values, so a webhook secret, the serve token or any other value with no
  known shape passes it. A memory reaches other tasks only after a person reads it, so read each
  one for secrets and for instructions an issue may have planted.

The plugin folder, the memory snapshot and the kit checkout below are read-only: each file is
`0444` and each folder `0555`. An agent can still make them writable with a shell, so before each
run the daemon reads its plugin folder and memory snapshot again. A folder whose files, bytes or
modes differ from what it should hold is written again, so an edit by one run changes nothing a
later run reads. A kit checkout is checked again the same way each time the runner loads its
commit. An edit made while a run is live can still reach that run, until it ends.

## A kit repository

`[kit]` in `factory.toml` names a git repository whose skills, instructions and memories every run
gets too (see [the reference](../reference/factory.md#kit)):

```toml
[kit]
repo = "git@github.com:owner/abbey.git"
ref = "main"
instructions = "claude/CLAUDE.md"
every = "5m"
```

### When a runner fetches it

Each runner fetches on its own, with git's own credentials, such as an SSH key or a credential
helper. Priory reads no token, and a fetch that would prompt for one fails instead.

- At daemon start, the runner fetches the ref before it starts any run.
- Every `every`, on the daemon's tick, it fetches again, off the daemon's thread. A new commit
  applies to runs that start after the fetch. A live run keeps the files it started with. A resumed
  session gets the newer commit, since its next run is a new launch.

The fetch goes into a bare mirror under Priory home, `$PRIORY_HOME/kit/repos/<hash>.git`. The runner
then checks the commit out into `$PRIORY_HOME/kit/<commit>/`, read-only, and loads its skills,
instructions and memories as `priory check` loads a factory's. A symbolic link in the kit becomes a
file that holds its target.

### When a fetch fails

Offline is normal, so a failed fetch never blocks runs:

- A runner that had a good commit keeps it, even across a restart, since the state file holds it.
- A runner with no good commit yet starts runs without the kit.
- A commit whose files `priory check` would refuse, such as a skill without a `description`, is
  refused whole, as a failed fetch is.

Either way the desk says so, after the intake's banner (see [desk.md](desk.md#intake-banner)):

```text
the kit fails to fetch since 2026-11-21T09:00:00Z: git fetch /srv/abbey.git main: fatal: '/srv/abbey.git' does not appear to be a git repository; runs keep commit 7876b593d4a6
```

The next fetch that works removes the banner.

### Which file wins

The kit's files come first, then the memories repository's, then the factory's:

- Skills and memories: a factory skill or memory hides the kit's of the same name. A memory in the
  memories repository hides the kit's of the same name, and a factory memory hides it in turn. The
  daemon's log names each repository file the factory hides.
- Instructions: a run gets the kit's instructions, a blank line, then the factory's
  `instructions.md`.

The run's plugin folder, memory snapshot and `kit_hash` cover the merged files, as above.

## A memories repository

`[memories]` in `factory.toml` names a git repository whose memories every run gets too (see
[the reference](../reference/factory.md#memories)). A person keeps skills in the kit and memories
here, so the frequent commits of accepted memories stay out of the skills' history, and the two
repositories can be shared with different people:

```toml
[memories]
repo = "git@github.com:owner/scriptorium.git"
ref = "main"
path = "memories"
every = "5m"
```

Each runner fetches it as it fetches the kit, at daemon start and every `every`, with git's own
credentials. Its bare mirror is its own, `$PRIORY_HOME/kit/repos/<hash>.git`, even when it names
the kit's URL. Each commit is checked out read-only into `$PRIORY_HOME/kit/<commit>/`, and checked
again each time the runner loads it, as a kit commit is. A commit with a memory that `priory check`
would refuse is refused whole.

A failed fetch keeps the last good commit, or starts runs without the repository's memories when the
runner has none, and the desk says so after the kit's banner:

```text
the memories repository fails to fetch since 2026-11-21T09:00:00Z: git fetch /srv/scriptorium.git main: fatal: '/srv/scriptorium.git' does not appear to be a git repository; runs keep commit 7876b593d4a6
```

A run gets the kit's memories, then this repository's, then the factory's, then the candidates it
sees, as [Which run gets which candidate](#which-run-gets-which-candidate) orders them. The kit's
own `memories` folder still counts. With `[memories]`, the daily pull request of accepted memories
goes to this repository, onto its `ref`, in place of the kit (see
[Accepted memories move into the kit](#accepted-memories-move-into-the-kit)).

Without `[memories]`, runs get the kit's memories and the pull request goes to the kit, as above.

## What a run records

Each run records `kit_hash`, a hash of the skills, instructions and memories it got, or none when it
got none of them. Both harnesses get all three, so the same files give both the same hash.
`priory runs` shows it in its `kit` column, and the `factory.step.ended`
trace record of a model step carries it (see [the trace schema](../trace-schema.md)). A run carried
into the next step of the same agent keeps its process, so it records the hash and kit commit of
the run it continues.

Two runners whose factory checkouts differ give the same agent's runs different hashes. That is how a
person sees that a runner needs a pull.

With `[kit]`, each run also records `kit_commit`, the kit commit it got, or none when the runner had
none. `priory runs` shows it in its `commit` column, and the `factory.step.ended` trace record of a
model step carries it. With `[memories]`, each run records `memories_commit` the same way, which
`priory runs` shows in its `memories` column. Two runners may run different commits for up to `every`, and this column
shows it.

A shared session compares its agent's prompt digest with the one it started with (see
[sessions.md](sessions.md)). With skills, instructions or memories, that digest covers the hash too, so a
change to any of them reads as a change to the agent's prompt.

## Known limits

- Priory never removes an old plugin folder, memory snapshot, kit or memories repository checkout,
  or `.system.md` file. They are small. Since they are read-only, run
  `chmod -R u+w $PRIORY_HOME/kit` before you remove them yourself.
- `priory check` cannot see a kit's files, so it neither warns about a factory skill that hides a
  kit skill nor counts the kit's instructions or skills toward a Codex agent's 8192-byte limit. A Codex run
  whose merged instructions pass tmux's limit fails to start.
- No test runs a real Claude Code or Codex: that a Claude run lists and uses `priory:<name>`, and
  that a Codex run reads a skill through its index, is checked by hand.
- Nothing keeps the runners' factory checkouts at one commit: a person pulls each one. The kit is
  fetched, but the factory itself is not.
