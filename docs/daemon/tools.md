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

Three tools end a turn: `result`, `ask` and `wait`. A run holds a result, or an open ask, an open
wait or both. A `result` after an open ask or wait is refused, as is a second `ask` or a second
`wait`. An ask and a wait side by side wake the task on the first answer or event (see An ask
beside a wait in [waits.md](waits.md)).

### `result(outcome, summary, evidence?, commit?, reason?, branches?)`

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

`branches`, optional, asks `open-pr` for a stack of pull requests: their branches, bottom first,
each based on the one before it. The last one must be the task's own branch, and each lower one is
named after it, such as `priory/7-api` for task 7. A list that breaks these rules, names a branch
twice or is empty is refused with:

```text
branches must list the stack's branches bottom first, each once, ending with the task's branch priory/7, each lower one named priory/7-<name>
```

For example, `"branches": ["priory/7-api", "priory/7-ui", "priory/7"]` opens three pull requests:
`priory/7-api` on the default branch, `priory/7-ui` on `priory/7-api`, and `priory/7` on
`priory/7-ui`.

#### Grades

A model step whose outcome is `red`, `yellow` or `green` grades the task. When the step ends, the
task takes the outcome as its grade and one line as its grade reason: the first line of `reason`,
or of `summary` when the result has no `reason`. A later grading step overwrites both. Every grade
stays in the step's result and in its `factory.step.ended` record, which carries a `grade` field
(see [trace-schema.md](../trace-schema.md)).

`priory tasks` prints the grade and its reason in the `grade` and `grade_reason` columns, and
`priory tasks --grade green` lists only the green tasks.

### `ask(question, options, recommendation, tradeoff, title?, text?, whatWaits?, nextAction?, command?)`

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

`options` is a list of plain strings. The other fields are optional strings that describe the ask:

| Field        | Meaning                                                              |
| ------------ | -------------------------------------------------------------------- |
| `title`      | The question in one line                                             |
| `text`       | The ask's body, in Markdown                                          |
| `whatWaits`  | What waits on the answer                                             |
| `nextAction` | What the agent does once it has the answer                           |
| `command`    | An object: a command that an answer runs (see Commands an answer runs) |

`priory asks` and `priory asks --json` show them, the JSON as `title`, `text`, `what_waits`,
`next_action` and `command`.

#### Commands an answer runs

An ask's `command` runs once when a person answers with one of its options. For example:

```json
"command": { "argv": ["scripts/deploy", "--env", "staging"], "target": "ops", "runs": ["deploy"] }
```

