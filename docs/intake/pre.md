# `pre` scripts

A trigger's `pre` script runs on each event the trigger matches, before the task request is
published. It can let the request through, change some of its fields, or drop it. The script is
code a person wrote and merged into the factory, so a filter that code can decide needs no model.
Its keys are in [triggers.md](../reference/triggers.md#pre). To write one, follow
[How to filter and enrich events with a script](../filtering-and-enriching-events.md).

## What the script gets

Priory execs `argv` as the trigger file lists it, never through a shell, in the factory folder.
Its stdin is one JSON object, then end of file:

```json
{
  "event": { "id": "...", "type": "sentry.issue.created", "subject": "PYTHON-Y", "time": "...", "data": {} },
  "request": { "workflow": "bug-investigation", "key": "sentry:4812", "band": "today", "lane": null,
               "trigger": "sentry-api" }
}
```

`event` is the source event as the log holds it (see [events.md](../reference/events.md)).
`request` is the request the trigger built.

No field of an event or request is ever put in an argument, an environment variable or a shell
line. An issue title that holds `$(...)` reaches the script as bytes of a JSON string. What the
script prints is still untrusted text: it was computed from an event an outsider wrote, and it
reaches an agent's prompt as the event's own text does.

The environment holds only `PATH`, `LANG` and `HOME`, copied from the process that runs the
script, `PRIORY_FACTORY`, the factory folder, `PRIORY_TRIGGER`, the trigger's name, and the
variables `env` names. A variable the process lacks is left out.

## What its exit decides

| The script | The request |
| ---------- | ----------- |
| exits 0 and prints nothing | is published unchanged |
| exits 0 and prints a JSON object | is published with the object's fields applied |
| exits 1 | is dropped. Its first line on stdout, if any, is the reason |
| exits with another code, is killed, runs past its timeout, cannot start, or prints what the rules below refuse | failed: see [When it fails](#when-it-fails) |

A drop withholds the task request only. The source event is still published, so an event about an
existing task still reaches it.

The object may hold these fields, each a string. Any other field fails the script.

| Field | Effect | Checked |
| ----- | ------ | ------- |
| `title` | the task's title, in place of the event's | one line, at most 200 characters |
| `text` | the task's request text, in place of the event's | at most 16 KiB |
| `context` | text the task reads after its request, under a line that names the script | at most 16 KiB |
| `band` | the task's band | one of the factory's bands |
| `lane` | the task's lane | `<team>/<theme>` |
| `key` | which events are the same work | Priory adds the source's scope: `fp-7` from a Sentry trigger becomes `sentry:fp-7` |

The scope means a script cannot name another source's task. `github:owner/name#42` printed by a
Sentry trigger's script becomes `sentry:github:owner/name#42`, which no GitHub event reaches.

A GitHub, Linear or Slack event names its own issue, pull request or thread, not the printed key.
So the task the request creates or joins also subscribes to the resource the trigger named before
the script ran, such as `github:owner/name#43` or `linear:ENG-12`. Each merged issue's comments,
edits and closes still reach the task (see [routing.md](../daemon/routing.md#resources-what-a-task-owns)).

A script cannot change the workflow, the trigger, the repository, a hold or `intervention`.
`priory check` reads those from files to say which paths reach an irreversible step and who
answers asks.

A task reads `title` and `text` in place of its source event's: its first prompt shows the source
event with them, and `priory search` and the task's title use them. `context` follows the source
event in the first prompt, under `Context from the trigger's pre script <argv[0]>:`.

## Limits

- **Time.** `timeout` counts from the start, the writing of stdin included, so a script that never
  reads its input is killed like any other. Past it, Priory kills the script and its process
  group.
- **Output.** Priory reads stdout and stderr as they come, and keeps at most 64 KiB of each. A
  script that prints more on either is killed and fails. Priory keeps the last 2000 bytes of
  stderr for the failure's text.
- **Children.** When the script ends, Priory kills its process group, so a child it left running
  ends with it.

## When it fails

A script that fails did not decide. `on_error` says what happens to the request:

| `on_error` | The request |
| ---------- | ----------- |
| `pass`, the default | is published unchanged, with `pre.failed` set to how the script failed |
| `drop` | is dropped, and the `factory.intake.dropped` record holds `failed` |

The default loses nothing: a script with a typo lets the alerts through as tasks, and the desk says
so.

Either way, the process that ran the script posts one report from `intake`,
`pre of trigger <name>: <how it failed>`, which the desk shows as an alarm (see
[desk.md](../daemon/desk.md#intake-banner)). It posts again only when the text changes. The
trigger's next run that exits 0 or 1 marks the report read, whether or not it changed anything.

## One decision per event

For one event and one trigger, one message says what `pre` decided: a `factory.task.requested`
with its `pre` object, or a `factory.intake.dropped`. Both take the same id, the UUIDv5 of the
trigger and the source event that the request has (see [task-requests.md](task-requests.md#keys-and-ids)).
The log keeps the first message of each id. So a script that runs twice for one event, and answers
differently the second time, still leaves the first decision.

A script can run twice. An intake that stops after the script ran and before the publish runs it
again at its next start, and so does a server restarted while requests wait. So a `pre` script
reads and changes nothing outside. Effects belong in a workflow step.

`priory inbox` lists a dropped request after its source event, with `dropped by <script>: <reason>`
in its `line` column.

## Where it runs

| Source | Process |
| ------ | ------- |
| Polled sources: GitHub, Linear, Slack | the intake |
| Schedules | the intake, once per firing, and again for the latest firing at the intake's start |
| Webhook deliveries: GitHub, Linear, Sentry, CloudWatch, Honeycomb, Figma, Intercom | the server that took the delivery |

Each runs the script from its own machine's checkout of the factory. Every request is published
once for the whole factory, so `pre` runs once per event, not once per runner.

A webhook sender waits for its reply, and a script may take 60 seconds. So the server still replies
`202` once it published the source event. In that same step it stores the request in the state
file, then runs `pre` on the stored requests one at a time, oldest first, and publishes each
decision. A server that restarts finds the stored requests and runs them. It does so even when no
trigger has a `pre` script any more: a request whose trigger lost its script, or was removed, is
published as the trigger built it. A stored request that does not parse, such as one an older
Priory wrote, is deleted and posts one report from `priory`,
`dropped a stored webhook request that does not parse: <error>`, so the requests behind it run.

Each process runs one script at a time, so a slow script delays the events behind it by at most its
timeout each.

## Trying a script on one event

```sh
priory intake pre [DIR] TRIGGER < event.json
```

It runs the trigger's script on one source event, as the log holds it, and prints the message the
intake would publish, with `outcome` (`published` or `dropped`) and `failed`. It publishes
nothing and posts no report. See [cli.md](../reference/cli.md).
