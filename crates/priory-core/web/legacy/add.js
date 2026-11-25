// The add page's legacy drawing and actions.
import { addBody, addOutcome, swapLine, withDefaults } from "../add.js";
import { taskLabel, workStep } from "../tasks.js";
import { api, refresh, signOut } from "./data.js";
import { focusKey, html, icon, post, raw, state, tile } from "./shared.js";


export const addButton = (kind) => html`<button class="${kind}" data-action="add-open" aria-label="Add a task" aria-keyshortcuts="N" aria-haspopup="dialog">${icon("plus", "s20")}${kind === "addtop" ? "Add" : ""}</button>`;

// Add (DESIGN.md, Add): one field and three chips; Enter queues the task through POST /api/request.
// The draft lives in localStorage until the daemon takes it, so a reload or a dropped connection keeps it.
const DRAFT_KEY = "priory.add";
export const addSheet = () => document.getElementById("add");
// Only an unsent draft keeps its phase across a reload, so it still asks before it sends.
export const keepDraft = () => localStorage.setItem(DRAFT_KEY, JSON.stringify({ draft: state.add.draft, phase: ["offline", "unsure"].includes(state.add.phase) ? state.add.phase : "empty" }));
// Lanes have no list of their own: the chip offers each lane a listed task carries.
const knownLanes = () => [...new Set([...state.data.tasks, ...state.data.queue.waiting].map((task) => task.lane).filter(Boolean))].sort();

// The factory may reload while the page is open; a read that fails keeps the last one.
export async function readAddDefaults() {
  state.add.defaults = await api("/api/request").catch(() => state.add.defaults);
}

export async function openAdd() {
  if (!state.data || addSheet().open) return;
  if (state.add.phase === "added") Object.assign(state.add, { phase: "empty", task: null });
  const before = state.add.defaults;
  const read = readAddDefaults();
  if (!before) await read;
  const { defaults } = state.add;
  if (addSheet().open) return renderAdd();
  renderAdd();
  addSheet().showModal();
  document.getElementById("add-text")?.focus();
  await read;
  // A redraw closes an open picker, so only a changed list draws again.
  if (addSheet().open && JSON.stringify(state.add.defaults) !== JSON.stringify(defaults)) renderAdd();
}

export function closeAdd() {
  addSheet().close();
}

export function chip(name, label, value, values, none) {
  const options = [...new Set([...values, value].filter(Boolean))];
  return html`<label class="chip"><span>${label}</span><select name="${name}" data-add-chip="${name}">
    <option value="" ${value ? "" : raw("selected")}>${none}</option>
    ${options.map((option) => html`<option ${option === value ? raw("selected") : ""}>${option}</option>`)}</select>${icon("chevron-down", "s13")}</label>`;
}

function addedTask() {
  const { key, id, body } = state.add.task;
  const task = body || { key, state: "queued" };
  return html`<div class="added" role="status">
    <p class="line">Added. The task waits in the queue for its turn.</p>
    <div class="row">${tile(task.state)}
      <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${workStep(task)}</span><span class="age">${task.state}</span></span>
      <span class="l2">${task.brief?.request || task.brief?.title || (id === null ? "No task took this key yet. It shows on the Desk once one does." : "")}</span>
      <span class="l3">${[task.band, task.lane].filter(Boolean).join(", ")}</span></div>
    <div class="acts"><button class="btn secondary" data-action="add-another">Add another</button><button class="btn primary" data-action="add-done">Close</button></div></div>`;
}

const ADD_LINES = {
  offline: "Offline. The draft stays on this device, and nothing is sent until you confirm it after you reconnect.",
  unsure: "The connection dropped while sending, so Priory may have queued it. The draft stays until you send it again or discard it.",
};

