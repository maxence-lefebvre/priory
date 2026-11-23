// The server data: one QueryClient holds each listing under its own key. A read keeps the last data
// on screen until the new data lands, and a failed read keeps it (stale-while-revalidate).
import { QueryClient, keepPreviousData } from "@tanstack/react-query";

// Queries never read by themselves: refresh() and the events long poll decide when to read again.
export const client = () => new QueryClient({
  defaultOptions: { queries: { retry: false, staleTime: Infinity, gcTime: 10 * 60_000, refetchOnWindowFocus: false, refetchOnReconnect: false } },
});

// What a refresh reads: the listings in full once the person scrolled to their older rows, and the
// report the arrows reached, null for the newest.
export const params = (all = { asks: false, tasks: false, runs: false }, reportShown = null) => ({ all: { ...all }, reportShown });
export const sameParams = (a, b) => JSON.stringify(a) === JSON.stringify(b);

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

const options = (api, [name, queryKey, read]) => ({ queryKey, queryFn: () => read(api), staleTime: ONCE.has(name) ? Infinity : 0 });

// Reads every listing again, at once. Throws the first failure; the cache keeps what each had.
// fetchQuery answers the raw reply; the cache holds the copy that shares what did not change.
export async function readAll(cache, api, wanted) {
  const all = reads(wanted);
  await Promise.all(all.map((read) => cache.fetchQuery(options(api, read))));
  return Object.fromEntries(all.map(([name, queryKey]) => [name, cache.getQueryData(queryKey)]));
}

// The listings as useQueries observes them: the last ones while new params read, null before
// every listing has data once.
export const observed = (api, wanted) => ({
  queries: reads(wanted).map((read) => ({ ...options(api, read), placeholderData: keepPreviousData })),
  combine: (results) => (results.every((result) => result.data !== undefined) ? Object.fromEntries(reads(wanted).map(([name], i) => [name, results[i].data])) : null),
});

// Whether two reads hold the same listings. Structural sharing keeps an unchanged listing the same
// object, so this compares references, not contents.
export const sameListings = (a, b) => Boolean(a && b) && Object.keys(a).length === Object.keys(b).length && Object.keys(a).every((name) => a[name] === b[name]);
