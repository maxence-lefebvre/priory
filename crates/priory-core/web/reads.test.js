import { test } from "node:test";
import assert from "node:assert/strict";
import { batchKey } from "./reads/batch.js";
import { attentionKey, costsKey } from "./reads/costs.js";
import { flowsKey } from "./reads/flows.js";
import { inboxKey } from "./reads/inbox.js";
import { requestKey } from "./reads/request.js";
import { client, moved, newest, read, restore, snapshot } from "./queries.js";

test("each family reads its own path", async () => {
  const paths = [];
  const api = async (path) => void paths.push(path);
  for (const key of [batchKey("2"), batchKey("batch:2026-11-25-1", 3), costsKey("band"), attentionKey(), flowsKey(), inboxKey(), requestKey()]) await read(api, key);
  assert.deepEqual(paths, ["/api/batches/2", "/api/batches/batch:2026-11-25-1", "/api/costs?by=band", "/api/attention", "/api/flows", "/api/inbox?unrouted=1", "/api/request"]);
});

test("a batch's plan is held for one revision of its go ask", () => {
  assert.notDeepEqual(batchKey("batch:2026-11-25-1", 3), batchKey("batch:2026-11-25-1", 4));
});

test("a device keeps the costs, the attention and the flows", () => {
  const cache = client({ api: async () => ({}) });
  cache.setQueryData(costsKey("band"), { by: "band" });
  cache.setQueryData(attentionKey(), { acts: 1 });
  cache.setQueryData(flowsKey(), { workflows: [] });
  cache.setQueryData(requestKey(), { workflows: [] });
  const next = client({ api: async () => ({}) });
  restore(next, JSON.parse(JSON.stringify(snapshot(cache))));
  assert.deepEqual(next.getQueryData(costsKey("band")), { by: "band" });
  assert.deepEqual(next.getQueryData(attentionKey()), { acts: 1 });
  assert.deepEqual(next.getQueryData(flowsKey()), { workflows: [] });
  assert.equal(next.getQueryData(requestKey()), undefined);
});

test("the spend a legacy Costs panel kept fills both of its reads", () => {
  const cache = client({ api: async () => ({}) });
  restore(cache, { reads: [[["spend"], { costs: { by: "workflow" }, attention: { acts: 2 }, updated: 5 }, 5]] });
  assert.deepEqual(cache.getQueryData(costsKey("workflow")), { by: "workflow" });
  assert.deepEqual(cache.getQueryData(attentionKey()), { acts: 2 });
});

test("the newest read of a listing answers, whichever rows it asked for", () => {
  const cache = client({ api: async () => ({}) });
  assert.equal(newest(cache, "tasks"), undefined);
  cache.setQueryData(["tasks", false], { tasks: [1] }, { updatedAt: 1 });
  cache.setQueryData(["tasks", true], { tasks: [1, 2] }, { updatedAt: 2 });
  assert.deepEqual(newest(cache, "tasks").tasks, [1, 2]);
});

test("a moved revision reads the mounted inbox again, and never the flows", async () => {
  const { QueryObserver } = await import("@tanstack/react-query");
  const paths = [];
  const cache = client({ api: async (path) => (paths.push(path), {}) });
  for (const queryKey of [inboxKey(), flowsKey()]) new QueryObserver(cache, { queryKey }).subscribe(() => {});
  await new Promise((done) => setTimeout(done, 10));
  paths.length = 0;
  await moved(cache);
  assert.deepEqual(paths, ["/api/inbox?unrouted=1"]);
});
