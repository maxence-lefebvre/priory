# Asks, waits and checks

## Asks and answers

`priory asks` prints each open ask: id, task, revision, question, options, then the fields of
[`ask`](tools.md#the-tools) and of an outside ask.
`priory answer <ask> <option-or-text> --revision <n>` sends the answer to the daemon. `--expected`
is the same flag, and `--` ends the flags, as in `priory answer --expected 2 -- cache.ship ship`.
The first answer to the ask's revision stands. Refused: an answer to another revision, a second
answer, or, for a step's ask, a text that is not one of its options. In a factory of several
runners, `priory answer <runner>:<ask>` answers another runner's ask through the log (see
[runners.md](runners.md#answering-another-runners-ask)).

`priory dismiss [--expected <n>] <ask>` closes an open ask with no answer. With `--expected`, a
changed question refuses it, as it refuses an answer. A dismissed ask is withdrawn: an answer to
it is refused with `ask <id> was dismissed`. An agent's ask wakes its task with one prompt:
`A person dismissed ask <id> without an answer. Continue your task.` A step's ask and a hold's
ask are refused, since only one of their options moves the task on.

- A step's ask moves the task by `next[option]`, as in the table. A model step makes the task
  `ready` with that step's prompt queued.
- An agent's ask queues the answer as the task's next prompt, after fixed words such as
  `Answer to ask 12 (revision 2): `, so an answer starting with `/` never runs as a slash command.
  An asleep task becomes `ready`. A task whose run has not stopped yet stays `running`, and the
  answer is typed after its `Stop`.

A `ready` task's next run resumes the session of the current step's agent and types its queued
prompts.

## Waits

A wait ends with the first event that fires it. To hear every event on a resource without ending
the turn, a task subscribes instead (see `subscribe` in [tools.md](tools.md#subscribeon-and-unsubscribeon)).

`wait(on)` names one condition in `on.type`. `wait(any)` names a list of them, and the task wakes
on the first that fires. Each condition fires the wait on:

| `type`         | Fields                  | Fires on                                                                |
| -------------- | ----------------------- | ----------------------------------------------------------------------- |
| `pull_request` | `repo`, `pull_request`  | `github.pull_request.closed` for it, merged or not, or feedback on it (see below) |
| `checks`       | `repo`, `pull_request`, optional `head_sha` | `github.check_run.completed` for it, with `pending` 0, on `head_sha` if given |
| `review`       | `repo`, `pull_request`  | `github.pull_request_review.submitted` for it, or a comment on it (see below) |
| `time`         | `until`                 | the daemon's loop, once `until` has passed                              |
| `task`         | `task`                  | the daemon's loop, once that task is done, failed or dropped            |
| `linear_comment` | `issue`, such as `ENG-7` | `linear.comment.created` on that issue                               |
| `linear_state` | `issue`, `state`, such as `Done` | `linear.issue.state_changed` on that issue, with `to` equal to `state` |
| `slack_reply`  | `channel`, `thread_ts`  | `slack.thread.replied` in that thread, which the intake never publishes for its own bot |
| `disk`         | `gb`, a number above 0  | the daemon's loop, once the room's disk reading shows `gb` GB free (a GB is 1024³ bytes) |

```json
{ "on": { "type": "linear_state", "issue": "ENG-7", "state": "Done" } }
{ "on": { "type": "slack_reply", "channel": "C0123", "thread_ts": "1763500000.000100" } }
{ "any": [
  { "type": "pull_request", "repo": "owner/name", "pull_request": 57 },
  { "type": "linear_state", "issue": "ENG-7", "state": "Done" },
  { "type": "linear_state", "issue": "ENG-7", "state": "Canceled" }
] }
```

A wait on `any` stores one wait per condition, each with its own id, and replies
`waits <id>, <id> stored`. A condition that is refused refuses the whole call, and nothing is
stored. When one of them fires, the daemon withdraws the others, so a later event on their
resources fires no wait. The task still owns those resources, so a later event on one still reaches
the agent as a line of a prompt, as any event on a resource it owns does.

### Feedback on a pull request

Feedback is a review, a comment or a review thread comment on the pull request:
`github.pull_request_review.submitted`, `github.pull_request.comment` or
`github.pull_request_review_comment`. It fires a `pull_request` or `review` wait, and the checks
gate before `done`, with one line that names who said what and where. A review whose `state` is
`approved` or `dismissed` does not fire the checks gate, since it asks the agent for nothing:

```text
wait 3 fired: ana reviewed github:owner/name#57: changes_requested: Split the parser. https://github.com/owner/name/pull/57#pullrequestreview-301
wait 3 fired: ana commented on github:owner/name#57: Can you split this? https://github.com/owner/name/pull/57#issuecomment-701
wait 3 fired: ana commented on src/lib.rs:12 of github:owner/name#57: Rename this. https://github.com/owner/name/pull/57#discussion_r802
```

The line quotes the first line of a comment or of a review's summary, when the review has one.
The event's own line follows with the whole body.

A comment on an issue or a Linear issue, or a Slack reply, that the task subscribes to fires the
checks gate the same way, so a request made while a stack waits to merge reaches the agent:

```text
wait 3 fired: Ana commented on linear:ENG-7: Also handle the empty case. https://linear.app/acme/issue/ENG-7#comment-1
```

A comment fires a wait only when its `time` is at or after the wait was stored, to the second. So
the agent's own comment, posted in the turn that stored the wait, fires nothing. Reviews fire at any
time, since GitHub keeps an author from approving or rejecting their own pull request. The intake
publishes no bot's comment (see [pull-requests.md](../intake/pull-requests.md)).

### An ask beside a wait

An agent can call `ask` and `wait` in the same turn, in either order. The task sleeps until the
answer or a condition comes, and the first one wins:

- An answer or a dismiss withdraws the run's open waits.
- A wait that fires withdraws the run's open ask. A later `priory answer` is refused with
  `ask <id> was withdrawn when a wait of its run fired first`, and `priory asks --all` shows the
  ask as withdrawn for that reason.

While an answer or a fired wait of the run sits in a queued prompt, a new `ask` or `wait` is
refused: the agent ends its turn and reads it first.

### Why an ask or wait was withdrawn

A withdrawn ask or wait keeps `withdrawn` set to 1, names why in `withdrawn_reason` and when in
`withdrawn_at`. `priory asks --json`, `/api/asks` and `priory task <id> --json` show both. An ask or
wait that is asked or set again clears all three. Schema 71 filled `withdrawn_at` for older asks
from their withdrawal log, and left it null for older waits, which have none. The reason is one of:

| Reason | On | What withdrew it |
| ------ | -- | ---------------- |
| `dismissed` | ask, wait | A person dismissed the ask. Its run's open waits go with it |
| `answered-elsewhere` | wait | A person answered the ask of the wait's run |
| `sibling-fired` | ask, wait | Another wait of the same run fired first, or another gate of the stack fired |
| `moved-on` | ask, wait | A later run of the task called another tool |
| `revived` | ask, wait | A person revived the task, or feedback reopened it, as its next generation |
| `asked` | wait | The daemon asked a person about the task's stack, which ends its gates |

State files before schema 58 kept the reason as a number in `withdrawn`. The upgrade names it: -1
is `dismissed`, -2 `sibling-fired`, 0 `moved-on` and a generation `revived`. A wait withdrawn by an
answer, a dismiss or a stack ask before then also held -2. Schema 64 corrects those where the state
shows it: a wait whose run had no fired wait becomes `dismissed` when its run's ask was dismissed,
else `answered-elsewhere` when that ask was answered. A checks gate of a task none of whose gates
ever fired becomes `asked`. Any other old -2 wait still reads `sibling-fired`.

A Linear or Slack condition makes the task own `linear:<issue>` or `slack:<channel>:<thread_ts>`, as
a pull request condition does, and is refused when another active task owns it. It stores the time
of the agent's last prompt. The wait also takes the events on its resource from then on that the
daemon ignored because no task owned it. A `linear_state` event at or after that time, to the
second, fires the wait. A `linear_comment` or `slack_reply` event fires it only when its `time` is
at or after the wait was stored, as a pull request comment does. So the agent's own question,
posted in the turn that stored the wait, neither fires it nor withdraws the ask beside it. A person's
reply posted in that same turn fires nothing either: the agent reads it in its next prompt. The
intake watches the issue or thread from then on, as `docs/intake/watches.md` describes.

`until` is an RFC 3339 instant, or a duration such as `30 seconds`, `5 minutes` or `2 hours`: an
SQLite date modifier that Priory adds to the time of the call. An unknown type or missing field is
refused, and nothing is stored. A pull request condition makes the task own the pull request, and is
refused when another active task already owns it.

The intake publishes one `github.check_run.completed` per check run, only once it completed, so the
daemon cannot count runs it has not seen. The intake reads every check run on the head commit in the
same poll, so each check event carries `pending`: how many of them had not completed then. "Every
check run has completed" is a check event with `pending` 0. Two runs that complete between two polls
both carry 0: the first fires the wait, and the second is kept. A check run GitHub has not created
yet is not counted. A pull request with no check runs never fires a checks wait. A checks wait
without `head_sha` takes a check event on any head, so a late event for an older head can fire it.
Over 100 check runs and statuses on a commit keep `pending` above 0, so such a pull request never
fires a checks wait.

When the wait is stored, the daemon checks the events the task kept since the agent's last prompt:
those not typed yet and on which no wait fired. The first one that satisfies the wait fires it at
once, and the time and task conditions are checked at once too. The daemon also reads the current
state of two conditions. Each read runs on a thread of its own once the wait is stored, so a slow
read never holds the daemon's loop or the tool call:

| Condition | Read | Fires when | Line |
| --------- | ---- | ---------- | ---- |
| `linear_state` | the issue's state, through the intake's Linear GraphQL transport (`curl`, 10 second connect and 60 second total timeout), only when the daemon has `LINEAR_API_KEY` | the state equals `state` | `wait <id> fired: <issue> is already <state>` |
| `pull_request` | `gh pr view <number> -R <repo> --json state`, under the factory's command timeout | the pull request is merged or closed | `wait <id> fired: <pull request> is already merged`, or `closed` |

So a wait on a Linear blocker that is already Done wakes the task within a second or two of the
call, and a person's answer or another condition that came first still wins. A read that fails
leaves the wait open and logs `cannot read the current state of wait <id>` on the daemon's stderr.
The daemon reads each open wait once per start, so a restart reads them again. The other conditions
are events, such as a comment or a review, with no current state to read. The run then does not sleep: the
event is typed after its `Stop`. A disk wait is checked at the next tick. Each tick measures the disk
with the room's reading, `df` on `PRIORY_HOME`, only while a disk wait is open.

A wait that fires queues one prompt ahead of the task's other queued prompts: the event's line. For
time, task and disk waits, it is `wait <id> fired: <until> passed`, `wait <id> fired: task <id> is <state>` or `wait <id> fired: <N> GB of disk free`. Then a line for each event the task kept while it slept. A wait fires in any active state. An asleep task becomes `ready`, and its next run resumes the session of its step's agent with that prompt. A ready, blocked or held task keeps its state, and its next run gets that prompt. An event that does not satisfy the wait stays kept until the wake.

A check event also carries `failing`: how many of the completed check runs on the head commit had a
conclusion other than `success`, `neutral` or `skipped` in the same poll.

`priory waits` prints each open wait: id, task, run, kind, what it waits on, such as `50.0 GB free`
for a disk wait, and when it was stored.

## Done and checks

A task owns a pull request when its `open-pr` opened it, when one of its waits named it, when an
event it kept or that woke it comes from a pull request on its branch, or when a pull request event
created it. An opened one comes first, then a waited one, then the latest from its branch. When such a task reaches
`done`, by a result, an answer or an action, it is not done yet. The daemon stores its own checks
wait on that pull request, pinned to the `commit` of the task's latest result, else to the commit
that `open-pr` pushed after that result. The task sleeps with no process, at that result's step, so
the agent that made it fixes what the checks find. A task with neither commit is blocked instead.

The daemon judges this wait on every event the inbox kept, ignored or woke a task on about the pull
request. That includes events the agent already read, such as a check that woke it through a
subscription, and those an earlier task kept. It judges when it stores the
wait and at each new event:

| Condition                                                                           | Effect                                 |
| ----------------------------------------------------------------------------------- | -------------------------------------- |
| The new event is feedback on the pull request (see [Feedback on a pull request](#feedback-on-a-pull-request)) | wakes the agent with the feedback's line |
| The newest event is `github.pull_request.closed` with `merged` true                 | marks the task done and starts no run  |
| The newest event is `github.pull_request.closed` without `merged` true              | wakes the agent: `wait <id> fired: <pull request> closed without a merge`, or `closed, and GitHub no longer says whether it merged` when `merged` is null |
| Else, the newest event names another head and came after the result              | wakes the agent: `wait <id> fired: the head of <pull request> moved to <sha>, away from your result's commit <commit>` |
| Else, the newest event names another head and came before the result             | the task sleeps on, since a later event tells whether the head is the commit |
| Else, the newest check at the commit is red, or counts `failing` and a red check at the commit was seen | wakes the agent with that red check: `wait <id> fired: check <name> at <commit> concluded <conclusion> <url>` |
| Else, the newest check at the commit has `pending` 0 and `failing` 0             | marks the task done and starts no run  |
| Else, no check at the commit, and the newest `github.pull_request.check_runs` snapshot at it has `runs` 0 | marks the task done and starts no run  |
| Otherwise                                                                           | the task sleeps on                     |

When the wait marks the task done while a person's `say` has a run going, that run ends as `done`
and its session is killed.

Each wake line is followed by the event's own line, unless a queued prompt already carries it. The
woken agent can fix the failure and send a new result, which stores a new checks wait at its new
commit. Old-commit checks that finish before that result are history, so they do not count as a
moved head. One that the intake reports after the result does, since the daemon cannot tell it from
a head moved back.

The `result` tool refuses an outcome that leads to `done`, or to a step with `after = "merged"`,
without `commit` when the task owns a pull request that has not merged yet.

A step with `after = "merged"` stores the same gates when the task reaches it. Green checks then
leave the task asleep, and the merge that would mark it done enters that step instead. When a
person's `say` started a run while the task slept, the merge moves that run as a result would: it
carries on into a model step of the same agent, and otherwise ends and its session is killed. See
[Steps after the merge](../reference/workflows.md#steps-after-the-merge).

### A stack of pull requests

When the task's [stack](routing.md#stacks) holds two or more pull requests, the daemon stores one
checks wait per open pull request of the stack instead. The top one is pinned to the commit as
above. A lower one is judged at its newest head, since a restack moves it, so its head never counts
as moved. A stack with a pull request closed without a merge that no person decided on asks first,
as below, and stores no gate. A stack with no open pull request left is done at once. Each wait is
judged only on events of its own pull request, as in the table above, with these differences:

| Condition | Effect |
| --------- | ------ |
| Green checks at the head | the task sleeps on: a stack is done only once every pull request merged. When this makes every open gate of the stack green, the daemon posts one report from `priory`: `the stack of task 1 (github:owner/repo#42) is green: merge #57 first`, naming the lowest open pull request |
| `github.pull_request.closed` with `merged` true, and every pull request of the stack merged | marks the task done and starts no run |
| `github.pull_request.closed` with `merged` true, and open pull requests above it | wakes the agent: `#57 merged; rebase #58 and #59 onto main`, where `main` is the merged pull request's base |
| `github.pull_request.closed` with `merged` true, and none open above it | the task sleeps on |
| `github.pull_request.closed` without `merged` true | asks a person, see below. Once they answered, it wakes the agent as in the table above |
| The bottom pull request's base is no longer the branch it was opened or claimed on | asks a person, see below |

The two asks are a person's to answer, since only they know where the stack should land. Each is
an agent ask on the task with the options `restack onto <base>` and `stop`, where `<base>` is the
base of the closed or retargeted pull request. Each is named so that a repeated event never asks
twice: `stack-<task>-closed-<number>` and `stack-<task>-retargeted-<number>`. For example: `#57 (1
of 3) closed without a merge, and #58 and #59 are based on it. What should the worker do?` The ask
ends the task's gates. The answer wakes the agent as any answer does, and its next `done` stores
fresh gates. A person's answer stands: a later event about the same pull request asks nothing
again. A dismissal does not stand: when the same pull request still needs a decision at the next
`done` or gate event, the ask comes back as its next revision. So does an ask that a revive or
reopen withdrew.

A pull request that replaces a closed one on the same branch takes its place in the stack, and a
pull request that is open again after a close is open in the stack again.

When one gate wakes the agent, the task's other gates end with it. A lower pull request that merges
while the task runs or sleeps on its own wait reaches it as the close event's line, with its place in
the stack, not as the restack line.

Known limits:

- Priory reads no branch protection, so every check run reported for the head counts as required.
  Commit statuses are not check runs and do not count.
- A head with no check runs finishes the task only once the intake publishes its snapshot: at least
  5 minutes after the pull request's last update, and later when polls fail. A check run that appears
  after the task finished does not count.
- A head pushed again after it had a snapshot with `runs` 0 reuses that snapshot, so a result at
  that head can finish the task within the grace period.
- An agent's own checks wait does not fire on `github.pull_request.check_runs`. Only the done gate
  takes it.
- A check event from an intake older than `failing` cannot mark a task done.
- Feedback after the task is done reopens it instead (see
  [routing.md](routing.md#feedback-reopens-a-done-task)).
