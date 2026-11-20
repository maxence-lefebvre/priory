# GitHub polling

This page explains how the intake polls a GitHub repository: its streams, the baseline of a first
poll, the API cost, and how a restart resumes. The events themselves are in
[github-issues.md](github-issues.md) and [pull-requests.md](pull-requests.md).

## What it watches

A repository is polled when a trigger names it, whatever the trigger's source, or when an open wait
or `--merge` hold names a pull request in it (see [watches.md](watches.md)). It has four streams,
each named after a type:

| Stream | Reads | Holds | Position |
| ------ | ----- | ----- | -------- |
| `github.issues.labeled` | `issues/events`, newest first | issue and pull request `labeled`, pull request `closed` | issue event id |
| `github.issues.opened` | `issues`, newest first by creation | issue and pull request `opened` | number |
| `github.pull_request_review.submitted` | GraphQL: every open pull request, and every pull request a new event above names; REST: `pulls/comments`, newest 100 | reviews, comments and review thread comments | review id |
| `github.check_run.completed` | the same query | check runs on each pull request's head commit, and their snapshots | check run id |

## Baseline

The first poll of a stream is a baseline. It reads GitHub's first page: up to 100 recent issue
events, up to 100 recent issues, or every open pull request's reviews and checks. Then:

1. It publishes each event with `baseline: true`.
2. It requests no task, except for a `backfill = true` trigger, which gets one for each event it
   matches.
3. It publishes one `factory.intake.baseline` message for the stream, even when the page held no
   event. Consumers can ignore it.

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

`position` is the highest position on that page, or 0 when it is empty. Its fields are in
[events.md](../reference/events.md#factoryintakebaseline).

Until that message is on the output, the stream stays in its baseline. A crash in the baseline makes
the next start read the first page again as a baseline.

## Keys and ids

| Message | Id name |
| ------- | ------- |
| `factory.intake.baseline` | `factory.intake.baseline:{repo}:{stream}` |

The events' ids are on their own pages.

## Calls and cost

| Poll | Calls |
| ---- | ----- |
| A repository with N open pull requests and nothing new | 4 `gh` calls: 3 REST requests (`issues/events`, `issues` and `pulls/comments`, one page each) and 1 GraphQL query |
| N over 100 | the query pages itself: ceil(N/100) requests, about 4 points each on `cli/cli` |
| New pull request events | 1 more GraphQL query for the pull requests they name |
| Over 100 new events in a stream since the last poll | 1 more REST page per 100 |

At one poll a minute, that is 180 REST requests and about 240 GraphQL points an hour per repository,
each against GitHub's 5,000 an hour. Review thread comments come from REST because the query would
pay for every thread of every open pull request: about 20 more points per poll at 20 threads each.

Each `gh api` call runs in its own process group. Past `command_timeout` in `factory.toml`, 5
minutes by default, the call is killed and the poll fails with `timed out after <n>s`.

## Where polling stops

The intake keeps no state file. On start, it reads `factory.intake` back, newest first, to each
stream's `factory.intake.baseline` message. Where polling stopped is the higher of:

- that message's `position`;
- the position of the stream's newest source event.

If that newest event is not a baseline, the intake republishes its task requests for the triggers in
its `triggers` that the factory still defines. So a crash between a source event and its task
requests loses nothing, and herdr-bus drops the repeats. A trigger added since gets no such task,
because it is not in the list. Each poll then publishes, oldest first, the events after that point.

The review and check streams have no such point. Each poll reads the current reviews, comments and
checks and publishes those not on the output yet. The intake remembers the ids it published, and herdr-bus
drops the rest, so after a restart the first poll offers every current review and check again.

A poll publishes reviews and checks before the issue streams. So a crash after a pull request's
`closed` event cannot skip the reviews and checks that only that event made the poll read.

## Failures

| What fails | Then |
| ---------- | ---- |
| `gh` for one repository | that repository publishes nothing; the others still publish |
| `gh`, without `--once` | the error is logged and retried on the next poll. With `PRIORY_DB` set, it shows on the desk until a poll succeeds: see [desk.md](../daemon/desk.md#intake-banner) |
| `gh`, with `--once` | the intake polls the other sources, records the failure, then exits non-zero |
| a publish | the intake ends, so its restart recovers what it left out |

## Limits

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
