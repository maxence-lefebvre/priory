import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const SOURCE = readFileSync(new URL("./sw.js", import.meta.url), "utf8");

// Loads sw.js as a browser would, with stand-ins for the worker's globals: one Cache Storage, a
// registration that records notifications and subscriptions, and a fetch that records requests.
function worker({ token = null, refuseKinds = false } = {}) {
  const stores = new Map();
  const caches = {
    async open(name) {
      if (!stores.has(name)) stores.set(name, new Map());
      const store = stores.get(name);
      return {
        async match(key) {
          return store.has(key) ? new Response(store.get(key)) : undefined;
        },
        async put(key, response) {
          store.set(key, await response.text());
        },
        async delete(key) {
          return store.delete(key);
        },
        async addAll(paths) {
          for (const path of paths) store.set(path, "");
        },
      };
    },
    async match() {},
    async keys() {
      return [...stores.keys()];
    },
    async delete(name) {
      return stores.delete(name);
    },
  };
  const listeners = {};
  const shown = [];
  const requests = [];
  const subscribed = [];
  const registration = {
    async showNotification(title, options) {
      shown.push({ title, ...options });
    },
    pushManager: {
      async subscribe(options) {
        subscribed.push(options);
        const subscription = { endpoint: "https://fcm.googleapis.com/fcm/send/new", keys: { p256dh: "p", auth: "a" } };
        return { ...subscription, toJSON: () => subscription };
      },
    },
  };
  const self = {
    registration,
    addEventListener: (type, listener) => (listeners[type] = listener),
    skipWaiting() {},
    clients: { claim: async () => {} },
  };
  const fetch = async (path, init = {}) => {
    requests.push({ path, method: init.method || "GET", auth: init.headers?.Authorization, body: init.body ? JSON.parse(init.body) : null });
    if (path === "/api/push/key") return new Response(JSON.stringify({ key: "AQID" }));
    if (refuseKinds && requests.at(-1).body?.kinds) return new Response("{}", { status: 400 });
    return new Response("{}");
  };
  vm.runInNewContext(SOURCE, { self, caches, fetch, location: { origin: "https://priory.test" }, URL, Response, atob, Uint8Array, JSON });
  const fire = async (type, event) => {
    const waits = [];
    listeners[type]({ ...event, waitUntil: (promise) => waits.push(promise) });
    await Promise.all(waits);
  };
  const ready = token ? caches.open("priory-push").then((cache) => cache.put("/api/worker/token", new Response(token))) : Promise.resolve();
  return { fire, shown, requests, subscribed, ready, stores };
}

test("an install caches the shell under its new name, and an upgrade drops the older shell", async () => {
  const { fire, stores } = worker();
  await fire("install", {});
  assert.deepEqual([...stores.get("priory-shell-2").keys()].filter((path) => path.endsWith(".js")), ["/boot.js", "/app.js"]);

  stores.set("priory-shell", new Map([["/asks.js", "old"]]));
  stores.set("priory-push", new Map([["/api/worker/token", "t"]]));
  await fire("activate", {});
  assert.deepEqual([...stores.keys()].sort(), ["priory-push", "priory-shell-2"]);
});

const push = (notice) => ({ data: { json: () => notice } });

test("a notification sent again with the same id never alerts twice", async () => {
  const { fire, shown } = worker();
  await fire("push", push({ id: "2026-11-21T10:00:00Z/1", title: "task cli:a blocked", url: "/tasks/1" }));
  await fire("push", push({ id: "2026-11-21T10:00:00Z/1", title: "task cli:a blocked", url: "/tasks/1" }));
  await fire("push", push({ id: "2026-11-21T10:00:00Z/2", title: "task cli:a failed", url: "/tasks/1" }));

  assert.deepEqual(shown.map((notice) => [notice.body, notice.tag, Boolean(notice.silent)]), [
    ["task cli:a blocked", "/tasks/1", false],
    ["task cli:a blocked", "/tasks/1", true],
    ["task cli:a failed", "/tasks/1", false],
  ]);
});

