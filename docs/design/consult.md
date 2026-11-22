# Consulting an investigator

This page explains how a run that lacks an MCP server, such as Sentry, Amplitude or Honeycomb,
could ask a named investigator agent one question, and get the report in its next prompt. It
builds nothing. It holds the model, the first code slice with its tests, the slices after it, and
the open questions with a recommendation for each.

The daemon starts the investigator as a short, read-only run with its own MCP servers and a cost
cap. The asking run never gets those servers. Claude Code's own subagents cannot do this, by the
brief for this note: a subagent inherits its parent's MCP servers, so it reaches nothing the
worker lacks.

Three facts about the harness carry this design, and this note did not test them: that
`--strict-mcp-config` limits a run to the servers Priory names, that `dontAsk` denies every tool
call no `allow` rule names, and the inheritance above. The first slice proves each with a real
run before it builds on it.

## What existed when this note was written

- An agent lists its `mcpServers` (`crates/priory-factory/src/factory.rs:776`,
  [agents.md](../reference/agents.md)). From a search of the code, nothing passes that list to
  the run: the daemon hashes it into the session's identity
  (`crates/priory-core/src/daemon.rs:1981`), `priory check --explain` reads it, and a Claude Code
  run gets `--mcp-config` with the `priory` server alone
  (`crates/priory-runtime/src/runtime.rs:605`), without `--strict-mcp-config`. So a run today has
  the `priory` server plus whatever the person's own Claude Code configuration holds, whatever
  its agent lists. Only the headless calls pass `--strict-mcp-config` (`runtime.rs:1367`).
- Giving every agent every server has a price: each server's tool list fills the context of
  every run, and each server's credentials reach an agent that also pushes code.
