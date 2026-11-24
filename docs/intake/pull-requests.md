# Pull requests

The intake publishes eight pull request events. Each names the pull request and its branches, so a
consumer can route it to the task that owns the pull request or branch. The daemon also reads its
`base_branch` to claim a stack a worker opened itself, and to tell when a stack's bottom pull
request was retargeted (see [Stacks in routing.md](../daemon/routing.md#stacks)).

## What it watches

| Event | Read from | Stream |
| ----- | --------- | ------ |
| opened | the repository's `issues` list | `github.issues.opened` |
| label added, closed | the repository's `issues/events` | `github.issues.labeled` |
| reviews and comments | a GraphQL query of every open pull request, and every pull request a new event above names | `github.pull_request_review.submitted` |
| review thread comments | the repository's `pulls/comments`, newest 100, kept for the pull requests the query read | `github.pull_request_review.submitted` |
| check runs and snapshots | the same query, on each pull request's head commit | `github.check_run.completed` |
| check runs at a merge | the commit's `check-runs`, for each task pull request merged in the last 7 days | `github.check_run.completed` |

An open wait, a `--merge` hold, a subscription or an active task's branch adds its repository to the
polled ones (see [watches.md](watches.md)). Polling and API cost are in [github-polling.md](github-polling.md).

## Trigger

A `github` trigger can start a task from five pull request events. Its keys are in
[triggers.md](../reference/triggers.md#github).

| `on` | `filter` | `key` |
| ---- | -------- | ----- |
| `pull_request.labeled` | `label`, optional | `pull_request.number` |
| `pull_request.opened` | none | `pull_request.number` |
| `pull_request.comment`, `pull_request_review_comment`, `pull_request_review.submitted` | `author`, and `commenter`, optional | `pull_request.number` |

A comment or review matches a trigger when its pull request is open and the pull request's
`author` is the filter's `author`.
With `commenter`, the comment's `actor` must be that login too. Without it, a bot's review matches
no trigger. A comment or review by someone without write access, whose `association` is not
`OWNER`, `MEMBER` or `COLLABORATOR`, matches no trigger either, and neither does one marked `agent`.
Its task request starts a task only
when no active task owns the pull request (see
[routing.md](../daemon/routing.md#comments-on-a-pull-request-no-task-owns)).

No trigger starts a task from a close or a check. Their `triggers` is always empty.

## Events

Their fields and examples are in [events.md](../reference/events.md#github).

| Type | One message per |
| ---- | --------------- |
| `github.pull_request.opened` | pull request opened |
| `github.pull_request.labeled` | label added to a pull request |
| `github.pull_request.closed` | pull request closed, merged or not |
| `github.pull_request_review.submitted` | review submitted; pending reviews are skipped |
| `github.pull_request.comment` | comment on the conversation; bots' comments are skipped |
| `github.pull_request_review_comment` | comment in a review thread; bots' comments are skipped |
| `github.check_run.completed` | completed check run on a pull request's head commit, or on its merge commit, per pull request |
| `github.pull_request.check_runs` | head whose check runs first appeared, or that has none past the grace period |

A review, a comment and a review thread comment carry `author`, the login that opened the pull
request, when GitHub names one. A review by a bot also carries `bot: true`.

They carry `own: true` when the login `gh` runs as wrote them, and `agent: true` when their body
also holds the line `<!-- agent-comment -->`. Agents post as the person's own login, so the marker
tells an agent's comment from the person's. The marker alone, from another login, proves nothing.
See [routing.md](../daemon/routing.md#an-agents-own-comments).

GitHub wraps each reply in a review thread in a `COMMENTED` review with an empty body. The marker is
in the thread comment, so the intake judges such a review by its comments when `gh`'s login wrote it:

| Its thread comments | The review |
| ------------------- | ---------- |
| all marked | `agent: true`, so it starts no task |
| all marked, and each is already on `factory.intake` | not published, except in a first poll that has an unread review: there it is `agent: true`, so the next poll need not look it up |
| some unmarked, or none | the person's: `agent: false` |

A review with a body is judged by its body. So is another login's review, which is never `agent`.
The comments take one REST call per review (see [github-polling.md](github-polling.md#calls-and-cost)).

An empty list for a review under a minute old is unread: GitHub can report a review before it lists
the review's comments. A later poll reads it again.

`merged` on a close is true only when the pull request is merged and this is its last `closed`
event the poll read.
A close also carries `merge_commit`, the commit the merge made on the base branch, or null unless
`merged`, and `body`, the description. [Outcome labels](../daemon/trace.md#outcome-labels) read both
to find a revert.

### Check runs, not suites

The intake reads check runs, not check suites. Branch protection and R6.6 name required checks by
check run, such as `test (ubuntu)`, while a suite folds every run of one app into one conclusion.
Commit statuses are not published. A re-run gets a new check run id, so it publishes again.

### Check runs at a merge

A check that fails on the base branch after a merge is how a task turns out red
([`red-after-merge`](../daemon/trace.md#outcome-labels)). A pull request's own query stops at its
head commit, so the intake also reads the merge commit. The same `github.check_run.completed` event
carries the merge commit as `head_sha` and the branch it merged into as `base_branch`. A failing one
also raises a `landed wrong` alarm on the desk, once per merge commit (see
[trace.md](../daemon/trace.md#outcome-labels)).

- It reads the pull requests that a task owns, by its stack or its head branch, and that merged in the
  last 7 days, in a repository it polls. It learns them from the state file, so without `PRIORY_DB` it reads none.
- It makes one call per merge commit and poll, `repos/{repo}/commits/{sha}/check-runs`, up to 100 runs.
- It stops reading a commit once a poll saw every run completed and the daemon stored every failing event of it. It also stops, until the next start, when the commit has no run 5 minutes after the merge.
- A restart reads once more only a commit that had no run, since the daemon's record of the others persists.
- A merge into a branch other than the default counts too: the event names the pull request's base branch as it is.
- Past 100 runs on a commit, the first 100 are read and `pending` stays above 0, so the commit is read until the 7 days end.
- A `cancelled` run, as a workflow's concurrency group leaves one when a newer push supersedes it,
  counts as red only when no later run of the same name succeeded. The intake reads that name's run
  on the tip of the base branch, `repos/{repo}/commits/heads/{base}/check-runs?check_name={name}`
  with `{base}` percent-encoded, and checks with `repos/{repo}/compare/{merge}...{tip}` that the
  tip descends from the merge commit. A success there publishes the cancelled run with
  `superseded_by` set to the tip. It then counts as passing and is left out of `failing`. A tip run
  that is still running or was cancelled too keeps the cancelled run pending, so the commit is read
  again on the next poll. So does a tip with no run of that name, for 5 minutes after the cancel,
  while a newer push's run may not exist yet. Any other answer, and any failed read, such as a 404
  for a base branch deleted after the merge, publishes the cancelled run as it is.

### Check run snapshots

A repository that runs no checks never produces a `github.check_run.completed` event, and a queued
or running check produces none yet. So the intake publishes `github.pull_request.check_runs`, a
snapshot of a head's check runs, at most twice per pull request and head:

| When | `runs` |
| ---- | ------ |
| The first poll that finds any check run on the head, completed or not | above 0 |
| The first poll that finds none, with the pull request not updated for 5 minutes | 0 |

The 5 minutes are the grace period, `GRACE_SECONDS` in `crates/priory-intake/src/intake/pulls.rs`.
GitHub creates check runs a few seconds after a push, and a push updates the pull request, so the
grace period counts from the last push or later.

- Commit statuses do not count as check runs, but contexts past the first 100 do.
- When an event names a pull request that is also open, the poll takes the snapshot from the open
  list, the later of its two reads.

## Keys and ids

| What | Value |
| ---- | ----- |
| Task key | `github:owner/name#43`. GitHub numbers issues and pull requests from one sequence |
| Subject | `owner/name#43` |

Each message id is the UUIDv5 of a name:

| Type | Name |
| ---- | ---- |
| `github.pull_request.opened` | `github:{repo}:issues:{github_id}:opened` |
| `github.pull_request.labeled`, `github.pull_request.closed` | `github:{repo}:issues/events:{github_id}` |
| `github.pull_request_review.submitted` | `github:{repo}:pulls/reviews:{github_id}` |
| `github.pull_request.comment` | `github:{repo}:issues/comments:{github_id}` |
| `github.pull_request_review_comment` | `github:{repo}:pulls/comments:{github_id}` |
| `github.check_run.completed` | `github:{repo}:check-runs:{github_id}:pulls/{pull_request}`, since one check run can sit on the head of two pull requests |
| `github.pull_request.check_runs` | `github:{repo}:pulls/{pull_request}:check-runs:{head_sha}:{some or none}` |

## Limits

- A check run that starts after every run on a merge commit completed is not read.
- Head and base fields come from the pull request when the intake polls, not when the event
  happened. They are null when GitHub no longer returns the pull request.
- Reviews and checks are read for open pull requests and for any pull request a new issue event
  names, such as its `closed` event. A check that completes after its pull request closed and a
  poll passed is missed.
- The query reads the last 50 reviews, the last 50 comments and the first 100 checks of the head
  commit per pull request.
- A poll reads the repository's newest 100 review thread comments. More than 100 between two polls
  loses the oldest, unless a webhook delivered them.
- Of the thread comments a review wraps, the first 100 are read.
- A review whose thread comments cannot be read is not published. The next poll tries again, if it
  still reads the pull request. After 10 failed tries it counts as the person's, so an agent's reply
  can then start a task. `priory intake --once` keeps no tries, so a lookup that fails for ever
  fails each run and keeps a first poll a baseline.
- While `gh` cannot name its login, no review is judged by its comments.
- A review dropped as a repeat is not on `factory.intake`, so each restart looks it up again.
- A comment's or review's author is a bot when GitHub types the account `Bot`. A person or an
  agent that comments through a personal access token is not one.
