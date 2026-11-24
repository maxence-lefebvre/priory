# How to adopt open pull requests

This guide gives a task to each pull request you opened by hand, so its comments, reviews, checks
and merge land in that task's Inbox. No agent runs until a comment trigger matches. It assumes a
factory with a GitHub trigger on `pull_request.comment`, `pull_request_review_comment` or
`pull_request_review.submitted`, filtered on your `author`, and a running daemon. How a parked
task routes its events is in [routing.md](daemon/routing.md#parked-pull-requests).

## 1. Adopt every open pull request of a trigger

Name the trigger. Priory lists the trigger's repository's open pull requests by its `author` with
`gh pr list`, and parks a task on each one that no task owns. It also skips a pull request whose
head branch an active task owns:

```sh
priory adopt my-prs
```

```text
adopted github:owner/name#57
adopted github:owner/name#58
github:owner/name#61: task 12 owns it
```

To adopt only some, name them as `57`, `owner/name#57` or the pull request's URL:

```sh
priory adopt my-prs 57 https://github.com/owner/name/pull/58
```

A trigger that is not a comment or review trigger, or a pull request that is not open by the
trigger's `author`, is refused, and nothing is adopted.

Running it again adopts only pull requests that still have no task. A pull request whose task is
done, failed or dropped keeps that task: a comment reopens a done task, and `priory revive ID`
brings back a failed or dropped one.

## 2. Check the parked tasks

```sh
priory tasks --all
```

Each adopted task is `held`, with the reason
`held: adopted for trigger my-prs, until a comment or review trigger fires on github:owner/name#57`.
The desk, the Queue and `priory tasks` without `--all` list none of them. `priory task ID` lists the events the
task kept since the adopt.

## 3. Let a comment start the agent

Comment on the pull request as the trigger expects. The trigger's request releases the task, and
its first run reads the step's prompt, the pull request, and every event the task kept. To start
one now, run `priory release ID`.

When the pull request merges first, the task is done with no run. When it closes unmerged, the
task is dropped. A close that arrived before the adopt ends the task the same way.

Events from before the adopt are not added to the task: they stay in `priory inbox --unrouted`.
