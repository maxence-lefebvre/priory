# Sweep

The daemon removes worktrees of ended tasks. It sweeps when it starts, when a stall short of disk
starts, and every 10 minutes after, or every `[sweep] every`, not on every tick. A sweep fetches
`origin` into each clone with a worktree, refreshes `origin/HEAD`, then checks each folder
`worktrees/<owner>/<name>/<id>`, or `<runner>-<id>` with several runners. A clone whose fetch
failed keeps its worktrees, and the sweep fetches it again an hour later (see A clone that fails to
fetch).

It removes a folder and its branch `priory/<id>` only when all checks hold:

- Task `<id>` is `done`, `failed` or `dropped`, and its recorded `worktree` is this folder.
- It is a worktree of Priory's clone `repos/<owner>/<name>`, on branch `priory/<id>`.
- Nothing is uncommitted or untracked, whatever `status.showUntrackedFiles` says.
- No process has its working directory inside it (read from `/proc`). Priory's own git commands,
  such as another sweep's checks, do not count. Each carries `PRIORY_GIT=<pid>` in its environment,
  the pid of the Priory process that runs it, and so do its children. A process counts as
  Priory's only when that pid runs Priory's program, is its ancestor, and that ancestor's child on
  the way carries the same value. Any process can set the variable, so the value alone hides none.
  On macOS, `lsof` lists the working directories. No process's environment can be read there, so
  Priory's own git counts too, and the worktree waits for the next sweep. A worktree also stays when
  `lsof` fails, runs past its deadline or lists nothing, and when its path holds a backslash or is
  not plain ASCII, which `lsof` escapes.
- Its task has no live run and no memory grant, waiting or held (see Memory gate).
- No mount is at or under it, and it holds no repository of its own (see What a removal never takes).
- The branch tip is in `origin/HEAD`, or is the head commit of a merged pull request for the
  branch (a `github.pull_request.closed` event with `merged` true). Otherwise it stays as
  `unpushed commits` when a commit is on no remote branch, else as `not merged`.

A `done` task whose branch did not merge loses only its worktree when every commit of the branch
is on a remote branch, any remote branch. A task that works on a person's pull request pushes to
that person's branch, so its own branch never merges. A commit also counts as pushed when a remote
branch holds a commit with the same patch (`git patch-id --verbatim`), as a rebase of that branch
leaves. When more than 2000 remote commits could match, the sweep compares none and keeps the
worktree. It also keeps it when the patches to compare take more than 64 MB. Each answer is kept until
`HEAD` or a remote ref changes, and a sweep runs at most 2 new comparisons, the oldest first. The
others stay as `its patch check waits for the next sweep`. A merge commit or an empty commit has no patch, so it counts only when it is on a remote
branch itself.

