# Events

The intake publishes every message on the herdr-bus output `factory.intake`. Each message is a
CloudEvent-shaped envelope whose `data` depends on its `type`. This page lists every type, with its
subject, its `data` fields and one example. The [intake pages](../intake/README.md) explain when
each one is published.

Each type has a JSON Schema in [`docs/schemas/events/`](../schemas/events/), named
`<type>.json`. The intake's tests check every message they publish against it.

- [The envelope](#the-envelope)
- GitHub: [`github.issues.labeled`](#githubissueslabeled), [`github.issues.opened`](#githubissuesopened),
  [`github.issues.closed`](#githubissuesclosed), [`github.issues.comment`](#githubissuescomment),
  [`github.pull_request.opened`](#githubpull_requestopened), [`github.pull_request.labeled`](#githubpull_requestlabeled),
  [`github.pull_request.closed`](#githubpull_requestclosed), [`github.pull_request_review.submitted`](#githubpull_request_reviewsubmitted),
  [`github.pull_request.comment`](#githubpull_requestcomment),
  [`github.pull_request_review_comment`](#githubpull_request_review_comment),
  [`github.check_run.completed`](#githubcheck_runcompleted), [`github.pull_request.check_runs`](#githubpull_requestcheck_runs),
  [`github.pull_request.review_requested`](#githubpull_requestreview_requested),
  [`github.pull_request.review_request_removed`](#githubpull_requestreview_request_removed)
- Linear: [`linear.issue.entered`](#linearissueentered), [`linear.issue.left`](#linearissueleft),
  [`linear.view.changed`](#linearviewchanged),
  [`linear.issue.created`](#linearissuecreated), [`linear.issue.state_changed`](#linearissuestate_changed),
  [`linear.issue.updated`](#linearissueupdated), [`linear.issue.blockers`](#linearissueblockers),
  [`linear.comment.created`](#linearcommentcreated)
- Slack: [`slack.message.posted`](#slackmessageposted), [`slack.thread.replied`](#slackthreadreplied)
- Others: [`schedule.fired`](#schedulefired), [`priory.task.requested.cli`](#priorytaskrequestedcli),
  [`factory.task.requested`](#factorytaskrequested), [`factory.intake.baseline`](#factoryintakebaseline)
- Runners: [`priory.task.claimed`](#priorytaskclaimed), [`priory.task.held`](#priorytaskheld),
  [`priory.runner.alive`](#prioryrunneralive), [`priory.ask.answered`](#prioryaskanswered),
  [`priory.ask.posted`](#prioryaskposted)

## The envelope

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | UUID string | A UUIDv5 of the message's name, so a retried publish keeps its id. See [envelope.md](../intake/envelope.md) |
| `type` | string | The type, such as `github.issues.labeled` |
| `subject` | string | A short line for listings; each type below gives its own |
| `time` | RFC 3339 | When the event happened |
| `data` | object | The fields below, by type |
| `links` | array | GitHub and Linear events carry one `{ "label", "url" }` to the event's page; others carry none |

Every GitHub, Linear and Slack event ends its `data` with the same three fields:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `time` | RFC 3339 | When the event happened; for a view or review request event, when the intake polled |
| `baseline` | boolean | True when it came from the first poll of its stream |
| `triggers` | array of strings | The triggers it matched, so the triggers it requests tasks for; may be empty |

A field typed "or null" is always present. A field marked optional may be absent.

## GitHub

The pull request types share these fields. Head and base fields are null when GitHub no longer
returns the pull request.

Every type but the check run ones also carries `delivery`, a string, when a webhook delivery
published it: its `X-GitHub-Delivery`. See [webhooks.md](../intake/webhooks.md).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string | `owner/name` |
| `pull_request` | integer | The pull request number |
| `url` | string | The pull request's page; the review's for a review, the check run's for a check |
| `head_branch` | string or null | The head branch when the intake polled |
| `head_sha` | string or null | The head commit when the intake polled; for a check, the commit it ran on |
| `base_branch` | string or null | The branch it merges into |
| `head_repo` | string, optional | The head branch's repository, `owner/name`; it differs from `repo` for a fork. Absent when the event does not say, as for a check run or a deleted fork. A fork's pull request joins no task's stack |

### `github.issues.labeled`

One message per label added to an issue. Subject: `owner/name#42`.
Schema: [`github.issues.labeled.json`](../schemas/events/github.issues.labeled.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string | `owner/name` |
| `issue` | integer | The issue number |
| `title` | string | The issue title when the intake saw the event |
| `url` | string | The issue's page |
| `label` | string or null | The label added |
| `actor` | string or null | Who added it; null for a deleted user |
| `github_id` | integer or null | GitHub's issue event id; null from a webhook |

<!-- example: events/github.issues.labeled.json -->
```json
{
  "id": "98c30eca-6e69-591d-b9ab-d4731abf371a",
  "type": "github.issues.labeled",
  "subject": "acme/widgets#42",
  "time": "2026-11-17T15:18:55Z",
  "data": {
    "repo": "acme/widgets",
    "issue": 42,
    "title": "Export fails on an empty project",
    "url": "https://github.com/acme/widgets/issues/42",
    "label": "agent",
    "actor": "octocat",
    "time": "2026-11-17T15:18:55Z",
    "github_id": 36421224333,
    "baseline": false,
    "triggers": ["agent-label"]
  },
  "links": [{ "label": "issue", "url": "https://github.com/acme/widgets/issues/42" }]
}
```

### `github.issues.opened`

One message per issue opened. Subject: `owner/name#42`.
Schema: [`github.issues.opened.json`](../schemas/events/github.issues.opened.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string | `owner/name` |
| `issue` | integer | The issue number |
| `title` | string | The title when the intake polled, which may differ from the title it opened with |
| `url` | string | The issue's page |
| `actor` | string or null | Who opened it |
| `github_id` | integer | GitHub's issue id |

<!-- example: events/github.issues.opened.json -->
```json
{
  "id": "49ef9d8d-81a0-5fe5-af06-a50c947012cf",
  "type": "github.issues.opened",
  "subject": "acme/widgets#42",
  "time": "2026-11-17T14:02:10Z",
  "data": {
    "repo": "acme/widgets",
    "issue": 42,
    "title": "Export fails on an empty project",
    "url": "https://github.com/acme/widgets/issues/42",
    "actor": "octocat",
    "time": "2026-11-17T14:02:10Z",
    "github_id": 3651177204,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://github.com/acme/widgets/issues/42" }]
}
```

### `github.issues.closed`

One message per issue closed, read from the issue events. A closed pull request is
[`github.pull_request.closed`](#githubpull_requestclosed) instead. Subject: `owner/name#42`.
Schema: [`github.issues.closed.json`](../schemas/events/github.issues.closed.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string | `owner/name` |
| `issue` | integer | The issue number |
| `title` | string | The title when the intake polled |
| `url` | string | The issue's page |
| `actor` | string or null | Who closed it |
| `github_id` | integer | GitHub's issue event id |

<!-- example: events/github.issues.closed.json -->
```json
{
  "id": "4a0f3c1e-2a36-5c1b-9d43-0d4b8f9e6a21",
  "type": "github.issues.closed",
  "subject": "acme/widgets#42",
  "time": "2026-11-18T09:12:40Z",
  "data": {
    "repo": "acme/widgets",
    "issue": 42,
    "title": "Export fails on an empty project",
    "url": "https://github.com/acme/widgets/issues/42",
    "actor": "octocat",
    "time": "2026-11-18T09:12:40Z",
    "github_id": 36433318021,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://github.com/acme/widgets/issues/42" }]
}
```

### `github.issues.comment`

One message per comment on an issue, from a person; bots' comments are skipped. A comment on a
pull request is [`github.pull_request.comment`](#githubpull_requestcomment) instead. Subject:
`owner/name#42`. Schema: [`github.issues.comment.json`](../schemas/events/github.issues.comment.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string | `owner/name` |
| `issue` | integer | The issue number |
| `url` | string | The comment's page |
| `actor` | string or null | Who commented; null for a deleted user |
| `body` | string | The comment's Markdown, as written |
| `github_id` | integer | GitHub's comment id |
| `association` | string, optional | GitHub's `author_association`, such as `OWNER`, `COLLABORATOR` or `NONE`. Feedback from anyone but `OWNER`, `MEMBER` or `COLLABORATOR` reaches no task (see [routing.md](../daemon/routing.md#outcomes)) |
| `own` | boolean, optional | Whether the login `gh api user` names wrote it; absent while `gh` cannot say. `false` wakes a task even when written during its agent's turn (see [tools.md](../daemon/tools.md)) |

<!-- example: events/github.issues.comment.json -->
```json
{
  "id": "7d2e8b45-90c4-5f3a-a1e6-3c5b2f7d9e10",
  "type": "github.issues.comment",
  "subject": "acme/widgets#42",
  "time": "2026-11-17T17:05:31Z",
  "data": {
    "repo": "acme/widgets",
    "issue": 42,
    "url": "https://github.com/acme/widgets/issues/42#issuecomment-2483019477",
    "actor": "octocat",
    "body": "It also fails when the project has one empty layer.",
    "time": "2026-11-17T17:05:31Z",
    "github_id": 2483019477,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://github.com/acme/widgets/issues/42#issuecomment-2483019477" }]
}
```

### `github.pull_request.opened`

One message per pull request opened. Subject: `owner/name#43`.
Schema: [`github.pull_request.opened.json`](../schemas/events/github.pull_request.opened.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `title` | string | The title when the intake polled |
| `actor` | string or null | Who opened it |
| `github_id` | integer or null | GitHub's issue id of the pull request; null from a webhook |

<!-- example: events/github.pull_request.opened.json -->
```json
{
  "id": "d7f22ece-2872-5e71-876d-87f5807dda0b",
  "type": "github.pull_request.opened",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:05:00Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "title": "Handle an empty project in export",
    "actor": "priory-bot",
    "time": "2026-11-17T16:05:00Z",
    "github_id": 3651302871,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43" }]
}
```

### `github.pull_request.labeled`

One message per label added to a pull request. Subject: `owner/name#43`.
Schema: [`github.pull_request.labeled.json`](../schemas/events/github.pull_request.labeled.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `title` | string | The title when the intake polled |
| `label` | string or null | The label added |
| `actor` | string or null | Who added it |
| `github_id` | integer or null | GitHub's issue event id; null from a webhook |

<!-- example: events/github.pull_request.labeled.json -->
```json
{
  "id": "7f3bf3cb-4729-5cc5-a328-8bbad0226ff0",
  "type": "github.pull_request.labeled",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:07:12Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "title": "Handle an empty project in export",
    "label": "agent",
    "actor": "octocat",
    "time": "2026-11-17T16:07:12Z",
    "github_id": 36425510102,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43" }]
}
```

### `github.pull_request.closed`

One message per close, merged or not. No trigger starts a task from it. Subject: `owner/name#43`.
Schema: [`github.pull_request.closed.json`](../schemas/events/github.pull_request.closed.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `title` | string | The title when the intake polled |
| `merged` | boolean or null | Whether this close merged it: it is merged and this is its last close the poll read |
| `merge_commit` | string or null | The commit the merge made on the base branch; null unless `merged` |
| `body` | string | The description when the intake read it; empty when it has none |
| `actor` | string or null | Who closed it |
| `github_id` | integer or null | GitHub's issue event id; null from a webhook |

<!-- example: events/github.pull_request.closed.json -->
```json
{
  "id": "8af78739-0e9c-52bd-8978-36d9b69cba29",
  "type": "github.pull_request.closed",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T17:36:14Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "title": "Handle an empty project in export",
    "merged": true,
    "merge_commit": "9c1d0e7a52b4f3e8d6a1c0b9f2e4d7a8b3c6e1f0",
    "body": "An export of a project with no layers wrote nothing. It now writes an empty file.",
    "actor": "octocat",
    "time": "2026-11-17T17:36:14Z",
    "github_id": 36429719781,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43" }]
}
```

### `github.pull_request_review.submitted`

One message per review submitted; pending reviews are skipped. Subject: `owner/name#43`.
Schema: [`github.pull_request_review.submitted.json`](../schemas/events/github.pull_request_review.submitted.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `state` | string | `approved`, `changes_requested`, `commented` or `dismissed`, at poll time |
| `body` | string | The review's summary Markdown, as written; empty when the reviewer wrote none |
| `commit` | string or null | The commit the review was written on; `head_sha` passes it once a commit is pushed after the review |
| `actor` | string or null | Who reviewed |
| `open` | boolean | Whether the pull request was open when the intake read it. A done task reopens only for feedback on an open one |
| `github_id` | integer | GitHub's review id |
| `association` | string, optional | GitHub's `author_association`, such as `OWNER`, `COLLABORATOR` or `NONE`. Feedback from anyone but `OWNER`, `MEMBER` or `COLLABORATOR` reaches no task (see [routing.md](../daemon/routing.md#outcomes)) |
| `own` | boolean, optional | Whether the login `gh api user` names wrote it; absent while `gh` cannot say. `false` wakes a task even when written during its agent's turn (see [tools.md](../daemon/tools.md)) |

<!-- example: events/github.pull_request_review.submitted.json -->
```json
{
  "id": "c8b9b416-5501-5046-ae18-57dfbfde2166",
  "type": "github.pull_request_review.submitted",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:40:03Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43#pullrequestreview-2561579718",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "state": "approved",
    "body": "Looks good. One nit inline.",
    "commit": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "actor": "octocat",
    "open": true,
    "time": "2026-11-17T16:40:03Z",
    "github_id": 2561579718,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43#pullrequestreview-2561579718" }]
}
```

### `github.pull_request.comment`

One message per comment on a pull request's conversation, from a person; bots' comments are
skipped. Subject: `owner/name#43`.
Schema: [`github.pull_request.comment.json`](../schemas/events/github.pull_request.comment.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `actor` | string or null | Who commented; null for a deleted user |
| `open` | boolean | Whether the pull request was open when the intake read it. A done task reopens only for feedback on an open one |
| `body` | string | The comment's Markdown, as written |
| `github_id` | integer | GitHub's comment id |
| `association` | string, optional | GitHub's `author_association`, such as `OWNER`, `COLLABORATOR` or `NONE`. Feedback from anyone but `OWNER`, `MEMBER` or `COLLABORATOR` reaches no task (see [routing.md](../daemon/routing.md#outcomes)) |
| `own` | boolean, optional | Whether the login `gh api user` names wrote it; absent while `gh` cannot say. `false` wakes a task even when written during its agent's turn (see [tools.md](../daemon/tools.md)) |

A webhook's message has null head and base fields: its payload names no branch.

<!-- example: events/github.pull_request.comment.json -->
```json
{
  "id": "266825f5-96a3-544d-bad7-0355d348cf9a",
  "type": "github.pull_request.comment",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:45:10Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43#issuecomment-2561579801",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "actor": "octocat",
    "open": true,
    "body": "Can you split the export fix from the rename?",
    "time": "2026-11-17T16:45:10Z",
    "github_id": 2561579801,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43#issuecomment-2561579801" }]
}
```

### `github.pull_request_review_comment`

One message per comment in a review thread on a pull request's diff, from a person; bots'
comments are skipped. Subject: `owner/name#43`.
Schema: [`github.pull_request_review_comment.json`](../schemas/events/github.pull_request_review_comment.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `actor` | string or null | Who commented; null for a deleted user |
| `open` | boolean | Whether the pull request was open when the intake read it. A done task reopens only for feedback on an open one |
| `body` | string | The comment's Markdown, as written |
| `path` | string | The file the thread is on |
| `line` | integer or null | The line of the diff's new side; null once the line is outdated |
| `thread` | integer | The thread: the id of its first comment |
| `github_id` | integer | GitHub's comment id |
| `association` | string, optional | GitHub's `author_association`, such as `OWNER`, `COLLABORATOR` or `NONE`. Feedback from anyone but `OWNER`, `MEMBER` or `COLLABORATOR` reaches no task (see [routing.md](../daemon/routing.md#outcomes)) |
| `own` | boolean, optional | Whether the login `gh api user` names wrote it; absent while `gh` cannot say. `false` wakes a task even when written during its agent's turn (see [tools.md](../daemon/tools.md)) |

<!-- example: events/github.pull_request_review_comment.json -->
```json
{
  "id": "6bc673bd-853a-5fac-aab6-f0d3fb641952",
  "type": "github.pull_request_review_comment",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:47:30Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43#discussion_r1890000002",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "actor": "octocat",
    "open": true,
    "body": "This returns early on an empty sheet. Keep the header row?",
    "path": "src/export.rs",
    "line": 42,
    "thread": 1890000001,
    "time": "2026-11-17T16:47:30Z",
    "github_id": 1890000002,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43#discussion_r1890000002" }]
}
```

### `github.check_run.completed`

One message per check run completed on a pull request's head commit, or on its merge commit when a task's pull request merged, per pull request. On a merge commit, `head_sha` is that commit and `base_branch` the branch it merged into. Subject:
`owner/name#43`.
Schema: [`github.check_run.completed.json`](../schemas/events/github.check_run.completed.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `name` | string or null | The check run's name, as branch protection names a required check |
| `conclusion` | string or null | `success`, `failure`, `neutral`, `cancelled`, `skipped`, `timed_out`, `action_required`, `stale` or `startup_failure` |
| `app` | string or null | The slug of the GitHub App that ran it |
| `pending` | integer | Check runs on the same commit not completed at the poll, counting those past the first 100 |
| `failing` | integer | Check runs on the same commit completed with a conclusion other than success, neutral or skipped |
| `github_id` | integer | GitHub's check run id |

<!-- example: events/github.check_run.completed.json -->
```json
{
  "id": "48326e7f-447a-595a-8632-62de12de5a61",
  "type": "github.check_run.completed",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:12:37Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/runs/63598796976",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "name": "test (ubuntu)",
    "conclusion": "success",
    "app": "github-actions",
    "pending": 1,
    "failing": 0,
    "time": "2026-11-17T16:12:37Z",
    "github_id": 63598796976,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/runs/63598796976" }]
}
```

### `github.pull_request.check_runs`

A snapshot of a pull request head's check runs: the first time a poll finds any, or none past the
5-minute grace period. See [pull-requests.md](../intake/pull-requests.md). Subject: `owner/name#43`.
Schema: [`github.pull_request.check_runs.json`](../schemas/events/github.pull_request.check_runs.json).

The pull request fields, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `runs` | integer | How many check runs the head had at the poll |
| `updated_at` | RFC 3339 | When GitHub last updated the pull request, before the poll |

<!-- example: events/github.pull_request.check_runs.json -->
```json
{
  "id": "5710ac5a-777d-5eae-a1bc-d3a9b3e49747",
  "type": "github.pull_request.check_runs",
  "subject": "acme/widgets#43",
  "time": "2026-11-17T16:06:01.482913Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "url": "https://github.com/acme/widgets/pull/43",
    "head_branch": "fix-empty-export",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "main",
    "runs": 3,
    "updated_at": "2026-11-17T16:05:00Z",
    "time": "2026-11-17T16:06:01.482913Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43" }]
}
```

### `github.pull_request.review_requested`

One message per open pull request that asks a reviewer for a review and did not at the previous
poll, in any repository. See [review-requests.md](../intake/review-requests.md). Subject:
`owner/name#43`.
Schema: [`github.pull_request.review_requested.json`](../schemas/events/github.pull_request.review_requested.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string | `owner/name` |
| `pull_request` | integer | The pull request number |
| `title` | string | The title when the intake polled |
| `url` | string | The pull request's page |
| `author` | string or null | The GitHub login who opened it; null for a deleted user |
| `since` | RFC 3339 | When the intake first found the request: the time of that poll |
| `reviewer` | string | The GitHub login asked for the review, as the trigger names it |

<!-- example: events/github.pull_request.review_requested.json -->
```json
{
  "id": "d417fdba-388d-5bba-8e5f-459630ca1557",
  "type": "github.pull_request.review_requested",
  "subject": "acme/widgets#43",
  "time": "2026-11-20T09:01:00.214Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "title": "Fix the empty export",
    "url": "https://github.com/acme/widgets/pull/43",
    "author": "hubot",
    "since": "2026-11-20T09:01:00.214Z",
    "reviewer": "octocat",
    "time": "2026-11-20T09:01:00.214Z",
    "baseline": false,
    "triggers": ["review-requests"]
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43" }]
}
```

### `github.pull_request.review_request_removed`

One message per pull request gone from a reviewer's requests since the previous poll. Its fields
come from that earlier poll. See [review-requests.md](../intake/review-requests.md). Subject:
`owner/name#43`.
Schema: [`github.pull_request.review_request_removed.json`](../schemas/events/github.pull_request.review_request_removed.json).

The fields of `github.pull_request.review_requested`, and:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `reviewed` | boolean | Whether the reviewer submitted a review since `since`. False means the request ended some other way: someone removed it, or the pull request closed |

<!-- example: events/github.pull_request.review_request_removed.json -->
```json
{
  "id": "d2d46f0a-ce30-5ae6-88ba-fd4e0d882c5a",
  "type": "github.pull_request.review_request_removed",
  "subject": "acme/widgets#43",
  "time": "2026-11-20T10:12:00.530Z",
  "data": {
    "repo": "acme/widgets",
    "pull_request": 43,
    "title": "Fix the empty export",
    "url": "https://github.com/acme/widgets/pull/43",
    "author": "hubot",
    "since": "2026-11-20T09:01:00.214Z",
    "reviewer": "octocat",
    "reviewed": false,
    "time": "2026-11-20T10:12:00.530Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43" }]
}
```

## Linear

Every Linear event carries the issue's fields, as the intake saw them. Subject: the issue
identifier, such as `ENG-7`.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `issue` | string | The identifier, such as `ENG-7` |
| `linear_id` | string | Linear's id of what the event is about: the issue, the history entry or the comment |
| `title` | string | The issue title |
| `url` | string | The issue's page, or the comment's for a comment |
| `team` | string | The team key |
| `project` | string or null | The project name; null without one |
| `project_id` | string or null | Linear's id of the project; null without one |
| `state` | string | The state name |
| `labels` | array of strings | The label names |

`blockers`, where present, lists the identifiers of the open issues that block it. It is null when
the issue has more than 20 relations.

### `linear.issue.entered`

One message per issue found in a view that was not in it at the previous poll.
Schema: [`linear.issue.entered.json`](../schemas/events/linear.issue.entered.json).

The issue fields, and `view` (string), the view as the trigger names it.

<!-- example: events/linear.issue.entered.json -->
```json
{
  "id": "8beb4cb2-80ac-53e2-b95c-eeb5e39b5a11",
  "type": "linear.issue.entered",
  "subject": "ENG-7",
  "time": "2026-11-17T11:00:02.512Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "9f2c4b1e-7d3a-4c55-8e21-0b6f3a9d1c47",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "Todo",
    "labels": ["agent", "bug"],
    "view": "6b3c2f9a1d4e",
    "time": "2026-11-17T11:00:02.512Z",
    "baseline": false,
    "triggers": ["agent-ready"]
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project" }]
}
```

### `linear.issue.left`

One message per issue gone from a view that was in it at the previous poll. Its issue fields come
from that earlier poll.
Schema: [`linear.issue.left.json`](../schemas/events/linear.issue.left.json).

The issue fields, and `view` (string).

<!-- example: events/linear.issue.left.json -->
```json
{
  "id": "03f11e72-fe38-5693-baf7-121b1a402c33",
  "type": "linear.issue.left",
  "subject": "ENG-7",
  "time": "2026-11-17T12:31:40.077Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "9f2c4b1e-7d3a-4c55-8e21-0b6f3a9d1c47",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "In Progress",
    "labels": ["agent", "bug"],
    "view": "6b3c2f9a1d4e",
    "time": "2026-11-17T12:31:40.077Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project" }]
}
```


### `linear.view.changed`

One message per poll that finds a view's filter changed since the previous poll. The issues the
change brought in or took out publish no `linear.issue.entered` or `linear.issue.left` and request
no task. Subject: the view. It carries no issue fields and no link.
Schema: [`linear.view.changed.json`](../schemas/events/linear.view.changed.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `view` | string | The view, as the trigger names it |
| `entered` | array of strings | The identifiers of the issues now in the view that were not in it at the previous poll |
| `left` | array of strings | The identifiers of the issues in the view at the previous poll that are not in it now |

<!-- example: events/linear.view.changed.json -->
```json
{
  "id": "5d0e8c3a-6f1b-5a7e-9c24-31b8f0e6d2a9",
  "type": "linear.view.changed",
  "subject": "6b3c2f9a1d4e",
  "time": "2026-11-17T13:05:12.340Z",
  "data": {
    "view": "6b3c2f9a1d4e",
    "entered": ["ENG-12", "ENG-15"],
    "left": ["ENG-7"],
    "time": "2026-11-17T13:05:12.340Z",
    "baseline": false,
    "triggers": []
  },
  "links": []
}
```
### `linear.issue.created`

One message per issue created.
Schema: [`linear.issue.created.json`](../schemas/events/linear.issue.created.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `actor` | string or null | Who created it, by display name |
| `blockers` | array of strings or null | Its open blockers |
| `delivery` | string | Present when a webhook delivery published it: its `Linear-Delivery`. See [webhooks.md](../intake/webhooks.md#linear) |

<!-- example: events/linear.issue.created.json -->
```json
{
  "id": "9fbb2b2f-7964-56df-887a-48a01dbdd601",
  "type": "linear.issue.created",
  "subject": "ENG-7",
  "time": "2026-11-17T10:58:21.904Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "9f2c4b1e-7d3a-4c55-8e21-0b6f3a9d1c47",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "Todo",
    "labels": ["agent", "bug"],
    "actor": "Ada Lovelace",
    "blockers": [],
    "time": "2026-11-17T10:58:21.904Z",
    "baseline": false,
    "triggers": ["triage"]
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project" }]
}
```

### `linear.issue.state_changed`

One message per history entry that moves an issue from one state to another. `linear_id` is the
history entry, or the issue from a webhook, which sees no history entry.
Schema: [`linear.issue.state_changed.json`](../schemas/events/linear.issue.state_changed.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `from` | string or null | The state it moved from; null from a webhook, whose payload names only its id |
| `to` | string | The state it moved to |
| `actor` | string or null | Who moved it |
| `blockers` | array of strings or null | Its open blockers |
| `delivery` | string | Present when a webhook delivery published it: its `Linear-Delivery`. See [webhooks.md](../intake/webhooks.md#linear) |

<!-- example: events/linear.issue.state_changed.json -->
```json
{
  "id": "6d072ee7-1381-5ff7-b58f-3f145ece9cd0",
  "type": "linear.issue.state_changed",
  "subject": "ENG-7",
  "time": "2026-11-17T12:31:12.330Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "c1d8e0a4-5b6f-4f2a-9d3e-7a8b9c0d1e2f",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "In Progress",
    "labels": ["agent", "bug"],
    "from": "Todo",
    "to": "In Progress",
    "actor": "Ada Lovelace",
    "blockers": ["ENG-3"],
    "time": "2026-11-17T12:31:12.330Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project" }]
}
```

### `linear.issue.updated`

One message per history entry that edits an issue's title, description, labels, assignee or
priority, for an issue a task owns or subscribes to. `linear_id` is the history entry. `title` is the
title the entry set when it changed the title. It never names a trigger.
Schema: [`linear.issue.updated.json`](../schemas/events/linear.issue.updated.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `changed` | array of strings | The fields the entry changed, of `title`, `description`, `labels`, `assignee` and `priority` |
| `description` | string or null | The first 200 characters of the issue's description when `changed` holds `description`; null otherwise. Linear's history records that the description changed, not its text, so this is the description when the intake read it |
| `actor` | string or null | Who edited it |

<!-- example: events/linear.issue.updated.json -->
```json
{
  "id": "0b8f4c2e-5d1a-5e7b-9c3f-2a6d8e1f4b70",
  "type": "linear.issue.updated",
  "subject": "ENG-7",
  "time": "2026-11-17T12:40:05.210Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "e4a7b2c9-3f1d-4a6e-8b5c-9d0e1f2a3b4c",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "In Progress",
    "labels": ["agent", "bug"],
    "changed": ["title", "description"],
    "description": "Exporting a project with no files crashes the app. Steps: create a project, open Export, press Save.",
    "actor": "Ada Lovelace",
    "time": "2026-11-17T12:40:05.210Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project" }]
}
```

### `linear.issue.blockers`

One message per issue that another issue blocks, each time the blocker changes. Linear does not
update an issue when its blocker closes, so this event carries the news.
Schema: [`linear.issue.blockers.json`](../schemas/events/linear.issue.blockers.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `blockers` | array of strings or null | Its open blockers |
| `blocker` | string | The blocker whose change published the event |

<!-- example: events/linear.issue.blockers.json -->
```json
{
  "id": "2bf5fb6a-416e-57ab-925c-faf8dec1b2d6",
  "type": "linear.issue.blockers",
  "subject": "ENG-7",
  "time": "2026-11-17T13:05:44.120Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "9f2c4b1e-7d3a-4c55-8e21-0b6f3a9d1c47",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "In Progress",
    "labels": ["agent", "bug"],
    "blockers": [],
    "blocker": "ENG-3",
    "time": "2026-11-17T13:05:44.120Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project" }]
}
```

### `linear.comment.created`

One message per comment on an issue. `linear_id` and `url` are the comment's.
Schema: [`linear.comment.created.json`](../schemas/events/linear.comment.created.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `body` | string | The comment, in Markdown |
| `actor` | string or null | Who commented |
| `delivery` | string | Present when a webhook delivery published it: its `Linear-Delivery`. See [webhooks.md](../intake/webhooks.md#linear) |

<!-- example: events/linear.comment.created.json -->
```json
{
  "id": "f0352bb3-8810-574d-940d-8d6c29e73f60",
  "type": "linear.comment.created",
  "subject": "ENG-7",
  "time": "2026-11-17T13:20:00.000Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "6f1c1c9e-2b7a-4d39-9a43-2d6a3e1b8f10",
    "title": "Export fails on an empty project",
    "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project#comment-6f1c1c9e",
    "team": "ENG",
    "project": "Desktop",
    "project_id": "c1d5e0a2-3b4f-4e8a-9a61-7f20d4b8e913",
    "state": "In Progress",
    "labels": ["agent", "bug"],
    "body": "Please also cover the empty case.",
    "actor": "Ada Lovelace",
    "time": "2026-11-17T13:20:00.000Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/export-fails-on-an-empty-project#comment-6f1c1c9e" }]
}
```

## Slack

### `slack.message.posted`

One message per message posted in a trigger's channel, outside a thread. Subject:
`<channel>:<ts>`.
Schema: [`slack.message.posted.json`](../schemas/events/slack.message.posted.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `channel` | string | The channel id |
| `ts` | string | Slack's id of the message: seconds since the epoch, with microseconds |
| `user` | string or null | The author's Slack user id |
| `text` | string | The message, in Slack's markup |
| `files` | array of strings | The names of the files uploaded with the message; empty when none |

<!-- example: events/slack.message.posted.json -->
```json
{
  "id": "7d892a3e-a083-5b61-836d-d1c7dfd5f1d5",
  "type": "slack.message.posted",
  "subject": "C0123:1795036000.000100",
  "time": "2026-11-18T21:06:40.000100Z",
  "data": {
    "channel": "C0123",
    "ts": "1795036000.000100",
    "user": "U0PERSON",
    "text": "@priory the export is broken again",
    "files": ["export.log"],
    "time": "2026-11-18T21:06:40.000100Z",
    "baseline": false,
    "triggers": ["support-channel"]
  },
  "links": []
}
```

### `slack.thread.replied`

One message per reply in a thread that an open `slack_reply` wait or a subscription names, a reply
also sent to the channel included. It never names a trigger.
Subject: `<channel>:<thread ts>`.
Schema: [`slack.thread.replied.json`](../schemas/events/slack.thread.replied.json).

The fields of `slack.message.posted`, and these. `baseline` is always false.

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `thread_ts` | string | The `ts` of the thread's first message |
| `actor` | string or null | The author's display name, from `users.info`; the user id without the `users:read` scope; null with no user id. See [slack.md](../intake/slack.md#events) |

<!-- example: events/slack.thread.replied.json -->
```json
{
  "id": "5f232f58-c5ec-5a26-b515-eaa03352c6ff",
  "type": "slack.thread.replied",
  "subject": "C0123:1795036000.000100",
  "time": "2026-11-18T21:06:41.000200Z",
  "data": {
    "channel": "C0123",
    "thread_ts": "1795036000.000100",
    "ts": "1795036001.000200",
    "user": "U0PERSON",
    "actor": "Ana",
    "text": "Looks good, ship it.",
    "files": [],
    "time": "2026-11-18T21:06:41.000200Z",
    "baseline": false,
    "triggers": []
  },
  "links": []
}
```

## Others

### `schedule.fired`

One message per firing of a schedule trigger. Subject: `schedule:<trigger>@<scheduled>`.
Schema: [`schedule.fired.json`](../schemas/events/schedule.fired.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `trigger` | string | The trigger's name |
| `scheduled` | RFC 3339 | When the firing was due, in UTC |

<!-- example: events/schedule.fired.json -->
```json
{
  "id": "a2899904-21cf-501b-afc1-9476ffc2fb37",
  "type": "schedule.fired",
  "subject": "schedule:support-day@2026-11-19T08:00:00Z",
  "time": "2026-11-19T08:00:00Z",
  "data": {
    "trigger": "support-day",
    "scheduled": "2026-11-19T08:00:00Z"
  },
  "links": []
}
```

### `priory.task.requested.cli`

One message per `priory request`, and one per report pick whose option requests a task (see
[reports.md](../daemon/reports.md)). Subject: the task key, `cli:<key>`.
Schema: [`priory.task.requested.cli.json`](../schemas/events/priory.task.requested.cli.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `text` | string | The request, as typed |
| `person` | string or null | `$USER`, or for the Slack shortcut the user who sent the form, as `<username> (<user id>)` |
| `time` | RFC 3339 | When the command ran |
| `repo` | string or null | `--repo` |

<!-- example: events/priory.task.requested.cli.json -->
```json
{
  "id": "b770d2d3-3057-5e09-bfca-a0bb555bafc8",
  "type": "priory.task.requested.cli",
  "subject": "cli:docs-1",
  "time": "2026-11-17T18:00:00Z",
  "data": {
    "text": "Fix the docs",
    "person": "max",
    "time": "2026-11-17T18:00:00Z",
    "repo": "acme/widgets"
  },
  "links": []
}
```

### `factory.task.requested`

One message per source event and matching trigger, and one per `priory request`. Only this message
creates a task. Subject: the task key. See [task-requests.md](../intake/task-requests.md).
Schema: [`factory.task.requested.json`](../schemas/events/factory.task.requested.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `workflow` | string | The workflow the task starts |
| `key` | string | Which events are the same work, such as `github:owner/name#42` or `linear:ENG-7` |
| `band` | string or null | The trigger's or request's band |
| `lane` | string or null | The trigger's or request's lane; null when `lanes.toml` derives it |
| `trigger` | string | The trigger's name, or `cli` |
| `source` | UUID string | The `id` of the source event's message |
| `team` | string or null, optional | The Linear issue's team. Linear sources only |
| `project` | string or null, optional | The Linear issue's project. Linear sources only |
| `repo` | string or null, optional | `priory request --repo` for a CLI request; the pull request's repository for a review request. Those sources only |
| `hold` | object, optional | The trigger's `hold` or `priory request --hold`. See [triggers.md](triggers.md#hold) |
| `tldr` | string, optional | `priory request --tldr` |
| `relates` | array, optional | `priory request` or `priory add --relates`: each `{ "kind", "task" }`, the relation's kind and the other task's id. See [relations.md](../daemon/relations.md) |
| `intervention` | `plan` or `hands-off`, optional | `priory request` or `priory add --intervention`; the task takes its trigger's when absent |
| `batch` | object, optional | `priory batch add`: `issues`, the batch's issues as task keys, and `entries`, the workflow of each entry's task. The task it creates plans that batch. See [batches.md](../daemon/batches.md) |
| `scheduled` | RFC 3339, optional | When the firing was due. Schedule sources only |
| `prompt` | string or null, optional | The trigger's `prompt`. Schedule sources only |

<!-- example: events/factory.task.requested.json -->
```json
{
  "id": "ef68a451-bc80-5a71-820f-2fb320bb3a8d",
  "type": "factory.task.requested",
  "subject": "github:acme/widgets#42",
  "time": "2026-11-17T15:18:55Z",
  "data": {
    "workflow": "implement",
    "key": "github:acme/widgets#42",
    "band": "today",
    "lane": "core/intake",
    "trigger": "agent-label",
    "source": "98c30eca-6e69-591d-b9ab-d4731abf371a"
  },
  "links": []
}
```

### `factory.intake.baseline`

A marker of where a stream stands, published after a poll that moved it. Consumers can ignore it.
See [github-polling.md](../intake/github-polling.md). Subject: `owner/name <stream>` for GitHub, the
stream otherwise.
Schema: [`factory.intake.baseline.json`](../schemas/events/factory.intake.baseline.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `repo` | string, optional | `owner/name`. GitHub streams only |
| `stream` | string | The stream, such as `github.issues.labeled`, `github.review_requests:octocat`, `linear.comments` or `slack.channel:C0123` |
| `position` | integer or string | A GitHub id or number, a count of set changes, a Linear time or a Slack `ts` |
| `members` | array, optional | The issue fields of every issue in the view, or the pull request fields of every review request. Linear view and review request streams only |
| `seen` | array of strings, optional | The ids of the events at the position's instant. Linear issue and comment streams only |

<!-- example: events/factory.intake.baseline.json -->
```json
{
  "id": "fc467208-9103-506d-9743-f29511b19ec4",
  "type": "factory.intake.baseline",
  "subject": "acme/widgets github.issues.labeled",
  "time": "2026-11-17T16:58:50.369119964Z",
  "data": {
    "repo": "acme/widgets",
    "stream": "github.issues.labeled",
    "position": 36430795541
  },
  "links": []
}
```

## Runners

A factory with two or more runner files has its daemons publish these on `factory.intake` too, and
`priory give` and `priory answer RUNNER:ASK` publish two of them. Every runner reads them all. See
[runners.md](../daemon/runners.md).

### `priory.task.claimed`

A runner claims a task's key: for a request no runner holds, to take over a waiting task of a
runner whose lease ended, or because a person gave it the task with `priory give`. The first claim
in log order that wins a key makes its runner the owner. Subject: the key.
Schema: [`priory.task.claimed.json`](../schemas/events/priory.task.claimed.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `key` | string | The task's key |
| `runner` | string | The runner that claims it |
| `request` | UUID string | The `id` of the task request the owner creates the task from |
| `from` | string, optional | The runner it takes the key from, for a takeover or a gift |
| `given` | boolean, optional | True when a person gave the task with `priory give` |
| `ended` | boolean, optional | True when the runner's task of the key ended before the factory had runners, so the claim only marks its request as handled |

<!-- example: events/priory.task.claimed.json -->
```json
{
  "id": "4f8a2c1e-6b3d-5a90-8e7f-1c2d3e4f5a6b",
  "type": "priory.task.claimed",
  "subject": "github:acme/widgets#42",
  "time": "2026-11-21T11:00:00Z",
  "data": {
    "key": "github:acme/widgets#42",
    "runner": "workstation",
    "request": "ef68a451-bc80-5a71-820f-2fb320bb3a8d",
    "from": "xps13"
  },
  "links": []
}
```

### `priory.task.held`

What another runner may do with a task, published by its owner each time it changes. Subject: the
key. Schema: [`priory.task.held.json`](../schemas/events/priory.task.held.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `key` | string | The task's key |
| `runner` | string | The runner that owns it |
| `hold` | `waiting`, `kept` or `ended` | `waiting`: no run of it started, so it moves once its runner's lease ends. `kept`: started or asleep, so it stays. `ended`: done, failed or dropped |

<!-- example: events/priory.task.held.json -->
```json
{
  "id": "0196f1c2-7a3e-7c41-9b2d-5e8f0a1b2c3d",
  "type": "priory.task.held",
  "subject": "github:acme/widgets#42",
  "time": "2026-11-21T10:00:05Z",
  "data": {
    "key": "github:acme/widgets#42",
    "runner": "xps13",
    "hold": "kept"
  },
  "links": []
}
```

### `priory.runner.alive`

A runner renews its lease, once a minute while its daemon runs. Its lease ends 30 minutes after this
message's `time`. Subject: the runner.
Schema: [`priory.runner.alive.json`](../schemas/events/priory.runner.alive.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `runner` | string | The runner, named by its hostname |

<!-- example: events/priory.runner.alive.json -->
```json
{
  "id": "9d3b7e21-4c5a-5f68-a1b2-c3d4e5f6a7b8",
  "type": "priory.runner.alive",
  "subject": "xps13",
  "time": "2026-11-21T10:00:00Z",
  "data": {
    "runner": "xps13"
  },
  "links": []
}
```

### `priory.ask.answered`

One message per `priory answer RUNNER:ASK`. Only that runner applies it, with the revision check of
a local answer. Subject: `<runner>:<ask>`.
Schema: [`priory.ask.answered.json`](../schemas/events/priory.ask.answered.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `runner` | string | The runner whose ask it answers |
| `ask` | integer | The ask's id on that runner |
| `revision` | integer | The question revision the person read |
| `text` | string | The answer |
| `run` | boolean | `--run`: the person confirmed that the answer runs the ask's command |
| `by` | string | `cli` |

<!-- example: events/priory.ask.answered.json -->
```json
{
  "id": "0196f1c3-1b2c-7d4e-8f9a-0b1c2d3e4f5a",
  "type": "priory.ask.answered",
  "subject": "xps13:7",
  "time": "2026-11-21T10:20:00Z",
  "data": {
    "runner": "xps13",
    "ask": 7,
    "revision": 1,
    "text": "approved",
    "run": false,
    "by": "cli"
  },
  "links": []
}
```

### `priory.ask.posted`

A runner posts one of its asks each time any part of it that a view shows changes: asked, revised,
answered, refused, dismissed or withdrawn. Every other runner keeps the post with the highest
`version` and lists the ask as `<runner>:<ask>`. An ask closed before the runner first posted is
never posted. Subject: `<runner>:<ask>`.
Schema: [`priory.ask.posted.json`](../schemas/events/priory.ask.posted.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `runner` | string | The runner that owns the ask |
| `ask` | integer | The ask's id on that runner |
| `version` | integer | Raised by every change the post shows. A refused answer raises it, never the question's `revision` |
| `kind` | `step`, `agent` or `outside` | Who asked: a human step, an agent, or a session Priory did not start |
| `listed` | object | The ask as the runner's own `priory asks --json` lists it. See [serve.md](serve.md#asks) |

<!-- example: events/priory.ask.posted.json -->
```json
{
  "id": "a3c81f5d-29e4-5b07-9c6a-d1e2f3a4b5c6",
  "type": "priory.ask.posted",
  "subject": "xps13:7",
  "time": "2026-11-21T10:00:02Z",
  "data": {
    "runner": "xps13",
    "ask": 7,
    "version": 1,
    "kind": "step",
    "listed": {
      "id": 7,
      "task": 3,
      "key": "github:acme/widgets#42",
      "step": "approve",
      "state": "open",
      "revision": 1,
      "question": "approve",
      "options": [
        "approved",
        "rejected"
      ],
      "since": "2026-11-21T10:00:01Z",
      "revisions": [],
      "refused": []
    }
  },
  "links": []
}
```
