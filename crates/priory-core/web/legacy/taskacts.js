// A task's actions, as the Desk, Tasks and the task page send them.
import { render } from "../app.js";
import { ARM_MS } from "../asks.js";
import { remoteId } from "../tasks.js";
import { refresh, signOut } from "./data.js";
import { html, post, state } from "./shared.js";


// Desk actions on a task, each the twin of its CLI verb. Cancel and revive arm on the first tap and
// send on the second, as a command ask does.
const ACTS = { resume: "Resume", revive: "Revive", cancel: "Cancel", release: "Release" };
const DONE_AS = { resume: "Resumed", revive: "Revived", cancel: "Cancelled", release: "Released", say: "Said" };
const ARMS = ["revive", "cancel"];
export const ENDED = ["done", "failed", "dropped"];
const actsOf = (taskState) => (taskState === "blocked" ? ["resume", "cancel"] : ["failed", "dropped"].includes(taskState) ? ["revive"] : taskState === "done" ? [] : ["cancel"]);

// The action buttons of a task in `taskState`, or the `acts` given, and its last action's note.
export function taskActs(task, taskState, acts = actsOf(taskState)) {
  const busy = state.taskBusy !== null || Boolean(state.failed);
  const buttons = acts.map((act) => {
    const armed = state.taskArmed?.task === task && state.taskArmed.act === act;
    const label = armed ? html`<span><small>Tap again to ${act}</small>${ACTS[act]}</span><span class="drain" aria-hidden="true"></span>` : ACTS[act];
    return html`<button id="act-${task}-${act}" class="btn secondary act ${armed ? "armed" : ""}" data-action="task-act" data-task="${task}" data-act="${act}" aria-disabled="${busy ? "true" : "false"}">${label}</button>`;
  });
  const notice = state.taskNotices.get(task);
  return html`<span class="task-acts">${buttons}</span>${notice ? html`<p class="note acts-note" role="status">${notice}</p>` : ""}`;
}

export function disarmTask() {
  if (!state.taskArmed) return;
  clearTimeout(state.taskArmed.timer);
  state.taskArmed = null;
}

// A task's id from the page: its number here, or `runner:key` for another runner's task.
export const taskIdOf = (value) => (/^\d+$/.test(value) ? Number(value) : value);
// The body an action carries: another runner's task names the generation the page drew.
export const remoteGeneration = (task) => state.data?.tasks.find((each) => each.id == null && remoteId(each) === task)?.generation;
export const remoteBody = (task, body) => (typeof task === "string" ? { ...body, generation: remoteGeneration(task) } : body);

export function taskAct(target) {
  const task = taskIdOf(target.dataset.task);
  const act = target.dataset.act;
  if (target.getAttribute("aria-disabled") === "true") return;
  const armed = state.taskArmed?.task === task && state.taskArmed.act === act;
  disarmTask();
  if (ARMS.includes(act) && !armed) {
    state.taskArmed = { task, act, generation: remoteGeneration(task), at: Date.now(), timer: setTimeout(() => { state.taskArmed = null; render(); }, ARM_MS) };
    return render();
  }
  // A cancel carries the revision the page showed, as `priory cancel --expected` does.
  taskWrite(task, act, typeof task === "string" ? remoteBody(task, {}) : act === "cancel" ? { revision: state.revision } : {});
}

export async function taskWrite(task, act, body) {
  state.taskBusy = task;
  render();
  let reply;
  try {
    reply = await post(`/api/tasks/${task}/${act}`, body);
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  state.taskBusy = null;
  const { status, body: answered } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  // Text typed while the say was in flight stays as the next draft.
  if ([200, 202].includes(status) && act === "say" && (state.sayDrafts.get(task) || "").trim() === body.text) state.sayDrafts.delete(task);
  const remote = typeof task === "string";
  // A sent action shows as the task's newest submission once the list reads it again.
  if (status === 202) state.taskNotices.delete(task);
  else state.taskNotices.set(task, status === 200 ? `${DONE_AS[act]}: ${answered.ok}.`
    : remote && status === 409 ? `Not sent: ${answered.error}. The list reads it again: check the task, then send again.`
    : remote && status === 503 ? `Could not send: ${answered.error}.`
    : status === 409 ? "The state changed since this page loaded. Read the task again, then tap again."
      : status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
        : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the task's state before you send again."
          : `Could not ${act}: ${answered.error || `the server answered ${status}`}.`);
  // A refused action changes no listing, so the refresh may not paint: the note shows now.
  render();
  refresh();
}
