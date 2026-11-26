import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";

const kept = new Map();
let wrote = [];
globalThis.localStorage = { getItem: (key) => kept.get(key) ?? null, setItem: (key, value) => { kept.set(key, value); wrote.push(key); }, removeItem: (key) => kept.delete(key) };
kept.set("priory.token", "t");
const { session, signIn, signOut } = await import("./session.js");

// A page load: data.js runs again over what this device kept.
let loads = 0;
const load = () => import(`./data.js?load=${++loads}`);
const OLD = JSON.stringify({ updated: 5, data: { views: {}, links: [], desk: { items: [{ id: 1 }] }, queue: {}, reports: [] } });
const NEW = JSON.stringify({ reads: [[["desk"], { items: [{ id: 2 }] }, 9]] });
const pause = (ms = 10) => new Promise((resolve) => setTimeout(resolve, ms));
beforeEach(() => {
  kept.clear();
  if (!session.get().token) signIn("t");
  kept.set("priory.token", "t");
  wrote = [];
});

test("what an earlier version kept draws, and its own key stays as that version wrote it", async () => {
  kept.set("priory.snapshot", OLD);
  const { cache } = await load();
  assert.deepEqual(cache.getQueryData(["desk"]), { items: [{ id: 1 }] });
  cache.setQueryData(["desk"], { items: [{ id: 3 }] });
  await pause();
  assert.equal(kept.get("priory.snapshot"), OLD);
  assert.equal(JSON.parse(kept.get("priory.snapshot.2")).reads.length, 5);
});

test("this version's copy wins over the earlier one's, and a broken one is dropped", async () => {
  kept.set("priory.snapshot", OLD);
  kept.set("priory.snapshot.2", NEW);
  assert.deepEqual((await load()).cache.getQueryData(["desk"]), { items: [{ id: 2 }] });
  kept.set("priory.snapshot.2", "{");
  await load();
  assert.equal(kept.has("priory.snapshot.2"), false);
});

test("a sign-out drops both copies, and a start without a token draws neither", async () => {
  kept.set("priory.snapshot", OLD);
  kept.set("priory.snapshot.2", NEW);
  signOut(null);
  assert.deepEqual([...kept.keys()], []);
  kept.set("priory.snapshot.2", NEW);
  const { cache } = await load();
  assert.deepEqual([cache.getQueryData(["desk"]), kept.size], [undefined, 0]);
});

test("a refresh is kept in one write, once its last read lands or fails", async () => {
  const { cache } = await load();
  await pause();
  wrote = [];
  const late = (ms, end) => new Promise((resolve, reject) => setTimeout(() => end(resolve, reject), ms));
  await Promise.all([
    cache.fetchQuery({ queryKey: ["desk"], queryFn: () => late(5, (land) => land({ items: [] })) }),
    cache.fetchQuery({ queryKey: ["queue"], queryFn: () => late(20, (land) => land({})) }),
    cache.fetchQuery({ queryKey: ["runs", false], queryFn: () => late(40, (_, fail) => fail(new Error("no"))) }).catch(() => {}),
  ]);
  await pause();
  assert.deepEqual(wrote, ["priory.snapshot.2"]);
  assert.deepEqual(JSON.parse(kept.get("priory.snapshot.2")).reads.map(([[name]]) => name).sort(), ["desk", "queue"]);
});

test("a refresh is kept once the read it waited for is cancelled", async () => {
  const { cache } = await load();
  const waits = cache.fetchQuery({ queryKey: ["task", 7], queryFn: ({ signal }) => new Promise((_, reject) => signal.addEventListener("abort", () => reject(new Error("left")))) }).catch(() => {});
  cache.setQueryData(["desk"], { items: [] });
  await pause();
  assert.deepEqual(wrote, []);
  await cache.cancelQueries({ queryKey: ["task", 7] });
  await waits;
  await pause();
  assert.deepEqual(wrote, ["priory.snapshot.2"]);
});
