# Events

The intake publishes every message on the herdr-bus output `factory.intake`. Each message is a
CloudEvent-shaped envelope whose `data` depends on its `type`. This page lists every type, with its
subject, its `data` fields and one example. The [intake pages](../intake/README.md) explain when
each one is published.

Each type has a JSON Schema in [`docs/schemas/events/`](../schemas/events/), named
`<type>.json`. The intake's tests check every message they publish against it.

- [The envelope](#the-envelope)
- GitHub: [`github.issues.labeled`](#githubissueslabeled), [`github.issues.opened`](#githubissuesopened),
  [`github.pull_request.opened`](#githubpull_requestopened), [`github.pull_request.labeled`](#githubpull_requestlabeled),
  [`github.pull_request.closed`](#githubpull_requestclosed), [`github.pull_request_review.submitted`](#githubpull_request_reviewsubmitted),
  [`github.check_run.completed`](#githubcheck_runcompleted), [`github.pull_request.check_runs`](#githubpull_requestcheck_runs),
  [`github.pull_request.review_requested`](#githubpull_requestreview_requested),
  [`github.pull_request.review_request_removed`](#githubpull_requestreview_request_removed)
- Linear: [`linear.issue.entered`](#linearissueentered), [`linear.issue.left`](#linearissueleft),
  [`linear.view.changed`](#linearviewchanged),
  [`linear.issue.created`](#linearissuecreated), [`linear.issue.state_changed`](#linearissuestate_changed),
  [`linear.issue.blockers`](#linearissueblockers), [`linear.comment.created`](#linearcommentcreated)
- Slack: [`slack.message.posted`](#slackmessageposted), [`slack.thread.replied`](#slackthreadreplied)
- Others: [`schedule.fired`](#schedulefired), [`priory.task.requested.cli`](#priorytaskrequestedcli),
  [`factory.task.requested`](#factorytaskrequested), [`factory.intake.baseline`](#factoryintakebaseline)

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
| `actor` | string or null | Who reviewed |
| `github_id` | integer | GitHub's review id |

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
    "actor": "octocat",
    "time": "2026-11-17T16:40:03Z",
    "github_id": 2561579718,
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/acme/widgets/pull/43#pullrequestreview-2561579718" }]
}
```

### `github.check_run.completed`

One message per check run completed on a pull request's head commit, per pull request. Subject:
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
history entry.
Schema: [`linear.issue.state_changed.json`](../schemas/events/linear.issue.state_changed.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `from` | string | The state it moved from |
| `to` | string | The state it moved to |
| `actor` | string or null | Who moved it |
| `blockers` | array of strings or null | Its open blockers |

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
    "time": "2026-11-18T21:06:40.000100Z",
    "baseline": false,
    "triggers": ["support-channel"]
  },
  "links": []
}
```

### `slack.thread.replied`

One message per reply in a thread that an open `slack_reply` wait names. It never names a trigger.
Subject: `<channel>:<thread ts>`.
Schema: [`slack.thread.replied.json`](../schemas/events/slack.thread.replied.json).

The fields of `slack.message.posted`, and `thread_ts` (string), the `ts` of the thread's first
message. `baseline` is always false.

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
    "text": "Looks good, ship it.",
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

One message per `priory request`. Subject: the task key, `cli:<key>`.
Schema: [`priory.task.requested.cli.json`](../schemas/events/priory.task.requested.cli.json).

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `text` | string | The request, as typed |
| `person` | string or null | `$USER` |
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
