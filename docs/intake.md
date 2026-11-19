# Intake messages

`priory intake [DIR] [--once]` polls GitHub for the repositories that the factory's triggers name,
Linear when the factory has a Linear trigger or an open wait names a Linear issue, and Slack when
the factory has a Slack trigger or an open wait names a Slack thread (see Slack events). It publishes on the herdr-bus output `factory.intake`. This page is the contract for every consumer of
that output: the Priory daemon, and another factory such as herdr-bowtie.

The intake checks the factory first, as `priory check` does, and refuses to start while it has
problems. `--once` polls once and exits. Without it, the intake polls every 60 seconds. It reads
GitHub through `gh api` and `gh api graphql`, as the user `gh` is logged in as, and Linear through
`curl` (see Linear calls). Each poll also fires the factory's schedule triggers that are due (see
`schedule.fired`).

## The envelope

Every message is a herdr-bus `Message`, shaped like a CloudEvent:

| Field     | Type          | Meaning                                                    |
| --------- | ------------- | ---------------------------------------------------------- |
| `id`      | UUID string   | A UUIDv5, stable across retries (see Message ids)          |
| `type`    | string        | A GitHub, Linear or Slack type below, `priory.task.requested.cli`, `schedule.fired`, `factory.task.requested` or `factory.intake.baseline` |
| `subject` | string        | `owner/name#42` for a GitHub event, the issue identifier such as `ENG-7` for a Linear event, `<channel>:<ts>` for a Slack event (the thread's `ts` for a reply), the task key for a CLI request or a task request, `schedule:<trigger>@<time>` for a firing, `owner/name <stream>` or the Linear stream for a baseline marker |
| `time`    | RFC 3339      | When the GitHub or Linear event happened; for a firing, its scheduled time; for a view event or a baseline marker, when it was published |
| `data`    | object        | The fields below, by type                                  |
| `links`   | array         | GitHub and Linear events carry one `{ "label": "issue" or "pull_request", "url": ... }`, with the event's `url` |

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

### Check run snapshots

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

## `schedule.fired`

A trigger with `source = "schedule"` fires at `at`, a local time of day, in `timezone` (an IANA
name; the machine's zone when unset), on the weekdays `days` lists (every day when unset). With
`every`, it fires again every that long after `at`, the same day, up to `until` included: a time of
day such as `"18:00"` or a duration after `at` such as `"9h"`. `until` defaults to the end of the
day, and a duration stops there too. `every` is at least `1m`, the intake's poll.

Each poll publishes, for each schedule trigger, its latest firing at or before now: this source
event, then one `factory.task.requested`. The firing's time names both messages' ids, so a second
poll in the same slot publishes nothing new, and a restart after missed slots publishes only the
latest of them, once.

| Field       | Type     | Meaning                              |
| ----------- | -------- | ------------------------------------ |
| `trigger`   | string   | The trigger's name                   |
| `scheduled` | RFC 3339 | When the firing was due, in UTC       |

Its task request adds two fields to those below: `scheduled`, the same time, and `prompt`, the
trigger's `prompt` or null. Its `key` is `schedule:<key>` when the trigger sets a fixed `key`, so
every firing names the same work, and `schedule:<trigger>@<scheduled>` otherwise, so each firing is
its own task. The daemon routes a firing to a task that already owns a fixed key by the task's
state (see `docs/daemon.md`).

```json
{
  "id": "5b0e9c8c-6c0a-5b8e-9a4d-2f7f3c1e8a10",
  "type": "factory.task.requested",
  "subject": "schedule:support-day",
  "time": "2026-11-19T09:00:00Z",
  "data": {
    "workflow": "support",
    "key": "schedule:support-day",
    "band": null,
    "lane": null,
    "trigger": "support-day",
    "source": "0f4a6d1e-3c1b-5d55-8a8e-6d2b0c9e7f21",
    "scheduled": "2026-11-19T09:00:00Z",
    "prompt": "Sweep the support queue."
  },
  "links": []
}
```

Limits:

- A new schedule trigger fires its latest past slot on its first poll, as a missed firing would.
- Only the latest slot fires, so a poll delayed past two slots, such as by a slow `gh`, skips the
  older one. The last slot of a day fires even when the poll that finds it comes after `until`.
- Changing `at`, `every` or `until` names new slots, so the latest of them can fire at once.
- `every` counts real time from `at`, so a daylight saving change on that day shifts the later
  firings by the hour it adds or removes.

## Linear events

A trigger with `source = "linear.view"` names a saved view by its id, or by the slug id that ends its
URL, and starts on `issue.entered` or `issue.left`. A trigger with `source = "linear"` starts on
`issue.created`, `issue.state_changed` or `comment.created`, and may filter on `team` (a team key),
`label` (one label of the issue) and `state` (the state an issue moved to, or the issue's state for
the other two). Both take `backfill`. Their `key`, when set, is `issue.identifier`.

| Type                         | One message per                                                  |
| ---------------------------- | ---------------------------------------------------------------- |
| `linear.issue.entered`       | issue found in a view that was not in it at the previous poll     |
| `linear.issue.left`          | issue gone from a view that was in it at the previous poll        |
| `linear.issue.created`       | issue created                                                    |
| `linear.issue.state_changed` | entry of an issue's history that moves it from one state to another |
| `linear.comment.created`     | comment on an issue                                              |
| `linear.issue.blockers`      | issue that another issue blocks, each time the blocker changes   |

The intake polls issues and comments whenever the factory has a Linear trigger, whatever the
triggers start on, so that a task receives the comments and state changes of its issue (R3). The
daemon routes a Linear event to the active task whose key is `linear:<issue>`.

Linear does not update an issue when one of its blockers closes, so the issues stream publishes
`linear.issue.blockers` for it instead. For each updated issue, it reads the issues that issue blocks
in a second query, by id, and publishes one event per blocked issue, with the blocked issue's fields
and open blockers. `linear.issue.created` and `linear.issue.state_changed` carry `blockers` too. View
and comment events do not: a view event that leaves can carry the issue's fields from an earlier poll.
`priory hold --blockers` reads them, as `docs/daemon.md` describes under Holds.

| Field       | Type             | Meaning                                                         |
| ----------- | ---------------- | --------------------------------------------------------------- |
| `issue`     | string           | The issue identifier, such as `ENG-7`                           |
| `linear_id` | string           | Linear's id of what the event is about: the issue for a view event or `created`, the history entry for `state_changed`, the comment for `comment.created` |
| `title`     | string           | The issue title when the intake saw the event                   |
| `url`       | string           | The issue's page, or the comment's for `comment.created`        |
| `team`      | string           | The issue's team key                                            |
| `project`   | string or null   | The issue's project name; null when the issue has no project    |
| `state`     | string           | The issue's state when the intake saw the event                 |
| `labels`    | array of strings | The issue's labels when the intake saw the event                |
| `blockers`  | array of strings or null | The identifiers of the issues that block it and are neither completed nor canceled. Null when the issue has more than 20 relations, since one past the first page could block it. `created`, `state_changed` and `blockers` only |
| `blocker`   | string           | The issue whose change published the event. `blockers` only     |
| `view`      | string           | The view, as the trigger names it. View events only             |
| `from`, `to`| string           | The states the issue moved from and to. `state_changed` only     |
| `body`      | string           | The comment, in Markdown. `comment.created` only                |
| `actor`     | string or null   | Linear display name: who created the issue, moved it or commented. Absent on view events |
| `time`      | RFC 3339         | When the issue was created, moved or commented; for a view event, when the intake polled |
| `baseline`  | boolean          | True when the event came from the first poll of its stream (see Baseline) |
| `triggers`  | array of strings | As for GitHub events                                            |

```json
{
  "id": "f0352bb3-8810-574d-940d-8d6c29e73f60",
  "type": "linear.comment.created",
  "subject": "ENG-7",
  "time": "2026-11-17T11:00:00Z",
  "data": {
    "issue": "ENG-7",
    "linear_id": "6f1c1c9e-2b7a-4d39-9a43-2d6a3e1b8f10",
    "title": "Fix the thing",
    "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-6f1c1c9e",
    "team": "ENG",
    "project": "Desktop",
    "state": "In Progress",
    "labels": ["agent"],
    "body": "Please also cover the empty case.",
    "actor": "max",
    "time": "2026-11-17T11:00:00.000Z",
    "baseline": false,
    "triggers": []
  },
  "links": [{ "label": "issue", "url": "https://linear.app/acme/issue/ENG-7/fix-the-thing#comment-6f1c1c9e" }]
}
```

## Slack events

A trigger with `source = "slack"` names a `channel` by its id, such as `C0123`, and starts on
`message.posted`. It may filter on `text`: the trigger matches a message whose text contains it,
ignoring case. It takes `backfill`, and its `key`, when set, is `message.ts`. Each task it requests
has the key `slack:<channel>:<ts>`.

| Type                   | One message per                                                     |
| ---------------------- | ------------------------------------------------------------------- |
| `slack.message.posted` | message posted in a trigger's channel, outside a thread             |
| `slack.thread.replied` | reply in a thread that an open `slack_reply` wait names, posted at or after its agent's last prompt |

The intake publishes neither for a message by its own bot, nor for a message with a `subtype`, such
as a join. A reply never names a trigger. The daemon routes both to the active task that owns
`slack:<channel>:<thread ts>`: a task a Slack trigger started, or one whose wait names the thread.

| Field       | Type             | Meaning                                                           |
| ----------- | ---------------- | ----------------------------------------------------------------- |
| `channel`   | string           | The channel id                                                    |
| `ts`        | string           | Slack's id of the message: seconds since the epoch, with microseconds |
| `thread_ts` | string           | The `ts` of the thread's first message. `slack.thread.replied` only |
| `user`      | string or null   | The Slack user id of the author                                   |
| `text`      | string           | The message, in Slack's markup                                    |
| `time`      | RFC 3339         | The instant `ts` names                                            |
| `baseline`  | boolean          | True when the message came from the first poll of its channel. Always false for a reply |
| `triggers`  | array of strings | As for GitHub events; empty for a reply                           |

```json
{
  "id": "0b8a4c1e-56f3-5a0e-9d7b-0d6d1c3f6a21",
  "type": "slack.thread.replied",
  "subject": "C0123:1763500000.000100",
  "time": "2026-11-18T21:06:41.000200Z",
  "data": {
    "channel": "C0123",
    "thread_ts": "1763500000.000100",
    "ts": "1763500001.000200",
    "user": "U0PERSON",
    "text": "Looks good, ship it.",
    "time": "2026-11-18T21:06:41.000200Z",
    "baseline": false,
    "triggers": []
  },
  "links": []
}
```

A channel is a stream named `slack.channel:<channel>`, whose position is the `ts` of the newest
message seen. Its first poll reads the 50 newest messages as a baseline; later polls read
`conversations.history` after the position, every page of 200. A poll that moves the position
publishes a `factory.intake.baseline` marker with it, and a restart starts from the newest marker. A
crash before the marker reads the same messages again, and herdr-bus drops the repeats.

### Which threads and issues the intake watches

The intake reads the open waits on `linear_comment`, `linear_state` and `slack_reply` from the state
file `PRIORY_DB` on every poll; without `PRIORY_DB` it watches none. Each wait gives a resource and
the time of its agent's last prompt.

- For each Slack thread, the intake reads `conversations.replies` from that time on, every poll,
  until the wait fires or is withdrawn. herdr-bus drops the replies it already holds.
- For a Linear issue, the intake starts the issues and comments streams when no Linear trigger has.
  Those streams read every issue updated and every comment created in the workspace since their
  position, in two queries, so the intake never polls one issue at a time. Their first poll is a
  baseline, whose events still reach the wait: the daemon judges an event by its time, not by
  `baseline`.

A missing key fails only the poll of that source, so a wait cannot stop the GitHub polling.

## `priory.task.requested.cli`

`priory request [DIR] --workflow <name> --key <key> [--band <band>] [--lane <lane>] [--repo <owner/name>] [--hold <reason> [--until <time> | --after <task> | --merge <pr> [--if-closed release|drop|ask] | --blockers]] [--] <text>`
publishes this source event, then one `factory.task.requested` for it. `--hold` files the task held,
with the condition that follows it, as `hold` below says. It loads the factory as
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
| `key`      | string         | Which events are the same work: `github:owner/name#42`, the issue or pull request number. GitHub numbers both from one sequence per repository. `linear:ENG-7` for a Linear issue. A schedule's key is under `schedule.fired` |
| `band`     | string or null | The trigger's `band`, or null when it sets none              |
| `lane`     | string or null | The trigger's `lane`, or null when `lanes.toml` derives it   |
| `team`     | string, absent or null | The Linear issue's team key. Absent when the source is not a Linear event |
| `project`  | string, absent or null | The Linear issue's project name, or null when it has none. Absent when the source is not a Linear event |
| `trigger`  | string         | The trigger's name: its file name without the extension      |
| `source`   | UUID string    | The `id` of the source event's message, on the same output   |
| `hold`     | object         | Optional: files the task held. The trigger's `hold`, or `priory request --hold` |

`hold` holds the new task before it can start, as `priory hold` does (see Holds in
`docs/daemon.md`). It has a `reason` and at most one condition: `until`, an RFC 3339 instant or a
duration such as `2 hours`; `after`, another task's id; `merge`, a pull request as `owner/name#42`
or `42` in the trigger's repository, with `if_closed` set to `release`, `drop` or `ask`, the
default; or `blockers: true`. A trigger sets it as a table, such as
`hold = { reason = "triage first", blockers = true }`. A condition the daemon cannot use, such as
`blockers` on a GitHub issue, leaves the task held for a person, with the error in its reason.

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
| `github.pull_request.check_runs` | `github:{repo}:pulls/{pull_request}:check-runs:{head_sha}:{some or none}` |
| `priory.task.requested.cli` | `priory.task.requested.cli:{len}:{key}:{text}`, with the `cli:` key and `len` its byte length, so a `:` in the key or text cannot make two requests collide |
| `schedule.fired`         | `schedule.fired:{trigger}@{scheduled}`       |
| `factory.task.requested` | `factory.task.requested:{trigger}:{source}`  |
| `factory.intake.baseline` | `factory.intake.baseline:{repo}:{stream}`, or `factory.intake.baseline:{stream}:{position}` for a Linear stream or a Slack channel |
| `linear.issue.entered`, `linear.issue.left` | `linear.view:{view}:{position}:{issue id}:{type}`, with the view stream's new position |
| `linear.issue.created`   | `linear:issue:{issue id}:created`          |
| `linear.issue.state_changed` | `linear:history:{history id}`          |
| `linear.comment.created` | `linear:comment:{comment id}`              |
| `linear.issue.blockers`  | `linear:blockers:{blocked issue id}:{blocker issue id}:{blocker updatedAt}` |
| `slack.message.posted`, `slack.thread.replied` | `slack:{channel}:{ts}` |

## Baseline

A repository has four streams, each named after a type:

| Stream                                 | Reads                                        | Holds                                   | Position |
| -------------------------------------- | -------------------------------------------- | --------------------------------------- | -------- |
| `github.issues.labeled`                | `issues/events`, newest first                | issue and pull request `labeled`, pull request `closed` | issue event id |
| `github.issues.opened`                 | `issues`, newest first by creation           | issue and pull request `opened`          | number   |
| `github.pull_request_review.submitted` | GraphQL: every open pull request, and every pull request a new event above names | reviews | review id |
| `github.check_run.completed`           | the same query                               | check runs on each pull request's head commit, and snapshots of them | check run id |

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

## Linear streams

Linear has one stream per view that a trigger names, `linear.view:<view>`, and two streams for the
whole workspace, `linear.issues` and `linear.comments`. Unlike a GitHub stream, a Linear stream
publishes a `factory.intake.baseline` message after every poll that moves it, and starts the next
poll from the newest one. A missing marker means the stream is still in its baseline.

| Stream             | Reads                                                   | Position                     |
| ------------------ | ------------------------------------------------------- | ---------------------------- |
| `linear.view:<view>` | every issue of the view                               | a count of membership changes; the marker also holds `members`, the fields of each issue in the view |
| `linear.issues`    | issues updated at or after the position, archived ones included, with their last 50 history entries and the issues they block | the latest `updatedAt` seen; the marker also holds `seen`, the ids of the events at that instant |
| `linear.comments`  | comments created at or after the position               | the latest `createdAt` seen, and `seen` as above |

The first poll of a view publishes each of its issues as `linear.issue.entered` with `baseline:
true`. The first poll of the other two reads their 50 most recent nodes, and publishes what they
hold with `baseline: true`. As for GitHub, a baseline event requests a task only for a `backfill`
trigger. When that first page is empty, the stream starts from 1970.

```json
{
  "type": "factory.intake.baseline",
  "subject": "linear.comments",
  "data": {
    "stream": "linear.comments",
    "position": "2026-11-17T11:00:00.000Z",
    "seen": ["f0352bb3-8810-574d-940d-8d6c29e73f60"]
  }
}
```

A poll reads again the events at its stream's position, because two events can share a millisecond.
It skips those the marker lists in `seen`, so a baseline event read again never requests a task.

A crash between an event and its stream's marker leaves events after the newest marker. On start,
the intake rebuilds the marker that poll would have published from those events, and publishes it.
It also publishes again the task requests of the newest of them, from its `triggers`, as the GitHub
source does, and herdr-bus drops the repeats. Polling then goes on from the rebuilt marker, so it
never publishes those events again with other triggers, and a view reports an issue that left
during the crash.

Limits:

- A view event's `time` is when the intake noticed the change, at most one poll late.
- An issue that enters and leaves a view between two polls produces no event.
- As for GitHub, start-up recovery never dispatches a baseline event, so a crash between a baseline
  event and the task request of a `backfill` trigger loses that request.
- A task's key names the issue identifier. Moving the issue to another team changes its
  identifier, so later events of that issue no longer reach the task.
- A view marker holds every issue of the view and is published whenever one of them changes, so a
  large, busy view grows the output.
- A state change past the last 50 entries of an issue's history between two polls is missed.
- `blockers` reads 20 relations of an issue, and `linear.issue.blockers` follows 20 relations of
  each updated blocker, so an issue with more is unknown, and a blocker with more misses some.
- A blocking relation that is removed, rather than its blocker closing, publishes no event about the
  issue it blocked: the blocker no longer names it. A hold on its blockers stays until another event
  about the issue says it has none, or a person releases it.

## Linear calls

The intake reads the API key from the environment variable `LINEAR_API_KEY`, and refuses to start
without it when the factory has a Linear trigger. It posts each GraphQL query to
`https://api.linear.app/graphql` with `curl`, which reads the key from its standard input, so the
key never appears in a process list. `--disable` makes curl ignore `~/.curlrc`, so no trace option
can log the key, and the intake replaces the key in any error it prints. A request times out after
60 seconds, and the next poll retries it. A poll makes one query per view and two for
issues and comments, plus one per extra page of 100, and one for each 50 issues that the updated
issues block. Linear allows 1,500 requests an hour per key.

## Slack calls

The intake reads a bot token from the environment variable `SLACK_BOT_TOKEN`, and refuses to start
without it when the factory has a Slack trigger. It calls `https://slack.com/api/<method>` with
`curl --get`, which reads the `Authorization` header from its standard input as for Linear, so the
token never appears in a process list or an error. On start it calls `auth.test` once to learn its
own bot. Each poll then makes one `conversations.history` call per trigger channel and one
`conversations.replies` call per watched thread, plus one per extra page. The bot needs the
`channels:history` scope, and `groups:history` for a private channel, and must be a member of the
channel. Slack limits these two methods per app: an app outside the Slack Marketplace created after
May 2025 gets one call a minute and 15 messages per call, so it can watch one channel or thread per
minute at most.

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
