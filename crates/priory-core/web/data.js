// The app's QueryClient, its copy on this device, and its refreshes.
import { client, heldRevision, moved, params, refreshMounted, restore, snapshot } from "./queries.js";
import { EARLIER_SNAPSHOT, SNAPSHOT, api, failureWords, onSignOut, refuse, session, setFailed } from "./session.js";
import { store } from "./store.js";

// When the first live read of the desk began, so an event between that read and the visit still shows next time.
let opened = null;
export const deskReadAt = () => opened;

export const cache = client({
  api: (path, signal) => {
    if (path === "/api/desk") opened ??= new Date().toISOString().replace(/\.\d+Z$/, "Z");
    return api(path, signal);
  },
  failed: (error) => setFailed(error && failureWords(error)),
  // A read from before a sign-out must not sign the new session out.
  refused: (error) => error.token === session.get().token && refuse(),
});

// An earlier version's sign-out leaves this version's copy behind.
if (!session.get().token) for (const key of [SNAPSHOT, EARLIER_SNAPSHOT]) localStorage.removeItem(key);
for (const key of [SNAPSHOT, EARLIER_SNAPSHOT]) {
  try {
    const kept = JSON.parse(localStorage.getItem(key) || "null");
    if (!kept) continue;
    restore(cache, kept);
    break;
  } catch {
    localStorage.removeItem(key);
  }
}

// The listings and their times go in one value, so a full storage never pairs old data with a new time.
let keeping = false;
let unkept = false;
cache.getQueryCache().subscribe((event) => {
  if (event.type !== "updated") return;
  unkept ||= event.action.type === "success";
  // A failed or cancelled read may be the last one the others waited for.
  if (!unkept || keeping) return;
  keeping = true;
  setTimeout(() => {
    keeping = false;
    // Once per refresh: the read still in flight keeps them all as it ends.
    if (!unkept || cache.isFetching() || !session.get().token) return;
    unkept = false;
    try {
      localStorage.setItem(SNAPSHOT, JSON.stringify(snapshot(cache)));
    } catch {}
  });
});
onSignOut(() => cache.clear());

export const revision = () => (session.get().token ? heldRevision(cache) : undefined);
export const refresh = async () => Boolean(session.get().token) && refreshMounted(cache);
export const reread = async () => Boolean(session.get().token) && moved(cache);

// Which rows and which report the page asks the server for.
export const asked = store(params());
export const askFor = (part) => asked.set({ ...asked.get(), ...part });
onSignOut(() => asked.set(params()));

export function loadOlder(listing) {
  // A render may call this: the page reads the older rows once it has drawn.
  setTimeout(() => askFor({ all: { ...asked.get().all, [listing]: true } }));
}

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
}
