# Pull request events

The intake publishes six pull request types. Each names the pull request and its branches, so a
consumer can route it to the task that owns the pull request or its branch.

| Type                                   | One message per                                   |
| -------------------------------------- | ------------------------------------------------- |
| `github.pull_request.opened`           | pull request opened                               |
| `github.pull_request.labeled`          | label added to a pull request                     |
| `github.pull_request.closed`           | pull request closed, merged or not                |
| `github.pull_request_review.submitted` | review submitted; pending reviews are skipped     |
| `github.check_run.completed`           | completed check run on a pull request's head commit, per pull request |
| `github.pull_request.check_runs`       | pull request head whose check runs first appeared, or that has none past the grace period (see Check run snapshots) |

| Field          | Type             | Meaning                                                        |
| -------------- | ---------------- | -------------------------------------------------------------- |
| `repo`         | string           | `owner/name`                                                   |
| `pull_request` | integer          | The pull request number                                        |
| `title`        | string           | The pull request title when the intake saw the event. `opened`, `labeled` and `closed` only |
| `url`          | string           | The pull request's page; the review's for a review, the check run's for a check |
| `head_branch`  | string or null   | The pull request's head branch when the intake saw the event   |
| `head_sha`     | string or null   | The pull request's head commit when the intake saw the event; for a check, the commit it ran on |
| `base_branch`  | string or null   | The branch the pull request merges into                        |
| `label`        | string           | The label added. `labeled` only                                |
| `merged`       | boolean or null  | Whether this close merged it: the pull request is merged and this is its last `closed` event the poll read. `closed` only |
| `state`        | string           | `approved`, `changes_requested`, `commented` or `dismissed`, at poll time. Review only |
| `name`         | string           | The check run's name, as branch protection names a required check. Check only |
| `conclusion`   | string           | `success`, `failure`, `neutral`, `cancelled`, `skipped`, `timed_out`, `action_required`, `stale` or `startup_failure`. Check only |
| `app`          | string or null   | The slug of the GitHub App that ran the check. Check only      |
| `pending`      | integer          | How many check runs on the same commit had not completed when the intake polled, counting those past the first 100 as not completed. Check only |
| `actor`        | string or null   | GitHub login: who opened, labeled or closed it, or who reviewed. Absent on a check |
| `time`         | RFC 3339         | When it was opened, labeled, closed, submitted or completed    |
| `github_id`    | integer          | GitHub's id: the issue id for `opened`, the issue event id for `labeled` and `closed`, the review id, the check run id |
| `baseline`     | boolean          | As for issue events                                            |
| `triggers`     | array of strings | As for issue events. Always empty for `closed`, reviews and checks: no trigger starts a task from them |

Head and base fields are null when GitHub no longer returns the pull request.

```json
{
  "id": "ba9864b6-2145-501f-8cc7-3929df5a8fbf",
  "type": "github.pull_request.opened",
  "subject": "cli/cli#14972",
  "time": "2026-11-06T19:32:28Z",
  "data": {
    "repo": "cli/cli",
    "pull_request": 14972,
    "title": "Remove accidental triage model pin",
    "url": "https://github.com/cli/cli/pull/14972",
    "head_branch": "bagtoad/triage-ghmock-skill",
    "head_sha": "3e6aeafb1b796b862389c1abe4deba5e60fa0b01",
    "base_branch": "trunk",
    "actor": "BagToad",
    "time": "2026-11-06T19:32:28Z",
    "github_id": 6249313623,
    "baseline": true,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/cli/cli/pull/14972" }]
}
```

```json
{
  "id": "536415c5-20f5-5b2d-a0ea-671ed78728bf",
  "type": "github.pull_request.labeled",
  "subject": "owner/name#43",
  "time": "2026-11-17T10:00:00Z",
  "data": {
    "repo": "owner/name",
    "pull_request": 43,
    "title": "Add the thing",
    "url": "https://github.com/owner/name/pull/43",
    "head_branch": "feature",
    "head_sha": "abc123",
    "base_branch": "main",
    "label": "agent",
    "actor": "max",
    "time": "2026-11-17T10:00:00Z",
    "github_id": 101,
    "baseline": false,
    "triggers": ["pr-label"]
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/owner/name/pull/43" }]
}
```

