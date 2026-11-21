// The service worker: keeps the page's own files for offline starts, and shows Web Push notifications.
// It never stores /api replies; the page keeps its last listings itself.
const SHELL = "priory-shell";
// The page loaded these before the worker controlled it, so it fetches them again for offline starts.
const FILES = ["/", "/boot.js", "/app.js", "/asks.js", "/add.js", "/links.js", "/reports.js", "/stack.js", "/desk.js", "/costs.js", "/inbox.js", "/flows.js", "/tasks.js", "/runs.js", "/app.css", "/tokens/structure.css", "/tokens/themes.css", "/favicon.svg", "/icon-192.png"];

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(SHELL).then((cache) => cache.addAll(FILES)));
  self.skipWaiting();
});
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

// Network first, so a new build shows at once; the cached copy only when the network fails.
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  const page = url.origin === location.origin && !url.pathname.startsWith("/api") && !url.pathname.startsWith("/hooks");
  if (event.request.method !== "GET" || !page) return;
  // Every page path serves the same index, so one copy covers /asks/42 too.
  const key = event.request.mode === "navigate" ? "/" : event.request;
  event.respondWith(
    fetch(event.request)
      .then((response) => {
        if (response.ok) {
          const copy = response.clone();
          event.waitUntil(caches.open(SHELL).then((cache) => cache.put(key, copy)));
        }
        return response;
      })
      .catch(() => caches.match(key).then((hit) => hit || Response.error())),
  );
});

// A path on this origin only, so a payload can never send the person elsewhere.
function own(path) {
  try {
    const url = new URL(path, location.origin);
    return url.origin === location.origin ? url.pathname : "/";
  } catch {
    return "/";
  }
}

// What the worker keeps between its starts: the page's token, and the ids of the last notifications.
// The keys sit under /api, which the fetch handler never serves from a cache.
const KEPT = "priory-push";
const TOKEN = "/api/worker/token";
const SEEN = "/api/worker/seen";
const SEEN_KEPT = 50;

async function keptText(key) {
  const hit = await (await caches.open(KEPT)).match(key);
  return hit ? hit.text() : null;
}

// Whether a notification with this id came before, and records it. A daemon stopped mid-send, or a
// retry after a lost answer, sends the same id again.
async function seenBefore(id) {
  if (typeof id !== "string" || !id) return false;
  let seen = [];
  try {
    seen = JSON.parse((await keptText(SEEN)) || "[]");
  } catch {}
  if (seen.includes(id)) return true;
  const cache = await caches.open(KEPT);
  await cache.put(SEEN, new Response(JSON.stringify([...seen, id].slice(-SEEN_KEPT))));
  return false;
}

self.addEventListener("push", (event) => {
  let notice = {};
  try {
    notice = event.data?.json() || {};
  } catch {}
  const url = own(notice.url);
  const body = typeof notice.title === "string" ? notice.title : "Something needs you.";
  // A copy still shows, as a browser that subscribed with userVisibleOnly needs, but silently and in
  // the first one's place, so it never alerts twice.
  event.waitUntil(seenBefore(notice.id).catch(() => false).then((again) =>
    self.registration.showNotification("Priory", { body, tag: url, data: { url }, icon: "/icon-192.png", ...(again ? { silent: true } : {}) })));
});

// The push service replaced or expired the subscription: subscribe again with the same key and
// tell the daemon, with the token the page kept for the worker. Without one, the page sends the
// new subscription the next time its Notifications screen opens.
async function resubscribe(old, given) {
  const token = await keptText(TOKEN);
  const send = (path, body) => fetch(path, { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify(body) });
  let fresh = given || null;
  if (!fresh) {
    let key = old?.options?.applicationServerKey || null;
    if (!key && token) {
      const response = await fetch("/api/push/key", { headers: { Authorization: `Bearer ${token}` }, cache: "no-store" });
      if (response.ok) key = Uint8Array.from(atob((await response.json()).key.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0));
    }
    if (!key) return;
    fresh = await self.registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: key });
  }
  if (!token) return;
  await send("/api/push/subscribe", fresh.toJSON());
  if (old?.endpoint && old.endpoint !== fresh.endpoint) await send("/api/push/unsubscribe", { endpoint: old.endpoint });
}

self.addEventListener("pushsubscriptionchange", (event) => {
  event.waitUntil(resubscribe(event.oldSubscription, event.newSubscription).catch(() => {}));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = new URL(own(event.notification.data?.url), location.origin).href;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      const open = windows.find((window) => new URL(window.url).origin === location.origin);
      if (!open) return self.clients.openWindow(url);
      // navigate rejects for a window this worker does not control yet; a new window still opens the ask.
      return open.focus().then((window) => window.navigate(url)).catch(() => self.clients.openWindow(url));
    }),
  );
});
