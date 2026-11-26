import { test } from "node:test";
import assert from "node:assert/strict";
import { QueryObserver } from "@tanstack/react-query";
import { attention } from "./reads/attention.js";
import { batch } from "./reads/batch.js";
import { costs } from "./reads/costs.js";
import { flows } from "./reads/flows.js";
import { inbox } from "./reads/inbox.js";
import { request } from "./reads/request.js";
import { client, hold, moved, newest, restore, snapshot } from "./queries.js";

test("each family reads its own path", async () => {
  const paths = [];
  const cache = client({ api: async (path) => (paths.push(path), {}) });
  for (const read of [batch("2"), batch("batch:2026-11-25-1", 3), costs("band"), attention(), flows(), inbox(), request()]) await cache.fetchQuery(read);
  assert.deepEqual(paths, ["/api/batches/2", "/api/batches/batch:2026-11-25-1", "/api/costs?by=band", "/api/attention", "/api/flows", "/api/inbox?unrouted=1", "/api/request"]);
});

test("a batch's plan is held for one revision of its go ask", () => {
  assert.notDeepEqual(batch("batch:2026-11-25-1", 3).queryKey, batch("batch:2026-11-25-1", 4).queryKey);
});

test("a device keeps the costs, the attention and the flows, also those no page showed since it opened", () => {
  const cache = client({ api: async () => ({}) });
  hold(cache, costs("band"), { by: "band" });
  hold(cache, attention(), { acts: 1 });
  hold(cache, flows(), { workflows: [] });
  hold(cache, request(), { workflows: [] });
  let next = cache;
  for (let opened = 0; opened < 2; opened++) {
    const kept = JSON.parse(JSON.stringify(snapshot(next)));
    next = client({ api: async () => ({}) });
    restore(next, kept);
  }
  assert.deepEqual(next.getQueryData(costs("band").queryKey), { by: "band" });
  assert.deepEqual(next.getQueryData(attention().queryKey), { acts: 1 });
  assert.deepEqual(next.getQueryData(flows().queryKey), { workflows: [] });
  assert.equal(next.getQueryData(request().queryKey), undefined);
});

test("a kept read is read again once a page shows it, and its own read answers", async () => {
  const paths = [];
  const cache = client({ api: async (path) => (paths.push(path), { live: true }) });
  restore(cache, { reads: [[["flows"], { workflows: [] }, 5]] });
  new QueryObserver(cache, flows()).subscribe(() => {});
  await new Promise((done) => setTimeout(done, 10));
  assert.deepEqual([paths, cache.getQueryData(flows().queryKey)], [["/api/flows"], { live: true }]);
  await moved(cache);
  assert.deepEqual(paths, ["/api/flows"]);
});

test("the spend a legacy Costs panel kept fills both of its reads, and the next snapshot keeps them alone", () => {
  const cache = client({ api: async () => ({}) });
  restore(cache, { reads: [[["spend"], { costs: { by: "workflow" }, attention: { acts: 2 }, updated: 5 }, 5]] });
  assert.deepEqual(cache.getQueryData(costs("workflow").queryKey), { by: "workflow" });
  assert.deepEqual(cache.getQueryData(attention().queryKey), { acts: 2 });
  assert.deepEqual(snapshot(cache).reads.map(([key]) => key[0]).sort(), ["attention", "costs"]);
});

test("a kept split of the costs reads nothing until the page shows it, then its own path", async () => {
  const paths = [];
  const cache = client({ api: async (path) => (paths.push(path), { by: "band", live: true }) });
  restore(cache, { reads: [[["costs", "band"], { by: "band" }, 5]] });
  await moved(cache);
  assert.deepEqual(paths, []);
  new QueryObserver(cache, costs("band")).subscribe(() => {});
  await new Promise((done) => setTimeout(done, 10));
  assert.deepEqual([paths, cache.getQueryData(costs("band").queryKey)], [["/api/costs?by=band"], { by: "band", live: true }]);
});

test("the newest read of a listing answers, whichever rows it asked for", () => {
  const cache = client({ api: async () => ({}) });
  assert.equal(newest(cache, "tasks"), undefined);
  cache.setQueryData(["tasks", false], { tasks: [1] }, { updatedAt: 1 });
  cache.setQueryData(["tasks", true], { tasks: [1, 2] }, { updatedAt: 2 });
  assert.deepEqual(newest(cache, "tasks").tasks, [1, 2]);
});

test("a moved revision reads the mounted inbox again, and never the flows", async () => {
  const paths = [];
  const cache = client({ api: async (path) => (paths.push(path), {}) });
  for (const read of [inbox(), flows()]) new QueryObserver(cache, read).subscribe(() => {});
  await new Promise((done) => setTimeout(done, 10));
  paths.length = 0;
  await moved(cache);
  assert.deepEqual(paths, ["/api/inbox?unrouted=1"]);
});

test("a moved revision reads the costs after the desk, and not while the desk fails", async () => {
  const paths = [];
  let down = false;
  const cache = client({ api: async (path) => {
    paths.push(path);
    if (down) throw Object.assign(new Error(`${path} answered 500`), { status: 500 });
    return { revision: 1 };
  } });
  for (const read of [{ queryKey: ["desk"] }, costs("workflow")]) new QueryObserver(cache, read).subscribe(() => {});
  await new Promise((done) => setTimeout(done, 10));
  paths.length = 0;
  await moved(cache);
  assert.deepEqual(paths, ["/api/desk", "/api/costs?by=workflow"]);
  paths.length = 0;
  down = true;
  await moved(cache);
  assert.deepEqual(paths, ["/api/desk"]);
});
