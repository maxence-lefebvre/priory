import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The data modules read the device's storage and the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => sheet, querySelector: () => null, querySelectorAll: () => [], documentElement: { dataset: {} } },
  addEventListener() {},
  location: { origin: "http://localhost" },
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { client, keyOf } = await import("../queries.js");
const { batch: batchRead } = await import("../reads/batch.js");
const { BatchPage } = await import("./Batch.jsx");
const { Place } = await import("./place.js");

const BATCH = {
  id: 1, key: "linear:ENG-238", task: 6, title: "Invoice <exports>", state: "going", lane: "payments", plan_revision: 2, repo: "acme/shop",
  counts: { merged: 1, waiting: 2 }, closes: [{ issue: "ENG-201", why: "done in ENG-238", closed: null }], splits: [],
  entries: [
    { entry: "e1", issue: "ENG-201", title: "Add the table", task: 7, state: "done", stage: "merged", after: [] },
    { entry: "e2", issue: "ENG-239", title: "Export as CSV", files: ["api/"], stage: "waiting", after: [{ entry: "e1", moment: "merged" }] },
  ],
};

function draw(t, { batch, tasks = [], size = "wide" }) {
  const cache = client({ api: async () => ({}) });
  cache.setQueryData(keyOf("links"), []);
  if (tasks) cache.setQueryData(keyOf("tasks"), { tasks, older: 0 });
  if (batch) cache.setQueryData(batchRead("1").queryKey, batch);
  t.after(() => cache.clear());
  const place = { size, href: "/batches/1", at: { here: "batches", batch: "1" }, trail: { list: "desk", listUrl: "/" }, entry: { from: "/tasks" } };
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={place}><BatchPage id="1" /></Place.Provider></QueryClientProvider>);
}

test("a batch page draws its head, stages, one lane row per entry and the issues it closes", (t) => {
  const html = draw(t, { batch: BATCH, tasks: [{ id: 7, key: "ENG-201", step: "merge", since: "2026-11-25T10:00:00Z", stack: [{ number: 12, state: "merged" }] }] });
  assert.match(html, /^<div class="list page"><div class="top"><a class="back" href="\/tasks">/);
  assert.match(html, /<h1 class="question">Invoice &lt;exports&gt;<\/h1>/);
  assert.match(html, /<p class="stagewords"><span><b>1<\/b> merged<\/span><span><b>2<\/b> waiting<\/span><\/p>/);
  assert.match(html, /<div class="sh"><svg[^>]*>.*?<\/svg>2 entries in start order<\/div>/);
  assert.equal(html.match(/<li class="erow /g).length, 2);
  assert.match(html, /<a class="mono" href="\/tasks\/7" title="task 7">ENG-201<\/a>/);
  assert.match(html, /<span>#12 merged, merge, /, "an entry's task says its pull request and step");
  assert.match(html, /<section class="sect" id="closes">.*Closes 1 issue/);
  assert.doesNotMatch(html, /iconbtn menu/, "the batch page edits nothing");
});

test("a batch waits for the tasks, then says why it could not be read", (t) => {
  assert.equal(draw(t, { batch: null, tasks: null }), '<p class="empty" role="status">Loading…</p>');
  assert.match(draw(t, { batch: null, size: "compact" }), /^<div class="flat"><div class="top">.*<h1 class="sr">Batch 1<\/h1><p class="empty">Loading batch 1…<\/p><\/div>$/);
});
