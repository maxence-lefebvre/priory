// The server data: one QueryClient holds each listing and detail under its own key. A page observes
// its own queries, so a moved revision reads only what is on screen. A read keeps the last data on
// screen until the new data lands, and a failed read keeps it.
import { QueryCache, QueryClient } from "@tanstack/react-query";

// The server reads its views and links once, when it starts, so a page reads them once.
const ONCE = new Set(["views", "links"]);
const LISTINGS = ["views", "links", "desk", "asks", "queue", "tasks", "runs", "report", "reports"];
const REVISIONED = ["desk", "asks", "queue", "tasks", "runs"];

// What a page reads: the listings in full once the person scrolled to their older rows, and the
// report the arrows reached, null for the newest.
export const params = (all = { asks: false, tasks: false, runs: false }, reportShown = null) => ({ all: { ...all }, reportShown });

/** The query key of a listing, as `wanted` reads it. */
export function keyOf(name, { all, reportShown } = params()) {
  if (name in all) return [name, Boolean(all[name])];
  return name === "report" ? ["report", reportShown] : [name];
}

/** Reads the query `key` from the server. */
export function read(api, [name, part], signal) {
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

/**
 * The client. `api` reads a path, `failed` hears why a listing's last read failed, null once one
 * lands, and `refused` hears a 401. Queries never read by themselves once they hold data: an event,
 * a refresh or a tab that is shown again marks them stale.
 */
export function client({ api, failed = () => {}, refused = () => {} }) {
  const cache = new QueryClient({
    queryCache: new QueryCache({
      onError: (error, query) => {
        if (error.status === 401) refused(error);
        else if (isListing(query)) failed(error);
      },
      onSuccess: (_, query) => {
        if (isListing(query) && !broken(cache)) failed(null);
      },
    }),
    defaultOptions: {
      queries: {
        queryFn: ({ queryKey, signal }) => read(api, queryKey, signal),
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
  return cache;
}

const active = (cache) => cache.getQueryCache().findAll({ type: "active" });
const broken = (cache) => active(cache).some((query) => isListing(query) && query.state.status === "error");
const moving = (query) => !ONCE.has(query.queryKey[0]) || query.state.status === "error" || query.state.data === undefined;

// Marks every read stale and reads the mounted ones again. True when none of them failed.
async function reread(cache, options) {
  await cache.invalidateQueries({ predicate: moving }, options);
  return !broken(cache);
}

/** The revision moved: one read of each mounted query, which joins one in flight. */
export const moved = (cache) => reread(cache, { cancelRefetch: false });

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
    const revision = query.state.data?.revision;
    if (revision === undefined && REVISIONED.includes(query.queryKey[0])) return undefined;
    if (revision !== undefined) oldest = Math.min(oldest ?? revision, revision);
  }
  return oldest;
}

/** When the mounted listings were read, the oldest first, else null. */
export function oldestRead(cache) {
  const times = active(cache).filter((query) => isListing(query) && !ONCE.has(query.queryKey[0]) && query.state.dataUpdatedAt).map((query) => query.state.dataUpdatedAt);
  return times.length ? Math.min(...times) : null;
}

const KEPT = new Set([...LISTINGS, "spend", "flows"]);
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
    if (!KEPT.has(name) || query.state.data === undefined) continue;
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
  for (const [[name, part], data, updatedAt] of reads) {
    // A page opens on its listings' newest rows, and on the newest report.
    const key = part === true ? [name, false] : name === "report" ? ["report", null] : part === undefined ? [name] : [name, part];
    cache.setQueryData(key, withoutRevision(data), { updatedAt });
    cache.invalidateQueries({ queryKey: key, exact: true, refetchType: "none" });
  }
}
