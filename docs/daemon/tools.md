# Tools

An agent talks to Priory through MCP tools. Every run gets one MCP server, `priory`, which serves
them. With these tools, an agent ends a step, asks a person a question, or sleeps until an event.

## The MCP server

The run's settings start the server as `priory mcp --session <id>`, a stdio MCP server. It answers
`initialize`, `ping`, `tools/list` and `tools/call`.

The server sends each tool call to the daemon over `priory.sock` as
`{"tool": ..., "session": ..., "arguments": ...}`. The daemon applies the call in one transaction
and replies with the tool's text or error. With no daemon listening, the call fails and stores
nothing.

## The tools

Three tools end a turn: `result`, `ask` and `wait`. A run holds at most one of a result, an open
ask and an open wait. Any later call of the three is refused.

### `result(outcome, summary, evidence?, commit?, reason?)`

`result` stores the run's result and ends the step. Nothing can change or delete a stored result.

The outcome must be a key of the step's `next`, or the error lists the valid outcomes. For a step
with `next = { passed = "approve-pr", failed = "stop" }`, a call looks like this:

```json
{
  "outcome": "passed",
  "summary": "Fix the typo in the README\n\nThe install section said `cargo biuld`.",
  "evidence": "cargo test passed: 112 tests.",
  "commit": "3f9c2a7e5b1d4c6f8a0e2b4d6f8a1c3e5b7d9f0a"
}
```

An outcome that leads to `done` needs `commit` when the task owns a pull request (see Done and
checks in [waits.md](waits.md)). `commit`, like a checks wait's `head_sha`, must be a full SHA,
because GitHub names pull request heads by their full SHA. An abbreviated SHA is refused. A SHA in
capitals is stored in lower case.

The `open-pr` action titles its pull request with the first line of `summary` (see Code steps
below).

#### Grades

A model step whose outcome is `red`, `yellow` or `green` grades the task. When the step ends, the
task takes the outcome as its grade and one line as its grade reason: the first line of `reason`,
or of `summary` when the result has no `reason`. A later grading step overwrites both. Every grade
stays in the step's result and in its `factory.step.ended` record, which carries a `grade` field
(see [trace-schema.md](../trace-schema.md)).

`priory tasks` prints the grade and its reason in the `grade` and `grade_reason` columns, and
`priory tasks --grade green` lists only the green tasks.

### `ask(question, options, recommendation, tradeoff)`

`ask` puts a question on the desk and stores it as an ask at revision 1. The agent then ends its
turn and sleeps with no process. The person's answer arrives as its next prompt (see
[waits.md](waits.md)). For example:

```json
{
  "question": "The issue asks for a new flag, but `--format` already covers it. Which do you want?",
  "options": ["add the flag", "document --format"],
  "recommendation": "document --format",
  "tradeoff": "A new flag is easier to find, but adds a second way to do the same thing."
}
```

### `wait(on)`

`wait` stores a wait on one condition, then the agent ends its turn and sleeps with no process.
The event that satisfies the condition arrives as its next prompt. For example, to sleep until a
pull request's checks finish:

```json
{ "on": { "type": "checks", "repo": "owner/name", "pull_request": 57 } }
```

[waits.md](waits.md) lists every condition.

### `compact(focus?)`

`compact` queues `/compact <focus>` in the outbox and tells the agent to end its turn. An agent
cannot type `/compact` itself, since only typed input reaches its harness. A Codex run gets a bare
`/compact`, because Codex's `/compact` takes no argument.

A task holds at most one queued compact, so a second call is refused. `result` is refused too until
the compact is typed, because a result could move the task to another agent's conversation first.
`ask` and `wait` resume the same conversation, so the compact follows their answer or event.
Compacting is described under Compact in [outbox.md](outbox.md).

### `tldr(text)` and `search(query, states?, limit?)`

`tldr` replaces the task's TL;DR, for when the work drifted from the request. `search` finds tasks.
Neither ends the turn, and either can come after a result, an ask or a wait. `priory mcp` answers
both itself from the state file, without the daemon. See [search.md](search.md).

## Moving on

When the run's turn ends, its `Stop` moves the task by its result to `next[outcome]`:

| Target     | Task                                                 | Run                       |
| ---------- | ---------------------------------------------------- | ------------------------- |
| model step, same agent | at that step, `running`, with its prompt queued | `moved`, into a new run |
| model step, other agent or irreversible | at that step, `ready`, with its prompt queued | `moved` |
| human step | at that step, `asleep`, with an ask of its `options` | `asleep`                  |
| code step  | at that step, `ready`: see Code steps                | `moved`                   |
| `done`     | `done`, or `asleep` on its checks: see Done and checks | `done`, or `asleep`       |
| `stop`     | `dropped`                                            | `dropped`                 |

A prompt that a move or an answer queues goes ahead of the prompts already queued for the task and
carries them after it. So an event kept during the last turn never reaches the agent before its new
instructions.

A `ready` task's next run starts the step's agent with its own system prompt, model and settings.
It resumes that agent's session for the task, or starts a new one if the agent has not run for it
yet.

A run with an open ask or wait and no result ends `asleep`, and its task sleeps. The daemon kills
the run's tmux session when the run ends, after the hook has its reply. The worktree stays until a
sweep removes it.

## Code steps

A code step runs its `action` in the daemon, with no agent and no run. Each loop, the daemon takes
the oldest `queued` or `ready` task at a code step and runs its action outside any transaction. One
transaction then records the action run and moves the task by `next[status]` (see the table under
Moving on). An action that is not built yet blocks the task with `action <name> is not built yet`.

`open-pr` is the one built-in action:

1. It pushes the commit of the task's branch, the one its approval covers, to that branch on the
   clone's `origin`.
2. It reuses the open pull request from that branch if `gh pr list` finds one.
3. Otherwise it runs `gh pr create -R <repo> --base <default branch> --head <branch>`. The title is
   the first line of the latest result's `summary`. The body is the `summary`, then its `evidence`.

Its status is `opened` or `failed`. A failure records the error, which a human step reached by
`next.failed` shows in its question. On `opened`, the task owns the pull request and its events
route to the task. A pull request that another active task owns fails instead, and that task is
named. If the task then reaches `done` and its latest result names no `commit`, the checks wait of
Done and checks pins the commit that `open-pr` pushed.

If the daemon stops after `gh pr create` and before recording it, the action runs again at its next
start, once the approval still holds. The push repeats and the open pull request is reused. If
someone closed that pull request meanwhile, a second one opens.

### Approvals

An irreversible step, code or model, runs only when an answer approved it at the task's current
commit, unless it sets `autonomous`. A step's ask approves each step in its `approves` that
`next[option]` leads to. The daemon records the approval with the commit of the task's branch at
the answer. A run never carries on into an irreversible model step, so each one starts a new run.
Before the step runs, or before that run starts, the daemon compares the approval's commit with the
branch's current commit:

| Approval                       | Effect                                                              |
| ------------------------------ | ------------------------------------------------------------------- |
| at the same commit             | the step runs                                                       |
| at another commit              | the ask gets a new revision, with a line naming both commits, and the task sleeps at that human step. The step's queued prompt is taken back, so the next run never reads it |
| none                           | the task blocks: `step <id> is irreversible and no answer approved it` |

A branch with no commit to compare also blocks the task. `priory check` refuses an irreversible step
that a path reaches with no approving human step. So the `none` row needs a factory changed while
the task ran.

`priory actions` prints each action run: id, task, step, action, status, pull request, pushed commit
and error.
