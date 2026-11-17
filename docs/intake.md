# Intake messages

`priory intake [DIR] [--once]` polls GitHub for the repositories that the factory's triggers name. It
publishes on the herdr-bus output `factory.intake`. This page is the contract for every consumer of
that output: the Priory daemon, and another factory such as herdr-bowtie.

The intake checks the factory first, as `priory check` does, and refuses to start while it has
problems. `--once` polls once and exits. Without it, the intake polls every 60 seconds. It reads
GitHub only, through `gh api` and `gh api graphql`, as the user `gh` is logged in as.

## The envelope

Every message is a herdr-bus `Message`, shaped like a CloudEvent:

| Field     | Type          | Meaning                                                    |
| --------- | ------------- | ---------------------------------------------------------- |
| `id`      | UUID string   | A UUIDv5, stable across retries (see Message ids)          |
| `type`    | string        | A GitHub type below, `priory.task.requested.cli`, `factory.task.requested` or `factory.intake.baseline` |
| `subject` | string        | `owner/name#42` for a GitHub event, the task key for a CLI request or a task request, `owner/name <stream>` for a baseline marker |
| `time`    | RFC 3339      | When the GitHub event happened; for a baseline marker, when it was published |
| `data`    | object        | The fields below, by type                                  |
| `links`   | array         | GitHub events carry one `{ "label": "issue" or "pull_request", "url": ... }`, with the event's `url` |

herdr-bus drops a message whose `(output, id)` is already on the output. A consumer can still see a
message twice, after its own crash before its ack, so it deduplicates by `id`.

## `github.issues.labeled` and `github.issues.opened`

One message per GitHub event, published once. Other issue events, such as `closed` or `mentioned`,
are not published.

| Field       | Type           | Meaning                                                          |
| ----------- | -------------- | ---------------------------------------------------------------- |
| `repo`      | string         | `owner/name`                                                     |
| `issue`     | integer        | The issue number                                                 |
| `title`     | string         | The issue title when the intake saw the event                    |
| `url`       | string         | The issue's page on GitHub                                       |
| `label`     | string         | The label added. `labeled` only; absent on `opened`              |
| `actor`     | string or null | GitHub login: who added the label, or who opened the issue. Null for a deleted user |
| `time`      | RFC 3339       | When the label was added, or the issue was created               |
| `github_id` | integer        | GitHub's issue event id for `labeled`, the issue id for `opened` |
| `baseline`  | boolean        | True when the event came from the first poll of its repository and type (see Baseline) |
| `triggers`  | array of strings | The triggers this event matched when the intake published it, so the triggers it requests tasks for; may be empty |

```json
{
  "id": "2674a560-8df6-5f05-b644-5ac1b1df8158",
  "type": "github.issues.labeled",
  "subject": "cli/cli#15066",
  "time": "2026-11-17T15:18:55Z",
  "data": {
    "repo": "cli/cli",
    "issue": 15066,
    "title": "`gh label list --json` prints nothing instead of `[]` when the repository has no labels",
    "url": "https://github.com/cli/cli/issues/15066",
    "label": "needs-investigation",
    "actor": "babakks",
    "time": "2026-11-17T15:18:55Z",
    "github_id": 36421224333,
    "baseline": false,
    "triggers": ["agent-label"]
  },
  "links": [{ "label": "issue", "url": "https://github.com/cli/cli/issues/15066" }]
}
```

GitHub's issue events list has no `opened` event, so the intake reads `opened` from the issues list,
newest first. Its `title` is the title at the time of the poll, which may differ from the title the
issue was opened with.

## Pull request events

The intake publishes five pull request types. Each names the pull request and its branches, so a
consumer can route it to the task that owns the pull request or its branch.

| Type                                   | One message per                                   |
| -------------------------------------- | ------------------------------------------------- |
| `github.pull_request.opened`           | pull request opened                               |
| `github.pull_request.labeled`          | label added to a pull request                     |
| `github.pull_request.closed`           | pull request closed, merged or not                |
| `github.pull_request_review.submitted` | review submitted; pending reviews are skipped     |
| `github.check_run.completed`           | completed check run on a pull request's head commit, per pull request |

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

## `priory.task.requested.cli`

`priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--] <text>`
publishes this source event, then one `factory.task.requested` for it. It loads the factory as
`priory check` does and refuses an unknown workflow, publishing nothing. `DIR` defaults to `.`. A flag value cannot start with `--`; put `--` before text that does.

