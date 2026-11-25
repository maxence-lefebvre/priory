// The app's QueryClient, its copy on this device, and its refreshes.
import { client, heldRevision, moved, refreshMounted, restore, snapshot } from "./queries.js";
import { api, failureWords, onSignOut, refuse, session, setFailed } from "./session.js";

const KEY = "priory.snapshot";

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

try {
  restore(cache, JSON.parse(localStorage.getItem(KEY) || "null"));
} catch {
  localStorage.removeItem(KEY);
}

// The listings and their times go in one value, so a full storage never pairs old data with a new time.
let keeping = false;
cache.getQueryCache().subscribe((event) => {
  if (event.type !== "updated" || event.action.type !== "success" || keeping) return;
  keeping = true;
  setTimeout(() => {
    keeping = false;
    if (!session.get().token) return;
    try {
      localStorage.setItem(KEY, JSON.stringify(snapshot(cache)));
    } catch {}
  });
});
onSignOut(() => cache.clear());

export const revision = () => (session.get().token ? heldRevision(cache) : undefined);
export const refresh = async () => Boolean(session.get().token) && refreshMounted(cache);
export const reread = async () => Boolean(session.get().token) && moved(cache);
