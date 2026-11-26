// A task's actions, as the legacy Desk, Tasks and Inbox draw them; acts.js sends them.
import { ACTS, actsOf, draft, pressTask, writeTask } from "../acts.js";
import { remoteId } from "../tasks.js";
import { html, state } from "./shared.js";

export { writeTask as taskWrite } from "../acts.js";

// Desk actions on a task, each the twin of its CLI verb. Cancel and revive arm on the first tap and
// send on the second, as a command ask does.
// The action buttons of a task in `taskState`, or the `acts` given, and its last action's note. They
// send the `revision` of the read the page drew the task from.
export function taskActs(task, taskState, acts = actsOf(taskState), revision = state.revision) {
  const remote = typeof task === "string";
  const busy = state.taskBusy !== null || Boolean(state.failed) || (!remote && revision === undefined);
  const buttons = acts.map((act) => {
    const armed = state.taskArmed?.task === task && state.taskArmed.act === act;
    const label = armed ? html`<span><small>Tap again to ${act}</small>${ACTS[act]}</span><span class="drain" aria-hidden="true"></span>` : ACTS[act];
    return html`<button id="act-${task}-${act}" class="btn secondary act ${armed ? "armed" : ""}" data-action="task-act" data-task="${task}" data-act="${act}" ${remote || revision === undefined ? "" : html`data-revision="${revision}"`} aria-disabled="${busy ? "true" : "false"}">${label}</button>`;
  });
  const notice = state.taskNotices.get(task);
  return html`<span class="task-acts">${buttons}</span>${notice ? html`<p class="note acts-note" role="status">${notice}</p>` : ""}`;
}

// A task's id from the page: its number here, or `runner:key` for another runner's task.
export const taskIdOf = (value) => (/^\d+$/.test(value) ? Number(value) : value);
// The body an action carries: another runner's task names the generation the page drew.
export const remoteGeneration = (task) => state.data?.tasks.find((each) => each.id == null && remoteId(each) === task)?.generation;
export const remoteBody = (task, body) => (typeof task === "string" ? { ...body, generation: remoteGeneration(task) } : body);

function taskAct(target) {
  const task = taskIdOf(target.dataset.task);
  if (target.getAttribute("aria-disabled") === "true") return;
  pressTask(task, target.dataset.act, { generation: remoteGeneration(task), revision: target.dataset.revision && Number(target.dataset.revision), body: typeof task === "string" ? remoteBody(task, {}) : undefined });
}

export function taskClick(action, target) {
  if (action !== "task-act") return false;
  taskAct(target);
  return true;
}

export function taskInput(target) {
  const form = target.id === "say" && target.closest('form[data-action="say"]');
  if (form) draft(taskIdOf(form.dataset.task), target.value);
}

export function taskSubmit(action, form) {
  if (action !== "say") return false;
  const task = taskIdOf(form.dataset.task);
  const text = (state.sayDrafts.get(task) || "").trim();
  if (text && state.taskBusy === null) writeTask(task, "say", remoteBody(task, { text }));
  return true;
}
