# Messages and routing

The daemon reads the intake's messages one at a time. For each message, it records an inbox entry,
finds the task the message concerns, and decides what to do with it. This page explains how.

## One message at a time

Each message is handled in one SQLite transaction. The transaction holds the message's inbox
entry, any task the message creates, and the entry's outcome. The daemon acks the message on the
bus only after that transaction commits.

So each message changes the state exactly once:

- If the daemon crashes after the commit and before the ack, the bus delivers the message again.
  The redelivery adds nothing, because the inbox drops a message id it already holds.
- If handling the message fails with an error, the daemon stops before the ack. The next start
  handles the message again.

The daemon and the intake reach the bus through the `Log` trait in `priory-intake`. The trait
names what the guarantees above need: publishing that drops a repeated id, reading past a named
consumer's cursor, and an ack that moves that cursor. Two stores implement it: herdr-bus's SQLite
file, the default, and a Postgres database that several machines share. The factory's
[`[log]`](../reference/factory.md#log) table picks one. A contract test suite in
`crates/priory-intake/tests/log.rs` runs against every implementation, so both keep these
guarantees. See
[ADR 0001](../adr/0001-the-intake-and-the-daemon-reach-the-event-log-through-one-log-trait.md) and
[ADR 0002](../adr/0002-a-factory-chooses-herdr-bus-or-postgres-for-its-event-log.md).

`priory inbox` lists every entry, with the task it went to and its outcome.

## Resources: what a task owns

A task owns resources. A resource is a string that names one thing an event can be about. The
daemon stores every task's resources in one table.

A task gets its resources at these moments:

| When | Resource | Example |
| ---- | -------- | ------- |
| `priory adopt` parks it on a pull request | that pull request as its key, subscribed, as the first row does | `github:owner/name#57` |
| The task is created | its trigger key, and a subscription to it when the key names an issue, a pull request, a Linear issue or a Slack thread | `github:owner/name#42`, `linear:ENG-7`, `cli:docs-1` |
| A request whose `pre` script printed another `key` creates the task or joins it | a subscription to the issue, pull request or thread the trigger named before the script | `github:owner/name#43` |
| Its first run starts | its branch | `github:owner/name@priory/3` |
| A wait of its names a pull request | that pull request | `github:owner/name#57` |
| A wait of its names a Linear issue or a Slack thread | that issue or thread | `linear:ENG-7`, `slack:C0123:1763500000.000100` |
| It subscribes, with the `subscribe` tool or `priory subscribe` | that resource, until `unsubscribe` | `github:owner/name#57` |
| Its `open-pr` opens a pull request | that pull request, subscribed, and each pull request of its stack | `github:owner/name#57` |
| The first GitHub event of an open pull request names a branch the task made, its own or one named after it, as the head | that pull request, subscribed, unless another active task owns it | `github:owner/name#58` |

The last row is the claim. A worker that opens its pull request with `gh pr create` instead of
`open-pr` still gets its reviews and comments, and its checks gate, once the intake reads an event of
that pull request. A pull request on another task's branch is that task's, not this one's. Only
the first event of a pull request claims it, so an `unsubscribe` from it holds.

A head named after the task's branch, such as `priory/3-api` for task 3, is claimed too when the
pull request's base is the trunk or another branch of the task. The trunk is any branch outside
`priory/`. So a stack the worker opens itself, with `gh stack` for example, is the task's.

The name alone claims nothing. The head must be a branch in the task's worktree, which its worker
made, and the event's `head_repo`, when it has one, must be the pull request's own repository. So a
pull request from a fork, or from a branch someone else pushed under a task's name, joins no stack.
A fork's branch is not a resource of the event at all, so its events reach no task by branch. A
task whose worktree the sweep removed claims no new pull request.

When a Linear issue moves to another team, the intake publishes a `linear.issue.updated` with
`changed: ["identifier"]` and `previous_issue` (see [linear.md](../intake/linear.md#team-moves)).
Before it routes that event, the daemon renames every task key `linear:<previous_issue>`, and the
keys of its batch entries, to the new identifier. It moves owned and subscribed resources, open
waits and open holds the same way. So the event and every later one of the issue reach the task.
In a factory of several runners, every runner reads the event and renames its copy of the key's
claim and of the task actions sent to it, so the owner still starts the task and no other runner
claims the new key (see [runners.md](runners.md)).

### Stacks

A task's stack is every pull request its `open-pr` opened or the claim gave it. Each one is based on
the one below: the bottom one on the trunk, the next on the bottom one's branch, and so on. A task
with one pull request has a stack of one. `priory task ID --json` lists the stack bottom first,
with each pull request's state (see [serve.md](../reference/serve.md#tasks)).

An event of any pull request of a stack of two or more reaches the task, and its prompt line names
which one, as `#58, 2 of 3`. Feedback reads `ana commented on github:owner/name#58 (#58, 2 of 3): ...`,
and another event starts with `#58, 2 of 3: `. Done needs every pull request of the stack merged
(see Done and checks in [waits.md](waits.md#done-and-checks)).

GitHub numbers issues and pull requests from one sequence per repository, so
`github:owner/name#42` names either an issue or a pull request.

A message names resources too:

| Message | Resources it names |
| ------- | ------------------ |
| `factory.task.requested` | its `key` |
| `priory.task.requested.cli` | its subject, such as `cli:docs-1` |
| A GitHub event | its issue or pull request, `github:{repo}#{n}`, and for a pull request event its head branch, `github:{repo}@{head_branch}` |
| A review request event | also its review task's key, `github:{repo}:pulls/{n}:review:{reviewer}`. The active task with that key takes it before any other owner |
| A Linear event | its issue, `linear:{issue}` |
| A Slack event | its thread, `slack:{channel}:{thread_ts}`, or `slack:{channel}:{ts}` for a message outside a thread |

## Finding the task

A task is active in any state except done, failed and dropped. Only active tasks take messages,
except feedback that reopens a done task (see below).

For most messages, the daemon looks for the active tasks that own any resource the message names:

1. If one of them has a key that the message names, that task takes the message.
2. Otherwise the oldest of them takes it.

So an agent that opens a pull request from its branch gets that pull request's events, with no
trigger. The pull request event names the head branch `github:owner/name@priory/3`, and task 3 owns
that branch.

Each other active task that owns one of the resources still has its open wait fired, when the
message satisfies that wait. So two tasks can share a pull request. For example:

1. Task A waits on pull request 7. It now owns `github:owner/name#7`.
2. A trigger creates task B for pull request 7. Task B's key is `github:owner/name#7`.
3. A review arrives on pull request 7. Task B takes the event, since its own key names it.
4. If task A waits for that review, its wait still fires, and task A wakes with the event in a
   prompt of its own.

A task may wait again on a resource it already owns, or report a pull request it already owns,
even while another task's key decides that resource's route. Its new wait catches up on any event
that another task kept since the waiting task's last prompt.

A task request is the exception to these rules. Only an active task whose own key equals the
request's `key` keeps it. A task that owns that key only as a pull request or issue it waits on
does not. In that case, the request creates its own task.

### Comments on a pull request no task owns

A [comment or review trigger](../reference/triggers.md#comments-and-reviews) requests a task for
each comment on a pull request it matches. Unlike other task requests, it joins a task that
already has the comment, and its outcome is `coalesced`:

- the task the comment's own event reached, kept, woke or reopened, such as a task that owns the
  pull request's head branch;
- else any active task that owns the pull request, by key, wait, claim or subscription.

The comment's event already reached that task, so the request adds nothing. When the pull
request's newest task failed or was dropped, the request is `ignored`: `priory revive` brings that
task back (see [Feedback on a failed or dropped task](#feedback-on-a-failed-or-dropped-task)).
Otherwise the request creates a task.

That task's key is the pull request, `github:owner/name#42`, and it subscribes to it on creation
(see the first row of [Resources](#resources-what-a-task-owns)). So a second comment goes to it:

1. Ana comments on pull request 42, which no task owns. Its event is `ignored`.
2. The trigger's request creates task 5, keyed `github:owner/name#42`. Its first prompt carries the
   comment.
3. Ana comments again. Task 5 owns the pull request, so it takes the event, `kept` or `woke`.
4. The trigger's second request is `coalesced` on task 5. No second task starts.

A `priory request --key github:owner/name#42` keys its task the same way, so that task subscribes
to the pull request too (see [cli.md](../intake/cli.md)).

### Parked pull requests

A pull request opened by hand has no task, so its events reach none. `priory adopt` parks a task
on it (see [adopting-pull-requests.md](../adopting-pull-requests.md)). The task is keyed by the
pull request, so it owns and subscribes to it as a comment trigger's task does. Its source is a
CLI request, and the pull request still counts as its own, so after its first run the checks gate
and the merge apply to it as they do to a pull request that `open-pr` opened (see
[waits.md](waits.md#done-and-checks)).

The task is `held` with one condition, `parked`. A held task has no run, no worktree and no
process, so it costs nothing while it waits. Every event of the pull request, a comment, a review,
a check or a push, is `kept` on it with no prompt, and its Inbox lists them.

1. Ana comments on pull request 42. Task 5 is parked on it, so it keeps the event.
2. The comment trigger's request for pull request 42 finds task 5 parked. Its outcome is
   `released`, and task 5 is `queued`.
3. Task 5's first run reads the step's prompt, the adopt request, then every event it kept, the
   comment among them.

So the agent starts exactly when the trigger would have started a task. A comment the trigger does
not match stays kept, and the next run reads it. `priory release` starts the task by hand.

While the task is parked, the pull request's `github.pull_request.closed` ends it: `done` with
the reason `<key> merged while parked` when it merged, else `dropped` with `<key> closed while
parked`. The newest event that says whether the pull request is open decides, even one older than
the adopt request: a `github.pull_request.closed`, or any event with an `open` field. The desk,
the Queue's waiting list and `priory tasks` list no row for a parked task. `priory tasks --all`
shows it held, with the reason
`held: adopted for trigger <name>, until a comment or review trigger fires on <key>`.

`priory adopt` skips a pull request that a task in any state owns. A comment reopens a done task,
and `priory revive` brings back a failed or dropped one.

Events of the pull request from before the adopt stay unrouted: the daemon routes only new
messages.

### An agent's own comments

Agents post with the person's own `gh` login, so the login alone cannot tell an agent's comment from
the person's. An agent marks its comment with the line `<!-- agent-comment -->` in its body, a fixed
marker that GitHub does not render. The intake sets two fields on a comment, review or review thread
comment:

| Field | `true` when | What reads it |
| ----- | ----------- | ------------- |
| `own` | the login `gh` runs as wrote it | the person's handling: [triage](../reference/tools.md#review-feedback) does not put it to the agent |
| `agent` | it is `own` and its body holds the marker | triggers, waking during a turn, and the agent's reply in triage |

Priory adds the marker to each comment it posts itself, and the starter factories tell agents to add
it. The marker in another login's comment proves nothing: that comment is neither `own` nor `agent`.

An `agent` comment starts no task, so an agent cannot restart its own task. It is the agent's reply:
it wakes nothing during the agent's turn (see [Subscriptions in tools.md](tools.md#subscriptions)),
and triage records no class for it. An `own` comment without the marker is the person's direction.
It can start a task, it wakes its task whenever it was written, the agent follows it, and the next
result records it as `persons-call` with no ask.

## Outcomes

Each inbox entry gets one of these outcomes:

| Message                                   | Outcome   | Task                                         |
| ----------------------------------------- | --------- | -------------------------------------------- |
| A schedule's or review request's `factory.task.requested`, with a task asleep at a model step owning its key | `woke` | that task, now `ready` |
| A schedule's or review request's `factory.task.requested`, with a task in another active state owning its key | `coalesced` | that task, unchanged |
| A comment or review trigger's `factory.task.requested`, with a parked task owning its pull request | `released` | that task, now `queued` (see [Parked pull requests](#parked-pull-requests)) |
| A comment or review trigger's `factory.task.requested`, with a task its comment reached or an active task owning its pull request | `coalesced` | that task, unchanged |
| A comment or review trigger's `factory.task.requested`, with no such task and the pull request's newest task failed or dropped | `ignored` | none |
| A review, comment or review thread comment, with no active task owning a resource it names, and a done task it reopens | `reopened` | that task, now `ready` at its reopen step |
| `priory.task.requested.cli`, with an active task owning its key, at a model step whose agent started or waiting on a step's ask | `said` | that task, which hears the text as `priory say` sends it |
| `priory.task.requested.cli`, with an active task owning its key whose agent has not started | `kept` | that task, which reads the text in its first prompt |
| `github.pull_request.review_request_removed` with `reviewed: false`, with an active task whose key is its review key | `withdrew` | that task, now `dropped`, its live run ended |
| Any other, with an active task asleep at a model step subscribing to a resource it names, that fires no wait, and whose result no gate holds | `woke` | that task, now `ready` (see [Subscriptions in tools.md](tools.md#subscriptions)) |
| Any other, with an active task owning a resource it names | `kept` | that task                   |
| `factory.task.requested`, no active task  | `created` | a new task, `queued` at its workflow's start |
| `factory.task.requested`, malformed       | `ignored` | none                                         |
| A GitHub comment or review whose `association` is not `OWNER`, `MEMBER` or `COLLABORATOR` | `outsider` | none: it wakes, claims and reaches no task. Only a review that requests changes may reopen one, as [below](#feedback-reopens-a-done-task) says: its outcome is then `reopened` |
| A task request whose key no runner holds, with several runners | `claiming` | none until this runner's claim wins (see [runners.md](runners.md)) |
| A task request whose key another runner holds | `elsewhere` | none                                    |
| Any task request, on a [view-only machine](runners.md#a-machine-that-only-views) | `elsewhere` | none: it claims nothing |
| `priory.task.claimed` that wins its key   | `claimed` | the task this runner creates, when it is the new owner |
| `priory.task.claimed` that loses          | `refused` | none; this runner's own losing claim drops its copy of the task, `owned by runner <name>` |
| `priory.task.held` or `priory.runner.alive` | `noted` | none                                         |
| `priory.ask.answered` for this runner     | `answered`, or `refused` by the revision check | the ask's task, as a local answer moves it |
| `priory.ask.answered` for another runner  | `elsewhere` | none                                       |
| Any other, or any source event on a view-only machine | `ignored` | none                 |

A task request can name a workflow the factory lacks. It still creates its task, `queued` with no
step, so the daemon keeps serving the other tasks. On each tick, that task gets the reason
`waits for workflow <name>, which the factory lacks`. Once a daemon starts with that workflow, the
task moves to the workflow's start step. A malformed request is ignored.

The intake publishes a source event before its task request. So when the source event arrives, no
task owns it yet, and it is recorded as `ignored`. The task that its request creates keeps the
event's id in `source`, and the task's first prompt carries the event.

## The task's inbox

`priory task ID --json` and `/api/tasks/{id}` list, as `inbox`, every event routed to the task,
newest first: its source event, the events routing gave it, and the events another task took that
fired one of its waits or that its subscription delivered to it (see
[serve.md](../reference/serve.md#task-in-detail)). Each
event has its outcome, `prompt`, the state of the outbox prompt that carries it, and `wait`, the
task's wait it fired. Together they say what the event did:

| `outcome` | `prompt` | What happened |
| --------- | -------- | ------------- |
| `woke` | `confirmed` | It woke the asleep task, and its prompt was delivered |
| `reopened` | `confirmed` | It reopened the done task, and its prompt was delivered |
| `coalesced` | null | A schedule firing or review request that the task already covered |
| `released` | null | A comment or review trigger's request that released the parked task: its first run reads the events it kept |
| `said` | `queued` or `confirmed` | A repeated `priory request` for the task's key: its text is queued, then delivered |
| `ignored` | `confirmed` | The task's source event, which its first prompt carried |
| `kept` | `queued` | Its prompt is still queued: for after the current turn, or for the next run |
| `kept` | `confirmed` | Delivered as a prompt |
| `kept` | null | Kept while the task was asleep, or not yet running: its next run reads it |
| any | `withdrawn` | A reopen or `priory revive` withdrew its prompt |

An event with `wait` set fired that wait: an asleep task wakes on it, and another task keeps its
state until its next run.

When routing gives an event to one task and a subscription delivers it to a second, the second
task's inbox lists it with `task` naming the first. Its `outcome` and `prompt` are the second
task's own: `woke` when the event woke it, or `kept` when it queued a prompt while the task ran or
kept the event for the task's next run.
A comment that wakes the second task from its checks gate shows no `wait`, since that delivery
records the prompt but not the wait.

`priory task ID` prints the task's key, workflow, step and state, then an Inbox section with one
line per event: its sequence number, time, type, subject and what it did, such as
`kept, still queued`, `woke, delivered as prompt` or `kept, for its next run (routed to task 2)`.

A prompt is `attempted` while the agent is being typed to, and `uncertain` when a daemon restart cut
that short (see [outbox.md](outbox.md)).

## The first prompt

A task's first run reads, in order: the step's prompt and how to end it, the task's source event,
then every event the task kept that no prompt carried yet. The source event shows the title and
text a [`pre` script](../intake/pre.md) gave in place of the event's, then the context it added.

When the task's trigger names an [`intent` or `noop`](../reference/triggers.md#intent-and-noop),
the lines after the source event carry them, as the trigger had them when the task was created:

```text
The trigger's intent: Fix each labeled bug with a reviewed pull request.
If one of these cases holds, raise no ask and write nothing: end the step with a result that names the case.
- The issue is a question, not a bug.
```

## Events that reach no task

An event is unrouted when its outcome is `ignored`, no task took it, and it is not the source event
of a task that its request created. Such an event concerned no active task, so it changed nothing.
An event that names no resource is not listed, since no task could own it.

Most unrouted events are noise: another person's pull request, a Slack message no trigger matched.
The ones that matter are about a resource a task owned and that task has ended, such as a review on
a done task's merged pull request. So `priory inbox --unrouted [--since SEQ]` and
`/api/inbox?unrouted=1` split them (see [serve.md](../reference/serve.md#unrouted)):

- Events whose first resource a task owned or subscribed to, in any state, are grouped by that
  resource, with the newest such task and the command that routes the next event: `priory revive
  ID` when the task failed or was dropped, `priory add` with the task's workflow and the
  resource's GitHub or Linear URL when it is done, since a done task cannot be revived and `add`
  keys the new task by that URL, and `priory subscribe ID RESOURCE` when it is still active. A
  done task's Slack thread gets no command: no CLI verb keys a task by a thread.
- Requests a trigger's `pre` script dropped are listed with the trigger, the script, its reason
  and the key, such as `linear:ENG-12`. Their source events are not counted with the rest. A
  script judges each event once, so an issue it dropped stays without a task until a new event of
  it matches a trigger (see [pre.md](../intake/pre.md#one-decision-per-event)).
- The rest, which no task ever owned, are one count per source.

The web Inbox panel's Unrouted view offers the same actions from a phone: Revive, the Add sheet, or
`POST /api/subscribe`. Its Dropped section lists the requests a script dropped, with why and no
action. The Desk counts the first kind and the drops, each over the last 24 hours (see
[desk.md](desk.md#unrouted-events)).

A comment or review from outside the repository is not unrouted: its outcome is `outsider`. The
same listing shows these events as a group of their own, `outsider`, with each one's author and no
command, since no task takes them. The Desk counts those of the last 24 hours in a line of its own.

## Events no agent read

A `kept` event reaches a task, but a task that is not running reads it only at its next run. An
asleep task may sleep for hours on a wait that the event does not fire, and a task can end before
it runs again. So `priory inbox --unrouted` lists a third group, `unheard`: per task, the events it
kept, itself or through a subscription, that no prompt carries and no wait took, once they are 5 minutes old (see
[serve.md](../reference/serve.md#unrouted)). Only what people say, edit and close counts: the
kinds a task's own key carries, and a `priory add` that joined the task, and not the agent's own
comments. Checks and pushes stay with the
checks gate. An item the triage model settled is left out too: it waits for the task's next
prompt by design (see [decisions.md](decisions.md#triage)).

Each group names its task and a command: `priory say ID` for an asleep or blocked task, whose next
run carries the events after the text, or what the owned group gives for an ended task. A task that
waits on a step's ask gets no command: its ask shows the events already (see
[What a kept message does](#what-a-kept-message-does)). The Desk
counts these events over the last 24 hours (see [desk.md](desk.md#unrouted-events)).

The web and Herdr Inbox panels list these groups first, under Unheard. On the web, Say sends the
group's `say_text` to an asleep or blocked task. A blocked task does not run on a say: it reads the
events when it resumes, so its group also offers Resume. An ended task's group offers the owned
group's Revive or Request a task. The Herdr panel has no Say: it shows an asleep task's command.

## What a kept message does

What happens to a `kept` message depends on the task's state:

- For a `running` task, the daemon also queues an outbox prompt with the message, so the agent reads
  it after its current turn. Until then, for feedback, the agent's `result`, `ask` and `wait` are
  refused (see [Ending a turn](tools.md#ending-a-turn)).
- If the message satisfies the task's open wait, the wait fires in any active state (see
  [waits.md](waits.md)). An asleep task wakes. A ready, blocked or held task keeps its state, and
  its next run gets the prompt.
- If the task subscribes to a resource the message names and has no open wait on it, an asleep
  task at a model step wakes with it, and the outcome becomes `woke`. The subscription to the task's
  own key takes only feedback, closes, Linear state changes, `linear.issue.updated` edits and
  `linear.issue.left` (see
  [Subscriptions in tools.md](tools.md#subscriptions)).
  Another active task that subscribes to
  it gets it too: a prompt of its own while it runs, a wake while it sleeps at a model step, or
  else the event kept for its next run, as this list says for the first task (see
  [Subscriptions in tools.md](tools.md#subscriptions)).
- For a task that waits on a step's ask, where a person decides and no agent runs, a message of
  the kinds the task's own key takes, and not the agent's own, is added to that ask as its next
  revision: `While this waited: <who> on <subject>: <first line> <url>`, each part cut at 200
  characters. The new revision pushes, and an answer to the earlier revision is refused, so the
  person who answers reads it. An ask keeps five such lines. After that, a last line counts the
  rest, `While this waited: and N more.`, with no new revision and no push, so a commenter can
  neither grow the ask nor keep it unanswerable. The answer may lead to `done` with no agent run
  at all. If a run follows, it reads the message too.
- Any other kept message waits for the task's next run, which reads it in its first prompt.

## Schedule firings

A schedule's task request carries `scheduled`, the time the firing was due (see
[schedule.md](../intake/schedule.md)). Only a schedule with a fixed `key` sends a second request for
the same key. Without a fixed key, each firing has its own key and creates its own task.

When a firing finds its task asleep at a model step, it wakes the task, even when the task's wait
does not match. The task becomes `ready`. Its prompt is the trigger's `prompt`, then the firing's
line, which carries the firing time, then the events the task kept while asleep.

The task's open ask and wait stay open through that wake. What happens next depends on how the run
ends its turn:

- If the run reaches `Stop` with no result, it takes over the task's open asks and agent waits, so
  the task sleeps on them again.
- If the run calls `result`, `ask` or `wait` instead, it moves on. The call withdraws the open asks
  and agent waits of earlier runs with the reason `moved-on`.

When a firing finds the task in any other state, the inbox records it as `coalesced` and nothing is
typed. That covers a running task, a task asleep at a human step, and a task asleep on the daemon's
checks gate. So firings never stack up.

## Feedback reopens a done task

Feedback is one of these events:

- on a pull request: `github.pull_request_review.submitted`, `github.pull_request.comment` or
  `github.pull_request_review_comment`. A review whose `state` is `approved` or `dismissed` asks
  for nothing, so it is not feedback here;
- on an issue: `github.issues.comment`;
- on a Linear issue: `linear.comment.created`;
- in a Slack thread: `slack.thread.replied`.

When no active task owns a resource it names, it reopens the newest done task that:

- owns the pull request it is on, as its key, through `open-pr`, a claim, a wait or a
  subscription; or, for a comment on an issue, a Linear issue or a Slack thread, subscribes to it,
  as every task does to its own key until it unsubscribes;
- became done at or before the feedback's `time`, to the second;
- has no `github.pull_request.closed` or `github.issues.closed` event of that pull request or issue
  in the inbox, merged or not, since the last event that read it open, and
  the feedback's `open` is not false, so a close the intake publishes after it reopens nothing.
  Feedback whose `open` is true was read on an open pull request, so an earlier close does not
  count;
- has no merged pull request, so a "Shipped, thanks" on the Linear issue or in the Slack thread a
  week later starts no new generation on a merged branch;
- shares its key with no active task.

The task goes back, as its next generation, to its workflow's `reopen` step, else the model step
whose result led to the `open-pr` that opened the pull request, else the step it was done at. As `priory revive` does, the
reopen withdraws the open asks, waits and queued prompts of the earlier generation. Its first
prompt is one line, then the event's own line:

```text
Feedback reopened this done task as generation 2: ana commented on github:owner/name#57: Can you split this? https://github.com/owner/name/pull/57#issuecomment-701
Feedback reopened this done task as generation 2: Ana commented on linear:ENG-7: Still broken. https://linear.app/acme/issue/ENG-7
Feedback reopened this done task as generation 2: Ana replied in slack:C0123:1763500000.000100: Still broken.
```

The task is `ready`, so its next run resumes the session of the step's agent. The daemon appends a
`factory.task.reopened` trace record (see [trace.md](trace.md)), and posts a report that keeps the
reason on the desk until it is read: `task 12 (linear:ENG-7) reopened as generation 2: ana commented
on ...`.

A person reopens a done task with [`priory say`](desk.md#say) while its pull request is open.

A workflow with
[`reopen_on_changes_requested = true`](../reference/workflows.md#top-level-keys) also reopens for a
review that requests changes by someone without write access, such as a GitHub App. The same rules
hold, and no active task may own the pull request. Anyone can review a pull request of a public
repository, so the review's words stay out of the prompt. The prompt names its author and URL and
says to read it as data. The agent may still open the URL and read the review there.

A prompt that carries feedback, on a reopen or any wake, ends with the rule to sort each item as
`trivial`, `nothing-to-do` or `persons-call`, and the result records the classes (see [Review
feedback](../reference/tools.md#review-feedback)).

Limits:

- A GitHub issue closed and reopened reopens nothing when the intake polls it, since a polled
  issue comment carries no `open`. A webhook event carries it.
- The intake hears a done task's Linear issue or Slack thread only while something watches it,
  such as the subscription to its key (see [watches.md](../intake/watches.md)).
- The intake publishes no bot's comment, and only feedback after the task was done reopens it. A
  person who comments as the same GitHub account Priory acts as reopens it like anyone else.
- Only feedback from someone with write access to the repository reopens or wakes a task, but for
  the opt-in above: GitHub's
  `author_association` must be `OWNER`, `MEMBER` or `COLLABORATOR`. A stranger's comment on a public
  repository stays in the inbox as `outsider`, for a person to read with `priory inbox
  --unrouted`. The Desk counts them. No other factory setting lets other authors through.

## Feedback on a failed or dropped task

Feedback never reopens a `failed` or `dropped` task: a person stopped it, or its workflow did. When
a comment or reply lands on the key of the newest task on that key, and that task failed or was
dropped before the comment was written, the inbox records the comment as `ignored` and posts one
`priory.report` that names the revive:

```text
priory: linear:ENG-7 has a new comment; its task 12 dropped: priory revive 12
```

A Linear issue that never left its view publishes no new `issue.entered`, so this report is how
"I added the missing criterion" reaches a person. The intake keeps reading that task's key for it
(see [watches.md](../intake/watches.md)). Priory's own comment posts no report.

