# Schedules

A schedule trigger fires at a time of day, on chosen weekdays, and can repeat until an end. Each
firing publishes a `schedule.fired` event and one task request. No network is involved.

## Trigger

Its keys are in [triggers.md](../reference/triggers.md#schedule).

| Key | Meaning |
| --- | ------- |
| `at` | The local time of day it fires, `HH:MM` |
| `timezone` | The IANA zone `at` is read in; the machine's zone when unset |
| `days` | The weekdays it fires on; every day when unset |
| `every` | Repeat at this interval after `at`, the same day. At least `1m`, the intake's poll |
| `until` | The last firing of the day, inclusive: a time of day such as `"18:00"`, or a duration after `at` such as `"9h"`. The end of the day when unset, and a duration stops there too |
| `key` | A fixed key, so every firing names the same task |
| `prompt` | What a firing tells the asleep task it wakes |

```toml
source = "schedule"
repo = "owner/name"
at = "09:00"
days = ["mon", "tue", "wed", "thu", "fri"]
every = "1h"
until = "18:00"
timezone = "Europe/Paris"
key = "support-day"
prompt = "Sweep the support queue."
workflow = "support"
```

## When it fires

Each poll publishes, per schedule trigger, its latest firing at or before now: the
`schedule.fired` event, then one `factory.task.requested`. After missed slots, a restart publishes
only the latest, once.

| The trigger has | The task request's key | So |
| --------------- | ---------------------- | -- |
| a fixed `key` | `schedule:<key>` | every firing names the same work |
| no `key` | `schedule:<trigger>@<scheduled>` | each firing is its own task |

The daemon routes a firing to a task that already owns a fixed key by that task's state. It wakes a
task asleep at a model step with the trigger's `prompt`, and coalesces the firing in any other
state, so firings never stack (see [routing.md](../daemon/routing.md)).

A run that finds problems hands each to a fix task with the `finding` tool, and the next firing's
prompt lists them (see [`finding`](../daemon/tools.md#findingkey-title-details)).

## Next firing

`priory schedules [DIR] [--json]` lists each schedule trigger of the factory in `DIR`, `.` by
default, with its next firing after now. `--json` prints `{"revision": N, "schedules": [...]}`,
with these fields in each entry:

| Field | Type | Meaning |
| ----- | ---- | ------- |
| `trigger` | string | The trigger's name |
| `workflow` | string | The workflow a firing starts |
| `next` | string or null | The first slot after now, in UTC. After `until`, the first slot of the next firing day |
| `key` | string or null | `schedule:<key>` for a fixed `key` |
| `task` | integer or null | The task not done, failed or dropped that the key names, which the firing wakes or coalesces into |
| `task_state` | string or null | That task's state. Only an `asleep` task wakes |

The next firing reads the same slots as the intake. The queue's [up next](../reference/serve.md#queue)
lists the firings that start or wake a task.

## Events

The fields and examples are in [events.md](../reference/events.md#schedulefired).

| Message | Data |
| ------- | ---- |
| `schedule.fired` | `trigger` and `scheduled`, when the firing was due, in UTC |
| `factory.task.requested` | the usual fields, plus `scheduled` and `prompt`, the trigger's `prompt` or null |

```json
{
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
  }
}
```

## Keys and ids

| What | Value |
| ---- | ----- |
| `schedule.fired` subject | `schedule:<trigger>@<scheduled>` |
| `schedule.fired` id | the UUIDv5 of `schedule.fired:{trigger}@{scheduled}` |
| Task request id | the UUIDv5 of `factory.task.requested:{trigger}:{source}` |

The firing's time names both ids, so a second poll in the same slot publishes nothing new.

## Limits

- A new schedule trigger fires its latest past slot on its first poll, as a missed firing would.
- Only the latest slot fires. A poll delayed past two slots, such as by a slow `gh`, skips the
  older one.
- The last slot of a day fires even when its poll comes after `until`.
- Changing `at`, `every` or `until` names new slots, so the latest can fire at once.
- `every` counts real time from `at`. A daylight saving change that day shifts later firings by the
  hour it adds or removes.
