// Priory's web view (DESIGN.md): the frame, the Desk and the Asks, with answering through the CLI's verbs.
// Agent text is untrusted: every value goes through `html`, which escapes it.
import { tasksClick } from "./legacy/tasks.js";
import { arrivalAsk } from "./desk.js";
import { addButton, addClick, addInput, addSheet, addSubmit, openAdd, renderAdd } from "./legacy/add.js";
import { answerSheet, askDetail, askOnScreen, asksClick, asksInput, asksKey, asksPhone, asksSubmit, disarm, drawer, renderSheet } from "./legacy/asks.js";
import { batchClick, batchPage, drawLanes } from "./legacy/batch.js";
import { costsChange, costsClick, costsPage, loadCosts } from "./legacy/costs.js";
import { keepTokenForWorker, refresh, signOut } from "./legacy/data.js";
import { deskClick, deskPhone, deskView, openAsks, seen } from "./legacy/desk.js";
import { flowsClick, flowsPage } from "./legacy/flows.js";
import { OWN_PAGE_LISTS, connection, laterPanel, listPanel, loading, notFound, shownPanels, signIn, staleStrip, tabs, tree, watchSections } from "./legacy/frame.js";
import { inboxClick, inboxViews, unroutedRows } from "./legacy/inbox.js";
import { disarmMemory, memoriesClick, memoriesPage } from "./legacy/memories.js";
import { queueClick, queueKey } from "./legacy/queue.js";
import { reportsClick, reportsInput, reportsPage, reportsSubmit } from "./legacy/reports.js";
import { morePage, settingsChange, settingsClick, settingsScreen } from "./legacy/settings.js";
import { current, focusKey, html, icon, layout, route, routeKey, selectedAsk, state, view } from "./legacy/shared.js";
import { taskPage } from "./legacy/task.js";
import { disarmTask, remoteGeneration, taskClick, taskInput, taskSubmit } from "./legacy/taskacts.js";
import { keyboardSheet, observeTerminal, paintTerminal, syncTerminal, terminalClick, terminalInput, terminalKey, terminalSubmit } from "./legacy/terminal.js";
import { PANELS, TYPED_FIELDS, pageTitle } from "./pages.js";
import { go } from "./router.js";

export { api, cache, refresh, revision, revisionMoved } from "./legacy/data.js";
if (state.token) keepTokenForWorker(state.token);

