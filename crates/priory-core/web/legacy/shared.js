// What every legacy page draws with: the state, the HTML template, icons, times and facets.
import { runnerOf } from "../asks.js";
import { taskRepo } from "../desk.js";
import { linked } from "../links.js";
import { decoded } from "../pages.js";
import { markdown } from "../markdown.js";
import { prStack } from "../stack.js";
import { leaves, parts, pathOf, searchOf } from "../frame.js";
import { ICONS, STATES } from "../icons.js";
import { filtered, taskLabel } from "../tasks.js";
import { age } from "../times.js";
import { askFor, asked } from "./data.js";
import { place, shape, state } from "./state.js";

export { ICONS, STATES, role } from "../icons.js";
export { post } from "../session.js";
export { place, shape, state, trail } from "./state.js";
export { age, ago, clock, date, today } from "../times.js";

class Raw { constructor(text) { this.text = text; } }
export const raw = (text) => new Raw(text);
const escapeText = (value) => String(value).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const piece = (value) => {
  if (value instanceof Raw) return value.text;
  if (Array.isArray(value)) return value.map(piece).join("");
  if (value === null || value === undefined || value === false) return "";
  return escapeText(value);
};
export function html(strings, ...values) {
  return raw(strings.reduce((out, text, i) => out + text + (i < values.length ? piece(values[i]) : ""), ""));
}
// Agent text with the factory's autolinks; `linked` escapes it.
export const autolinked = (text, repo) => raw(linked(text ?? "", state.data?.links || [], repo ?? null));
// A Desk row's text links `#n` in its task's repository.
export const repoOf = (task) => taskRepo(state.data?.tasks, task);
export const md = (text, repo) => html`<div class="md">${raw(markdown(text, state.data?.links || [], repo ?? null))}</div>`;
export const icon = (name, size = "") => raw(`<svg class="ico ${size}" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`);

