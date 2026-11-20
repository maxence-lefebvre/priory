# Envelope and message ids

Every message on `factory.intake` is a herdr-bus `Message`, shaped like a CloudEvent. This page
describes the envelope every type shares and how message ids are made. The `data` of each type is in
[events.md](../reference/events.md), and its JSON Schema in [`docs/schemas/events/`](../schemas/events/).

## The envelope

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `id` | UUID string | A UUIDv5, stable across retries. See [Message ids](#message-ids) |
| `type` | string | The message type, such as `github.issues.labeled` |
| `subject` | string | A short line for listings. See the table below |
| `time` | RFC 3339 | When the event happened. See the table below |
| `data` | object | The fields of the type |
| `links` | array | GitHub and Linear events carry one `{ "label": "issue" or "pull_request", "url": ... }`, with the event's `url`. Others carry none |

| Message | `subject` | `time` |
| ------- | --------- | ------ |
| GitHub event | `owner/name#42` | when it happened |
| Linear event | the issue identifier, such as `ENG-7` | when it happened; for a view event, when it was published |
| Slack event | `<channel>:<ts>`; the thread's `ts` for a reply | the instant `ts` names |
| `priory.task.requested.cli` | the task key | when the command ran |
| `factory.task.requested` | the task key | the source event's `time` |
| `schedule.fired` | `schedule:<trigger>@<time>` | its scheduled time |
| `factory.intake.baseline` | `owner/name <stream>`, or the Linear or Slack stream | when it was published |

```json
{
  "id": "98c30eca-6e69-591d-b9ab-d4731abf371a",
  "type": "github.issues.labeled",
  "subject": "acme/widgets#42",
  "time": "2026-11-17T15:18:55Z",
  "data": { "repo": "acme/widgets", "issue": 42, "...": "..." },
  "links": [{ "label": "issue", "url": "https://github.com/acme/widgets/issues/42" }]
}
```

## Duplicates

herdr-bus drops a message whose `(output, id)` is already on the output. After its own crash before
an ack, a consumer can still get a duplicate, so it deduplicates by `id`.

## Message ids

Each id is a UUIDv5 of a name, in the namespace:

```text
UUIDv5(NAMESPACE_URL, "https://github.com/maxence-lefebvre/priory/intake")
```

A retried publish makes the same name, so it keeps its id. Each source page lists its names under
"Keys and ids":

| Messages | Names on |
| -------- | -------- |
| GitHub issue events | [github-issues.md](github-issues.md#keys-and-ids) |
| GitHub pull request, review and check events | [pull-requests.md](pull-requests.md#keys-and-ids) |
| Linear events | [linear.md](linear.md#keys-and-ids) |
| Slack events | [slack.md](slack.md#keys-and-ids) |
| `schedule.fired` | [schedule.md](schedule.md#keys-and-ids) |
| `priory.task.requested.cli` | [cli.md](cli.md#keys-and-ids) |
| `factory.task.requested` | [task-requests.md](task-requests.md#keys-and-ids) |
| `factory.intake.baseline` | `factory.intake.baseline:{repo}:{stream}` for a GitHub stream. `factory.intake.baseline:{stream}:{position}:{seen}` for a Linear stream, with `seen` as JSON, `null` for a view. `factory.intake.baseline:{stream}:{position}` for a Slack channel |
