# Envelope and message ids

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
