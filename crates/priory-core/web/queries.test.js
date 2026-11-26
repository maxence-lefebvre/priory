import { test } from "node:test";
import assert from "node:assert/strict";
import { QueriesObserver, QueryObserver } from "@tanstack/react-query";
import { client, heldRevision, keyOf, moved, oldestRead, params, refreshMounted, restore, snapshot, staleAll, stuck, unread } from "./queries.js";

// A server whose listings the test changes, which records each path read.
function server() {
  const body = { desk: { revision: 1, items: [] }, asks: { asks: [{ id: 1 }], revision: 1 }, queue: { waiting: [], revision: 1 }, tasks: { tasks: [{ id: 1, title: "a" }], revision: 1 }, runs: { runs: [], revision: 1 } };
  const paths = [];
  const failing = new Map();
  let held = null;
  const api = async (path) => {
    paths.push(path);
    // The answer is as it was when the read began.
    const was = structuredClone(body);
    if (held) await held;
    const name = path.split("?")[0].replace("/api/", "");
    if (failing.has(name)) throw Object.assign(new Error(`${path} answered ${failing.get(name)}`), { status: failing.get(name) });
    if (name === "views") return { factory: "shop" };
    if (name === "links") return { links: [] };
    if (name === "reports") return { reports: [] };
    if (name.startsWith("reports/")) return { report: null };
    if (name.startsWith("tasks/")) return { id: Number(name.slice(6)), revision: was.tasks.revision };
    return was[name];
  };
  return { api, body, paths, failing, hold: () => { let release; held = new Promise((resolve) => (release = resolve)); return () => { held = null; release(); }; } };
}

// A client over a server, which drops its queries once the test ends, and what it reported.
function start(t) {
  const heard = { failed: [], refused: 0 };
  const from = server();
  const cache = client({ api: from.api, failed: (error) => heard.failed.push(error?.message ?? null), refused: () => heard.refused++ });
  t.after(() => cache.clear());
  return { ...from, cache, heard };
}

// A page that shows `keys`, as a component that observes them does, once they landed.
async function mount(t, cache, keys, options = {}) {
  const observer = new QueriesObserver(cache, keys.map((queryKey) => ({ queryKey, ...options })));
  const leave = observer.subscribe(() => {});
  t.after(leave);
  await settled(cache);
  return { observer, leave };
}

async function settled(cache) {
  for (let i = 0; i < 100; i++) {
    await new Promise((resolve) => setTimeout(resolve, 1));
    if (!cache.isFetching()) return;
  }
  throw new Error("still reading");
}

const FRAME = [["views"], ["links"], ["desk"]];

test("a page reads its own listings and no other", async (t) => {
  const { cache, paths } = start(t);
  await mount(t, cache, [...FRAME, keyOf("runs")]);
  assert.deepEqual(paths.sort(), ["/api/desk", "/api/links", "/api/runs", "/api/views"]);
});

test("a moved revision reads the mounted queries again, and the views and links no more", async (t) => {
  const { cache, paths } = start(t);
  await mount(t, cache, [...FRAME, ["task", 7]]);
  paths.length = 0;
  assert.equal(await moved(cache), true);
  assert.deepEqual(paths.sort(), ["/api/desk", "/api/tasks/7"]);
});

test("a query no page shows reads nothing until a page shows it again", async (t) => {
  const { cache, paths } = start(t);
  await mount(t, cache, FRAME);
  const runs = await mount(t, cache, [keyOf("runs")]);
  runs.leave();
  paths.length = 0;
  await moved(cache);
  assert.deepEqual(paths, ["/api/desk"]);
  await mount(t, cache, [keyOf("runs")]);
  assert.deepEqual(paths, ["/api/desk", "/api/runs"]);
});

test("a page shown again with no move since reads nothing", async (t) => {
  const { cache, paths } = start(t);
  const runs = await mount(t, cache, [keyOf("runs")]);
  runs.leave();
  paths.length = 0;
  await mount(t, cache, [keyOf("runs")]);
  assert.deepEqual(paths, []);
});