test("a notification with no id alerts every time", async () => {
  const { fire, shown } = worker();
  await fire("push", push({ title: "ask 3", url: "/asks/3" }));
  await fire("push", push({ title: "ask 3", url: "/asks/3" }));
  assert.deepEqual(shown.map((notice) => Boolean(notice.silent)), [false, false]);
});

test("a changed subscription subscribes again with the same key and tells the daemon", async () => {
  const { fire, requests, subscribed, ready } = worker({ token: "secret" });
  await ready;
  const key = new Uint8Array([4, 5, 6]).buffer;
  await fire("pushsubscriptionchange", { oldSubscription: { endpoint: "https://fcm.googleapis.com/fcm/send/old", options: { applicationServerKey: key } } });

  assert.equal(subscribed.length, 1);
  assert.equal(subscribed[0].applicationServerKey, key);
  assert.equal(subscribed[0].userVisibleOnly, true);
  assert.deepEqual(requests.map((request) => [request.method, request.path, request.auth]), [
    ["POST", "/api/push/subscribe", "Bearer secret"],
    ["POST", "/api/push/unsubscribe", "Bearer secret"],
  ]);
  assert.equal(requests[0].body.endpoint, "https://fcm.googleapis.com/fcm/send/new");
  assert.deepEqual(requests[1].body, { endpoint: "https://fcm.googleapis.com/fcm/send/old" });
});

test("a new subscription the browser gives is sent as it is, and without an old one the key is read", async () => {
  const given = worker({ token: "secret" });
  await given.ready;
  const subscription = { endpoint: "https://updates.push.services.mozilla.com/wpush/v2/x", keys: { p256dh: "p", auth: "a" } };
  await given.fire("pushsubscriptionchange", { oldSubscription: null, newSubscription: { ...subscription, toJSON: () => subscription } });
  assert.equal(given.subscribed.length, 0);
  assert.deepEqual(given.requests.map((request) => [request.path, request.body]), [["/api/push/subscribe", subscription]]);

  const read = worker({ token: "secret" });
  await read.ready;
  await read.fire("pushsubscriptionchange", { oldSubscription: null });
  assert.deepEqual([...read.subscribed[0].applicationServerKey], [1, 2, 3]);
  assert.deepEqual(read.requests.map((request) => request.path), ["/api/push/key", "/api/push/subscribe"]);
});

test("a replaced subscription keeps the device's kinds, with or without the old one", async () => {
  for (const oldSubscription of [null, { endpoint: "https://fcm.googleapis.com/fcm/send/old", options: {} }]) {
    const { fire, requests, ready, stores } = worker({ token: "secret" });
    await ready;
    stores.get("priory-push").set("/api/worker/kinds", JSON.stringify({ ask: true, stopped: false }));
    await fire("pushsubscriptionchange", { oldSubscription });
    const sent = requests.filter((request) => request.path === "/api/push/subscribe");
    assert.deepEqual(sent.map((request) => request.body.kinds), [{ ask: true, stopped: false }]);
  }
});

test("a daemon that refuses the kinds still gets the replaced subscription", async () => {
  const { fire, requests, ready, stores } = worker({ token: "secret", refuseKinds: true });
  await ready;
  stores.get("priory-push").set("/api/worker/kinds", JSON.stringify({ ask: true }));
  await fire("pushsubscriptionchange", { oldSubscription: null });
  const sent = requests.filter((request) => request.path === "/api/push/subscribe");
  assert.deepEqual(sent.map((request) => request.body.kinds), [{ ask: true }, undefined]);
});

test("with no token kept, the worker subscribes again and leaves telling the daemon to the page", async () => {
  const { fire, requests, subscribed } = worker();
  await fire("pushsubscriptionchange", { oldSubscription: { endpoint: "https://fcm.googleapis.com/fcm/send/old", options: { applicationServerKey: new Uint8Array([1]).buffer } } });
  assert.equal(subscribed.length, 1);
  assert.deepEqual(requests, []);
});
