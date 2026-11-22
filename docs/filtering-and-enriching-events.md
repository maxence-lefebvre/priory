# How to filter and enrich events with a script

This guide adds a `pre` script to a trigger, so that code decides which events become tasks and
what each task reads. It assumes a factory that passes `priory check` and has a trigger. How the
intake runs the script is in [pre.md](intake/pre.md), and its keys in
[triggers.md](reference/triggers.md#pre).

## 1. Write the script

Put an executable file in the factory folder, such as `scripts/sentry-pre`. It reads one JSON
object on stdin, with `event` and `request`, and answers through its exit code and stdout:

- Exit 0 and print nothing to keep the request.
- Exit 0 and print a JSON object to change it: `title`, `text`, `context`, `band`, `lane` or `key`.
- Exit 1 to drop it. The first line you print is the reason.

This one drops Sentry issues below a level:

```sh
#!/bin/sh
set -eu
level=$(sed -n 's/.*"level":"\([a-z]*\)".*/\1/p' | head -n 1)
case "$level" in
  debug|info|warning) echo "level $level is below error"; exit 1 ;;
esac
```

Other uses, each a few lines:

- **Enrich the text.** Ask another system how many users hit the error, and print
  `{"context": "312 users hit it today."}`.
- **Set the band from a label.** Print `{"band": "blocker"}` when the event's labels hold `p0`.
- **Merge duplicate alerts.** Print a `key` built from the alert's fingerprint rather than its id.
  A factory holds at most one active task per key, so the second alert creates no second task.

Keep the script free of effects. It only reads: the intake can run it twice for one event.
Post to Slack or label a pull request from a workflow step instead.

Make it executable:

```sh
chmod +x scripts/sentry-pre
```

## 2. Name it in the trigger

Add a `[pre]` table to the trigger file:

```toml
[pre]
argv = ["scripts/sentry-pre"]
timeout = "10s"                    # at most 60s
env = ["AMPLITUDE_READ_TOKEN"]     # variables the script needs, if any
```

Never put event text in `argv`: it comes from the trigger file only, and the event reaches the
script on stdin.

Choose `on_error` for a script that fails, such as one with a typo:

- `pass`, the default, publishes the request unchanged, so an alert is never lost.
- `drop` drops it, for a trigger where noise costs more than a missed event.

Either way the desk shows a report from `intake` until the script runs cleanly again.

## 3. Check the factory

```sh
priory check
```

It refuses a script outside the factory folder or without its execute bit, and a timeout over 60
seconds.

## 4. Try it on one event

Save one source event as JSON, such as a row of `priory inbox --json` or a message from the log,
then run:

```sh
priory intake pre sentry-api < event.json
```

It prints the message the intake would publish, with `outcome` (`published` or `dropped`) and
`failed`. It publishes nothing.

## 5. Restart the intake and the server

Restart `priory intake` for polled sources and schedules, and the process that serves the
webhooks, `priory daemon` or `priory serve`, for webhook sources. Each reads the trigger at start.

`priory inbox` then lists each dropped request after its source event, with the reason.
