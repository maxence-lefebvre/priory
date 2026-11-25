// The legacy parts of the app: what each LegacySlot draws with innerHTML, and the events of the whole
// page. It names no page: each has its own file under entries/.
// Agent text is untrusted: every value goes through `html`, which escapes it.
import { acts, disarmTask } from "./acts.js";
import { landing, openPage, ready, refresh } from "./legacy/data.js";
import { loading, watchSections } from "./legacy/frame.js";
import { focusKey, place, route, routeKey, shape, state, trail } from "./legacy/shared.js";
import { remoteGeneration } from "./legacy/taskacts.js";
import { each, entries, entryOf, first, needsOf } from "./registry.js";
import { go } from "./router.js";
import { signOut } from "./session.js";

// What each legacy element draws, { slot, kind, name, id, panels }, and the page it drew last.
const slots = new Map();
// The elements on screen, in document order.
const hosts = () => [...slots.keys()].sort((a, b) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));

// The focus in an element React takes away before the next draw, such as a panel a resize moves.
let lost = null;
const typed = () => entries.flatMap((entry) => entry.fields ?? []);
const focusOf = (active) => ({ key: focusKey(active), caret: typed().includes(active.id) ? [active.id, active.selectionStart, active.selectionEnd, active.selectionDirection] : null });

/** A LegacySlot shows `element`, which draws `part`; the answer takes it away again. */
export function show(element, part) {
  slots.set(element, part);
  return () => {
    keepScroll(element);
    if (element.contains(document.activeElement)) lost = focusOf(document.activeElement);
    slots.delete(element);
  };
}

/** The first legacy slot of a page is mounted; the answer runs as the last one leaves. */
export function mount() {
  return () => {
    dropObservers();
    state.renderedKey = null;
    lost = null;
    each("left");
    // No legacy page is on screen, so none of its keys or actions finds an ask.
    Object.assign(place, { href: "", entry: {} });
  };
}

/** Takes the place React shows, and the listings its legacy slots read, before they draw. */
export function open(at) {
  Object.assign(place, { href: at.href, entry: at.entry });
  Object.assign(trail, at.trail);
  shape.size = at.size;
  openPage(at.names);
  draw();
}

const dropObservers = () => {
  state.observers.forEach((observer) => observer.disconnect());
  state.observers = [];
};

// The page the element drew, which the last legacy page to leave may have forgotten.
const scrollKey = (element) => `${slots.get(element).slot} ${slots.get(element).drawn}`;
const keepScroll = (element) => {
  // An element React just put there has drawn nothing: its scroll is the one kept before.
  if (slots.get(element).drawn) state.scrolls.set(scrollKey(element), [...element.querySelectorAll(".scroll")].map((part) => part.scrollTop));
};

// A draw that throws is reported and leaves the page as it was: React would unmount the root.
export function draw() {
  try {
    render();
  } catch (error) {
    reportError(error);
  }
}

// One pass over every legacy slot, so they never show two reads, and focus goes from one to another.
export function render() {
  // The sheets live outside #app.
  each("sheet");
  const shown = hosts();
  landing.set(!ready());
  if (!shown.length || !state.token) return;
  each("before");
  shown.forEach(keepScroll);
  const active = document.activeElement;
  const { key: focused, caret } = shown.some((host) => host.contains(active)) ? focusOf(active) : (active === document.body && lost) || {};
  lost = null;
  dropObservers();
  // A Cancel armed on another runner's task holds for the generation the list shows.
  if (typeof state.taskArmed?.task === "string" && remoteGeneration(state.taskArmed.task) !== state.taskArmed.generation) disarmTask();
  // Every part is built before the first one lands: a part that throws leaves them all as they were.
  const drawn = shown.map((host, at) => [host, text(slots.get(host), at)]);
  // The answer bar shows once the options scroll away; a redraw of the same page keeps it shown, so focus on it stays.
  const samePage = state.renderedKey === routeKey();
  for (const [host, html] of drawn) {
    const barShown = samePage && host.querySelector(".abar:not([hidden])") !== null;
    host.innerHTML = html;
    slots.get(host).drawn = routeKey();
    if (barShown) host.querySelector(".abar")?.removeAttribute("hidden");
  }
  state.renderedKey = routeKey();
  state.rendered = Date.now();
  shown.forEach(settle);
  // The Desk table draws some controls twice and CSS hides one copy: focus goes to the shown one.
  const again = (selector) => shown.flatMap((host) => [...host.querySelectorAll(selector)]);
  if (focused) again(focused).find((element) => !element.closest("[inert]") && element.getClientRects().length)?.focus({ preventScroll: true });
  if (caret) again(`#${caret[0]}`)[0]?.setSelectionRange(caret[1], caret[2], caret[3]);
}

// The HTML of a legacy part. Until the page's listings are read, the first part says so and the others
// stay empty; a part whose own listings are read draws, so an ask answers beside a list that failed.
function text({ kind, name, id, panels }, at) {
  const mine = kind !== "loading" && ready(needsOf([{ kind, name, id }], route()[1]));
  if (!mine) return (at && !ready()) || kind === "preview" ? "" : loading().text;
  const size = shape.size;
  const entry = entryOf(name);
  if (kind === "drawer") return entry.drawer(id, size === "wide", panels).text;
  if (kind === "preview") return entry.preview().text;
  if (kind === "detail") return entry.detail(id, size).text;
  return (kind === "list" ? entry.list(size === "wide") : entry.page(size)).text;
}

// What a drawn element needs beside its HTML.
function settle(host) {
  // A render must not restart the countdown; CSP allows no inline style, so it is set here.
  host.querySelectorAll(".drain").forEach((drain) => {
    const armed = entries.map((entry) => entry.drain?.(drain)).find((found) => found !== undefined);
    drain.style.animationDelay = `-${armed ? Date.now() - armed.at : 0}ms`;
  });
  const scrolls = state.scrolls.get(scrollKey(host)) || [];
  host.querySelectorAll(".scroll").forEach((element, i) => { element.scrollTop = scrolls[i] || 0; });
  // A link or control inside a row, such as its key, batch or Move menu, acts on its own.
  for (const row of host.querySelectorAll("[data-href]")) row.addEventListener("click", (event) => { if (!event.target.closest("a, button, details")) go(row.dataset.href); });
  host.querySelectorAll("[data-grow]").forEach((part) => { part.style.flexGrow = part.dataset.grow; });
  watchSections(host);
  each("after", host);
}

// Events: each page's file acts on its own controls. The links are the router's, and React's controls
// carry no data-action but the frame's Add.
document.addEventListener("click", (event) => {
  const target = event.target.closest("a, button, [data-action]");
  if (!target) return;
  const action = target.dataset.action;
  if (action === "retry") return refresh();
  if (action === "sign-out") return signOut(null);
  first("click", action, target, event);
});

document.addEventListener("change", ({ target }) => each("change", target));
document.addEventListener("input", ({ target }) => each("input", target));

document.addEventListener("submit", (event) => {
  if (first("submit", event.target.dataset.action, event.target)) event.preventDefault();
});

// A Say draft is in its field already, and a redraw would replace the field being typed in.
let drawnActs = acts.get();
acts.subscribe(() => {
  const { armed, busy, notices } = acts.get();
  const same = armed === drawnActs.armed && busy === drawnActs.busy && notices === drawnActs.notices;
  drawnActs = acts.get();
  if (!same) draw();
});
