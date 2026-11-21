# Pull requests

The intake publishes eight pull request events. Each names the pull request and its branches, so a
consumer can route it to the task that owns the pull request or branch.

## What it watches

| Event | Read from | Stream |
| ----- | --------- | ------ |
| opened | the repository's `issues` list | `github.issues.opened` |
| label added, closed | the repository's `issues/events` | `github.issues.labeled` |
| reviews and comments | a GraphQL query of every open pull request, and every pull request a new event above names | `github.pull_request_review.submitted` |
| review thread comments | the repository's `pulls/comments`, newest 100, kept for the pull requests the query read | `github.pull_request_review.submitted` |
| check runs and snapshots | the same query, on each pull request's head commit | `github.check_run.completed` |

An open wait, a `--merge` hold, a subscription or an active task's branch adds its repository to the
polled ones (see [watches.md](watches.md)). Polling and API cost are in [github-polling.md](github-polling.md).

## Trigger

A `github` trigger can start a task from two pull request events. Its keys are in
[triggers.md](../reference/triggers.md#github).

| `on` | `filter` | `key` |
| ---- | -------- | ----- |
| `pull_request.labeled` | `label`, optional | `pull_request.number` |
| `pull_request.opened` | none | `pull_request.number` |

No trigger starts a task from a close, a review, a comment or a check. Their `triggers` is always empty.

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
| `github.check_run.completed` | completed check run on a pull request's head commit, per pull request |
| `github.pull_request.check_runs` | head whose check runs first appeared, or that has none past the grace period |

`merged` on a close is true only when the pull request is merged and this is its last `closed`
event the poll read.

### Check runs, not suites

The intake reads check runs, not check suites. Branch protection and R6.6 name required checks by
check run, such as `test (ubuntu)`, while a suite folds every run of one app into one conclusion.
Commit statuses are not published. A re-run gets a new check run id, so it publishes again.

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

- Head and base fields come from the pull request when the intake polls, not when the event
  happened. They are null when GitHub no longer returns the pull request.
- Reviews and checks are read for open pull requests and for any pull request a new issue event
  names, such as its `closed` event. A check that completes after its pull request closed and a
  poll passed is missed.
- The query reads the last 50 reviews, the last 50 comments and the first 100 checks of the head
  commit per pull request.
- A poll reads the repository's newest 100 review thread comments. More than 100 between two polls
  loses the oldest, unless a webhook delivered them.
- A comment's author is a bot when GitHub types the account `Bot`. A person or an agent that
  comments through a personal access token is not one.
