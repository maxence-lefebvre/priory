import { test } from "node:test";
import assert from "node:assert/strict";
import { QueryObserver, notifyManager } from "@tanstack/react-query";
import { client, land, params, readAll, sameListings } from "./queries.js";

// A refresh: a read, then its landing.
const refresh = async (cache, api, wanted = params()) => land(cache, await readAll(cache, api, wanted), wanted);

// A cache that drops its queries, and their timers, once the test ends.
function fresh(t) {
  const cache = client();
  t.after(() => cache.clear());
  return cache;
}

// A server whose listings the test changes, which records each path read.
function server() {
  const body = { desk: { revision: 1, since_you_looked: [] }, asks: { asks: [{ id: 1 }], revision: 1 }, queue: { waiting: [], revision: 1 }, tasks: { tasks: [{ id: 1, title: "a" }], revision: 1 }, runs: { runs: [], revision: 1 } };
  const paths = [];
  const failing = new Set();
  let held = null;
  const api = async (path) => {
    paths.push(path);
    if (held) await held;
    const name = path.split("?")[0].replace("/api/", "");
    if (failing.has(name)) throw Object.assign(new Error(`${path} answered 500`), { status: 500 });
    if (name === "views") return { factory: "shop" };
    if (name === "links") return { links: [] };
    if (name.startsWith("reports/")) return { report: null };
    if (name === "reports") return { reports: [] };
    return structuredClone(body[name]);
  };
  return { api, body, paths, failing, hold: () => { let release; held = new Promise((resolve) => (release = resolve)); return () => { held = null; release(); }; } };
}

test("a read with nothing new keeps every listing the same object", async (t) => {
  const { api } = server();
  const cache = fresh(t);
  const first = await refresh(cache, api);
  const second = await refresh(cache, api);
  assert.ok(sameListings(first, second));
});

test("a read keeps the listings that did not change and replaces the one that did", async (t) => {
  const { api, body } = server();
  const cache = fresh(t);
  const first = await refresh(cache, api);
  body.tasks.tasks[0].title = "b";
  const second = await refresh(cache, api);
  assert.equal(second.asks, first.asks);
  assert.notEqual(second.tasks, first.tasks);
  assert.equal(second.tasks.tasks[0].title, "b");
  assert.ok(!sameListings(first, second));
});

test("the cache shows the last listings until a re-read lands", async (t) => {
  const { api, body, hold } = server();
  const cache = fresh(t);
  await refresh(cache, api);
  body.tasks.tasks[0].title = "b";
  const release = hold();
  const reading = refresh(cache, api);
  assert.equal(cache.getQueryData(["tasks", false]).tasks[0].title, "a");
  release();
  await reading;
  assert.equal(cache.getQueryData(["tasks", false]).tasks[0].title, "b");
});

test("a failed re-read throws and keeps the last listings", async (t) => {
  const { api, failing } = server();
  const cache = fresh(t);
  const first = await refresh(cache, api);
  failing.add("desk");
  await assert.rejects(refresh(cache, api), /answered 500/);
  assert.equal(cache.getQueryData(["desk"]), first.desk);
  assert.ok(sameListings(Object.fromEntries(Object.keys(first).map((name) => [name, cache.getQueryCache().findAll({ queryKey: [name] })[0].state.data])), first));
});

test("the views and links are read once, the listings every time", async (t) => {
  const { api, paths } = server();
  const cache = fresh(t);
  await refresh(cache, api);
  paths.length = 0;
  await refresh(cache, api);
  assert.deepEqual(paths.sort(), ["/api/asks", "/api/desk", "/api/queue", "/api/reports", "/api/reports/latest", "/api/runs", "/api/tasks"]);
});

test("older rows read the listing in full, under a key of its own", async (t) => {
  const { api, paths } = server();
  const cache = fresh(t);
  await refresh(cache, api, params({ asks: false, tasks: true, runs: false }, 7));
  assert.ok(paths.includes("/api/tasks?all=1"));
  assert.ok(paths.includes("/api/reports/7"));
  assert.ok(cache.getQueryData(["tasks", true]));
  assert.equal(cache.getQueryData(["tasks", false]), undefined);
});

test("a report the arrows reached that is gone reads as none", async (t) => {
  const cache = fresh(t);
  const api = async (path) => {
    if (path === "/api/reports/9") throw Object.assign(new Error("gone"), { status: 404 });
    return path === "/api/links" ? { links: [] } : path === "/api/reports" ? { reports: [] } : { revision: 1 };
  };
  const read = await refresh(cache, api, params(undefined, 9));
  assert.equal(read.report, null);
});

test("a read lands all its listings in one notification", async (t) => {
  const { api, body } = server();
  const cache = fresh(t);
  await refresh(cache, api);
  for (const listing of Object.values(body)) listing.revision = 2;
  const read = await readAll(cache, api, params());
  let heard = 0;
  for (const name of ["asks", "tasks"]) t.after(new QueryObserver(cache, { queryKey: [name, false], enabled: false }).subscribe(notifyManager.batchCalls(() => heard++)));
  let flushes = 0;
  notifyManager.setBatchNotifyFunction((flush) => (flushes++, flush()));
  t.after(() => notifyManager.setBatchNotifyFunction((flush) => flush()));
  land(cache, read, params());
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(heard, 2);
  assert.equal(flushes, 1);
});

test("overlapping reads each ask the server, and write nothing until they land", async (t) => {
  const { api, body, paths, hold } = server();
  const cache = fresh(t);
  const first = await refresh(cache, api);
  paths.length = 0;
  const release = hold();
  const superseded = readAll(cache, api, params());
  const latest = readAll(cache, api, params());
  assert.equal(paths.length, 14);
  body.tasks.tasks[0].title = "b";
  release();
  await Promise.all([superseded, latest]);
  assert.equal(cache.getQueryData(["tasks", false]), first.tasks);
  assert.equal(land(cache, await latest, params()).tasks.tasks[0].title, "b");
});

test("a listing read at another revision with the same rows draws nothing", async (t) => {
  const { api, body } = server();
  const cache = fresh(t);
  const first = await refresh(cache, api);
  for (const listing of [body.asks, body.tasks, body.runs]) listing.revision = 2;
  assert.ok(sameListings(first, await refresh(cache, api)));
  body.desk.revision = 2;
  assert.ok(!sameListings(first, await refresh(cache, api)));
});
