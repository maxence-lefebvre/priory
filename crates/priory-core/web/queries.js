// The server data: one QueryClient holds each listing and detail under its own key. A page observes
// its own queries, so a moved revision reads only what is on screen. A read keeps the last data on
// screen until the new data lands, and a failed read keeps it.
import { QueryCache, QueryClient } from "@tanstack/react-query";

// The server reads its views and links once, when it starts, so a page reads them once.
const ONCE = ["views", "links"];
const once = (query) => ONCE.includes(query.queryKey[0]) || Boolean(query.meta?.once);
const LISTINGS = ["views", "links", "desk", "asks", "queue", "tasks", "runs", "report", "reports"];
// What an answer depends on. Another read that fails is its page's to say.
const ANSWERED_FROM = ["views", "desk", "asks"];
const REVISIONED = ["desk", "asks", "queue", "tasks", "runs"];

// What a page reads: the listings in full once the person scrolled to their older rows, and the
// report the arrows reached, null for the newest.
export const params = (all = { asks: false, tasks: false, runs: false }, reportShown = null) => ({ all: { ...all }, reportShown });

/** The query key of a listing, as `wanted` reads it. */
export function keyOf(name, { all, reportShown } = params()) {
  if (name in all) return [name, Boolean(all[name])];
  return name === "report" ? ["report", reportShown] : [name];
}

/** Reads the query `key` from the server. A file under reads/ brings its own read. */
export function read(api, key, signal) {
  const [name, part] = key;
  if (name === "links") return api("/api/links", signal).then((body) => body.links);
  if (name === "reports") return api("/api/reports", signal).then((body) => body.reports);
  if (name === "task") return api(`/api/tasks/${part}`, signal);
  if (name === "report") {
    // `report` is null until the first `priory report put`; a report the arrows reached may be gone.
    return api(part == null ? "/api/reports/latest" : `/api/reports/${part}`, signal)
      .then((found) => (found.report ? found : null))
      .catch((error) => (error.status === 404 ? null : Promise.reject(error)));
  }
  return api(part ? `/api/${name}?all=1` : `/api/${name}`, signal);
}

const isListing = (query) => LISTINGS.includes(query.queryKey[0]);
const locks = (query) => ANSWERED_FROM.includes(query.queryKey[0]);

/**
 * The client. `api` reads a path, `failed` hears why the last read of what an answer depends on
 * failed, null once it lands, and `refused` hears a 401. Queries never read by themselves once they hold data: an event,
 * a refresh or a tab that is shown again marks them stale.
 */
export function client({ api, failed = () => {}, refused = () => {} }) {
  const cache = new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => {
        if (error.status === 401) refused(error);
        else if (locks(query)) failed(error);
      },
      onSuccess: () => healed(),
    }),
    defaultOptions: {
      queries: {
        queryFn: ({ queryKey, signal, meta }) => (meta?.read ? meta.read(api, signal) : read(api, queryKey, signal)),
        // A read that started is worth its answer offline too: the fetch itself says it failed.
        networkMode: "always",
        retry: false,
        staleTime: Infinity,
        gcTime: Infinity,
        refetchOnWindowFocus: false,
        refetchOnReconnect: false,
      },
    },
  });
  const healed = () => {
    if (!errored(cache).some(locks)) failed(null);
  };
  cache.getQueryCache().subscribe((event) => {
    if (event.type === "observerRemoved") healed();
  });
  return cache;
}

const active = (cache) => cache.getQueryCache().findAll({ type: "active" });
const errored = (cache) => active(cache).filter((query) => query.state.status === "error");
const moving = (query) => !once(query) || query.state.status === "error" || query.state.data === undefined;

// A page's own read, from a file under reads/, rather than a listing or a task.
const own = (query) => Boolean(query.meta?.read);

// Marks every read stale and reads the mounted ones again. True when none of them failed. A page's own
// read follows the others, and waits while what an answer depends on failed or is read by a later
// refresh, as the legacy loaders did: that refresh reads it once it lands.
async function reread(cache, options, predicate = moving) {
  await cache.invalidateQueries({ predicate: (query) => predicate(query) && !own(query) }, options);
  const unsure = active(cache).some((query) => locks(query) && (query.state.status === "error" || query.state.fetchStatus === "fetching"));
  const refetchType = unsure ? "none" : "active";
  await cache.invalidateQueries({ predicate: (query) => predicate(query) && own(query), refetchType }, options);
  return errored(cache).length === 0;
}

/**
 * The revision moved: one read of each mounted query, which joins one in flight. What a joined read
 * answers may be from before the move, so that query is read once more.
 */
export async function moved(cache) {
  const joined = new Set(active(cache).filter((query) => query.state.fetchStatus === "fetching"));
  const ok = await reread(cache, { cancelRefetch: false });
  return joined.size ? reread(cache, { cancelRefetch: false }, (query) => joined.has(query) && moving(query)) : ok;
}

