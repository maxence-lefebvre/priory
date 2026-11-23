# Reload

The daemon and `priory serve` follow edits of the factory's files without a restart. A restart also
stops the running tasks' daemon for a moment, so most edits no longer need one.

## How a reload happens

Every second, the daemon notes the size and modification time of each file the factory loads
from:

- `factory.toml`, `lanes.toml`, `prices.toml` (or their `.yaml` and `.yml` forms),
  `instructions.md` and the guide `writing` names.
- The `.toml`, `.yaml` and `.yml` files directly in `triggers/`, `workflows/`, `views/`,
  `scorers/`, `benchmarks/` and `runners/`.
- Every file under `agents/`, `skills/` and `memories/`.

It skips hidden entries such as `.git`, editor backups such as `foo.toml~` and `#foo.toml#`,
`__pycache__/` and `node_modules/` folders, and does not follow a linked folder. Any other file in
the factory folder, such as one a `deliver` command writes, never reloads it.

When the files changed, the daemon waits one more second. If nothing changed again, it compares
their bytes with the files the live factory was loaded from. An edit that left them the same, such
as a touch or an undo, does nothing. Otherwise the daemon loads the factory and runs every rule of
`priory check`. A file an editor is still writing so never loads half written.

- **The edit passes.** The new factory is live from the next tick, and the server answers from it
  on the next request. The daemon prints `priory daemon: reloaded the factory`.
- **The edit fails.** The daemon keeps the last good factory. It prints each problem once, with
  its file and line, as `priory check` does, then `priory daemon: kept the last good factory`. It
  prints again only after the next edit.

`priory daemon --view-only` and `priory serve` reload the same way. `priory serve` prints
`priory serve:` in place of `priory daemon:`.

## What a reload changes

A reload changes what the daemon reads on each tick: workflows, agents, `lanes.toml`, `prices.toml`,
bands, `[queue]`, `[caps]`, `[links]`, `[request]`, views, memories and the rest. After a reload, the
daemon records the new bands, fairness and irreversible approvals, as it does when it starts. A
reload that gives a workflow its first `sandbox` checks that this runner can start bubblewrap, as a
start does (see [sandboxes.md](sandboxes.md)).

Some keys set up a process, a connection or a secret when the daemon starts. A reload reads them,
but they wait for a restart:

| Key | What it set up |
| --- | -------------- |
| `name` | The `source` of every trace record and the ids of a task's spans |
| [`[serve]`](../reference/factory.md#serve) | The listen address, the token, the extension origins and the terminal |
| [`[webhooks.*]`](../reference/factory.md#webhooksgithub) | Each webhook's secret and route |
| [`[log]`](../reference/factory.md#log) | The event log's connection |
| [`[telemetry]`](../reference/factory.md#telemetry) | The OTLP export and its headers |
| [`[notify]`](../reference/factory.md#notify) | Desktop notifications |
| [`[search]`](../reference/factory.md#search) | The embedder |
| [`[kit]`](../reference/factory.md#kit), [`[memories]`](../reference/factory.md#memories) | The repositories each runner fetches |
| `command_timeout` | The bound on every git, gh and tmux command |
| [`[sweep]`](../reference/factory.md#sweep) `every` | How often the sweep runs |
| `runners/` | This machine's runner and its share |

When an edit changes one of them, the daemon prints `priory daemon: <key> changed: restart priory
daemon to apply it`. The rest of the edit is live.

## Running tasks

A task holds its workflow by name and reads its step from the live factory each time it moves. No
task keeps the workflow it started with. So a reload acts as a restart does:

- A live run keeps the prompt and the agent it started with.
- The task's next move follows the edited workflow. A task asleep at an ask takes the edited
  step's `next` when the ask is answered.

A task would strand if its workflow or its step vanished. So the daemon holds an edit that removes
the workflow or the step of a task that has not ended. It keeps the last good factory and prints
one line per task, once:

```text
workflows/implement.toml: task 12 is at step approve, which this edit removes: end or move task 12, or keep the step
```

Each second, the daemon checks the held edit again. It applies the edit, with no new edit of the
files, once no task is at a workflow or step it removes: when the task moves on, or after `priory
cancel` ends it. A cancelled task can start again at a step the edit keeps, with `priory revive
TASK --step STEP`. A new edit replaces the held one, and an edit back to the live files drops it.

## Trace records

Each record carries the factory's revision, the last commit that touched its folder. The daemon
also watches the git reflog of the factory's `HEAD`. A commit or a checkout after a reload updates
the revision of the next records, with no reload.

Each reload appends a `factory.config.reloaded` record to the trace, and each refused reload a
`factory.config.refused` record (see [the trace schema](../trace-schema.md#factoryconfigreloaded-and-factoryconfigrefused)).
`priory trace` prints them, and the OpenTelemetry export sends each as a span (see
[trace.md](trace.md#opentelemetry-export)). `priory serve` reads the state file only, so it records
nothing.

## The intake

The intake reads `triggers/` once, when it starts, and binds each trigger to its source. Restart
`priory intake` after you edit a trigger.

## Limits

- An edit between the moment the daemon loads the factory and the moment it starts watching is
  missed until the next edit. That window is a few milliseconds at start.
- A script `factory.toml` names, such as `[asks] deliver`, runs from disk each time, so it is not
  watched and its edit needs no reload.
- An edit that keeps a file's size and modification time, on a file system that stores times in
  whole seconds, is missed until the next edit.
- The guard checks this runner's tasks only. Another runner checks its own when it reloads.
