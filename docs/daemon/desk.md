# Desk

`priory desk` prints the stall banner first while the queue is stalled (see Room), then what waits on a person, oldest first: since, kind, id, task, lane, what and the task's generation.

| Kind | Id | What | Since |
| ---- | -- | ---- | ----- |
| `ask` | the ask | its question, then its options | it was asked |
| `blocked` | the task | its reason | the task last changed state |
| `failed` | the task | its reason, then `revive: priory revive <task>` | the task last changed state |
| `prompt` | the outbox prompt | its text, for an `uncertain` prompt | it was attempted |
| `held` | the hold | the task's reason: why it is held, and until what | it was held |
| `memory` | the memory grant | `waits for <N> GB: <command>` | it was requested |
| `queue` | the task's place in the queue | its band, then its reason when it has one | the task last changed state |
| `sweep` | the script's run | its command, then `running`, or its status and output, for the open stall | it started |
| `retired` | the task | `its worktree was removed to free disk; it resurrects from <branch> when its wait fires` | it retired |
| `worktree` | the worktree's path | why it stays: see Sweep | `-` |

The lane column is the task's lane, or `-` for a `worktree` row.

It lists only asks and prompts of active tasks, holds of held tasks and waiting memory grants. Then the `queue` rows in start order, then one `worktree` row per worktree the sweep keeps. `priory asks` still prints every open ask.

`priory resume <task>` makes a blocked task `ready`. It queues the prompt `A person resumed this
task after: <reason>. Check the worktree, then continue.` ahead of its other queued prompts, so its
next run resumes the step agent's session with it. If its latest run never reached `SessionStart`, or
it has none, the reason concerns the launch, not the agent's work, so the prompt is only
`A person resumed this task. Check the worktree, then continue.` A resume line that no run took yet is
replaced, not repeated. The line that ends the step is added only when no queued prompt carries it
already. `priory cancel <task>` drops an active task with the reason `cancelled from the desk`, ends its
live run as `cancelled` and kills its tmux session. Both go to the daemon over `priory.sock`, as
`priory answer` does, and are refused for a task in any other state. An answer to an ask of a done,
failed or dropped task is refused too.

## Revive

`priory revive <task> [--step <step>]` restarts a failed or dropped task as its next generation. A
task starts at generation 1. The revive goes to the daemon over `priory.sock` and, in one
transaction:

1. Withdraws every open ask and wait of the task, recording the new generation on each.
2. Raises the task's generation by one.
3. Enters the step it ended on, or `<step>`, as a move into it would (see Moving on): a model step
   makes the task `ready` with its prompt queued, a human step asks again, and a code step makes it
   `ready`.
4. At a model step, queues ahead of that prompt: `A person revived this task as generation <n>
   after it was <state>: <reason>. The asks and waits of its earlier generation are withdrawn. Check
   the worktree, then continue.`

It replies `task <id> is <state> at <step>, generation <n>`. It refuses a task in any other state, a
step the task's workflow lacks, and a task whose key another active task holds, naming that task. A
factory holds at most one active task per key.

The task keeps its id, key, branch, worktree and sessions. Its next run resumes the step's agent
session, like a resume. The agent keeps what it learned in the earlier generation, and the revive
prompt tells it that the asks and waits it remembers are gone. A new session would lose that context,
and the worktree already holds the earlier generation's work. When the sweep removed the worktree, the
next run adds it back from the branch (see Priory home). A revive straight into a code step, such as
`open-pr`, adds it back before the action runs.

A withdrawn ask or wait stays in the state file but counts as closed. An answer to one is refused with
`ask <id> was withdrawn when task <task> was revived as generation <n>`. An event for a withdrawn wait
fires nothing, and the task keeps it like any event that matches no wait, for the next run to read.
`priory asks`, `priory waits` and the desk leave withdrawn ones out. A new run never takes over a
withdrawn ask or wait.

A failed task is one its repair could not restart (see Repair).
