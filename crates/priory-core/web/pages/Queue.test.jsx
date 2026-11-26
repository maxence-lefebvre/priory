import { test } from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { QueryClientProvider } from "@tanstack/react-query";

// The data modules these pages import read the device's storage and the page as they load.
const sheet = { addEventListener() {}, open: false };
Object.assign(globalThis, {
  localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
  document: { hidden: false, addEventListener() {}, getElementById: () => sheet, documentElement: { dataset: {} } },
  addEventListener() {},
  matchMedia: () => ({ matches: false, addEventListener() {} }),
});
const { client, keyOf } = await import("../queries.js");
const { QueueList, QueuePage, moving } = await import("./Queue.jsx");
const { Place } = await import("./place.js");

const SINCE = "2026-11-25T10:00:00Z";
const task = (id, state, more = {}) => ({ id, key: `linear:ENG-${id}`, state, workflow: "build", step: "implement", lane: "web", band: "now", since: SINCE, ...more });

function draw(t, Part, { href = "/tasks", size = "wide", data = {}, errors = {} } = {}) {
  const cache = client({ api: async () => ({}) });
  for (const [key, value] of Object.entries(data)) cache.setQueryData(JSON.parse(key), value);
  for (const [key, message] of Object.entries(errors)) cache.getQueryCache().build(cache, { queryKey: JSON.parse(key) }).setState({ status: "error", error: new Error(message) });
  t.after(() => cache.clear());
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ href, size }}><Part /></Place.Provider></QueryClientProvider>);
}
const K = (name, all) => JSON.stringify(keyOf(name, { all: { asks: false, tasks: Boolean(all), runs: Boolean(all) } }));

const QUEUE = {
  revision: 9,
  bands: ["now", "later"],
  waiting: [
    task(5, "queued", { band_position: 1, title: "First" }),
    task(6, "queued", { band_position: 2, title: "Second" }),
    task(7, "held", { reason: "" }),
  ],
  ended: [task(8, "done", { ended: SINCE })],
  up_next: [{ kind: "queued", task: 5, key: "linear:ENG-5", why: "first in the queue", at: SINCE }],
  lanes: [{ lane: "web", running: 2, cap: 2, max_running: 2, window: null, max_running_outside: null }],
};

test("the Queue draws Up next, lane caps, bands and Move menus as before React", (t) => {
  const html = draw(t, QueueList, { href: "/queue", data: { [K("queue")]: QUEUE } });
  assert.match(html, /<div class="scroll"><div class="sec">Up next<span>next 3 runs<\/span><\/div><div class="strip"><ol><li><time [^>]*>now, when a runner has room<\/time><a href="\/tasks\/5" title="task 5"><span class="mono">ENG-5<\/span> first in the queue<\/a><\/li><\/ol><\/div>/);
  assert.match(html, /<div class="sec">Lane caps<span>runs per lane on this runner<\/span><\/div><ul class="strip caps"><li><span class="mono">web<\/span><span class="t-warn">2 of 2 running<\/span><span class="cap">max_running 2<\/span><\/li><\/ul>/);
  assert.match(html, /<table class="tbl q"><thead><tr><th class="r">#<\/th><th>State<\/th><th>Task<\/th><th>Title and why it waits<\/th><th><\/th><\/tr><\/thead><tbody><tr class="grp"><td colSpan="5">Band now<span>2<\/span><\/td><\/tr><tr data-href="\/tasks\/5"><td class="r">#1<\/td>/);
  assert.match(html, /<td class="q full"><span>Second<\/span><span class="why t3">Waits behind linear:ENG-5 in band now.<\/span><\/td><td class="act"><details class="move" data-move="6"><summary id="move-6" class="btn secondary" aria-label="Move ENG-6">Move<svg/);
  assert.match(html, /<div class="pick"><button type="button" id="move-6-0" data-action="move" data-task="6" data-option="0">To the top<\/button><button type="button" id="move-6-1" data-action="move" data-task="6" data-option="1">Up one<\/button><button type="button" id="move-6-2" data-action="move" data-task="6" data-option="2">To band later<\/button><\/div><\/details>/);
  assert.match(html, /<a href="\/queue\?view=held" aria-current="false">Held<span class="num">1<\/span><\/a><a href="\/queue\?view=ended" aria-current="false">Last day<span class="num">1<\/span><\/a>/);
});

test("an open Move menu stays open, and its refusal shows under its row", (t) => {
  t.after(() => moving.set({ busy: null, notices: new Map(), open: null }));
  moving.set({ busy: 5, notices: new Map([[5, "Not moved: the queue changed."]]), open: 5 });
  const html = draw(t, QueuePage, { href: "/queue", size: "compact", data: { [K("queue")]: QUEUE } });
  assert.match(html, /<div class="row qrow" data-href="\/tasks\/5">.*<span class="l1"><span class="num">#1<\/span><a class="mono" href="\/tasks\/5" title="task 5">ENG-5<\/a><span>queued<\/span><span class="age">/);
  assert.match(html, /<span class="l3 wrap">Waits for its turn: first in band now.<\/span><details class="move" data-move="5" open="">/);
  assert.match(html, /id="move-5-0" data-action="move" data-task="5" data-option="0" aria-disabled="true">Down one/);
  assert.match(html, /<\/details><p class="note warn" role="status"><svg[^>]*>.*?<\/svg><span>Not moved: the queue changed.<\/span><\/p><\/div>/);
  assert.match(html, /<div class="sec">Band now<span>2<\/span><\/div><div class="row qrow"/);
});

test("the Queue's Held and Last day views draw task rows, with stacks from the tasks listing", (t) => {
  const tasks = { tasks: [task(7, "held", { stack: [{ state: "open" }, { state: "merged" }] })] };
  const held = draw(t, QueueList, { href: "/queue?view=held", size: "medium", data: { [K("queue")]: QUEUE, [K("tasks")]: tasks } });
  assert.match(held, /<div class="row" data-href="\/tasks\/7">.*<span>held<\/span><span class="tag">2 PRs<\/span>.*<span class="l3"><span class="t-[a-z]+">held<\/span><\/span>/);
  assert.match(draw(t, QueueList, { href: "/queue?view=ended", data: { [K("queue")]: { ...QUEUE, ended: [] } } }), /<div class="scroll"><p class="empty">No task ended in the last day.<\/p><\/div>/);
  assert.match(draw(t, QueueList, { href: "/queue", data: { [K("queue")]: { ...QUEUE, waiting: [], up_next: [], lanes: [] } } }), /<div class="scroll"><div class="sec">Up next<span>next 3 runs<\/span><\/div><p class="empty">Nothing is queued or scheduled to run next.<\/p><p class="empty">The queue is empty. A task waits here for its turn once it is added.<\/p><\/div>/);
  assert.equal(draw(t, QueueList, { errors: { [K("queue")]: "/api/queue answered 500" } }), '<p class="empty">Could not load this page: /api/queue answered 500. Check that <code>priory serve</code> or the daemon runs, then retry.</p>');
});