export function render() {
  const app = document.getElementById("app");
  if (!app) return;
  if (state.reportPending && !state.reportPending.sending && (!state.token || current() !== "reports")) {
    clearTimeout(state.reportPending.timer);
    state.reportPending = null;
  }
  // Leaving the page closes an entry's menu and disarms its Drop.
  if (state.renderedKey && state.renderedKey !== routeKey()) Object.assign(state.planEdit, { open: null, armed: null });
  if (state.renderedKey) state.scrolls.set(state.renderedKey, [...app.querySelectorAll(".scroll")].map((element) => element.scrollTop));
  const focused = focusKey(document.activeElement);
  const typed = app.querySelector("#token")?.value;
  const field = TYPED_FIELDS.includes(document.activeElement?.id) ? document.activeElement : null;
  const caret = field ? [field.id, field.selectionStart, field.selectionEnd, field.selectionDirection] : undefined;
  state.observers.forEach((observer) => observer.disconnect());
  state.observers = [];
  // A new revision is a new question: a tap armed on the old one must not run it.
  if (state.armed && (!state.token || selectedAsk() !== String(state.armed.id) || askOnScreen()?.revision !== state.armed.revision)) disarm();
  // Likewise a Cancel armed on another runner's task at a generation the list no longer shows.
  if (typeof state.taskArmed?.task === "string" && remoteGeneration(state.taskArmed.task) !== state.taskArmed.generation) disarmTask();
  if (state.memoryArmed && current() !== "memories") disarmMemory();
  // The answer bar shows once the options scroll away; a redraw keeps it shown, so focus on it stays.
  const barShown = app.querySelector(".abar:not([hidden])") !== null;
  app.innerHTML = page().text;
  if (barShown) app.querySelector(".abar")?.removeAttribute("hidden");
  document.title = state.token && state.data ? pageTitle(location.pathname, { tasks: [...state.data.tasks, state.task?.body].filter(Boolean) }) : "Priory";
  paintTerminal(app, true);
  observeTerminal(app);
  // A render must not restart the countdown; CSP allows no inline style, so it is set here.
  app.querySelectorAll(".drain").forEach((drain) => {
    const armed = drain.closest("[data-action=memory-act]") ? state.memoryArmed : drain.closest(".act") ? state.taskArmed : state.armed;
    drain.style.animationDelay = `-${armed ? Date.now() - armed.at : 0}ms`;
  });
  state.renderedKey = routeKey();
  state.rendered = Date.now();
  const scrolls = state.scrolls.get(state.renderedKey) || [];
  app.querySelectorAll(".scroll").forEach((element, i) => { element.scrollTop = scrolls[i] || 0; });
  if (typed && app.querySelector("#token")) app.querySelector("#token").value = typed;
  // The Desk table draws some controls twice and CSS hides one copy: focus goes to the shown one.
  if (focused) [...app.querySelectorAll(focused)].find((element) => !element.closest("[inert]") && element.getClientRects().length)?.focus({ preventScroll: true });
  if (caret !== undefined) app.querySelector(`#${caret[0]}`)?.setSelectionRange(caret[1], caret[2], caret[3]);
  // A link or control inside a row, such as its key, batch or Move menu, acts on its own.
  for (const row of app.querySelectorAll("[data-href]")) row.addEventListener("click", (event) => { if (!event.target.closest("a, button, details")) go(row.dataset.href); });
  app.querySelectorAll("[data-grow]").forEach((part) => { part.style.flexGrow = part.dataset.grow; });
  drawLanes(app);
  watchSections(app);
  if (app.querySelector("[data-since]")) seen();
  // A render may call this; the read renders again once it ends.
  const spend = state.data?.spend;
  const old = !spend || Date.now() - spend.updated > 60000;
  if (app.querySelector("[data-costs]") && old && !state.failed && !state.costs.loading && !state.costs.error) setTimeout(loadCosts);
  // The sheet lives outside #app; it redraws when the connection drops or comes back under it.
  if (addSheet().open && state.add.shownOffline !== Boolean(state.failed)) renderAdd();
  if (answerSheet().open) renderSheet();
}

