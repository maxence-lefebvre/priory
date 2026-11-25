// The tasks page's legacy drawing and actions.
import { render } from "../app.js";
import { TASK_VIEWS, elsewhere, filtered, olderPart, remoteActs, remoteId, remoteSent, taskFacets, taskLabel, taskViews, workStep } from "../tasks.js";
import { ago, batchTag, facet, facetPicks, facetQuery, html, icon, listView, olderRow, olderTasks, prsTag, raw, reasonShown, role, savedViews, state, stateWord, tile, timeTag } from "./shared.js";
import { taskActs } from "./taskacts.js";


// Another runner's task (DESIGN.md, Task row and run row): its hold for a state, and no page here.
// A tap opens its actions in place; the newest action sent to it takes a line of its own.
const TONE = { action: "t-action", warn: "t-warn", quiet: "" };
const toggle = (id, open, key) => html`<button type="button" id="remote-${id}" class="chev" data-action="remote-open" data-task="${id}" aria-expanded="${open ? "true" : "false"}" aria-label="${open ? "Close" : "Open"} the actions of ${key}">${icon(open ? "chevron-up" : "chevron-down", "s15")}</button>`;
const sentOf = (task) => {
  const sent = remoteSent(task);
  return sent ? html`<span class="${TONE[sent.tone]}">${sent.text}</span>` : "";
};
function remoteActions(task) {
  const id = remoteId(task);
  const said = state.taskBusy === id ? raw('aria-disabled="true"') : "";
  const say = task.hold === "kept" ? html`<form class="writeform" data-action="say" data-task="${id}"><label class="sr" for="say">Say to the agent</label>
    <textarea id="say" name="text" rows="2" placeholder="Say to the agent">${state.sayDrafts.get(id) || ""}</textarea><button class="btn primary" ${said}>Say</button></form>` : "";
  return html`${taskActs(id, null, remoteActs(task))}${say}<p class="note acts-note">${icon("corner-down-right", "s15")}<span>This task runs on ${task.runner}. An action goes there through the log, and ${task.runner} applies it when it reads it.</span></p>`;
}

function remoteListRow(task) {
  const id = remoteId(task);
  const open = state.remoteOpen === id;
  const sent = sentOf(task);
  return html`<div class="row remote" data-action="remote-open" data-task="${id}">${tile("elsewhere")}
    <span class="l1"><span class="mono">${taskLabel(task.key)}</span><span>${task.workflow || ""}</span><span>${task.hold}</span><span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || taskLabel(task.key)}</span>${sent ? html`<span class="l3">${sent}</span>` : ""}<span class="l3"><span>${elsewhere(task, ago)}</span></span>
    ${toggle(id, open, taskLabel(task.key))}${open ? remoteActions(task) : ""}</div>`;
}

function remoteTableRow(task) {
  const id = remoteId(task);
  const open = state.remoteOpen === id;
  const sent = sentOf(task);
  const row = html`<tr class="remote" data-action="remote-open" data-task="${id}"><td><span class="sti">${tile("elsewhere", "sm")}${task.hold}</span></td><td class="mono">${taskLabel(task.key)}</td>
    <td class="q full"><span>${task.title || taskLabel(task.key)}</span>${sent ? html`<span class="why">${sent}</span>` : ""}<span class="why t3">${elsewhere(task, ago)}</span></td>
    <td>${task.workflow || ""}</td><td>${task.lane || ""}</td><td>${task.band || ""}</td><td class="r">${timeTag(task.since)} ${toggle(id, open, taskLabel(task.key))}</td></tr>`;
  return open ? html`${row}<tr class="remote-acts"><td></td><td></td><td colspan="5">${remoteActions(task)}</td></tr>` : row;
}

export function taskListRow(task) {
  if (elsewhere(task, ago)) return remoteListRow(task);
  const line3 = reasonShown(task) ? html`<span class="t-${role(task.state)}">${task.reason}</span>${batchTag(task)}` : html`<span>${task.lane || ""}</span><span>${task.band || ""}</span>${batchTag(task)}`;
  return html`<div class="row" data-href="/tasks/${task.id}">${tile(task.state)}
    <span class="l1"><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span>${workStep(task)}</span><span>${task.state}</span>${prsTag(task)}<span class="age">${timeTag(task.since)}</span></span>
    <span class="l2 r">${task.title || task.tldr || ""}</span><span class="l3">${line3}</span><span class="chev">${icon("chevron-right", "s15")}</span></div>`;
}

function taskTableRow(task) {
  if (elsewhere(task, ago)) return remoteTableRow(task);
  return html`<tr data-href="/tasks/${task.id}"><td>${stateWord(task.state)}</td><td class="mono"><a href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a></td>
    <td class="q full"><span>${task.title || task.tldr || ""}${task.batch == null ? "" : " "}${batchTag(task)}</span>${reasonShown(task) ? html`<span class="why t-${role(task.state)}">${task.reason}</span>` : ""}</td>
    <td>${task.workflow} <span class="t3">${task.step}</span></td><td>${task.lane || ""}</td><td>${task.band || ""}</td><td class="r">${timeTag(task.since)}</td></tr>`;
}

export function tasksBody(wide) {
  const all = state.data.tasks;
  const specs = taskFacets(all);
  const picks = facetPicks(specs);
  const current = listView(TASK_VIEWS);
  const views = taskViews(filtered(all, picks, specs));
  const { counted, unloaded } = olderPart({ view: current, history: "ended", older: olderTasks(), picked: picks.size > 0 });
  const chips = savedViews([
    ["active", "Active", views.active.length, current === "active"],
    ["blocked", "Blocked", views.blocked.length, current === "blocked"],
    ["ended", "Ended", views.ended.length + counted, current === "ended"],
  ], "/tasks", facetQuery(picks));
  const facets = html`<div class="facets" aria-label="Facets">${specs.map((spec) => facet(current, picks, spec, "/tasks", all))}</div>`;
  const list = views[current];
  const empty = { active: html`No active tasks. Add one with the + button, or run <code>priory add</code>.`, blocked: "No blocked tasks. A task shows here when it needs a person to go on.", ended: "No task ended in the last 7 days. A task shows here once it is done, failed or dropped." }[current];
  if (!list.length && !unloaded) return { views: html`${chips}${facets}`, rows: html`<p class="empty">${empty}</p>` };
  const rows = wide
    ? html`<table class="tbl tasks"><thead><tr><th>State</th><th>Task</th><th>Title</th><th>Workflow and step</th><th>Lane</th><th>Band</th><th class="r">Age</th></tr></thead><tbody>${list.map(taskTableRow)}${olderRow("tasks", unloaded, true, 7)}</tbody></table>`
    : [...list.map(taskListRow), olderRow("tasks", unloaded, false)];
  return { views: html`${chips}${facets}`, rows };
}

export function tasksClick(action, target, event) {
  if (action !== "remote-open" || event.target.closest("form, .task-acts")) return false;
  state.remoteOpen = state.remoteOpen === target.dataset.task ? null : target.dataset.task;
  render();
  return true;
}
