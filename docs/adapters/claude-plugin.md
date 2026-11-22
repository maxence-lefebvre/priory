# The Claude Code plugin

A person's own Claude Code session is not a run. Priory did not start it, it has no task, and the
person sits in front of it. The plugin in `adapters/claude-plugin/` lets that session read the
desk, ask the person a question through the desk, and add a task. This page explains what the
plugin holds and why a model in it cannot answer an ask. To install it, follow
[How to use Priory from your own Claude Code session](../using-priory-from-your-own-session.md).

## Two actors in one terminal

A person's session holds two actors: the person types, and the model decides tool calls. An answer
is an approval, and only a person approves (see [guard.md](../daemon/guard.md)). A model that reads
an ask's text could be talked into answering it, and that text is an agent's. So the plugin gives
the model the tools to read and to ask, and nothing that answers:

| Act | Who may do it | How |
| --- | ------------- | --- |
| Read the desk, the asks, a task | the model and the person | the `desk`, `asks` and `task` tools, `/priory:desk` |
| Add a task | the model and the person | the `add` tool, `/priory:add` |
| Ask the person a question that outlives the session | the model | the `ask` tool, `/priory:ask` |
| Answer or dismiss an ask, or any other person verb | the person only | the desk, or a line the person runs |

## What the plugin holds

| File | What it does |
| ---- | ------------ |
| `.claude-plugin/plugin.json` | The manifest. The plugin is named `priory`, so its skills and command are `/priory:...` |
| `.mcp.json` | Starts [`priory mcp --outside`](../daemon/tools.md#the-outside-server), which serves `desk`, `asks`, `task`, `ask` and `add` |
| `hooks/hooks.json` | Runs [`priory hook PreToolUse --outside`](../daemon/tools.md#the-outside-hook) before each Bash call |
| `skills/desk/SKILL.md` | Reads the desk and says what needs the person, most urgent first |
| `skills/ask/SKILL.md` | When the work is blocked on the person's decision, calls `ask` with a question, options, a recommendation and a trade-off, then goes on |
| `skills/add/SKILL.md` | Turns a follow-up from the conversation into a task with one line of text |
| `commands/answer.md` | `/priory:answer ASK`: shows the ask and prints the line that answers it |

A skill is also a slash command, so the three skills are `/priory:desk`, `/priory:ask` and
`/priory:add`. Claude Code's docs say a command or skill takes the same frontmatter, and that
`disable-model-invocation: true` keeps Claude from invoking it on its own. `/priory:answer` sets
it, and grants no tool permission.

## Why the answer stays the person's

Three things keep an answer the person's, and each covers what the one before it does not:

- The server serves no `answer` or `dismiss` tool. A call to one is an unknown tool.
- `/priory:answer` only prints a line, such as `! priory answer --revision 2 14 approved`. The
  person reads it and runs it themselves. Only the person can invoke the command.
- The hook denies a shell call the model makes to a `priory-person` verb, such as
  `priory answer` or `env priory dismiss`, with the line the person can run instead.

The hook ships even though the person could approve the call in a permission prompt. Many people
run with permission prompts off, and the approval rule should not depend on a session's
permission mode. It applies the `priory-person` guard alone: `gh pr merge` in a person's own
session is the person's business. Like the run's guard, it is not a sandbox, for the reasons
[guard.md](../daemon/guard.md#how-a-command-is-read) gives.

Claude Code's documentation does not say whether a line the person types with the `!` prefix
passes through `PreToolUse` hooks. If it does, the hook denies the person's own `! priory answer`
line too, and the person runs it in another terminal.

Each result of `desk`, `asks` and `task` starts with a line that marks the rest as data, and the
skills repeat it. That is a request to the model, not a control: the missing tools and the hook
are the controls.

## How the answer comes back

The person answers on the desk or with `priory answer`, as for any ask. The `ask` tool's reply
names whether the answer will reach the session:

- In an hc terminal with a Claude session, in a factory with `[asks] deliver`, the Herdr `deliver`
  script types the answer into the terminal, or resumes the session once the terminal closed.
- In an hc terminal with no Claude session id, it types the answer while the terminal stays open.
- Otherwise the answer is stored only, and the session reads it with the `asks` tool.

The `ask` tool never blocks. Where delivery is set up it wakes the session; elsewhere the skill
says to check once when the person comes back, never in a loop.

## Tasks a session added

The `add` tool asks for a task as [`POST /api/request`](../reference/serve.md#writes) does. The
task records the session that added it as `added_by`: the hc terminal and Claude session ids, or
`outside`. The desk's Since you looked names the session on the task request's line, so the person
sees work a session added without them.

## Which factory it reaches

A person's session runs in any folder. The server reaches the daemon behind `PRIORY_HOME` and
`PRIORY_DB`, as every listing command does, and refuses to start when either is unset. It refuses
to start in a run, where `PRIORY_SESSION` is set: a run has its own server.

## What the plugin is not

- Not a way to run a task. A session that wants work done calls `add`, and a run does it.
- Not a second desk. The web view and the Herdr panels stay the place to answer.
- Not a way for a run to gain tools. `--outside` and `--session` never mix.

The design note [priory-plugin.md](../design/priory-plugin.md) lists the slices after this one:
`search` and `todo`, a Codex setup, and delivery beyond hc.
