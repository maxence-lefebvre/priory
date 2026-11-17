# Intake messages

`priory intake [DIR] [--once]` polls GitHub for the repositories that the factory's triggers name. It
publishes on the herdr-bus output `factory.intake`. This page is the contract for every consumer of
that output: the Priory daemon, and another factory such as herdr-bowtie.

The intake checks the factory first, as `priory check` does, and refuses to start while it has
problems. `--once` polls once and exits. Without it, the intake polls every 60 seconds. It reads
GitHub only, through `gh api`, as the user `gh` is logged in as.

## The envelope

Every message is a herdr-bus `Message`, shaped like a CloudEvent:

| Field     | Type          | Meaning                                                    |
| --------- | ------------- | ---------------------------------------------------------- |
| `id`      | UUID string   | A UUIDv5, stable across retries (see Message ids)          |
| `type`    | string        | `github.issues.labeled`, `github.issues.opened` or `factory.task.requested` |
| `subject` | string        | `owner/name#42` for a GitHub event, the task key for a task request |
| `time`    | RFC 3339      | When the GitHub event happened                             |
| `data`    | object        | The fields below, by type                                  |
| `links`   | array         | GitHub events carry one `{ "label": "issue", "url": ... }` |

herdr-bus drops a message whose `(output, id)` is already on the output. A consumer can still see a
message twice, after its own crash before its ack, so it deduplicates by `id`.

## `github.issues.labeled` and `github.issues.opened`

One message per GitHub event, published once. The intake publishes only these two types. Events on
pull requests, and other issue events such as `closed` or `mentioned`, are not published yet.

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
| `triggers`  | array of strings | The triggers the intake requested a task for, from this event; may be empty |

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

## `factory.task.requested`

One message per source event and trigger that matches it, the triggers its source lists in
`triggers`. A trigger matches when its `repo` is the event's repository, its `on` names the event's
type, and its `filter`, if any, names the event's `label`. A baseline event matches only a trigger
with `backfill = true`. Only this message creates a task.

| Field      | Type           | Meaning                                                      |
| ---------- | -------------- | ------------------------------------------------------------ |
| `workflow` | string         | The workflow the task starts                                 |
| `key`      | string         | Which events are the same work: `github:owner/name#42`       |
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
| `factory.task.requested` | `factory.task.requested:{trigger}:{source}`  |

## Baseline

The first poll of a repository and type is a baseline. It reads GitHub's first page: up to 100
recent issue events, or up to 100 recent issues. It publishes each of those events with
`baseline: true`, which records where polling stands. It requests no task for them, except for a
trigger with `backfill = true`, which gets a task for each event it matches.

## Where polling stops

The intake keeps no state file. On start, it reads `factory.intake` back, newest first, for the
newest `labeled` and `opened` message of each repository. Their issue event id and issue number are
where polling stopped. When such a message is not a baseline, the intake publishes again the task
requests of the triggers it lists, so a crash between a source event and its task requests loses
nothing; herdr-bus drops the repeats. A trigger added since gets no task from this, because it is not
in the list. Each poll
then publishes, oldest first, the events after that point. Without `--once`, a failing `gh` is
logged and retried on the next poll, while a failing publish ends the intake, so that its restart
recovers what it left out.

Limits:

- The point is kept per repository and type, not per trigger. A trigger added later for a repository
  the intake already polls starts from where that repository stands, so `backfill` has no effect
  on it.
- A crash between a baseline event and the task request of a `backfill` trigger loses that task
  request: start-up recovery never dispatches a baseline event.
- Until a repository has a `labeled` message, the intake reads only the first page of its issue
  events. A label added there and followed by 100 or more other issue events within one poll is
  missed.
- `opened` pages follow creation time and stop at issue numbers. An issue transferred or imported
  into the repository with an older creation time can be missed.
