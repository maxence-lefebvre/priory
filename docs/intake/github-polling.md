# GitHub polling

## Baseline

A repository has four streams, each named after a type:

| Stream                                 | Reads                                        | Holds                                   | Position |
| -------------------------------------- | -------------------------------------------- | --------------------------------------- | -------- |
| `github.issues.labeled`                | `issues/events`, newest first                | issue and pull request `labeled`, pull request `closed` | issue event id |
| `github.issues.opened`                 | `issues`, newest first by creation           | issue and pull request `opened`          | number   |
| `github.pull_request_review.submitted` | GraphQL: every open pull request, and every pull request a new event above names | reviews | review id |
| `github.check_run.completed`           | the same query                               | check runs on each pull request's head commit, and snapshots of them | check run id |

The first poll of a stream is a baseline. It reads GitHub's first page: up to 100 recent issue
events, up to 100 recent issues, or every open pull request's reviews and checks. It publishes each
event with `baseline: true` and requests no task, except for a `backfill = true` trigger, which gets
one for each event it matches. It then publishes one `factory.intake.baseline` message for the
stream, even when the page held no event. Consumers can ignore it.

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

Until that message is on the output, the stream stays in its baseline. A crash in the baseline
makes the next start read the first page again as a baseline.

## GitHub calls

One poll of a repository with N open pull requests and nothing new makes 3 `gh` calls: 2 REST
requests (`issues/events` and `issues`, one page each) and 1 GraphQL query. The query pages itself
when N is over 100 (ceil(N/100) requests) and costs 3 points per page on `cli/cli`. A poll that
finds new pull request events adds 1 GraphQL query for the pull requests they name. At one poll a
minute, that is 120 REST requests and about 180 GraphQL points an hour per repository, each against
GitHub's 5,000 an hour. A stream with over 100 new events since the last poll reads one more REST
page per 100.

## Where polling stops

The intake keeps no state file. On start, it reads `factory.intake` back, newest first, to each
stream's `factory.intake.baseline` message. Where polling stopped is the higher of that message's
`position` and the position of the stream's newest source event. If that newest event is not a
baseline, the intake republishes its task requests for the triggers in its `triggers` that the
factory still defines. So a crash between a source event and its task requests loses nothing, and
herdr-bus drops the repeats. A trigger added since gets no such task, because it is not in the
list. Each poll then publishes, oldest first, the events after that point.

The review and check streams have no such point. Each poll reads the current reviews and checks and
publishes those not on the output yet. The intake remembers the ids it published, and herdr-bus
drops the rest, so after a restart the first poll offers every current review and check again. A
poll publishes reviews and checks before the issue streams, so a crash after a pull request's
`closed` event cannot skip the reviews and checks that only that event made the poll read. A
failing `gh` publishes nothing for its repository, and the other repositories still publish.
Without `--once`, the error is logged and retried on the next poll. With `--once`, the intake exits
non-zero after publishing the rest. A failing publish ends the intake, so its restart recovers what
it left out.

Limits:

- The point is kept per repository and type, not per trigger. A trigger added later for an already
  polled repository starts where that repository stands, so `backfill` has no effect.
- A crash between a baseline event and the `backfill` trigger's task request loses that request.
  Start-up recovery never dispatches a baseline event.
- A trigger removed after a crash, before the restart, gets no recovered task request.
- An event during a crash-interrupted baseline, before the restart, lands on the replayed first page
  and is published as a baseline. Only a `backfill` trigger gives it a task.
- Each start reads the output back to the baseline markers, which grows with the output.
- `opened` pages follow creation time and stop at issue numbers. An issue transferred or imported
  into the repository with an older creation time can be missed.
- Head and base fields come from the pull request when the intake polls, not when the event
  happened.
- Reviews and checks are read for open pull requests and for any pull request a new issue event
  names, such as its `closed` event. A check that completes after its pull request closed and a
  poll passed is missed.
- The query reads the last 50 reviews and the first 100 checks of the head commit per pull request.
