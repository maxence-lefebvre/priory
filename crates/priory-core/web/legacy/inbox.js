// The inbox page's legacy drawing and actions.
import { render } from "../app.js";
import { droppedWhy, eventWords, neverOwned, requestedWorkflow, routeAction, unheardActs } from "../inbox.js";
import { taskLabel } from "../tasks.js";
import { addSheet, keepDraft, openAdd, readAddDefaults, renderAdd } from "./add.js";
import { api, refresh, signOut } from "./data.js";
import { html, post, raw, savedViews, state, taskName, tile, timeTag } from "./shared.js";
import { taskActs, taskWrite } from "./taskacts.js";


// Unrouted (DESIGN.md, Desk): events that reached no task, by resource, each with a way to route the next one.
export async function loadUnrouted() {
  const view = state.unrouted;
  if (view.loading || !state.token || !state.data || state.failed) return;
  view.loading = true;
  try {
    const body = await api("/api/inbox?unrouted=1");
    Object.assign(view, { groups: body.unrouted, revision: body.revision, error: null });
  } catch (error) {
    view.error = error.message;
  }
  view.loading = false;
  render();
}

export function unroutedRows() {
  const view = state.unrouted;
  if (view.error) return html`<p class="empty">Could not read the unrouted events: ${view.error}. <button class="btn secondary" data-action="retry">Try again</button></p>`;
  if (!view.groups) {
    queueMicrotask(loadUnrouted);
    return html`<p class="empty">Loading the unrouted events…</p>`;
  }
  const { owned, other, unheard = [], dropped = [] } = view.groups;
  const rest = neverOwned(other);
  const restLine = rest ? html`<p class="meta unowned">${rest}</p>` : "";
  const heard = unheard.length || dropped.length ? html`${unheard.length ? html`<div class="sec" id="unheard">Unheard<span>no agent read them</span></div>${unheard.map(unheardGroup)}` : ""}${dropped.length ? html`<div class="sec" id="dropped">Dropped<span>a script judged each once</span></div>${dropped.map(droppedGroup)}` : ""}<div class="sec">Unrouted<span>reached no task</span></div>` : "";
  if (!owned.length) return html`${heard}<p class="empty">No event of a task's resource went unrouted. An event about an ended task's pull request or issue shows here.</p>${restLine}`;
  return [heard, ...owned.map((group) => {
    const { task } = group;
    const notice = view.notices.get(group.resource);
    return html`<section class="ugroup" aria-label="${group.resource}"><p class="l1"><span class="mono">${group.resource}</span><span class="num">${group.events.length}</span></p>
      <p class="meta">${tile(task.state, "sm")}<a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a> ${task.state}</p>
      ${eventRows(group.events, "queued")}
      <div class="routeform">${routeButton(group, routeAction(task))}</div>
      ${notice ? html`<p class="note" role="status">${notice}</p>` : ""}</section>`;
  }), restLine];
}

// A group's 5 newest events as history rows, then how many are older.
function eventRows(events, state) {
  return html`<ol class="hist">${events.slice(0, 5).map((event) => html`<li>${tile(state, "sm")}<span><b title="${event.type}">${eventWords(event.type)}</b> ${event.subject || ""}${event.line ? html`<span class="meta">${event.line}</span>` : ""}</span>${timeTag(event.time)}</li>`)}</ol>
    ${events.length > 5 ? html`<p class="meta">and ${events.length - 5} older</p>` : ""}`;
}

// The one button that routes a group's next event: revive, request a task or subscribe.
function routeButton(group, action) {
  const { task } = group;
  if (action === "revive") return taskActs(task.id, task.state);
  if (action === "request") return group.request_text
    ? html`<button type="button" class="btn secondary" data-action="request-task" data-text="${group.request_text}" data-workflow="${task.workflow}">Request a task</button>`
    : html`<p class="meta">No new task can take this resource from here: start one from its thread.</p>`;
  const busy = state.unrouted.busy === group.resource ? raw('aria-disabled="true"') : "";
  return html`<button type="button" class="btn secondary" data-action="subscribe-task" data-resource="${group.resource}" data-task="${task.id}" ${busy}>Subscribe ${taskLabel(task.key)}</button>`;
}

