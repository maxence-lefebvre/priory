// Priory's web view (DESIGN.md): the frame, the Desk and the Asks, with answering through the CLI's verbs.
// Agent text is untrusted: every value goes through `html`, which escapes it.
import { optionKey, snoozeShortcut, writeShown } from "./asks.js";
import { arrivalAsk } from "./desk.js";
import { addButton, addSheet, closeAdd, discardAdd, keepDraft, openAdd, renderAdd, submitAdd } from "./legacy/add.js";
import { act, answer, answerSheet, askDetail, askOnScreen, asksPhone, disarm, drawer, lockOf, renderSheet, snooze } from "./legacy/asks.js";
import { batchPage, drawLanes, editPlan, planEditAt, planEditOf } from "./legacy/batch.js";
import { costsPage, loadCosts } from "./legacy/costs.js";
import { keepTokenForWorker, refresh, signOut } from "./legacy/data.js";
import { deskPhone, deskView, markDone, openAsks, rowAct, seen } from "./legacy/desk.js";
import { flowsPage } from "./legacy/flows.js";
import { OWN_PAGE_LISTS, connection, laterPanel, listPanel, loading, notFound, shownPanels, signIn, staleStrip, tabs, tree, watchSections } from "./legacy/frame.js";
import { inboxViews, requestTask, subscribeTask, unroutedRows } from "./legacy/inbox.js";
import { disarmMemory, memoriesPage, memoryAct } from "./legacy/memories.js";
import { moveTask } from "./legacy/queue.js";
import { reportAct, reportsPage, submitRemark } from "./legacy/reports.js";
import { chooseKind, morePage, settingsScreen, turnPush } from "./legacy/settings.js";
import { current, focusKey, html, icon, layout, route, routeKey, selectedAsk, state, view } from "./legacy/shared.js";
import { taskPage } from "./legacy/task.js";
import { disarmTask, remoteBody, remoteGeneration, taskAct, taskIdOf, taskWrite } from "./legacy/taskacts.js";
import { copyTerminal, keyboard, keyboardAct, keyboardSheet, observeTerminal, openKeyboardSheet, paintTerminal, screenText, sendInput, syncTerminal, terminal, watch } from "./legacy/terminal.js";
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