test("moves during a read join it: at most one read of each is in flight", async (t) => {
  const { cache, paths, hold } = start(t);
  await mount(t, cache, [["desk"]]);
  paths.length = 0;
  const release = hold();
  const all = Promise.all([moved(cache), moved(cache), moved(cache)]);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(paths, ["/api/desk"]);
  release();
  assert.deepEqual(await all, [true, true, true]);
});

test("a person's refresh supersedes the read in flight, whose answer never lands", async (t) => {
  const { cache, body, paths, hold } = start(t);
  await mount(t, cache, [keyOf("tasks")]);
  paths.length = 0;
  body.tasks.tasks[0].title = "late";
  const release = hold();
  const late = moved(cache);
  await new Promise((resolve) => setTimeout(resolve, 5));
  body.tasks.tasks[0].title = "b";
  const asked = refreshMounted(cache);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(paths.length, 2);
  release();
  await Promise.all([late, asked]);
  assert.equal(cache.getQueryData(keyOf("tasks")).tasks[0].title, "b");
});

test("a hidden tab reads nothing, and reads once when it is shown", async (t) => {
  const { cache, paths } = start(t);
  const { observer } = await mount(t, cache, [["desk"]]);
  paths.length = 0;
  observer.setQueries([{ queryKey: ["desk"], enabled: false }]);
  await moved(cache);
  await refreshMounted(cache);
  assert.deepEqual(paths, []);
  await staleAll(cache);
  observer.setQueries([{ queryKey: ["desk"], enabled: true }]);
  await settled(cache);
  assert.deepEqual(paths, ["/api/desk"]);
});

test("the last data stays on screen until a re-read lands", async (t) => {
  const { cache, body, hold } = start(t);
  await mount(t, cache, [keyOf("tasks")]);
  body.tasks.tasks[0].title = "b";
  const release = hold();
  const reading = moved(cache);
  assert.equal(cache.getQueryData(keyOf("tasks")).tasks[0].title, "a");
  release();
  await reading;
  assert.equal(cache.getQueryData(keyOf("tasks")).tasks[0].title, "b");
});

test("a failed re-read keeps the last data, says why, and a later read says it is over", async (t) => {
  const { cache, failing, heard } = start(t);
  await mount(t, cache, FRAME);
  const desk = cache.getQueryData(["desk"]);
  failing.set("desk", 500);
  assert.equal(await moved(cache), false);
  assert.equal(cache.getQueryData(["desk"]), desk);
  assert.equal(heard.failed.at(-1), "/api/desk answered 500");
  failing.clear();
  assert.equal(await moved(cache), true);
  assert.equal(heard.failed.at(-1), null);
});

test("one listing that lands does not say another's failure is over", async (t) => {
  const { cache, failing, heard } = start(t);
  await mount(t, cache, [["desk"], keyOf("asks")]);
  failing.set("asks", 500);
  await moved(cache);
  heard.failed.length = 0;
  await moved(cache);
  assert.deepEqual(heard.failed, ["/api/asks answered 500"]);
});

test("only the listings an answer depends on are heard, and a page's own listing says it was not read", async (t) => {
  for (const name of ["queue", "tasks", "runs", "report", "reports"]) {
    const { cache, failing, heard } = start(t);
    await mount(t, cache, [["desk"], keyOf("asks"), keyOf(name)]);
    assert.equal(unread(cache), null);
    failing.set(name === "report" ? "reports/latest" : name, 500);
    assert.equal(await moved(cache), false);
    assert.deepEqual([heard.failed.filter(Boolean), unread(cache).name], [[], name]);
    failing.clear();
    await moved(cache);
    assert.equal(unread(cache), null);
  }
});

test("a detail that fails is its page's to say", async (t) => {
  const { cache, failing, heard } = start(t);
  failing.set("tasks/9", 404);
  await mount(t, cache, [["desk"], ["task", 9]]);
  assert.deepEqual(heard.failed.filter(Boolean), []);
});

