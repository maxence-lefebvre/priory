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
| `type`    | string        | `github.issues.labeled`, `github.issues.opened`, `factory.task.requested` or `factory.intake.baseline` |
| `subject` | string        | `owner/name#42` for a GitHub event, the task key for a task request, `owner/name <stream>` for a baseline marker |
| `time`    | RFC 3339      | When the GitHub event happened; for a baseline marker, when it was published |
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
| `factory.intake.baseline` | `factory.intake.baseline:{repo}:{stream}`   |

## Baseline

The first poll of a repository and type, a stream, is a baseline. It reads GitHub's first page: up to
100 recent issue events, or up to 100 recent issues. It publishes each of those events with
`baseline: true`, and requests no task for them, except for a trigger with `backfill = true`, which
gets a task for each event it matches. It then publishes one `factory.intake.baseline` message for
the stream, even when the page held no event. A consumer can ignore that message.

| Field      | Type    | Meaning                                                                |
| ---------- | ------- | ---------------------------------------------------------------------- |
| `repo`     | string  | `owner/name`                                                           |
| `stream`   | string  | `github.issues.labeled` or `github.issues.opened`                      |
| `position` | integer | The highest issue event id, or issue number, on that page; 0 when empty |

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

## Where polling stops

The intake keeps no state file. On start, it reads `factory.intake` back, newest first, to each
stream's `factory.intake.baseline` message. Where polling stopped is the higher of that message's
`position` and the position of the stream's newest source event. When that newest event is not a
baseline, the intake publishes again its task requests, for the triggers in its `triggers` that
the factory still defines. A crash between a source event and its task requests then loses
nothing, and herdr-bus drops the repeats. A trigger added since gets no task from this, because it
is not in the list. Each poll then publishes, oldest first, the events after that point. Without `--once`, a failing `gh` is
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
