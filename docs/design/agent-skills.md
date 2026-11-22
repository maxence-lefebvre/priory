# Skills per agent

This page explains how an agent could name the skills its runs get. Today every run gets every
skill of the factory and of the kit. The PRD's Concepts table already lists skills among what an
agent is: "harness, model, effort, MCP servers, skills, prompt, session". It builds nothing. It
holds the model, the first code slice with its tests, the slices after it, and the open questions
with a recommendation for each.

## What existed when this note was written

- A factory's `skills/<name>/SKILL.md` reach every run on every runner (#203,
  [skills.md](../daemon/skills.md)). A `[kit]` repository adds its own, and a factory skill hides
  the kit's skill of the same name (#248).
- One function merges them for a run: `sources` takes the kit's skills, then the factory's, with
  no agent in its arguments (`crates/priory-core/src/kit.rs:48`). The daemon calls it once per run
  start (`crates/priory-core/src/daemon.rs:1838`), and once for all the shared sessions it compacts
  while idle (`daemon.rs:2085`).
- A Claude Code run gets the merged skills as a plugin folder, `$PRIORY_HOME/kit/<hash>/plugin/`,
  through `--plugin-dir` (`plugin`, `kit.rs:160`, and `runtime::plugin`,
  `crates/priory-runtime/src/runtime.rs:868`). The hash covers the skills' names, paths and bytes,
  so two runs with the same skills share one folder.
- A Codex run gets the same folder, and an index of it in `developer_instructions`: one line per
  skill with its `SKILL.md` path and description (#276, `launch`, `daemon.rs:2034`, and
  `skill_index`, `crates/priory-factory/src/factory.rs:2112`). Codex 0.170.0 has no flag that adds
  a skills folder.
- `priory check` counts that index toward a Codex agent's limit of 8192 bytes for its
  instructions, its prompt and its indexes (`CODEX_PROMPT_LIMIT`, `factory.rs:1742`). It counts
  every factory skill, for every Codex agent. It checks the size only when the factory has
  instructions, memories or skills (`crates/priory-factory/src/rules.rs:787`), so a prompt over
  the limit passes in a factory with none.
- A skill's name is its folder's name. The loader asks for no more than a `name` equal to the
  folder and a `description` (`factory.rs:2437`), so `review_notes` is a skill name today.