/** Whether a listing on screen failed with nothing live to draw, so the events poll cannot start for it. */
export const stuck = (cache) => errored(cache).some((query) => isListing(query) && (query.state.data === undefined || (REVISIONED.includes(query.queryKey[0]) && query.state.data.revision === undefined)));

/** A listing on screen that no answer depends on, whose last read failed: its name, the error and whether it holds an earlier read, else null. */
export function unread(cache) {
  const query = errored(cache).find((each) => isListing(each) && !locks(each));
  return query ? { name: query.queryKey[0], error: query.state.error, held: query.state.data !== undefined } : null;
}

/** The person acted, or asked: a new read of each mounted query, which supersedes one in flight. */
export const refreshMounted = (cache) => reread(cache, { cancelRefetch: true });

/** The tab is about to show: the mounted queries read once as they are enabled. */
export const staleAll = (cache) => cache.invalidateQueries({ predicate: moving, refetchType: "none" });

/**
 * The oldest revision the mounted queries hold, so the next wait catches what they missed.
 * Undefined until each of them landed from the server: data kept on the device names no revision.
 */
export function heldRevision(cache) {
  let oldest;
  for (const query of active(cache)) {
    // A detail that cannot be read again would hold the wait at a revision long gone.
    if (query.state.status === "error" && !isListing(query)) continue;
    const revision = query.state.data?.revision;
    if (revision === undefined && REVISIONED.includes(query.queryKey[0])) return undefined;
    if (revision !== undefined) oldest = Math.min(oldest ?? revision, revision);
  }
  return oldest;
}

/** When the mounted listings were read, the oldest first, else null. */
export function oldestRead(cache) {
  const times = active(cache).filter((query) => isListing(query) && !once(query) && query.state.dataUpdatedAt).map((query) => query.state.dataUpdatedAt);
  return times.length ? Math.min(...times) : null;
}

const kept = (query) => LISTINGS.includes(query.queryKey[0]) || Boolean(query.meta?.kept);

/** Puts `data` in the cache under a read's own key and meta, which setQueryData drops. */
export const hold = (cache, { queryKey, meta }, data, options) => cache.getQueryCache().build(cache, cache.defaultQueryOptions({ queryKey, meta })).setData(data, { ...options, manual: true });

/** The newest read of `name` the cache holds, whichever rows it asked for. */
export function newest(cache, name) {
  const held = cache.getQueryCache().findAll({ queryKey: [name] }).filter((query) => query.state.data !== undefined);
  return held.sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt)[0]?.state.data;
}

const withoutRevision = (data) => {
  if (!data || Array.isArray(data) || typeof data !== "object" || !("revision" in data)) return data;
  const { revision: _, ...rest } = data;
  return rest;
};

/** The newest read of each listing, to keep on this device: its key, its data and when it was read. */
export function snapshot(cache) {
  const newest = new Map();
  for (const query of cache.getQueryCache().getAll()) {
    const [name] = query.queryKey;
    if (!kept(query) || query.state.data === undefined) continue;
    const before = newest.get(name);
    if (!before || before.state.dataUpdatedAt < query.state.dataUpdatedAt) newest.set(name, query);
  }
  return { reads: [...newest.values()].map((query) => [query.queryKey, query.state.data, query.state.dataUpdatedAt]) };
}

// The value an earlier version of the web view kept: every listing at one time.
function earlier({ data, updated }) {
  const { views, links, desk, asks, bands, queue, tasks, runs, older = {}, report, reports, spend, flows } = data;
  return [
    [["views"], views], [["links"], links], [["desk"], desk], [["asks", false], asks && { asks, bands, older: older.asks }],
    [["queue"], queue], [["tasks", false], tasks && { tasks, older: older.tasks }], [["runs", false], runs && { runs, older: older.runs }],
    [["report", null], report], [["reports"], reports], [["spend"], spend ?? undefined], [["flows"], flows ?? undefined],
  ].filter(([, body]) => body !== undefined).map(([key, body]) => [key, body, updated ?? 0]);
}

/** Puts a kept snapshot in the cache, marked stale so a live page reads it again. */
export function restore(cache, kept) {
  const reads = kept?.reads ?? (kept?.data ? earlier(kept) : []);
  // The legacy Costs panel kept one value of two reads.
  const spend = reads.find(([[name]]) => name === "spend");
  const split = spend?.[1] ? [[["costs", spend[1].costs.by], spend[1].costs, spend[2]], [["attention"], spend[1].attention, spend[2]]] : [];
  const missing = split.filter(([[name]]) => !reads.some(([key]) => key[0] === name));
  for (const [[name, part], data, updatedAt] of [...reads.filter(([[name]]) => name !== "spend"), ...missing]) {
    // A page opens on its listings' newest rows, and on the newest report.
    const key = part === true ? [name, false] : name === "report" ? ["report", null] : part === undefined ? [name] : [name, part];
    hold(cache, { queryKey: key, meta: { kept: true } }, withoutRevision(data), { updatedAt });
    cache.invalidateQueries({ queryKey: key, exact: true, refetchType: "none" });
  }
}
