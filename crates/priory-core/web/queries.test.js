import { test } from "node:test";
import assert from "node:assert/strict";
import { client, observed, params, readAll, sameListings, sameParams } from "./queries.js";

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
  const first = await readAll(cache, api, params());
  const second = await readAll(cache, api, params());
  assert.ok(sameListings(first, second));
});

test("a read keeps the listings that did not change and replaces the one that did", async (t) => {
  const { api, body } = server();
  const cache = fresh(t);
  const first = await readAll(cache, api, params());
  body.tasks.tasks[0].title = "b";
  const second = await readAll(cache, api, params());
  assert.equal(second.asks, first.asks);
  assert.notEqual(second.tasks, first.tasks);
  assert.equal(second.tasks.tasks[0].title, "b");
  assert.ok(!sameListings(first, second));
});

test("the cache shows the last listings until a re-read lands", async (t) => {
  const { api, body, hold } = server();
  const cache = fresh(t);
  await readAll(cache, api, params());
  body.tasks.tasks[0].title = "b";
  const release = hold();
  const reading = readAll(cache, api, params());
  assert.equal(cache.getQueryData(["tasks", false]).tasks[0].title, "a");
  release();
  await reading;
  assert.equal(cache.getQueryData(["tasks", false]).tasks[0].title, "b");
});

test("a failed re-read throws and keeps the last listings", async (t) => {
  const { api, failing } = server();
  const cache = fresh(t);
  const first = await readAll(cache, api, params());
  failing.add("desk");
  await assert.rejects(readAll(cache, api, params()), /answered 500/);
  assert.equal(cache.getQueryData(["desk"]), first.desk);
  assert.equal(cache.getQueryState(["desk"]).status, "error");
});

test("the views and links are read once, the listings every time", async (t) => {
  const { api, paths } = server();
  const cache = fresh(t);
  await readAll(cache, api, params());
  paths.length = 0;
  await readAll(cache, api, params());
  assert.deepEqual(paths.sort(), ["/api/asks", "/api/desk", "/api/queue", "/api/reports", "/api/reports/latest", "/api/runs", "/api/tasks"]);
});

test("older rows read the listing in full, under a key of its own", async (t) => {
  const { api, paths } = server();
  const cache = fresh(t);
  await readAll(cache, api, params({ asks: false, tasks: true, runs: false }, 7));
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
  const read = await readAll(cache, api, params(undefined, 9));
  assert.equal(read.report, null);
});

test("useQueries' listings are none until every listing has data, then each by name", () => {
  const { queries, combine } = observed(async () => ({}), params());
  assert.equal(queries.length, 9);
  assert.ok(queries.every((query) => typeof query.placeholderData === "function"));
  assert.equal(combine(queries.map(() => ({ data: undefined }))), null);
  const all = combine(queries.map((query) => ({ data: query.queryKey[0] })));
  assert.equal(all.tasks, "tasks");
  assert.equal(all.reports, "reports");
});

test("params compare by value", () => {
  assert.ok(sameParams(params(), params()));
  assert.ok(!sameParams(params(), params({ asks: true, tasks: false, runs: false })));
});