// A request a pre script dropped (DESIGN.md, Inbox): its trigger, script, source event and why, with no action.
function droppedGroup(drop) {
  const events = drop.event ? [drop.event] : [];
  return html`<section class="ugroup" aria-label="${drop.key || "dropped request"}"><p class="l1"><span class="mono">${drop.key || "-"}</span><span class="num">${events.length}</span></p>
    <p class="meta">${tile("dropped", "sm")}trigger ${drop.trigger}, ${drop.script}</p>
    ${eventRows(events, "dropped")}
    <p class="meta">${droppedWhy(drop)}</p></section>`;
}

// One task's unheard events (DESIGN.md, Inbox), with what gets them to an agent.
function unheardGroup(group) {
  const { task } = group;
  const [first, ...rest] = unheardActs(task);
  const busy = state.taskBusy !== null || Boolean(state.failed);
  const act = first === "say"
    ? html`<button type="button" class="btn secondary" data-action="unheard-say" data-task="${task.id}" data-text="${group.say_text}" aria-disabled="${busy ? "true" : "false"}">Say to ${taskLabel(task.key)}</button>${taskActs(task.id, task.state, rest)}
      <p class="meta">Say sends “${group.say_text}”${rest.includes("resume") ? " It reads them when it resumes." : ""}</p>`
    : first ? routeButton({ ...group, resource: task.key }, first)
      : html`<p class="meta">Its next run reads them.</p>`;
  return html`<section class="ugroup" aria-label="${taskLabel(task.key)}"><p class="l1"><a class="mono" href="/tasks/${task.id}" title="task ${task.id}">${taskLabel(task.key)}</a><span class="num">${group.events.length}</span></p>
    <p class="meta">${tile(task.state, "sm")}${task.state}, ${task.workflow}</p>
    ${eventRows(group.events, "asleep")}
    <div class="routeform">${act}</div></section>`;
}

// The Inbox panel (DESIGN.md, Inbox): its one saved view, Unrouted, counts the events of a task's resources.
export function inboxViews() {
  const groups = state.unrouted.groups;
  const count = groups ? groups.owned.reduce((sum, group) => sum + group.events.length, 0) : null;
  return savedViews([["unrouted", "Unrouted", count, true]], "/inbox");
}

// Subscribes a task to the resource, as `priory subscribe`, with the revision the page showed.
export async function subscribeTask(resource, task) {
  const view = state.unrouted;
  view.busy = resource;
  render();
  let reply;
  try {
    // The revision the Inbox groups were read at, so a group shown from an older read is refused.
    reply = await post("/api/subscribe", { task, resource, revision: view.revision });
  } catch {
    reply = { status: 0, body: { error: "the connection dropped" } };
  }
  view.busy = null;
  const { status, body } = reply;
  if (status === 401) return signOut("The daemon refused this token. Paste the current one.");
  view.notices.set(resource, status === 200 ? `Subscribed: ${body.ok}. Its next event goes to ${taskName(task)}.`
    : status === 409 ? "The state changed since this page loaded. Check the task, then subscribe again."
      : status === 503 ? "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again."
        : status === 504 || status === 0 ? "The daemon may have applied this but did not confirm it. Check the task before you send again."
          : `Could not subscribe: ${body.error || `the server answered ${status}`}.`);
  render();
  refresh();
}

// The Add sheet with the server's text, whose URL keys the new task by the group's resource.
export async function requestTask(text, workflow) {
  await readAddDefaults();
  // The person may have opened the sheet and typed while the read ran.
  if (addSheet().open) return renderAdd();
  const { draft, defaults } = state.add;
  draft.text = text;
  const offered = requestedWorkflow(workflow, defaults?.opted_out);
  if (offered === undefined) delete draft.workflow;
  else draft.workflow = offered;
  state.add.swapped = offered === undefined ? workflow : null;
  keepDraft();
  openAdd();
}

export function inboxClick(action, target) {
  if (action === "request-task") requestTask(target.dataset.text, target.dataset.workflow);
  else if (action === "unheard-say") {
    if (target.getAttribute("aria-disabled") !== "true") taskWrite(Number(target.dataset.task), "say", { text: target.dataset.text });
  } else if (action === "subscribe-task") {
    if (state.unrouted.busy === null) subscribeTask(target.dataset.resource, Number(target.dataset.task));
  } else return false;
  return true;
}
