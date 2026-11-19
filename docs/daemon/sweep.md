# Sweep

The daemon removes the worktrees of ended tasks. It sweeps when it starts, when a stall short of
disk starts, and every 10 minutes after, or every `[sweep] every`, not on every tick. A sweep fetches `origin` into each clone with a worktree and refreshes
`origin/HEAD`, then looks at each folder `worktrees/<owner>/<name>/<id>`. A clone whose fetch
failed is skipped until the next sweep.

It removes a folder, and its branch `priory/<id>`, only when every check holds:

- Task `<id>` is `done`, `failed` or `dropped`, and its recorded `worktree` is this folder.
- It is a worktree of Priory's clone `repos/<owner>/<name>`, on branch `priory/<id>`.
- Nothing is uncommitted or untracked, whatever `status.showUntrackedFiles` says.
- No process has its working directory inside it (read from `/proc`).
- The branch tip is in `origin/HEAD`, or it is the head commit of a merged pull request for the
  branch (a `github.pull_request.closed` event with `merged` true). Otherwise it stays as
  `unpushed commits` when a commit is on no remote branch, else as `not merged`.

The sweep runs `git worktree remove`, then `git branch -d`, both without force, so git refuses
what the checks missed. A squash-merged branch is not in the default branch, so `git branch -d`
refuses it: the worktree goes and the branch stays, and no sweep looks at that branch again. A
failed worktree removal is logged and retried at the next sweep. A git command that cannot run
keeps the worktree, as `cannot check: <error>`.

`priory desk` runs the same checks without fetching or removing, and lists every folder that stays
with its reason: `no task <id>`, `task <id> still <state>`, `not a worktree of <clone>`,
`not recorded as the worktree of task <id>`, `not on branch priory/<id>`, `uncommitted changes`, `untracked files`, `a process runs in it`,
`unpushed commits` or `not merged`. It reads the refs of the last fetch.

## Retiring asleep tasks

After each sweep, when free disk is at or under the room's disk floor (see Room), the daemon retires
asleep tasks, the longest asleep first, until the floor is met again. Retiring runs
`git worktree remove` without force: the worktree goes, and the branch `priory/<id>` and the task's
sessions stay. A task retires only when its worktree is Priory's, on its branch, has nothing
uncommitted or untracked, and no process runs in it. The daemon marks the task before it removes the worktree, so a
crash in between leaves a mark on a folder that still exists, which the next sweep checks again.

Its commits need not be pushed: the branch stays in Priory's clone, on the same disk, and no sweep
deletes the branch of an active task. Requiring a push would keep most asleep tasks, since an agent
pushes only at `open-pr`, and pushing is an irreversible step Priory does not take on its own.

A retired task stays `asleep`, with its `retired` time set. It keeps every wait, ask and event, so
routing, holds and waits treat it as any asleep task, and R4.1's states stay as they are. When its
wait fires or its ask is answered, it becomes `ready`, as R3.2 says. Its next run adds the worktree
back from the branch, at the same path, so `claude --resume` finds the session it stored for that
folder. Starting the run, or a code step's action, clears `retired`. An answer that approves a step
while the task is retired reads the branch's commit from Priory's clone.

A code step skips the room check, except when it must add a worktree back: while free disk is at or
under the floor, the task stays `ready` with the reason `waits for room to add back its worktree`,
or `waits for room: <error>` when the disk cannot be measured,
and the code steps behind it wait too.

A task that cannot retire gets the reason `cannot retire: <why>`, with the same reasons as the
sweep's, such as `untracked files`. The daemon sets it, and logs it, only when it changes, so it is
reported once. A wake clears it.

Retiring runs only at a sweep: when the daemon starts, when a disk stall starts, and every `[sweep]
every`. A disk that runs short with nothing waiting for room waits for the next sweep.

## Sweep scripts and the sweeper

`factory.toml` can add a `[sweep]` table:

```toml
[sweep]
every = "10m"                 # the built-in worktree sweep's cadence
scripts = [
  { run = "docker builder prune -f", when = "disk", timeout = "5m" },
]
after = "30m"                 # how long the queue stays stalled after its scripts
agent = "sweeper"             # no agent, no sweeper
repo = "maxence-lefebvre/priory"  # where the sweeper works and files its issue; this is the default
```

When a stall starts, the daemon runs every script whose `when` names its resource: `memory`, `disk`
or `cpu`. Each runs once per stall, through `sh -c` in `PRIORY_HOME`, in its own process group. A
script past its `timeout`, 5 minutes by default, is killed with its children. The daemon keeps the
last 2000 bytes of its output and its status, `exit <code>` or `timed out after <n>s`, and the desk
lists each script of the open stall as a `sweep` row. When a script ends, children it left in the
background are killed too. A script that fails or cannot start never stops the daemon. A restarted
daemon kills the scripts its predecessor left running, after checking each process group's start
time in `/proc` so that a reused pid is left alone, and records them as `interrupted by a daemon
restart` with the tail of their output.

Once the stall's scripts have ended, or at its start when none matches, the stall waits `after`, 30
minutes by default. If it still holds and `agent` names an agent, the daemon creates a task with the
key `priory:sweep` and the trigger `priory/sweep` on the `sweep` workflow, in the highest band and
the lane `priory/sweep`. A stall gets at most one sweeper. A key has at most one active task, so while
the sweeper of an earlier stall is still active, a new stall waits for it to end before it gets its
own. A sweeper task skips the room check: it frees the room the queue waits for.
Its source event, `priory.queue.stalled`, carries the stall, `PRIORY_HOME` and each script's
command, status and output, so its first prompt names them.

Unless the factory defines its own `sweep` workflow, the sweeper runs this built-in one:

1. `clean`, a model step with the sweeper agent. It removes only what Priory or the scripts manage
   and never a worktree with unpushed, uncommitted or untracked work. It is `irreversible` and
   `autonomous`: it deletes files, and waiting for a person would keep the queue stalled. It ends
   `cleaned`, with a summary whose first line titles an issue, or `nothing`, which ends the task.
2. `approve-issue`, a human step with the options `open` and `skip`, which approves `open-issue`.
3. `open-issue`, a code step that runs `gh issue create -R <repo>` with the latest result's summary
   and evidence, as `open-pr` does. The body ends with `Filed by Priory for event <id>`, the sweeper's
   source event. Before it creates one, the step looks for an issue in any state whose body holds
   that line, which a crash after its creation leaves, and reuses it. It is irreversible and needs that approval: it posts an agent's
   text to a repository other people read, and a sweeper runs rarely, so the ask costs little.

`priory check` refuses a `[sweep]` field it does not know, a malformed duration or resource, and an
`agent` the factory lacks, and a `repo` that is not `owner/name`, with its line.

Known limits:

- The sweeper's worktree is a clone of `repo`, which takes some disk on a disk stall.
- What the sweeper may remove is set by its prompt, not enforced on paths.
- A script's log file grows until it ends; only its tail is read.
- A crash between starting a script and recording its pid leaves that script unbounded.
- A restarted daemon cannot kill the background children of a lost script whose shell already
  exited, since it cannot tell that group from a process that reused the shell's pid.
- A stall that alternates between two resources starts a new stall, and new scripts, at each change.