function page() {
  if (!state.token) return signIn();
  if (!state.data) return loading();
  const size = layout();
  const here = current();
  const arrival = size === "wide" && here === "desk" && !selectedAsk() && arrivalAsk({ from: state.drawn, list: state.list, view: deskView(), asks: openAsks() });
  if (arrival) {
    Object.assign(state, { list: "desk", listUrl: routeKey(), from: routeKey() });
    history.replaceState(history.state, "", `/asks/${encodeURIComponent(arrival)}`);
  }
  state.drawn = routeKey();
  if (["desk", "asks"].includes(here) && !selectedAsk()) Object.assign(state, { list: here, listUrl: routeKey() });
  // An ask keeps the list it was opened from in its history entry, so Back to it restores that list.
  else if (selectedAsk() && history.state?.listUrl) Object.assign(state, { list: history.state.list, listUrl: history.state.listUrl });
  else if (selectedAsk()) history.replaceState({ ...history.state, list: state.list, listUrl: state.listUrl }, "");
  const taskId = here === "tasks" && /^\d+$/.test(route()[1] || "") ? Number(route()[1]) : null;
  const batchId = here === "batches" && /^\d+$/.test(route()[1] || "") ? route()[1] : null;
  if (PANELS[here] && !selectedAsk() && !taskId) state.from = routeKey();
  // A detail page keeps the list it was opened from in its history entry, so Back restores its link.
  else if ((taskId || batchId) && !history.state?.from) history.replaceState({ ...history.state, from: state.from }, "");
  if (size === "compact") {
    let body;
    if (selectedAsk()) body = askDetail(selectedAsk(), false);
    else if (here === "desk") body = deskPhone();
    else if (here === "asks") body = asksPhone();
    else if (OWN_PAGE_LISTS[here] && !taskId) {
      const { views, rows } = OWN_PAGE_LISTS[here](false);
      body = html`<div class="top"><h1>${PANELS[here][0]}</h1><span class="sp"></span>${connection()}</div>
        <div class="panel" role="region" aria-label="${PANELS[here][0]}"><div class="scroll">${views}${rows}</div></div>`;
    } else if (here === "inbox") body = html`<div class="top"><h1>Inbox</h1><span class="sp"></span>${connection()}</div>
      <div class="panel" role="region" aria-label="Inbox"><div class="scroll">${inboxViews()}${unroutedRows()}</div></div>`;
    else if (here === "more") body = morePage();
    else if (taskId) body = taskPage(taskId);
    else if (batchId) body = batchPage(batchId);
    else if (here === "memories") body = memoriesPage();
    else if (here === "reports") body = reportsPage(true);
    else if (here === "costs") body = costsPage(true);
    else if (here === "workflow") body = flowsPage(true);
    else if (here === "settings") body = html`<div class="top"><a class="back" href="/more">${icon("chevron-left", "s22")}More</a></div>${settingsScreen()}`;
    else if (PANELS[here]) body = html`<div class="top"><h1>${PANELS[here][0]}</h1></div>${laterPanel(here)}`;
    else body = notFound();
    const tab = selectedAsk() ? null : here === "settings" ? "more" : batchId ? "tasks" : here === "memories" ? "desk" : here;
    return html`${staleStrip()}${body}${tab ? tabs(tab, "bar") : ""}`;
  }
  const list = selectedAsk() ? state.list : here;
  const top = html`<header class="wtop"><span class="fac">${state.data.views.factory}</span>${connection()}<span class="sp"></span>${addButton("addtop")}</header>`;
  const wide = size === "wide";
  const utility = taskId ? taskPage(taskId) : batchId ? batchPage(batchId) : here === "memories" ? memoriesPage() : here === "reports" ? reportsPage(false) : here === "costs" ? costsPage(false) : here === "workflow" ? flowsPage(false) : here === "settings" ? settingsScreen() : here === "more" ? morePage() : PANELS[list] ? null : notFound();
  const nav = (kind) => tabs(here === "settings" && wide ? "settings" : here === "settings" ? "more" : batchId ? "tasks" : here === "memories" ? "desk" : list, kind);
  if (utility) {
    const capped = taskId || batchId || here === "memories" ? "page" : "";
    return html`${staleStrip()}${top}<div class="${size} no-drawer">${wide ? nav("rail") : ""}<div class="list ${capped}">${utility}</div></div>${wide ? "" : nav("bar")}`;
  }
  if (OWN_PAGE_LISTS[list]) return html`${staleStrip()}${top}<div class="${size} no-drawer">${wide ? nav("rail") : ""}${listPanel(list, wide)}</div>${wide ? "" : nav("bar")}`;
  if (!wide) return html`${staleStrip()}${top}<div class="medium">${listPanel(list, false)}${drawer(false, [list])}</div>${nav("bar")}`;
  const layoutNode = view().layout;
  const plain = layoutNode.tabs?.every((child) => child.panel);
  // Events open no detail, so a wide Inbox takes the drawer's width too.
  if (plain && list === "inbox") return html`${staleStrip()}${top}<div class="wide no-drawer">${nav("rail")}${listPanel(list, true)}</div>`;
  const main = plain ? listPanel(list, true) : tree(layoutNode, list);
  return html`${staleStrip()}${top}<div class="wide">${nav("rail")}${main}${drawer(true, plain ? [list] : shownPanels(layoutNode, list))}</div>`;
}