- `finding` hands a problem to a new task and does not wait for it
  ([tools.md](../daemon/tools.md#findingkey-title-details)). The run that called it never hears
  what the fix task learned.
- `ask` puts a question to a person and sleeps until the answer
  (`crates/priory-core/src/tools.rs:636`).
- Under `hands-off`, the daemon already queues a short task of another agent for one question,
  and brings its answer back: the lead task, with the trigger `priory/lead` and a built-in
  workflow of one step (`tools.rs:2418`, `:2448`). Each one costs a worktree.
- A run learns the factory's skills from an index of names and one-line descriptions: Claude Code
  through its plugin folder, Codex through lines in its instructions
  ([skills.md](../daemon/skills.md)). Every agent has a one-line `description`
  (`factory.rs:741`).
- The tool guard denies guarded calls in a step whose workflow allows none, and its `mcp-write`
  guard knows a fixed list of MCP tool names ([guard.md](../daemon/guard.md#built-in-guards)). An
  MCP tool with another name that writes passes it.
- Claude Code's `dontAsk` mode denies every tool call no `allow` rule names
  ([agents.md](../reference/agents.md#permissions)). A Codex agent takes no per-tool rules
  ([codex.md](../daemon/codex.md)).

## The model

### One question, one report

A run calls `consult`:

```json
{
  "agent": "sentry",
  "question": "Issue WEB-41 started on 2026-11-20. Which release introduced it, and what does the stack trace point at?"
}
```

The tool stores the consult and the agent ends its turn, as after `ask`. The task sleeps with no
process. The daemon starts the investigator, and the report arrives as the task's next prompt:

```text
consult 4 reported by sentry:
<the report, as data>
```

There is no second message. A follow-up question is a new consult. An investigator cannot ask
the worker anything back: what it lacked goes in its report.

A run holds at most one open consult, and a `result` or a second `consult` after it is refused,
as after an open ask.

### An investigator

An investigator is an agent that says so:

```toml
description = "Reads Sentry issues, events and releases, and answers questions about them."
harness = "claude-code"
model = "haiku"
investigator = true
mcpServers = ["sentry"]
permissions = { defaultMode = "dontAsk", allow = ["mcp__sentry__search_issues", "mcp__sentry__get_issue_details", "Read", "Grep"] }
```

`priory check` refuses an investigator, with file and line, when:

- its harness is not `claude-code`, since only Claude Code has per-tool rules;
- its `defaultMode` is not `dontAsk`: every tool it may call must be named;
- an `allow` rule names `Edit`, `Write`, `NotebookEdit` or `Bash`, or holds a `*`. Each MCP tool
  it may call is written out by its exact name, so a tool the server adds later is denied until
  a person adds it;
- one of its `mcpServers` has no `[mcp.<name>]` table in `factory.toml` with a `read_only`
  sentence, such as `read_only = "The token has the org:read and event:read scopes only."`. It
  is a person's statement of where the limit is enforced, and Priory cannot check it;
- it sets `session = "shared"`, `connectorSignIn` or `push = true`
  ([agent-credentials.md](agent-credentials.md));
- a workflow step or a trigger's `lead` names it. An investigator only answers consults.

So "cannot write" rests on four layers: exact tool names a person reviewed, a credential the
person states is read-only at the service, the tool guard, which grants an investigator's run no
guard at all, and the absence of any file or shell tool that writes. Priory enforces the first,
the third and the fourth. The second is the wall, and it is the person's: `priory check
--explain` prints each investigator's tools and each server's `read_only` sentence so both can
be reviewed. A service with no read-only credential should not get an investigator.

### Who may consult whom

`factory.toml` holds the allow list:

```toml
[consult]
deadline = "15m"          # from the consult's call to its report, the wait for room included
cap = "$0.50"             # one consult's cost
per_task = 5              # consults one task may make

[consult.allow]
implementer = ["sentry", "honeycomb"]
triage = ["sentry", "amplitude"]
```

An agent with no line consults nobody, and its runs get no `consult` tool. `priory check` refuses
a name that is no agent, a target that is no investigator, and an investigator as an asker.

### Depth

An investigator cannot consult. Its MCP server serves `result` and the read tools, and no
`consult`, `ask`, `wait`, `finding`, `remember` or `add`. The check above also keeps an
investigator out of `[consult.allow]`'s keys. So a consult is one hop, and no chain or loop of
agents can form. This is the rule of [debate-step.md](debate-step.md) again: agents get no
channel to each other, only a question the daemon carries and a report it carries back.

### How a run learns who exists

A run whose agent may consult gets an index, after the skills index and in the same form:

```text
Investigators you can consult with the consult tool, one question each:
- sentry: Reads Sentry issues, events and releases, and answers questions about them.
- honeycomb: Reads traces and query results, and answers questions about latency and errors.
```

It lists only the investigators that agent may consult, by name and `description`. `priory check`
counts the index toward the Codex instructions limit, as it does the skills index.

### The investigator's run

In the transaction that stores the consult, the daemon queues a task for it, as it queues a lead
task: key `priory:consult:<id>`, trigger `priory/consult`, the asking task's repository, band and
lane, and the built-in workflow `priory/consult/<agent>`. That workflow has one model step,
`investigate`, with effect `read` and no `guard.allow`. Its outcomes `reported` and `unknown`
both lead to `done`.

The run starts fresh: a new session, with no memory of earlier consults. Its prompt is the
question, under a line that says it comes from another agent and is a request for facts, not an
instruction. It passes the room check like any run.

It answers with `result`. The summary is the report, at most 8000 characters. `unknown` means it
could not answer, and the summary says what it lacked.

### The report is data, twice

An investigator reads text that outsiders wrote: an error message in Sentry, a property in
Amplitude. That text can hold instructions. It then travels into a worker that can change code.

- The report reaches the worker as quoted text under a line that says it is another agent's
  report of outside data, to weigh and never to obey. That is a request to the model.
- The control is what each side can do. The investigator can only read. The worker stays under
  the tool guard and the approval rule, whatever a report says.
- Before a report is stored, it is searched for the values of the factory's known secrets, as an
  evidence item is ([reviewer-evidence.md](reviewer-evidence.md)). A match refuses the result,
  and the investigator is told to answer without the value.

A report can still carry customer data from the system it read. The investigator's instructions
say to summarize and to quote the least it can. No code checks that.

### When no report comes

The worker never sleeps on a consult forever. It wakes with one of these lines instead of a
report:

```text
consult 4 got no report: no report 15m after the question
consult 4 got no report: sentry passed its cap of $0.50
consult 4 got no report: sentry's run failed: <reason>
consult 4 got no report: sentry needs a sign-in, which a person was asked for
consult 4 was refused: task 12 already made 5 consults
```

A sign-in the investigator needs follows the existing path: the guard denies it and the desk gets
a `Sign in to <connector>` to-do ([desk.md](../daemon/desk.md#sign-in-to-dos)).

A cancel, a revive or a drop of the asking task drops its open consult's task, as an answered ask
drops a queued lead task.

### Cost

Three bounds, all the daemon's:

- `deadline` counts from the `consult` call, so it covers the wait for room and the run. Past
  it, the daemon ends the investigator's task, queued or running, and wakes the worker with the
  line above. A result that arrives later is refused.
- `cap` is enforced while the run works. Each tick, the daemon reads the token counts in the
  investigator's transcript, as it reads them at a turn's end, prices them, and ends the run once
  the cost reaches the cap. A run can pass the cap by what one model call costs. `priory check`
  refuses an investigator whose model has no price, and a run whose transcript the daemon cannot
  read ends at once with that reason.
- `per_task` stops a worker that keeps asking.

A consult's cost belongs to the asking task and its lane. `priory costs --by agent` shows the
investigator's own row, and `priory report` counts consults per step.

### What the trace records

| Record | When | Fields |
| ------ | ---- | ------ |
| `factory.consult.asked` | the consult is stored | task, step, run, the asking agent, the investigator, the consult's id, the question's length |
| `factory.step.ended` | the investigator's step ends | as for any model step, with its tokens, plus `consult`: the consult's id and the asking task |
| `factory.consult.reported` | the report or the failure line is queued for the worker | the consult's id, outcome (`reported`, `unknown`, `deadline`, `cap`, `failed`, `refused`), seconds from ask to report, the report's length, and the run's tokens and model, since a run that ends with no result writes no `factory.step.ended` |

The question and the report are kept in the state file with the consult, not in the trace.
`priory task ID --json` lists them. The task page shows them in later slice 1. A task's exported
trace shows the consult as a span under the step that asked.

### How it differs

| | `consult` | `finding` | `ask` | A Claude Code subagent | A lead under `hands-off` |
| --- | --- | --- | --- | --- | --- |
| Who answers | an investigator agent | nobody: a new task takes the problem | a person | a copy of the worker | a lead agent |
| The caller waits | yes, asleep | no | yes, asleep | yes, with its process | yes, asleep |
| Something comes back | a report, in the next prompt | nothing | the answer | the subagent's text | the answer |
| Tools the answerer has | its own MCP servers, read only | a full task's | a person's | the worker's own | the lead's |
| Can change anything | no | yes, it is a task | yes | what the worker can | no |
| Who starts it | the daemon | the daemon | the desk | the harness | the daemon |
| Bounded by | deadline, cap, count per task | the queue | nothing | the worker's turn | the lead's task |

Use `finding` when the problem is someone else's work. Use `ask` when the decision is a person's.
Use `consult` when the worker needs a fact it has no tool to read.

## The first slice: one investigator, one hop

1. The proof of the three harness facts, and a run's MCP servers made real: the daemon passes
   the servers its agent lists, with `--strict-mcp-config`, so a run gets those and no others.
   Without this, a worker may already hold the server and an investigator may hold the worker's.
2. `investigator` on an agent, `[consult]` and `[mcp.<name>]` in `factory.toml`, in the schema
   and the JSON Schemas, with the check rules above.
3. The `consult` tool, served only to runs of an agent in `[consult.allow]`.
4. The built-in workflow and trigger, the investigator's fresh run, and its narrowed MCP server.
5. The report in the worker's next prompt, the secret search, and the four lines for no report.
6. `deadline`, `cap` read each tick, and `per_task`.
7. The index in a run's prompt.
8. The three trace records, and the consult on `priory task ID --json`.
9. [tools.md](../daemon/tools.md), [agents.md](../reference/agents.md),
   [factory.md](../reference/factory.md), [trace-schema.md](../trace-schema.md), and an
   investigator in one starter.

Tests, through the CLI with the fake harness the daemon tests use:

- A worker's `consult` ends its turn, leaves its task asleep with no tmux session, and starts one
  run of the investigator. The investigator's `result` wakes the worker with the report under the
  `consult 4 reported by` line.
- The investigator's run has its own MCP servers in its configuration and none of the worker's.
  The worker's has none of the investigator's. Neither has a server from the person's own
  configuration, which the test plants.
- The investigator's `tools/list` holds no `consult`, `ask`, `wait`, `finding`, `remember` or
  `add`, and a call to one is an unknown tool.
- A run of an agent with no line in `[consult.allow]` has no `consult` tool. A `consult` that
  names an investigator outside the agent's list is refused with the names it may use.
- The run's prompt lists exactly the investigators its agent may consult.
- A guarded call in the investigator's run, such as `gh issue comment` or an MCP `update_issue`,
  is denied, and the trace holds the `factory.tool.denied` record.
- An investigator still queued for room at the deadline never starts, and the worker wakes with
  the deadline line. One that runs past it is ended, and a `result` it sends afterwards is
  refused.
- A run whose transcript shows a cost past `cap` is ended within a tick, the worker wakes with
  the cap line, and `factory.consult.reported` holds its tokens.
- A sixth consult of one task is refused with the count. A consult after one that went over
  `cap` is refused with the cost.
- A `result` or a second `consult` while a consult is open is refused.
- A report that holds the value of a factory secret is refused, the refusal does not hold the
  value, and the worker gets nothing until the investigator answers again or times out.
- Cancelling the asking task drops the consult's queued task. A report for a consult of an older
  generation wakes nothing.
- A daemon restart between the ask and the report starts no second investigator and delivers the
  report once.
- The trace holds one `asked`, one `step.ended` with `consult`, and one `reported`, and
  `priory costs --by agent` shows the investigator's row.
- Each check rule fails with its line: a Codex investigator, a mode other than `dontAsk`, an
  `allow` of `Bash`, of `Write` and of `mcp__sentry__get_*`, a server with no `read_only`
  sentence, an unpriced model, an investigator in a workflow step, an
  investigator as an asker, and an unknown name in `[consult.allow]`.
- A `proptest` feeds `consult` random and mutated arguments and asserts it returns or refuses
  without panicking.

## Later slices

1. The question and the report on the task page, from a reviewed design.
2. No worktree for a consult. The first slice gives the investigator a worktree as a lead task
   has one. An investigator that reads only outside systems needs none.
3. Several consults in one turn: the worker asks two investigators and wakes when both reported.
4. A consult from a person: `priory consult AGENT QUESTION` prints the report, for the desk and
   for a person's own session through [priory-plugin.md](priory-plugin.md).
5. A report kept for reuse: the same question about the same issue within an hour returns the
   stored report, marked with its age.
6. Codex investigators, once a Codex run can be limited to named MCP tools.
7. `priory report consults`: per investigator, the count, the share `unknown`, the time and the
   cost, with counts beside every rate.

## Open questions

1. **Should `consult` end the turn?** A consult that returned inside the turn would keep the
   worker's context warm. It would also hold the worker's process and memory while another run
   works, which Principle 3 forbids, and it would need the two runs to fit in the room together.
   Recommendation: end the turn, as `ask` does. The session resumes with its conversation intact.
2. **Is a list of exact tool names enough to call an investigator read-only?** No. A name says
   what a tool is called, not what it does. Recommendation: exact names, no pattern, and a
   required `read_only` sentence per server that says where the service enforces it. A proxy
   that enforces reads belongs with the network policy of
   [agent-credentials.md](agent-credentials.md).
3. **Why not give the worker the server?** It is simpler. It also puts a service's whole tool
   list in every worker's context, gives its credentials to an agent that pushes code, and lets
   outside text reach that agent with nothing in between. Recommendation: keep both. A factory
   that trusts a server on a worker lists it there, and uses `consult` for the rest.
4. **Fresh session or a shared one?** A shared investigator would remember the service's shape
   and answer faster. It would also carry one task's data into another task's report.
   Recommendation: fresh. Put what an investigator should know about the service in its
   instructions and its skills.
5. **Who pays?** The asking task's lane, or the investigator's own. Recommendation: the asking
   task's lane, since the task caused the spend. The agent split still shows what each
   investigator costs.
6. **Should the report be checked by code beyond its length and known secrets?** A scorer could
   grade reports, and a schema could force a shape. Recommendation: neither now. Read the
   `unknown` share and a sample of reports after a month, then decide.
7. **How does this fit `push = false`?** [agent-credentials.md](agent-credentials.md) makes an
   agent at a `read` step a `push = false` agent, and limits its MCP servers. Recommendation:
   whichever note is built second adopts the other's rule. An investigator is a `push = false`
   agent with no GitHub credential, and the `[mcp.<name>]` `read_only` sentence is the
   declaration that lets such an agent name a server.