```json
{
  "id": "5ff24b78-5284-58d7-b5d3-64feff5f8f46",
  "type": "github.pull_request.closed",
  "subject": "maxence-lefebvre/priory#1",
  "time": "2026-11-17T16:36:14Z",
  "data": {
    "repo": "maxence-lefebvre/priory",
    "pull_request": 1,
    "title": "Run the repository checks in CI",
    "url": "https://github.com/maxence-lefebvre/priory/pull/1",
    "head_branch": "ci",
    "head_sha": "b1e04cd5f3194049908b7672f5285ab5c9e23f1a",
    "base_branch": "main",
    "merged": true,
    "actor": "maxence-lefebvre",
    "time": "2026-11-17T16:36:14Z",
    "github_id": 36429719781,
    "baseline": true,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/maxence-lefebvre/priory/pull/1" }]
}
```

```json
{
  "id": "88803d8e-0f8c-5ca8-8ae8-0596a26c2a77",
  "type": "github.pull_request_review.submitted",
  "subject": "cli/cli#10275",
  "time": "2025-01-20T07:55:23Z",
  "data": {
    "repo": "cli/cli",
    "pull_request": 10275,
    "url": "https://github.com/cli/cli/pull/10275#pullrequestreview-2561579718",
    "head_branch": "issue10262",
    "head_sha": "dc16499be87ac81f8443f74d641150c4f14cf49c",
    "base_branch": "trunk",
    "state": "commented",
    "actor": "heaths",
    "time": "2025-01-20T07:55:23Z",
    "github_id": 2561579718,
    "baseline": true,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/cli/cli/pull/10275#pullrequestreview-2561579718" }]
}
```

```json
{
  "id": "53160d53-b108-539d-958f-6fbe8794707d",
  "type": "github.check_run.completed",
  "subject": "cli/cli#12682",
  "time": "2026-02-14T01:51:37Z",
  "data": {
    "repo": "cli/cli",
    "pull_request": 12682,
    "url": "https://github.com/cli/cli/runs/63598796976",
    "head_branch": "terminal-hyperlinks",
    "head_sha": "4b2ee8eb670ba9f77d54d59ab97ced618f024fb9",
    "base_branch": "trunk",
    "name": "pr-auto",
    "conclusion": "success",
    "app": "github-actions",
    "pending": 0,
    "time": "2026-02-14T01:51:37Z",
    "github_id": 63598796976,
    "baseline": true,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/cli/cli/runs/63598796976" }]
}
```

The intake reads check runs, not check suites: branch protection and R6.6 name required checks by
check run, such as `test (ubuntu)`, and a suite folds every run of one app into one conclusion.
Commit statuses are not published. A re-run gets a new check run id, so it publishes again.

## Check run snapshots

A repository that runs no checks never produces a `github.check_run.completed` event, and a check
run that is queued or running produces none yet. So the intake publishes
`github.pull_request.check_runs`, a snapshot of a pull request head's check runs, at most twice per
pull request and head:

- `runs` above 0, the first time a poll finds any check run on the head, completed or not;
- `runs` 0, the first time a poll finds no check run on the head and the pull request not updated
  for 5 minutes, the grace period (`GRACE_SECONDS` in `crates/priory-intake/src/intake/pulls.rs`).

GitHub creates check runs a few seconds after a push, and a push updates the pull request, so the
grace period counts from the last push or later. Commit statuses do not count as check runs.
Contexts past the first 100 count as check runs. When an event names a pull request that is also
open, the poll takes its snapshot from the open list, the later of its two reads.

| Field          | Type             | Meaning                                                        |
| -------------- | ---------------- | -------------------------------------------------------------- |
| `repo`         | string           | `owner/name`                                                   |
| `pull_request` | integer          | The pull request number                                        |
| `url`          | string           | The pull request's page                                        |
| `head_branch`  | string           | The pull request's head branch at the poll                     |
| `head_sha`     | string           | The head commit                                                |
| `base_branch`  | string           | The branch the pull request merges into                        |
| `runs`         | integer          | How many check runs the head had at the poll                   |
| `updated_at`   | RFC 3339         | When GitHub last updated the pull request, before the poll     |
| `time`         | RFC 3339         | When the poll read the head                                    |
| `baseline`     | boolean          | As for check events                                            |
| `triggers`     | array of strings | Always empty                                                   |

```json
{
  "id": "128fb513-3328-5df1-8323-e15a99050dad",
  "type": "github.pull_request.check_runs",
  "subject": "owner/name#43",
  "time": "2026-11-18T09:12:00.123456789Z",
  "data": {
    "repo": "owner/name",
    "pull_request": 43,
    "url": "https://github.com/owner/name/pull/43",
    "head_branch": "feature",
    "head_sha": "abc123",
    "base_branch": "main",
    "runs": 0,
    "updated_at": "2026-11-18T09:05:41Z",
    "time": "2026-11-18T09:12:00.123456789Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "pull_request", "url": "https://github.com/owner/name/pull/43" }]
}
```