The type has no `github.` prefix because no GitHub event produced it, and ends in `.cli` because a
later source, such as Linear, adds its own `priory.task.requested.<source>`. It is a request, not a
fact about a repository, so it names the person and the text, not a GitHub id.

| Field    | Type           | Meaning                                                    |
| -------- | -------------- | ---------------------------------------------------------- |
| `text`   | string         | The request, as typed                                      |
| `person` | string or null | `$USER`; null when it is unset                             |
| `time`   | RFC 3339       | When the command ran                                       |
| `repo`   | string or null | `--repo`, or null                                          |

The message's `subject` is the task key, `cli:<key>`, and its `time` equals `data.time`. The task
request that follows has `trigger` `cli`, `source` set to this message's `id`, `band` and `lane`
from the flags (null when absent), and the same key. No trigger file is involved.

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
    "repo": "owner/repo"
  },
  "links": []
}
```

```json
{
  "id": "1d4a502b-b1b3-54da-8165-013fe15a8d3a",
  "type": "factory.task.requested",
  "subject": "cli:docs-1",
  "time": "2026-11-17T18:00:00Z",
  "data": {
    "workflow": "implement",
    "key": "cli:docs-1",
    "band": "today",
    "lane": null,
    "trigger": "cli",
    "source": "b770d2d3-3057-5e09-bfca-a0bb555bafc8"
  },
  "links": []
}
```

Running the same command again, with the same key and text, adds nothing: the ids come from those
two values (see Message ids), so herdr-bus drops both messages. Another text under the same key is
a new request for the same task.

## `factory.task.requested`

One message per source event and trigger that matches it, the triggers its source lists in
`triggers`. `priory request` publishes one with `trigger` `cli` for its own source event. A trigger matches when its `repo` is the event's repository, its `on` names the event's
type (`pull_request.opened` names `github.pull_request.opened`), and its `filter`, if any, names the
event's `label`. A trigger can start from `issues.labeled`, `issues.opened`, `pull_request.labeled`
or `pull_request.opened`. A baseline event matches only a trigger
with `backfill = true`. Only this message creates a task.

| Field      | Type           | Meaning                                                      |
| ---------- | -------------- | ------------------------------------------------------------ |
| `workflow` | string         | The workflow the task starts                                 |
| `key`      | string         | Which events are the same work: `github:owner/name#42`, the issue or pull request number. GitHub numbers both from one sequence per repository |
| `band`     | string or null | The trigger's `band`, or null when it sets none              |
| `lane`     | string or null | The trigger's `lane`, or null when `lanes.toml` derives it   |
| `trigger`  | string         | The trigger's name: its file name without the extension      |
| `source`   | UUID string    | The `id` of the source event's message, on the same output   |

The intake publishes a task request after its source event, so a consumer has already seen the
source when it reads the request.

```json
{
  "id": "8826a421-4031-5495-a6ed-dd2af83c6842",
  "type": "factory.task.requested",
  "subject": "github:cli/cli#15066",
  "time": "2026-11-17T15:18:55Z",
  "data": {
    "workflow": "implement",
    "key": "github:cli/cli#15066",
    "band": "today",
    "lane": null,
    "trigger": "agent-label",
    "source": "2674a560-8df6-5f05-b644-5ac1b1df8158"
  },
  "links": []
}
```

## Message ids

Ids are UUIDv5 in the namespace `UUIDv5(NAMESPACE_URL, "https://github.com/maxence-lefebvre/priory/intake")`,
of these names:

| Message                  | Name                                         |
| ------------------------ | -------------------------------------------- |
| `github.issues.labeled`  | `github:{repo}:issues/events:{github_id}`    |
| `github.issues.opened`   | `github:{repo}:issues:{github_id}:opened`    |
| `github.pull_request.labeled`, `github.pull_request.closed` | `github:{repo}:issues/events:{github_id}` |
| `github.pull_request.opened` | `github:{repo}:issues:{github_id}:opened` |
| `github.pull_request_review.submitted` | `github:{repo}:pulls/reviews:{github_id}` |
| `github.check_run.completed` | `github:{repo}:check-runs:{github_id}:pulls/{pull_request}`, since one check run can sit on the head of two pull requests |
| `priory.task.requested.cli` | `priory.task.requested.cli:{len}:{key}:{text}`, with the `cli:` key and `len` its byte length, so a `:` in the key or text cannot make two requests collide |
| `factory.task.requested` | `factory.task.requested:{trigger}:{source}`  |
| `factory.intake.baseline` | `factory.intake.baseline:{repo}:{stream}`   |

