// What the legacy pages draw from: the listings of the page on screen, landed together.
import { draw } from "../app.js";
import { cache, refresh as refreshQueries } from "../data.js";
import { keyOf, params } from "../queries.js";
import { onSignOut, session } from "../session.js";
import { store } from "../store.js";
import { each, entries } from "../registry.js";
import { state } from "./state.js";

// Which rows and which report the page asks the server for.
export const asked = store(params());
export const askFor = (part) => asked.set({ ...asked.get(), ...part });

// The listings the page on screen shows, and those landed last, by name.
const page = { names: [], landed: {}, stamp: null };

export function openPage(names) {
  page.names = names;
  // Older rows load again when the person scrolls back to them.
  const { all } = asked.get();
  const left = Object.fromEntries(Object.entries(all).map(([name, on]) => [name, on && entries.some((entry) => entry.older?.[name]?.())]));
  if (Object.keys(all).some((name) => all[name] !== left[name])) askFor({ all: left });
  land();
}

// Whether every listing of the page has data: the page draws a loading line until then.
export const ready = () => Boolean(state.data) && page.names.every((name) => name in page.landed);
export const landing = store(true);

const kept = () => entries.flatMap((entry) => entry.kept ?? []);
const drawn = (data) => [data.desk, data.queue, data.asks, data.tasks, data.runs, data.older.asks, data.older.tasks, data.older.runs, ...kept().map((name) => data[name])];

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
  if (desk.revision !== undefined) each("live", desk);
  const [asksList, tasksList, runsList] = ["asks", "tasks", "runs"].map(of);
  const before = state.data;
  state.data = {
    views: of("views"), links: of("links") ?? [], desk,
    asks: asksList?.asks ?? NONE, bands: asksList?.bands,
    queue: of("queue") ?? EMPTY_QUEUE,
    tasks: tasksList?.tasks ?? NONE, runs: runsList?.runs ?? NONE,
    older: { asks: asksList?.older, tasks: tasksList?.older, runs: runsList?.older },
    // Each page's own; a refresh keeps the last ones.
    ...Object.fromEntries(kept().map((name) => [name, of(name) ?? null])),
  };
  if (fresh && ready()) {
    each("landed");
  }
  const same = before && drawn(before).every((part, i) => part === drawn(state.data)[i]);
  if (!same || Date.now() - state.rendered >= 60000) draw();
}

const NONE = [];
const EMPTY_QUEUE = { waiting: [], ended: [], asleep: [], running: [], up_next: [], bands: [], lanes: [] };

// The ask a refused write answered with, in every listing that holds it, as old as the listing it joins.
export async function acceptAsk(fresh) {
  // A read that began before the refusal would bring the older ask back.
  await cache.cancelQueries({ queryKey: ["asks"] });
  const patched = (body) => ({ ...body, asks: body.asks.map((ask) => (ask.id === fresh.id ? fresh : ask)) });
  for (const [queryKey, body] of cache.getQueriesData({ queryKey: ["asks"] })) {
    if (!body) continue;
    const { dataUpdatedAt: updatedAt, isInvalidated } = cache.getQueryState(queryKey);
    cache.setQueryData(queryKey, patched(body), { updatedAt });
    if (isInvalidated) cache.invalidateQueries({ queryKey, exact: true, refetchType: "none" });
  }
  const { asks } = page.landed;
  if (asks) page.landed.asks = { ...asks, data: patched(asks.data) };
  if (state.data) state.data.asks = patched(state.data).asks;
}

export async function refresh() {
  const ok = await refreshQueries();
  land();
  return ok;
}
export { cache };
export { api, refuse, signOut, KEPT_KINDS } from "../session.js";

onSignOut(() => {
  each("signedOut");
  Object.assign(page, { landed: {}, stamp: null });
  state.data = null;
  asked.set(params());
});
session.subscribe(draw);
