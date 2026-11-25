// What the frame draws around a page: where an address leads, the listings it shows, the list a
// detail was opened from, and the shell that fits the screen.
import { arrivalAsk } from "./desk.js";
import { PANELS } from "./pages.js";

export const pathOf = (href) => href.split(/[?#]/)[0];
export const searchOf = (href) => /\?[^#]*/.exec(href)?.[0] ?? "";
export const parts = (href) => pathOf(href).split("/").filter(Boolean);

/** Where `href` leads: the page, and the ask, task or batch it opens. */
export function where(href) {
  const [first, id] = parts(href);
  const here = !first || first === "index.html" ? "desk" : first;
  const number = /^\d+$/.test(id || "") ? id : null;
  return {
    here,
    ask: here === "asks" && id ? decodeURIComponent(id) : null,
    task: here === "tasks" && number ? Number(number) : null,
    batch: here === "batches" && number ? number : null,
  };
}

export function leaves(node) {
  if (node.panel) return [node.panel];
  return Object.values(node).flat().flatMap(leaves);
}

export const shownTab = (children, list) => children.find((child) => leaves(child).includes(list)) || children[0];

// The panels the view file's tree draws: each tabs node shows one child.
export function shownPanels(node, list) {
  if (node.panel) return [node.panel];
  const [kind, children] = Object.entries(node)[0];
  return kind === "tabs" ? shownPanels(shownTab(children, list), list) : children.flatMap((child) => shownPanels(child, list));
}

export const plain = (layout) => Boolean(layout.tabs?.every((child) => child.panel));

// Lists whose rows open a page of their own, so they take the whole width, with no drawer.
const OWN_PAGE = ["tasks", "runs", "queue"];
// Pages with neither a list nor a drawer.
const UTILITY = ["memories", "reports", "costs", "workflow", "settings", "more"];

/**
 * The shell around the page: the class of its grid, the navigation entry that is current, and the
 * list panels it draws.
 */
export function shellOf(at, { size, list, layout }) {
  const { here, ask, task, batch } = at;
  const detail = Boolean(task || batch);
  const entry = batch ? "tasks" : here === "memories" ? "desk" : here;
  if (size === "compact") return { nav: ask ? null : here === "settings" ? "more" : entry, panels: ask || detail ? [] : [here] };
  const shown = ask ? list : here;
  const nav = here === "settings" ? (size === "wide" ? "settings" : "more") : batch || here === "memories" ? entry : shown;
  if (detail || UTILITY.includes(here) || !PANELS[shown]) return { grid: `${size} no-drawer`, nav, panels: [] };
  if (OWN_PAGE.includes(shown)) return { grid: `${size} no-drawer`, nav, panels: [shown] };
  if (size === "medium") return { grid: "medium", nav, panels: [shown] };
  // Events open no detail, so a wide Inbox takes the drawer's width too.
  if (plain(layout) && shown === "inbox") return { grid: "wide no-drawer", nav, panels: ["inbox"] };
  return { grid: "wide", nav, panels: plain(layout) ? [shown] : shownPanels(layout, shown) };
}

const DESK = ["asks", "queue", "tasks"];
const PANEL_NEEDS = { desk: DESK, asks: ["asks"], tasks: ["tasks"], runs: ["runs"], queue: ["queue", "tasks"], inbox: ["tasks"], terminal: ["tasks"], reports: ["report", "reports", "tasks"] };

/** The listings a page shows beside the frame's own: those of its panels, and its detail's. */
export function needs({ here, ask, task, batch }, { panels }, settings = "appearance") {
  const wanted = new Set(panels.flatMap((panel) => PANEL_NEEDS[panel] ?? []));
  if (PANEL_NEEDS[here] && !task && !ask) PANEL_NEEDS[here].forEach((name) => wanted.add(name));
  if (ask) ["asks", "tasks"].forEach((name) => wanted.add(name));
  if (task) ["asks", "tasks"].forEach((name) => wanted.add(name));
  if (batch) wanted.add("tasks");
  // The theme previews draw the Desk.
  if (here === "settings" && settings !== "notifications") DESK.forEach((name) => wanted.add(name));
  return [...wanted].sort();
}

/** The list a person came from, as it stood before they opened anything. */
export const START = { list: "desk", listUrl: "/", from: null };

/**
 * The trail after a visit to `href`, whose history entry holds `entry`: an ask keeps the list it was
 * opened from, so Back to it restores that list, and a detail page its back link's.
 */
export function step(trail, href, entry) {
  const { here, ask, task } = where(href);
  const key = pathOf(href) + searchOf(href);
  let next = trail;
  if (["desk", "asks"].includes(here) && !ask) next = { ...next, list: here, listUrl: key };
  else if (ask && entry?.listUrl) next = { ...next, list: entry.list, listUrl: entry.listUrl };
  if (PANELS[here] && !ask && !task) next = { ...next, from: key };
  return next;
}

export const DESK_VIEWS = ["needs", "running", "queue", "asleep", "all"];

/** The ask a wide Desk opens in its drawer as the person arrives at `href`, else null. */
export function arrive(desk, { href, from, list }) {
  if (where(href).here !== "desk") return null;
  const view = new URLSearchParams(searchOf(href)).get("view");
  const asks = desk.items.filter((item) => item.kind === "ask");
  return arrivalAsk({ from, list, view: DESK_VIEWS.includes(view) ? view : "needs", asks });
}

/** What a new history entry for `href` holds. */
export function entryOf(trail, href) {
  const { ask, task, batch } = where(href);
  if (ask) return { list: trail.list, listUrl: trail.listUrl };
  return task || batch ? { from: trail.from } : {};
}