The worktree goes at the next sweep, even while the task's pull request is open. The sweep keeps
`priory/<id>`, since it fetches remote refs without `--prune`. A review round that reopens the task
adds the worktree back from that branch at the same path and runs the repository's `resume` recipe,
not its `setup` recipe (see [runs.md](runs.md#recipes)). The worktree's ignored files, such as
`target/`, and the reflog of its `HEAD` go with it, so the first build after a reopen relies on a
compiler cache, such as one the `resume` recipe points at, to stay cheap. No later sweep deletes
that branch once the pull request merges, since the sweep walks folders. A `failed` or `dropped`
task keeps its worktree as `not merged`, since a person revives it in place.

When the task's repository has a [`teardown` recipe](runs.md#recipes), the sweep runs it in the
worktree first, once all checks hold, and removes the worktree at the sweep after it succeeds. The
daemon sweeps again as soon as a teardown ends. A failed teardown keeps the worktree. The desk says
`teardown failed: <status>. Its output ended with: <lines>`, and no sweep runs it again until the
factory's teardown changes. While it runs, the desk says `teardown runs` or
`a process runs in it`.

The sweep runs `git worktree remove`, then `git branch -d`, both without force, so git refuses what
the checks missed. A squash-merged branch is not in the default branch, so `git branch -d` refuses
it. The worktree goes, the branch stays, and no sweep looks again. A failed worktree removal is
logged and retried at the next sweep. A git command that cannot run keeps the worktree as
`cannot check: <error>`.

The sweep waits at most 30 seconds for one git command, or `command_timeout` when that is shorter
(see [home.md](home.md#command-timeout)), except for `git worktree remove` itself. A command that
runs past it keeps its worktree, and the sweep asks git nothing about the worktrees after it, which
stay as `its checks wait for the next sweep`. After a fetch that times out, the other clones wait for
the next sweep too. Each of the fetches, the checks and the removals stops starting new work after
30 seconds, and the next sweep takes what waited longest first. A walk of a worktree's files stops after 10 seconds and keeps the worktree, with
`<path>: its files take over 10s to check`.

Before `git worktree remove`, the sweep makes every folder under the worktree writable by its
owner. A read-only folder, such as one a test run sealed under an ignored `target/`, would
otherwise stop git after it deleted the tracked files and the worktree's entry in the clone. When
a task's folder is left that way, with a `.git` file naming a gone entry of Priory's clone, the
task's next run moves the folder to `PRIORY_HOME/reclaim/`, deletes it on a thread, and adds the worktree back from its branch.

`priory desk` lists each kept folder with its reason: `no task <id>`, `kept while task <id> is <state>`,
`not a worktree of <clone>`, `not recorded as the worktree of task <id>`, `not on branch priory/<id>`,
`uncommitted changes`, `untracked files`, `a process runs in it`, `unpushed commits`, `not merged`,
`its patch check waits for the next sweep`, `its checks wait for the next sweep`,
`<path>: its files take over 10s to check`, `teardown runs`, `teardown failed: ...`, `<path> is a mount`, `<path> holds a git repository` or
`task <id> has a live run or a memory grant, or left its state`.
It runs no git: it reads the reasons from `sweep/sizes.json`, which each sweep writes. Before the
first sweep, it lists only the reasons it reads from the database. Each reason ends with the size of
the folder in binary units, such as `not merged, 7.4 GiB`. After each periodic sweep, a thread of its
own walks the kept folders and records their sizes, since that walk takes seconds. A folder the walk
has not measured yet shows its reason alone.

## What a removal never takes

`git worktree remove` deletes the files of a folder mounted under the worktree, then fails. So the
sweep and a retirement check, after any teardown and before git runs. Mounts are checked again
before and after folders are made writable, right before the delete:

- No mount is at or under the worktree, as `/proc/self/mountinfo` shows, or `mount` on macOS. A table that cannot be read
  keeps the worktree.
- No ignored file or folder is part of a repository: a `.git` entry, or a `HEAD` beside `objects` and
  `refs`. Nor is a `.git` in a folder of tracked files. Its commits can be nowhere else, and the reclaim keeps it too. A repository in the `tmp`
  folder of a cargo target does not count. A tracked folder that looks like a repository does not count
  either, since the branch holds it.
- The checks listed above still hold: the task's state, its runs and grants, uncommitted and untracked
  files, and processes.

A worktree that fails one stays, with the reason on the desk or the task, and the next sweep tries
again. Unmount the mount (`findmnt -R <path>`) or move the repository out.

Only the tick starts runs, and the checks and the removal are one step of it, so no run starts between
them. A process that already runs elsewhere can still mount a folder in the worktree during the
removal. A mount in a private namespace, such as a jail's, does not show in the daemon's table, and
git does not enter it.

A folder without its entry in the clone (see above) waits the same way: its task stays `queued` or
`ready` with `waits to add its worktree back: <path> is a mount`, and each tick tries again. One that
holds a repository, ignored or not, moves to `PRIORY_HOME/trash/` instead of going.

The checkouts of a `record` command and of a `command` scorer also stay when a mount is in them. They
are logged and not tried again.

## Reclaiming build output

While free disk is under `[sweep] reclaim`, by default 20 % of the disk and at most 40 GB, each sweep starts a walk that lists the
build output of idle tasks. The daemon then deletes it one folder at a time, at most one per tick. A threshold under
the room's disk floor (see Room) acts as the floor. So build output goes before the floor stops new
runs, and before a whole worktree retires.

A task is idle when it is `done`, `dropped`, `held` or `asleep`, has no live run, and holds no
memory grant, waiting or held (see Memory gate). A build that a run wrapped in `priory memory
acquire` keeps its grant after the run sleeps, so its output stays until it ends.

`[repos."owner/name"] build` names the build folders, relative to the worktree. Without it, a
worktree with a `Cargo.toml` at its root has `target`, and other worktrees have none. `build = []`
opts a repository out: none of its worktrees loses build output. A worktree keeps all its build
output when any retirement check fails: it is not Priory's worktree, it is off its branch, a process runs in it, or it has an uncommitted change or an untracked file. Untracked
files inside the build folders do not count; a change to a tracked file there does. A build folder
that holds a file git tracks, such as a committed bundle, stays. A build folder that is a symlink,
or that a symlink leads to, is never followed, and a symlink inside one goes without what it points
to.

These build folders stay too. One of them does not keep the worktree's other build folders, while a
sweep or a retirement keeps the whole worktree for a mount or a repository (see What a removal never
takes):

- One with a mount at or under it, as `/proc/self/mountinfo` shows.
- One inside a submodule or a nested repository.
- One that holds a `.git` entry. A `.git` in the `tmp` folder of a cargo target does not count: tests
  keep scratch files there. Cargo's `CACHEDIR.TAG` marks such a target.
- One that is or holds a bare repository: a `HEAD` beside `objects` and `refs`. The same `tmp` does not count.

A thread lists and sizes each folder as `du` does, off the daemon's tick. Then each tick
takes one folder, the largest first, and on equal sizes the task idle longest first. The tick checks
again that the task is idle and that every check above still passes, then moves the folder to
`PRIORY_HOME/reclaim/`, a rename that takes no longer for a large folder. A thread deletes it there.
Only the tick starts runs, so none starts between those checks and the move, and a run that starts
during the delete finds no build folder and makes a new one. No other folder goes until the delete
ends. The tick measures the disk before each folder and stops once free disk is back at the
threshold. A folder that a test made read-only is made writable, then deleted. The worktree,
its branch and its session stay, so the task's next run rebuilds what it needs.

Each deletion, once it ends, appends a `factory.worktree.reclaimed` record with the task, the folder
and its bytes (see [trace-schema.md](../trace-schema.md#factoryworktreereclaimed)). `priory desk` and the web
desk show `reclaimed 14.2 GB of build output from 3 folders in the last 24 hours, last at
2026-11-23T19:30:02Z` while any happened in the last 24 hours.

The thread looks for mounts and repositories once more before it deletes a moved folder. If it finds
one, it moves the folder back. If a new build folder took its place, the moved one stays in
`PRIORY_HOME/reclaim/`.

A daemon that stops during a delete leaves files in `PRIORY_HOME/reclaim/`, and the next sweep
deletes them. It deletes only folders with a name it gave, a task's id and 18 or more digits of
nanoseconds, such as `304-1763926202412000000`. When `reclaim` is not a real folder, such as a symlink, nothing moves and
nothing goes.

The daemon logs a folder it cannot delete once, tries it again each sweep, and goes on with other
build output. To remove one by hand, read the path in the log. Look for a mount (`findmnt -R <path>`), a
repository, an immutable file (`lsattr -R <path>`) or a file of another user. Unmount the mount and
move the repository out. Then run `rm -rf <path>`, with `sudo chattr -R -i <path>` first for immutable
files, and `sudo` for another user's.

When a worktree is on another filesystem than `PRIORY_HOME`, the move fails. The daemon logs it, keeps
the folder with its mode, and does not try that folder again until it restarts.

## Retiring asleep tasks

After each sweep, once the reclaim has nothing left to take, while free disk is at or under the room's disk floor (see Room), the daemon
retires asleep tasks, longest asleep first, until the floor is met again. While a retirement waits,
no sweep starts a walk or a delete of leftover files, so sweeps that come often cannot postpone it
for ever. A delete that runs for 10 minutes no longer holds it up: the daemon logs the folder once
and retires. Nothing else is reclaimed until that delete ends. Retiring runs
`git worktree remove` without force. The worktree goes, while the branch `priory/<id>` and the
task's sessions stay. A task retires only when its worktree is Priory's, on its branch, has nothing
uncommitted or untracked, no process runs in it, its task has no memory grant, and nothing in it is
what a removal never takes. The daemon marks the task before removing the
worktree, so a crash in between leaves a mark on an existing folder, which the next sweep checks
again.

Its commits need not be pushed: the branch stays in Priory's clone on the same disk, and no sweep
deletes an active task's branch. Requiring a push would keep most asleep tasks, since an agent
pushes only at `open-pr`, and Priory does not take the irreversible step of pushing on its own.

With a `teardown` recipe, a task's teardown runs before its retirement, one task at a time. The
retirement waits for it, then measures the disk again before it takes the next task. A failed
teardown keeps the worktree, with the reason `cannot retire: teardown failed: ...`, and the next
task is tried. When the task resurrects, its `resume` recipe runs before its next run (see
[runs.md](runs.md#recipes)).

A retired task stays `asleep` with its `retired` time set. It keeps every wait, ask and event, so
routing, holds and waits treat it as any asleep task. R4.1's states stay as they are. When its wait
fires or its ask is answered, it becomes `ready`, as R3.2 says. Its next run re-adds the worktree
from the branch at the same path, so `claude --resume` finds the session stored for that folder.
Starting the run, or an `open-pr` or `run` action that adds the worktree back, clears `retired`. An answer approving
a step, and the action it approves, read the branch's commit from Priory's clone while the task is
retired.

A code step skips the room check, except `open-pr` or `run` on a retired task: `open-pr` pushes from
the worktree and `run` works in it, so each must add it back. Other actions, such as `open-issue`, run without it. While free disk is at or
under the floor, the task stays `ready` with the reason `waits for room to add back its worktree`,
with `: cannot measure it: <error>` added when the room cannot be measured. The daemon goes on to the next task
at a code step, so the code steps behind it still run.

A task that cannot retire gets the reason `cannot retire: <why>`, with the same reasons as the sweep,
such as `untracked files`. The daemon sets and logs it only when it changes, so it is reported once.
A wake clears it.

Retiring runs only at a sweep: when the daemon starts, when a disk stall starts, and every `[sweep]
every`. A disk that runs short with nothing waiting for room waits for the next sweep.

## A clone that fails to fetch

A repository whose remote is gone, such as a test repository left in Priory home, fails every
fetch. On its first failure, the daemon logs `fetching <owner>/<name>: <error>` once, and records
the failure in `fetches/<owner>/<name>.json` under Priory home. The sweep then skips that clone,
and keeps its worktrees, until an hour has passed since its last try. A fetch that works again
removes the record and logs `fetching <owner>/<name> works again`.

While the repository still has worktrees, the desk's banner names it, since when it fails, git's
error, and the two folders to remove when its remote is gone for good (see
[desk.md](desk.md#intake-banner)). A repository with no worktree left is no longer fetched, and the
sweep forgets its record.

## Sweep scripts and the sweeper

`factory.toml` can add a `[sweep]` table:

```toml
[sweep]
every = "10m"                 # the built-in worktree sweep's cadence
scripts = [
  { run = "docker builder prune -f", when = "disk", timeout = "5m" },
  { run = "nh clean all --keep 3", every = "4h" },
]
after = "30m"                 # how long the queue stays stalled after its scripts
agent = "sweeper"             # no agent, no sweeper
repo = "maxence-lefebvre/priory"  # where the sweeper works and files its issue; this is the default
```

When a stall starts, the daemon runs each script whose `when` names its resource: `memory`, `disk`
or `cpu`. Each runs once per stall, through `sh -c` in `PRIORY_HOME`, in its own process group. A
script past its `timeout`, 5 minutes by default, is killed with its children. The daemon keeps the
last 2000 bytes of output and the status, `exit <code>` or `timed out after <n>s`. The desk lists
each script of the open stall as a `sweep` row. Children a script leaves in the background are
killed when it ends. A script that fails or cannot start never stops the daemon. A restarted daemon
kills its predecessor's running scripts, after checking each process group's start time in `/proc`
so a reused pid is left alone. It records them as `interrupted by a daemon restart`, with the tail
of their output.

A script with `every` also runs on a schedule: once that long has passed since it last started,
for a stall or on its schedule. A script needs `when`, `every` or both. A scheduled run uses the
same shell, process group, timeout, output tail and restart handling as a stall's run. The last
start is kept in the state database, so a restarted daemon neither reruns a script early nor
postpones it. A script with no run on record, such as a new one, runs at the daemon's next tick.
A script never runs twice at once: its schedule waits while it runs, and a stall that opens during
a scheduled run of a matching script takes that run as its own instead of starting another. From then on it is a run of the stall: it
keeps its start, the stall waits for it to end, and the desk lists it only while the stall is open,
even if it fails. The desk lists a scheduled script as a `sweep` row with no task while it runs. A
failed run stays there until the script runs again. A clean scheduled run leaves no row. `every` is
at least 10 seconds and at most 36500 days.

At each tick, the daemon forgets a scheduled run that ended once its script has a later run, so the
state database keeps the latest one per script. It also forgets a failed last run once no entry of
`[sweep] scripts` with `every` has that `run` text, so a fixed or removed command leaves the desk.
It keeps a clean last run, so a script you remove and add back keeps its schedule. Runs of a stall stay.

After the stall's scripts end, or at its start when none matches, the stall waits `after`, 30
minutes by default. If it still holds and `agent` names an agent, the daemon creates a task with the
key `priory:sweep` and the trigger `priory/sweep` on the `sweep` workflow, in the highest band and
the lane `priory/sweep`. A stall gets at most one sweeper. A key has at most one active task, so a
new stall waits for an earlier sweeper to end before it gets its own. A sweeper task skips the room
check, since it frees the room the queue waits for. Its source event, `priory.queue.stalled`,
carries the stall, `PRIORY_HOME` and each script's command, status and output, so its first prompt
names them.

Unless the factory defines its own `sweep` workflow, the sweeper runs this built-in one:

1. `clean`, a model step with the sweeper agent. It removes only what Priory or the scripts manage,
   never a worktree with unpushed, uncommitted or untracked work. It is `irreversible` and
   `autonomous`: it deletes files, and waiting for a person would keep the queue stalled. It ends
   `cleaned`, with a summary whose first line titles an issue, or `nothing`, which ends the task.
2. `approve-issue`, a human step with the options `open` and `skip`, which approves `open-issue`.
3. `open-issue`, a code step that runs `gh issue create -R <repo>` with the latest result's summary
   and evidence, as `open-pr` does. The body ends with `Filed by Priory for event <id>`, the
   sweeper's source event. Before creating one, it looks for an issue in any state whose body holds
   that line, which a crash after creation leaves, and reuses it. It is irreversible and needs that
   approval, since it posts an agent's text to a repository other people read. A sweeper runs
   rarely, so the ask costs little.

`priory check` refuses a `[sweep]` field it does not know, a malformed duration or resource, a
script with neither `when` nor `every`, a script's `every` under 10 seconds or over 36500 days, an
`agent` the factory lacks, and a `repo` that is not `owner/name`, with its line.

Known limits:

- The sweeper's worktree is a clone of `repo`, which takes some disk on a disk stall.
- What the sweeper may remove is set by its prompt, not enforced on paths.
- A script's log file grows until it ends; only its tail is read.
- A crash between starting a script and recording its pid leaves that script unbounded.
- A restarted daemon cannot kill background children of a lost script whose shell already exited.
  It cannot tell that group from a process that reused the shell's pid.
- A stall that alternates between two resources starts a new stall, and new scripts, at each change.
- A script's history is keyed by its `run` text: editing the command runs it at the next tick, and
  two entries with the same `run` share one schedule.
- A failed scheduled script you remove and add back runs at the next tick, since its run is
  forgotten.
- A scheduled run that a daemon restart interrupts counts as a run: its script runs again `every`
  after the interrupted run started, and the desk lists it as failed until then.
- Schedules read the wall clock: setting it back delays every script until the clock passes its
  last start by `every` again.
