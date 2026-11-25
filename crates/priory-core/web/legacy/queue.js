// The queue page's legacy drawing and actions.
import { render } from "../app.js";
import { actionNote } from "../desk.js";
import { QUEUE_VIEWS, laneCap, moveOptions, queueBands, waitReason } from "../queue.js";
import { taskLabel } from "../tasks.js";
import { refresh, signOut } from "./data.js";
import { deskModel, upNext } from "./desk.js";
import { html, icon, listOrEmpty, listView, post, raw, savedViews, state, stateWord, tile, timeTag } from "./shared.js";
import { taskListRow } from "./tasks.js";


// Queue (DESIGN.md, Queue panel): the waiting tasks by band in admission order, why each waits, and
// a Move menu that sends `priory move` at the revision the page drew.
function moveMenu(task, bands) {
  const options = moveOptions(task, bands, state.data.queue.bands, state.data.queue.revision);
  const busy = state.moveBusy === task.id ? raw('aria-disabled="true"') : "";
  // A live redraw keeps the menu that is open, and the ids let focus return to it.
  return html`<details class="move" data-move="${task.id}" ${state.moveOpen === task.id ? raw("open") : ""}><summary id="move-${task.id}" class="btn secondary" aria-label="Move ${taskLabel(task.key)}">Move${icon("chevron-down", "s13")}</summary>
    <div class="pick">${options.map((option, at) => html`<button type="button" id="move-${task.id}-${at}" data-action="move" data-task="${task.id}" data-option="${at}" ${busy}>${option.label}</button>`)}</div></details>`;
}

const moveNote = (task) => (state.moveNotices.has(task.id) ? html`<p class="note warn" role="status">${icon("circle-x", "s15")}<span>${state.moveNotices.get(task.id)}</span></p>` : "");

function queueListRow(task, bands) {
  return html`<div class="row qrow" data-href="/tasks/${task.id}">${tile(task.state)}
    <span class="l1"><span class="num">#${task.band_position}</span><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span>${task.state}</span><span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || ""}</span><span class="l3 wrap">${waitReason(task, bands)}</span>${moveMenu(task, bands)}${moveNote(task)}</div>`;
}

function queueTableRow(task, bands) {
  return html`<tr data-href="/tasks/${task.id}"><td class="r">#${task.band_position}</td><td>${stateWord(task.state)}</td><td class="mono"><a href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a></td>
    <td class="q full"><span>${task.title || ""}</span><span class="why t3">${waitReason(task, bands)}</span>${moveNote(task)}</td><td class="act">${moveMenu(task, bands)}</td></tr>`;
}

// A daemon from before lane caps sends no `lanes`.
function laneCaps(lanes = []) {
  if (!lanes.length) return "";
  const line = ({ lane, now, cap, full }) => html`<li><span class="mono">${lane}</span><span class="${full ? "t-warn" : ""}">${now}</span><span class="cap">${cap}</span></li>`;
  return html`<div class="sec">Lane caps<span>runs per lane on this runner</span></div><ul class="strip caps">${lanes.map((lane) => line(laneCap(lane)))}</ul>`;
}

export function queueBody(wide) {
  const { queue } = state.data;
  const current = listView(QUEUE_VIEWS);
  const bands = queueBands(queue);
  const held = queue.waiting.filter((task) => task.state === "held");
  const waiting = bands.reduce((sum, group) => sum + group.tasks.length, 0);
  const views = savedViews([["waiting", "Waiting", waiting, current === "waiting"], ["held", "Held", held.length, current === "held"], ["ended", "Last day", queue.ended.length, current === "ended"]], "/queue");
  if (current === "held") return { views, rows: listOrEmpty(held.map((task) => taskListRow({ ...task, reason: task.reason || "held" })), "No task is held. A task waits here while a person or a condition holds it.") };
  if (current === "ended") return { views, rows: listOrEmpty(queue.ended.map((task) => taskListRow({ ...task, since: task.ended })), "No task ended in the last day.") };
  const strip = html`${upNext(deskModel(), false)}${laneCaps(queue.lanes)}`;
  if (!bands.length) return { views, rows: html`${strip}<p class="empty">The queue is empty. A task waits here for its turn once it is added.</p>` };
  if (wide) {
    const groups = bands.map((group) => html`<tr class="grp"><td colspan="5">Band ${group.band}<span>${group.tasks.length}</span></td></tr>${group.tasks.map((task) => queueTableRow(task, bands))}`);
    return { views, rows: html`${strip}<table class="tbl q"><thead><tr><th class="r">#</th><th>State</th><th>Task</th><th>Title and why it waits</th><th></th></tr></thead><tbody>${groups}</tbody></table>` };
  }
  return { views, rows: [strip, ...bands.map((group) => html`<div class="sec">Band ${group.band}<span>${group.tasks.length}</span></div>${group.tasks.map((task) => queueListRow(task, bands))}`)] };
}

// Sends one Move option of a task, as `priory move --expected`; a refusal shows under the row.
export async function moveTask(target) {
  if (state.moveBusy !== null) return;
  const task = state.data.queue.waiting.find((each) => each.id === Number(target.dataset.task));
  const option = task && moveOptions(task, queueBands(state.data.queue), state.data.queue.bands, state.data.queue.revision)[Number(target.dataset.option)];
  if (!option) return;
  state.moveBusy = task.id;
  render();
  const { status, body } = await post(`/api/tasks/${task.id}/move`, option.body).catch(() => ({ status: 0, body: { error: "the connection dropped" } }));
  Object.assign(state, { moveBusy: null, moveOpen: null });
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  if (status === 200) state.moveNotices.delete(task.id);
  else state.moveNotices.set(task.id, actionNote("Move", status, body));
  render();
  document.getElementById(`move-${task.id}`)?.focus();
  refresh();
}

export function queueClick(action, target) {
  if (action !== "move") return false;
  moveTask(target);
  return true;
}

export function queueKey(event) {
  const menu = event.key === "Escape" && document.querySelector("details.move[open]");
  if (!menu) return false;
  menu.open = false;
  menu.querySelector("summary").focus();
  return true;
}

// A Move menu's toggle does not bubble, so it is caught on its way down.
document.addEventListener("toggle", (event) => {
  if (!event.target.matches?.("details.move")) return;
  const task = Number(event.target.dataset.move);
  if (event.target.open) state.moveOpen = task;
  else if (state.moveOpen === task) state.moveOpen = null;
}, true);