## Baseline

A repository has four streams, each named after a type:

| Stream                                 | Reads                                        | Holds                                   | Position |
| -------------------------------------- | -------------------------------------------- | --------------------------------------- | -------- |
| `github.issues.labeled`                | `issues/events`, newest first                | issue and pull request `labeled`, pull request `closed` | issue event id |
| `github.issues.opened`                 | `issues`, newest first by creation           | issue and pull request `opened`          | number   |
| `github.pull_request_review.submitted` | GraphQL: every open pull request, and every pull request a new event above names | reviews | review id |
| `github.check_run.completed`           | the same query                               | check runs on each pull request's head commit | check run id |

The first poll of a stream is a baseline. It reads GitHub's first page: up to 100 recent issue
events, up to 100 recent issues, or the reviews and checks of every open pull request. It publishes each of those events with
`baseline: true`, and requests no task for them, except for a trigger with `backfill = true`, which
gets a task for each event it matches. It then publishes one `factory.intake.baseline` message for
the stream, even when the page held no event. A consumer can ignore that message.

| Field      | Type    | Meaning                                                                |
| ---------- | ------- | ---------------------------------------------------------------------- |
| `repo`     | string  | `owner/name`                                                           |
| `stream`   | string  | One of the four streams above                                          |
| `position` | integer | The highest position on that page; 0 when empty                        |

```json
{
  "id": "40b37648-1c78-5b20-afb0-fd16d84af39e",
  "type": "factory.intake.baseline",
  "subject": "cli/cli github.issues.labeled",
  "time": "2026-11-17T16:58:50.369119964Z",
  "data": { "repo": "cli/cli", "stream": "github.issues.labeled", "position": 36430795541 },
  "links": []
}
```

Until that message is on the output, the stream is still in its baseline: a crash during the
baseline makes the next start read the first page again as a baseline.

## GitHub calls

One poll of a repository with N open pull requests and nothing new makes 3 `gh` calls: 2 REST
requests (`issues/events` and `issues`, one page each) and 1 GraphQL query, which pages itself
when N is over 100 (ceil(N/100) requests). The query costs 3 points per page on `cli/cli`. A poll
that finds new pull request events adds 1 GraphQL query for the pull requests they name. At one
poll a minute, that is 120 REST requests and about 180 GraphQL points an hour per repository, each
against GitHub's 5,000 an hour. A stream with more than 100 new events since the last poll reads one
more REST page per 100.

## Where polling stops

The intake keeps no state file. On start, it reads `factory.intake` back, newest first, to each
stream's `factory.intake.baseline` message. Where polling stopped is the higher of that message's
`position` and the position of the stream's newest source event. When that newest event is not a
baseline, the intake publishes again its task requests, for the triggers in its `triggers` that
the factory still defines. A crash between a source event and its task requests then loses
nothing, and herdr-bus drops the repeats. A trigger added since gets no task from this, because it
is not in the list. Each poll then publishes, oldest first, the events after that point. The review and check streams
have no such point: each poll reads the current reviews and checks again and publishes those not on
the output yet. The intake remembers the ids it published, and herdr-bus drops the rest, so after a
restart the first poll offers every current review and check again. A poll publishes reviews and
checks before the issue streams, so a crash after a pull request's `closed` event cannot skip the
reviews and checks that only that event made the poll read. Without `--once`, a failing `gh` is
logged and retried on the next poll, while a failing publish ends the intake, so that its restart
recovers what it left out.

Limits:

- The point is kept per repository and type, not per trigger. A trigger added later for a repository
  the intake already polls starts from where that repository stands, so `backfill` has no effect
  on it.
- A crash between a baseline event and the task request of a `backfill` trigger loses that task
  request: start-up recovery never dispatches a baseline event.
- A trigger removed after a crash and before the restart gets no recovered task request.
- An event that happens while a crash interrupts a baseline, before the restart, lands on the
  replayed first page and is published as a baseline: it gets a task only from a `backfill` trigger.
- Each start reads the output back to the baseline markers, which grows with the output.
- `opened` pages follow creation time and stop at issue numbers. An issue transferred or imported
  into the repository with an older creation time can be missed.
- Head and base fields come from the pull request when the intake polls, not when the event
  happened.
- Reviews and checks are read for open pull requests, and for a pull request a new issue event
  names, such as its `closed` event. A check that completes after its pull request closed and a poll
  passed is missed.
- The query reads the last 50 reviews and the first 100 checks of the head commit per pull request.
