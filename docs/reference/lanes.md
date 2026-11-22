# `lanes.toml`

`lanes.toml`, or `lanes.yaml`, at the factory's root derives the lane of each task whose trigger
or request sets none, and sets how the queue treats a lane. The file is optional. Without it, every
such task gets the lane `{team}/misc`. The [PRD](../prd.md#lanes) explains when the daemon derives a
lane and when it derives it again.

## Top-level keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `fallback` | string | no | `"{team}/misc"` | The lane when no rule matches. `{team}` stands for the task's team, or `none` without one | `"{team}/misc"` |
| `rules` | list of tables | no | `[]` | The rules, tried in order. The first that matches sets the lane | see below |
| `lane` | table of tables | no | | Each lane's settings, by its `<team>/<theme>` name | see below |

## `[[rules]]`

A rule matches when every field it sets matches the task. It sets at least one.

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `key` | string | no | The task's key | `"3D-600"` |
| `team` | string | no | The team of the task's issue | `"3D"` |
| `project` | string | no | The project of the task's issue | `"Mobile"` |
| `branch` | string | no | A pattern on the task's branch, in which `*` matches any run of characters | `"*desktop*"` |
| `lane` | string | yes | The lane the rule sets, `<team>/<theme>` | `"3D/mobile"` |

## `[lane."<team>/<theme>"]`

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `max_running` | integer | no | no cap | The most of the lane's tasks that may have a live run at once, 1 or more. New starts past it wait. See [runs.md](../daemon/runs.md#at-most-so-many-running) | `2` |

## Example

```toml
fallback = "{team}/misc"

[[rules]]
key = "3D-600"
lane = "3D/mobile"

[[rules]]
branch = "*desktop*"
lane = "FND/desktop"

[lane."3D/mobile"]
max_running = 2
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` lane `<lane>` is not `<team>/<theme>` `` | A `fallback` or a rule's `lane` that is not `<team>/<theme>` |
| `` a rule needs `key`, `team`, `project` or `branch` `` | A rule with no field to match on |
| `` `[lane]` `<name>` is not a lane such as `team/theme` `` | A `[lane]` table whose name is not `<team>/<theme>` |
| `` `max_running` is 0: give at least 1, or hold the tasks instead `` | `max_running = 0` |
| `` unknown field `<key>`, expected one of ... `` | A key the tables above do not list |
