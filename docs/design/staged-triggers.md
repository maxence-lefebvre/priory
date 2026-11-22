# Staged triggers

`priory check` proves who approves which irreversible step, and fake agents test the workflow's
graph. Neither shows what a new trigger does with a real model on a real event. This note keeps
the plan for a trial run that writes nothing, and for caps on what a step writes per task. It comes
from the second scout report of other agent factories (idea 5). GitHub Agentic Workflows inspired
it: `safe-outputs.staged` previews each output with no API call, and a `max` caps each output per
run. Antigravity starts new automations paused and records the permissions a run needed.

## The first slice: a trigger's tasks stop before writing

Built. See [Staged tasks in tools.md](../daemon/tools.md#staged-tasks).

1. A trigger key `staged`, a boolean. Without it, nothing changes.
2. A staged task's model steps run. At an irreversible code step, once its approval holds, the
   daemon records an action with status `staged` whose stdout says what it would have done: the
   pull requests with their branches, title and body, the issue, or the command. It runs nothing,
   pushes nothing and blocks the task, so the desk shows it.
3. At an irreversible model step, a staged task starts no run and blocks.
4. The daemon reads `staged` from the trigger file, so removing it and resuming a blocked task runs
   the step for real. No schema change.

## Later slices

1. `priory trigger try NAME`: poll the trigger once and run one task from its next matching event,
   staged, whatever the trigger file says. It needs a task request field for `staged` and a
   migration.
2. Run an irreversible model step staged: the guard denies each irreversible call and records it as
   what the step would have done, so the agent's comments and labels show in the preview.
3. The trial's report: the tools, guards and permissions the task's runs used, from the trace and
   the guard's records, on the desk next to the preview.
4. Write caps per step and per task, such as one pull request and three comments, beside the
   workflow's `max_prs`. The guard counts each irreversible call and denies the one past the cap.
5. The preview on the web and Herdr Desks, from a design a person reviewed.

## Open questions

1. **Should a staged step need its approval?** It writes nothing, so it could skip it. The first
   slice checks it, so the trial walks the same path a real task does. Recommendation: keep it.
2. **Should a staged task stay staged when the trigger changes?** Reading the trigger each time lets
   a person promote a trial task by resuming it. A trial that should never write could store the
   flag on the task instead. Recommendation: store it on the task with `priory trigger try`, and keep
   reading the trigger key for trigger-wide trials.
