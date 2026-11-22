# MCP tools

Every run gets one MCP server, `priory`, whose tools let its agent end a step, ask a person, or
sleep until an event. This page lists each tool: its arguments, an example call, its reply and its
refusals. [Tools](../daemon/tools.md) explains how a run uses them: which tools end a turn, how a
subscription carries events, and how the task moves on.

## `result(outcome, summary, evidence?, commit?, reason?, branches?, risk?, goal?, testing?, brief?, verification?, feedback?, batch?)`

`result` stores the run's result and ends the step. Nothing can change or delete a stored result.

`batch` is a batch's plan, sent by its planning task. The daemon checks it and stores it with the
batch, or refuses the result with what to fix; [batches.md](../daemon/batches.md) lists the checks.

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

An outcome that leads to `done`, or to a step with `after = "merged"`, needs `commit` when the task
owns a pull request that has not merged yet (see Done and checks in [waits.md](../daemon/waits.md)). `commit`, like a checks wait's `head_sha`, must be a full SHA,
because GitHub names pull request heads by their full SHA. An abbreviated SHA is refused. A SHA in
capitals is stored in lower case.

The `open-pr` action titles its pull request with the first line of `summary` (see [Code steps](../daemon/tools.md#code-steps)).

`goal`, `testing`, `brief` and `verification`, optional strings, are for a plan step: what the
change must achieve, how it is tested (the person's method, else the repository's), what to change
and why, and how the change will be proved, as commands or checks. The task keeps the latest of
each as `plan_goal`, `plan_testing`, `plan_brief` and `verification`, and a result that omits one
leaves the stored one. `priory task <id> --json` shows all four. The ask that approves the plan, a
`plan` task's or a human step's, carries them as its `text`: `Goal: ...`, `Testing: ...`,
`Brief: ...`, `Verification: ...`, then the merge risk when the task has one. The ask's text is
refreshed when an irreversible step's approval is asked again. Each model
step the task enters afterwards gets the line `Prove the change with: ` and the verification at the
end of its prompt. A plan without them changes nothing. A step's `plan_requires` makes
`priory check` refuse a workflow whose step and agent never ask for them (see
[workflows.md](workflows.md)).

An outcome that the step's `proof` names is refused until its proof holds. See [Proofs](../daemon/tools.md#proofs).

### Review feedback

A prompt that carries review feedback ends with a rule: sort each item by its URL before acting.
An item is a review that is not an approval or a dismissal, a pull request comment, a review thread
comment, an issue comment or a Linear comment, and its URL is the event's `url`. Each item is one of:

| Class | Meaning | What the agent does |
| ----- | ------- | ------------------- |
| `trivial` | One obvious answer inside the change's scope | Fix it and reply |
| `nothing-to-do` | An approval or thanks | Nothing |
| `persons-call` | A behaviour, design or scope change, a disagreement, or anything unclear | Asks the person with `ask`, and changes nothing for it |

`feedback`, an optional object from item URL to class, records the sorting:

```json
{
  "outcome": "passed",
  "summary": "Answer the review",
  "commit": "3f9c2a7e5b1d4c6f8a0e2b4d6f8a1c3e5b7d9f0a",
  "feedback": {
    "https://github.com/owner/name/pull/57#discussion_r802": "trivial",
    "https://github.com/owner/name/pull/57#pullrequestreview-301": "persons-call"
  }
}
```

A result is refused when an item the agent was typed since the task's last result has no class,
from this result or an earlier one, when `feedback` is not an object of the three classes, or when
it sorts an item as `persons-call` and no ask of the task names the item's URL in its `question`,
`title` or `text`. An ask the person dismissed still counts: the dismissal is their call. An item typed to a run before the task's last result was that result's to sort,
so a later result need not name it. One refusal names every item without a class. A `result`
cannot coexist with an open ask, so the agent asks, reads the answer in its next turn and then
sends the result. The refusals read:

```text
feedback item https://github.com/owner/name/pull/57#discussion_r803 has no class: feedback must be an object from each item's URL to trivial, nothing-to-do or persons-call
feedback items https://github.com/owner/name/pull/57#discussion_r803, https://github.com/owner/name/pull/57#discussion_r804 have no class: feedback must be an object from each item's URL to trivial, nothing-to-do or persons-call
```

An item written by the login `gh api user` names, the account the intake runs as and so the
factory's person, goes to the person: the rule does not name it, the agent need not sort it, and
the next result records it as `persons-call` with no ask. Only an item with `own` true is the
person's, so a Linear or issue comment never is. The agent's own replies post as the same account:
an item with `own` true written inside a run of the task, past its first second and before its
last, is the agent's, as routing reads it, and no class is recorded for it. A person's own comment
written in that window reads as the agent's too. No factory key names another login.

A factory with a calibrated triage decision model sorts some items itself: an item it marks
`nothing-to-do` is not named by the rule, and the next result records its class with no word from
the agent (see [decisions.md](../daemon/decisions.md)).

The task keeps each class, and a later result can sort an item again. `priory task <id> --json`
gives them as `triage` (see [Task in detail](serve.md#task-in-detail)), and the task
page lists them in its PR section. A result without `feedback` is a plain result while no typed
item waits for a class. Priory adds the rule to a prompt only; no event type changes.

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

### Grades

A model step whose outcome is `red`, `yellow` or `green` grades the task. When the step ends, the
task takes the outcome as its grade and one line as its grade reason: the first line of `reason`,
or of `summary` when the result has no `reason`. A later grading step overwrites both. Every grade
stays in the step's result and in its `factory.step.ended` record, which carries a `grade` field
(see [trace-schema.md](../trace-schema.md)).

`priory tasks` prints the grade and its reason in the `grade` and `grade_reason` columns, and
`priory tasks --grade green` lists only the green tasks.

### Merge risk

A grade is a forecast made before the work. A merge risk is a verdict on the finished change, given
by a risk step: a model step whose result carries `risk` and whose outcome, `low`, `medium` or
`high`, is the verdict. Give it its own agent, not
the implementer, so the verdict is a fresh look at the diff.

Its result carries `risk`, one line per dimension, each starting with its level:

```json
{
  "outcome": "medium",
  "summary": "Merge risk medium",
  "risk": {
    "ask": "low: does what the issue asks, nothing more",
    "blast_radius": "medium: every task listing reads the new column",
    "verification": "low: a test covers each new branch",
    "rollback": "medium: the migration adds a column a revert leaves behind",
    "detection": "low: the listing tests fail on a wrong value"
  }
}
```

| Dimension | Asks |
| --------- | ---- |
| `ask` | Does the change do what was asked, and nothing else? |
| `blast_radius` | How much breaks if the change is wrong? |
| `verification` | How well do the tests and checks prove it works? |
| `rollback` | How costly is it to undo once merged? |
| `detection` | Would a failure be noticed soon? |

The outcome must be the highest level of the five lines. Each line is its level, a colon, a space
and why, with no tab or line break. `risk` with another outcome, a missing or extra dimension, and a
line of another shape are refused with:

```text
risk must be an object with one line per dimension, ask, blast_radius, verification, rollback, detection, each its level, a colon and why, on one line
outcome low must be the highest level of risk's lines: medium
risk needs a low, medium or high outcome: its verdict
```

A `low`, `medium` or `high` outcome without `risk` is a plain outcome and stores no verdict.

The result keeps the verdict as `merge_risk`, and the task takes the latest one as its own
`merge_risk`: `verdict` and the five lines (see [Task in
detail](serve.md#task-in-detail)). Every ask of the task carries it: `priory asks
--json` gives it as `merge_risk` and adds the verdict to the ask's `risk`, so `priory asks --risk
low` lists the asks of tasks graded low. `priory asks` prints the five lines in its last column,
`merge_risk`. A human step's ask also takes the verdict and the five lines as its `text`, which the
web view and a routed ask's lead read.

## `ask(question, options, recommendation, tradeoff, title?, text?, whatWaits?, nextAction?, command?)`

`ask` puts a question on the desk and stores it as an ask at revision 1. The agent then ends its
turn and sleeps with no process. The person's answer arrives as its next prompt (see
[waits.md](../daemon/waits.md)). For example:

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

### Commands an answer runs

An ask's `command` runs once when a person answers with one of its options. For example:

```json
"command": { "argv": ["scripts/deploy", "--env", "staging"], "target": "ops", "runs": ["deploy"] }
```

| Field    | Type             | Meaning |
| -------- | ---------------- | ------- |
| `argv`   | array of strings | The program and its arguments. Priory execs them as they are, never through a shell |
| `target` | string           | Where it runs: `worktree`, the task's worktree, or a name in the factory's [`[asks.targets]`](factory.md#asks). Default `worktree` |
| `runs`   | array of strings | The options whose answer runs it. Default: the recommended option, the one equal to `recommendation` |

The tool refuses a `command` that is not such an object, a target the factory does not name, a
`worktree` target for a task with no worktree, and a `runs` entry that is not an option. Without
`runs`, it refuses a recommendation that is not an option. It stores the target as an absolute
folder, so a later factory change never moves the command. An `argv[0]` with a slash runs from that
folder; a bare name is looked up in `PATH`.

An answer with an option in `runs` must confirm that it runs the command: `priory answer --run`, or
`"run": true` over HTTP. Without it, the answer is refused with the command, its folder, its target
and the revision it is bound to. An answer with another option, or free text, answers as usual and
runs nothing. An option that runs a command takes no note (see
[waits.md](../daemon/waits.md#asks-and-answers)).

The answer is bound to the revision it read, as any answer is, and the command stored with that
revision is the one that runs. The first answer stands, so a second answer never runs it again.

In the transaction that stores the answer, the daemon stores the command as an `answer` action,
with the ask, the revision, the command and the task's generation, and keeps the task `running`, so
the sweep leaves its worktree alone. The daemon starts it at its next tick, as the `run` action
starts its command (see [The `run` action](../daemon/tools.md#the-run-action)), bounded by `command_timeout`, or by its step's `timeout`
for a human step's command, with the same environment.
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
[Runbooks](workflows.md#runbooks)). Its ask stores the command, with its target
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

## Asks from outside Priory

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
`session`, as [asks.md](../daemon/asks.md) describes.

## `wait(on?, any?)`

`wait` stores a wait on one condition, `on`, or on a list of them, `any`. A call takes one of the
two. The agent then ends its turn and sleeps with no process. The event that satisfies a condition
arrives as its next prompt, and the other conditions are withdrawn with the reason `sibling-fired`
([withdrawal reasons](../daemon/waits.md#why-an-ask-or-wait-was-withdrawn)). For example, to sleep until a
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

[waits.md](../daemon/waits.md) lists every condition.

## `subscribe(on)` and `unsubscribe(on)`

`subscribe` makes the task own a pull request, a Linear issue or a Slack thread until the resource
closes, the task retires (fails or is dropped), or `unsubscribe`. It takes one condition object, as
`wait` does, and reads only the fields that name the resource. A close sets the resource's
subscriptions aside, and a later event that says the pull request or issue is open again (its
`open` is true) gives them back, so a pull request closed and reopened to run CI again keeps its
subscribers. An event of a GitHub issue that the intake polls carries no `open`, so only a webhook
event tells a reopened issue apart:

| `on.type` | Resource |
| --------- | -------- |
| `pull_request`, `checks` or `review` | `github:{repo}#{pull_request}`. An issue number names the issue |
| `linear_comment` or `linear_state` | `linear:{issue}` |
| `slack_reply` | `slack:{channel}:{thread_ts}` |

Neither tool ends the turn, and either can come after a result, an ask or a wait.
[Subscriptions](../daemon/tools.md#subscriptions) says how its events reach the task, by the task's
state.

For example, to hear every review of an upstream pull request while the task works on:

```json
{ "on": { "type": "pull_request", "repo": "owner/name", "pull_request": 57 } }
```

`unsubscribe` ends the subscription. What the task owns otherwise, such as its key, a pull request
its `open-pr` opened or one it waited on, it still owns. Unsubscribing from a resource the task does
not subscribe to is refused.

`priory subscribe TASK RESOURCE` and `priory unsubscribe TASK RESOURCE` do the same for any task,
writing the state file directly. They take the resource string, such as `github:owner/name#57`,
and exit 2 on one they cannot read. `priory subscribe` also takes `--expected REVISION`, the state
revision the caller read: once the state moved past it, the subscribe exits 1 and changes nothing.

```sh
priory subscribe 7 github:owner/name#57
priory unsubscribe 7 linear:ENG-7
```

`POST /api/subscribe` and `POST /api/unsubscribe` are their twins on `priory serve`, and the Herdr
library's `subscribe` and `unsubscribe` run the commands ([serve.md](serve.md#writes)).

`priory task ID --json` and `/api/tasks/ID` list the live subscriptions as `subscriptions`.

## `relate(kind, task)`

`relate` records how the caller's task relates to task `task`: `builds-on`, `fixes`, `takes-up` or
`relates`. It does not end the turn, and `priory mcp` answers it from the state file. Its task is
always the caller's own. `priory relate TASK KIND OTHER` does the same for any task. See
[relations.md](../daemon/relations.md).

```json
{ "kind": "builds-on", "task": 7 }
```

## `finding(key, title, details?)`

`finding` hands one problem that a scheduled run found to a fix task. It does not end the turn, and
it works in any run, though it is made for the task a `schedule` trigger wakes again and again
(see [schedule.md](../intake/schedule.md)).

```json
{ "key": "disk-full", "title": "The disk is full", "details": "df shows 100% on /var" }
```

| Field     | Meaning |
| --------- | ------- |
| `key`     | A stable name for the problem, the same in every run. Required |
| `title`   | The problem in one line. Required |
| `details` | Evidence and links the fix task needs |

The first call of a key publishes a task request, as `priory request` does, with the factory's
`[request]` defaults for its workflow and band and the scan's repository. Its key is
`finding:<scan task>:1:<key>`, its text is the title and details, and it relates to the scan task
(see [relations.md](../daemon/relations.md)). A second call in the same run changes nothing.

Priory remembers each finding by task and key, so a scan that reports the same problem in its next
run opens no second task. A scan that skips a run has shown the problem gone: if it comes back, the
finding opens a new task, keyed `finding:<scan task>:2:<key>`, then `:3:<key>`.

### Run memory

A firing that wakes the scan adds the earlier findings to the prompt that wakes it, after the
trigger's `prompt`:

```text
Findings from your earlier runs. Report a finding again with the same key, and no second fix task opens:
- disk-full: The disk is full (reported in the last 2 runs in a row; fix task finding:1:1:disk-full)
```

A finding the last run did not report reads `not reported in the last run, so it looks fixed`. A
person who wants the scan to look differently changes its prompt in a pull request on the factory.

### Streaks

A finding reported in 3 runs in a row moves its fix task up one band, once, unless that task is
done, failed, dropped or already in the first band. The tool's reply says so. The count is a
constant, not a setting.

## `remember(name, description, text)`, `forget(name)` and `memory(name)`

`remember` saves one fact for later runs, as a memory. `forget` removes the memory of a name, such
as one the kit or the factory holds. `memory` replies with the file of a memory, as the caller's
task sees it now. None of them ends the turn.

```json
{ "name": "release-day", "description": "When releases go out.", "text": "Releases go out on Tuesdays." }
```

| Field | Meaning |
| ----- | ------- |
| `name` | The memory's name, in lowercase letters, digits and hyphens. Required by all three |
| `description` | When an agent should read it, as the memory index shows it. Required by `remember` |
| `text` | The fact, in Markdown. Required by `remember` |

Each `remember` or `forget` call is a candidate. The daemon stores it, then publishes
`priory.memory.written` on the log through its queue, as it does a claim. Its own task's later runs
get it at once, and other tasks' runs only once a person accepts it. `remember` refuses a file over
4096 bytes and text that looks like a credential. The reply names the candidate:

```text
remembered release-day as candidate 0196f1d0-4a5b-7c6d-8e9f-0a1b2c3d4e5f: your task's later runs see it now, and other tasks' runs once a person accepts it
```

See [skills.md](../daemon/skills.md#memories-agents-write) for how a candidate reaches runs, and
[memories.md](memories.md#candidates) for the states, the commands and every error.

## `compact(focus?)`

`compact` queues `/compact <focus>` in the outbox and tells the agent to end its turn. An agent
cannot type `/compact` itself, since only typed input reaches its harness. A Codex run gets a bare
`/compact`, because Codex's `/compact` takes no argument.

A task holds at most one queued compact, so a second call is refused. `result` is refused too until
the compact is typed, because a result could move the task to another agent's conversation first.
`ask` and `wait` resume the same conversation, so the compact follows their answer or event.
Compacting is described under Compact in [outbox.md](../daemon/outbox.md).

## `tldr(text)` and `search(query, states?, limit?)`

`tldr` replaces the task's TL;DR, for when the work drifted from the request. `search` finds tasks.
Neither ends the turn, and either can come after a result, an ask or a wait. `priory mcp` answers
both itself from the state file, without the daemon. See [search.md](../daemon/search.md).

## `progress(goal?, acceptanceCriteria?, latestChange?, nextAction?)`

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

## `report(date, since, greeting, sections)`

`report` stores the task's report: ordered sections of rows, some with a pick. It replaces the
task's report and keeps the choices already made. It does not end the turn. `priory mcp` answers it
itself from the state file, without the daemon. A pick or a comment on a row arrives as the next
prompt; the agent answers a comment with `priory report reply`. See [reports.md](../daemon/reports.md).
