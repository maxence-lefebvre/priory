# Tools

`priory mcp --session <id>` is a stdio MCP server. It answers `initialize`, `ping`, `tools/list`
and `tools/call`. It sends each tool call to the daemon over `priory.sock` as
`{"tool": ..., "session": ..., "arguments": ...}`. The daemon applies the call in one transaction
and replies with the tool's text or error. With no daemon listening, the call fails and stores
nothing.

- `result(outcome, summary, evidence?, commit?)` stores the run's result. Nothing can change or
  delete it. The outcome must be a key of the step's `next`, or the error lists the valid outcomes.
  An outcome that leads to `done` needs `commit` when the task owns a pull request (see Done and
  checks). `commit`, like a checks wait's `head_sha`, must be a full SHA, because GitHub names pull
  request heads by theirs. An abbreviated SHA is refused. A SHA in capitals is stored in lower case.
- `ask(question, options, recommendation, tradeoff)` stores an ask at revision 1.

- `wait(on)` stores a wait (see Waits).
- `compact(focus?)` queues `/compact <focus>` in the outbox and tells the agent to end its turn. An
  agent cannot type `/compact` itself, since only typed input reaches its harness. A Codex run gets a
  bare `/compact`, because Codex's `/compact` takes no argument. A task holds at most one queued
  compact, so a second call is refused, and so is `result` until it is typed: a result could move
  the task to another agent's conversation first. `ask` and `wait` resume the same conversation, so
  the compact follows their answer or event. Compacting is described under Compact in Outbox.
- `tldr(text)` replaces the task's TL;DR, for when the work drifted from the request. `search(query,
  states?, limit?)` finds tasks. Neither ends the turn, and either can come after a result, an ask or
  a wait. `priory mcp` answers both itself from the state file, without the daemon. See
  [search.md](search.md).

A run holds at most one of a result, an open ask and an open wait. Any later call is refused.

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
