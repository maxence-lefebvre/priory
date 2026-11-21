// The service worker: keeps the page's own files for offline starts, and shows Web Push notifications.
// It never stores /api replies; the page keeps its last listings itself.
const SHELL = "priory-shell";
// The page loaded these before the worker controlled it, so it fetches them again for offline starts.
const FILES = ["/", "/boot.js", "/app.js", "/asks.js", "/add.js", "/links.js", "/reports.js", "/stack.js", "/desk.js", "/costs.js", "/inbox.js", "/flows.js", "/app.css", "/tokens/structure.css", "/tokens/themes.css", "/favicon.svg", "/icon-192.png"];

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

self.addEventListener("push", (event) => {
  let notice = {};
  try {
    notice = event.data?.json() || {};
  } catch {}
  const url = own(notice.url);
  const body = typeof notice.title === "string" ? notice.title : "Something needs you.";
  event.waitUntil(self.registration.showNotification("Priory", { body, tag: url, data: { url }, icon: "/icon-192.png" }));
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
