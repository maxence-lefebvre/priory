# `schedule.fired`

A trigger with `source = "schedule"` fires at `at`, a local time of day, in `timezone` (an IANA
name; the machine's zone when unset), on the weekdays in `days` (every day when unset). With
`every`, it repeats at that interval after `at` on the same day, up to `until` inclusive. `until` is
a time of day such as `"18:00"` or a duration after `at` such as `"9h"`. It defaults to the end of
the day, and a duration stops there too. `every` is at least `1m`, the intake's poll.

Each poll publishes, per schedule trigger, its latest firing at or before now: this source event,
then one `factory.task.requested`. The firing's time names both messages' ids, so a second poll in
the same slot publishes nothing new. After missed slots, a restart publishes only the latest, once.

| Field       | Type     | Meaning                              |
| ----------- | -------- | ------------------------------------ |
| `trigger`   | string   | The trigger's name                   |
| `scheduled` | RFC 3339 | When the firing was due, in UTC       |

Its task request adds two fields to those below: `scheduled`, the same time, and `prompt`, the
trigger's `prompt` or null. Its `key` is `schedule:<key>` when the trigger sets a fixed `key`, so
every firing names the same work. Otherwise it is `schedule:<trigger>@<scheduled>`, so each firing
is its own task. The daemon routes a firing to a task already owning a fixed key by that task's
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
- Only the latest slot fires, so a poll delayed past two slots (such as by a slow `gh`) skips the
  older one. The last slot of a day fires even when its poll comes after `until`.
- Changing `at`, `every` or `until` names new slots, so the latest can fire at once.
- `every` counts real time from `at`, so a daylight saving change that day shifts later firings by
  the hour it adds or removes.
