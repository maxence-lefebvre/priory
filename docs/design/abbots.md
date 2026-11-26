# Abbots

This page explains how a factory could give a person named secretary agents, called abbots. An
abbot has a name and a face, works one lane, owns the triggers that start its work, and shows a
person one panel: what needs them, what it closed, and what it will do next. A person talks to it
in plain words. It builds nothing. It holds the model, the first code slice with its tests, the
slices after it, and the open questions with a recommendation for each.

The idea comes from another tool's secretary agent, a named bot with an avatar whose panel lists
what needs you, what it closed and its automations, and whose items each carry an activity thread.
Almost every part of that already exists in Priory under other names. An abbot gathers them.

## What existed when this note was written

- An ask carries a question, options, a recommendation, a trade-off, an optional `text` and a
  title (`ask`, [tools.md](../reference/tools.md#askquestion-options-recommendation-tradeoff-confidence-rationale-title-text-whatwaits-nextaction-command)).
  Every open ask also takes a free-text answer: the web view's ask detail has "Write an answer"
  ([DESIGN.md](../../DESIGN.md#ask-detail), item 6).
- A trigger names its tasks' `lane` and `labels`
  ([triggers.md](../reference/triggers.md#keys-every-trigger-takes), lines 39 and 40). A label is a
  `key=value` pair on a task, and `priory add --label` and `priory request --label` set one on a
  task a person or an agent adds ([cli.md](../reference/cli.md), lines 67 and 69).
- A schedule trigger fires at a time of day, and `priory schedules --json` prints each one's next
  firing ([schedule.md](../intake/schedule.md#next-firing)). The web view shows firings only as
  entries of the Desk's Up next strip, the next 3 runs of any kind, and of the Queue panel
  ([serve.md](../reference/serve.md#up-next)). No view lists the triggers themselves. A trigger of
  any other source has no next firing: it fires on an event.
- An agent with `session = "shared"` keeps one conversation across every task it serves, one turn
  at a time ([sessions.md](../daemon/sessions.md#shared-sessions)). A `hands-off` trigger's `lead`
  is such an agent, and it answers asks in the person's place, except an ask whose answer lets an
  irreversible step run and one whose answer runs a command
  ([triggers.md](../reference/triggers.md#intervention-and-lead)).
- `priory say TASK TEXT` sends a task's agent a message and wakes it if it sleeps
  ([cli.md](../reference/cli.md), line 142). The task page has a "Say to the agent" box
  (`Say`, `crates/priory-core/web/pages/TaskPage.jsx:79`). `say` is a person verb: the tool guard
  denies it to a run (`PERSON_VERBS`, `crates/priory-factory/src/guard.rs:132`), as it denies
  `answer`, `dismiss`, `cancel`, `label` and `memories accept`.
- `GET /api/tasks/{id}` returns a task's runs, asks, results, waits and inbox, each with its time
  ([serve.md](../reference/serve.md#task-in-detail)). The task page draws them as separate
  sections: open asks, actions, Say, answered asks, progress
  ([DESIGN.md](../../DESIGN.md#task-detail-and-workflow-grid)). Nothing draws them as one thread
  in time order.
- The Claude Code plugin lets a person's own session read the desk, ask through it and add a task.
  It answers or dismisses an ask only when the person tells it to
  ([adapters/claude-plugin/README.md](../../adapters/claude-plugin/README.md)). It has one command,
  `answer`, and three skills, `desk`, `ask` and `add`.
- Memories are one flat folder per factory, kit or memories repository. A memory's file name is
  lowercase letters, digits and hyphens, with no subfolder
  (`crates/priory-factory/src/factory.rs:3771`). Every run gets the index of every memory
  ([skills.md](../daemon/skills.md#memories)). An agent's `skills` key narrows the skills its runs
  get ([skills.md](../daemon/skills.md#skills-per-agent)); no key narrows memories.
- A nightly dream proposes memories from the day with `priory recap`, and a person accepts each
  one ([dreaming.md](dreaming.md), [examples/dreaming/](../../examples/dreaming/)). Accepted
  candidates reach the kit or memories repository through a pull request
  ([shared-skills-and-memories.md](shared-skills-and-memories.md#memories)).
- The PRD lists "A chat interface." among its non-goals ([prd.md](../prd.md#non-goals)).
- No file, view, command or doc names an abbot.

So the parts exist: asks with options, labels, lanes, schedules, shared sessions, `say`, and
memories behind a person's accept. Missing are a file that ties them to one name, a panel that
shows them together, a thread for one item, a way to talk to the abbot, and memories of its own.

## The model

### An abbot is a file

An abbot is `abbots/<name>.toml` in the factory. It adds no new runtime: it names things the
factory already has and tags the work they start.

```toml
# abbots/wren.toml
name = "Wren"                  # what views show; the file name is its id
avatar = "abbots/wren.png"     # a PNG or SVG in the factory, at most 64 KB, square
description = "Keeps my week tidy: inbox, follow-ups, memory."
agent = "wren"                 # agents/wren/agent.md, with session = "shared"
lane = "me/secretary"
triggers = ["look-around", "tidy-memory", "follow-ups"]
```

`priory check` reads it with these rules:

- `agent` names a `claude-code` agent with `session = "shared"`. The abbot's conversation is that
  session, so the abbot remembers what it did across items without a new store.
- Each trigger in `triggers` exists and belongs to at most one abbot. A trigger that names its own
  `lane` must name the abbot's.
- `avatar` is a file under the factory, PNG or SVG, at most 64 KB. An SVG is served as an image,
  never inlined, so it runs no script.

Each task one of its triggers starts gets the abbot's lane and the label `abbot=<id>`. A task the
abbot's agent adds with `priory add` gets them too: the daemon sets them from the run that added
it. The label is the one fact every view reads. An item of an abbot is a task with that label.

### Its panel

One panel, `abbot`, with the abbot's id as its argument. The web view serves it at
`/abbots/<id>`, and a Herdr mod draws the same data. It lists, top to bottom:

1. **Header**: the avatar, 40 px, in a square with `--radius-md`, the name in the display role,
   and "1 needs you" in `--count`, or "Nothing needs you" in `--text-2`.
2. **Needs you**: the open asks and stopped tasks of its items, oldest first, as the Desk's rows.
3. **Closed**: its items done, failed or dropped in the last 7 days, newest first, at most 5,
   with "Show all" opening the Tasks view filtered on its label.
4. **Automations**: its triggers, each with an icon for its source, its name and, for a schedule,
   its next firing ("16:00", "Mon 09:00"). Another source reads what it waits for: "on a Sentry
   issue". A held or paused trigger says so.
5. **Say to Wren**: from [slice 3](#later-slices) on, a text box at the foot, with the hint "Ask
   Wren to run something on a schedule or when something happens." Before that slice the panel
   has no foot.

`GET /api/abbots/{id}` returns `{id, name, avatar, description, lane, agent, needs, closed,
automations}`, each list in the row shape the Desk and the Queue already use, so the Herdr mod and
the web view read one endpoint.

The panel answers nothing itself: a row opens the ask detail or the task page, as the Desk's rows
do. The Desk keeps every item. An abbot's panel is a filtered window on it, never a second inbox.

### An item's thread

The task page gains an **Activity** section: one list, oldest first, that merges the arrays
`GET /api/tasks/{id}` already returns.

| Entry | From | Drawn as |
| ----- | ---- | -------- |
| Added | the task's creation | "Wren added this item", or the trigger's name, with the time |
| Message | an ask's `text`, a result's summary, a `note` from [slice 3](#later-slices) | the abbot's avatar and name, the text as sanitized Markdown |
| Needs you | an open ask | the question, the options as buttons, the recommended one outlined, and Write an answer |
| Answer | an answered ask | who answered, the option, the time |
| Said | a person's `say` | the person's words, right under the entry they answered |
| Event | an inbox event | one line in `--text-2`: "GitHub pull request merged" |
| Closed | the final result | the outcome word and its state tile |

An open ask in the thread follows [Ask detail](../../DESIGN.md#ask-detail): options in the agent's
order, the recommended one outlined in `--recommended` with "Recommended" under its label, full
width on a phone and in one row on a wide screen. The source design marks the recommendation with
a filled star chip; DESIGN.md fills an option only while it is armed, and DESIGN.md wins.

### Talking to an abbot

The PRD rules out "a chat interface". This note proposes to narrow that, because a person who
names an agent will want to tell it things, and `say` already does it for one task. Talking to an
abbot is `say` to one standing task.

- Each abbot has a standing task, key `abbot:<id>`, with a built-in workflow of one model step run
  by the abbot's agent. It sleeps on a wait for `say` and for its triggers' firings, so it costs
  nothing while idle.
- A person talks to it from the panel's Say box, from `priory say abbot:wren TEXT`, or from their
  own Claude Code session with a new plugin skill, `/priory:abbot wren TEXT`, which runs that
  `say` only with the person's own words, as `answer` already works.
- The abbot replies in the standing task's thread with a new tool, `note(text)`, which records one
  message and ends nothing. When it needs a decision, it asks, as every agent does.

What the abbot may do from a chat, and what stays with a person:

| Act | How | Who decides |
| --- | --- | ----------- |
| Open an item | `priory add --label abbot=wren` from its run | the abbot; spend caps on its triggers and lane bound it |
| Answer a question about its items | reads `priory tasks --lane me/secretary --json` and replies with `note` | the abbot |
| Add or change a trigger | asks with the trigger's TOML as `text`; on the person's answer, a code step `write-trigger` writes `triggers/<name>.toml` once `priory check` passes, and adds it to the abbot's file | the person, always: the step is `irreversible`, so no lead, autopilot or `auto_answer` answers it |
| Answer another item's ask | only as that item's `lead`, under the existing `hands-off` rules | a person for every irreversible step and every command ask |
| Accept a memory, cancel, hold, label | never | the guard's person verbs deny them to its run |

The abbot's agent cannot write the factory's files: its `permissions` deny the factory folder,
and `priory check` refuses an abbot whose agent allows it. A trigger reaches the factory only
through the ask and the code step.

### Memory of its own

An abbot needs two kinds of memory. Its conversation is its short-term memory: the shared
session, compacted by `compactAt` as any lead's is. Its facts, such as "Max reads Slack at 11:00
and 16:00", are memories, behind the same accept as every other.

- A memory whose name starts with `<id>-`, such as `wren-slack-hours`, belongs to the abbot. Its
  runs get it in their index, and no other run does. This keeps the flat folder and the loader's
  name rule.
- `remember` from a run of the abbot's agent prefixes the name with `<id>-` when the name lacks
  it. A person accepts or drops the candidate on `/memories`, which groups candidates by abbot.
- The abbot's "Tidy memory" automation is a dream ([dreaming.md](dreaming.md#one-night)) scoped to
  its label: `priory recap --label abbot=wren` reads only its items, and its candidates carry its
  prefix. A factory with several abbots runs one dream each, in band `later`.
- Accepted candidates reach the kit or memories repository by the existing daily pull request
  ([shared-skills-and-memories.md](shared-skills-and-memories.md#memories)), as files named with
  the prefix. A person reviews them there.

## The first slice: an abbot file and its panel, read-only

1. `abbots/<name>.toml` in `priory-factory`, with the rules of [An abbot is a file](#an-abbot-is-a-file).
2. The daemon sets the abbot's lane and the `abbot=<id>` label on a task its triggers start.
3. `priory abbots [DIR] [--json]` prints each abbot with its counts and automations, and
   `GET /api/abbots/{id}` and `GET /api/abbots/{id}/avatar` serve the panel.
4. The `abbot` panel on the web view, at `/abbots/<id>`, from the design Max approves, and a rail
   item per abbot under the default view's panels, which is a row under More on a phone.
5. Docs: `abbots/` in [factory.md](../reference/factory.md) and the PRD's Configuration block,
   `priory abbots` in [cli.md](../reference/cli.md), the endpoints in
   [serve.md](../reference/serve.md), the panel in [views.md](../reference/views.md) and DESIGN.md.

No chat, no thread, no memory and no Herdr mod in this slice. It shows whether one panel per abbot
earns its place before anything writes.

Tests, through the CLI and the web view's tests:

- `priory check` accepts an abbot whose agent is shared and whose triggers exist, and refuses,
  each with its file and line: an unknown agent, a `per-task` agent, a Codex agent, an unknown
  trigger, a trigger two abbots name, a trigger whose `lane` differs, an avatar over 64 KB, an
  avatar outside the factory, and an avatar that is neither PNG nor SVG.
- A schedule firing of one of its triggers starts a task with the abbot's lane and label.
- `priory abbots --json` lists an open ask of its item under `needs`, a task done 2 days ago under
  `closed` and not one done 8 days ago, and each trigger with the `next` that `priory schedules`
  prints for it.
- A task without the label never shows in any abbot's lists.
- `GET /api/abbots/{id}/avatar` serves an SVG with `Content-Type: image/svg+xml` and a
  `Content-Security-Policy` that allows no script.
- The panel's test renders the header count, the three sections, and each section's empty
  sentence.
- The test that runs `priory check` on each starter covers a new `examples/abbot/`.

## Later slices

1. **The thread.** The Activity section on the task page, from the arrays the endpoint returns,
   and the Herdr `priory-abbot` mod with the panel and the thread.
2. **The standing task.** `abbot:<id>` and its built-in workflow, woken by `say` and its
   triggers.
3. **Chat.** The `note` tool, the panel's Say box, and the plugin's `/priory:abbot` skill.
4. **Triggers from a chat.** The `write-trigger` code step, its ask, and the guard rule on the
   factory folder.
5. **Memory per abbot.** The name prefix, its index filter, `remember`'s prefix,
   `priory recap --label`, and `/memories` grouped by abbot.
6. **Avatars elsewhere.** The avatar beside an abbot's items on the Desk and in notifications,
   once a person has more than one abbot.

## Open questions

1. **A file of its own, or keys on the agent?** `abbot = { name, avatar, lane, triggers }` in the
   agent's frontmatter would add no folder. But an agent is a launch recipe that several workflows
   share, and an abbot is a face a person sees. Recommendation: its own file, which may name only
   a shared agent.
2. **Should each item run in the abbot's shared session?** Then the abbot remembers every item,
   but its items run one at a time, and one long item blocks its chat. Recommendation: no. Items
   run the agents their workflows name, in parallel. The shared session serves the standing task
   and any step whose `agent` is the abbot's, and it reads its items through `priory tasks`.
3. **Does the PRD change?** Recommendation: yes, narrow the non-goal. Replace "A chat interface."
   with "A general chat assistant. A person talks to an abbot about its factory's work through
   `say` and asks, and every irreversible step still waits on a person's answer." Add a Concepts
   row: "Abbot | A named agent with an avatar that works one lane, owns its triggers and talks to a
   person through say and asks". This pull request makes both edits for review.
4. **Where does chat live first, the web view or the plugin?** The plugin costs one skill and
   reaches a person at their desk. The Say box reaches them on a phone, which is the M2 test.
   Recommendation: both in slice 3, since each is a `say` and neither adds a server path.
5. **May an abbot add a trigger without a person?** A trigger spends money on every firing.
   Recommendation: never. The step is `irreversible` so no lead, autopilot or `auto_answer` can
   take it, and the ask shows the trigger's TOML as `text`.
6. **What does a closed item mean?** Recommendation: done, failed or dropped in the last 7 days.
   A failed item shows its state tile and stays in Needs you while it waits for a revive ask.
7. **One avatar format?** Recommendation: PNG or SVG, square, at most 64 KB, served from the
   factory. A missing avatar draws the name's first letter on `--raised`.
