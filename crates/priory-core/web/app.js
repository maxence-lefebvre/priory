// The legacy pages: what LegacyPage draws with innerHTML, and their events. A port deletes its page's
// file under legacy/ and its lines here.
// Agent text is untrusted: every value goes through `html`, which escapes it.
import { acts, disarmTask } from "./acts.js";
import { where, plain, shownPanels } from "./frame.js";
import { addClick, addInput, addSheet, addSubmit, openAdd, renderAdd } from "./legacy/add.js";
import { answerSheet, askDetail, askOnScreen, asksClick, asksInput, asksKey, asksPhone, asksSubmit, disarm, drawer, renderSheet } from "./legacy/asks.js";
import { batchClick, batchPage, drawLanes } from "./legacy/batch.js";
import { costsChange, costsClick, costsPage, loadCosts } from "./legacy/costs.js";
import { openPage, ready, refresh } from "./legacy/data.js";
import { deskClick, deskPhone, seen } from "./legacy/desk.js";
import { flowsClick, flowsPage } from "./legacy/flows.js";
import { OWN_PAGE_LISTS, connection, laterPanel, listPanel, loading, tree, watchSections } from "./legacy/frame.js";
import { inboxClick, inboxViews, unroutedRows } from "./legacy/inbox.js";
import { disarmMemory, memoriesClick, memoriesPage } from "./legacy/memories.js";
import { queueClick, queueKey } from "./legacy/queue.js";
import { reportsClick, reportsInput, reportsPage, reportsSubmit } from "./legacy/reports.js";
import { morePage, settingsChange, settingsClick, settingsScreen } from "./legacy/settings.js";
import { current, focusKey, html, icon, layout, place, routeKey, selectedAsk, shape, state, trail, view } from "./legacy/shared.js";
import { remoteGeneration, taskClick, taskInput, taskSubmit } from "./legacy/taskacts.js";
import { tasksClick } from "./legacy/tasks.js";
import { keyboardSheet, syncTerminal, terminalClick, terminalInput, terminalKey, terminalSubmit } from "./legacy/terminal.js";
import { PANELS, TYPED_FIELDS } from "./pages.js";
import { go } from "./router.js";
import { signOut } from "./session.js";

// The element a legacy page draws in, while LegacyPage shows one.
let host = null;

/** LegacyPage shows `element`; the answer takes it away again. */
export function mount(element) {
  host = element;
  return () => {
    keepScrolls();
    state.observers.forEach((observer) => observer.disconnect());
    state.observers = [];
    state.renderedKey = null;
    host = null;
    disarm();
    disarmMemory();
    Object.assign(state.planEdit, { open: null, armed: null });
  };
}

/** Takes the place React shows, and the listings it reads, before the page draws. */
export function open(at) {
  Object.assign(place, { href: at.href, entry: at.entry });
  Object.assign(trail, at.trail);
  shape.size = at.size;
  openPage(at.names);
  draw();
}

const keepScrolls = () => {
  if (host && state.renderedKey) state.scrolls.set(state.renderedKey, [...host.querySelectorAll(".scroll")].map((element) => element.scrollTop));
};

// A draw that throws is reported and leaves the page as it was: React would unmount the root.
export function draw() {
  try {
    render();
  } catch (error) {
    reportError(error);
  }
}