test("a refused token is heard from any read", async (t) => {
  const { cache, failing, heard } = start(t);
  failing.set("tasks/9", 401);
  await mount(t, cache, [["task", 9]]);
  assert.equal(heard.refused, 1);
});

test("a read that the cache dropped, such as one from before a sign-out, is not heard", async (t) => {
  const { cache, failing, heard, hold } = start(t);
  const observer = new QueryObserver(cache, { queryKey: ["desk"] });
  t.after(observer.subscribe(() => {}));
  const release = hold();
  failing.set("desk", 401);
  cache.clear();
  release();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(heard.refused, 0);
});

test("a read with nothing new keeps the listing the same object, and one with a change replaces only it", async (t) => {
  const { cache, body } = start(t);
  await mount(t, cache, [keyOf("asks"), keyOf("tasks")]);
  const before = ["asks", "tasks"].map((name) => cache.getQueryData(keyOf(name)));
  await moved(cache);
  assert.deepEqual(["asks", "tasks"].map((name, at) => cache.getQueryData(keyOf(name)) === before[at]), [true, true]);
  body.tasks.tasks[0].title = "b";
  await moved(cache);
  assert.deepEqual(["asks", "tasks"].map((name, at) => cache.getQueryData(keyOf(name)) === before[at]), [true, false]);
});

test("older rows read the listing in full, under a key of its own", async (t) => {
  const { cache, paths } = start(t);
  const wanted = params({ asks: false, tasks: true, runs: false }, 7);
  await mount(t, cache, [keyOf("tasks", wanted), keyOf("report", wanted), keyOf("asks", wanted)]);
  assert.deepEqual(paths.sort(), ["/api/asks", "/api/reports/7", "/api/tasks?all=1"]);
  assert.equal(cache.getQueryData(keyOf("tasks")), undefined);
});

test("a report the arrows reached that is gone reads as none", async (t) => {
  const { cache, failing, heard } = start(t);
  failing.set("reports/9", 404);
  await mount(t, cache, [["report", 9]]);
  assert.equal(cache.getQueryData(["report", 9]), null);
  assert.deepEqual(heard.failed.filter(Boolean), []);
});

test("the next wait starts at the oldest revision on screen, once every listing landed", async (t) => {
  const { cache, body } = start(t);
  assert.equal(heldRevision(cache), undefined);
  await mount(t, cache, FRAME);
  body.runs.revision = 4;
  await mount(t, cache, [keyOf("runs")]);
  assert.equal(heldRevision(cache), 1);
  restore(cache, { reads: [[["queue"], { waiting: [], revision: 9 }, 5]] });
  const observer = new QueryObserver(cache, { queryKey: ["queue"], enabled: true, staleTime: "static" });
  t.after(observer.subscribe(() => {}));
  assert.equal(heldRevision(cache), undefined, "data kept on the device names no revision");
});

test("a kept snapshot draws at once and is read again when a page shows it", async (t) => {
  const { cache, paths } = start(t);
  await mount(t, cache, [...FRAME, keyOf("tasks", params({ asks: false, tasks: true, runs: false }))]);
  const kept = JSON.parse(JSON.stringify(snapshot(cache)));
  const later = start(t);
  restore(later.cache, kept);
  assert.deepEqual(later.cache.getQueryData(keyOf("tasks")), { tasks: [{ id: 1, title: "a" }] });
  await mount(t, later.cache, [["desk"], ["views"]]);
  assert.deepEqual(later.paths.sort(), ["/api/desk", "/api/views"]);
  assert.ok(paths.length);
});

test("each kept listing keeps its own age, and the strip names the oldest one on screen", async (t) => {
  const { cache } = start(t);
  restore(cache, { reads: [[["desk"], { items: [] }, 100], [["runs", false], { runs: [] }, 50], [["queue"], {}, 10]] });
  for (const queryKey of [["desk"], ["runs", false]]) t.after(new QueryObserver(cache, { queryKey, staleTime: "static" }).subscribe(() => {}));
  assert.equal(oldestRead(cache), 50);
});

