# Lanes

A lane is a `<team>/<theme>` label, such as `core/intake`, that groups tasks in the listings, the
reports and the spend caps. The file `lanes.toml`, `.yaml` or `.yml`, beside `factory.toml`, gives a
lane to each task whose trigger or request names none. Without the file, every such task gets
`<team>/misc`.

Schema: [`config/lanes.json`](../schemas/config/lanes.json).

## Top-level keys

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `fallback` | string | no | `"{team}/misc"` | The lane of a task that no rule matches. `{team}` stands for the task's team, or `none` for a task with no team | `"{team}/misc"` |
| `rules` | array of tables | no | `[]` | The rules, in the order they are tried. The first rule that matches gives the lane | see below |

## Rule keys

A rule names at least one of `key`, `team`, `project` and `branch`. A rule that names several
matches only a task that fits all of them.

| Key | Type | Required | Meaning | Example |
| --- | ---- | -------- | ------- | ------- |
| `lane` | string | yes | The lane the rule gives, `<team>/<theme>` | `"core/intake"` |
| `key` | string | no | The task's key, exactly | `"linear:ENG-7"` |
| `team` | string | no | The team of the task's source event, exactly. Linear events carry one | `"ENG"` |
| `project` | string | no | The project of the task's source event, exactly | `"Desktop"` |
| `branch` | string | no | A pattern for the task's branch, in which `*` matches any run of characters | `"*desktop*"` |

## When a lane is set

1. A lane that the trigger's [`lane`](triggers.md#keys-every-trigger-takes) or the request's
   `--lane` names is kept, and no rule changes it.
2. Otherwise the daemon derives the lane when it creates the task. The task has no branch yet, so
   a rule with `branch` cannot match.
3. When the task's first run creates its branch, the daemon derives the lane once more, with the
   branch. The lane does not change after that: a later edit of the file moves no task that has a
   branch.

## Example

<!-- example: factory/lanes.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/lanes.json
fallback = "{team}/misc"       # {team} is the task's team, or `none` without one; this is the default

[[rules]]
key = "linear:ENG-7"           # the task's key, exactly
lane = "core/intake"

[[rules]]
team = "ENG"                   # a rule that names several fields needs all of them
project = "Desktop"
lane = "core/desktop"

[[rules]]
branch = "*desktop*"           # a glob on the task's branch; `*` matches any run of characters
lane = "core/desktop"
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` lane `<lane>` is not `<team>/<theme>` `` | `fallback` or a rule's `lane` is not two parts around one `/`, or a part is empty or holds a space |
| `` a rule needs `key`, `team`, `project` or `branch` `` | A rule that names only a `lane` |
| `` unknown field `<key>`, expected one of ... `` | A key the tables above do not list |
| `` `lanes` is also defined in lanes.toml `` | Two of `lanes.toml`, `lanes.yaml` and `lanes.yml` exist |
