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
const { TasksList, TasksPage } = await import("./Tasks.jsx");
const { remoteOpen } = await import("../components/TaskRows.jsx");
const { session } = await import("../session.js");
const { Place } = await import("./place.js");

const SINCE = "2026-11-25T10:00:00Z";
const task = (id, state, more = {}) => ({ id, key: `linear:ENG-${id}`, state, workflow: "build", step: "implement", lane: "web", band: "now", since: SINCE, ...more });
const TASKS = {
  revision: 7,
  older: 3,
  tasks: [
    task(1, "running", { title: "Ship <it>", stack: [{ state: "open" }, { state: "open" }] }),
    task(2, "blocked", { reason: "needs a key", batch: 4 }),
    task(3, "done"),
    { key: "linear:ENG-262", runner: "laptop", hold: "kept", workflow: "build", since: SINCE, last_seen: SINCE, generation: 2, title: "Remote work", submissions: [{ act: "cancel" }] },
  ],
};

function draw(t, Part, { href = "/tasks", size = "wide", data = {}, errors = {} } = {}) {
  const cache = client({ api: async () => ({}) });
  for (const [key, value] of Object.entries(data)) cache.setQueryData(JSON.parse(key), value);
  for (const [key, message] of Object.entries(errors)) cache.getQueryCache().build(cache, { queryKey: JSON.parse(key) }).setState({ status: "error", error: new Error(message) });
  t.after(() => cache.clear());
  return renderToStaticMarkup(<QueryClientProvider client={cache}><Place.Provider value={{ href, size }}><Part /></Place.Provider></QueryClientProvider>);
}
const K = (name, all) => JSON.stringify(keyOf(name, { all: { asks: false, tasks: Boolean(all), runs: Boolean(all) } }));