// Events
document.addEventListener("click", (event) => {
  const target = event.target.closest("a, button, [data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action === "retry") return refresh();
  if (action === "term-fit") return (terminal.fit = !terminal.fit), paintTerminal(document.getElementById("app"));
  if (action === "term-copy") return watch.state().screen && copyTerminal(screenText(watch.state().screen), "screen");
  if (action === "term-earlier") return watch.earlier();
  if (action === "term-type") {
    Object.assign(terminal, { typing: !terminal.typing, said: null });
    paintTerminal(document.getElementById("app"), true);
    return terminal.typing && document.getElementById("term-line")?.focus();
  }
  if (action === "term-attach") return copyTerminal(`priory attach ${target.dataset.task} --read-only`, "attach");
  if (action === "term-menu") return (terminal.menu = !terminal.menu), paintTerminal(document.getElementById("app"), true);
  if (action === "term-take-ask" || action === "term-give-ask") {
    terminal.menu = false;
    paintTerminal(document.getElementById("app"), true);
    return openKeyboardSheet(action === "term-take-ask" ? "take" : "give");
  }
  if (action === "term-take" || action === "term-give") return keyboardAct(action);
  if (action === "keyboard-close") return keyboardSheet().close();
  if (action === "term-key") return sendInput({ key: target.dataset.key });
  // A refresh that reaches the daemon reads the costs again.
  if (action === "costs-retry") return (state.costs.error = null), refresh();
  if (action === "add-open") return openAdd();
  if (action === "add-close" || action === "add-done") return closeAdd();
  if (action === "sheet-close") return answerSheet().close();
  if (action === "add-discard") return discardAdd();
  if (action === "add-another") {
    Object.assign(state.add, { phase: "empty", task: null });
    renderAdd();
    return document.getElementById("add-text")?.focus();
  }
  if (action === "flows-retry") { state.flowsError = null; return render(); }
  if (action === "sign-out") { event.preventDefault(); return signOut(null); }
  if (action === "push-on" || action === "push-off") return turnPush(action === "push-on");
  if (action === "install" && state.install) {
    state.install.prompt();
    state.install = null;
    return render();
  }
  if (action === "task-act") return taskAct(target);
  if (action === "remote-open" && !event.target.closest("form, .task-acts")) {
    state.remoteOpen = state.remoteOpen === target.dataset.task ? null : target.dataset.task;
    return render();
  }
  if (action === "entry-menu" && state.batch?.body) {
    const edit = planEditOf(state.batch.body);
    const open = edit.open === target.dataset.entry ? null : target.dataset.entry;
    state.planEdit = { ...edit, at: planEditAt(state.batch.body), batch: state.batch.body.id, open, armed: null };
    return render();
  }
  if (action === "batch-edit") return editPlan(target);
  if (action === "todo-done") return markDone(target);
  if (action === "row-act") return rowAct(target);
  if (action === "memory-act") return memoryAct(target);
  if (action === "memory-current") {
    event.preventDefault();
    const { id } = target.dataset;
    if (!state.memoryCurrent.delete(id)) state.memoryCurrent.add(id);
    return render();
  }
  if (action === "move") return moveTask(target);
  if (action === "request-task") return requestTask(target.dataset.text, target.dataset.workflow);
  if (action === "unheard-say") {
    if (target.getAttribute("aria-disabled") !== "true") taskWrite(Number(target.dataset.task), "say", { text: target.dataset.text });
    return;
  }
  if (action === "subscribe-task") {
    if (state.unrouted.busy === null) subscribeTask(target.dataset.resource, Number(target.dataset.task));
    return;
  }
  if (["pick", "pick-undo", "pick-change", "pick-keep", "report-go", "report-next", "report-search", "compose", "compose-cancel", "reply", "resolve", "reopen"].includes(action)) return reportAct(target);
  if (target.dataset.answer !== undefined || ["options", "write", "dismiss", "snooze", "unsnooze"].includes(action)) return act(target);
  if (action === "mode" || action === "theme") {
    const root = document.documentElement;
    if (action === "mode") root.dataset.mode = target.dataset.pickMode;
    else root.dataset.theme = target.dataset.themePick;
    localStorage.setItem("priory.mode", root.dataset.mode);
    localStorage.setItem("priory.theme", root.dataset.theme);
    return render();
  }
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

// A Move menu's toggle does not bubble, so it is caught on its way down.
document.addEventListener("toggle", (event) => {
  if (!event.target.matches?.("details.move")) return;
  const task = Number(event.target.dataset.move);
  if (event.target.open) state.moveOpen = task;
  else if (state.moveOpen === task) state.moveOpen = null;
}, true);

document.addEventListener("change", (event) => {
  const kind = event.target.dataset.pushKind;
  if (kind) return chooseKind(kind, event.target.checked);
  if (event.target.dataset.action !== "costs-by") return;
  state.costs.by = event.target.value;
  loadCosts();
});

document.addEventListener("input", (event) => {
  if (event.target.id === "add-text" || event.target.dataset.addChip) {
    state.add.draft[event.target.dataset.addChip || "text"] = event.target.value;
    keepDraft();
    if (event.target.dataset.addChip === "workflow") {
      state.add.swapped = null;
      document.getElementById("add-swapped")?.remove();
    }
    // Only the button's state changes; a full redraw would reset the caret on every letter.
    addSheet().querySelector("form .btn.primary")?.setAttribute("aria-disabled", String(!state.add.draft.text.trim() || state.add.phase === "sending"));
    return;
  }
  const ask = event.target.id === "draft" && askOnScreen();
  if (ask) state.drafts.set(ask.id, event.target.value);
  if (event.target.id === "report-draft" && state.compose) state.compose.text = event.target.value;
  if (event.target.id === "report-query") {
    state.reportQuery = event.target.value;
    render();
  }
  const form = event.target.id === "say" && event.target.closest("form");
  if (form) state.sayDrafts.set(taskIdOf(form.dataset.task), event.target.value);
  if (event.target.id === "term-keys") terminal.keys = event.target.value;
  if (event.target.id === "term-line") {
    terminal.line = event.target.value;
    // Only the button changes; a repaint per letter would cost the caret.
    event.target.form.querySelector(".send").setAttribute("aria-disabled", String(!terminal.line.trim() || terminal.saying));
  }
});

document.addEventListener("submit", (event) => {
  if (event.target.dataset.action === "add") {
    event.preventDefault();
    return submitAdd();
  }
  if (event.target.dataset.action === "send-text") {
    event.preventDefault();
    const ask = askOnScreen();
    const text = (state.drafts.get(ask?.id) || "").trim();
    if (ask && text && writeShown(state.writing.has(ask.id), lockOf(ask), Boolean(state.pending)) === "form") answer(ask, text, true);
    return;
  }
  if (event.target.dataset.action === "remark") {
    event.preventDefault();
    return submitRemark();
  }
  if (event.target.dataset.action === "term-paste") {
    event.preventDefault();
    return terminal.keys && sendInput({ text: terminal.keys });
  }
  if (event.target.dataset.action === "term-say") {
    event.preventDefault();
    return sayFromPane(Number(event.target.dataset.task));
  }
  if (event.target.dataset.action === "say") {
    event.preventDefault();
    const task = taskIdOf(event.target.dataset.task);
    const text = (state.sayDrafts.get(task) || "").trim();
    if (text && state.taskBusy === null) taskWrite(task, "say", remoteBody(task, { text }));
    return;
  }
  if (event.target.dataset.action !== "sign-in") return;
  event.preventDefault();
  const token = new FormData(event.target).get("token").trim();
  if (!token) return;
  localStorage.setItem("priory.token", token);
  keepTokenForWorker(token);
  Object.assign(state, { token, signInError: null });
  refresh();
});

// Desktop shortcuts: j and k move, Enter opens, Esc closes, S snoozes the selected ask for 1 h. Never inside an input.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && terminal.menu && event.target.closest?.(".term")) {
    terminal.menu = false;
    paintTerminal(document.getElementById("app"), true);
    return document.querySelector('[data-action="term-menu"]')?.focus();
  }
  if (event.key === "Escape" && event.target.id === "term-line") {
    Object.assign(terminal, { typing: false, said: null });
    paintTerminal(document.getElementById("app"), true);
    return document.querySelector('[data-action="term-type"]')?.focus();
  }
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
  } else if (event.key === "s" || event.key === "S") {
    const body = askOnScreen() && !state.failed && !state.pending && snoozeShortcut(askOnScreen(), Date.now());
    if (body) snooze(askOnScreen(), { dataset: { for: body.for } });
  } else if (selectedAsk() && /^[1-9]$/.test(event.key)) {
    const index = optionKey(event);
    if (index !== null) document.querySelector(`.opt[data-answer="${index}"]`)?.click();
  } else if (event.key === "j" || event.key === "k") {
    const next = rows[Math.min(rows.length - 1, Math.max(0, at + (event.key === "j" ? 1 : -1)))];
    if (next) go(hrefOf(next));
  } else if (event.key === "Enter" && at >= 0 && document.activeElement === document.body) {
    go(hrefOf(rows[at]));
  } else if (event.key === "Escape" && document.querySelector("details.move[open]")) {
    const menu = document.querySelector("details.move[open]");
    menu.open = false;
    menu.querySelector("summary").focus();
  } else if (event.key === "Escape" && selectedAsk()) {
    go(state.listUrl);
  }
});

