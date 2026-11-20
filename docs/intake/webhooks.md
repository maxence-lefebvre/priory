# GitHub webhooks

GitHub can push events to Priory instead of waiting for the next poll. This page explains how a
signed delivery becomes the same message a poll publishes, and how the two never start one change
twice. The keys are under [`[webhooks.github]`](../reference/factory.md#webhooksgithub), and the
endpoint in [serve.md](../reference/serve.md#post-hooksgithub).

Polling keeps running. A webhook only makes an event arrive sooner. A delivery that never arrives
still reaches the factory through the next poll.

## The signature comes first

The server that [serve.md](../daemon/serve.md) describes takes deliveries at `POST /hooks/github`,
with no token. GitHub signs each delivery with the webhook's secret, and that signature is its
authentication:

1. The server reads the body that `Content-Length` announces, up to 1 MiB. A larger body gets `413`
   before it is read.
2. It computes the HMAC-SHA256 of the raw body under the secret, and compares it with
   `X-Hub-Signature-256` in constant time. A missing or wrong signature gets `401`. The body is
   dropped unparsed.
3. Only then does it parse the JSON, map it and publish it. It replies `202` within the request.

The secret lives in the environment variable `secret_env` names. Priory never stores or logs it.
Generate it once, keep it in the services' environment file, and paste the same value into
GitHub's webhook settings:

```sh
echo "PRIORY_GITHUB_WEBHOOK_SECRET=$(openssl rand -hex 32)" >> ~/.config/priory/env
```

GitHub caps a payload at 25 MB. The events Priory maps carry at most a 65,536-character body and
the repository's fields, well under 1 MiB. Subscribe the webhook to only Issues, Issue comments, Pull
requests, Pull request reviews and Pull request review comments: a `push` delivery can exceed the limit, and Priory does not map it.

## What a delivery publishes

A delivery publishes on `factory.intake`, with the type and fields the poll uses, so triggers and
routing treat it the same. It requests a task for each trigger it matches, as a poll does.

| Event and action | Publishes |
| ---------------- | --------- |
| `issues` `opened` | `github.issues.opened` |
| `issues` `labeled` | `github.issues.labeled` |
| `pull_request` `opened`, `labeled`, `closed` | `github.pull_request.opened`, `.labeled`, `.closed` |
| `pull_request_review` `submitted` | `github.pull_request_review.submitted` |
| `issue_comment` `created`, on a pull request | `github.pull_request.comment` |
| `pull_request_review_comment` `created` | `github.pull_request_review_comment` |
| anything else, `check_run` included | nothing; `202` |

A delivery for a repository that no trigger names publishes nothing, so a webhook never widens what
the factory acts on.

`check_run` publishes nothing because its payload lacks the counts the checks gate reads: how many
check runs on the head are still pending or failing. Polling keeps publishing check runs.

A webhook message differs from the poll's in two fields:

- `delivery` holds the `X-GitHub-Delivery` GUID. GitHub does not sign it, so treat it as a label.
- `github_id` is null on the labeled, closed and pull request opened types. Their payloads lack
  the issue event id and the issue id that the poll reads.

Its `time` is the payload's: `updated_at` for a label, `closed_at` for a close, `created_at` for an
opening or a comment, and `submitted_at` for a review. A bot's comment publishes nothing. An
`issue_comment` names no branch, so its head and base fields are null.

## Duplicates

Each message id is a UUIDv5, so herdr-bus drops a message whose id is already on the output (see
[envelope.md](envelope.md#duplicates)).

| Messages | Id name |
| -------- | ------- |
| `github.issues.opened`, `github.pull_request_review.submitted`, `github.pull_request.comment`, `github.pull_request_review_comment` | The poll's name. See [github-issues.md](github-issues.md#keys-and-ids) and [pull-requests.md](pull-requests.md#keys-and-ids) |
| `github.issues.labeled`, `github.pull_request.opened`, `.labeled`, `.closed` | `github:webhook:{event}:{sha256 of the body}` |

So GitHub's redelivery of a delivery, or a replay of its signed body under a forged GUID, makes the
same id and publishes nothing.

A webhook and a poll that see one change publish it once:

- Opened issues, reviews and comments share the poll's id, so herdr-bus drops the second.
- The other types cannot share it. Before it publishes one, each writer looks for the other
  writer's report of the same change: same type, issue or pull request, and label. A label must be
  within 60 seconds, a close at the same second. A pull request opens once.

That look goes back over `factory.intake`, newest first, for at most 7 days and 5,000 messages. The
poll looks only when it has such an event to publish. A lock file next to the herdr-bus database,
`<db>.changes.lock`, keeps the two writers from looking and publishing at the same time.

A delivery whose event is over 7 days old publishes nothing. GitHub redelivers for 3 days, so only a
replay is that old, and the look would no longer reach the poll's report of it.

## Reaching the server

The server binds `127.0.0.1` unless `[serve]` sets `public = true`, so GitHub cannot reach it by
default. Two ways open it, and each is the factory owner's choice for each machine:

- A tunnel to the loopback port, such as `tailscale funnel 7340`, which gives a public HTTPS URL.
- `public = true` with a `listen` address GitHub can reach, behind a TLS proxy.

Point the webhook at `<url>/hooks/github`, with content type `application/json` and the secret.

## Limits

- A label removed and re-added within 60 seconds, whose second delivery is lost, is dropped as a
  duplicate of the first.
- On a factory that publishes over 5,000 messages in 7 days, the look can stop before it reaches
  the other writer's report, and a late redelivery of a labeled, closed or opened change can
  publish it again.
- If the server stops between a delivery's event and its task requests before the intake's first
  poll, those task requests are lost. GitHub shows the delivery as failed; redeliver it.
- Check runs and check run snapshots come only from polling.
- A repository that only an open wait names, not a trigger, gets its events from polling only.
