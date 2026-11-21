# Review requests

The intake turns review requests into events: a pull request that asks a person for a review, and
one that stops asking. It reads them in every repository the `gh` user can see, not only the ones a
trigger names.

## What it watches

| Stream | Reads | Started by |
| ------ | ----- | ---------- |
| `github.review_requests:<login>` | GitHub search `is:pr is:open user-review-requested:<login>`, and `team-review-requested:<org/team>` for each of its triggers' `teams`, every page of 100 | a `pull_request.review_requested` trigger naming `<login>` as its `reviewer` |

`user-review-requested` matches only the pull requests that ask the person directly. A request to a
team matches only when a trigger for that person lists the team in `teams`. The stream holds the
union of both searches, so every trigger naming the reviewer sees the requests of every team those
triggers list. GitHub drops a request from the search once its reviewer submits a review, and lists
it again when someone asks again.

## Trigger

A `github` trigger with `on = "pull_request.review_requested"` and a `reviewer` starts a task for
each new request. Its keys are in [triggers.md](../reference/triggers.md#review-requests).

```toml
source = "github"
on = "pull_request.review_requested"
reviewer = "octocat"
teams = ["acme/core"]
workflow = "review"
```

It takes no `repo`: each task works in its pull request's repository, which the task request carries
in `repo`. A request to a team in `teams` starts a task keyed by `reviewer`, as a direct request
does. The reviews query reads only the reviewer's own reviews, so a team request that leaves the
search after a teammate's review says `reviewed: false`, and the reviewer's task is dropped.

## Events

Their fields and examples are in [events.md](../reference/events.md#githubpull_requestreview_requested).

Each poll compares the search's pull requests with those in the stream's last marker, as a Linear
view does (see [linear.md](linear.md#events)):

| Type | One message per |
| ---- | --------------- |
| `github.pull_request.review_requested` | pull request in the search that was not in the last marker |
| `github.pull_request.review_request_removed` | pull request in the last marker that is not in the search |

A pull request leaves the search for three reasons: the reviewer reviewed it, someone removed the
request, or it closed. For each one that left, the intake reads the reviewer's last 20 reviews of it.
`reviewed` is true when one was submitted at or after `since`, the time the intake first found the
request. A pending review has no submission time, so it does not count. When a review and a
removal or a close fall between two polls, the event says `reviewed: true`.

## Tasks

| Event | What the daemon does |
| ----- | -------------------- |
| `review_requested` | The trigger requests a task with the key `github:owner/name:pulls/43:review:octocat` |
| `review_request_removed`, `reviewed: false` | It drops the active task with that key, with the reason `review request removed`, and ends its live run |
| `review_request_removed`, `reviewed: true` | The task with that key keeps it, as any event |
| A second `review_requested` for the same pull request | Its task request wakes the task with that key if it sleeps at a model step, as a schedule firing does. Otherwise it is coalesced into it |

So a task that reviews, then sleeps, wakes when the author asks again. The re-request has its own
message id, since the stream's position is part of it. A done, failed or dropped task is not
active, so a re-request after it creates a new task with the same key.

## Baseline and markers

The first poll is the baseline. It publishes each pull request in the search as
`review_requested` with `baseline: true`, and requests a task only for a `backfill = true` trigger.

After each poll that changes the set, the stream publishes a `factory.intake.baseline` marker:

```json
{
  "stream": "github.review_requests:octocat",
  "position": 2,
  "members": [
    {
      "repo": "acme/widgets",
      "pull_request": 43,
      "title": "Fix the empty export",
      "url": "https://github.com/acme/widgets/pull/43",
      "author": "hubot",
      "since": "2026-11-20T09:01:00.214Z"
    }
  ]
}
```

`position` counts the changes. On start, the intake reads the output back to each stream's newest
marker.

A crash can stop a poll between its events and its marker. On start, the intake then:

1. Rebuilds the marker that poll would have published, from those events, and publishes it.
2. Republishes their task requests. herdr-bus drops the repeats.
3. Polls on from the rebuilt marker. A request that changed during the crash publishes that change
   with new ids, so a re-request is never dropped as a repeat.

| Message | Id name |
| ------- | ------- |
| `review_requested`, `review_request_removed` | `github.review_requests:{login}:{position}:{owner/name}#{n}:{type}`, with the stream's new position |
| `factory.intake.baseline` | `factory.intake.baseline:github.review_requests:{login}:{position}` |

## Failures

A failed search is not an empty set. When a search page or a reviews query fails, or GitHub reports
incomplete results, or the pages hold more or fewer results than `total_count`, that reviewer's
stream publishes nothing and keeps its marker. The next poll tries again. With `--once`, the intake
exits non-zero.

## Calls and cost

| Poll | Calls |
| ---- | ----- |
| Each reviewer, and each team in its triggers' `teams` | 1 REST search request per page of 100, at most 10. GitHub allows 30 search requests a minute per user |
| Each pull request that left the set | 1 GraphQL query |

## Limits

- A crash in the first poll, before its marker, makes the next start poll the baseline again, as a
  Linear view does. With `backfill = true`, a request removed during that crash then never
  publishes its removal, so its task stays active.
- At most 10 pages of 100, 1,000 requests, per search: GitHub search returns no more. Past that,
  every poll of the reviewer fails rather than read a partial set.
- A request that arrives or leaves between two pages of a search shifts the later pages. When the
  pages then hold more or fewer results than `total_count`, the poll fails and the next one reads
  again. A shift the counts do not show can miss one pull request for a poll: it publishes as
  removed, and as requested again at the next poll.
- A pull request the `gh` user can no longer read leaves the search, and GitHub answers its reviews
  query with `Could not resolve`. The intake then publishes it with `reviewed: false`, so its task
  is dropped.