export function tile(state, size = "") {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="tile ${kind} ${size}" role="img" aria-label="${state}">${name ? icon(name) : raw('<i class="dot"></i>')}</span>`;
}
export const stateWord = (state) => {
  const [kind, name] = STATES[state] || ["muted", "circle-help"];
  return html`<span class="sti t-${kind}">${name ? icon(name, "s15") : raw('<i class="dot"></i>')}${state}</span>`;
};

export const timeTag = (iso, text = age(iso)) => html`<time datetime="${iso}" title="${iso ? new Date(iso).toString() : ""}">${text}</time>`;

export const route = () => parts(place.href);

export const layout = () => shape.size;

export const view = () => state.data.views.views.default;
export const panelsOf = () => [...new Set(leaves(view().layout))];

// The last row of a list that left out older rows; scrolling to it loads them.
export function olderRow(listing, count, wide, columns = 6) {
  if (!count) return "";
  const text = `Loading older ${listing}…`;
  return wide ? html`<tr class="grp" data-older="${listing}"><td colspan="${columns}">${text}</td></tr>` : html`<p class="empty" data-older="${listing}">${text}</p>`;
}

// "3 PRs" for a task with a stack of two or more. The queue and desk carry no stacks; the tasks listing does.
export const prs = (task, id = task.id) => prStack(task.stack ?? state.data.tasks.find((each) => each.id === id)?.stack).label;
export const prsTag = (task, id) => {
  const label = prs(task, id);
  return label ? html`<span class="tag">${label}</span>` : "";
};
// While an ask is open, the list keeps the view it was opened from.
export const listSearch = () => (selectedAsk() ? new URL(state.listUrl, location.origin).search : searchOf(place.href));

export function savedViews(list, base, facets = "") {
  return html`<nav class="views" aria-label="Saved views">${list.map(([key, label, count, on]) =>
    html`<a href="${base}?view=${key}${facets}" aria-current="${on ? "true" : "false"}">${label}${count === null ? "" : html`<span class="num">${count}</span>`}</a>`)}</nav>`;
}

export const olderTasks = () => state.data.older.tasks;
export const listOrEmpty =(rows, empty) => (rows.length ? rows : html`<p class="empty">${empty}</p>`);

// Each facet's key, label, an ask's values and the order the picker offers them. An ask with no
// band, such as an outside ask, shows under `none`, and so does an ask with no risk.
const FACETS = [
  ["band", "Band", (ask) => [ask.band ?? "none"], () => [...(state.data.bands || []), "none"]],
  ["risk", "Risk", (ask) => (ask.risk?.length ? ask.risk : ["none"]), () => ["irreversible", "high", "medium", "low", "red", "yellow", "green", "none"]],
  ["runner", "Runner", (ask) => [ask.runner ?? "none"], null, (name) => (state.data.asks.some((ask) => ask.runner === name && !runnerOf(ask)) ? `${name} (here)` : name)],
];
// The Runner facet shows only in a factory of several runners, where every ask names one.
export const shownFacets = () => FACETS.filter(([key]) => key !== "runner" || state.data.asks.some((ask) => ask.runner));
export const facetPicks = (facets = FACETS) => {
  const search = new URLSearchParams(listSearch());
  return new Map(facets.map(([key]) => [key, search.get(key)]).filter(([, value]) => value));
};
export const faceted = (asks, picks) => filtered(asks, picks, FACETS);
export const facetQuery = (picks) => [...picks].map(([key, value]) => `&${key}=${encodeURIComponent(value)}`).join("");

// A facet chip of the list at `base` over `items`; a facet with no order offers its values A to Z.
export function facet(view, picks, [key, label, values, order, named = (name) => name], base = "/asks", items = state.data.asks) {
  const others = new Map([...picks].filter(([other]) => other !== key));
  const picked = picks.get(key);
  if (picked) return html`<a class="facet" aria-current="true" href="${base}?view=${view}${facetQuery(others)}" aria-label="${label}: ${picked}. Clear">${picked}${icon("x", "s13")}</a>`;
  const counts = new Map();
  for (const item of items) for (const value of values(item)) counts.set(value, (counts.get(value) || 0) + 1);
  const offered = [...counts].sort(([a], [b]) => (order ? order().indexOf(a) - order().indexOf(b) : a.localeCompare(b)));
  return html`<details class="facet"><summary aria-label="Filter by ${key}">${label}${icon("chevron-down", "s13")}</summary>
    <div class="pick">${offered.map(([name, count]) => html`<a href="${base}?view=${view}${facetQuery(others)}&${key}=${encodeURIComponent(name)}">${named(name)}<span class="num">${count}</span></a>`)}</div></details>`;
}

// The saved view the URL names, else the list's first.
export const listView = (views) => {
  const name = new URLSearchParams(searchOf(place.href)).get("view");
  return views.includes(name) ? name : views[0];
};

// Tasks (DESIGN.md, Tasks panel): every task, by saved view and facets; each row opens its page.
// A task by its id, named by its key once the task list holds it.
export const taskName = (id) => {
  const task = state.data?.tasks?.find((each) => each.id === id);
  return task ? taskLabel(task.key) : `task ${id}`;
};

// Ask detail
export const selectedAsk = () => {
  const [first, id] = route();
  return first === "asks" && id ? decoded(id) : null;
};

// An id no other control's parts can spell: whitespace and quotes are escaped away.
export const focusId = (...parts) => encodeURIComponent(JSON.stringify(parts));

// Why a write did not go; `refused` words a refusal the daemon gave.
export function writeRefusal(status, body, refused = () => `Not sent: ${body.error || `the server answered ${status}`}.`) {
  if (status === 503) return "Could not send: no priory daemon runs beside this server. Start priory daemon, then send again.";
  if (status === 504 || status === 0) return "The daemon may have applied this but did not confirm it. Check the report before you send again.";
  return refused();
}

export function current() {
  const [first] = route();
  if (!first || first === "index.html") return "desk";
  if (first === "settings") return "settings";
  return first;
}

export const routeKey = () => pathOf(place.href) + searchOf(place.href);
const FOCUS_ATTRIBUTES = ["id", "href", "data-option", "data-add-chip", "data-key", "data-action"];
export function focusKey(element) {
  if (!element || element === document.body) return null;
  const name = FOCUS_ATTRIBUTES.find((attribute) => element.hasAttribute(attribute));
  return name ? `[${name}="${CSS.escape(element.getAttribute(name))}"]` : null;
}

export const track = (observer) => (state.observers.push(observer), observer);

export function loadOlder(listing) {
  // A render may call this: the page reads the older rows once it has drawn.
  setTimeout(() => askFor({ all: { ...asked.get().all, [listing]: true } }));
}