export function renderAdd() {
  const sheet = addSheet();
  if (!sheet) return;
  const { phase, error, defaults, swapped } = state.add;
  const draft = withDefaults(state.add.draft, defaults);
  const sending = phase === "sending";
  const back = !state.failed && ["offline", "unsure"].includes(phase);
  const status = phase === "refused"
    ? html`<p class="field-error" id="add-status">${icon("circle-x", "s15")}Could not add: ${error}.</p>`
    : back
      ? html`<p class="warnline" id="add-status">${icon("unplug", "s15")}<span>${phase === "unsure"
        ? "Reconnected. Priory may have queued this before the drop: check the Desk's All view, then send it if it is missing."
        : "Reconnected. This draft has not been sent. Send it now?"}</span></p>`
      : ADD_LINES[phase] ? html`<p class="warnline" id="add-status">${icon("unplug", "s15")}<span>${ADD_LINES[phase]}</span></p>` : "";
  const empty = !draft.text.trim();
  const label = sending ? "Adding…" : back ? "Send" : state.failed ? "Keep draft" : "Add";
  const body = phase === "added" ? addedTask() : html`<form data-action="add" ${sending ? raw('aria-busy="true"') : ""}>
    <label class="sr" for="add-text">What should the factory do?</label>
    <input id="add-text" name="text" autocomplete="off" enterkeyhint="send" placeholder="What should the factory do?" value="${draft.text}" ${sending ? raw("readonly") : ""} ${status || swapped ? raw('aria-describedby="add-status add-swapped"') : ""}>
    ${status}${swapped ? html`<p class="warnline" id="add-swapped">${icon("circle-alert", "s15")}<span>${swapLine(swapped, defaults?.workflow)}</span></p>` : ""}
    <div class="chips">${chip("workflow", "Workflow", draft.workflow, defaults?.workflows || [], defaults?.workflow ? `default (${defaults.workflow})` : "pick one")}
      ${chip("band", "Band", draft.band, defaults?.bands || state.data?.bands || [], "default")}
      ${chip("lane", "Lane", draft.lane, state.data ? knownLanes() : [], "routed")}</div>
    <div class="acts">${phase === "empty" || sending ? "" : html`<button type="button" class="ghost" data-action="add-discard">Discard draft</button>`}<span class="sp"></span>
      <button class="btn primary" aria-disabled="${empty || sending ? "true" : "false"}">${label}</button></div></form>`;
  const focused = document.activeElement?.closest?.("#add") ? focusKey(document.activeElement) : null;
  const caret = document.activeElement?.id === "add-text" ? document.activeElement.selectionStart : undefined;
  state.add.shownOffline = Boolean(state.failed);
  sheet.innerHTML = html`<div class="shead"><h2 id="add-title">Add a task</h2><button class="iconbtn" data-action="add-close" aria-label="Close">${icon("x", "s20")}</button></div>${body}`.text;
  if (focused) sheet.querySelector(focused)?.focus();
  if (caret !== undefined) sheet.querySelector("#add-text")?.setSelectionRange(caret, caret);
}

export async function submitAdd() {
  const { draft, phase } = state.add;
  const body = addBody(withDefaults(draft, state.add.defaults));
  if (!body || phase === "sending") return;
  // Offline, Enter only keeps the draft; it goes once the person confirms it after reconnecting.
  if (state.failed) {
    state.add.phase = "offline";
    keepDraft();
    return renderAdd();
  }
  // Stored as unsure first: a reload while it sends must not offer it as unsent.
  localStorage.setItem(DRAFT_KEY, JSON.stringify({ draft, phase: "unsure" }));
  state.add.phase = "sending";
  renderAdd();
  const reply = await post("/api/request", body).catch(() => ({ status: 0, body: {} }));
  const outcome = addOutcome(reply.status, reply.body);
  if (outcome.state === "signed-out") {
    state.add.phase = "empty";
    keepDraft();
    closeAdd();
    return signOut("The daemon refused this token. Paste the current one.");
  }
  if (outcome.state === "added") {
    state.add.draft = { text: "" };
    state.add.swapped = null;
    state.add.task = { key: outcome.key, id: outcome.id, body: null };
    localStorage.removeItem(DRAFT_KEY);
    if (outcome.id !== null) api(`/api/tasks/${outcome.id}`).then((task) => { if (state.add.task?.id === outcome.id) { state.add.task.body = task; renderAdd(); } }, () => {});
    refresh();
  }
  Object.assign(state.add, { phase: outcome.state, error: outcome.error || null });
  if (outcome.state !== "added") keepDraft();
  renderAdd();
  if (outcome.state === "added") addSheet().querySelector(".added .btn.primary")?.focus();
}

export function discardAdd() {
  Object.assign(state.add, { phase: "empty", error: null, swapped: null, draft: { text: "" } });
  localStorage.removeItem(DRAFT_KEY);
  renderAdd();
  document.getElementById("add-text")?.focus();
}
