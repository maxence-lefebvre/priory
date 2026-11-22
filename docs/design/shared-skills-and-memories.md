# Shared skills, instructions and memories

This page explains how R11.1 could work: "Skills, instructions and memories live in git and sync
to every runner." It builds nothing. It holds the model, the first code slice with its tests, the
slices after it, and the open questions with a recommendation for each.

## What existed when this note was written

- The PRD lists `skills/<name>/SKILL.md` in the factory's files, but nothing reads that folder yet.
- A Claude Code run starts with `--append-system-prompt` (the agent's body), `--settings` (Priory's
  hooks and the agent's permissions) and `--mcp-config` (Priory's tools). A Codex run gets the body
  as `developer_instructions`, Priory's hooks and tools, and a sandbox and approval policy mapped
  from the agent's `defaultMode`, all through `-c` overrides. It does not get the agent's
  `permissions` lists. See `ClaudeCode::args` and `Codex::args` in
  `crates/priory-runtime/src/runtime.rs`.
- No run sets `CLAUDE_CONFIG_DIR` or `CODEX_HOME`. So a run reads the person's own `~/.claude` or
  `~/.codex`: their settings, their `CLAUDE.md`, their skills and their login. What a run knows
  depends on whose machine it lands on.
- Claude Code keeps its automatic memory under the config folder, keyed by the working folder. Each
  task runs in its own worktree, so each task starts with an empty memory and its notes stay on one
  machine.
- Each runner has its own checkout of the factory, and reads the shared log
  ([runners.md](../daemon/runners.md)). Nothing keeps those checkouts at the same commit: a person
  pulls each one. Each runner fetches a `[kit]` repository on its own (later slice 2), agents
  write memories on the log (later slice 3), and a daily pull request moves accepted memories into
  the kit (later slice 4).

Max keeps his own kit, `maxence-lefebvre/abbey`, in a private repository: `claude/` (a `CLAUDE.md`
and the docs it points to), `codex/AGENTS.md` and a `skills/` folder. It is the natural first
source.

## The model

### What syncs, and whose it is

Three kinds of files, all per factory:

| Kind | Files | What a run gets |
| ---- | ----- | --------------- |
| Skills | `skills/<name>/SKILL.md` and the files beside it | The skill, loaded by the harness |
| Instructions | one Markdown file, such as `CLAUDE.md` | Its text before the agent's own body |
| Memories | `memories/<name>.md`, one fact per file with a `description` | An index in the prompt, the files on request |

They belong to the factory, not to a person. Runs are the factory's work, and the same task must
behave the same on every runner. A person's own `~/.claude` stays theirs: Priory never reads it
into a run on purpose and never writes to it.

### From where

Two sources, read in this order, the later winning on a name both hold:

1. A kit repository that `factory.toml` names:

   ```toml
   [kit]
   repo = "git@github.com:maxence-lefebvre/abbey.git"
   ref = "main"                        # a branch, a tag or a commit
   skills = "skills"                   # folders inside the repository; each optional
   instructions = "claude/CLAUDE.md"
   memories = "memories"
   every = "5m"                        # how often a runner fetches the ref
   ```

2. The factory's own `skills/`, `instructions.md` and `memories/`. A factory without `[kit]` still
   shares these, because every runner has the factory checkout.

`priory check` refuses a `[kit]` path that leaves the repository, and warns when a factory skill
hides a kit skill of the same name.

### To where

Each runner keeps a checkout of the kit next to its state file, in Priory's own folder:
`<state dir>/kit/<commit>/`, one folder per commit, kept while a run uses it. Nothing goes in
`~/.claude` or `~/.codex`.

A run reaches it through its launch arguments only:

- Skills: Priory writes a plugin folder for the commit, `<state dir>/kit/<commit>/plugin/`, with a
  `.claude-plugin/plugin.json` and the skills under `skills/`, and adds
  `--plugin-dir <that folder>` to the run. Claude Code loads it "for this session only", so no
  other session sees it. Its skills show as `priory:<name>`.
