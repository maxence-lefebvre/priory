# `schedule.fired`

A trigger with `source = "schedule"` fires at `at`, a local time of day, in `timezone` (an IANA
name; the machine's zone when unset), on the weekdays `days` lists (every day when unset). With
`every`, it fires again every that long after `at`, the same day, up to `until` included: a time of
day such as `"18:00"` or a duration after `at` such as `"9h"`. `until` defaults to the end of the
day, and a duration stops there too. `every` is at least `1m`, the intake's poll.

Each poll publishes, for each schedule trigger, its latest firing at or before now: this source
event, then one `factory.task.requested`. The firing's time names both messages' ids, so a second
poll in the same slot publishes nothing new, and a restart after missed slots publishes only the
latest of them, once.

| Field       | Type     | Meaning                              |
| ----------- | -------- | ------------------------------------ |
| `trigger`   | string   | The trigger's name                   |
| `scheduled` | RFC 3339 | When the firing was due, in UTC       |

Its task request adds two fields to those below: `scheduled`, the same time, and `prompt`, the
trigger's `prompt` or null. Its `key` is `schedule:<key>` when the trigger sets a fixed `key`, so
every firing names the same work, and `schedule:<trigger>@<scheduled>` otherwise, so each firing is
its own task. The daemon routes a firing to a task that already owns a fixed key by the task's
state (see `docs/daemon/routing.md`).

```json
{
  "id": "5b0e9c8c-6c0a-5b8e-9a4d-2f7f3c1e8a10",
  "type": "factory.task.requested",
  "subject": "schedule:support-day",
  "time": "2026-11-19T09:00:00Z",
  "data": {
    "workflow": "support",
    "key": "schedule:support-day",
    "band": null,
    "lane": null,
    "trigger": "support-day",
    "source": "0f4a6d1e-3c1b-5d55-8a8e-6d2b0c9e7f21",
    "scheduled": "2026-11-19T09:00:00Z",
    "prompt": "Sweep the support queue."
  },
  "links": []
}
```

Limits:

- A new schedule trigger fires its latest past slot on its first poll, as a missed firing would.
- Only the latest slot fires, so a poll delayed past two slots, such as by a slow `gh`, skips the
  older one. The last slot of a day fires even when the poll that finds it comes after `until`.
- Changing `at`, `every` or `until` names new slots, so the latest of them can fire at once.
- `every` counts real time from `at`, so a daylight saving change on that day shifts the later
  firings by the hour it adds or removes.
