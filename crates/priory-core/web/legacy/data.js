// What the legacy pages draw from: the listings of the page on screen, landed together.
import { draw } from "../app.js";
import { cache, deskReadAt, refresh as refreshQueries } from "../data.js";
import { mergeSince } from "../desk.js";
import { keyOf, params } from "../queries.js";
import { onSignOut, session } from "../session.js";
import { store } from "../store.js";
import { RUN_VIEWS } from "../runs.js";
import { TASK_VIEWS } from "../tasks.js";
import { askView } from "./asks.js";
import { loadCosts } from "./costs.js";
import { deskView } from "./desk.js";
import { loadUnrouted } from "./inbox.js";
import { acceptReport } from "./reports.js";
import { current, listView, selectedAsk, state } from "./shared.js";
import { keyboard, watch } from "./terminal.js";

// Which rows and which report the page asks the server for.
export const asked = store(params());
export const askFor = (part) => asked.set({ ...asked.get(), ...part });

// The listings the page on screen shows, and those landed last, by name.
const page = { names: [], landed: {}, stamp: null };

export function openPage(names) {
  page.names = names;
  // Older rows load again when the person scrolls back to them.
  const history = {
    asks: Boolean(selectedAsk()) || (current() === "asks" && ["answered", "all"].includes(askView())),
    tasks: (current() === "desk" && deskView() === "all") || (current() === "tasks" && listView(TASK_VIEWS) === "ended"),
    runs: current() === "runs" && listView(RUN_VIEWS) === "recent",
  };
  const { all } = asked.get();
  const left = Object.fromEntries(Object.entries(all).map(([name, on]) => [name, on && history[name]]));
  if (Object.keys(all).some((name) => all[name] !== left[name])) askFor({ all: left });
  land();
}

// Whether every listing of the page has data: the page draws a loading line until then.
export const ready = () => page.names.every((name) => name in page.landed);

const drawn = ({ desk, queue, report, reports, asks, tasks, runs, older }) => [desk, queue, report, reports, asks, tasks, runs, older.asks, older.tasks, older.runs];

// Lands the page's listings in one step, once none of them is being read: a draw never mixes two reads.
export function land() {
  const queries = cache.getQueryCache();
  const held = [...new Set(["views", "links", "desk", ...page.names])].map((name) => [name, queries.find({ queryKey: keyOf(name, asked.get()), exact: true })]);
  // A read in flight, and one that failed, change nothing that has landed before.
  const settled = !held.some(([, query]) => query?.state.fetchStatus === "fetching" || query?.state.status === "error");
  for (const query of queries.getAll()) {
    const [name] = query.queryKey;
    // A listing not landed yet draws from what the cache keeps, while it is read again.
    if (query.state.data === undefined || (!settled && name in page.landed)) continue;
    const key = keyOf(name, asked.get());
    // A listing this page does not show keeps its last read, whichever rows that held.
    if (!page.names.includes(name) ? !(name in page.landed) || query.state.dataUpdatedAt >= page.landed[name].at : JSON.stringify(key) === JSON.stringify(query.queryKey)) page.landed[name] = { data: query.state.data, at: query.state.dataUpdatedAt };
  }
  const of = (name) => page.landed[name]?.data;
  if (!of("views") || !of("desk")) return;
  const stamp = JSON.stringify([page.names, held.map(([, query]) => query?.state.dataUpdatedAt)]);
  const fresh = settled && stamp !== page.stamp;
  if (settled) page.stamp = stamp;
  accept({ of, fresh });
}

// Takes the landed listings into state, and draws them when one changed. A listing that did not
// change is the same object, so a read with nothing new only repaints the ages, once a minute at most.
function accept({ of, fresh }) {
  const desk = of("desk");
  // A desk kept on this device names no revision: the visit opens on the first live read.
  if (desk.revision !== undefined) {
    if (!state.since.opened) Object.assign(state.since, { opened: deskReadAt(), from: desk.last_visit });
    state.since.events = mergeSince(state.since.events, desk.since_you_looked);
  }
  const [asksList, tasksList, runsList] = ["asks", "tasks", "runs"].map(of);
  const before = state.data;
  state.data = {
    views: of("views"), links: of("links") ?? [], desk,
    asks: asksList?.asks ?? NONE, bands: asksList?.bands,
    queue: of("queue") ?? EMPTY_QUEUE,
    tasks: tasksList?.tasks ?? NONE, runs: runsList?.runs ?? NONE,
    older: { asks: asksList?.older, tasks: tasksList?.older, runs: runsList?.older },
    report: of("report") ?? null, reports: of("reports") ?? null,
    // The Costs and Flows panels read their own; a refresh keeps the last ones.
    spend: of("spend") ?? null, flows: of("flows") ?? null,
  };
  if (fresh && ready()) {
    acceptReport();
    if (state.task) state.task.stale = true;
    if (state.batch) state.batch.stale = true;
    if (current() === "costs") loadCosts();
    if (current() === "inbox") loadUnrouted();
  }
  const same = before && drawn(before).every((part, i) => part === drawn(state.data)[i]);
  if (!same || Date.now() - state.rendered >= 60000) draw();
}

const NONE = [];
const EMPTY_QUEUE = { waiting: [], ended: [], asleep: [], running: [], up_next: [], bands: [], lanes: [] };

export async function refresh() {
  const ok = await refreshQueries();
  land();
  return ok;
}
export { cache };
export { api, refuse, signOut, KEPT_KINDS } from "../session.js";

onSignOut(() => {
  watch.stop();
  // Before the token goes: giving the keyboard back needs it.
  keyboard.leave();
  Object.assign(page, { landed: {}, stamp: null });
  Object.assign(state, { data: null, flowsError: null, costs: { by: "workflow", loading: false, slow: false, error: null } });
  asked.set(params());
});
session.subscribe(draw);
