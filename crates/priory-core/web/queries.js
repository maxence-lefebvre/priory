// The server data: one QueryClient holds each listing under its own key. A read keeps the last data
// on screen until the new data lands, and a failed read keeps it.
import { QueryClient, notifyManager } from "@tanstack/react-query";

// Queries never read by themselves: refresh() and the events long poll decide when to read again.
export const client = () => new QueryClient({
  defaultOptions: { queries: { retry: false, staleTime: Infinity, gcTime: Infinity, refetchOnWindowFocus: false, refetchOnReconnect: false } },
});

// What a refresh reads: the listings in full once the person scrolled to their older rows, and the
// report the arrows reached, null for the newest.
export const params = (all = { asks: false, tasks: false, runs: false }, reportShown = null) => ({ all: { ...all }, reportShown });

// The server reads its views once, when it starts, so a page reads them and the links once.
const ONCE = new Set(["views", "links"]);

// Each read: its name in the listings, its query key and how it reads.
export function reads({ all, reportShown }) {
  const full = (listing) => (all[listing] ? `/api/${listing}?all=1` : `/api/${listing}`);
  const get = (path, pick = (body) => body) => (api) => api(path).then(pick);
  // `report` is null until the first `priory report put`; a report the arrows reached may be gone.
  const report = (api) => api(reportShown == null ? "/api/reports/latest" : `/api/reports/${reportShown}`)
    .then((found) => (found.report ? found : null))
    .catch((error) => (error.status === 404 ? null : Promise.reject(error)));
  return [
    ["views", ["views"], get("/api/views")],
    ["links", ["links"], get("/api/links", (body) => body.links)],
    ["desk", ["desk"], get("/api/desk")],
    ["asks", ["asks", Boolean(all.asks)], get(full("asks"))],
    ["queue", ["queue"], get("/api/queue")],
    ["tasks", ["tasks", Boolean(all.tasks)], get(full("tasks"))],
    ["runs", ["runs", Boolean(all.runs)], get(full("runs"))],
    ["report", ["report", reportShown], report],
    ["reports", ["reports"], get("/api/reports", (body) => body.reports)],
  ];
}

// Reads every listing at once and writes nothing, so a read that fails or that a later one
// supersedes leaves the cache as it was. It never joins a read already in flight.
export async function readAll(cache, api, wanted) {
  return Promise.all(reads(wanted).map(([name, queryKey, read]) => (ONCE.has(name) && cache.getQueryData(queryKey)) || read(api)));
}

// Writes a read into the cache in one step, and returns the listings by name, as the cache holds
// them: structural sharing keeps each unchanged listing the same object.
export function land(cache, listings, wanted) {
  return notifyManager.batch(() => Object.fromEntries(reads(wanted).map(([name, queryKey], i) => [name, cache.setQueryData(queryKey, listings[i])])));
}

// What a page draws of each listing: a changed revision alone must not draw it again.
const drawn = ({ desk, queue, report, reports, asks, tasks, runs }) => [desk, queue, report, reports, asks.asks, asks.older, tasks.tasks, tasks.older, runs.runs, runs.older];

// Whether two reads draw the same. Structural sharing keeps what did not change the same object, so
// this compares references, not contents.
export function sameListings(a, b) {
  if (!a || !b) return false;
  const before = drawn(a);
  return drawn(b).every((part, i) => part === before[i]);
}