| Field    | Type             | Meaning |
| -------- | ---------------- | ------- |
| `argv`   | array of strings | The program and its arguments. Priory execs them as they are, never through a shell |
| `target` | string           | Where it runs: `worktree`, the task's worktree, or a name in the factory's [`[asks.targets]`](../reference/factory.md#asks). Default `worktree` |
| `runs`   | array of strings | The options whose answer runs it. Default: the recommended option, the one equal to `recommendation` |

The tool refuses a `command` that is not such an object, a target the factory does not name, a
`worktree` target for a task with no worktree, and a `runs` entry that is not an option. Without
`runs`, it refuses a recommendation that is not an option. It stores the target as an absolute
folder, so a later factory change never moves the command. An `argv[0]` with a slash runs from that
folder; a bare name is looked up in `PATH`.

An answer with an option in `runs` must confirm that it runs the command: `priory answer --run`, or
`"run": true` over HTTP. Without it, the answer is refused with the command, its folder, its target
and the revision it is bound to. An answer with another option, or free text, answers as usual and
runs nothing.

The answer is bound to the revision it read, as any answer is, and the command stored with that
revision is the one that runs. The first answer stands, so a second answer never runs it again.

In the transaction that stores the answer, the daemon stores the command as an `answer` action,
with the ask, the revision, the command and the task's generation, and keeps the task `running`, so
the sweep leaves its worktree alone. The daemon starts it at its next tick, as the `run` action
starts its command (see The `run` action), bounded by `command_timeout`, with the same environment.
A worktree target that a retirement removed is added back first, once there is room. When it ends, the action is `done` on exit 0 and
`failed` otherwise, with the last 2000 bytes of stdout and stderr. The task then wakes with this
prompt, followed by the two tails:

```text
Ask 7 was answered "deploy" at revision 1, which ran ["scripts/deploy","--env","staging"] in /srv/ops: exit 0.
```

The command belongs to the generation it was answered in. A cancel or a revive kills it, or keeps
it from starting, and the task is not woken. An answer that beats the run's Stop does not end the
turn's process early: `ask`, `wait` and `result` are refused until the command ends, and the Stop ends the
run while the task stays `running`. The task wakes when the command ends.

A daemon restart never runs the command again. The restarted daemon kills a command its
predecessor left running and ends its action `uncertain`. The task wakes with `... and Priory
stopped while it ran, so whether it finished is uncertain. Check the target before you act on it.`
The desk shows a `command` item while the task is active. A command whose process the stopped
daemon never recorded never started, so the next tick starts it.

A human step can carry the same command as its `command` key (see
[Runbooks](../reference/workflows.md#runbooks)). Its ask stores the command, with its target
resolved to a folder, when the task enters the step. A step whose target is `worktree` blocks a
task with no worktree, with the reason `step <id>: command target worktree: this task has no
worktree`. The answer runs it as above. When it ends, the task moves by the answer on exit 0 and
asks the step again otherwise, with `answer "<option>" ran <argv> in <dir>: <exit>.` in the
question, or the uncertain outcome. Its ask counts as one whose answer runs a command: a lead
never answers it, and its risk is `irreversible`, as an agent's command ask's is.

`priory actions` lists each answer's action. `priory asks --json` and `/api/asks` show it on the
ask as `ran`: its `status`, `revision`, `error`, `stdout` and `stderr`.

Known limits:

- A daemon that stops after recording the process but before releasing it ends the action
  `uncertain`, although the command never started.
- The command runs on the daemon's machine only.

### Asks from outside Priory

`priory ask ID JSON` stores an ask from a session that Priory did not start, such as a person's
Claude session. It takes the JSON of Fleet's `priory ask`: the fields of the `ask` tool, all
optional except `question` or `title`, and `coordinator`, `key`, `runId` and `session`. It prints
`ID`. An ID is 1 to 100 letters, digits, `.`, `_` or `-`, and neither only digits
nor only dots, so it stays one segment of a web view link.

```sh
priory ask cache.ship '{"title": "Ship the cache today?", "options": ["ship", "wait"], "session": "cache-worker"}'
```

The ask belongs to no task, and shows on the desk and in `priory asks` under its ID. A field the ask
does not know is ignored, with `priory ask: ignored unknown field <name>` on stderr. Asking again
under the same ID replaces every field. If any field changed, the ask gets a new revision and is
open again, even after an answer or a dismiss. The same fields change nothing.

Without `session` in the JSON, `priory ask` stores the session that ran it: the hc terminal in
`HC_TERMINAL_ID`, then a space and the Claude session in `CLAUDE_CODE_SESSION_ID` when it is set.
Outside an hc terminal, `session` stays empty.

`priory answer` stores the answer to an outside ask, under the same revision rule as any ask. An
outside ask's `command` is a string that Priory stores and never runs; the answer says so. When
the factory names a `deliver` command, the daemon then delivers the answer or the dismissal to
`session`, as [asks.md](asks.md) describes.

### `wait(on?, any?)`

`wait` stores a wait on one condition, `on`, or on a list of them, `any`. A call takes one of the
two. The agent then ends its turn and sleeps with no process. The event that satisfies a condition
arrives as its next prompt, and the other conditions are withdrawn. For example, to sleep until a
pull request's checks finish:

```json
{ "on": { "type": "checks", "repo": "owner/name", "pull_request": 57 } }
```

Or until the pull request closes or a Linear blocker is Done, whichever comes first:

```json
{ "any": [
  { "type": "pull_request", "repo": "owner/name", "pull_request": 57 },
  { "type": "linear_state", "issue": "ENG-7", "state": "Done" }
] }
```

[waits.md](waits.md) lists every condition.

### `subscribe(on)` and `unsubscribe(on)`

`subscribe` makes the task own a pull request, a Linear issue or a Slack thread until the resource
closes, the task retires (fails or is dropped), or `unsubscribe`. It takes one condition object, as
`wait` does, and reads only the fields that name the resource:

| `on.type` | Resource |
| --------- | -------- |
| `pull_request`, `checks` or `review` | `github:{repo}#{pull_request}`. An issue number names the issue |
| `linear_comment` or `linear_state` | `linear:{issue}` |
| `slack_reply` | `slack:{channel}:{thread_ts}` |

Neither tool ends the turn, and either can come after a result, an ask or a wait. Unlike a wait, a
subscription does not end when one event arrives. Its events reach the task by its state:

| Task | An event on the resource |
| ---- | ------------------------ |
| `running` | is queued as a prompt, read after the current turn |
| `asleep` at a model step, with no checks gate | wakes it, unless the task has an open wait on the resource, which decides instead. The inbox records `woke` |
| `done` | reopens it, when it is a comment or a review that asks for something (see [routing.md](routing.md#feedback-reopens-a-done-task)) |

For example, to hear every review of an upstream pull request while the task works on:

```json
{ "on": { "type": "pull_request", "repo": "owner/name", "pull_request": 57 } }
```

A task subscribes to its own key when it is created, when the key names an issue, a pull request,
a Linear issue or a Slack thread. So a reply in the Slack thread that started a task wakes it, and a
comment on its Linear issue reopens it once done. That subscription reads from the time of the
event that requested the task, so a reply posted before the task existed still reaches it.

The subscription to the task's own key carries only what people say and closes: reviews, comments,
review thread comments, a pull request or issue closed, a Linear comment or state change, and a
Slack reply. A check run, a label or a push on the task's own pull request wakes nothing, since
each would start a run; the checks gate and waits take those. A subscription made with `subscribe`
carries every event.

`subscribe` is refused while another active task owns the resource: `github:owner/name#57 belongs
to task 3, not this one`. A done task does not block it: its subscription ends and the active
task's starts. A `github.pull_request.closed` or `github.issues.closed` event ends every
subscription to its pull request or issue. Linear issues and Slack threads stay subscribed until
`unsubscribe` or the task retires. The intake stops reading a done task's own Slack thread 7 days
after it was done (see [watches.md](../intake/watches.md)).

When another task takes an event on the resource, such as the task keyed by that pull request, the
subscriber still gets it: a prompt of its own while it runs, or a wake while it sleeps. A close
reaches the subscribers before it ends their subscriptions.

`unsubscribe` ends the subscription. What the task owns otherwise, such as its key, a pull request
its `open-pr` opened or one it waited on, it still owns. Unsubscribing from a resource the task does
not subscribe to is refused.

`priory subscribe TASK RESOURCE` and `priory unsubscribe TASK RESOURCE` do the same for any task,
writing the state file directly. They take the resource string, such as `github:owner/name#57`,
and exit 2 on one they cannot read:

```sh
priory subscribe 7 github:owner/name#57
priory unsubscribe 7 linear:ENG-7
```

`priory task ID --json` and `/api/tasks/ID` list the live subscriptions as `subscriptions`.

### `relate(kind, task)`

`relate` records how the caller's task relates to task `task`: `builds-on`, `fixes`, `takes-up` or
`relates`. It does not end the turn, and `priory mcp` answers it from the state file. Its task is
always the caller's own. `priory relate TASK KIND OTHER` does the same for any task. See
[relations.md](relations.md).

```json
{ "kind": "builds-on", "task": 7 }
```

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

### `progress(goal?, acceptanceCriteria?, latestChange?, nextAction?)`

`progress` publishes where the task stands, for a person who reads the desk. It does not end the
turn, so the agent can call it at any point while its run is live. The daemon stores it on the task.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `goal` | string | What the task must achieve |
| `acceptanceCriteria` | array | Each entry `{ "id", "description", "required" }`: two strings and a boolean. A new list replaces the stored one |
| `latestChange` | string | What changed last |
| `nextAction` | string | What the agent does next |

A call merges into what is stored: a field left out, or null, keeps its value. A call with no
field, an unknown field or a field of the wrong type is refused, and nothing is stored. Each change
appends a `factory.task.progressed` record to the trace with the merged progress, so the trace keeps
every earlier value (see [the trace schema](../trace-schema.md)).

`priory progress TASK JSON` does the same for any task, through the daemon:

```sh
priory progress 7 '{"nextAction": "Wait for CI"}'
```

`priory tasks --json`, `/api/tasks` and `/api/tasks/ID` show the stored value as `progress`.

### `report(date, since, greeting, sections)`

`report` stores the task's report: ordered sections of rows, some with a pick. It replaces the
task's report and keeps the choices already made. It does not end the turn. `priory mcp` answers it
itself from the state file, without the daemon. A pick or a comment on a row arrives as the next
prompt; the agent answers a comment with `priory report reply`. See [reports.md](reports.md).

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

## Intervention

A task's `intervention`, from its trigger or its request (see
[triggers.md](../reference/triggers.md#intervention-and-lead)), sets who answers its asks. Unset,
every ask goes to a person.

### `plan`

When a step's result grades a `plan` task red, yellow or green, and the grade leads to a model or
code step, the task does not move on. It sleeps at the grading step on an ask:

```text
plan: approve the plan graded green? <the result's summary>
```

Its options are `approved` and `rejected`. `approved` moves the task to the step its grade leads
to. `rejected` drops it. Until a person answers, an event on the task's issue, pull request or
subscriptions is kept for its next run and wakes nothing. A grade that leads to a human step, `done` or `stop` moves on as usual:
the human step already waits for a person.

### `hands-off`

An ask of a `hands-off` task goes to the trigger's `lead`, a Claude Code agent with a shared
session, instead of the person. These asks still go to the person:

- an ask whose answer can let an irreversible step run before a person decides again, the asks
  that `priory asks --json` marks with the `irreversible` risk;
- the ask of a human step with `approves`, since its answer grants an approval;
- an ask whose answer runs a command;
- the asks the daemon makes itself: a hold's, a stack's, and an approval asked again at a new
  commit;
- every ask once no trigger names the task's lead any more, since its workflow is gone.

In the transaction that stores a routed ask, the daemon queues a lead task for it, with the key
`priory:ask:<ask>:<revision>`, the trigger `priory/lead`, and the asking task's repository, band and
lane. It starts the built-in workflow `priory/lead/<lead>`: one model step, `answer`, whose
outcomes `answered` and `skipped` both lead to `done`. Its first prompt carries a
`priory.ask.routed` event: the task, its key, the ask, its revision, question, options,
recommendation, trade-off, title and text.

The lead answers with `result`:

```json
{ "outcome": "answered", "summary": "sqlite" }
```

The summary, trimmed, is the answer. When the ask lists options, it must be one of them, or the
result is refused with `ask <id> takes one of: <options>`. A person can still answer an agent's ask
in free text. In the same
transaction the daemon applies it to the ask, as a person's answer would, and records it as
answered by `agent:<lead>`. The asking task then wakes as after any answer. When the answer is
refused, for example because the ask is no longer open, the result is refused with the reason, and
the lead ends with `skipped` instead.

The routed ask stays on the desk, marked `routed to <lead>`, so a person can still answer it. The
first answer stands. A person's answer or dismissal drops the lead task while it is still `queued`,
with the reason `ask <id> was answered first` or `ask <id> was dismissed`. A lead task already
running reads the refusal and ends `skipped`. A routed ask sends no push notification.

Known limits:

- Each lead task gets its own worktree of the repository, like any task, although the lead only
  answers. A routed ask costs one `git worktree add`.
- A lead task queued for an ask that is withdrawn, as when its task is cancelled or revived, still
  runs, and ends `skipped`.
- The level is read when an ask is stored. Changing a trigger does not move a task already
  created, and a task keeps the `lead` its trigger had when it was created.

## Code steps

A code step runs its `action` in the daemon, with no agent and no run. Each loop, the daemon takes
the oldest `queued` or `ready` task at a code step that does not wait for disk (see
[sweep.md](sweep.md)) and runs its action outside any transaction. One
transaction then records the action run and moves the task by `next[status]` (see the table under
Moving on). A `run` action records itself first and runs in the background (see The `run` action).
An action that is not built yet blocks the task with `action <name> is not built yet`.

The built-in actions are `open-pr`, `open-issue` and `run`. `open-pr` opens one pull request per
branch of the latest result's `branches`, bottom first, or for the task's branch alone without it.
For each branch:

1. It pushes the branch to the clone's `origin`. The task's branch, the top one, is pushed at the
   commit its approval covers. A lower branch is pushed at its own head, which must be that commit
   or one of its ancestors, so the approval covers the whole stack.
2. It reuses the open pull request from that branch if `gh pr list` finds one.
3. Otherwise it runs `gh pr create -R <repo> --base <base> --head <branch>`. The base is the
   branch below, or the default branch for the bottom one. The title is the first line of the
   latest result's `summary`, followed by ` (2 of 3)` in a stack of two or more. The body is the
   `summary`, then its `evidence`.

Its status is `opened` or `failed`. A failure records the error, which a human step reached by
`next.failed` shows in its question. On `opened`, the task owns each pull request and subscribes to
it, so its events route to the task until it closes, and its repository is polled. The pull
requests form the task's [stack](routing.md#stacks). A pull request that another active task owns
fails instead, and that task is named. So do these:

| Error | Cause |
| ----- | ----- |
| `the stack has <n> branches, past max_prs = <cap> of workflow <name>` | More branches than the workflow's [`max_prs`](../reference/workflows.md#top-level-keys) |
| `branch <branch> is not below the approved commit <commit>` | A lower branch is missing, or holds a commit the approval does not cover |
| `pull request #<n> from <branch> is based on <found>, not <base>` | The open pull request it would reuse is not based on the branch below. Retarget or close it |

If the task then reaches `done` and its latest result names no `commit`, the checks wait of Done
and checks pins the commit that `open-pr` pushed.

If the daemon stops after `gh pr create` and before recording it, the action runs again at its next
start, once the approval still holds. The push repeats and the open pull request is reused. If
someone closed that pull request meanwhile, a second one opens.

`open-issue` opens an issue the same way; [sweep.md](sweep.md) describes it.

### The `run` action

A `run` action runs the step's `run` command with `sh -c`, so a script can decide a step without an
agent. It runs in the task's worktree when it has one, else in the factory folder. Its environment
adds:

| Variable         | Value                                    |
| ---------------- | ---------------------------------------- |
| `PRIORY_TASK`    | the task's id                            |
| `PRIORY_HOME`    | Priory home, as an absolute path         |
| `PRIORY_DB`      | the state file, as an absolute path      |
| `PRIORY_FACTORY` | the factory folder, as an absolute path  |

The daemon stores the action as `running`, and the task as `running`, before the command starts.
The shell then waits until the daemon has recorded its process group, and exits without running
the command if the daemon dies first. The command runs in its own process group while the daemon
carries on with other tasks, with stdin closed.

Its outcome is the last line of its stdout, ignoring trailing blank lines and surrounding spaces,
when the step's `next` names it and the command exited. Otherwise it is `ok` on exit 0 and `failed`
on any other exit, a signal or a kill. A command past the step's `timeout`, or the factory's
`command_timeout` when the step sets none, is killed with its process group and ends `failed`
whatever it printed. Children it leaves in the background are killed when it ends. A non-zero exit
or a kill is the action's error, such as `exit 3` or `timed out after 120s`.

A task whose worktree the sweep removed gets it back before the command starts, as for `open-pr`. A
command that cannot start, such as in a missing folder, ends `failed` with `cannot start: <error>`.

The action keeps the last 2000 bytes of stdout and of stderr, which `priory actions` and the task's
`steps` in `GET /api/tasks/<id>` show. A cancelled task's command is killed, and the task stays
dropped. Only the task's latest action moves it, so a command killed by a cancel never moves the
task that a revive started again.

A daemon restart never runs a command twice. The restarted daemon kills the command its predecessor
left running, after checking its process group's start time in `/proc`, and ends the action
`failed` with the error `interrupted by a daemon restart`. The task then follows `next.failed`. So
a restart reads as a failure even when the command had finished just before it.

Known limits:

- A restarted daemon cannot kill background children of a command whose shell already exited. It
  cannot tell that group from a process that reused the shell's pid.
- `priory queue` lists a task whose command runs in neither `waiting` nor `running`.

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

`priory actions` prints each action run: id, task, step, action, status, pull request, pushed commit,
error, stdout and stderr. A `run` action's status is `running` until its command ends.