- Each run records `kit_hash`, which covers the skills, instructions and memories it got. A shared
  session reads a changed hash as a change to its agent's prompt
  ([skills.md](../daemon/skills.md#what-a-run-records)).
- The agent's frontmatter has no `skills` key (`Agent`, `factory.rs:739`), and it refuses unknown
  keys, so `skills = [...]` is an error today ([agents.md](../reference/agents.md)).
- A run still reads the person's own `~/.claude` or `~/.codex`, and the `.claude/skills` or
  `.agents/skills` of the repository it works in. Priory adds its skills beside those and removes
  nothing.
- `priory check` cannot see a kit's files, so it knows the factory's skill names and not the
  kit's.

So the factory decides which skills exist, and no agent can narrow them.

## The model

### Why narrow

Every skill costs every run something, whether the run uses it or not:

- Claude Code puts each plugin skill's name and description in the session's context, so the
  model can pick one. Thirty skills are thirty descriptions in every run, of every agent.
- A Codex run pays the same in bytes, against a hard limit: the index shares 8192 bytes with the
  instructions and the agent's prompt. A factory with many skills leaves a Codex agent little room
  for its own prompt, or fails `priory check`.
- A skill is an instruction. A reviewer that sees a `release-notes` skill may follow it. The fewer
  skills an agent sees that are not its work, the fewer wrong turns it takes.
- A kit that is a person's own repository holds skills written for that person's interactive
  sessions, such as a standup. No unattended run should get them
  ([dreaming.md](dreaming.md#the-home-repository-as-the-kit)).

### The key

An agent's frontmatter gains `skills`, a list of skill names:

```markdown
+++
description = "Reviews one pull request."
harness = "claude-code"
skills = ["review-comments", "writing-tests"]
+++
```

| `skills` | The run gets |
| -------- | ------------ |
| Unset | Every skill, as today |
| A list | Only the skills it names |
| `[]` | No skill: no plugin folder and no index |

The list filters the merged set. A name stands for whichever skill a run would get under it today:
the factory's when the factory has it, else the kit's. An agent cannot ask for the kit's copy of a
skill the factory hides, and it has no reason to.

The list holds exact names. It takes no pattern and no exclusion (see open question 1).

The list narrows what Priory adds. It does not touch the skills a harness finds by itself: the
person's own, and the repository's. Open question 2 covers those.

### A name nobody holds

`priory check` knows the factory's skills and cannot see the kit's. So:

- In a factory without `[kit]`, `priory check` refuses a name that is no skill of the factory, with
  the agent's file and the line of `skills`:

  ```text
  agents/reviewer/agent.md:4: `skills` names `review-coments`, which is not a skill of this factory: review-comments, writing-tests
  ```

- In a factory with `[kit]`, a name the factory lacks may be the kit's, so `priory check` accepts
  it.
- At a run's start, a name that neither the factory nor the runner's kit commit holds gives no
  skill, and the run starts without it. The daemon logs
  `agent reviewer names skill review-coments, which neither the factory nor kit commit 7876b593d4a6 holds`.
  Blocking the task would break a rule the kit already follows: a failed fetch never blocks runs,
  and a runner with no kit commit yet has none of the kit's skills.

`priory check` also refuses a name that appears twice, and an empty name. It adds no rule on
what a name looks like: any name the loader accepts for a skill's folder can be in the list.

### How it reaches a Claude Code run

`sources` takes the run's agent and keeps only the skills it names, at a run's start and when the
daemon compacts an idle shared session, which launches the agent too. Everything after it works on
the filtered set with no change:

- The plugin folder holds only those skills. Its hash covers only them, so each distinct set of
  skills has one folder, and agents with the same list share it.
- The run gets `--plugin-dir <that folder>`, and Claude Code lists the skills as `priory:<name>`.
  With an empty set, the run gets no `--plugin-dir`, as a factory without skills gives none today.
- `kit_hash` covers the filtered skills. Two agents whose lists give different skills record
  different hashes, which is correct: they got different files. The hash covers the files and not
  the list, so a reordered list, or a name nobody holds, changes nothing.

### How it reaches a Codex run

The same filtered folder, and an index that lists only its skills. Two things follow:

- The index is shorter, so a Codex agent with a short list keeps more of its 8192 bytes for its
  prompt. `priory check` counts only the factory skills the agent names. It still cannot count a
  kit skill, which stays a known limit of [skills.md](../daemon/skills.md#known-limits).
- With an empty set, the run gets no `# Skills` section at all.

### A shared session

A shared session's digest covers `kit_hash`. So a change to the skills its agent gets reads as a
change to the agent, and `onChange` applies: Priory tells the resumed session what changed, or
starts a fresh one ([sessions.md](../daemon/sessions.md#when-the-agent-changes)). No new rule is
needed, as long as every path that computes the digest filters the same way. The idle compaction
is the second such path: with the unfiltered set there, a restricted agent's digest would never
match and its session would never compact.

### What it is not

- **Not a permission.** A run with a shell can read any file under Priory home, the other plugin
  folders included. The list keeps a skill out of the agent's context. It does not hide a secret,
  and a skill must not hold one.
- **Not a filter on memories or instructions.** Every run still gets the instructions and the
  memory index. A memory costs one line until an agent reads it, and the instructions are meant for
  every agent (see open question 4).
- **Not a way to add a skill.** A skill exists because the factory or the kit holds its folder. An
  agent names it, and cannot bring its own.

## The first slice: `skills` on an agent

This slice is built, with each open question's recommendation taken:
[skills.md](../daemon/skills.md#skills-per-agent) describes it. It differs from this section in one
detail: without a kit commit yet, the daemon's log says the runner has none rather than naming a
commit.

1. `Agent` gains `skills: Option<Vec<String>>`, in the JSON Schema and in
   [agents.md](../reference/agents.md).
2. `priory check` applies the rules of [A name nobody holds](#a-name-nobody-holds), and counts only
   the named factory skills toward a Codex agent's limit. It checks that limit whether or not any
   instructions or index remain, which closes the gap above.
3. `sources` takes the agent and filters the merged skills, in the run start and in the idle
   compaction. The daemon logs each name nobody holds, once per run start.
4. Docs: a section in [skills.md](../daemon/skills.md) after "A kit repository", and the key in the
   review starter's agent, which has one skill today.

No migration, no new flag and no new dependency: the plugin folder, the index and the hash already
follow the set they are given.

Tests, through the CLI with the fake `claude` and the fake `codex`, as the tests of #203 and #276
read a run's arguments:

- In a factory with skills `a` and `b`, an agent with `skills = ["a"]` starts a Claude run whose
  `--plugin-dir` folder holds `skills/a` and no `skills/b`.
- An agent with no `skills` gets both, in the folder a run got before this change, with the same
  `kit_hash`.
- An agent with `skills = []` starts a run with no `--plugin-dir`, and a Codex agent with it gets
  no `# Skills` section.
- Two agents with the same list share one plugin folder. Two agents with different lists get two
  folders and record two `kit_hash` values.
- A Codex agent with `skills = ["a"]` gets an index with one line, for `a`.
- A Codex agent whose prompt and full index pass 8192 bytes fails `priory check`. With a list that
  leaves the largest skill out, it passes.
- With a kit that holds `c`, an agent with `skills = ["c"]` gets the kit's `c`. When the factory
  also has a `c`, the run gets the factory's.
- Without `[kit]`, `priory check` refuses an unknown name with the file and line, and lists the
  factory's skills. With `[kit]`, it accepts the name.
- With `[kit]` and a name nobody holds, the run starts, gets the other named skills, and the
  daemon's log names the missing one.
- `priory check` refuses `skills = ["a", "a"]` and `skills = [""]`, and accepts a list that names
  a skill whose folder is `review_notes`.
- A Codex agent with `skills = []`, in a factory with no instructions and no memories, whose own
  prompt passes 8192 bytes fails `priory check`.
- Priory tells a shared session whose agent's skills changed so at its next run, with
  `onChange = "tell"`. A reordered list tells it nothing.
- An idle shared session whose agent has a `skills` list, and whose agent did not change, still
  compacts, and its compaction launch gets the same plugin folder as its runs.

## Later slices

1. Kit names in `priory check`: `priory check --kit DIR` loads a kit checkout beside the factory,
   so a name nobody holds is an error before a run, and a kit skill counts toward the Codex limit.
   It would lift two known limits of [skills.md](../daemon/skills.md#known-limits) at once.
2. `skills` on a workflow step, which narrows the agent's list for that step. See open question 3.
3. What each agent gets, in a listing: the skills, by name and source, in `priory flows --json`
   and on the web's Flows panel, from a design a person reviewed.

## Open questions

1. **Patterns and exclusions?** `skills = ["review-*"]` or `["!standup"]` would be shorter in a
   factory with many skills. A pattern makes the set depend on what the kit holds that day, so a
   new kit skill reaches an agent with no change to the factory. Recommendation: exact names only.
   The list is short by design, and a pull request that adds a name is the review.
2. **Should the list also cover the skills a harness finds by itself?** A run on the person's
   machine still gets the person's own skills, and the repository's. Claude Code's permission
   rules can deny a skill by name, and an agent's `permissions.deny` already passes rules through,
   but Priory would have to list the person's folder to write them, and nobody has checked the
   rule's syntax by hand. Recommendation: no. The list covers what Priory adds. Full isolation is
   open question 1 of [shared-skills-and-memories.md](shared-skills-and-memories.md), a
   Priory-owned config folder, and belongs there.
3. **Per step as well?** One agent can serve a plan step and an implement step, which want
   different skills. A step-level list changes the plugin folder between two steps that share a
   process, and the process cannot reload its plugins. Recommendation: not yet. Two agents with two
   lists do the same today, at the cost of one more session.
4. **Should memories filter the same way?** Recommendation: no. A memory is one index line, and the
   agent reads the file only when its description fits. The cost that justifies a filter is not
   there, and a memory that matters to every agent would have to be named in every list.
5. **Should a missing kit skill block the task?** A run that starts without a skill its agent
   names may do worse work, silently. Recommendation: no block, as above, since offline is normal.
   If it proves a problem, add the missing names to the kit's desk banner, which already tells a
   person when the kit fails to fetch.
6. **Should Codex get the skills as real skills?** Writing the filtered skills into the worktree's
   `.agents/skills` would list them in Codex's own picker. The agent could commit them, which is
   why #276 chose the index. Recommendation: keep the index. The filter makes it short enough.
