# Triggers

A trigger names an event source, a filter and the workflow its events start. Each file
`triggers/<name>.toml`, `.yaml` or `.yml` defines one trigger, named after the file. The intake
reads the triggers, polls their sources and publishes one task request per matching event (see
[task-requests.md](../intake/task-requests.md)).

Schema: [`config/trigger.json`](../schemas/config/trigger.json). It covers every source, so it
cannot tell which keys a source refuses; `priory check` does.

- [Keys every trigger takes](#keys-every-trigger-takes)
- [`github`](#github)
- [`linear`](#linear)
- [`linear.view`](#linearview)
- [`sentry`](#sentry)
- [`cloudwatch`](#cloudwatch)
- [`honeycomb`](#honeycomb)
- [`figma`](#figma)
- [`intercom`](#intercom)
- [`slack`](#slack)
- [`schedule`](#schedule)
- [Task requests: `triggers/cli.toml` and `hold`](#task-requests-triggersclitoml-and-hold)
- [`intervention` and `lead`](#intervention-and-lead)
- [`pre`](#pre)
- [Errors from `priory check`](#errors-from-priory-check)

## Keys every trigger takes

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `source` | string | yes | | `github`, `linear`, `linear.view`, `sentry`, `honeycomb`, `intercom`, `cloudwatch`, `figma`, `slack` or `schedule` | `"github"` |
| `workflow` | string | yes | | The workflow a matching event starts, by file name | `"implement"` |
| `repo` | string | per source | | The repository the trigger's tasks work in, a GitHub `owner/name`. `priory check` refuses any other shape, such as `..` or a path | `"owner/name"` |
| `key` | string | no | per source | Which events are the same work. A factory holds at most one active task per key | `"issue.number"` |
| `band` | string | no | the workflow's `band`, else the last band | The band of its tasks | `"today"` |
| `lane` | string | no | derived by `lanes.toml` | The lane of its tasks, `<team>/<theme>` | `"core/intake"` |
| `hold` | table | no | | Files each task held. See [`hold`](#hold) | `{ reason = "triage first" }` |
| `intervention` | `plan` or `hands-off` | no | a person answers every ask | Who answers its tasks' asks. See [`intervention` and `lead`](#intervention-and-lead) | `"plan"` |
| `lead` | string | with `hands-off` | | The agent that answers asks of a `hands-off` task | `"lead"` |
| `caps` | table | no | no cap | Spend caps in dollars for its tasks: `task`, `day` and `week`, each above zero. See [spend.md](../daemon/spend.md) | `{ task = 2.0, day = 10.0 }` |
| `pre` | table | no | | A script that runs on each matching event before its task request, and can drop or change it. See [`pre`](#pre) | `{ argv = ["scripts/sentry-pre"] }` |

Without a `repo`, a Linear, Sentry, Honeycomb, Intercom or Slack trigger's tasks block before their first run.

## `github`

Starts a task when an issue or pull request of `repo` is opened or labeled. See
[github-issues.md](../intake/github-issues.md) and [pull-requests.md](../intake/pull-requests.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `repo` | string | yes | | The repository the intake polls, `owner/name` | `"owner/name"` |
| `on` | string | yes | | `issues.labeled`, `issues.opened`, `pull_request.labeled` or `pull_request.opened`. For `pull_request.review_requested`, see [Review requests](#review-requests) | `"issues.labeled"` |
| `filter` | table | no | | `label`: only events that add this label. `*.labeled` only | `{ label = "agent" }` |
| `key` | string | no | the number | `issue.number`, or `pull_request.number` for a `pull_request.*` event. The task key is `github:owner/name#42` | `"issue.number"` |
| `backfill` | boolean | no | `false` | Whether the first poll's events start tasks too | `true` |

<!-- example: factory/triggers/agent-label.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "github"
repo = "owner/name"            # the repository the intake polls
on = "issues.labeled"          # or issues.opened, pull_request.labeled, pull_request.opened
filter = { label = "agent" }   # *.labeled only
workflow = "implement"
key = "issue.number"           # pull_request.number for a pull_request event
band = "today"
lane = "core/intake"           # otherwise lanes.toml derives it
backfill = false               # true: the first poll's events start tasks too
```

### Review requests

A `github` trigger with `on = "pull_request.review_requested"` starts a task when someone asks
`reviewer`, or one of its `teams`, for a review of an open pull request, in any repository `gh` can read. It polls
no repository and takes no `repo`, `filter` or `key`: its tasks work in the pull request's
repository. See [review-requests.md](../intake/review-requests.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `pull_request.review_requested` | `"pull_request.review_requested"` |
| `reviewer` | string | yes | | The GitHub login asked for reviews. The task key is `github:owner/name:pulls/43:review:octocat` | `"octocat"` |
| `teams` | list of strings | no | | GitHub teams, as `org/team`, whose review requests start a task for `reviewer` too, keyed by `reviewer` as a direct request is. `priory check` refuses a team that is not an organization login and a team slug | `["acme/core"]` |
| `backfill` | boolean | no | `false` | Whether the requests open at the first poll start tasks too | `true` |

<!-- example: factory/triggers/review-requests.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "github"
on = "pull_request.review_requested"
reviewer = "octocat"           # the GitHub login asked for reviews, in any repository
workflow = "implement"
band = "today"
```

## `linear`

Starts a task when a Linear issue is created, changes state or gets a comment, anywhere in the
workspace. The intake reads the key from `LINEAR_API_KEY`. See [linear.md](../intake/linear.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `issue.created`, `issue.state_changed` or `comment.created` | `"issue.created"` |
| `filter` | table | no | | `team` (a team key), `project` (the project's name or id), `label` (one of the issue's labels) and `state` (the state it moved to, or its state). Every key must match | `{ project = "Mobile" }` |
| `key` | string | no | the identifier | `issue.identifier`. The task key is `linear:ENG-7` | `"issue.identifier"` |
| `backfill` | boolean | no | `false` | Whether the first poll's events start tasks too | `true` |

<!-- example: factory/triggers/triage.yaml -->
```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source: linear
on: issue.created              # or issue.state_changed, comment.created
filter:                        # every key must match
  team: ENG                    # the issue's team key
  project: Desktop             # the project's name or id
  label: bug                   # one of the issue's labels
repo: owner/name
workflow: implement
key: issue.identifier
hold:                          # file each task held until a person releases it
  reason: triage first
```

## `linear.view`

Starts a task when an issue enters or leaves a saved Linear view. See
[linear.md](../intake/linear.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `view` | string | yes | | The saved view: its id, or the slug id ending its URL | `"6b3c2f9a1d4e"` |
| `on` | string | yes | | `issue.entered` or `issue.left` | `"issue.entered"` |
| `key` | string | no | the identifier | `issue.identifier`. The task key is `linear:ENG-7` | `"issue.identifier"` |
| `backfill` | boolean | no | `false` | Whether the issues in the view at the first poll start tasks | `true` |

A `linear.view` trigger takes no `filter`: the view filters.

<!-- example: factory/triggers/agent-ready.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "linear.view"
view = "6b3c2f9a1d4e"          # a saved view: its id, or the slug id ending its URL
on = "issue.entered"           # or issue.left
repo = "owner/name"            # where its tasks work; without it they block
workflow = "implement"
key = "issue.identifier"       # the task key is linear:ENG-7
band = "now"
```

## `sentry`

Starts a task when Sentry creates an issue. Sentry pushes it to `POST /hooks/sentry`, so the
trigger needs [`[webhooks.sentry]`](factory.md#webhookssentry). See
[sentry.md](../intake/sentry.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `issue.created` | `"issue.created"` |
| `filter` | table | no | | `project` (the project's slug) and `level` (such as `error` or `fatal`). Every key must match | `{ project = "api" }` |
| `key` | string | no | the issue id | `issue.id`. The task key is `sentry:1234567890` | `"issue.id"` |

A `sentry` trigger takes no `backfill`: Sentry sends only new issues.

<!-- example: factory/triggers/sentry-errors.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "sentry"
on = "issue.created"
filter = { project = "api", level = "error" }   # the project's slug, and the issue's level
repo = "owner/name"            # where its tasks work; without it they block
workflow = "implement"
key = "issue.id"               # the task key is sentry:1234567890
```

## `cloudwatch`

Starts a task when a CloudWatch alarm changes state. An EventBridge API destination pushes the
change to `POST /hooks/cloudwatch`, so the trigger needs
[`[webhooks.cloudwatch]`](factory.md#webhookscloudwatch). See
[cloudwatch.md](../intake/cloudwatch.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `alarm.state_changed` | `"alarm.state_changed"` |
| `filter` | table | no | | `name` (the alarm name), `state` (the new state: `OK`, `ALARM` or `INSUFFICIENT_DATA`) and `previous_state` (the state before). Every key must match | `{ state = "ALARM" }` |
| `key` | string | no | the alarm ARN | `alarm.arn`. The task key is `cloudwatch:<alarm ARN>` | `"alarm.arn"` |

A `cloudwatch` trigger takes no `backfill`: EventBridge sends only new changes.

<!-- example: factory/triggers/cloudwatch-alarms.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "cloudwatch"
on = "alarm.state_changed"
filter = { state = "ALARM", previous_state = "OK" }   # the alarm fired after it was OK
repo = "owner/name"            # where its tasks work; without it they block
workflow = "implement"
key = "alarm.arn"              # the task key is cloudwatch:<alarm ARN>
```

## `honeycomb`

Starts a task when a Honeycomb Trigger fires. Honeycomb pushes the alert to
`POST /hooks/honeycomb`, so the trigger needs
[`[webhooks.honeycomb]`](factory.md#webhookshoneycomb) and the payload template in
[honeycomb.md](../intake/honeycomb.md#the-payload-template).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `trigger.fired` | `"trigger.fired"` |
| `filter` | table | no | | `environment` (the Honeycomb environment's name) and `trigger` (the Honeycomb Trigger's id). Every key must match | `{ environment = "production" }` |
| `key` | string | no | the Trigger id | `trigger.id`. The task key is `honeycomb:JwwZxfcP5kH` | `"trigger.id"` |

A `honeycomb` trigger takes no `backfill`: Honeycomb sends only new alerts.

<!-- example: factory/triggers/honeycomb-alerts.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "honeycomb"
on = "trigger.fired"
filter = { environment = "production" }   # the Honeycomb environment's name
repo = "owner/name"            # where its tasks work; without it they block
workflow = "implement"
key = "trigger.id"             # the task key is honeycomb:JwwZxfcP5kH
```

## `figma`

Starts a task when someone comments on a Figma file. Figma pushes the comment to
`POST /hooks/figma`, so the trigger needs [`[webhooks.figma]`](factory.md#webhooksfigma) and a
webhook you create for `FILE_COMMENT`. See [figma.md](../intake/figma.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `file.comment_created` | `"file.comment_created"` |
| `filter` | table | no | | `file` (the file's key). Every key must match | `{ file = "zH44k2FUM629Fa4EMShiHL" }` |
| `key` | string | no | the file key | `file.key`. The task key is `figma:zH44k2FUM629Fa4EMShiHL` | `"file.key"` |

A `figma` trigger takes no `backfill`: Figma sends only new comments.

<!-- example: factory/triggers/figma-comments.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "figma"
on = "file.comment_created"
filter = { file = "zH44k2FUM629Fa4EMShiHL" }   # the file's key
repo = "owner/name"            # where its tasks work; without it they block
workflow = "implement"
key = "file.key"               # the task key is figma:zH44k2FUM629Fa4EMShiHL
```

## `intercom`

Starts a task when a customer starts an Intercom conversation. Intercom pushes it to
`POST /hooks/intercom`, so the trigger needs [`[webhooks.intercom]`](factory.md#webhooksintercom),
whose `workspace` picks the workspace. See [intercom.md](../intake/intercom.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `on` | string | yes | | `conversation.created` | `"conversation.created"` |
| `key` | string | no | the conversation id | `conversation.id`. The task key is `intercom:1295` | `"conversation.id"` |

An `intercom` trigger takes no `filter` and no `backfill`: every conversation of the workspace
matches, and Intercom sends only new ones.

<!-- example: factory/triggers/intercom-conversations.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "intercom"
on = "conversation.created"    # a customer started a conversation
repo = "owner/name"            # where its tasks work; without it they block
workflow = "implement"
key = "conversation.id"        # the task key is intercom:1295
```

## `slack`

Starts a task when someone posts in a channel, outside a thread. The intake reads a bot token from
`SLACK_BOT_TOKEN`. See [slack.md](../intake/slack.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `channel` | string | yes | | The channel id, not its name | `"C0123"` |
| `on` | string | yes | | `message.posted` | `"message.posted"` |
| `filter` | table | no | | `text`: only messages that contain it, ignoring case | `{ text = "@priory" }` |
| `key` | string | no | the message | `message.ts`. The task key is `slack:<channel>:<ts>` | `"message.ts"` |
| `backfill` | boolean | no | `false` | Whether the 50 messages of the first poll start tasks too | `true` |

<!-- example: factory/triggers/support-channel.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "slack"
channel = "C0123"              # the channel id, not its name
on = "message.posted"
filter = { text = "@priory" }  # the message contains it, ignoring case
repo = "owner/name"
workflow = "implement"
key = "message.ts"             # the task key is slack:C0123:<ts>
backfill = false
```

## `schedule`

Starts or wakes a task at a time of day. See [schedule.md](../intake/schedule.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `repo` | string | yes | | The repository its runs work in | `"owner/name"` |
| `at` | `HH:MM` | yes | | The local time of day it fires | `"09:00"` |
| `days` | array of `mon` to `sun` | no | every day | The weekdays it fires on | `["mon", "fri"]` |
| `every` | duration | no | once a day | How often it fires again after `at`, the same day. At least `1m` | `"1h"` |
| `until` | `HH:MM` or duration | no | the end of the day | When its day of firings ends: a time of day, or a duration after `at` | `"18:00"`, `"9h"` |
| `timezone` | string | no | the machine's zone | The IANA zone `at` is read in | `"Europe/Paris"` |
| `key` | string | no | one task per firing | A fixed key: every firing names the same task, `schedule:<key>` | `"support-day"` |
| `prompt` | string | no | | What a firing tells the asleep task it wakes | `"Sweep the support queue."` |

<!-- example: factory/triggers/support-day.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "schedule"
repo = "owner/name"            # the repository its runs work in
at = "09:00"                   # local time of day
days = ["mon", "tue", "wed", "thu", "fri"]   # every day when unset
every = "1h"                   # fire again every hour after at, the same day
until = "18:00"                # a time of day, or a duration after at such as "9h"
timezone = "Europe/Paris"      # the machine's zone when unset
key = "support-day"            # every firing names the same task
prompt = "Sweep the support queue."
workflow = "implement"
```

## Task requests: `triggers/cli.toml` and `hold`

`priory request` starts a task by hand, without a matching trigger (see
[cli.md](../intake/cli.md)). It reads only `repo` from `triggers/cli.toml`, when the request names
none. Like every trigger's `repo`, that repository is polled for GitHub events.

<!-- example: factory/triggers/cli.toml -->
```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "cli"                 # read by priory request
repo = "owner/name"            # the repository when a request names none
workflow = "implement"
```

### `hold`

A trigger can file its tasks held, so each waits for a person or a condition before it starts.
`priory request` ignores the `hold` of `triggers/cli.toml`; `priory request --hold` holds one
request. See Holds in
[runs.md](../daemon/runs.md).

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `reason` | string | yes | | Why the task waits, shown to a person | `"triage first"` |
| `until` | string | no | | An RFC 3339 instant, or a duration such as `2 hours` after the hold | `"2 hours"` |
| `after` | integer, string, or a list of them | no | | Other tasks' ids, until each ends, or Linear issues, until each is Done or Canceled | `[12, "ENG-7"]` |
| `merge` | string, or a list of them | no | | Pull requests, `owner/name#42` or `42` in the task's repository: until each merges | `"owner/name#42"` |
| `if_closed` | `release`, `drop` or `ask` | no | `ask` | What a pull request closed without a merge does. With `merge` only | `"drop"` |
| `blockers` | boolean | no | `false` | Until the task's Linear issue has no open blocker | `true` |

The task is released once every condition is met. With none, a person releases it.

## `intervention` and `lead`

`intervention` sets how much a person steps in on the trigger's tasks. `priory request
--intervention` and `priory add --intervention` set it for one task, and otherwise a CLI request
takes the one in `triggers/cli.toml`. [tools.md](../daemon/tools.md#intervention) describes what
each level does.

| Value | Effect |
| ----- | ------ |
| unset | Every ask goes to a person |
| `plan` | A step that grades the task waits for a person to approve its plan before the task moves on, even for a green grade |
| `hands-off` | The `lead` agent answers the task's asks in the person's place, except an ask whose answer lets an irreversible step run and an ask whose answer runs a command |

`lead` names a `claude-code` agent whose `session` is `shared`, so one conversation answers every
ask. A CLI request's lead is the `lead` of `triggers/cli.toml`.

```toml
source = "github"
repo = "owner/name"
on = "issues.labeled"
filter = { label = "agent" }
workflow = "implement"
intervention = "hands-off"
lead = "lead"                  # agents/lead/agent.md, with session = "shared"
```

## `pre`

`[pre]` names a script of the factory that runs on each event the trigger matches, before the
request is published. What the script does with the request is in
[pre.md](../intake/pre.md). Every source takes it, except `triggers/cli.toml`, where `priory check`
refuses it.

| Key | Type | Required | Default | Meaning | Example |
| --- | ---- | -------- | ------- | ------- | ------- |
| `argv` | list of strings | yes | | The command and its arguments, run with no shell. The first is an executable file inside the factory folder | `["scripts/sentry-pre", "--min-level", "error"]` |
| `timeout` | duration | no | `10s` | How long it may run, at most `60s`, counted from its start | `"10s"` |
| `env` | list of strings | no | `[]` | Variables copied from the process that runs it, beside `PATH`, `LANG`, `HOME`, `PRIORY_FACTORY` and `PRIORY_TRIGGER` | `["AMPLITUDE_READ_TOKEN"]` |
| `on_error` | `pass` or `drop` | no | `pass` | What a script that fails does with the request: `pass` publishes it unchanged, `drop` drops it | `"drop"` |

```toml
#:schema https://raw.githubusercontent.com/maxence-lefebvre/priory/main/docs/schemas/config/trigger.json
source = "sentry"
on = "issue.created"
repo = "owner/name"
filter = { project = "api" }
workflow = "bug-investigation"
key = "issue.id"

[pre]
argv = ["scripts/sentry-pre", "--min-level", "error"]
timeout = "10s"
env = ["AMPLITUDE_READ_TOKEN"]
```

## Errors from `priory check`

| Message | Cause |
| ------- | ----- |
| `` unknown field `<key>`, expected one of ... `` | A key no table above lists |
| `` missing field `workflow` `` | No `workflow` |
| `<n> is not a spend cap: give dollars above zero` | A `caps` value of zero or less, or not finite |
| `` workflow `<name>` does not exist `` | `workflow` names no file under `workflows/` |
| `` band `<band>` is not one of <bands> `` | `band` is not a band of the factory |
| `` lane `<lane>` is not `<team>/<theme>` `` | A malformed `lane` |
| `` `<name>` is also defined in triggers/<name>.toml `` | `<name>.toml` and `<name>.yaml` both exist |
| `` a GitHub trigger needs `repo = "owner/name"` `` | A `github` trigger without a valid `repo`. A `schedule` trigger gets the same with `schedule` |
| `` a GitHub trigger handles `issues.labeled`, `issues.opened`, `pull_request.labeled`, `pull_request.opened` or `pull_request.review_requested`, not `<on>` `` | A bad `on` |
| `` a `pull_request.review_requested` trigger needs `reviewer = "<GitHub login>"` `` | No `reviewer`, or one that is not letters, digits and dashes |
| `` a `pull_request.review_requested` trigger takes no `<key>`: ... `` | `repo`, `filter` or `key` on a review request trigger |
| `` only a GitHub `pull_request.review_requested` trigger names a `reviewer` `` | `reviewer` on another trigger |
| `` `filter` works only with `on = "*.labeled"`, on `label` `` | A GitHub `filter` on another event or key |
| `` a GitHub trigger's `key` is `issue.number` `` | Another `key`. `pull_request.number` for a pull request event |
| `` a `linear.view` trigger, and only it, names a `view` `` | `view` missing on `linear.view`, or set on `linear` |
| `` a `linear` trigger handles `issue.created`, `issue.state_changed`, `comment.created`, not `<on>` `` | A bad `on`. `linear.view` lists `issue.entered`, `issue.left` |
| `` a `linear` trigger filters on `team`, `project`, `label` or `state` `` | Another `filter` key |
| `` a `linear.view` trigger takes no `filter`: its view filters `` | A `filter` on a view trigger |
| `` a Linear trigger's `repo` is `owner/name` `` | A malformed `repo`. Slack gets the same with `a Slack trigger's` |
| `` a Linear trigger's `key` is `issue.identifier` `` | Another `key` |
| `` a Slack trigger handles `message.posted`, not `<on>` `` | A bad `on` |
| `` a Slack trigger needs `channel = "<channel id>"` `` | No `channel` |
| `` a Slack trigger filters on `text` only `` | Another `filter` key |
| `` a Slack trigger names no `view` `` | `view` on a Slack trigger |
| `` a Slack trigger's `key` is `message.ts` `` | Another `key` |
| `` a `cloudwatch` trigger needs `[webhooks.cloudwatch]` in factory.toml: CloudWatch alarm state changes arrive only by webhook `` | A `cloudwatch` trigger in a factory without `[webhooks.cloudwatch]` |
| `` a `cloudwatch` trigger handles `alarm.state_changed`, not `<on>` `` | A bad `on` |
| `` a `cloudwatch` trigger filters on `name` or `state` or `previous_state` `` | Another `filter` key |
| `` a CloudWatch trigger's `repo` is `owner/name` `` | A malformed `repo` |
| `` a CloudWatch trigger's `key` is `alarm.arn` `` | Another `key` |
| `` a `figma` trigger needs `[webhooks.figma]` in factory.toml: Figma comments arrive only by webhook `` | A `figma` trigger in a factory without `[webhooks.figma]` |
| `` a `figma` trigger handles `file.comment_created`, not `<on>` `` | A bad `on` |
| `` a `figma` trigger filters on `file` `` | Another `filter` key |
| `` a Figma trigger's `repo` is `owner/name` `` | A malformed `repo` |
| `` a Figma trigger's `key` is `file.key` `` | Another `key` |
| `` only a GitHub, Linear or Slack trigger has `backfill` `` | `backfill` on a schedule, `cli`, `sentry`, `honeycomb`, `intercom`, `cloudwatch` or `figma` trigger |
| `` only a Slack trigger names a `channel` `` | `channel` on another source |
| `` only a schedule trigger has `<key>` `` | `at`, `days`, `every`, `until`, `timezone` or `prompt` on another source |
| `` a schedule trigger needs `at = "HH:MM"` `` | A schedule without `at` |
| `` `every` is shorter than the intake's 1-minute poll `` | `every` under `1m` |
| `` `until` is before `at`, so it never fires `` | A time-of-day `until` earlier than `at` |
| `"<text>" is not a time of day such as "09:00"` | A bad `at` |
| `"<text>" is not a day: use mon, tue, wed, thu, fri, sat or sun` | A bad entry in `days` |
| `` `days` is empty, so it never fires `` | `days = []` |
| `"<text>" is not a duration such as 90s, 15m or 2h` | A bad `every` |
| `"<text>" is not a time of day such as "18:00" or a duration such as 8h` | A bad `until` |
| `"<text>" is not a time zone such as "Europe/Paris"` | A `timezone` that is not an IANA name |
| `` `intervention = "hands-off"` needs a `lead`: the agent that answers asks in the person's place `` | `hands-off` without `lead` |
| `` lead `<agent>` does not exist `` | `lead` names no file under `agents/` |
| `` lead `<agent>` needs harness = "claude-code" and session = "shared" `` | A `lead` that is a Codex agent or has a `per-task` session |
| `` `[pre]` runs on every source except `triggers/cli.toml`: priory add and priory request never run it `` | A `[pre]` table on the `cli` trigger |
| `` `[pre] argv`: <program> leaves the factory folder `` | An absolute `argv[0]`, one with `..`, or a symbolic link that resolves outside the factory folder |
| `` `[pre] argv`: <program> is not a file in the factory folder `` | `argv[0]` names no file |
| `` `[pre] argv`: <program> is not executable: chmod +x it `` | `argv[0]` is a file without an execute bit |
| `` `[pre] argv` needs a program `` | `argv = []` |
| `` `[pre] timeout` is <n>s: a pre script runs at most 60s `` | A `timeout` over `60s` |
| `` `[pre] env`: `<name>` is not a variable name `` | An `env` entry that is not letters, digits and `_`, starting with a letter or `_` |
| `` unknown variant `<value>`, expected `pass` or `drop` `` | Another `on_error` |