// Events: each page's file acts on its own controls.
const CLICKS = [terminalClick, costsClick, addClick, asksClick, flowsClick, settingsClick, taskClick, tasksClick, batchClick, deskClick, memoriesClick, queueClick, inboxClick, reportsClick];
const SUBMITS = [addSubmit, asksSubmit, reportsSubmit, terminalSubmit, taskSubmit];

document.addEventListener("click", (event) => {
  const target = event.target.closest("a, button, [data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action === "retry") return refresh();
  if (action === "sign-out") { event.preventDefault(); return signOut(null); }
  if (CLICKS.some((click) => click(action, target, event))) return;
  const modified = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
  if (target.tagName === "A" && target.origin === location.origin && !target.hash && !target.target && !modified) {
    event.preventDefault();
    go(target.pathname + target.search);
  }
  const inPage = target.origin === location.origin && target.pathname === location.pathname && target.search === location.search;
  if (target.tagName === "A" && inPage && target.hash && !target.target && !modified) {
    event.preventDefault();
    document.querySelector(target.hash)?.scrollIntoView({ behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }
});

document.addEventListener("change", ({ target }) => {
  settingsChange(target);
  costsChange(target);
});

document.addEventListener("input", ({ target }) => {
  if (addInput(target)) return;
  asksInput(target);
  reportsInput(target);
  taskInput(target);
  terminalInput(target);
});

document.addEventListener("submit", (event) => {
  const action = event.target.dataset.action;
  if (SUBMITS.some((submit) => submit(action, event.target))) return event.preventDefault();
  if (action !== "sign-in") return;
  event.preventDefault();
  const token = new FormData(event.target).get("token").trim();
  if (!token) return;
  localStorage.setItem("priory.token", token);
  keepTokenForWorker(token);
  Object.assign(state, { token, signInError: null });
  refresh();
});

// Desktop shortcuts: j and k move, Enter opens, Esc closes. Never inside an input.
document.addEventListener("keydown", (event) => {
  if (terminalKey(event)) return;
  if ((event.key === " " || event.key === "Enter") && event.target.matches?.("[role=radio]")) {
    event.preventDefault();
    return event.target.click();
  }
  if (addSheet().open || answerSheet().open || keyboardSheet().open || event.target.closest("input, textarea, select") || event.metaKey || event.ctrlKey || event.altKey || !state.data) return;
  const rows = [...document.querySelectorAll(".list [href^='/asks/'], .list tr[data-href^='/asks/']")];
  const hrefOf = (row) => row.getAttribute("href") || row.dataset.href;
  const at = rows.findIndex((row) => row.getAttribute("aria-current") === "true");
  if (event.key === "n" || event.key === "N") {
    event.preventDefault();
    openAdd();
  } else if (asksKey(event)) {
    return;
  } else if (event.key === "j" || event.key === "k") {
    const next = rows[Math.min(rows.length - 1, Math.max(0, at + (event.key === "j" ? 1 : -1)))];
    if (next) go(hrefOf(next));
  } else if (event.key === "Enter" && at >= 0 && document.activeElement === document.body) {
    go(hrefOf(rows[at]));
  } else if (queueKey(event)) {
    return;
  } else if (event.key === "Escape" && selectedAsk()) {
    go(state.listUrl);
  }
});

for (const query of ["(min-width: 600px)", "(min-width: 1024px)"]) matchMedia(query).addEventListener("change", render);
document.addEventListener("visibilitychange", () => {
  syncTerminal(document.getElementById("app"));
  if (!document.hidden && state.token) refresh();
});
// Ages read in minutes, so repaint them once a minute even when nothing changes.
setInterval(() => { if (!document.hidden && state.token) refresh(); }, 60000);
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