test("Tasks draws its views, facets and table rows as before React", (t) => {
  const html = draw(t, TasksList, { data: { [K("tasks")]: TASKS } });
  assert.match(html, /^<div class="list" role="region" aria-label="Tasks"><div class="lhead"><h1>Tasks<\/h1><\/div><nav class="views" aria-label="Saved views"><a href="\/tasks\?view=active" aria-current="true">Active<span class="num">3<\/span><\/a><a href="\/tasks\?view=blocked" aria-current="false">Blocked<span class="num">1<\/span><\/a><a href="\/tasks\?view=ended" aria-current="false">Ended<span class="num">4<\/span><\/a><\/nav><div class="facets" aria-label="Facets"><details class="facet"><summary aria-label="Filter by state">State<svg/);
  assert.match(html, /<div class="scroll"><table class="tbl tasks"><thead><tr><th>State<\/th><th>Task<\/th><th>Title<\/th><th>Workflow and step<\/th><th>Lane<\/th><th>Band<\/th><th class="r">Age<\/th><\/tr><\/thead><tbody><tr data-href="\/tasks\/1"><td><span class="sti t-/);
  assert.match(html, /<td class="mono"><a href="\/tasks\/1" title="task 1">ENG-1<\/a><\/td><td class="q full"><span>Ship &lt;it&gt;<\/span><\/td><td>build <span class="t3">implement<\/span><\/td><td>web<\/td><td>now<\/td><td class="r"><time/);
  assert.match(html, /<span>needs a key <a class="tag" href="\/batches\/4"><svg[^>]*>.*?<\/svg>batch 4<\/a><\/span><span class="why t-[a-z]+">needs a key<\/span>|<span> <a class="tag" href="\/batches\/4">/);
  // A runner facet shows in a factory of several runners, and names no runner here.
  assert.match(html, /<summary aria-label="Filter by runner">Runner/);
  assert.match(html, /<tr class="remote" data-action="remote-open" data-task="laptop:linear%3AENG-262"><td><span class="sti"><span class="tile [a-z]+ sm" role="img" aria-label="elsewhere">/);
  assert.match(html, /<span class="why"><span class="t-action">Sent to laptop: cancel<\/span><\/span><span class="why t3">On runner laptop, last seen [^<]+<\/span>/);
  assert.match(html, /<time [^>]*>[^<]*<\/time> <button type="button" id="remote-laptop:linear%3AENG-262" class="chev" data-action="remote-open" data-task="laptop:linear%3AENG-262" aria-expanded="false" aria-label="Open the actions of ENG-262">/);
  assert.ok(!html.includes("remote-acts"));
});

test("Tasks on a phone draws rows, a picked facet and its older rows", (t) => {
  const html = draw(t, TasksPage, { href: "/tasks?view=ended&state=done", size: "compact", data: { [K("tasks")]: TASKS } });
  assert.match(html, /^<div class="top"><h1>Tasks<\/h1><span class="sp"><\/span><span class="conn ">live<\/span><\/div><div class="panel" role="region" aria-label="Tasks"><div class="scroll"><nav class="views"/);
  assert.match(html, /<a href="\/tasks\?view=ended&amp;state=done" aria-current="true">Ended<span class="num">1<\/span><\/a>/);
  assert.match(html, /<a class="facet" aria-current="true" href="\/tasks\?view=ended" aria-label="State: done. Clear">done<svg/);
  assert.match(html, /<div class="row" data-href="\/tasks\/3"><span class="tile [a-z]+ " role="img" aria-label="done">.*<span class="l1"><a class="mono" href="\/tasks\/3" title="task 3">ENG-3<\/a><span>build, implement<\/span><span>done<\/span><span class="age"><time/);
  assert.match(html, /<span class="l3"><span>web<\/span><span>now<\/span><\/span><span class="chev"><svg/);
  assert.match(html, /<p class="empty" data-older="tasks">Loading older tasks…<\/p><\/div><\/div>$/);
});

test("Tasks list rows tag a stack, and a blocked task shows why", (t) => {
  const html = draw(t, TasksList, { size: "medium", data: { [K("tasks")]: TASKS } });
  assert.match(html, /<span>running<\/span><span class="tag">2 PRs<\/span><span class="age">/);
  assert.match(html, /<span class="l3"><span class="t-[a-z]+">needs a key<\/span><a class="tag" href="\/batches\/4">/);
  assert.match(html, /<div class="row remote" data-action="remote-open" data-task="laptop:linear%3AENG-262"><span class="tile [a-z]+ " role="img" aria-label="elsewhere">/);
});

test("an open remote row shows its actions, its Say and where they go", (t) => {
  t.after(() => remoteOpen.set(null));
  remoteOpen.set("laptop:linear%3AENG-262");
  const html = draw(t, TasksList, { data: { [K("tasks")]: TASKS } });
  assert.match(html, /aria-expanded="true" aria-label="Close the actions of ENG-262">/);
  assert.match(html, /<tr class="remote-acts"><td><\/td><td><\/td><td colSpan="5"><span class="task-acts"><button type="button" id="act-laptop:linear%3AENG-262-resume" class="btn secondary act " aria-disabled="false">Resume<\/button>.*Cancel.*Release<\/button><\/span>/);
  assert.match(html, /<form class="writeform"><label class="sr" for="say">Say to the agent<\/label><textarea id="say" name="text" rows="2" placeholder="Say to the agent"><\/textarea><button type="submit" class="btn primary">Say<\/button><\/form><p class="note acts-note"><svg[^>]*>.*?<\/svg><span>This task runs on laptop. An action goes there through the log, and laptop applies it when it reads it.<\/span><\/p>/);
});

test("Tasks says it loads, then why it could not, and keeps its rows through a failed re-read", (t) => {
  assert.equal(draw(t, TasksList), '<p class="empty" role="status">Loading…</p>');
  assert.equal(draw(t, TasksList, { errors: { [K("tasks")]: "/api/tasks answered 500" } }), '<p class="empty">Could not load this page: /api/tasks answered 500. Check that <code>priory serve</code> or the daemon runs, then retry.</p>');
  t.after(() => session.set({ ...session.get(), failed: null }));
  session.set({ ...session.get(), failed: "the connection dropped" });
  assert.match(draw(t, TasksPage, { size: "compact" }), /Could not load this page: the connection dropped./);
  assert.match(draw(t, TasksList, { data: { [K("tasks")]: TASKS } }), /ENG-262/);
});