- Instructions: their text goes before the agent's body. tmux refuses a command over about 16 KiB,
  which is why the first prompt already reaches the harness through a file. So Claude Code gets
  instructions and body through `--append-system-prompt-file`, written next to the prompt file.
  Codex gets them in `developer_instructions` only while the two stay under a limit `priory check`
  enforces, until a file-backed Codex setting is confirmed.
- Memories: see [Memories](#memories).

A run records the kit commit it started with, beside its model and harness. Reports and traces can
then group by kit commit, so a skill change can be measured like a model change (R10).

### When

- At daemon start, the runner fetches the ref before it starts any run. A failed fetch keeps the
  last good commit and adds a desk banner. Offline is normal (principle 5), so it never blocks
  runs. A runner with no good commit yet starts runs without the kit and says so on the desk.
- Every `every`, on the daemon's tick, it fetches again. A new commit applies to runs that start
  after it. A live run keeps the commit it started with. A resumed session gets the newer commit:
  its next run is a new launch.
- Later, a GitHub `push` event on the kit repository can trigger the fetch at once, through the
  intake's existing webhook route.

Each runner fetches on its own. Two runners may run different commits for up to `every`. The run's
recorded commit shows it.

### Memories

A memory written on one runner must reach the others. Three ways were weighed:

| Way | For | Against |
| --- | --- | ------- |
| The run commits and pushes to the kit repository | The memory lands in git at once | Every runner needs push rights; two runners race on the branch; an agent pushes text into the instructions of every later run with no review |
| A log message, `priory.memory.written` | Every runner reads the log already, in one order; offline runners catch up; no new credential | The memory is in the log, not git, until something commits it |
| The harness's own memory folder, synced | Nothing new for the agent | It is keyed by worktree path and lives in the person's config folder |

The recommendation is the log, then git through a pull request:

1. An agent saves a memory with a Priory tool, `remember(name, description, text)`. The daemon
   stores it, then publishes `priory.memory.written` with the name, the text, the task and the
   runner, through the `sends` table as claims are. The message id is the write's id: each write is
   an immutable candidate, and a later write of the same name is a new candidate, not an edit.
2. A `forget(name)` tool publishes a removal, which is a candidate too: a tombstone for the name.
3. Every runner records each candidate in log order. A candidate reaches its own task's later runs
   at once. It reaches other tasks' runs only once a person accepts it, by its id, through
   `priory.memory.accepted`. Accepting a candidate never accepts a later write of the same name, so
   unreviewed text never inherits an acceptance. When two accepted candidates share a name, the
   later accept in log order wins, and every runner agrees, because they read one order.
4. A run gets an index of the memories it may see, each name and description, in its instructions,
   and reads a memory's text with a `memory(name)` tool. So a long memory costs nothing until it is
   needed.
5. A daily code step on one runner writes the accepted candidates to a branch of the kit repository,
   a tombstone as a deleted file, and opens one pull request. Once the kit commit that a runner
   fetched holds a candidate's result, the file's text or its absence, the runner drops the
   candidate. A person merges the pull request, and the memory moves into git, as R11.1 asks.

### Conflicts

- Skills and instructions: git resolves them. Each runner checks out a commit, so there is no merge
  on a runner.
- Memories: one name, one file. Each write is its own candidate, and a person accepts one by its
  id. Between two accepted candidates of one name, log order decides. The pull request shows the
  final text against the kit's, so a person sees the conflict once more, where they review.
- A factory file and a kit file of the same name: the factory's wins, and `priory check` warns.

### Secrets

- `factory.toml` names the repository only. The runner fetches with git's own credentials, such as
  an SSH key or a credential helper. Priory reads no token.
- A kit is code that steers every run, like the factory. Point `[kit]` only at a repository the
  factory's owner controls.
- An agent's memory can carry text the agent read in an issue, and a memory reaches every later
  run. That makes a memory a path for a prompt injection to persist. `remember` refuses text over
  4 KB and text that matches common credential shapes. A memory reaches the runs of other tasks only
  after a person accepts it, as above.
- Memories travel on the log. On Postgres, the log leaves the machine, so it holds what the state
  file holds and needs the same care.

### The cloud case

A claude.ai cloud session is not a Priory runner: it has no daemon, no tmux and no log. Abbey's
`cloud-setup.sh` already covers a person's own cloud sessions, and Priory has nothing to add there.
A cloud machine that runs a Priory daemon is an ordinary runner, and fetches the kit as any other
does, with a deploy key that reads the kit repository.

## The first slice: factory skills and instructions reach every run

The smallest change that delivers part of R11.1 uses files every runner already has: the factory's
own `skills/` and `instructions.md`. It adds no fetch, no new message and no new credential.

This slice is built. [skills.md](../daemon/skills.md) describes it. It differs from this section in
three ways: the plugin folder lives in Priory home, `$PRIORY_HOME/kit/<hash>/plugin/`, beside the
prompt files, rather than beside the state file; `priory check` also refuses a skill whose `name`
differs from its folder; and the Codex limit, 8192 bytes, counts the instructions and the agent's
own prompt together.

1. `priory-factory` loads `skills/<name>/SKILL.md`, checks the frontmatter's `name` and
   `description`, and loads `instructions.md` when present. `priory check` reports a skill without
   them, with its file and line.
2. Before a run starts, the daemon writes the plugin folder for the factory's skills under its state
   folder, named by a hash of their contents, unless it exists already.
3. `ClaudeCode::args` adds `--plugin-dir` when the factory has skills, and passes the instructions
   and the agent's body through `--append-system-prompt-file`. `Codex::args` puts the instructions
   before the body in `developer_instructions`, and `priory check` refuses instructions over the
   limit. Codex skills waited for the open question below (later slice 5).
4. The run records the hash of the skills and instructions it got, so two runners whose factory
   checkouts differ show it in the run's record. That hash is how a person sees that a runner needs
   a pull.

Tests:

- `priory check` on a factory with a skill missing its `description` reports the file and line.
- `ClaudeCode::args` for a factory with skills holds `--plugin-dir` and the folder, and the
  instructions file holds the instructions before the body. Without skills or instructions, the
  arguments are as today.
- A 40 KB instructions file starts a Claude Code run in a real tmux server, and `priory check`
  refuses it for a Codex agent with the limit in the message.
- The plugin folder holds `.claude-plugin/plugin.json` and each skill's files, byte for byte, and a
  second run with the same skills writes nothing.
- An example factory gains a skill, so the test that runs `priory check` on each example covers it.
- A real-agent check, by hand, as the operator rules ask: a run lists `priory:<skill>` and uses it.
  Claude Code's plugin naming is the part a unit test cannot see.

## Later slices

1. Reviewed memories from git: the factory's `memories/`, read-only, as an index in the
   instructions and the files in the run's snapshot. No tool writes them yet, so a memory changes
   only through a pull request. This shares memories with no new message and no new risk.
   This slice is built: [skills.md](../daemon/skills.md#memories) describes it. The snapshot is
   `$PRIORY_HOME/kit/<hash>/memories/`, named by a hash of the memories, and a Claude Code run gets
   `--add-dir` on it. `priory check` refuses a memory over 4096 bytes, and counts the index toward a
   Codex agent's 8192-byte limit.
2. `[kit]`: fetch at start and on the tick, the per-commit checkout, the desk banner on a failed
   fetch, and the run's recorded commit. Tests run against a local bare repository.
   This slice is built: [skills.md](../daemon/skills.md#a-kit-repository) describes it. It differs
   from this note in four ways: the mirror and checkouts live in Priory home, under
   `$PRIORY_HOME/kit/`; the kit's instructions come before the factory's rather than giving way to
   them; a kit commit that `priory check` would refuse is refused whole, as a failed fetch is; and
   the daemon's log, not `priory check`, names a factory skill that hides a kit skill, since check
   cannot see the kit.
3. Memories on the log: `remember`, `forget` and `memory`, candidates and acceptance, and the
   `priory.memory.written` and `priory.memory.accepted` schemas. Tests run two runners on one
   herdr-bus file, as the runner tests do, and cover an overwrite after an accept and a forget of a
   name the kit holds.
   This slice is built: [skills.md](../daemon/skills.md#memories-agents-write) describes it. It
   differs from this note in four ways: a person's drop is a third message,
   `priory.memory.dropped`, which also takes the candidate from its own task's runs; an accepted
   candidate overrides the factory's memory of its name too, not only the kit's; the desk item
   lists every candidate that waits, not only the day's; and the `memory` tool reads the memory as
   the task sees it at the call, while the run's snapshot keeps what it saw at its start. A
   person's drop of an accepted candidate retires it, so a wrong accept is not kept for good.
4. The daily pull request of accepted memories to the kit repository.
   This slice is built:
   [skills.md](../daemon/skills.md#accepted-memories-move-into-the-kit) describes it. A schedule
   trigger starts a workflow whose code step runs the `open-memories-pr` action, as open question 3
   recommends. It differs from this note in three ways: any runner that can win the schedule's
   claim needs push rights, since no trigger names a runner; the pull request opens on the
   trigger's `repo`, the kit's GitHub owner/name, since a kit's `repo` can be any git URL; and a
   runner does not yet drop a candidate once the kit commit it fetched holds the candidate's result.
5. Codex skills, as open question 4 answers. This slice is built:
   [skills.md](../daemon/skills.md#a-codex-run) describes it. A Codex run gets the same sealed
   plugin folder as a Claude Code run, and an index of its skills in `developer_instructions`,
   after the memory index: the folder, then each skill's `SKILL.md` path and description.
   `priory check` counts the index toward the 8192-byte limit. Since both harnesses now get the
   skills, their runs record the same `kit_hash`.
6. Skills and memories by path: a `paths` glob in a skill's or memory's frontmatter, so the
   `PreToolUse` hook adds the file to a run's context when it reads, edits or creates a matching
   path, once per run, with no reliance on the model picking it from the index. This slice is
   built: [skills.md](../daemon/skills.md#skills-and-memories-by-path) describes it. Only Claude
   Code runs get it, through the hook's `additionalContext` on the file tools. The daemon records
   each file a run got in its state file, and the hook counts every match as new when no daemon
   answers.

## Open questions

1. **Should runs stop reading the person's own config folder?** Today a run reads `~/.claude`. A
   Priory-owned `CLAUDE_CONFIG_DIR` per runner would isolate runs fully, but every runner would need
   its own login, through `CLAUDE_CODE_OAUTH_TOKEN` or an API key, and each clone's trust again.
   Recommendation: not in R11.1. Keep the session flags, which add the kit and change nothing of the
   person's. Revisit when a factory has more than one person.
2. **How does a person accept memories?** Live memories help most, and a poisoned memory hurts
   most, which is why the model gates other tasks' runs on acceptance. Recommendation: one desk item
   lists the day's new candidates, each with accept and drop, so it costs one look a day, not one per
   memory.
3. **Who opens the memory pull request?** Recommendation: a schedule trigger in the factory, so it
   runs on one owner through the existing claims, with a code step that needs push rights on that
   one runner only.
4. **How do Codex runs get skills?** Codex reads skills from its own home folder. Recommendation:
   check whether a `-c` override can add a skills folder. If none can, list the skill names and
   paths in `developer_instructions`, and leave `CODEX_HOME` alone, for the reason in question 1.
   Answered for Codex 0.170.0: no override can. Codex reads skills from the config folders of its
   user and system layers (`$CODEX_HOME/skills`, `/etc/codex/skills`), `~/.agents/skills`, the
   repository's `.agents/skills`, installed plugins, whose store is under `CODEX_HOME`, and roots
   that an app-server client sets. The layer that `-c` writes adds no skill folder. Writing
   `.agents/skills` into the task's worktree would work, but the agent could commit it. So a run
   gets the index, as recommended.
5. **Per person or per factory?** Recommendation: per factory, as above. A person who wants their
   own kit in runs names it as the factory's kit, which is Max's case with abbey.
