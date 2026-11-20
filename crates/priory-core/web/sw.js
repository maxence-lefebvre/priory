// The service worker: keeps the page's own files for offline starts, and shows Web Push notifications.
// It never stores /api replies; the page keeps its last listings itself.
const SHELL = "priory-shell";

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(SHELL).then((cache) => cache.add("/")));
  self.skipWaiting();
});
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

// Network first, so a new build shows at once; the cached copy only when the network fails.
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  const own = url.origin === location.origin && !url.pathname.startsWith("/api") && !url.pathname.startsWith("/hooks");
  if (event.request.method !== "GET" || !own) return;
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
const own = (path) => (typeof path === "string" && path.startsWith("/") && !path.startsWith("//") ? path : "/");

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
      return open.focus().then((window) => window.navigate(url));
    }),
  );
});