test("a snapshot an earlier version kept still draws", (t) => {
  const { cache } = start(t);
  restore(cache, { updated: 77, data: { views: { factory: "shop" }, links: [], desk: { items: [], revision: 3 }, asks: [{ id: 1 }], bands: ["now"], queue: { waiting: [] }, tasks: [{ id: 2 }], runs: [], older: { tasks: 4 }, report: null, reports: [], spend: null, flows: null } });
  assert.deepEqual(cache.getQueryData(keyOf("asks")), { asks: [{ id: 1 }], bands: ["now"], older: undefined });
  assert.deepEqual(cache.getQueryData(keyOf("tasks")), { tasks: [{ id: 2 }], older: 4 });
  assert.deepEqual(cache.getQueryData(["desk"]), { items: [] });
  assert.equal(cache.getQueryState(["desk"]).dataUpdatedAt, 77);
  assert.equal(cache.getQueryData(["spend"]), undefined);
});

test("no snapshot, and one that is not ours, restore nothing", (t) => {
  const { cache } = start(t);
  restore(cache, null);
  restore(cache, {});
  assert.deepEqual(cache.getQueryCache().getAll(), []);
});

test("a detail that cannot be read again fails the refresh, locks nothing, and holds no poll back", async (t) => {
  const { cache, body, failing, heard } = start(t);
  await mount(t, cache, [["desk"], ["task", 7]]);
  failing.set("tasks/7", 500);
  body.desk.revision = 2;
  assert.equal(await moved(cache), false);
  assert.deepEqual(heard.failed.filter(Boolean), []);
  assert.equal(heldRevision(cache), 2);
});

test("a detail never read keeps a listing's recovery heard", async (t) => {
  const { cache, failing, heard } = start(t);
  failing.set("tasks/9", 404);
  await mount(t, cache, [["desk"], ["task", 9]]);
  failing.set("desk", 500);
  await moved(cache);
  assert.equal(heard.failed.at(-1), "/api/desk answered 500");
  failing.delete("desk");
  await moved(cache);
  assert.equal(heard.failed.at(-1), null);
});

test("leaving the page whose read failed says the failure is over", async (t) => {
  const { cache, failing, heard } = start(t);
  await mount(t, cache, [["desk"]]);
  failing.set("asks", 500);
  const page = await mount(t, cache, [keyOf("asks")]);
  assert.equal(heard.failed.at(-1), "/api/asks answered 500");
  page.leave();
  assert.equal(heard.failed.at(-1), null);
});

test("a page is stuck while a listing it shows failed with nothing live to draw", async (t) => {
  const { cache, failing } = start(t);
  assert.equal(stuck(cache), false);
  failing.set("runs", 500);
  await mount(t, cache, [["desk"], keyOf("runs")]);
  assert.equal(stuck(cache), true);
  failing.clear();
  await moved(cache);
  assert.equal(stuck(cache), false);
  failing.set("runs", 500);
  await moved(cache);
  assert.equal(stuck(cache), false, "the poll reads a page that landed once");
});

test("a listing the device kept, whose live read fails, is stuck too", async (t) => {
  const { cache, failing } = start(t);
  restore(cache, { reads: [[["runs", false], { runs: [] }, 5]] });
  failing.set("runs", 500);
  await mount(t, cache, [keyOf("runs")]);
  assert.equal(stuck(cache), true);
});

test("a read that began before the move is read once more after it, never two at a time", async (t) => {
  const { cache, body, paths, hold } = start(t);
  await mount(t, cache, [["desk"]]);
  paths.length = 0;
  const release = hold();
  const early = moved(cache);
  await new Promise((resolve) => setTimeout(resolve, 5));
  body.desk = { revision: 2, items: [{ id: 9 }] };
  const joined = moved(cache);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.deepEqual(paths, ["/api/desk"]);
  release();
  await Promise.all([early, joined]);
  assert.deepEqual(cache.getQueryData(["desk"]), { revision: 2, items: [{ id: 9 }] });
  assert.deepEqual(paths, ["/api/desk", "/api/desk"]);
});
