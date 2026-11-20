# Runs

A run is one task step worked by one agent's `claude` process. A model step that leads to another
model step with the same agent carries the process on into a new run, so each run has at most one
result. A model step with another agent gets a new run and a new process (see Moving on). Each
second, the daemon starts a run for each `queued` or `ready` task whose step decides `model`, in
queue order, if it fits in the room (see Queue and Room). It works in the task's repository: its
task request's `repo`, else its trigger's, stored when the task is created. A task stored before
tasks kept a repository uses its trigger's.

Before that, each tick checks every `queued` and `ready` task's step. A step its workflow no longer
has blocks the task with `step <id> is not in workflow <name>`. A human step with no open ask gets
its ask and the task falls `asleep`, as a move into the step does, so a workflow can start at a human
step. A held task gets its ask once its hold releases it.

The daemon creates the worktree, records the run and task as `running`, then starts the agent's
harness in a detached tmux session named `priory-<task id>`, in the worktree. An agent with
`harness = "claude-code"` runs `claude` (below). One with `harness = "codex"` runs `codex` (see
Codex). Each step starts its own agent's harness, so one workflow can mix both.
`claude` gets:

- `--session-id`: a UUID Priory chooses and stores, so a later run can resume the conversation. A
  task keeps one session per agent. When an earlier run of the step's agent reached `SessionStart`,
  the run gets `--resume <that id>` instead, in the same worktree. An agent with
  `session = "shared"` keeps one session across tasks (see Shared sessions).
- `--model` and `--effort`: the agent's, when it sets them.
- `--append-system-prompt`: the agent body, the Markdown after its frontmatter.
- `--permission-mode`: the agent's `permissions.defaultMode`, or `manual` when it sets none, whatever
  the user's own settings choose. In `manual` mode, every permission request reaches Priory's deny
  hook (see Health). In `auto` mode, Claude's auto mode decides the run's permission prompts.
  `priory check` refuses `bypassPermissions` and accepts `default` as Claude's older name for
  `manual`.
- `--settings`: hooks that run `priory hook SessionStart`, `UserPromptSubmit` and `Stop`; a
  permission for the `priory` MCP server's tools plus the agent's own `allow` and `deny` rules, in
  Claude Code's syntax; a `PreToolUse` hook that denies `AskUserQuestion` and tells the agent to
  call `ask` instead; and the permission hooks under Health.
- `--mcp-config`: the `priory` MCP server, `priory mcp --session <id>` (see Tools).

The same tmux command tags the new pane with a pane option `@priory` set to the harness session
id, so a pane is never left untagged. The daemon types only into that pane, even when a person
opens other panes in the session. Each second, it checks every live run's health (see Health). A
git or tmux error ends the run as `failed`, or blocks the task when no run exists yet.

tmux owns the agents, so the daemon can stop and start again without stopping them. On start, it
keeps every live run whose session still exists. It kills each tmux session whose tagged pane
carries the harness session of an ended run that no live run continues in that tmux session. A
crash between ending a run and killing its session leaves such sessions.

A live run whose pane is gone when the daemon starts was lost with its tmux server, by a reboot or
a killed server, so the agent is not at fault. Unless its result, ask or wait ended the turn (see
Health), the run ends `lost` and its task becomes `ready`, with the prompt `Priory restarted and
found tmux session <name> gone, as after a reboot, so this run was interrupted. Check the worktree,
then continue.` The next run resumes the session in the same worktree. This happens once. If the
task's previous run was lost too and this run stored no ask or wait, the task blocks with
`run <id> was lost again before it made progress: tmux session <name> is gone`. A prompt the lost
run had attempted stays `uncertain` and is never typed again, unless the run never reached
`SessionStart`: then it never took the prompt, as a Codex run takes its first one at launch, so the
prompt is queued again after the one that says the run was lost.