// A tap on the scrim, outside the sheet's box, closes it; closing hands focus back to an Add button.
addSheet().addEventListener("click", (event) => { if (event.target === addSheet()) closeAdd(); });
answerSheet().addEventListener("click", (event) => { if (event.target === answerSheet()) answerSheet().close(); });
// A page that goes away gives its keyboard back, so the run's outbox does not wait out the 15 minutes.
addEventListener("pagehide", () => keyboard.leave());
keyboardSheet().addEventListener("click", (event) => { if (event.target === keyboardSheet()) keyboardSheet().close(); });
keyboardSheet().addEventListener("close", () => document.querySelector("#app [data-action='term-menu']")?.focus());
addSheet().addEventListener("close", () => document.querySelector("#app [data-action='add-open']")?.focus());
for (const query of ["(min-width: 600px)", "(min-width: 1024px)"]) matchMedia(query).addEventListener("change", render);
addEventListener("resize", () => { if (terminal.fit) paintTerminal(document.getElementById("app")); });
document.addEventListener("visibilitychange", () => {
  syncTerminal(document.getElementById("app"));
  if (!document.hidden && state.token) refresh();
});
// Ages read in minutes, so repaint them once a minute even when nothing changes.
setInterval(() => { if (!document.hidden && state.token) refresh(); }, 60000);
// The browser offers to install only once the page handles this; the Notifications screen shows it.
addEventListener("beforeinstallprompt", (event) => {
  event.preventDefault();
  state.install = event;
  render();
});
if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
