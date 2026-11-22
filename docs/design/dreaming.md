# Dreaming

This page explains how a factory could learn from its own day with nobody watching. Each night, a
workflow reads what the day's tasks concluded and what people answered, and proposes memories. A
person accepts or drops each one the next morning. It also explains how a person's own repository
of skills and instructions serves as the factory's kit, and receives those memories. It builds
nothing. It holds the model, the first code slice with its tests, the slices after it, and the open
questions with a recommendation for each.

## What existed when this note was written

- An agent saves a fact with `remember(name, description, text)` and removes one with
  `forget(name)` (#267, [tools.md](../reference/tools.md#remembername-description-text-forgetname-and-memoryname)).
  Each call is a candidate (`write`, `crates/priory-core/src/memories.rs:88`).
- A candidate reaches its own task's later runs at once, and other tasks' runs only once a person
  accepts it ([skills.md](../daemon/skills.md#which-run-gets-which-candidate)).
- A person accepts or drops a candidate with `priory memories accept` and `drop`
  (`decide`, `memories.rs:404`), or on the web's `/memories` page and the Herdr Desk, which show
  each candidate's full text, the text it replaces and who wrote it (#272). The desk shows one
  `memories` row while any candidate waits ([desk.md](../daemon/desk.md#memory-candidates)).
- `remember` refuses a bad name, a file over 4096 bytes, and a name, description or text that
  matches a credential's shape (`refused`, `memories.rs:35`, and the shapes in
  `crates/priory-core/src/screen.rs:25`). Every runner applies the same rules to each candidate it
  reads from the log.
- `remember` refuses nothing as a duplicate. A second write of the same name and text is a new
  candidate, and so is a write that a person dropped the day before.
- The tool guard keeps some `priory` verbs for a person, such as `answer` and `label`
  (`PERSON_VERBS`, `crates/priory-factory/src/guard.rs:128`). `memories` is not among them. So the
  guard does not stop a run with a shell from running `priory memories accept` on its own
  candidate.
- A daily code step, `open-memories-pr`, writes the accepted candidates as files on the kit's
  `priory/memories` branch and opens one pull request (#270, `open_pull_request`,
  `memories.rs:483`).
- `[kit]` names a git repository whose skills, instructions and memories every run gets (#248,
  [skills.md](../daemon/skills.md#a-kit-repository)). Each runner checks the whole commit out,
  read-only, under `$PRIORY_HOME/kit/<commit>/`.
- The kit's loader refuses a file in the memories folder whose name is not lowercase letters,
  digits and hyphens (`crates/priory-factory/src/factory.rs:2494`), and a refused file refuses the
  whole commit.
- A schedule trigger starts a task at a time of day, and a fixed `key` keeps firings from stacking
  (R2.5). The morning digest starter runs an unattended agent this way, with permissions that deny
  edits and pushes (`examples/morning-digest/`).
- A model step's `timeout` bounds one turn ([health.md](../daemon/health.md)). No key bounds what a
  run spends. The daemon already counts a run's tokens at each turn end (`account`,
  `crates/priory-core/src/trace.rs:44`, called from `state.rs:3544`), and `prices.toml` turns
  tokens into dollars for `priory costs`. Only `priory bench`, `eval` and `score` take a
  `--budget`. Priory reads no Codex tokens.
- Every Claude Code run may call every tool of the `priory` MCP server, `ask` included: the launch
  puts `mcp__priory` first in the run's allow rules (`crates/priory-runtime/src/runtime.rs:1347`).
- `priory trace`, `priory asks --all --json` and `priory task TASK --json` print the day's records.
  They have no size limit, hide no secret, and carry outside text such as issue bodies.

So the tool, the gate and the pull request exist. Missing are the nightly run, a bounded thing for
it to read, a cap on its cost, and two guards.

## The model

### One night

A starter factory, `examples/dreaming/`, holds the whole feature as files:

```toml
# triggers/nightly.toml
source = "schedule"
repo = "owner/abbey"           # any repository: the run changes nothing in it
at = "03:00"
key = "dream"                  # one task a night, and firings never stack
workflow = "dream"
band = "later"
```

```toml
# workflows/dream.toml
start = "dream"
band = "later"

[[steps]]
id = "dream"
decides = "model"
agent = "dreamer"
prompt = "Run `priory recap`. Propose at most 5 memories with remember, then send your result."
effect = "reversible"
timeout = "20m"
budget = "1.00"                # new: dollars this step's run may spend
next = { proposed = "done", nothing = "done", failed = "stop" }
```

```markdown
+++
description = "Reads the day and proposes memories."
harness = "claude-code"
model = "sonnet"
skills = []
permissions = { defaultMode = "dontAsk", allow = ["Bash(priory recap:*)"], deny = ["mcp__priory__ask", "mcp__priory__wait", "mcp__priory__subscribe", "mcp__priory__relate", "mcp__priory__finding", "mcp__priory__report", "mcp__priory__search"] }
+++
```

The workflow has no human step and no irreversible step, so it asks nothing at night. Its only
writes are candidates, which reach no other task until a person accepts them. The band is `later`,
so a dream never starts before real work, and an [urgent wake](urgent-wakes.md) may stop it.

The morning after, the desk's `memories` row lists what the dream proposed. The person opens
`/memories`, reads each candidate beside the text it replaces, and accepts or drops it. The next
firing of the `open-memories-pr` workflow carries the accepted ones to the kit.

### What it may read

The agent reads one thing: the output of a new command, `priory recap`.

```sh
priory recap [--since 24h] [--max-bytes 200000]
```

`priory recap` prints, for the window, oldest first:

| Part | From | Why |
| ---- | ---- | --- |
| Each result: task, workflow, step, outcome, summary, evidence | The `results` table | What tasks concluded |
| Each ask with its answer: question, options, recommendation, the answer and its note | The asks | Where a person corrected an agent or chose against its recommendation |
| Each `say` a person sent | The outbox | A person's corrections in their own words |
| Each failed or blocked task with its reason, and each step's outcome and token count | The tasks and the `factory.step.ended` records | What went wrong, and what was costly |
| The name, and no more, of each pending candidate and of each candidate dropped in the last 30 days, with the date | The memory candidates | What not to propose again |

It never prints:

- A transcript, a run's screen or a prompt file. They are large, and they hold whatever a command
  printed, secrets included.
- The body of an outside event, such as an issue or a comment. That text is untrusted, and the
  day's agents already read it.
- A file of a worktree, an environment variable or the state file itself.

Before it prints, `priory recap` replaces each credential shape with `[redacted]`, with the shapes
`remember` refuses. It stops at `--max-bytes`, keeps the newest records when the day is larger, and
says how many it left out. Each quoted text sits in a fenced block that names its task and its
author, so the agent can tell a person's words from an agent's.

A candidate's name is the one piece of unaccepted text that reaches the dreamer, which is another
task. The rule that a candidate reaches no other task before an accept is about its description
and text, and those stay out. A name is lowercase letters, digits and hyphens, and without it the
dream would propose a dropped memory again every night.

The agent also has what every run has: the index of the memories runs get now, the files of that
snapshot, and the `memory` tool to read one. Its permissions bound the rest in three parts:

- `allow` holds one shell command, `priory recap`.
- `dontAsk` makes Claude Code deny every call that would ask for a permission, without a prompt.
  So no other command runs, and the run never waits on a permission.
- Every run may call the whole `priory` MCP server, so `deny` takes out the tools a dream has no
  use for, `ask` and `wait` first. A dream uses `remember`, `forget`, `memory` and `result`. A
  dream that cannot ask sends `failed` instead, and nothing waits on a person at night.

Nobody has run this agent against a real Claude Code. That its deny rules win over the launch's
allow rule, and that `dontAsk` leaves no file readable beyond the worktree and the memory
snapshot, is a hand check of the first slice.

### The cost cap

Four bounds hold the cost of one night, from input to output:

| Bound | Value in the starter | State |
| ----- | -------------------- | ----- |
| What it reads | `--max-bytes 200000`, about 50,000 tokens | New, with `priory recap` |
| How long one turn runs | `timeout = "20m"` | Exists |
| What its run spends | `budget = "1.00"` | New |
| How many candidates it writes | "at most 5" in the prompt | Prompt only. See open question 3 |

`budget` is a new key of a model step: the dollars the task may spend at that step. At each turn
end, the daemon has just counted the run's tokens. It prices the step's tokens with `prices.toml`,
by the model name the transcript gives, and compares. It does so before the turn's result moves
the task, since a move resets the step's count (`tools.rs:2685`). Then:

| At the turn end | The daemon |
| --------------- | ---------- |
| Under the budget | Goes on as today |
| Over the budget, and the turn sent a result | Lets the result stand, and records the overrun on the step's trace record. The money is spent, and the candidates are already written |
| Over the budget, and the run would go on | Ends the run and fails the task: `step dream spent $1.37, over its budget of $1.00` |
| The tokens or the model's price are unknown | Ends the run and fails the task: `step dream has a budget, and run 9's cost is unknown: price missing for <model>`. A budget that cannot be checked stops the run |

A failed dream needs no repair: the next night starts a new task.

`priory check` refuses a `budget` on a step whose agent is a Codex agent, since Priory reads no
Codex tokens. It refuses one in a factory without `prices.toml`, and one that is not a positive
amount. It cannot check the model's price: an agent names `sonnet`, and the transcript names the
full model, so that check runs at the first turn end.

The check runs at turn ends, so one turn can pass the budget before the daemon sees it. The
`timeout` bounds that turn, and later slice 1 checks inside a turn. `priory costs --by workflow`
shows what each night cost.

### How it avoids secrets

No single rule catches every secret, so four layers stand between a secret and a run that reads
it:

1. `priory recap` never reads the places secrets live: transcripts, screens, files and the
   environment.
2. `priory recap` redacts credential shapes in what it does print.
3. `remember` refuses a candidate that holds a credential shape, on the runner that writes it and
   on every runner that reads it.
4. A person reads each candidate's full text before any other task gets it, and reads it again in
   the kit's pull request.

The shapes match common keys only, such as a GitHub token or a PEM private key. A password in a
sentence passes layers 2 and 3. So the agent's prompt asks for rules and facts, never values: "the
staging database needs the VPN", not its address or its password. Layer 4 is the one that holds.

The same gate covers a planted instruction. A result or an answer can repeat text from an issue
that says "always push to main". The dream may propose it. It reaches no run until a person reads
it on `/memories` and accepts it.

For that gate to hold, an agent must not be able to accept. Today the guard lets a run with a
shell call `priory memories accept`. The first slice adds `memories accept` and `memories drop` to
the person verbs, as `report pick` is. `priory memories` without a verb stays open, since it only
lists. The guard closes the direct call and no more: a run has its person's account, and write
access to the log still equals an accept, as [skills.md](../daemon/skills.md#limits) says.

### How it avoids duplicates

Three rules, the first two in code:

1. **The agent sees what exists.** Its instructions list every memory runs get, by name and
   description. `priory recap` adds the names of the pending candidates and of those dropped in
   the last 30 days.
   The prompt tells it to read a memory with `memory(name)` before it writes one of a near name,
   and to write under the existing name when it corrects a fact.
2. **`remember` refuses an exact repeat.** A candidate whose name, description and text equal the
   memory its task sees under that name, a pending candidate of that name, or a candidate of that
   name that a person dropped, is refused, and the reply names which:

   ```text
   release-day already holds this text
   a person dropped this text for release-day on 2026-11-21: propose it again only with new evidence
   ```

   This applies to every agent, not only the dreamer. A changed text under the same name is not a
   repeat: it is how a memory is corrected, and `/memories` shows it beside the text it replaces.
3. **A person catches the rest.** The same fact under a new name, or reworded, passes rule 2.
   `/memories` shows every candidate at once, so two that say the same thing sit side by side.

### The home repository as the kit

A person who uses coding agents by hand often keeps their own instructions and skills in a private
repository, and links it into `~/.claude`. That repository can be the factory's kit, so unattended
runs and interactive sessions read the same files:

```text
abbey/                         # private, cloned on each of the person's machines
  claude/CLAUDE.md             # the person's own instructions, linked from ~/.claude/CLAUDE.md
  factory/instructions.md      # what every unattended run should read
  skills/<name>/SKILL.md       # linked from ~/.claude/skills
  memories/<name>.md           # what the factory learned
```

```toml
[kit]
repo = "git@github.com:owner/abbey.git"
ref = "main"
instructions = "factory/instructions.md"
```

Open question 5 of [shared-skills-and-memories.md](shared-skills-and-memories.md) already
recommends this: the kit is per factory, and a person who wants their own files in runs names their
repository as the kit. What follows is what that choice needs.

**Instructions.** `[kit] instructions` can point at the person's own `CLAUDE.md`. It should not.
That file speaks to interactive sessions: how to talk to the person, which tools their desk has. A
run on the person's machine reads it anyway through `~/.claude`, so it would get it twice, and a
run on an always-on box should not get it at all. A second file written for unattended runs keeps
both readers served. See open question 5.

**Skills.** Every run would get every skill of the home repository, a standup skill included. The
`skills` list of each agent chooses which ones ([agent-skills.md](agent-skills.md)). On the
person's machine a named skill shows twice, as `<name>` from `~/.claude` and as `priory:<name>`
from the kit. Both hold the same text, so the cost is one more description.

**Memories.** The daily pull request (#270) opens on the home repository: its trigger names
`repo = "owner/abbey"`, the step pushes `priory/memories`, and the person merges. Runs get the
merged memories at the next kit fetch. Interactive sessions get them at the next `git pull` of the
person's clone. Three things stand in the way today:

- Interactive sessions need an index to find a memory, and the memories folder cannot hold one:
  the loader refuses a file named `MEMORY.md` or `README.md` there, and then refuses the whole kit
  commit. Later slice 3 lets the pull request write an index beside the folder.
- A person edits a memory by hand in their own repository, and the edit reaches no run: an accepted
  candidate still overrides the kit's file of its name
  ([skills.md](../daemon/skills.md#limits)). Later slice 2 removes that limit, which matters more
  here than in a kit nobody edits.
- Any runner can win the claim of the pull request's task, so each needs push rights to the home
  repository, or the step fails there. A deploy key with write access to that one repository, on
  each runner, is the least that works.

**What every run can read.** The kit checkout holds the whole commit, not only the three paths
`[kit]` names, and the bare mirror beside it holds the whole repository
(`crates/priory-core/src/kit.rs:331`). Every run on every runner can read every file of the home
repository. Keep no secret and no private note there. Later slice 4 copies fewer files and does
not change that rule.

**Whose facts.** Memories describe the factory's work. A home repository that is personal, under a
factory that works on an employer's code, moves facts about that code into a personal repository.
The home repository as kit fits the PRD's primary user: one person, their machines, their
repositories.

## The first slice: a nightly dream behind the existing gate

This slice is built. [memories.md](../reference/memories.md#priory-recap) describes `priory recap`
and the repeat rule, [health.md](../daemon/health.md#budget) the budget, and
[How to set up a nightly dream](../setting-up-a-nightly-dream.md) the starter. It differs from this
section in four ways:

- Item 3 was on `main` already: `memories accept` and `memories drop` were person verbs before this
  note, so "What existed" is wrong on that point. The slice only adds the listing to the guard's
  tests.
- The starter's agent has no `skills = []`, since an agent's `skills` list
  ([agent-skills.md](agent-skills.md)) is not built.
- `priory recap` finds a person's `say` as a prompt line that starts with `A person says: `, since
  Priory keeps no other record of it. A `say` of several lines shows its first.
- A budget failure ends the run as `budget`. A missing transcript reads `its transcript gave no
  tokens`, and the repeat rule counts a `sending` candidate as waiting for a person.

1. `priory recap [--since D] [--max-bytes N]`, in `priory-core` beside `report.rs`, with the parts,
   the redaction and the limit of [What it may read](#what-it-may-read).
2. `budget` on a model step, checked at each turn end, with the two `priory check` rules of
   [The cost cap](#the-cost-cap).
3. `memories accept` and `memories drop` join the person verbs of the tool guard.
4. `remember` refuses an exact repeat, as [How it avoids duplicates](#how-it-avoids-duplicates)
   says. Each runner applies it to its own writes only: a candidate read from the log is recorded
   as today, since two runners can write the same text before either reads the other.
5. The starter `examples/dreaming/`, with its README, the three files above and the
   `open-memories-pr` trigger and workflow. The test that runs `priory check` on each starter
   covers it.
6. Docs: `priory recap` in [cli.md](../reference/cli.md), `budget` in
   [workflows.md](../reference/workflows.md), the guard's verbs in [guard.md](../daemon/guard.md),
   the repeat rule in [memories.md](../reference/memories.md), and a how-to guide,
   `docs/setting-up-a-nightly-dream.md`, added to `GROUPS` in `site/build.py`.

Tests, through the CLI, with the fake `claude`:

- `priory recap` over a day with two results, an ask answered against its recommendation, a `say`
  and a failed task prints each, oldest first, each text in a block that names its task and author.
- A result whose summary holds a GitHub token prints with `[redacted]` in its place.
- `priory recap` prints no line of a transcript, a run's screen or an issue's body that the day
  held.
- With `--max-bytes` under the day's size, the output stays under it, keeps the newest records and
  ends with the count it left out.
- A pending candidate and one dropped 3 days ago show in the output by name, with no word of their
  description or text. One dropped 31 days ago does not show.
- The starter's agent launches with its deny rules in the run's settings, and `priory check`
  accepts it.
- A `proptest` feeds `priory recap` results and answers with random text, and asserts it never
  panics, never passes `--max-bytes`, and that its output holds no text `holds_secret` matches.
- A schedule firing starts the dream task. Its run calls `remember` twice and sends `proposed`: the
  task is done, both candidates are `pending`, and the desk shows one `memories` row.
- A second firing while the first task still runs is coalesced.
- A run whose step has `budget = "0.01"` and whose first turn costs more, and ends with no tool,
  fails the task with the spent amount and the budget. A run under its budget goes on.
- The same turn, ending with a `result`, moves the task by that result, and the step's trace record
  carries the overrun.
- A turn whose transcript is missing, and a turn whose model `prices.toml` does not price, fail the
  task with `cost is unknown`.
- A resumed run counts the tokens of the step's earlier runs toward the budget.
- `priory check` refuses a `budget` on a Codex agent's step, a `budget` in a factory without
  `prices.toml`, and `budget = "-1"`, each with its line.
- The guard denies a run's `priory memories accept ID` and `priory memories drop ID`, and allows
  `priory memories` and `priory memories --json`.
- `remember` with the name, description and text of a memory the task sees is refused. So is one
  equal to a pending candidate, and one equal to a dropped candidate, each with its sentence. A
  changed text under the same name is stored.
- A candidate the dream wrote reaches no other task's run before an accept, and every task's run
  after it.

## Later slices

1. The budget inside a turn: the daemon reads a live run's transcript on its tick, once a minute,
   for a step with a `budget`, and ends the run when it passes.
2. The kit takes over a name once it holds an accepted candidate's text, so a later edit in the
   kit reaches runs. [skills.md](../daemon/skills.md#limits) lists this as not built. Removing that
   one candidate is not enough: an older accepted candidate of the same name would show again. So
   each name records the accept up to which the kit has taken over, and every candidate and forget
   of the name up to it stops overriding the kit. The candidates stay in the state as history,
   which the repeat rule reads.
3. An index for interactive sessions: `[kit] index = "claude/MEMORY.md"` names a file that
   `open-memories-pr` writes with the pull request, one line per memory with its name and
   description, as the index runs get. The person's `CLAUDE.md` imports it.
4. A kit checkout of the three paths `[kit]` names. It copies fewer files, and it does not hide
   the rest: the bare mirror under Priory home still holds the whole repository, and a run with a
   shell can read it.
5. A code step before the dream that runs `priory recap` and ends the task when the day is empty,
   so a quiet night starts no model. See open question 1.
6. A weekly dream over the memories themselves, which proposes `forget` for a memory no result
   used and a merge of two that say the same.

## Open questions

1. **Should a code step collect the day, or the agent?** A code step costs nothing on an empty
   night, and the agent would need no shell at all. But its output must reach the agent as a file
   outside the worktree, which needs a read permission and a path the prompt names.
   Recommendation: the agent runs the one command, in the first slice. Its permissions allow
   nothing else, and an empty night costs one short turn. Build slice 5 if `priory costs` shows
   empty nights matter.
2. **Should the dream read transcripts?** They hold the most: every wrong turn an agent took.
   They also hold every secret a command printed, and a day of them is millions of tokens.
   Recommendation: no. Results, answers and `say` are what agents and people chose to write down.
   If the dreams prove shallow, add the `progress` and `finding` records before any transcript.
3. **Should the daemon cap the candidates of one task?** The prompt says five, and a model can
   ignore it. Every candidate costs a person a read. Recommendation: prompt only, in the first
   slice. If a night ever floods the row, add `max_memories` to the step, which `remember` checks.
4. **Should an accepted memory skip the pull request, or a pull request skip the accept?** One
   review would be less work than two. Recommendation: keep both. The accept decides what runs get
   tonight. The pull request decides what git keeps, and is the only review of how memories read
   together as files.
5. **One instructions file, or two?** Pointing the kit at the person's `CLAUDE.md` is one file to
   keep. Recommendation: two, as above. The person's file can import the factory's, so shared rules
   live once, and the kit adds only the instructions written for unattended runs.
6. **What does the dream see in a factory of several runners?** One runner wins the task's claim,
   and `priory recap` reads that runner's state file. This note did not check how much of the other
   runners' results and answers that file holds. Recommendation: in the first slice, the dream
   covers the runner that runs it, and its README says so. Check what the log already carries
   before choosing between one dream per runner and one dream over the log.
7. **Should the pull request's task run on one named runner?** Then only that runner needs push
   rights to a private repository. An agent's `runner` key is reserved and read by nothing, and a
   code step has no agent. Recommendation: not for this feature alone. A deploy key per runner
   works today. Revisit when the PRD's runner placement is built.
8. **Should the dream propose skills too?** A repeated procedure is a skill, not a fact.
   Recommendation: no. A skill is instructions with files, and its review is a pull request on the
   kit, which an ordinary task can open. Memories first, and see what people accept.