A code step's `run` action is not a run: it starts a command without tmux, an agent or a room check,
and a restarted daemon kills that command and ends its step `failed` instead of keeping it. See The
`run` action in [tools.md](tools.md#the-run-action).

## Queue

The queue holds the `queued` and `ready` tasks at a model step. It starts their runs in band order,
then in the order a person set inside the band, then oldest first. Code steps still run oldest first
(see Code steps).

Bands default to `blocker`, `now`, `today` and `later`, highest first. `factory.toml` can name
its own, highest first:

```toml
name = "demo"
bands = ["urgent", "normal", "someday"]
```

A new task's band is its request's `band`, else its trigger's, else its workflow's `band`, else the
last band. A request band the factory lacks, from a message published before a rename, counts as no
band. `priory check` refuses a trigger or workflow naming a band the factory lacks, with its line;
`priory request --band` refuses one too. The daemon records the bands in the state file when it
starts, so listings and `priory move` read them without the factory. A task whose band a later
`factory.toml` dropped comes after the last band. A task from before bands existed gets the last
band when the daemon starts.

`priory move <task> --band <band>` puts an active task in another band, after the tasks a person
ordered there. `priory move <task> --top`, `--before <other>` or `--after <other>` sets its place in
the person's order of its band, or of the other task's band, which it joins. `--band` combines with
`--top`, or with `--before` and `--after` when it names the other task's band. A move that sets a
place numbers every active task of the band in its current order, so tasks that arrive later go
after them, oldest first. `priory move` writes the state file directly. It needs no daemon, and the
daemon reads the new order on its next tick.

## Holds

`priory hold <task> --reason <text>` holds a `queued` or `ready` task, so it never starts until
its hold is released. It refuses any other state: a running task already has a process, which a
hold would have to kill mid-turn, and an asleep task waits on its own ask or wait, whose wake would
race the hold. Hold such a task once it is back in the queue. `priory release <task>` lets a held
task go, back to the state it was held from. Both go to the daemon over `priory.sock`, as
`priory resume` does, so a hold and a run start never race.

A hold takes at most one condition, which releases it without a person:

| Flag | Released when |
| ---- | ------------- |
| none | a person runs `priory release` |
| `--until <time>` | the time passes: an RFC 3339 instant, or a duration such as `2 hours` from the hold, as for a time wait |
| `--after <task>` | the other task is done, failed or dropped |
| `--merge <pr>` | the pull request merges: `owner/name#42`, or `42` in the task's repository |
| `--blockers` | the task's Linear issue has no open blocker left |

`--merge` takes `--if-closed release|drop|ask` for a pull request closed without a merge. `release`
lets the task go. `drop` drops it with the reason `<pr> closed without a merge`. `ask`, the default,
puts an ask on the desk with the options `release` and `drop` while the task stays held.
`priory answer` takes either. A merge after the ask releases the task and answers it. A close whose
`merged` is null, because GitHub no longer returns the pull request, does not say whether it merged,
so it asks whatever `--if-closed` says.

`--blockers` reads the `blockers` field of Linear events (see `docs/intake/linear.md`). It is refused
for a task whose source event is not a Linear event, such as a GitHub issue or a CLI request. The
newest event about the issue that carries `blockers` decides: an empty list releases the task, and a
null one, which Linear's paging left unknown, does not. That event is a creation or state change of
the issue, or a `linear.issue.blockers` that one of its blockers published when it changed.

The daemon judges a hold when it is stored. Then it judges a time or task hold on each tick, and a
merge or blockers hold on each message that names its pull request or issue, whatever the message's
route. A condition that already holds, such as a pull request merged before the hold, releases the
task at once. A task request with `hold` (see `docs/intake/task-requests.md`) creates its task held
the same way.

A held task has the reason `held: <reason>, <condition>`, which `priory tasks` and the desk show.
For example: `held: needs the schema, until github:owner/repo#7 merges, and asks if it closes unmerged`.
Events for a held task are kept for its first run, as for a queued one, and a schedule's tick for it
is coalesced. `priory cancel` drops a held task and ends its hold.

Known limits:

- A pull request closed, reopened and merged later is judged on its newest close only. With
  `drop` or `ask`, the first close already acted.
- A hold names one condition; holding until two of them takes a person.
