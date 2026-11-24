# Runners

A runner is a machine that runs agents. Each file `runners/<hostname>.toml`, `.yaml` or `.yml`
describes one, named by the machine's Tailscale hostname. Capacity is measured live, so the file
holds only the platform and the share of the machine its agents may take together.

Schema: [`config/runner.json`](../schemas/config/runner.json).

## Keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `platform` | table | yes | | The machine's operating system and CPU architecture | `{ os = "linux" }` |
| `platform.os` | `linux`, `macos` or `windows` | yes | | The operating system | `"linux"` |
| `platform.arch` | string | no | | The CPU architecture | `"x86_64"` |
| `agents` | table | no | 90% of each | The share of the machine the runner's agents may take together. See [agents-slice.md](../daemon/agents-slice.md) | `{ cpu = "50%" }` |
| `agents.cpu` | share | no | `"90%"` | The share of the machine's CPUs they may load | `"50%"` |
| `agents.memory` | share | no | `"90%"` | The share of the machine's memory they may take | `"60%"` |
| `filters` | list of tables | no | none | The tasks it claims and starts. See [Filters](#filters) | `[{ match = "!project=oss" }]` |
| `filters.match` | string | yes | | `key=value` accepts only tasks with that label. `!key=value` accepts only tasks without it | `"!project=oss"` |
| `filters.when` | table | no | always | When the filter applies. Outside it, the filter is lifted | `{ hours = "08:00-18:00" }` |
| `filters.when.days` | list of `mon` to `sun` | no | every day | The weekdays it applies on. With `hours` that pass midnight, the day they start on | `["mon", "tue", "wed", "thu", "fri"]` |
| `filters.when.hours` | string | no | the whole day | `HH:MM-HH:MM`, from the first time to before the second. It may pass midnight | `"08:00-18:00"` |
| `filters.when.timezone` | string | with two or more runner files | the machine's | The IANA zone the days and hours are read in | `"Europe/Paris"` |

A share is a number followed by `%`, above 0 and at most 100, such as `"60%"`.

## Example

<!-- example: factory/runners/laptop.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/runner.json
platform = { os = "linux", arch = "x86_64" }
agents = { cpu = "50%", memory = "60%" }   # a laptop in use; omit for 90% of each
```

## Filters

A runner claims and starts only the tasks that every filter whose window is open accepts. A
filter matches the task's labels, which its trigger's `labels` or `priory request --label` set
(see [triggers.md](triggers.md)). This desktop keeps tasks labelled `project=oss` off it on
weekday work hours, and takes them at night and at weekends:

```toml
platform = { os = "linux" }

[[filters]]
match = "!project=oss"
when = { days = ["mon", "tue", "wed", "thu", "fri"], hours = "08:00-18:00", timezone = "Europe/Paris" }
```

A factory of one runner applies its runner file's filters too. A task they hold waits, queued,
with the reason `runner desktop's filter !project=oss holds it until 18:00`. The time is in the
filter's timezone, with the weekday when it is not today, such as `Mon 08:00`. A filter with no
window holds its tasks with no time. [runners.md](../daemon/runners.md#placing-tasks-with-filters)
says when each runner checks them.

## Several runners

With two or more runner files, each task is owned by one runner at a time. The daemon is the
runner named by its machine's hostname, and refuses to start on a machine with no runner file:

```text
this machine, `<hostname>`, has no runner file in runners/, and the factory has several runners
```

A task that never started moves to another runner 30 minutes after its runner's last alive
message. `priory give [DIR] KEY RUNNER` moves any other once its runner's lease ends, or at once
with `--force`. See [runners.md](../daemon/runners.md).

A machine with no runner file serves every runner's asks and tasks with
`priory daemon --view-only [DIR]`, which claims and runs nothing. See
[A machine that only views](../daemon/runners.md#a-machine-that-only-views).

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` missing field `platform` `` | No `platform` table |
| `` missing field `os` `` | `platform` without `os` |
| `` unknown variant `<os>`, expected one of `linux`, `macos`, `windows` `` | An operating system not listed above |
| `` unknown field `<key>`, expected one of ... `` | A key the table above does not list |
| `"<text>" is not a share such as 60% or 90%` | A share without `%`, or outside 0 to 100 |
| `"<text>" is not a match such as "project=oss" or "!project=oss"` | A `match` that is not a label, with or without `!` |
| `"<text>" is not hours such as "08:00-18:00"` | `hours` that are not two `HH:MM` times joined by `-` |
| `"<text>" starts and ends at once: leave hours out for the whole day` | `hours` such as `"08:00-08:00"` |
| `"<day>" is not a day: use mon, tue, wed, thu, fri, sat or sun` | An unknown entry of `days` |
| `` `days` is empty, so it never fires `` | `days = []` |
| `"<zone>" is not a time zone such as "Europe/Paris"` | An unknown `timezone` |
| `` filter `<match>` has a window with no `timezone`, which a factory of <n> runners needs `` | A `when` without `timezone`, with two or more runner files |
| `` `<hostname>` is also defined in runners/<hostname>.toml `` | `<hostname>.toml` and `<hostname>.yaml` both exist |