export function render() {
  // The sheets live outside #app; the add sheet redraws when the connection drops or comes back under it.
  if (addSheet().open && state.add.shownOffline !== Boolean(state.failed)) renderAdd();
  if (answerSheet().open && state.data) renderSheet();
  if (state.reportPending && !state.reportPending.sending && (!state.token || !host || current() !== "reports")) {
    clearTimeout(state.reportPending.timer);
    state.reportPending = null;
  }
  const app = host;
  if (!app || !state.token) return;
  // Leaving the page closes an entry's menu and disarms its Drop.
  if (state.renderedKey && state.renderedKey !== routeKey()) Object.assign(state.planEdit, { open: null, armed: null });
  keepScrolls();
  const focused = focusKey(document.activeElement);
  const field = TYPED_FIELDS.includes(document.activeElement?.id) ? document.activeElement : null;
  const caret = field ? [field.id, field.selectionStart, field.selectionEnd, field.selectionDirection] : undefined;
  state.observers.forEach((observer) => observer.disconnect());
  state.observers = [];
  // A new revision is a new question: a tap armed on the old one must not run it.
  if (state.armed && (selectedAsk() !== String(state.armed.id) || askOnScreen()?.revision !== state.armed.revision)) disarm();
  // Likewise a Cancel armed on another runner's task at a generation the list no longer shows.
  if (typeof state.taskArmed?.task === "string" && remoteGeneration(state.taskArmed.task) !== state.taskArmed.generation) disarmTask();
  if (state.memoryArmed && current() !== "memories") disarmMemory();
  // The answer bar shows once the options scroll away; a redraw keeps it shown, so focus on it stays.
  const barShown = app.querySelector(".abar:not([hidden])") !== null;
  app.innerHTML = page().text;
  if (barShown) app.querySelector(".abar")?.removeAttribute("hidden");
  // A render must not restart the countdown; CSP allows no inline style, so it is set here.
  app.querySelectorAll(".drain").forEach((drain) => {
    const armed = drain.closest("[data-action=memory-act]") ? state.memoryArmed : drain.closest(".act") ? state.taskArmed : state.armed;
    drain.style.animationDelay = `-${armed ? Date.now() - armed.at : 0}ms`;
  });
  state.renderedKey = routeKey();
  state.rendered = Date.now();
  const scrolls = state.scrolls.get(state.renderedKey) || [];
  app.querySelectorAll(".scroll").forEach((element, i) => { element.scrollTop = scrolls[i] || 0; });
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
}

function page() {
  const size = layout();
  const { here, ask, batch } = where(place.href);
  const list = ask ? state.list : here;
  if (!state.data || !ready()) return loading();
  if (size === "compact") {
    if (ask) return askDetail(ask, false);
    if (here === "desk") return deskPhone();
    if (here === "asks") return asksPhone();
    if (OWN_PAGE_LISTS[here]) {
      const { views, rows } = OWN_PAGE_LISTS[here](false);
      return html`<div class="top"><h1>${PANELS[here][0]}</h1><span class="sp"></span>${connection()}</div>
        <div class="panel" role="region" aria-label="${PANELS[here][0]}"><div class="scroll">${views}${rows}</div></div>`;
    }
    if (here === "inbox") return html`<div class="top"><h1>Inbox</h1><span class="sp"></span>${connection()}</div>
      <div class="panel" role="region" aria-label="Inbox"><div class="scroll">${inboxViews()}${unroutedRows()}</div></div>`;
    if (here === "more") return morePage();
    if (batch) return batchPage(batch);
    if (here === "memories") return memoriesPage();
    if (here === "reports") return reportsPage(true);
    if (here === "costs") return costsPage(true);
    if (here === "workflow") return flowsPage(true);
    if (here === "settings") return html`<div class="top"><a class="back" href="/more">${icon("chevron-left", "s22")}More</a></div>${settingsScreen()}`;
    return html`<div class="top"><h1>${PANELS[here][0]}</h1></div>${laterPanel(here)}`;
  }
  const utility = batch ? batchPage(batch) : here === "memories" ? memoriesPage() : here === "reports" ? reportsPage(false) : here === "costs" ? costsPage(false) : here === "workflow" ? flowsPage(false) : here === "settings" ? settingsScreen() : here === "more" ? morePage() : null;
  if (utility) return html`<div class="list ${batch || here === "memories" ? "page" : ""}">${utility}</div>`;
  const wide = size === "wide";
  if (OWN_PAGE_LISTS[list]) return listPanel(list, wide);
  if (!wide) return html`${listPanel(list, false)}${drawer(false, [list])}`;
  const layoutNode = view().layout;
  if (plain(layoutNode) && list === "inbox") return listPanel(list, true);
  return html`${plain(layoutNode) ? listPanel(list, true) : tree(layoutNode, list)}${drawer(true, plain(layoutNode) ? [list] : shownPanels(layoutNode, list))}`;
}

// Events: each page's file acts on its own controls. The links are the router's.
const CLICKS = [terminalClick, costsClick, addClick, asksClick, flowsClick, settingsClick, taskClick, tasksClick, batchClick, deskClick, memoriesClick, queueClick, inboxClick, reportsClick];
const SUBMITS = [addSubmit, asksSubmit, reportsSubmit, terminalSubmit, taskSubmit];

document.addEventListener("click", (event) => {
  const target = event.target.closest("a, button, [data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action === "retry") return refresh();
  if (action === "sign-out") return signOut(null);
  CLICKS.some((click) => click(action, target, event));
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
  if (SUBMITS.some((submit) => submit(action, event.target))) event.preventDefault();
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

document.addEventListener("visibilitychange", () => syncTerminal(document.getElementById("app")));
acts.subscribe(draw);
