# Relations

A relation records how one task relates to another. `priory tasks --graph` draws the tasks a
relation links, and the task JSON lists them as `relations`. A relation changes no routing: it
tells a person, or an agent that reads the task, where the work came from.

## Kinds

Each relation reads `TASK KIND OTHER`, such as `5 fixes 3`.

| Kind | `TASK` ... `OTHER` |
| ---- | ------------------ |
| `builds-on` | builds on the other's work |
| `fixes` | fixes what the other merged |
| `takes-up` | takes up the other after it was dropped or failed |
| `relates` | relates to the other in another way. It has no direction: `5 relates 3` and `3 relates 5` are one relation |

A task never relates to itself, and a pair holds each kind once. Priory stores who recorded each
relation, as `by`, and when, as `created`.

## Recording a relation

| Way | Records | `by` |
| --- | ------- | ---- |
| `priory relate TASK KIND OTHER` | that relation | `$USER`, or `person` when it is unset |
| The `relate(kind, task)` tool | that the caller's own task relates to `task` | `agent` |
| `priory request` or `priory add` `--relates KIND:TASK` | that the new task relates to `TASK`. The flag repeats | `request` |
| A new task on the key of a dropped or failed task | that the new task `takes-up` the latest such task | `priory` |
| A new task whose request links a merged pull request | that the new task `fixes` each other task that owns it | `priory` |

`priory relate` writes the state file directly, as `priory subscribe` does. It exits 1 with one of
these errors, and records nothing:

| Error | Cause |
| ----- | ----- |
| `` `<kind>` is not a relation: name one of builds-on, fixes, takes-up, relates `` | An unknown kind |
| `no task <id>` | Either task does not exist |
| `a task does not relate to itself` | `TASK` and `OTHER` are the same |

`priory relate TASK KIND OTHER --remove` removes the relation, or fails with
`task <id> has no <kind> relation to task <other>`.

The tool takes `kind` and `task`, the other task's id, and refuses any other field. Its task is
always the caller's, so an agent records relations of its own task only. The daemon answers it,
like every other tool. It does not end the turn.

A `--relates` value that is not `KIND:TASK` with a known kind is a usage error. `priory request` and
`priory add` read the state file, which `PRIORY_DB` names, and refuse a `--relates` task it does not
hold with `no task <id>`, before they publish anything. The web view's Add control gets the same
refusal from the daemon. The daemon still skips such a relation in a request that reaches it.

### The two automatic relations

The daemon records both when it creates the task, in the same transaction:

- `takes-up`: the new task's key equals that of a dropped or failed task. A key frees up when its
  task ends, so a new request on it starts a new task. `priory revive` brings back the same task
  instead, so it records nothing.
- `fixes`: the new task's request text links a pull request, as
  `https://github.com/owner/name/pull/7`, that the inbox holds a merged close of. Each other task
  that owns it gets the relation: as its key, in its stack, or through a wait. The request
  text is the source event's `text`, `body`, `description` or `prompt`, so an issue body counts
  too. An issue URL counts for nothing, nor does a pull request that is still open or closed
  unmerged. A `fixes` relation does not say the fixed work was wrong: `priory report risk` lists
  it as a candidate, and a person decides with `priory label` (see
  [trace.md](trace.md#risk)).

## `priory tasks --graph`

```sh
priory tasks --graph [TASK] [--mermaid]
```

It prints the tasks related to `TASK` as an indented tree. Each line is the task's id, key and
state, then the relation that reached it. Without `TASK`, it prints every related task, one tree
per group of linked tasks, each from its lowest id:

```text
1 cli:login done
  2 cli:login-ui running (2 builds-on 1)
    3 cli:login-ui-fix queued (3 fixes 2)
  4 cli:docs done (4 relates 1)
```

A tree shows each task once, so a relation between two tasks it already shows, such as one that
closes a loop, is left out. `--mermaid` prints the same tasks as a Mermaid flowchart, in the style
of `priory draw`, with every relation between them as a link:

```text
flowchart TD
    t1["1 cli:login done"]
    t2["2 cli:login-ui running"]
    t2 -->|"builds-on"| t1
```

`priory tasks --json`, `priory task ID --json` and `/api/tasks/ID` list each task's relations, on
either side, as `relations` (see [serve.md](../reference/serve.md#tasks)).
